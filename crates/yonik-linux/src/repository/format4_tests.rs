//! Только synthetic encrypted fixtures; production authority — проверенный backup.
use super::*;
use std::os::unix::fs::DirBuilderExt;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yonik-format4-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
    async fn open(&self) -> EncryptedSubscriptionRepository {
        EncryptedSubscriptionRepository::open(self.0.join("live"), Zeroizing::new([37; 32]))
            .await
            .unwrap()
    }
    async fn backup(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        EncryptedSubscriptionRepository::backup_closed(
            self.0.join("live"),
            path.clone(),
            Zeroizing::new([37; 32]),
        )
        .await
        .unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
const V1: &[u8] = br#"{"version":1,"rules":[],"default_action":"proxy"}"#;
const V2: &[u8] = br#"{"version":2,"rules":[],"default_action":"proxy"}"#;
async fn version(repo: &EncryptedSubscriptionRepository) -> i64 {
    repo.run(|db| {
        db.connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn routing_format4_requires_backup_cas_and_rolls_back_fault() {
    let f = Fixture::new();
    let repo = f.open().await;
    assert!(repo.routing_draft().await.unwrap().is_none());
    repo.save_routing_draft(0, V1.to_vec()).await.unwrap();
    assert_eq!(
        repo.save_routing_draft(1, V2.to_vec()).await,
        Err(RepositoryError::InvalidInput)
    );
    assert_eq!(
        repo.save_routing_draft_v2(1, V2.to_vec(), None).await,
        Err(RepositoryError::InvalidInput)
    );
    assert!(
        repo.save_routing_draft_v2(1, V2.to_vec(), Some(f.0.join("live")))
            .await
            .is_err()
    );
    repo.close().await.unwrap();
    let backup = f.backup("v3").await;
    let repo = f.open().await;
    assert_eq!(
        repo.save_routing_draft_v2(0, V2.to_vec(), Some(backup.clone()))
            .await,
        Err(RepositoryError::Conflict)
    );
    repo.run(|db| {
        db.fail_format4 = true;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        repo.save_routing_draft_v2(1, V2.to_vec(), Some(backup.clone()))
            .await,
        Err(RepositoryError::Storage)
    );
    assert_eq!(version(&repo).await, 3);
    let (revision, bytes) = repo.routing_draft().await.unwrap().unwrap();
    assert_eq!(revision, 1);
    assert_eq!(bytes.as_slice(), V1);
    repo.run(|db| {
        db.fail_format4 = false;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        repo.save_routing_draft_v2(1, V2.to_vec(), Some(backup))
            .await
            .unwrap(),
        2
    );
    assert_eq!(version(&repo).await, 4);
    repo.close().await.unwrap();
    let repo = f.open().await;
    assert_eq!(version(&repo).await, 4);
    assert_eq!(
        repo.routing_draft().await.unwrap().unwrap().1.as_slice(),
        V2
    );
    repo.save_routing_draft(2, V1.to_vec()).await.unwrap();
    assert_eq!(version(&repo).await, 4);
    repo.close().await.unwrap();
    let backup4 = f.backup("v4").await;
    let restored = f.0.join("restored");
    EncryptedSubscriptionRepository::restore_closed(
        backup4,
        restored.clone(),
        Zeroizing::new([37; 32]),
    )
    .await
    .unwrap();
    let repo = EncryptedSubscriptionRepository::open(restored, Zeroizing::new([37; 32]))
        .await
        .unwrap();
    assert_eq!(version(&repo).await, 4);
    repo.close().await.unwrap();
}
#[tokio::test]
async fn source_v2_upgrade_and_stale_backup_refusal() {
    let f = Fixture::new();
    let repo = f.open().await;
    repo.close().await.unwrap();
    let backup = f.backup("v3").await;
    let repo = f.open().await;
    let prior = repo.create_source(b"v1".to_vec()).await.unwrap();
    assert!(
        repo.change_source(prior.id, prior.generation, V2.to_vec())
            .await
            .is_err()
    );
    assert!(
        repo.update_source_metadata(
            prior.id,
            prior.generation,
            Zeroizing::new(b"v1".to_vec()),
            V2.to_vec()
        )
        .await
        .is_err()
    );
    assert!(matches!(
        repo.create_source_v2(V2.to_vec(), Some(backup)).await,
        Err(RepositoryError::Conflict)
    ));
    assert_eq!(version(&repo).await, 3);
    repo.close().await.unwrap();
    let fresh = f.backup("fresh3").await;
    let repo = f.open().await;
    assert!(repo.create_source(V2.to_vec()).await.is_err());
    repo.run(|db| {
        db.fail_format4 = true;
        Ok(())
    })
    .await
    .unwrap();
    assert!(
        repo.create_source_v2(V2.to_vec(), Some(fresh.clone()))
            .await
            .is_err()
    );
    assert_eq!(version(&repo).await, 3);
    repo.run(|db| {
        db.fail_format4 = false;
        Ok(())
    })
    .await
    .unwrap();
    let source = repo
        .create_source_v2(V2.to_vec(), Some(fresh))
        .await
        .unwrap();
    assert_eq!(source.id, 2);
    assert_eq!(version(&repo).await, 4);
    repo.create_source_v2(V2.to_vec(), None).await.unwrap();
    repo.close().await.unwrap();
}

#[tokio::test]
async fn local_snapshot_atomic_cancel_fault_and_success() {
    let f = Fixture::new();
    let repo = f.open().await;
    repo.close().await.unwrap();
    let backup = f.backup("v3").await;
    let repo = f.open().await;
    let original = b"vless://00112233-4455-6677-8899-aabbccddeeff@127.0.0.1:24443".to_vec();
    let (sender, cancel) = crate::Cancellation::new();
    sender.send_replace(true);
    assert!(
        repo.create_local_snapshot(V2.to_vec(), original.clone(), cancel, Some(backup.clone()))
            .await
            .is_err()
    );
    assert_eq!(version(&repo).await, 3);
    assert_eq!(repo.source_records(0, 10).await.unwrap().total, 0);
    let (_sender, cancel) = crate::Cancellation::new();
    repo.run(|db| {
        db.fail_format4 = true;
        Ok(())
    })
    .await
    .unwrap();
    assert!(
        repo.create_local_snapshot(V2.to_vec(), original.clone(), cancel, Some(backup.clone()))
            .await
            .is_err()
    );
    assert_eq!(version(&repo).await, 3);
    assert_eq!(repo.source_records(0, 10).await.unwrap().total, 0);
    repo.run(|db| {
        db.fail_format4 = false;
        Ok(())
    })
    .await
    .unwrap();
    let (_sender, cancel) = crate::Cancellation::new();
    let (late_sender, late_cancel) = crate::Cancellation::new();
    *repo.commit_boundary.lock().unwrap() = Some(Box::new(move |phase| {
        if phase == CommitCheckpoint::BeforeAdmission {
            late_sender.send_replace(true);
        }
    }));
    assert!(
        repo.create_local_snapshot(
            V2.to_vec(),
            original.clone(),
            late_cancel,
            Some(backup.clone())
        )
        .await
        .is_err()
    );
    assert_eq!(version(&repo).await, 3);
    assert_eq!(repo.source_records(0, 10).await.unwrap().total, 0);
    *repo.commit_boundary.lock().unwrap() = None;
    let source = repo
        .create_local_snapshot(V2.to_vec(), original.clone(), cancel, Some(backup))
        .await
        .unwrap();
    assert_eq!(source.id, 1);
    assert_eq!(version(&repo).await, 4);
    let loaded = repo.load(source.id).await.unwrap().unwrap();
    let snapshot = loaded.snapshot.unwrap();
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.body.as_slice(), original);
    let profiles = repo
        .profiles(source.id, ProfileQuery::default())
        .await
        .unwrap();
    assert_eq!(profiles.snapshot_revision, 1);
    assert_eq!(profiles.total, 1);
    repo.close().await.unwrap();
    let repo = f.open().await;
    assert_eq!(version(&repo).await, 4);
    assert_eq!(repo.source_records(0, 10).await.unwrap().total, 1);
    repo.close().await.unwrap();
}
