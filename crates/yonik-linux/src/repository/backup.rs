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
    validate_schema(connection, version)?;
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
        let mut stmt = connection.prepare("SELECT p.id,p.source_id,p.fingerprint,p.generation,p.revision,p.ordinal,p.present,p.payload,s.source_generation,s.revision FROM profiles p LEFT JOIN snapshots s ON s.source_id=p.source_id").map_err(storage)?;
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
            validate_profile_snapshot(
                &row,
                r.get::<_, Option<i64>>(8).map_err(storage)?.unwrap_or(0),
                r.get::<_, Option<i64>>(9).map_err(storage)?.unwrap_or(0),
            )?;
            open_profile(key, &id, &row)?;
        }
    }
    if table_exists(connection, "routing_draft")? {
        let mut stmt = connection
            .prepare("SELECT revision,payload FROM routing_draft")
            .map_err(|_| RepositoryError::Corrupt)?;
        let mut rows = stmt.query([]).map_err(storage)?;
        while let Some(row) = rows.next().map_err(storage)? {
            let revision = row.get(0).map_err(storage)?;
            let raw = bounded_blob(row, 1, 262144 + 40).map_err(storage)?;
            decrypt(key, &id, 8, 0, 0, revision, &raw, 262144)?;
        }
    }
    Ok(())
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool> {
    let kind: Option<String> = connection
        .query_row(
            "SELECT type FROM sqlite_master WHERE name=?1",
            [name],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| RepositoryError::Corrupt)?;
    match kind.as_deref() {
        None => Ok(false),
        Some("table") => Ok(true),
        _ => Err(RepositoryError::Corrupt),
    }
}

// Проверяем поля даже у пустых таблиц. Lazy catalog и routing_draft не
// создаются и не чинятся: preflight не должен изменять проверяемую БД.
fn validate_schema(connection: &Connection, version: i64) -> Result<()> {
    let mut contracts = vec![
        (
            "repository",
            "id,sentinel",
            "typeof(id)!='blob' OR typeof(sentinel)!='blob'",
        ),
        (
            "sources",
            "id,generation,payload",
            "typeof(id)!='integer' OR id<=0 OR typeof(generation)!='integer' OR generation<=0 OR typeof(payload)!='blob'",
        ),
        (
            "snapshots",
            "source_id,source_generation,revision,body,metadata,validators",
            "typeof(source_id)!='integer' OR source_id<=0 OR typeof(source_generation)!='integer' OR source_generation<=0 OR typeof(revision)!='integer' OR revision<=0 OR typeof(body)!='blob' OR typeof(metadata)!='blob' OR typeof(validators) NOT IN ('blob','null') OR NOT EXISTS(SELECT 1 FROM sources WHERE id=source_id)",
        ),
    ];
    if version >= 2 {
        contracts.extend([
            ("profiles", "id,source_id,fingerprint,generation,revision,ordinal,present,payload", "typeof(id)!='integer' OR id<=0 OR typeof(source_id)!='integer' OR NOT EXISTS(SELECT 1 FROM sources WHERE sources.id=profiles.source_id) OR typeof(fingerprint)!='blob' OR length(fingerprint)!=32 OR typeof(generation)!='integer' OR generation<=0 OR typeof(revision)!='integer' OR revision<=0 OR typeof(ordinal)!='integer' OR ordinal<0 OR ordinal>=20000 OR typeof(present)!='integer' OR present NOT IN (0,1) OR typeof(payload)!='blob'"),
            ("profile_catalog", "source_id,generation,revision,raw_count,unique_count", "typeof(source_id)!='integer' OR NOT EXISTS(SELECT 1 FROM snapshots WHERE snapshots.source_id=profile_catalog.source_id) OR typeof(generation)!='integer' OR generation<=0 OR typeof(revision)!='integer' OR revision<=0 OR typeof(raw_count)!='integer' OR raw_count<0 OR raw_count>20000 OR typeof(unique_count)!='integer' OR unique_count<0 OR unique_count>raw_count"),
            ("sqlite_sequence", "name,seq", "name='profiles' AND (typeof(seq)!='integer' OR seq<0 OR seq<(SELECT COALESCE(MAX(id),0) FROM profiles))"),
        ]);
    }
    if version >= 3 {
        contracts.push(("source_sequence", "singleton,last_id", "typeof(singleton)!='integer' OR singleton!=1 OR typeof(last_id)!='integer' OR last_id<0 OR last_id<(SELECT COALESCE(MAX(id),0) FROM sources)"));
    }
    if table_exists(connection, "routing_draft")? {
        contracts.push(("routing_draft", "singleton,revision,payload", "typeof(singleton)!='integer' OR singleton!=1 OR typeof(revision)!='integer' OR revision<=0 OR typeof(payload)!='blob'"));
    }
    for (table, columns, invalid) in contracts {
        if !table_exists(connection, table)? {
            return Err(RepositoryError::Corrupt);
        }
        // Все SQL-фрагменты — статический контракт, не пользовательский ввод.
        connection
            .prepare(&format!("SELECT {columns} FROM {table} LIMIT 0"))
            .map_err(|_| RepositoryError::Corrupt)?;
        let bad: bool = connection
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {invalid})"),
                [],
                |r| r.get(0),
            )
            .map_err(|_| RepositoryError::Corrupt)?;
        if bad {
            return Err(RepositoryError::Corrupt);
        }
    }
    let mut keys = vec![("sources", "id"), ("snapshots", "source_id")];
    if version >= 2 {
        keys.extend([("profiles", "id"), ("profile_catalog", "source_id")]);
    }
    if version >= 3 {
        keys.push(("source_sequence", "singleton"));
    }
    if table_exists(connection, "routing_draft")? {
        keys.push(("routing_draft", "singleton"));
    }
    for (table, key) in keys {
        let valid: bool = connection.query_row(
            "SELECT count(*)=1 AND COALESCE(MAX(name=?2 AND upper(type)='INTEGER'),0) FROM pragma_table_info(?1) WHERE pk>0",
            params![table,key], |r| r.get(0)
        ).map_err(|_| RepositoryError::Corrupt)?;
        if !valid {
            return Err(RepositoryError::Corrupt);
        }
    }
    if version >= 2 {
        let unique: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_index_list('profiles') i WHERE i.\"unique\"=1 AND (SELECT group_concat(name, ',') FROM pragma_index_info(i.name))='source_id,fingerprint')",
            [], |r| r.get(0)
        ).map_err(|_| RepositoryError::Corrupt)?;
        if !unique {
            return Err(RepositoryError::Corrupt);
        }
    }
    if version >= 3 {
        let count: i64 = connection
            .query_row("SELECT count(*) FROM source_sequence", [], |r| r.get(0))
            .map_err(|_| RepositoryError::Corrupt)?;
        if count != 1 {
            return Err(RepositoryError::Corrupt);
        }
    }
    if version >= 2 {
        let excessive: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM profiles GROUP BY source_id HAVING count(*)>40000)",
                [],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if excessive {
            return Err(RepositoryError::Corrupt);
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
