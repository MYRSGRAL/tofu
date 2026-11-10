//! Maintenance-копии: существующее назначение никогда не заменяется.
use super::*;

const BACKUP_LIMIT: u64 = 256 * 1024 * 1024;

impl EncryptedSubscriptionRepository {
    /// Вызывается после закрытия всех handles. Возвращает Conflict при занятой БД.
    /// Копия содержит исходные ciphertext, домены и версию; ключ в неё не входит.
    pub async fn backup_closed(
        source: PathBuf,
        destination: PathBuf,
        key: Zeroizing<[u8; 32]>,
    ) -> Result<()> {
        let permit = OPEN_GATE
            .acquire()
            .await
            .map_err(|_| RepositoryError::Storage)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            copy_closed(&source, &destination, &key)
        })
        .await
        .map_err(|_| RepositoryError::Storage)?
    }

    /// Восстановление только в новый путь. Переключение приложения на него —
    /// отдельное явное действие после закрытия текущего repository.
    pub async fn restore_closed(
        source: PathBuf,
        destination: PathBuf,
        key: Zeroizing<[u8; 32]>,
    ) -> Result<()> {
        Self::backup_closed(source, destination, key).await
    }
}

fn validate(connection: &Connection, key: &[u8; 32]) -> Result<()> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(storage)?;
    if !(1..=3).contains(&version) {
        return Err(RepositoryError::UnsupportedVersion);
    }
    let check: String = connection
        .query_row("PRAGMA integrity_check(1)", [], |r| r.get(0))
        .map_err(|_| RepositoryError::Corrupt)?;
    if check != "ok" {
        return Err(RepositoryError::Corrupt);
    }
    let count: i64 = connection
        .query_row("SELECT count(*) FROM repository", [], |r| r.get(0))
        .map_err(storage)?;
    if count != 1 {
        return Err(RepositoryError::Corrupt);
    }
    let (id, sentinel) = connection
        .query_row("SELECT id,sentinel FROM repository", [], |r| {
            Ok((
                bounded_blob(r, 0, 32)?,
                bounded_blob(r, 1, SMALL_LIMIT + 40)?,
            ))
        })
        .map_err(storage)?;
    let id: [u8; 32] = id.try_into().map_err(|_| RepositoryError::Corrupt)?;
    if decrypt(key, &id, 0, 0, 0, 0, &sentinel, SMALL_LIMIT)?.as_slice() != SENTINEL {
        return Err(RepositoryError::Corrupt);
    }
    let mut stmt = connection
        .prepare("SELECT id,generation,payload FROM sources LIMIT 20001")
        .map_err(storage)?;
    let mut rows = stmt.query([]).map_err(storage)?;
    let mut count = 0;
    while let Some(r) = rows.next().map_err(storage)? {
        count += 1;
        if count > SOURCE_LIMIT {
            return Err(RepositoryError::Corrupt);
        }
        let source = r.get(0).map_err(storage)?;
        let generation = r.get(1).map_err(storage)?;
        let payload = bounded_blob(r, 2, SMALL_LIMIT + 40).map_err(storage)?;
        decrypt(key, &id, 1, source, generation, 0, &payload, SMALL_LIMIT)?;
    }
    let mut stmt = connection
        .prepare(
            "SELECT source_id,source_generation,revision,body,metadata,validators FROM snapshots",
        )
        .map_err(storage)?;
    let mut rows = stmt.query([]).map_err(storage)?;
    while let Some(r) = rows.next().map_err(storage)? {
        let source = r.get(0).map_err(storage)?;
        let generation = r.get(1).map_err(storage)?;
        let revision = r.get(2).map_err(storage)?;
        for (index, kind, limit) in [(3, 2, BODY_LIMIT), (4, 3, SMALL_LIMIT), (5, 4, SMALL_LIMIT)] {
            if let Some(value) = optional_blob(r, index, limit + 40).map_err(storage)? {
                decrypt(key, &id, kind, source, generation, revision, &value, limit)?;
            } else if index != 5 {
                return Err(RepositoryError::Corrupt);
            }
        }
    }
    if version >= 2 {
        let mut stmt = connection.prepare("SELECT id,source_id,fingerprint,generation,revision,ordinal,present,payload FROM profiles").map_err(storage)?;
        let mut rows = stmt.query([]).map_err(storage)?;
        while let Some(r) = rows.next().map_err(storage)? {
            let row = ProfileRow {
                id: r.get(0).map_err(storage)?,
                source: r.get(1).map_err(storage)?,
                fingerprint: bounded_blob(r, 2, 32).map_err(storage)?,
                generation: r.get(3).map_err(storage)?,
                revision: r.get(4).map_err(storage)?,
                ordinal: r.get(5).map_err(storage)?,
                present: r.get(6).map_err(storage)?,
                payload: bounded_blob(r, 7, SMALL_LIMIT + 40).map_err(storage)?,
            };
            open_profile(key, &id, &row)?;
        }
    }
    Ok(())
}

