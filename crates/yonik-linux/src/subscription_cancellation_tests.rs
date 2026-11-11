//! Actual cancel сервиса при SQLite write-lock без сети и keyring.
use super::*;
use rusqlite::Connection;
use std::{fs, os::unix::fs::DirBuilderExt, path::PathBuf};
use zeroize::Zeroizing;

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[tokio::test]
async fn cancellation_service_commit_acknowledges_locked_write() {
    let dir = Directory(std::env::temp_dir().join(format!(
        "yonik-service-cancel-{}-{}",
        std::process::id(),
        CONTROLLERS.fetch_add(1, Ordering::Relaxed)
    )));
    fs::DirBuilder::new().mode(0o700).create(&dir.0).unwrap();
    let path = dir.0.join("db");
    let repo = EncryptedSubscriptionRepository::open(path.clone(), Zeroizing::new([9; 32]))
        .await
        .unwrap();
    let service = SubscriptionService::new(repo.clone());
    let source = SubscriptionSource::new(
        "Тест".into(),
        "https://fixture.invalid/sub".into(),
        false,
        FetchRoute::Direct,
    )
    .unwrap();
    let validators =
        encode_validators(&FetchValidators::new(&source.validate().unwrap(), None, None).unwrap())
            .unwrap();
    let id = service.add(source).await.unwrap();
    let old = b"trojan://old@fixture.invalid:443#old".to_vec();
    repo.commit_snapshot(
        id,
        1,
        0,
        SnapshotInput {
            body: old.clone(),
            metadata: b"{}".to_vec(),
            validators: None,
        },
    )
    .await
    .unwrap();
    let mut controller = Subscription::new(NonZeroU64::MIN);
    let ticket = controller.begin().unwrap();
    let staged = controller
        .stage(ticket, b"trojan://new@new.invalid:8443#new".to_vec())
        .unwrap();
    let (sender, receiver) = watch::channel(false);
    let ack = Arc::new(Semaphore::new(1));
    service.requests.lock().await.insert(
        id,
        Request {
            serial: 100,
            cancellation: sender,
            ack: ack.clone(),
        },
    );
    let pending = PendingRefresh {
        service: service.identity.clone(),
        request: 100,
        id,
        generation: 1,
        revision: 1,
        validators,
        controller,
        staged,
    };
    let approved = pending.accept_clean().unwrap();
    let lock = Connection::open(path).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let blocked = Arc::new(tokio::sync::Notify::new());
    repo.observe_write_wait(blocked.clone());
    let worker = service.clone();
    let task = tokio::spawn(async move { worker.commit(approved).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while ack.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), blocked.notified())
        .await
        .unwrap();
    let cancelled = tokio::time::timeout(Duration::from_secs(1), service.cancel(id)).await;
    // Даже при ошибке deadline освобождаем lock перед assertion/JoinHandle.
    lock.execute_batch("ROLLBACK").unwrap();
    assert!(cancelled.unwrap());
    assert!(*receiver.borrow());
    assert_eq!(
        task.await.unwrap(),
        Err(SubscriptionError::Fetch(FetchError::Cancelled))
    );
    let stored = tokio::time::timeout(Duration::from_secs(1), repo.load(id))
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .snapshot
        .unwrap();
    assert_eq!(stored.revision, 1);
    assert_eq!(*stored.body, old);
    assert_eq!(ack.available_permits(), 1);
    // Последующая запись подтверждает освобождение repository/IMPORT gates.
    assert_eq!(
        repo.commit_snapshot(
            id,
            1,
            1,
            SnapshotInput {
                body: stored.body.to_vec(),
                metadata: b"{}".to_vec(),
                validators: None
            }
        )
        .await
        .unwrap(),
        2
    );
    service.close().await.unwrap();
    repo.close().await.unwrap();
}

struct NotModifiedHost;
impl SubscriptionFetch for NotModifiedHost {
    fn fetch<'a>(
        &'a self,
        source: &'a Source,
        _route: FetchRoute,
        _validators: Option<&'a FetchValidators>,
        _cancel: watch::Receiver<bool>,
    ) -> futures_util::future::BoxFuture<'a, Result<FetchResponse, FetchError>> {
        Box::pin(async move {
            Ok(FetchResponse::NotModified {
                validators: FetchValidators::new(source, None, None)?,
            })
        })
    }
}

#[tokio::test]
async fn cancellation_external_not_modified_rolls_back_locked_write() {
    let dir = Directory(std::env::temp_dir().join(format!(
        "yonik-304-cancel-{}-{}",
        std::process::id(),
        CONTROLLERS.fetch_add(1, Ordering::Relaxed)
    )));
    fs::DirBuilder::new().mode(0o700).create(&dir.0).unwrap();
    let path = dir.0.join("db");
    let repo = EncryptedSubscriptionRepository::open(path.clone(), Zeroizing::new([9; 32]))
        .await
        .unwrap();
    let service = SubscriptionService::new(repo.clone());
    let source = SubscriptionSource::new(
        "Тест".into(),
        "https://fixture.invalid/sub".into(),
        false,
        FetchRoute::Direct,
    )
    .unwrap();
    let id = service.add(source).await.unwrap();
    let old = b"trojan://old@fixture.invalid:443#old".to_vec();
    repo.commit_snapshot(
        id,
        1,
        0,
        SnapshotInput {
            body: old.clone(),
            metadata: b"{}".to_vec(),
            validators: None,
        },
    )
    .await
    .unwrap();
    let lock = Connection::open(path).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (sender, cancel) = watch::channel(false);
    let blocked = Arc::new(tokio::sync::Notify::new());
    repo.observe_write_wait(blocked.clone());
    let worker = service.clone();
    let task = tokio::spawn(async move {
        worker
            .refresh_with_host(id, None, &NotModifiedHost, cancel)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if service
                .requests
                .lock()
                .await
                .get(&id)
                .is_some_and(|r| r.ack.available_permits() == 0)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), blocked.notified())
        .await
        .unwrap();
    sender.send_replace(true);
    let result = tokio::time::timeout(Duration::from_secs(1), task).await;
    lock.execute_batch("ROLLBACK").unwrap();
    assert!(matches!(
        result.unwrap().unwrap(),
        Err(SubscriptionError::Fetch(FetchError::Cancelled))
    ));
    let stored = repo.load(id).await.unwrap().unwrap().snapshot.unwrap();
    assert_eq!(stored.revision, 1);
    assert_eq!(*stored.body, old);
    assert!(service.requests.lock().await.get(&id).is_none());
    service.close().await.unwrap();
    repo.close().await.unwrap();
}
