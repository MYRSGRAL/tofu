//! Rooted encrypted store. Переданный directory FD принадлежит trusted host.
use crate::rule_set_custody::{RuleSetFormat, RuleSetRef, open_resource, seal_resource};
use rustix::fs::{self as fs, AtFlags, FileType, Mode, OFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Write},
    os::fd::AsRawFd,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;
const DISK_CAP: u64 = 512 * 1024 * 1024;
const FLAGS: OFlags = OFlags::NOFOLLOW
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);
fn bad() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Небезопасное или повреждённое хранилище",
    )
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    digest: [u8; 32],
    bytes: u64,
    records: u64,
    format: u8,
}
impl Descriptor {
    fn from_ref(r: &RuleSetRef) -> Self {
        Self {
            digest: *r.digest(),
            bytes: r.bytes(),
            records: r.records(),
            format: match r.format() {
                RuleSetFormat::XrayDomainsV1 => 1,
                RuleSetFormat::XrayCidrsV1 => 2,
                RuleSetFormat::SingSourceV3 => 3,
            },
        }
    }
    fn reference(&self) -> io::Result<RuleSetRef> {
        RuleSetRef::new(
            self.digest,
            self.bytes,
            self.records,
            match self.format {
                1 => RuleSetFormat::XrayDomainsV1,
                2 => RuleSetFormat::XrayCidrsV1,
                3 => RuleSetFormat::SingSourceV3,
                _ => return Err(bad()),
            },
        )
    }
    fn name(&self) -> String {
        let mut s = String::from("r-");
        for b in self.digest {
            use std::fmt::Write;
            write!(&mut s, "{b:02x}").expect("Запись в String");
        }
        s.push_str(".enc");
        s
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    generation: u64,
    current: Vec<Descriptor>,
    previous: Vec<Descriptor>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            generation: 0,
            current: Vec::new(),
            previous: Vec::new(),
        }
    }
}
struct Inner {
    root: File,
    key: Zeroizing<[u8; 32]>,
    leases: Mutex<HashMap<u64, Vec<Descriptor>>>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        // Последний Arc завершает custody, даже если fork ещё держит CLOEXEC-копию OFD.
        // RuntimeLease удерживает этот Arc до собственного освобождения.
        loop {
            match fs::flock(&self.root, fs::FlockOperation::Unlock) {
                Ok(()) => break,
                Err(rustix::io::Errno::INTR) => continue,
                Err(_) => {
                    // Как в managed_network::storage::LockedDirectory: Drop не
                    // возвращает ошибки; File закроется, копия fork — при exec.
                    break;
                }
            }
        }
    }
}
pub struct EncryptedRuleSetStore {
    inner: Arc<Inner>,
    state: State,
    next_lease: u64,
    poisoned: bool,
}
/// Drop не освобождает reservation: без подтверждения runtime lease остаётся
/// в store как quarantine. Worker integration должен предоставить evidence.
pub struct RuntimeLease {
    id: u64,
    inner: Arc<Inner>,
    resources: Vec<Descriptor>,
}
pub trait WorkerEmptyEvidence {
    fn worker_and_cgroup_empty(&self) -> bool;
}
impl RuntimeLease {
    pub fn release_after_empty(self, evidence: &impl WorkerEmptyEvidence) -> io::Result<()> {
        if !evidence.worker_and_cgroup_empty() {
            return Err(bad());
        }
        self.inner
            .leases
            .lock()
            .map_err(|_| bad())?
            .remove(&self.id);
        Ok(())
    }
    pub fn open_verified(&self, index: usize) -> io::Result<Zeroizing<Vec<u8>>> {
        let d = self.resources.get(index).ok_or_else(bad)?;
        read_resource(&self.inner, d)
    }
}
fn safe(file: &File, dir: bool) -> io::Result<()> {
    let s = fs::fstat(file)?;
    if s.st_uid != rustix::process::geteuid().as_raw()
        || FileType::from_raw_mode(s.st_mode)
            != if dir {
                FileType::Directory
            } else {
                FileType::RegularFile
            }
        || s.st_mode & 0o7777 != if dir { 0o700 } else { 0o600 }
        || !dir && s.st_nlink != 1
    {
        return Err(bad());
    }
    Ok(())
}
fn open(root: &File, name: &str) -> io::Result<File> {
    let f = File::from(fs::openat(
        root,
        name,
        FLAGS | OFlags::RDONLY,
        Mode::empty(),
    )?);
    safe(&f, false)?;
    Ok(f)
}
fn read_resource(inner: &Inner, d: &Descriptor) -> io::Result<Zeroizing<Vec<u8>>> {
    let r = d.reference()?;
    let f = open(&inner.root, &d.name())?;
    if f.metadata()?.len() != r.bytes() + 40 {
        return Err(bad());
    }
    open_resource(&r, &inner.key, f)
}
impl EncryptedRuleSetStore {
    pub fn open(directory: File, key: [u8; 32]) -> io::Result<Self> {
        safe(&directory, true)?;
        fs::flock(&directory, fs::FlockOperation::NonBlockingLockExclusive)?;
        let inner = Arc::new(Inner {
            root: directory,
            key: Zeroizing::new(key),
            leases: Mutex::new(HashMap::new()),
        });
        let state = match open(&inner.root, "state") {
            Ok(mut f) => {
                let mut header = [0u8; 48];
                f.read_exact(&mut header)?;
                let mut digest = [0u8; 32];
                digest.copy_from_slice(&header[..32]);
                let bytes = u64::from_be_bytes(header[32..40].try_into().map_err(|_| bad())?);
                let count = u64::from_be_bytes(header[40..48].try_into().map_err(|_| bad())?);
                if bytes > 65536 || count != 1 || f.metadata()?.len() != 48 + bytes + 40 {
                    return Err(bad());
                }
                let r = RuleSetRef::new(digest, bytes, count, RuleSetFormat::SingSourceV3)?;
                let plain = open_resource(&r, &inner.key, f)?;
                serde_json::from_slice::<State>(&plain).map_err(|_| bad())?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => State::default(),
            Err(e) => return Err(e),
        };
        if state.version != 1 || state.current.len() > 16 || state.previous.len() > 16 {
            return Err(bad());
        }
        for d in state.current.iter().chain(&state.previous) {
            read_resource(&inner, d)?;
        }
        let store = Self {
            inner,
            state,
            next_lease: 1,
            poisoned: false,
        };
        store.reserve(0, 0)?;
        Ok(store)
    }
    fn reserve(&self, extra: u64, extra_files: usize) -> io::Result<()> {
        let mut bytes = extra;
        let mut count = extra_files;
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", self.inner.root.as_raw_fd()))? {
            count += 1;
            if count > 128 {
                return Err(bad());
            }
            let entry = entry?;
            let name = entry.file_name();
            let f = File::from(fs::openat(
                &self.inner.root,
                &name,
                FLAGS | OFlags::RDONLY,
                Mode::empty(),
            )?);
            safe(&f, false)?;
            bytes = bytes.checked_add(f.metadata()?.len()).ok_or_else(bad)?;
            if bytes > DISK_CAP {
                return Err(bad());
            }
        }
        Ok(())
    }
    /// No-overwrite публикация: существующий digest обязательно проверяется.
    pub fn publish(&mut self, reference: &RuleSetRef, body: &[u8]) -> io::Result<()> {
        if self.poisoned {
            return Err(bad());
        }
        let d = Descriptor::from_ref(reference);
        // seal_resource проверяет digest/size до любого filesystem I/O.
        let mut ciphertext = Vec::new();
        seal_resource(reference, body, &self.inner.key, &mut ciphertext)?;
        match open(&self.inner.root, &d.name()) {
            Ok(_) => {
                read_resource(&self.inner, &d)?;
                self.reserve(0, 0)?;
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        self.atomic_write(&d.name(), &ciphertext, false)
    }
    fn atomic_write(&self, name: &str, bytes: &[u8], replace: bool) -> io::Result<()> {
        self.reserve(bytes.len() as u64, 1)?;
        let mut random = [0u8; 8];
        getrandom::getrandom(&mut random).map_err(|_| bad())?;
        let staging = format!("stage-{:016x}", u64::from_le_bytes(random));
        let mut file = File::from(fs::openat(
            &self.inner.root,
            &staging,
            FLAGS | OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
            Mode::from_raw_mode(0o600),
        )?);
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            if replace {
                match open(&self.inner.root, name) {
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                };
                fs::renameat(&self.inner.root, &staging, &self.inner.root, name)?;
            } else {
                fs::renameat_with(
                    &self.inner.root,
                    &staging,
                    &self.inner.root,
                    name,
                    fs::RenameFlags::NOREPLACE,
                )?;
            }
            self.inner.root.sync_all()
        })();
        if result.is_err() {
            let _ = fs::unlinkat(&self.inner.root, &staging, AtFlags::empty());
        }
        result
    }
    fn persist(&mut self, next: State) -> io::Result<()> {
        let plain = Zeroizing::new(serde_json::to_vec(&next).map_err(|_| bad())?);
        if plain.len() > 65536 {
            return Err(bad());
        }
        let r = RuleSetRef::new(
            Sha256::digest(&*plain).into(),
            plain.len() as u64,
            1,
            RuleSetFormat::SingSourceV3,
        )?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(r.digest());
        bytes.extend_from_slice(&r.bytes().to_be_bytes());
        bytes.extend_from_slice(&1u64.to_be_bytes());
        seal_resource(&r, &plain, &self.inner.key, &mut bytes)?;
        if let Err(error) = self.atomic_write("state", &bytes, true) {
            self.poisoned = true;
            return Err(error);
        }
        self.state = next;
        Ok(())
    }
    pub fn generation(&self) -> u64 {
        self.state.generation
    }
    pub fn commit(&mut self, expected: u64, references: &[RuleSetRef]) -> io::Result<u64> {
        if self.poisoned
            || expected != self.state.generation
            || references.is_empty()
            || references.len() > 16
        {
            return Err(bad());
        }
        let mut unique = std::collections::HashSet::new();
        let mut records = 0u64;
        let mut bytes = 0u64;
        for r in references {
            records = records.checked_add(r.records()).ok_or_else(bad)?;
            bytes = bytes.checked_add(r.bytes()).ok_or_else(bad)?;
            if !unique.insert(r.digest()) || records > 1_500_000 || bytes > DISK_CAP {
                return Err(bad());
            }
        }
        let current: Vec<_> = references.iter().map(Descriptor::from_ref).collect();
        for d in &current {
            read_resource(&self.inner, d)?;
        }
        let next = State {
            version: 1,
            generation: expected.checked_add(1).ok_or_else(bad)?,
            current,
            previous: self.state.current.clone(),
        };
        self.persist(next)?;
        Ok(self.state.generation)
    }
    pub fn rollback(&mut self, expected: u64) -> io::Result<u64> {
        if self.poisoned || expected != self.state.generation || self.state.previous.is_empty() {
            return Err(bad());
        }
        for d in &self.state.previous {
            read_resource(&self.inner, d)?;
        }
        self.persist(State {
            version: 1,
            generation: expected.checked_add(1).ok_or_else(bad)?,
            current: self.state.previous.clone(),
            previous: self.state.current.clone(),
        })?;
        Ok(self.state.generation)
    }
    pub fn lease_current(&mut self) -> io::Result<RuntimeLease> {
        if self.poisoned {
            return Err(bad());
        }
        let id = self.next_lease;
        self.next_lease = id.checked_add(1).ok_or_else(bad)?;
        let resources = self.state.current.clone();
        for d in &resources {
            read_resource(&self.inner, d)?;
        }
        self.inner
            .leases
            .lock()
            .map_err(|_| bad())?
            .insert(id, resources.clone());
        Ok(RuntimeLease {
            id,
            inner: self.inner.clone(),
            resources,
        })
    }
}
pub struct SealedNativeResource {
    pub name: String,
    pub descriptor: File,
}
impl RuntimeLease {
    /// Возвращает sealed memfd, а не путь caller. Manifest name выводится из digest.
    /// Lease должен жить до доказанного worker/cgroup-empty либо quarantine.
    pub fn sealed_resources(&self) -> io::Result<Vec<SealedNativeResource>> {
        let mut result = Vec::new();
        for (index, d) in self.resources.iter().enumerate() {
            let body = self.open_verified(index)?;
            let mut fd = File::from(fs::memfd_create(
                "yonik-resource",
                fs::MemfdFlags::CLOEXEC | fs::MemfdFlags::ALLOW_SEALING,
            )?);
            fd.write_all(&body)?;
            fs::fcntl_add_seals(
                &fd,
                fs::SealFlags::SEAL
                    | fs::SealFlags::SHRINK
                    | fs::SealFlags::GROW
                    | fs::SealFlags::WRITE,
            )?;
            let read_only = File::from(fs::open(
                format!("/proc/self/fd/{}", fd.as_raw_fd()),
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
            )?);
            let suffix = if d.format == 3 { ".json" } else { ".dat" };
            let name = d.name().trim_end_matches(".enc").to_owned() + suffix;
            result.push(SealedNativeResource {
                name,
                descriptor: read_only,
            });
        }
        Ok(result)
    }
}

