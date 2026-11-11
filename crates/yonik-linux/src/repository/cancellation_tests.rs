use super::*;

use std::time::Duration;

#[tokio::test]
async fn cancellation_write_lock_acknowledges_before_release() {
    let dir = TestDirectory::new();
    let repo = dir.open().await;
    let source = repo.create_source(b"source".to_vec()).await.unwrap().id;
    repo.commit_snapshot(source, 1, 0, snapshot())
        .await
        .unwrap();
    let lock = Connection::open(dir.database()).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (sender, cancel) = crate::Cancellation::new();
    let blocked = Arc::new(tokio::sync::Notify::new());
    repo.observe_write_wait(blocked.clone());
    let worker = repo.clone();
    let mut task = tokio::spawn(async move {
        worker
            .commit_snapshot_cancellable(source, 1, 1, snapshot(), cancel)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while repo.gate.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), blocked.notified())
        .await
        .unwrap();
    sender.send_replace(true);
    let ack = tokio::time::timeout(Duration::from_secs(1), &mut task).await;
    lock.execute_batch("ROLLBACK").unwrap();
    let (on_time, result) = match ack {
        Ok(result) => (true, result.unwrap()),
        Err(_) => (false, task.await.unwrap()),
    };
    assert!(
        on_time,
        "worker не подтвердил отмену до снятия write-lock: {result:?}"
    );
    assert_eq!(result, Err(SnapshotCommitError::Cancelled));
    assert_original_snapshot(&repo, source).await;
    assert_eq!(repo.gate.available_permits(), 1);
    repo.close().await.unwrap();
}

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "yonik-cancel-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
    fn database(&self) -> PathBuf {
        self.0.join("subscriptions.sqlite")
    }
    async fn open(&self) -> EncryptedSubscriptionRepository {
        EncryptedSubscriptionRepository::open(self.database(), Zeroizing::new([7; 32]))
            .await
            .unwrap()
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn snapshot() -> SnapshotInput {
    SnapshotInput {
        body: b"trojan://secret@fixture.invalid:443#test".to_vec(),
        metadata: b"metadata".to_vec(),
        validators: None,
    }
}
async fn assert_original_snapshot(repo: &EncryptedSubscriptionRepository, source: i64) {
    let stored = repo.load(source).await.unwrap().unwrap().snapshot.unwrap();
    assert_eq!(stored.revision, 1);
    assert_eq!(*stored.body, snapshot().body);
    assert_eq!(*stored.metadata, snapshot().metadata);
}

#[tokio::test]
async fn cancellation_commit_boundary_has_two_linearizable_outcomes() {
    for after_admission in [false, true] {
        let dir = TestDirectory::new();
        let repo = dir.open().await;
        let source = repo.create_source(b"source".to_vec()).await.unwrap().id;
        repo.commit_snapshot(source, 1, 0, snapshot())
            .await
            .unwrap();
        let (reached, mut boundary) = tokio::sync::mpsc::unbounded_channel();
        let (release, resume) = std::sync::mpsc::channel();
        let resume = Mutex::new(resume);
        *repo.commit_boundary.lock().unwrap() = Some(Box::new(move |admitted| {
            if admitted
                == if after_admission {
                    CommitCheckpoint::Admitted
                } else {
                    CommitCheckpoint::BeforeAdmission
                }
            {
                reached.send(()).unwrap();
                resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap();
            }
        }));
        let (sender, cancel) = crate::Cancellation::new();
        let worker = repo.clone();
        let task = tokio::spawn(async move {
            let mut replacement = snapshot();
            replacement.body = b"trojan://changed@new.invalid:8443#new".to_vec();
            worker
                .commit_snapshot_cancellable(source, 1, 1, replacement, cancel)
                .await
        });
        tokio::time::timeout(Duration::from_secs(3), boundary.recv())
            .await
            .unwrap()
            .unwrap();
        sender.send_replace(true);
        release.send(()).unwrap();
        let result = task.await.unwrap();
        let stored = repo.load(source).await.unwrap().unwrap().snapshot.unwrap();
        if after_admission {
            assert_eq!(result, Ok(2));
            assert_eq!(stored.revision, 2);
            assert_ne!(*stored.body, snapshot().body);
        } else {
            assert_eq!(result, Err(SnapshotCommitError::Cancelled));
            assert_original_snapshot(&repo, source).await;
        }
        assert_eq!(repo.gate.available_permits(), 1);
        repo.close().await.unwrap();
    }
}
