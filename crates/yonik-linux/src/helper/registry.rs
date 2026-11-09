//! Системный allowlist: пути и параметры запуска не поступают от клиента.

use std::{fs::File, io::Read, os::unix::fs::MetadataExt, str::FromStr};

use rustix::fs::{Mode, OFlags, open, openat};
use serde::Deserialize;
use yonik_engine::Engine;

use crate::host::{BinarySnapshot, HostError, Sha256};

const MAX_MANIFEST: u64 = 4096;
const FILE_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK);
const DIR_FLAGS: OFlags = FILE_FLAGS.union(OFlags::DIRECTORY);

/// Ошибки не содержат исходные пути, содержимое файлов или ошибки ОС.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryError {
    Unavailable,
    UnsafeEntry,
    Manifest,
    Version,
    HashMismatch,
    InvalidBinary,
    Snapshot,
    MissingEngine,
}
impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Системный реестр ядер недоступен",
            Self::UnsafeEntry => "Недоверенная запись системного реестра",
            Self::Manifest => "Некорректный манифест системного реестра",
            Self::Version => "Версия ядра не соответствует контракту",
            Self::HashMismatch => "SHA-256 системного ядра не совпадает",
            Self::InvalidBinary => "Некорректный бинарник системного ядра",
            Self::Snapshot => "Не удалось создать неизменяемый снимок системного ядра",
            Self::MissingEngine => "Ядро отсутствует в системном реестре",
        })
    }
}
impl std::error::Error for RegistryError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    engines: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    engine: InstalledEngine,
    version: String,
    sha256: String,
}

#[derive(Clone, Copy, Deserialize)]
enum InstalledEngine {
    #[serde(rename = "xray")]
    Xray,
    #[serde(rename = "sing-box")]
    SingBox,
}
impl InstalledEngine {
    fn engine(self) -> Engine {
        match self {
            Self::Xray => Engine::Xray,
            Self::SingBox => Engine::SingBox,
        }
    }
    fn filename(self) -> &'static str {
        match self {
            Self::Xray => "xray",
            Self::SingBox => "sing-box",
        }
    }
}

/// Манифест проверен целиком; бинарник проверяется при создании snapshot.
/// Один и тот же snapshot передаётся version/config-check/start без чтения пути.
pub struct TrustedRegistry {
    directory: File,
    entries: Vec<InstalledEntry>,
}

struct InstalledEntry {
    engine: InstalledEngine,
    hash: Sha256,
}

impl std::fmt::Debug for TrustedRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TrustedRegistry(<системный реестр>)")
    }
}
impl TrustedRegistry {
    /// Единственный production-корень: /usr/lib/yonik/engines, владелец root.
    pub fn system() -> Result<Self, RegistryError> {
        let mut directory = File::from(
            open("/", DIR_FLAGS, Mode::empty()).map_err(|_| RegistryError::Unavailable)?,
        );
        directory_metadata(&directory, 0)?;
        for component in ["usr", "lib", "yonik", "engines"] {
            directory = child(&directory, component, DIR_FLAGS)?;
            directory_metadata(&directory, 0)?;
        }
        Self::load(directory, 0)
    }

    /// Проверяет выбранный бинарник и копирует его из того же проверенного FD.
    pub fn snapshot(&self, engine: Engine) -> Result<BinarySnapshot, RegistryError> {
        self.snapshot_for_uid(engine, 0)
    }

