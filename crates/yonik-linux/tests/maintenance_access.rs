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

#[tokio::test]
async fn cancelled_copy_preserves_source_and_does_not_publish() {
    let directory =
        std::env::temp_dir().join(format!("yonik-maintenance-cancel-{}", std::process::id()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .expect("fixture");
    let path = directory.join("source.sqlite");
    let destination = directory.join("new.sqlite");
    let repository = EncryptedSubscriptionRepository::open(path.clone(), Zeroizing::new([47; 32]))
        .await
        .expect("БД");
    let access = repository.maintenance_access().await.expect("lease");
    repository.close().await.expect("закрытие");
    let before = fs::read(&path).expect("исходник");
    let control = yonik_linux::MaintenanceControl::default();
    control.cancel();
    assert_eq!(
        access.copy_closed(None, destination.clone(), control).await,
        Err(yonik_linux::MaintenanceError::Cancelled)
    );
    assert!(!destination.exists());
    assert_eq!(fs::read(&path).expect("исходник"), before);
    let control = yonik_linux::MaintenanceControl::default();
    assert_eq!(
        access
            .copy_closed(None, destination.clone(), control.clone())
            .await,
        Ok(())
    );
    control.cancel();
    assert!(destination.exists());
    drop(access);
    for entry in fs::read_dir(&directory).expect("fixture") {
        fs::remove_file(entry.expect("файл").path()).expect("очистка");
    }
    fs::remove_dir(directory).expect("очистка");
}