pub struct SealedResourceBundle {
    pub digest: [u8; 32],
    pub resources: Vec<SealedNativeResource>,
    lease: RuntimeLease,
}
impl SealedResourceBundle {
    pub fn release_after_empty(self, evidence: &impl WorkerEmptyEvidence) -> io::Result<()> {
        self.lease.release_after_empty(evidence)
    }
}
impl RuntimeLease {
    pub fn seal_bundle(
        self,
        bundle: &yonik_engine::resource_bundle::CompiledResourceBundle,
    ) -> io::Result<SealedResourceBundle> {
        use yonik_engine::resource_bundle::ResourceKind;
        if self.resources.len() != bundle.manifest().len() {
            return Err(bad());
        }
        for (d, r) in self.resources.iter().zip(bundle.manifest()) {
            let format = match (r.engine, r.kind) {
                (yonik_engine::Engine::Xray, ResourceKind::Domains) => 1,
                (yonik_engine::Engine::Xray, ResourceKind::Cidrs) => 2,
                _ => 3,
            };
            if d.digest != r.digest
                || d.bytes != r.bytes
                || d.records != r.records
                || d.format != format
            {
                return Err(bad());
            }
        }
        let digest = crate::resource_catalog::bundle_digest(bundle)?;
        let resources = self.sealed_resources()?;
        Ok(SealedResourceBundle {
            digest,
            resources,
            lease: self,
        })
    }
}