    fn snapshot_for_uid(&self, engine: Engine, uid: u32) -> Result<BinarySnapshot, RegistryError> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.engine.engine() == engine)
            .ok_or(RegistryError::MissingEngine)?;
        directory_metadata(&self.directory, uid)?;
        let source = child(&self.directory, entry.engine.filename(), FILE_FLAGS)?;
        regular_metadata(&source, uid, BinarySnapshot::MAX_SIZE, true)?;
        BinarySnapshot::load_file(source, entry.hash, engine).map_err(|error| match error {
            HostError::BinaryHash => RegistryError::HashMismatch,
            HostError::InvalidBinary => RegistryError::InvalidBinary,
            HostError::Snapshot => RegistryError::Snapshot,
            _ => RegistryError::Unavailable,
        })
    }

    fn load(directory: File, uid: u32) -> Result<Self, RegistryError> {
        directory_metadata(&directory, uid)?;
        let manifest_file = child(&directory, "registry.json", FILE_FLAGS)?;
        regular_metadata(&manifest_file, uid, MAX_MANIFEST, false)?;
        let mut bytes = Vec::new();
        manifest_file
            .take(MAX_MANIFEST + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| RegistryError::Unavailable)?;
        if bytes.len() as u64 > MAX_MANIFEST {
            return Err(RegistryError::Manifest);
        }
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| RegistryError::Manifest)?;
        if manifest.schema != 1 || manifest.engines.is_empty() || manifest.engines.len() > 2 {
            return Err(RegistryError::Manifest);
        }
        let mut entries: Vec<InstalledEntry> = Vec::with_capacity(manifest.engines.len());
        for entry in manifest.engines {
            let engine = entry.engine.engine();
            if entries.iter().any(|entry| entry.engine.engine() == engine) {
                return Err(RegistryError::Manifest);
            }
            if entry.version != engine.version() {
                return Err(RegistryError::Version);
            }
            let hash = Sha256::from_str(&entry.sha256).map_err(|_| RegistryError::Manifest)?;
            entries.push(InstalledEntry {
                engine: entry.engine,
                hash,
            });
        }
        Ok(Self { directory, entries })
    }

    /// Тесты передают уже открытый собственный корень, не ослабляя production walk.
    #[cfg(test)]
    fn for_test(directory: File, expected_uid: u32) -> Result<Self, RegistryError> {
        Self::load(directory, expected_uid)
    }
}

fn child(directory: &File, name: &str, flags: OFlags) -> Result<File, RegistryError> {
    openat(directory, name, flags, Mode::empty())
        .map(File::from)
        .map_err(|_| RegistryError::Unavailable)
}

fn directory_metadata(file: &File, uid: u32) -> Result<(), RegistryError> {
    let metadata = file.metadata().map_err(|_| RegistryError::Unavailable)?;
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o7022 != 0 {
        return Err(RegistryError::UnsafeEntry);
    }
    Ok(())
}

