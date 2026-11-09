//! Зашифрованное хранилище подписок. Ключ предоставляется вызывающей стороной.

use std::fs::{self, DirBuilder, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

const SMALL_LIMIT: usize = 32 * 1024;
const BODY_LIMIT: usize = 8 * 1024 * 1024;
const SOURCE_LIMIT: i64 = 20_000;
static OPEN_GATE: Semaphore = Semaphore::const_new(2);
const SENTINEL: &[u8] = b"tofu subscription repository v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryError {
    InvalidInput,
    UnsafePath,
    UnsupportedVersion,
    Corrupt,
    Storage,
    Conflict,
    NotFound,
}
impl std::fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "Некорректные данные хранилища",
            Self::UnsafePath => "Небезопасный путь хранилища",
            Self::UnsupportedVersion => "Неподдерживаемая версия хранилища",
            Self::Corrupt => "Не удалось проверить зашифрованное хранилище",
            Self::Storage => "Ошибка хранилища",
            Self::Conflict => "Запись была изменена",
            Self::NotFound => "Запись не найдена",
        })
    }
}
impl std::error::Error for RepositoryError {}
type Result<T> = std::result::Result<T, RepositoryError>;

// Секретные поля намеренно не реализуют Debug.
pub struct SourceRecord {
    pub id: i64,
    pub generation: i64,
    pub payload: Zeroizing<Vec<u8>>,
}
pub struct SnapshotInput {
    pub body: Vec<u8>,
    pub metadata: Vec<u8>,
    pub validators: Option<Vec<u8>>,
}
pub struct StoredSnapshot {
    pub revision: i64,
    pub source_generation: i64,
    pub body: Zeroizing<Vec<u8>>,
    pub metadata: Zeroizing<Vec<u8>>,
    pub validators: Option<Zeroizing<Vec<u8>>>,
}
pub struct StoredSubscription {
    pub source: SourceRecord,
    pub snapshot: Option<StoredSnapshot>,
}

#[derive(Clone)]
pub struct EncryptedSubscriptionRepository {
    inner: Arc<Mutex<Database>>,
    gate: Arc<Semaphore>,
}
struct Database {
    connection: Connection,
    key: Zeroizing<[u8; 32]>,
    id: [u8; 32],
}

