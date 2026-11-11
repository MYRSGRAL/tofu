//! Opaque lease проверяется только с приватной fixture и ephemeral ключом.
use std::{fs, os::unix::fs::DirBuilderExt};
use yonik_linux::EncryptedSubscriptionRepository;
use zeroize::Zeroizing;

#[tokio::test]
async fn lease_reopens_authoritative_repository_after_handles_close() {
    let directory = std::env::temp_dir().join(format!(
        "yonik-maintenance-access-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("часы fixture")
            .as_nanos()
    ));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .expect("приватный каталог");
    let path = directory.join("fixture.sqlite");
    let repository = EncryptedSubscriptionRepository::open(path.clone(), Zeroizing::new([41; 32]))
        .await
        .expect("ephemeral БД");
    let source = repository
        .create_source(b"fixture".to_vec())
        .await
        .expect("источник");
    let access = repository.maintenance_access().await.expect("opaque lease");
    assert_eq!(access.source(), path);
    repository.close().await.expect("закрытие handles");
    let reopened = access.reopen().await.expect("тот же ключ без keyring");
    assert_eq!(
        reopened
            .create_source(b"second".to_vec())
            .await
            .expect("запись после reopen")
            .id,
        source.id + 1
    );
    reopened.close().await.expect("закрытие reopen");
    drop(access);
    for entry in fs::read_dir(&directory).expect("каталог fixture") {
        fs::remove_file(entry.expect("файл fixture").path()).expect("очистка fixture");
    }
    fs::remove_dir(directory).expect("очистка каталога fixture");
}