fn regular_metadata(
    file: &File,
    uid: u32,
    max_size: u64,
    executable: bool,
) -> Result<(), RegistryError> {
    let metadata = file.metadata().map_err(|_| RegistryError::Unavailable)?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o7022 != 0
        || metadata.len() > max_size
        || (executable && metadata.mode() & 0o111 == 0)
    {
        return Err(RegistryError::UnsafeEntry);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BinarySnapshot, DIR_FLAGS, Engine, FILE_FLAGS, File, MAX_MANIFEST, Mode, Read,
        RegistryError, TrustedRegistry, child, directory_metadata, open, regular_metadata,
    };
    use sha2::{Digest, Sha256 as Hasher};
    use std::{
        fs::{self, Permissions},
        io::{Seek, SeekFrom, Write},
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);
    const ELF: &[u8] = b"\x7fELFfixture: never execute";

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "yonik-registry-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, Permissions::from_mode(0o700)).unwrap();
            let fixture = Self(path);
            fixture.binary("xray", ELF);
            fixture.binary("sing-box", ELF);
            fixture.manifest(&serde_json::json!({"schema":1,"engines":[entry("xray")]}));
            fixture
        }
        fn binary(&self, name: &str, bytes: &[u8]) {
            fs::write(self.0.join(name), bytes).unwrap();
            self.mode(name, 0o700);
        }
        fn mode(&self, name: &str, mode: u32) {
            fs::set_permissions(self.0.join(name), Permissions::from_mode(mode)).unwrap();
        }
        fn manifest(&self, value: &serde_json::Value) {
            fs::write(
                self.0.join("registry.json"),
                serde_json::to_vec(value).unwrap(),
            )
            .unwrap();
            self.mode("registry.json", 0o600);
        }
        fn directory(&self) -> File {
            File::from(open(&self.0, DIR_FLAGS, Mode::empty()).unwrap())
        }
        fn uid(&self) -> u32 {
            self.directory().metadata().unwrap().uid()
        }
        fn registry(&self) -> Result<TrustedRegistry, RegistryError> {
            TrustedRegistry::for_test(self.directory(), self.uid())
        }
        fn snapshot(&self) -> Result<BinarySnapshot, RegistryError> {
            self.registry()?.snapshot_for_uid(Engine::Xray, self.uid())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("Удалить собственный fixture");
        }
    }
    fn entry(name: &str) -> serde_json::Value {
        let engine = if name == "xray" {
            Engine::Xray
        } else {
            Engine::SingBox
        };
        serde_json::json!({"engine":name,"version":engine.version(),"sha256":format!("{:x}",Hasher::digest(ELF))})
    }
    fn bytes(snapshot: &mut BinarySnapshot) -> Vec<u8> {
        snapshot.file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        snapshot.file.read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn subset_and_both_engines_create_sealed_snapshots_without_execution() {
        let fixture = Fixture::new();
        let registry = fixture.registry().unwrap();
        assert_eq!(
            registry
                .snapshot_for_uid(Engine::SingBox, fixture.uid())
                .err(),
            Some(RegistryError::MissingEngine)
        );
        let mut image = registry
            .snapshot_for_uid(Engine::Xray, fixture.uid())
            .unwrap();
        assert_eq!(bytes(&mut image), ELF);
        assert!(image.file.write_all(b"replace").is_err());
        fixture
            .manifest(&serde_json::json!({"schema":1,"engines":[entry("xray"),entry("sing-box")]}));
        let registry = fixture.registry().unwrap();
        for engine in [Engine::Xray, Engine::SingBox] {
            assert_eq!(
                bytes(&mut registry.snapshot_for_uid(engine, fixture.uid()).unwrap()),
                ELF
            );
        }
    }

    #[test]
    fn every_manifest_entry_is_strict_even_when_not_selected() {
        let fixture = Fixture::new();
        let good = entry("xray");
        let mut unknown_field = good.clone();
        unknown_field["argv"] = serde_json::json!(["--version"]);
        let mut wrong_version = entry("sing-box");
        wrong_version["version"] = serde_json::json!("0.0.0");
        let mut wrong_hash = entry("sing-box");
        wrong_hash["sha256"] = serde_json::json!("bad");
        for (manifest, error) in [
            (
                serde_json::json!({"schema":2,"engines":[good.clone()]}),
                RegistryError::Manifest,
            ),
            (
                serde_json::json!({"schema":1,"engines":[]}),
                RegistryError::Manifest,
            ),
            (
                serde_json::json!({"schema":1,"engines":[good.clone(),good.clone()]}),
                RegistryError::Manifest,
            ),
            (
                serde_json::json!({"schema":1,"engines":[good.clone()],"path":"/x"}),
                RegistryError::Manifest,
            ),
            (
                serde_json::json!({"schema":1,"engines":[unknown_field]}),
                RegistryError::Manifest,
            ),
            (
                serde_json::json!({"schema":1,"engines":[entry("unknown")]}),
                RegistryError::Manifest,
            ),
            (
                serde_json::json!({"schema":1,"engines":[good.clone(),wrong_version]}),
                RegistryError::Version,
            ),
            (
                serde_json::json!({"schema":1,"engines":[good,wrong_hash]}),
                RegistryError::Manifest,
            ),
        ] {
            fixture.manifest(&manifest);
            assert_eq!(fixture.registry().err(), Some(error));
        }
        for malformed in [
            "{",
            "{}",
            "{\"schema\":1,\"schema\":1,\"engines\":[]}",
            "{\"schema\":1,\"engines\":[{\"engine\":\"xray\",\"engine\":\"xray\"}]}",
        ] {
            fs::write(fixture.0.join("registry.json"), malformed).unwrap();
            assert_eq!(fixture.registry().err(), Some(RegistryError::Manifest));
        }
    }

    #[test]
    fn wrong_owner_modes_links_and_oversized_files_are_rejected() {
        let fixture = Fixture::new();
        assert_eq!(
            TrustedRegistry::for_test(fixture.directory(), fixture.uid().wrapping_add(1)).err(),
            Some(RegistryError::UnsafeEntry)
        );
        let binary = File::open(fixture.0.join("xray")).unwrap();
        assert_eq!(
            regular_metadata(
                &binary,
                fixture.uid().wrapping_add(1),
                BinarySnapshot::MAX_SIZE,
                true
            ),
            Err(RegistryError::UnsafeEntry)
        );
        let manifest = File::open(fixture.0.join("registry.json")).unwrap();
        assert_eq!(
            regular_metadata(
                &manifest,
                fixture.uid().wrapping_add(1),
                MAX_MANIFEST,
                false
            ),
            Err(RegistryError::UnsafeEntry)
        );
        for name in ["registry.json", "xray"] {
            for mode in [0o722, 0o770, 0o4700, 0o2700] {
                fixture.mode(name, mode);
                assert_eq!(fixture.snapshot().err(), Some(RegistryError::UnsafeEntry));
            }
            fixture.mode(name, 0o700);
            fs::hard_link(fixture.0.join(name), fixture.0.join("link")).unwrap();
            assert_eq!(fixture.snapshot().err(), Some(RegistryError::UnsafeEntry));
            fs::remove_file(fixture.0.join("link")).unwrap();
        }
        fixture.mode("", 0o770);
        assert_eq!(fixture.registry().err(), Some(RegistryError::UnsafeEntry));
        fixture.mode("", 0o1700);
        assert_eq!(
            directory_metadata(&fixture.directory(), fixture.uid()),
            Err(RegistryError::UnsafeEntry)
        );
        fixture.mode("", 0o700);
        fixture.mode("xray", 0o600);
        assert_eq!(fixture.snapshot().err(), Some(RegistryError::UnsafeEntry));
        fixture.mode("xray", 0o700);
        File::options()
            .write(true)
            .open(fixture.0.join("xray"))
            .unwrap()
            .set_len(BinarySnapshot::MAX_SIZE + 1)
            .unwrap();
        assert_eq!(fixture.snapshot().err(), Some(RegistryError::UnsafeEntry));
        File::options()
            .write(true)
            .open(fixture.0.join("registry.json"))
            .unwrap()
            .set_len(MAX_MANIFEST + 1)
            .unwrap();
        assert_eq!(fixture.registry().err(), Some(RegistryError::UnsafeEntry));
    }

    #[test]
    fn symlinks_directories_and_fifos_are_never_followed_or_read() {
        for name in ["registry.json", "xray"] {
            let fixture = Fixture::new();
            fs::rename(fixture.0.join(name), fixture.0.join("original")).unwrap();
            symlink("original", fixture.0.join(name)).unwrap();
            assert_eq!(fixture.snapshot().err(), Some(RegistryError::Unavailable));
            fs::remove_file(fixture.0.join(name)).unwrap();
            fs::create_dir(fixture.0.join(name)).unwrap();
            assert_eq!(fixture.snapshot().err(), Some(RegistryError::UnsafeEntry));
            fs::remove_dir(fixture.0.join(name)).unwrap();
            rustix::fs::mknodat(
                rustix::fs::CWD,
                fixture.0.join(name),
                rustix::fs::FileType::Fifo,
                Mode::RUSR | Mode::WUSR,
                0,
            )
            .unwrap();
            assert_eq!(fixture.snapshot().err(), Some(RegistryError::UnsafeEntry));
        }
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join("directory")).unwrap();
        symlink("directory", fixture.0.join("alias")).unwrap();
        assert!(child(&fixture.directory(), "alias", DIR_FLAGS).is_err());
    }

    #[test]
    fn valid_manifest_does_not_claim_binary_verified_until_snapshot() {
        let fixture = Fixture::new();
        let registry = fixture.registry().unwrap();
        fixture.binary("xray", b"\x7fELFchanged");
        assert_eq!(
            registry.snapshot_for_uid(Engine::Xray, fixture.uid()).err(),
            Some(RegistryError::HashMismatch)
        );
        fixture.binary("xray", b"not an ELF");
        assert_eq!(fixture.snapshot().err(), Some(RegistryError::InvalidBinary));
        fs::remove_file(fixture.0.join("xray")).unwrap();
        assert_eq!(fixture.snapshot().err(), Some(RegistryError::Unavailable));
    }

    #[test]
    fn pinned_directory_and_source_fd_survive_path_replacement() {
        let fixture = Fixture::new();
        let registry = fixture.registry().unwrap();
        let uid = fixture.uid();
        let renamed = fixture.0.with_extension("renamed");
        fs::rename(&fixture.0, &renamed).unwrap();
        fs::create_dir(&fixture.0).unwrap();
        let mut image = registry.snapshot_for_uid(Engine::Xray, uid).unwrap();
        assert_eq!(bytes(&mut image), ELF);
        fs::remove_dir(&fixture.0).unwrap();
        fs::rename(&renamed, &fixture.0).unwrap();

        let source = child(&registry.directory, "xray", FILE_FLAGS).unwrap();
        regular_metadata(&source, uid, BinarySnapshot::MAX_SIZE, true).unwrap();
        fs::rename(fixture.0.join("xray"), fixture.0.join("old")).unwrap();
        fixture.binary("xray", b"\x7fELFreplacement");
        let mut from_fd =
            BinarySnapshot::load_file(source, registry.entries[0].hash, Engine::Xray).unwrap();
        assert_eq!(bytes(&mut from_fd), ELF);
        fixture.binary("old", b"\x7fELFafter snapshot");
        assert_eq!(bytes(&mut image), ELF);
        assert_eq!(bytes(&mut from_fd), ELF);
        assert_eq!(
            registry.snapshot_for_uid(Engine::Xray, uid).err(),
            Some(RegistryError::HashMismatch)
        );
    }
}