#[derive(Debug, Default)]
pub struct PhysicalReservations {
    pub physical: u64,
    pub staging: u64,
    pub current: u64,
    pub previous: u64,
    pub active_quarantine: u64,
    pub retained: u64,
}
impl EncryptedRuleSetStore {
    /// Категории не пересекаются: active/quarantine имеет приоритет перед current.
    pub fn reservations(&self) -> io::Result<PhysicalReservations> {
        let leases = self.inner.leases.lock().map_err(|_| bad())?;
        let active: std::collections::HashSet<_> =
            leases.values().flatten().map(Descriptor::name).collect();
        let current: std::collections::HashSet<_> =
            self.state.current.iter().map(Descriptor::name).collect();
        let previous: std::collections::HashSet<_> =
            self.state.previous.iter().map(Descriptor::name).collect();
        let mut result = PhysicalReservations::default();
        let mut count = 0;
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", self.inner.root.as_raw_fd()))? {
            count += 1;
            if count > 128 {
                return Err(bad());
            }
            let entry = entry?;
            let name = entry.file_name();
            let f = File::from(fs::openat(
                &self.inner.root,
                &name,
                FLAGS | OFlags::RDONLY,
                Mode::empty(),
            )?);
            safe(&f, false)?;
            let bytes = f.metadata()?.len();
            result.physical = result.physical.checked_add(bytes).ok_or_else(bad)?;
            if result.physical > DISK_CAP {
                return Err(bad());
            }
            let name = name.to_str().ok_or_else(bad)?;
            if name.starts_with("stage-") {
                result.staging += bytes;
            } else if active.contains(name) {
                result.active_quarantine += bytes;
            } else if current.contains(name) {
                result.current += bytes;
            } else if previous.contains(name) {
                result.previous += bytes;
            } else {
                result.retained += bytes;
            }
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::DirBuilderExt;
    #[test]
    fn atomic_current_previous_rollback_corruption_and_lease() {
        let path = std::env::temp_dir().join(format!("yonik-store-{}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let key = [19; 32];
        let a = b"private-a";
        let b = b"private-b";
        let ra = RuleSetRef::new(
            Sha256::digest(a).into(),
            a.len() as u64,
            1,
            RuleSetFormat::SingSourceV3,
        )
        .unwrap();
        let rb = RuleSetRef::new(
            Sha256::digest(b).into(),
            b.len() as u64,
            1,
            RuleSetFormat::SingSourceV3,
        )
        .unwrap();
        let mut store = EncryptedRuleSetStore::open(File::open(&path).unwrap(), key).unwrap();
        store.publish(&ra, a).unwrap();
        store.publish(&rb, b).unwrap();
        store.commit(0, std::slice::from_ref(&ra)).unwrap();
        let lease = store.lease_current().unwrap();
        store.commit(1, std::slice::from_ref(&rb)).unwrap();
        assert_eq!(&*lease.open_verified(0).unwrap(), a);
        store.rollback(2).unwrap();
        assert_eq!(
            &*store.lease_current().unwrap().open_verified(0).unwrap(),
            a
        );
        assert!(store.commit(1, std::slice::from_ref(&rb)).is_err());
        assert!(store.publish(&rb, a).is_err());
        assert_eq!(store.generation(), 3);
        let before = store.reservations().unwrap().active_quarantine;
        drop(lease);
        assert_eq!(store.reservations().unwrap().active_quarantine, before);
        drop(store);
        let mut store = EncryptedRuleSetStore::open(File::open(&path).unwrap(), key).unwrap();
        assert_eq!(store.generation(), 3);
        let file = path.join(Descriptor::from_ref(&rb).name());
        std::fs::write(&file, b"corrupt").unwrap();
        assert!(store.commit(3, std::slice::from_ref(&rb)).is_err());
        assert_eq!(store.generation(), 3);
        drop(store);
        for e in std::fs::read_dir(&path).unwrap() {
            std::fs::remove_file(e.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(path).unwrap();
    }
}

#[cfg(test)]
mod refusal_tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, symlink};
    #[test]
    fn nofollow_and_physical_cap_refuse_without_commit() {
        let path = std::env::temp_dir().join(format!("yonik-store-refusal-{}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let mut store = EncryptedRuleSetStore::open(File::open(&path).unwrap(), [4; 32]).unwrap();
        let body = b"fixture";
        let reference = RuleSetRef::new(
            Sha256::digest(body).into(),
            body.len() as u64,
            1,
            RuleSetFormat::SingSourceV3,
        )
        .unwrap();
        let name = Descriptor::from_ref(&reference).name();
        symlink("absent", path.join(&name)).unwrap();
        assert!(store.publish(&reference, body).is_err());
        assert_eq!(store.generation(), 0);
        std::fs::remove_file(path.join(name)).unwrap();
        let padding = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path.join("padding"))
            .unwrap();
        padding.set_len(DISK_CAP).unwrap();
        assert!(store.publish(&reference, body).is_err());
        assert_eq!(store.generation(), 0);
        drop(padding);
        std::fs::remove_file(path.join("padding")).unwrap();
        drop(store);
        std::fs::remove_dir(path).unwrap();
    }
}

#[cfg(test)]
mod idempotency_tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    #[test]
    fn existing_resource_needs_no_new_disk_reservation() {
        let path =
            std::env::temp_dir().join(format!("yonik-store-idempotent-{}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let mut store = EncryptedRuleSetStore::open(File::open(&path).unwrap(), [7; 32]).unwrap();
        let body = b"already-published";
        let reference = RuleSetRef::new(
            Sha256::digest(body).into(),
            body.len() as u64,
            1,
            RuleSetFormat::SingSourceV3,
        )
        .unwrap();
        store.publish(&reference, body).unwrap();
        let padding = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path.join("padding"))
            .unwrap();
        padding.set_len(DISK_CAP - reference.bytes() - 40).unwrap();
        let result = store.publish(&reference, body);
        drop(padding);
        drop(store);
        for e in std::fs::read_dir(&path).unwrap() {
            std::fs::remove_file(e.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(path).unwrap();
        assert!(
            result.is_ok(),
            "Повторная публикация verified digest не занимает дополнительное место"
        );
    }
}

#[cfg(test)]
mod inventory_boundary_tests {
    use super::*;
    use std::os::unix::fs::DirBuilderExt;

    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            let mut nonce = [0; 8];
            getrandom::getrandom(&mut nonce).expect("nonce");
            let path = std::env::temp_dir().join(format!(
                "yonik-inventory-{}-{:016x}",
                std::process::id(),
                u64::from_le_bytes(nonce)
            ));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .expect("свой каталог");
            Self(path)
        }
        fn open(&self) -> EncryptedRuleSetStore {
            EncryptedRuleSetStore::open(File::open(&self.0).expect("каталог"), [29; 32])
                .expect("reopen принятой операции")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            if let Err(error) = std::fs::remove_dir_all(&self.0) {
                // Не скрываем ошибку fixture, но не вызываем двойную панику при RED.
                eprintln!("Не удалось убрать тестовый каталог: {error}");
            }
        }
    }
    fn resource(index: usize) -> (RuleSetRef, Vec<u8>) {
        let body =
            format!(r#"{{"version":3,"rules":[{{"domain":["s{index}.test"]}}]}}"#).into_bytes();
        let reference = RuleSetRef::new(
            Sha256::digest(&body).into(),
            body.len() as u64,
            1,
            RuleSetFormat::SingSourceV3,
        )
        .expect("ref");
        (reference, body)
    }
    #[test]
    fn last_store_owner_releases_lock_despite_inherited_description() {
        let directory = Directory::new();
        let store = directory.open();
        // try_clone сохраняет тот же OFD, как fork до закрытия CLOEXEC при exec.
        let inherited = store.inner.root.try_clone().expect("унаследованный OFD");
        let custody = store.inner.clone();
        drop(store);
        assert!(
            EncryptedRuleSetStore::open(File::open(&directory.0).expect("каталог"), [29; 32])
                .is_err()
        );
        drop(custody);
        let reopened = directory.open();
        assert!(inherited.metadata().is_ok());
        drop(reopened);
        drop(inherited);
    }
    #[test]
    fn inventory_file_capacity_refuses_129th_and_reopens_every_accepted_write() {
        let directory = Directory::new();
        for index in 0..129 {
            let mut store = directory.open();
            let (reference, body) = resource(index);
            let result = store.publish(&reference, &body);
            if index == 128 {
                assert!(
                    result.is_err(),
                    "129-й постоянный файл должен быть отклонён"
                );
            } else {
                result.expect("публикация в пределах cap");
            }
            assert!(
                store.reservations().is_ok(),
                "успех не повреждает inventory"
            );
            drop(store);
            drop(directory.open());
        }
        assert_eq!(
            std::fs::read_dir(&directory.0).expect("inventory").count(),
            128
        );
    }
    #[test]
    fn inventory_file_capacity_first_state_refusal_preserves_resources() {
        let directory = Directory::new();
        let mut store = directory.open();
        let mut first = None;
        for index in 0..128 {
            let (reference, body) = resource(index);
            store.publish(&reference, &body).expect("ресурс");
            if index == 0 {
                first = Some(reference);
            }
        }
        assert!(
            store.commit(0, &[first.expect("первый ref")]).is_err(),
            "state не должен стать 129-м файлом"
        );
        assert!(!directory.0.join("state").exists());
        assert_eq!(
            std::fs::read_dir(&directory.0).expect("inventory").count(),
            128
        );
        drop(store);
        let reopened = directory.open();
        assert_eq!(reopened.generation(), 0);
        assert!(reopened.reservations().is_ok());
    }
}
