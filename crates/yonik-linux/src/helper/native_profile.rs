//! Закрытая проверка установленного producer; сама по себе не выдаёт admission.
use super::systemd_worker::{
    BootstrapUnitObservation, SystemWorkerSupervisor, WorkerSupervisorError as Error,
    child, root, trusted, verify_installed, FILE_FLAGS,
};
use crate::Cancellation;
use rustix::fs::{openat, Mode};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, os::unix::fs::MetadataExt};
use tokio::time::Instant;

type Result<T> = std::result::Result<T, Error>;
const MAX_METADATA: u64 = 4096;
const MAX_ELF: u64 = 128 * 1024 * 1024;
const MAX_SOURCE: u64 = 512 * 1024 * 1024;
// Identity политики отдельно от хеша image, который эту политику исполняет.
const POLICY: &str = "yonik-own-namespace-fixed16-v1";
const ABI: &str = "linux-native-abi260-ynkch004-ynkns004-v1";

// derive отвергает duplicate fields, deny_unknown_fields закрывает расширения.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Deployment {
    schema: u32,
    worker_sha256: [u8; 32],
    source_archive_sha256: [u8; 32],
    policy_id: String,
    policy_sha256: [u8; 32],
    kernel_release: String,
    kernel_machine: String,
    kernel_abi: String,
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
}
fn stamp(file: &File) -> Result<FileStamp> {
    let m = file.metadata().map_err(|_| Error::Unavailable)?;
    Ok(FileStamp { dev: m.dev(), ino: m.ino(), len: m.len(),
        mtime: m.mtime(), mtime_ns: m.mtime_nsec(),
        ctime: m.ctime(), ctime_ns: m.ctime_nsec() })
}
fn immutable(file: &File) -> Result<()> {
    trusted(file, false)?;
    // «Immutable» здесь — граница root/package TCB, не FS_IMMUTABLE_FL.
    if file.metadata().map_err(|_| Error::Unavailable)?.mode() & 0o222 != 0 {
        return Err(Error::Untrusted);
    }
    Ok(())
}
fn fixed_file(directory: &File, name: &str) -> Result<File> {
    let f = File::from(openat(directory, name, FILE_FLAGS, Mode::empty())
        .map_err(|_| Error::Unavailable)?);
    immutable(&f)?;
    Ok(f)
}
fn hash(file: &File, limit: u64, elf: bool) -> Result<[u8; 32]> {
    let before = stamp(file)?;
    if before.len == 0 || before.len > limit { return Err(Error::Untrusted); }
    let mut reader = file;
    let mut buf = [0u8; 16384];
    let mut digest = Sha256::new();
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|_| Error::Unavailable)?;
        if n == 0 { break; }
        if total == 0 && elf && (n < 4 || &buf[..4] != b"\x7fELF") {
            return Err(Error::Untrusted);
        }
        total = total.checked_add(n as u64).ok_or(Error::Untrusted)?;
        if total > limit { return Err(Error::Untrusted); }
        digest.update(&buf[..n]);
    }
    if total != before.len || stamp(file)? != before { return Err(Error::Stale); }
    Ok(digest.finalize().into())
}
fn parse(file: &File) -> Result<Deployment> {
    let before = stamp(file)?;
    if before.len == 0 || before.len > MAX_METADATA { return Err(Error::Untrusted); }
    let mut bytes = Vec::with_capacity(before.len as usize);
    file.take(MAX_METADATA + 1).read_to_end(&mut bytes).map_err(|_| Error::Unavailable)?;
    if bytes.len() as u64 != before.len || stamp(file)? != before {
        return Err(Error::Stale);
    }
    let value: Deployment = serde_json::from_slice(&bytes).map_err(|_| Error::Untrusted)?;
    if value.schema != 1 || value.policy_id != POLICY || value.kernel_abi != ABI
        || value.worker_sha256 == [0; 32] || value.source_archive_sha256 == [0; 32]
        || value.policy_sha256 == [0; 32]
        || value.kernel_release.is_empty() || value.kernel_release.len() > 128
        || !value.kernel_release.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        || value.kernel_machine != std::env::consts::ARCH
    { return Err(Error::Untrusted); }
    Ok(value)
}

