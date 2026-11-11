//! Приватное immutable-хранилище пресетов с атомарным active/previous.
//! SHA из источника подтверждает целостность, но не подлинность издателя.
use crate::{FetchHost, FetchResponse, FetchRoute, Source};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use yonik_engine::presets::{Candidate, MAX_LIST_BYTES, valid_token};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub url: String,
    pub revision: String,
    pub sha256: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub source_id: String,
    pub version: String,
    /// Единственный поддерживаемый формат: plain-domain-cidr-v1.
    pub format: String,
    pub checked_at_unix: u64,
    pub enabled: bool,
    pub domains: Resource,
    pub ips: Resource,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    metadata: Metadata,
    domains: Vec<u8>,
    ips: Vec<u8>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    active: Option<String>,
    previous: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Fetch,
    Storage,
    Busy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    Durable,
    PublishedDurabilityUnknown,
}

pub struct Store {
    root: PathBuf,
}
impl Store {
    /// Каталог должен принадлежать приложению; существующий небезопасный каталог отвергается.
    pub fn open(root: &Path) -> Result<Self, Error> {
        match fs::DirBuilder::new().mode(0o700).create(root) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(_) => return Err(Error::Storage),
        }
        let meta = fs::symlink_metadata(root).map_err(|_| Error::Storage)?;
        if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
            return Err(Error::Storage);
        }
        let store = Self {
            root: root.to_owned(),
        };
        store.read_index()?;
        Ok(store)
    }
    pub fn active(&self) -> Result<Option<(Metadata, Candidate)>, Error> {
        self.read_index()?
            .active
            .map(|id| {
                self.load(&id).and_then(|s| {
                    let c = validate(&s)?;
                    Ok((s.metadata, c))
                })
            })
            .transpose()
    }
    /// Explicit local разрешает loopback/LAN HTTP только по выбору вызывающего кода.
    pub async fn update(
        &self,
        host: &FetchHost,
        metadata: Metadata,
        allow_local: bool,
        cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<PublishOutcome, Error> {
        validate_metadata(&metadata)?;
        let mut bodies = Vec::with_capacity(2);
        for resource in [&metadata.domains, &metadata.ips] {
            let source = if allow_local {
                Source::local(&resource.url)
            } else {
                Source::public(&resource.url)
            }
            .map_err(|_| Error::Invalid)?;
            match host
                .fetch(&source, FetchRoute::Direct, None, cancel.clone())
                .await
                .map_err(|_| Error::Fetch)?
            {
                FetchResponse::Modified { body, .. } => bodies.push(body.into_bytes()),
                _ => return Err(Error::Fetch),
            }
        }
        let ips = bodies.pop().ok_or(Error::Invalid)?;
        let domains = bodies.pop().ok_or(Error::Invalid)?;
        if *cancel.borrow() || cancel.has_changed().is_err() {
            return Err(Error::Fetch);
        }
        self.install(metadata, domains, ips)
    }
    pub fn install(
        &self,
        metadata: Metadata,
        domains: Vec<u8>,
        ips: Vec<u8>,
    ) -> Result<PublishOutcome, Error> {
        let snapshot = Snapshot {
            metadata,
            domains,
            ips,
        };
        validate(&snapshot)?;
        let _lock = self.lock()?;
        let bytes = serde_json::to_vec(&snapshot).map_err(|_| Error::Invalid)?;
        let id = hash(&bytes);
        let target = self.root.join(&id);
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).map_err(|_| Error::Storage)?;
        let staging = self.root.join(format!("stage-{}", hash(&random)));
        let stored = (|| {
            write_new(&staging, &bytes)?;
            match fs::hard_link(&staging, &target) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    self.load(&id).map(|_| ())
                }
                Err(_) => Err(Error::Storage),
            }
        })();
        let cleanup = fs::remove_file(&staging);
        stored?;
        cleanup.map_err(|_| Error::Storage)?;
        File::open(&self.root)
            .and_then(|f| f.sync_all())
            .map_err(|_| Error::Storage)?;
        let old = self.read_index()?;
        if old.active.as_ref() == Some(&id) {
            return Ok(PublishOutcome::Durable);
        }
        self.publish(&Index {
            active: Some(id),
            previous: self
                .healthy_pointer(old.active)
                .or_else(|| self.healthy_pointer(old.previous)),
        })
    }
    pub fn rollback(&self) -> Result<PublishOutcome, Error> {
        let _lock = self.lock()?;
        let old = self.read_index()?;
        let previous = old.previous.ok_or(Error::Invalid)?;
        self.load(&previous)?;
        self.publish(&Index {
            active: Some(previous),
            previous: self.healthy_pointer(old.active),
        })
    }
    fn healthy_pointer(&self, id: Option<String>) -> Option<String> {
        // Повреждённая версия не должна вытеснять исправную резервную копию.
        id.filter(|id| self.load(id).is_ok())
    }
    fn load(&self, id: &str) -> Result<Snapshot, Error> {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid);
        }
        let bytes = read_bounded(&self.root.join(id), 40 * 1024 * 1024)?;
        if hash(&bytes) != id {
            return Err(Error::Invalid);
        }
        let snapshot = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
        validate(&snapshot)?;
        Ok(snapshot)
    }
    fn read_index(&self) -> Result<Index, Error> {
        let path = self.root.join("index");
        if !path.try_exists().map_err(|_| Error::Storage)? {
            return Ok(Index::default());
        }
        let index: Index =
            serde_json::from_slice(&read_bounded(&path, 1024)?).map_err(|_| Error::Invalid)?;
        for id in [&index.active, &index.previous].into_iter().flatten() {
            if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error::Invalid);
            }
        }
        Ok(index)
    }
    fn lock(&self) -> Result<File, Error> {
        let file = File::open(&self.root).map_err(|_| Error::Storage)?;
        rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| Error::Busy)?;
        Ok(file)
    }
    fn publish(&self, index: &Index) -> Result<PublishOutcome, Error> {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).map_err(|_| Error::Storage)?;
        let staging = self.root.join(format!("stage-{}", hash(&random)));
        let result = (|| {
            write_new(
                &staging,
                &serde_json::to_vec(index).map_err(|_| Error::Invalid)?,
            )?;
            fs::rename(&staging, self.root.join("index")).map_err(|_| Error::Storage)?;
            Ok(
                if File::open(&self.root).and_then(|f| f.sync_all()).is_ok() {
                    PublishOutcome::Durable
                } else {
                    PublishOutcome::PublishedDurabilityUnknown
                },
            )
        })();
        if staging.exists() {
            let _ = fs::remove_file(staging);
        }
        result
    }
}
fn validate_metadata(m: &Metadata) -> Result<(), Error> {
    if !valid_token(&m.source_id)
        || !valid_token(&m.version)
        || m.format != "plain-domain-cidr-v1"
        || m.checked_at_unix == 0
        || m.domains.revision != m.version
        || m.ips.revision != m.version
    {
        return Err(Error::Invalid);
    }
    for r in [&m.domains, &m.ips] {
        // Метаданные не принимают credentials/query: безопасны для UI и хранения.
        let source = Source::local(&r.url).map_err(|_| Error::Invalid)?;
        if source.expose_url().contains('?')
            || r.sha256.len() != 64
            || !r
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}
fn validate(s: &Snapshot) -> Result<Candidate, Error> {
    validate_metadata(&s.metadata)?;
    if s.domains.len() > MAX_LIST_BYTES
        || s.ips.len() > MAX_LIST_BYTES
        || hash(&s.domains) != s.metadata.domains.sha256
        || hash(&s.ips) != s.metadata.ips.sha256
    {
        return Err(Error::Invalid);
    }
    Candidate::parse(
        &s.metadata.domains.revision,
        &s.metadata.ips.revision,
        &s.domains,
        &s.ips,
    )
    .map_err(|_| Error::Invalid)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| Error::Storage)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| Error::Storage)
}
fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, Error> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .map_err(|_| Error::Storage)?;
    if !file.metadata().map_err(|_| Error::Storage)?.is_file() {
        return Err(Error::Storage);
    }
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Storage)?;
    if bytes.len() as u64 > max {
        return Err(Error::Invalid);
    }
    Ok(bytes)
}