fn copy_closed(source: &Path, destination: &Path, key: &[u8; 32]) -> Result<()> {
    if destination
        .try_exists()
        .map_err(|_| RepositoryError::UnsafePath)?
    {
        return Err(RepositoryError::Conflict);
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = destination.as_os_str().to_os_string();
        name.push(suffix);
        match fs::symlink_metadata(PathBuf::from(name)) {
            Ok(_) => return Err(RepositoryError::Conflict),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(RepositoryError::UnsafePath),
        }
    }
    let meta = fs::symlink_metadata(source).map_err(|_| RepositoryError::UnsafePath)?;
    if meta.len() == 0 || meta.len() > BACKUP_LIMIT {
        return Err(RepositoryError::InvalidInput);
    }
    secure_path(source)?;
    let mut total = meta.len();
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = source.as_os_str().to_os_string();
        name.push(suffix);
        match fs::metadata(PathBuf::from(name)) {
            Ok(m) => {
                total = total
                    .checked_add(m.len())
                    .ok_or(RepositoryError::InvalidInput)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(RepositoryError::Storage),
        }
    }
    if total > BACKUP_LIMIT {
        return Err(RepositoryError::InvalidInput);
    }
    // Проверка родителя назначения без резервирования самого публичного имени.
    let parent = destination.parent().ok_or(RepositoryError::UnsafePath)?;
    if !destination.is_absolute()
        || parent
            .canonicalize()
            .map_err(|_| RepositoryError::UnsafePath)?
            != parent
    {
        return Err(RepositoryError::UnsafePath);
    }
    let meta = fs::symlink_metadata(parent).map_err(|_| RepositoryError::UnsafePath)?;
    if meta.uid() != rustix::process::getuid().as_raw() || meta.mode() & 0o7777 != 0o700 {
        return Err(RepositoryError::UnsafePath);
    }
    let connection = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(storage)?;
    connection
        .busy_timeout(std::time::Duration::ZERO)
        .map_err(storage)?;
    connection
        .execute_batch(
            "PRAGMA trusted_schema=OFF; PRAGMA locking_mode=EXCLUSIVE; BEGIN EXCLUSIVE; COMMIT;",
        )
        .map_err(|_| RepositoryError::Conflict)?;
    let pages: u64 = connection
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .map_err(storage)?;
    let page_size: u64 = connection
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .map_err(storage)?;
    if pages
        .checked_mul(page_size)
        .is_none_or(|size| size > BACKUP_LIMIT)
    {
        return Err(RepositoryError::InvalidInput);
    }
    validate(&connection, key)?;
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(|_| RepositoryError::Storage)?;
    let stage = parent.join(format!(".yonik-backup-{:032x}", u128::from_le_bytes(nonce)));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&stage)
        .map_err(|_| RepositoryError::Storage)?;
    let result = (|| {
        // VACUUM INTO доступен без rusqlite backup feature; копирует ciphertext.
        connection
            .execute(
                "VACUUM INTO ?1",
                [stage.to_str().ok_or(RepositoryError::UnsafePath)?],
            )
            .map_err(storage)?;
        let file = OpenOptions::new()
            .read(true)
            .open(&stage)
            .map_err(|_| RepositoryError::Storage)?;
        if file.metadata().map_err(|_| RepositoryError::Storage)?.len() > BACKUP_LIMIT {
            return Err(RepositoryError::InvalidInput);
        }
        file.sync_all().map_err(|_| RepositoryError::Storage)?;
        // hard_link публикует атомарно и никогда не заменяет существующий файл.
        fs::hard_link(&stage, destination).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                RepositoryError::Conflict
            } else {
                RepositoryError::Storage
            }
        })?;
        Ok(())
    })();
    let cleanup = fs::remove_file(&stage).map_err(|_| RepositoryError::Storage);
    result?;
    cleanup?;
    // Ошибка здесь означает неопределённую durability уже опубликованного файла.
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|_| RepositoryError::Storage)
}