impl EncryptedSubscriptionRepository {
    pub async fn open(path: PathBuf, key: Zeroizing<[u8; 32]>) -> Result<Self> {
        let permit = OPEN_GATE
            .acquire()
            .await
            .map_err(|_| RepositoryError::Storage)?;
        let database = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            Database::open(&path, key)
        })
        .await
        .map_err(|_| RepositoryError::Storage)??;
        Ok(Self {
            inner: Arc::new(Mutex::new(database)),
            gate: Arc::new(Semaphore::new(1)),
        })
    }

    /// Закрывает этот handle на blocking pool. Последний handle в GUI следует
    /// освобождать этим методом: SQLite при закрытии может checkpoint-ить WAL.
    pub async fn close(self) -> Result<()> {
        let permit = self
            .gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| RepositoryError::Storage)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            drop(self);
        })
        .await
        .map_err(|_| RepositoryError::Storage)
    }

    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Database) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let permit = self
            .gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| RepositoryError::Storage)?;
        let database = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut database = database.lock().map_err(|_| RepositoryError::Storage)?;
            operation(&mut database)
        })
        .await
        .map_err(|_| RepositoryError::Storage)?
    }

    pub async fn create_source(&self, payload: Vec<u8>) -> Result<SourceRecord> {
        let payload = Zeroizing::new(payload);
        check_small(&payload)?;
        self.run(move |db| {
            let tx = db
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            let count: i64 = tx
                .query_row("SELECT count(*) FROM sources", [], |r| r.get(0))
                .map_err(storage)?;
            if count >= SOURCE_LIMIT {
                return Err(RepositoryError::InvalidInput);
            }
            let next: i64 = tx
                .query_row("SELECT COALESCE(MAX(id),0) FROM sources", [], |r| r.get(0))
                .map_err(storage)?;
            let id = next.checked_add(1).ok_or(RepositoryError::InvalidInput)?;
            let encrypted = encrypt(&db.key, &db.id, 1, id, 1, 0, &payload)?;
            tx.execute(
                "INSERT INTO sources(id,generation,payload) VALUES (?1,1,?2)",
                params![id, encrypted],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(SourceRecord {
                id,
                generation: 1,
                payload,
            })
        })
        .await
    }

    pub async fn change_source(
        &self,
        id: i64,
        expected_generation: i64,
        payload: Vec<u8>,
    ) -> Result<SourceRecord> {
        positive(id)?;
        positive(expected_generation)?;
        let generation = expected_generation
            .checked_add(1)
            .ok_or(RepositoryError::InvalidInput)?;
        let payload = Zeroizing::new(payload);
        check_small(&payload)?;
        self.run(move |db| {
            let encrypted = encrypt(&db.key, &db.id, 1, id, generation, 0, &payload)?;
            let tx = db
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            let changed = tx
                .execute(
                    "UPDATE sources SET generation=?1,payload=?2 WHERE id=?3 AND generation=?4",
                    params![generation, encrypted, id, expected_generation],
                )
                .map_err(storage)?;
            if changed != 1 {
                return Err(RepositoryError::Conflict);
            }
            tx.execute(
                "UPDATE snapshots SET validators=NULL WHERE source_id=?1",
                [id],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(SourceRecord {
                id,
                generation,
                payload,
            })
        })
        .await
    }

    pub async fn commit_snapshot(
        &self,
        id: i64,
        expected_generation: i64,
        expected_revision: i64,
        input: SnapshotInput,
    ) -> Result<i64> {
        positive(id)?;
        positive(expected_generation)?;
        if expected_revision < 0 {
            return Err(RepositoryError::InvalidInput);
        }
        let revision = expected_revision
            .checked_add(1)
            .ok_or(RepositoryError::InvalidInput)?;
        let body = Zeroizing::new(input.body);
        let metadata = Zeroizing::new(input.metadata);
        let validators = input.validators.map(Zeroizing::new);
        if body.len() > BODY_LIMIT {
            return Err(RepositoryError::InvalidInput);
        }
        check_small(&metadata)?;
        if let Some(value) = &validators {
            check_small(value)?;
        }
        self.run(move |db| {
            let body = encrypt(&db.key, &db.id, 2, id, expected_generation, revision, &body)?;
            let metadata = encrypt(&db.key, &db.id, 3, id, expected_generation, revision, &metadata)?;
            let validators = validators.as_ref().map(|v| encrypt(&db.key, &db.id, 4, id, expected_generation, revision, v)).transpose()?;
            let tx = db.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let generation: Option<i64> = tx.query_row("SELECT generation FROM sources WHERE id=?1", [id], |r| r.get(0)).optional().map_err(storage)?;
            let previous: i64 = tx.query_row("SELECT revision FROM snapshots WHERE source_id=?1", [id], |r| r.get(0)).optional().map_err(storage)?.unwrap_or(0);
            if generation != Some(expected_generation) || previous != expected_revision { return Err(RepositoryError::Conflict); }
            tx.execute("INSERT INTO snapshots(source_id,source_generation,revision,body,metadata,validators) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(source_id) DO UPDATE SET source_generation=excluded.source_generation,revision=excluded.revision,body=excluded.body,metadata=excluded.metadata,validators=excluded.validators", params![id, expected_generation, revision, body, metadata, validators]).map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(revision)
        }).await
    }

    pub async fn list_ids(&self) -> Result<Vec<i64>> {
        self.run(|db| {
            let mut statement = db
                .connection
                .prepare("SELECT id FROM sources ORDER BY id LIMIT 20001")
                .map_err(storage)?;
            let ids = statement
                .query_map([], |r| r.get::<_, i64>(0))
                .map_err(storage)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(storage)?;
            if ids.len() > SOURCE_LIMIT as usize || ids.iter().any(|id| *id <= 0) {
                return Err(RepositoryError::Corrupt);
            }
            Ok(ids)
        })
        .await
    }

    pub async fn load(&self, id: i64) -> Result<Option<StoredSubscription>> {
        positive(id)?;
        self.run(move |db| {
            // Транзакция фиксирует согласованную пару source/snapshot при втором соединении.
            let tx = db.connection.transaction().map_err(storage)?;
            let source: Option<(i64, Vec<u8>)> = tx.query_row("SELECT generation,payload FROM sources WHERE id=?1", [id], |r| Ok((r.get(0)?, bounded_blob(r, 1, SMALL_LIMIT + 40)?))).optional().map_err(|_| RepositoryError::Corrupt)?;
            let Some((generation, payload)) = source else { return Ok(None); };
            if generation <= 0 { return Err(RepositoryError::Corrupt); }
            let payload = decrypt(&db.key, &db.id, 1, id, generation, 0, &payload, SMALL_LIMIT)?;
            type SnapshotRow = (i64, i64, Vec<u8>, Vec<u8>, Option<Vec<u8>>);
            let snapshot: Option<SnapshotRow> = tx.query_row("SELECT source_generation,revision,body,metadata,validators FROM snapshots WHERE source_id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,bounded_blob(r, 2, BODY_LIMIT + 40)?,bounded_blob(r, 3, SMALL_LIMIT + 40)?,optional_blob(r, 4, SMALL_LIMIT + 40)?))).optional().map_err(|_| RepositoryError::Corrupt)?;
            let snapshot = snapshot.map(|(source_generation, revision, body, metadata, validators)| {
                if source_generation <= 0 || source_generation > generation || revision <= 0 { return Err(RepositoryError::Corrupt); }
                Ok(StoredSnapshot {
                    source_generation, revision,
                    body: decrypt(&db.key, &db.id, 2, id, source_generation, revision, &body, BODY_LIMIT)?,
                    metadata: decrypt(&db.key, &db.id, 3, id, source_generation, revision, &metadata, SMALL_LIMIT)?,
                    validators: validators.map(|v| decrypt(&db.key, &db.id, 4, id, source_generation, revision, &v, SMALL_LIMIT)).transpose()?,
                })
            }).transpose()?;
            Ok(Some(StoredSubscription { source: SourceRecord { id, generation, payload }, snapshot }))
        }).await
    }
}

impl Database {
    fn open(path: &Path, key: Zeroizing<[u8; 32]>) -> Result<Self> {
        secure_path(path)?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let mut connection = Connection::open_with_flags(path, flags).map_err(storage)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(3))
            .map_err(storage)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)?;
        if version > 1 {
            return Err(RepositoryError::UnsupportedVersion);
        }
        if version < 0 {
            return Err(RepositoryError::Corrupt);
        }
        let id = if version == 0 {
            let tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            let tables: i64 = tx
                .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
                .map_err(storage)?;
            if tables != 0 {
                return Err(RepositoryError::Corrupt);
            }
            let mut id = [0_u8; 32];
            getrandom::getrandom(&mut id).map_err(|_| RepositoryError::Storage)?;
            let sentinel = encrypt(&key, &id, 0, 0, 0, 0, SENTINEL)?;
            tx.execute_batch("CREATE TABLE repository(id BLOB NOT NULL,sentinel BLOB NOT NULL); CREATE TABLE sources(id INTEGER PRIMARY KEY CHECK(id>0),generation INTEGER NOT NULL CHECK(generation>0),payload BLOB NOT NULL); CREATE TABLE snapshots(source_id INTEGER PRIMARY KEY REFERENCES sources(id),source_generation INTEGER NOT NULL CHECK(source_generation>0),revision INTEGER NOT NULL CHECK(revision>0),body BLOB NOT NULL,metadata BLOB NOT NULL,validators BLOB); PRAGMA user_version=1;").map_err(storage)?;
            tx.execute(
                "INSERT INTO repository(id,sentinel) VALUES (?1,?2)",
                params![id.as_slice(), sentinel],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            id
        } else {
            let count: i64 = connection
                .query_row("SELECT count(*) FROM repository", [], |r| r.get(0))
                .map_err(|_| RepositoryError::Corrupt)?;
            if count != 1 {
                return Err(RepositoryError::Corrupt);
            }
            let (id, sentinel): (Vec<u8>, Vec<u8>) = connection
                .query_row("SELECT id,sentinel FROM repository", [], |r| {
                    Ok((
                        bounded_blob(r, 0, 32)?,
                        bounded_blob(r, 1, SMALL_LIMIT + 40)?,
                    ))
                })
                .map_err(|_| RepositoryError::Corrupt)?;
            let id: [u8; 32] = id.try_into().map_err(|_| RepositoryError::Corrupt)?;
            if decrypt(&key, &id, 0, 0, 0, 0, &sentinel, SMALL_LIMIT)?.as_slice() != SENTINEL {
                return Err(RepositoryError::Corrupt);
            }
            id
        };
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA trusted_schema=OFF;").map_err(storage)?;
        Ok(Self {
            connection,
            key,
            id,
        })
    }
}

fn secure_path(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(RepositoryError::UnsafePath);
    }
    let parent = path.parent().ok_or(RepositoryError::UnsafePath)?;
    if !parent.exists() {
        DirBuilder::new()
            .mode(0o700)
            .create(parent)
            .map_err(|_| RepositoryError::UnsafePath)?;
    }
    // canonicalize выявляет симлинки и в предках, а не только в последнем компоненте.
    if parent
        .canonicalize()
        .map_err(|_| RepositoryError::UnsafePath)?
        != parent
    {
        return Err(RepositoryError::UnsafePath);
    }
    let meta = fs::symlink_metadata(parent).map_err(|_| RepositoryError::UnsafePath)?;
    let uid = rustix::process::getuid().as_raw();
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o7777 != 0o700 {
        return Err(RepositoryError::UnsafePath);
    }
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_file()
                || meta.uid() != uid
                || meta.mode() & 0o7777 != 0o600
                || meta.nlink() != 1
            {
                return Err(RepositoryError::UnsafePath);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
                .open(path)
                .map_err(|_| RepositoryError::UnsafePath)?;
        }
        Err(_) => return Err(RepositoryError::UnsafePath),
    }
    // SQLite использует эти имена самостоятельно. Не разрешаем готовые симлинки.
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        match fs::symlink_metadata(PathBuf::from(name)) {
            Ok(meta)
                if !meta.is_file()
                    || meta.uid() != uid
                    || meta.mode() & 0o077 != 0
                    || meta.nlink() != 1 =>
            {
                return Err(RepositoryError::UnsafePath);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(RepositoryError::UnsafePath),
        }
    }
    Ok(())
}
fn bounded_blob(row: &rusqlite::Row<'_>, index: usize, limit: usize) -> rusqlite::Result<Vec<u8>> {
    match row.get_ref(index)? {
        rusqlite::types::ValueRef::Blob(value) if value.len() <= limit => Ok(value.to_vec()),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
fn optional_blob(
    row: &rusqlite::Row<'_>,
    index: usize,
    limit: usize,
) -> rusqlite::Result<Option<Vec<u8>>> {
    if row.get_ref(index)? == rusqlite::types::ValueRef::Null {
        return Ok(None);
    }
    bounded_blob(row, index, limit).map(Some)
}
fn storage(_: rusqlite::Error) -> RepositoryError {
    RepositoryError::Storage
}
fn positive(value: i64) -> Result<()> {
    if value > 0 {
        Ok(())
    } else {
        Err(RepositoryError::InvalidInput)
    }
}
fn check_small(value: &[u8]) -> Result<()> {
    if value.len() <= SMALL_LIMIT {
        Ok(())
    } else {
        Err(RepositoryError::InvalidInput)
    }
}
fn aad(db: &[u8; 32], kind: u8, id: i64, generation: i64, revision: i64) -> [u8; 57] {
    let mut value = [0_u8; 57];
    value[..32].copy_from_slice(db);
    value[32] = kind;
    value[33..41].copy_from_slice(&id.to_le_bytes());
    value[41..49].copy_from_slice(&generation.to_le_bytes());
    value[49..].copy_from_slice(&revision.to_le_bytes());
    value
}
fn encrypt(
    key: &[u8; 32],
    db: &[u8; 32],
    kind: u8,
    id: i64,
    generation: i64,
    revision: i64,
    value: &[u8],
) -> Result<Vec<u8>> {
    let mut nonce = [0_u8; 24];
    getrandom::getrandom(&mut nonce).map_err(|_| RepositoryError::Storage)?;
    let encrypted = XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: value,
                aad: &aad(db, kind, id, generation, revision),
            },
        )
        .map_err(|_| RepositoryError::Corrupt)?;
    let mut result = Vec::with_capacity(24 + encrypted.len());
    result.extend_from_slice(&nonce);
    result.extend_from_slice(&encrypted);
    Ok(result)
}
#[allow(clippy::too_many_arguments)]
fn decrypt(
    key: &[u8; 32],
    db: &[u8; 32],
    kind: u8,
    id: i64,
    generation: i64,
    revision: i64,
    value: &[u8],
    limit: usize,
) -> Result<Zeroizing<Vec<u8>>> {
    if value.len() < 40 || value.len() > limit + 40 {
        return Err(RepositoryError::Corrupt);
    }
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(&value[..24]),
            Payload {
                msg: &value[24..],
                aad: &aad(db, kind, id, generation, revision),
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| RepositoryError::Corrupt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            let root = std::env::temp_dir();
            let path = root.join(format!(
                "tofu-repository-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .expect("приватный каталог теста");
            Self(path)
        }
        fn database(&self) -> PathBuf {
            self.0.join("subscriptions.sqlite")
        }
        async fn open(&self) -> EncryptedSubscriptionRepository {
            EncryptedSubscriptionRepository::open(self.database(), Zeroizing::new([7; 32]))
                .await
                .expect("открытие")
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("очистка своего каталога теста");
        }
    }
    fn snapshot() -> SnapshotInput {
        SnapshotInput {
            body: b"secret-snapshot-marker".to_vec(),
            metadata: b"secret-metadata-marker".to_vec(),
            validators: Some(b"secret-validator-marker".to_vec()),
        }
    }

    #[tokio::test]
    async fn reopen_and_no_plaintext() {
        let dir = TestDirectory::new();
        let db = dir.open().await;
        let source = db
            .create_source(b"secret-source-marker".to_vec())
            .await
            .expect("источник");
        assert_eq!(db.commit_snapshot(source.id, 1, 0, snapshot()).await, Ok(1));
        for entry in fs::read_dir(&dir.0).expect("файлы") {
            let bytes = fs::read(entry.expect("файл").path()).expect("данные");
            for marker in [
                b"secret-source-marker".as_slice(),
                b"secret-snapshot-marker",
                b"secret-metadata-marker",
                b"secret-validator-marker",
            ] {
                assert!(!bytes.windows(marker.len()).any(|v| v == marker));
            }
        }
        db.close().await.expect("закрытие");
        let reopened = dir.open().await;
        let value = reopened
            .load(source.id)
            .await
            .expect("чтение")
            .expect("запись");
        assert_eq!(value.source.payload.as_slice(), b"secret-source-marker");
        let value = value.snapshot.expect("снимок");
        assert_eq!(value.body.as_slice(), b"secret-snapshot-marker");
        assert_eq!(
            value.validators.expect("валидаторы").as_slice(),
            b"secret-validator-marker"
        );
    }

    #[tokio::test]
    async fn source_change_preserves_snapshot_and_cas_across_instances() {
        let dir = TestDirectory::new();
        let first = dir.open().await;
        let source = first
            .create_source(b"old".to_vec())
            .await
            .expect("источник");
        let second = dir.open().await;
        first
            .commit_snapshot(source.id, 1, 0, snapshot())
            .await
            .expect("снимок");
        assert_eq!(
            second.commit_snapshot(source.id, 1, 0, snapshot()).await,
            Err(RepositoryError::Conflict)
        );
        second
            .change_source(source.id, 1, b"new".to_vec())
            .await
            .expect("изменение");
        assert!(matches!(
            first.change_source(source.id, 1, b"stale".to_vec()).await,
            Err(RepositoryError::Conflict)
        ));
        assert_eq!(
            first.commit_snapshot(source.id, 1, 1, snapshot()).await,
            Err(RepositoryError::Conflict)
        );
        let value = first
            .load(source.id)
            .await
            .expect("чтение")
            .expect("запись");
        assert_eq!(value.source.generation, 2);
        assert_eq!(value.source.payload.as_slice(), b"new");
        let value = value.snapshot.expect("старый снимок");
        assert_eq!(value.revision, 1);
        assert_eq!(value.source_generation, 1);
        assert_eq!(value.body.as_slice(), b"secret-snapshot-marker");
        assert!(value.validators.is_none());
        assert_eq!(
            first.commit_snapshot(source.id, 2, 1, snapshot()).await,
            Ok(2)
        );
    }

    #[tokio::test]
    async fn wrong_key_and_tampering_are_rejected() {
        let dir = TestDirectory::new();
        let db = dir.open().await;
        let source = db
            .create_source(b"source".to_vec())
            .await
            .expect("источник");
        db.commit_snapshot(source.id, 1, 0, snapshot())
            .await
            .expect("снимок");
        db.close().await.expect("закрытие");
        assert!(matches!(
            EncryptedSubscriptionRepository::open(dir.database(), Zeroizing::new([8; 32])).await,
            Err(RepositoryError::Corrupt)
        ));
        let connection = Connection::open(dir.database()).expect("тестовое соединение");
        connection
            .execute("UPDATE snapshots SET revision=2", [])
            .expect("повреждение AAD");
        drop(connection);
        let db = dir.open().await;
        assert!(matches!(
            db.load(source.id).await,
            Err(RepositoryError::Corrupt)
        ));
        db.close().await.expect("закрытие");
        let connection = Connection::open(dir.database()).expect("тестовое соединение");
        connection
            .execute("UPDATE repository SET sentinel=zeroblob(64)", [])
            .expect("повреждение sentinel");
        drop(connection);
        assert!(matches!(
            EncryptedSubscriptionRepository::open(dir.database(), Zeroizing::new([7; 32])).await,
            Err(RepositoryError::Corrupt)
        ));
    }

    #[tokio::test]
    async fn future_version_is_not_modified() {
        let dir = TestDirectory::new();
        drop(dir.open().await);
        assert!(matches!(
            EncryptedSubscriptionRepository::open(dir.database(), Zeroizing::new([8; 32])).await,
            Err(RepositoryError::Corrupt)
        ));
        let connection = Connection::open(dir.database()).expect("тестовое соединение");
        connection
            .execute_batch("PRAGMA journal_mode=DELETE; PRAGMA user_version=2;")
            .expect("будущая версия");
        drop(connection);
        let before = fs::read(dir.database()).expect("данные");
        assert!(matches!(
            EncryptedSubscriptionRepository::open(dir.database(), Zeroizing::new([7; 32])).await,
            Err(RepositoryError::UnsupportedVersion)
        ));
        assert_eq!(before, fs::read(dir.database()).expect("данные"));
        assert!(!dir.0.join("subscriptions.sqlite-wal").exists());
    }

    #[tokio::test]
    async fn symlinks_permissions_and_limits() {
        let dir = TestDirectory::new();
        let db = dir.open().await;
        assert!(matches!(
            db.create_source(vec![1; SMALL_LIMIT + 1]).await,
            Err(RepositoryError::InvalidInput)
        ));
        let source = db.create_source(vec![]).await.expect("источник");
        assert_eq!(db.list_ids().await.expect("список"), vec![source.id]);
        assert!(matches!(
            db.load(0).await,
            Err(RepositoryError::InvalidInput)
        ));
        assert!(matches!(
            db.change_source(source.id, i64::MAX, vec![]).await,
            Err(RepositoryError::InvalidInput)
        ));
        let mut input = snapshot();
        input.body = vec![0; BODY_LIMIT + 1];
        assert_eq!(
            db.commit_snapshot(source.id, 1, 0, input).await,
            Err(RepositoryError::InvalidInput)
        );
        db.close().await.expect("закрытие");
        let link = dir.0.join("link.sqlite");
        std::os::unix::fs::symlink(dir.database(), &link).expect("симлинк");
        assert!(matches!(
            EncryptedSubscriptionRepository::open(link, Zeroizing::new([7; 32])).await,
            Err(RepositoryError::UnsafePath)
        ));
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.database(), fs::Permissions::from_mode(0o644)).expect("права");
        assert!(matches!(
            EncryptedSubscriptionRepository::open(dir.database(), Zeroizing::new([7; 32])).await,
            Err(RepositoryError::UnsafePath)
        ));
    }
}
