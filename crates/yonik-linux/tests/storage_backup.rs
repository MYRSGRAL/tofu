use std::{fs, os::unix::fs::DirBuilderExt};
use yonik_linux::{EncryptedSubscriptionRepository as Repository, RepositoryError};
use zeroize::Zeroizing;

#[tokio::test]
async fn backup_restore_reopen_and_refuse_overwrite() {
    let dir = std::env::temp_dir().join(format!("yonik-backup-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let source = dir.join("source.sqlite");
    let backup = dir.join("backup.sqlite");
    let restored = dir.join("restored.sqlite");
    let repo = Repository::open(source.clone(), Zeroizing::new([7; 32]))
        .await
        .unwrap();
    let id = repo
        .create_source(b"secret-marker".to_vec())
        .await
        .unwrap()
        .id;
    assert!(
        Repository::backup_closed(source.clone(), backup.clone(), Zeroizing::new([7; 32]))
            .await
            .is_err()
    );
    repo.commit_snapshot(
        id,
        1,
        0,
        yonik_linux::SnapshotInput {
            body: b"socks5://fixture.invalid:1080".to_vec(),
            metadata: b"metadata".to_vec(),
            validators: Some(b"validator".to_vec()),
        },
    )
    .await
    .unwrap();
    repo.close().await.unwrap();
    let sidecar = dir.join("backup.sqlite-wal");
    fs::write(&sidecar, b"foreign-sidecar").unwrap();
    assert_eq!(
        Repository::backup_closed(source.clone(), backup.clone(), Zeroizing::new([7; 32])).await,
        Err(RepositoryError::Conflict)
    );
    fs::remove_file(sidecar).unwrap();
    Repository::backup_closed(source.clone(), backup.clone(), Zeroizing::new([7; 32]))
        .await
        .unwrap();
    assert!(
        !fs::read(&backup)
            .unwrap()
            .windows(13)
            .any(|v| v == b"secret-marker")
    );
    assert!(
        Repository::restore_closed(backup.clone(), restored.clone(), Zeroizing::new([8; 32]))
            .await
            .is_err()
    );
    assert!(!restored.exists());
    Repository::restore_closed(backup.clone(), restored.clone(), Zeroizing::new([7; 32]))
        .await
        .unwrap();
    assert_eq!(
        Repository::restore_closed(backup.clone(), restored.clone(), Zeroizing::new([7; 32])).await,
        Err(RepositoryError::Conflict)
    );
    let repo = Repository::open(restored.clone(), Zeroizing::new([7; 32]))
        .await
        .unwrap();
    assert_eq!(
        repo.load(id)
            .await
            .unwrap()
            .unwrap()
            .source
            .payload
            .as_slice(),
        b"secret-marker"
    );
    repo.close().await.unwrap();
    // Повреждение ciphertext при корректной структуре SQLite.
    let connection = rusqlite::Connection::open(&backup).unwrap();
    connection
        .execute("UPDATE sources SET payload=zeroblob(length(payload))", [])
        .unwrap();
    drop(connection);
    assert!(
        Repository::restore_closed(
            backup.clone(),
            dir.join("corrupt.sqlite"),
            Zeroizing::new([7; 32])
        )
        .await
        .is_err()
    );
    assert!(!dir.join("corrupt.sqlite").exists());
    // Уже восстановленные данные пережили отказ кандидата.
    let repo = Repository::open(restored.clone(), Zeroizing::new([7; 32]))
        .await
        .unwrap();
    assert_eq!(
        repo.load(id)
            .await
            .unwrap()
            .unwrap()
            .source
            .payload
            .as_slice(),
        b"secret-marker"
    );
    repo.close().await.unwrap();
    let bytes = fs::read(&backup).unwrap();
    fs::write(&backup, &bytes[..bytes.len() / 2]).unwrap();
    assert!(
        Repository::restore_closed(backup, dir.join("bad.sqlite"), Zeroizing::new([7; 32]))
            .await
            .is_err()
    );
    assert!(!dir.join("bad.sqlite").exists());
    let connection = rusqlite::Connection::open(&source).unwrap();
    connection.execute_batch("DROP TABLE profiles; DROP TABLE profile_catalog; DROP TABLE source_sequence; PRAGMA user_version=1;").unwrap();
    let sentinel: Vec<u8> = connection
        .query_row("SELECT sentinel FROM repository", [], |r| r.get(0))
        .unwrap();
    drop(connection);
    let legacy = dir.join("legacy.sqlite");
    Repository::backup_closed(source, legacy.clone(), Zeroizing::new([7; 32]))
        .await
        .unwrap();
    let connection = rusqlite::Connection::open(&legacy).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("SELECT sentinel FROM repository", [], |r| r
                .get::<_, Vec<u8>>(0))
            .unwrap(),
        sentinel
    );
    drop(connection);
    let repo = Repository::open(legacy, Zeroizing::new([7; 32]))
        .await
        .unwrap();
    assert_eq!(
        repo.load(id)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .unwrap()
            .revision,
        1
    );
    repo.close().await.unwrap();
    fs::remove_dir_all(dir).unwrap();
}