struct InstalledContract {
    _metadata: File,
    _source: File,
    image: File,
    deployment: Deployment,
}
impl InstalledContract {
    fn verify() -> Result<Self> {
        verify_installed()?;
        let directory = child(&child(&child(&root()?, "usr")?, "lib")?, "yonik")?;
        let metadata = fixed_file(&directory, "native-profile-v1.json")?;
        let deployment = parse(&metadata)?;
        let image = fixed_file(&directory, "yonik-worker")?;
        if image.metadata().map_err(|_| Error::Unavailable)?.mode() & 0o111 == 0
            || hash(&image, MAX_ELF, true)? != deployment.worker_sha256
        { return Err(Error::Untrusted); }
        let source = fixed_file(&directory, "native-source-v1.tar")?;
        if hash(&source, MAX_SOURCE, false)? != deployment.source_archive_sha256 {
            return Err(Error::Untrusted);
        }
        // Читаем declared kernel/proc TCB, не запускаем feature probes.
        let release = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map_err(|_| Error::Unavailable)?;
        if release != format!("{}\n", deployment.kernel_release) {
            return Err(Error::Untrusted);
        }
        Ok(Self { _metadata: metadata, _source: source, image, deployment })
    }
}
struct ExecutableIdentity {
    _actual: File,
}
impl ExecutableIdentity {
    fn verify(observation: &BootstrapUnitObservation, installed: &InstalledContract) -> Result<Self> {
        observation.validate_identity()?;
        // Единственная намеренная proc magic-link: actual exe canonical pidfd/start.
        // Никакой путь из sidecar или caller здесь не используется.
        let actual = File::open(format!("/proc/{}/exe", observation.native_pid()))
            .map_err(|_| Error::Unavailable)?;
        immutable(&actual)?;
        if stamp(&actual)? != stamp(&installed.image)?
            || hash(&actual, MAX_ELF, true)? != installed.deployment.worker_sha256
        { return Err(Error::Untrusted); }
        observation.validate_identity()?;
        Ok(Self { _actual: actual })
    }
}

/// Move-only owning input будущего coverage verifier. Не UnitProfile/permit.
/// Поля закрыты; нет bytes/FD/bool конструктора, Clone или Serialize.
pub(super) struct VerifiedInstalledProfile {
    observation: BootstrapUnitObservation,
    installed: InstalledContract,
    _executable: ExecutableIdentity,
}
impl VerifiedInstalledProfile {
    pub(super) async fn verify(
        supervisor: &SystemWorkerSupervisor,
        observation: BootstrapUnitObservation,
        cancel: &mut Cancellation,
        deadline: Instant,
    ) -> Result<Self> {
        supervisor.validate_native_profile_context(&observation, cancel, deadline).await?;
        let installed = InstalledContract::verify()?;
        let executable = ExecutableIdentity::verify(&observation, &installed)?;
        supervisor.validate_native_profile_context(&observation, cancel, deadline).await?;
        Ok(Self { observation, installed, _executable: executable })
    }
    /// Consuming recheck после await; старый token при ошибке уничтожается.
    /// Consumers удерживают возвращённый token до consume coverage/owner lease.
    pub(super) async fn revalidate(
        self, supervisor: &SystemWorkerSupervisor,
        cancel: &mut Cancellation, deadline: Instant,
    ) -> Result<Self> {
        let fresh = Self::verify(supervisor, self.observation, cancel, deadline).await?;
        if fresh.installed.deployment.worker_sha256 != self.installed.deployment.worker_sha256
            || fresh.installed.deployment.source_archive_sha256 != self.installed.deployment.source_archive_sha256
            || fresh.installed.deployment.policy_sha256 != self.installed.deployment.policy_sha256
        { return Err(Error::Stale); }
        Ok(fresh)
    }
    pub(super) fn policy_digest(&self) -> [u8; 32] {
        self.installed.deployment.policy_sha256
    }
    pub(super) fn observation(&self) -> &BootstrapUnitObservation { &self.observation }
}
