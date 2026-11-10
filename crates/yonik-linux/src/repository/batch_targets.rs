//! Одна проверенная проекция snapshot для массовой TCP-диагностики.
use super::{
    BODY_LIMIT, Database, EncryptedSubscriptionRepository, RepositoryError, Result, ServerProfile,
    TransactionBehavior, bounded_blob, decrypt, fingerprint, open_profile, positive, read_profiles,
    storage,
};
use crate::{Cancellation, TcpProbeTarget};
use rusqlite::OptionalExtension;

/// Содержит только публичные поля и TCP endpoint; ParsedServer наружу не выходит.
pub struct BatchTargetProjection {
    pub profile: ServerProfile,
    pub target: TcpProbeTarget,
}
pub struct BatchTargetSnapshot {
    pub targets: Vec<BatchTargetProjection>,
    pub source_generation: i64,
    pub snapshot_generation: i64,
    pub revision: i64,
}
impl EncryptedSubscriptionRepository {
    /// None означает отмену. Подготовка выполняется на существующем blocking pool.
    /// Каталог должен быть предварительно открыт через profiles; ленивое создание
    /// здесь потребовало бы второго разбора snapshot и ослабило бы expected CAS.
    pub async fn batch_targets(
        &self,
        source: i64,
        expected_source_generation: i64,
        expected_revision: i64,
        cancel: Cancellation,
    ) -> Result<Option<BatchTargetSnapshot>> {
        positive(source)?;
        positive(expected_source_generation)?;
        positive(expected_revision)?;
        if cancel.check().is_err() {
            return Ok(None);
        }
        let mut observer = cancel.clone();
        let operation = self.run(move |db| {
            prepare_batch(
                db,
                source,
                expected_source_generation,
                expected_revision,
                &cancel,
            )
        });
        tokio::select! {
            biased;
            _ = observer.cancelled() => Ok(None),
            result = operation => result,
        }
    }
}

fn prepare_batch(
    db: &mut Database,
    source: i64,
    expected_source_generation: i64,
    expected_revision: i64,
    cancel: &Cancellation,
) -> Result<Option<BatchTargetSnapshot>> {
    if cancel.check().is_err() {
        return Ok(None);
    }
    let tx = db
        .connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(storage)?;
    let source_generation: i64 = tx
        .query_row(
            "SELECT generation FROM sources WHERE id=?1",
            [source],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?
        .ok_or(RepositoryError::NotFound)?;
    if source_generation != expected_source_generation {
        return Err(RepositoryError::Conflict);
    }
    let (generation, revision, body): (i64, i64, Vec<u8>) = tx
        .query_row(
            "SELECT source_generation,revision,body FROM snapshots WHERE source_id=?1",
            [source],
            |r| Ok((r.get(0)?, r.get(1)?, bounded_blob(r, 2, BODY_LIMIT + 40)?)),
        )
        .optional()
        .map_err(storage)?
        .ok_or(RepositoryError::NotFound)?;
    if revision != expected_revision {
        return Err(RepositoryError::Conflict);
    }
    if generation <= 0 || revision <= 0 {
        return Err(RepositoryError::Corrupt);
    }
    let marker: Option<(i64,i64,usize,usize)> = tx.query_row("SELECT generation,revision,raw_count,unique_count FROM profile_catalog WHERE source_id=?1",[source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(storage)?;
    let Some((catalog_generation, catalog_revision, raw, unique)) = marker else {
        return Err(RepositoryError::Conflict);
    };
    if catalog_generation != generation || catalog_revision != revision {
        return Err(RepositoryError::Conflict);
    }
    if raw > yonik_import::MAX_RECORDS || unique > raw {
        return Err(RepositoryError::Corrupt);
    }
    let count: usize = tx
        .query_row(
            "SELECT count(*) FROM profiles WHERE source_id=?1",
            [source],
            |r| r.get(0),
        )
        .map_err(storage)?;
    if count > 10_000 {
        return Err(RepositoryError::InvalidInput);
    }
    if cancel.check().is_err() {
        return Ok(None);
    }
    let body = decrypt(
        &db.key, &db.id, 2, source, generation, revision, &body, BODY_LIMIT,
    )?;
    let report = yonik_import::import_bytes(body.to_vec()).map_err(|_| RepositoryError::Corrupt)?;
    if cancel.check().is_err() {
        return Ok(None);
    }
    let mut targets = Vec::with_capacity(count);
    for row in read_profiles(&tx, source)? {
        if cancel.check().is_err() {
            return Ok(None);
        }
        let profile = open_profile(&db.key, &db.id, &row)?;
        if !profile.available {
            return Err(RepositoryError::NotFound);
        }
        if row.generation != generation || row.revision != revision {
            return Err(RepositoryError::Corrupt);
        }
        let server = report
            .servers
            .get(profile.ordinal)
            .ok_or(RepositoryError::Corrupt)?;
        if fingerprint(&db.key, &db.id, source, server)? != row.fingerprint {
            return Err(RepositoryError::Corrupt);
        }
        targets.push(BatchTargetProjection {
            profile,
            target: TcpProbeTarget::for_server(server),
        });
    }
    if cancel.check().is_err() {
        return Ok(None);
    }
    tx.commit().map_err(storage)?;
    Ok(Some(BatchTargetSnapshot {
        targets,
        source_generation,
        snapshot_generation: generation,
        revision,
    }))
}

#[cfg(test)]
mod tests {
    use super::super::{ProfileQuery, SnapshotInput};
    use super::*;
    use std::{
        fs,
        os::unix::fs::DirBuilderExt,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };
    use tokio::{io::AsyncReadExt, net::TcpListener};
    use zeroize::Zeroizing;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("свой каталог");
        }
    }
    async fn fixture(port: u16) -> (Directory, EncryptedSubscriptionRepository, i64) {
        let dir = Directory(std::env::temp_dir().join(format!(
            "yonik-bulk-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir.0)
            .expect("каталог");
        let repo =
            EncryptedSubscriptionRepository::open(dir.0.join("db"), Zeroizing::new([91; 32]))
                .await
                .expect("БД");
        let source = repo
            .create_source(b"fixture".to_vec())
            .await
            .expect("источник");
        repo.commit_snapshot(
            source.id,
            1,
            0,
            SnapshotInput {
                body: format!("trojan://fixture-secret@127.0.0.1:{port}#Тест").into_bytes(),
                metadata: vec![],
                validators: None,
            },
        )
        .await
        .expect("snapshot");
        repo.profiles(source.id, ProfileQuery::default())
            .await
            .expect("каталог");
        (dir, repo, source.id)
    }
    #[tokio::test]
    async fn bulk_matches_single_resolve_and_enforces_cas_and_cancellation() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("TCP fixture");
        let port = listener.local_addr().expect("адрес").port();
        let server = tokio::spawn(async move {
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().await.expect("accept");
                assert_eq!(stream.read(&mut [0]).await.expect("EOF"), 0);
            }
        });
        let (_dir, repo, source) = fixture(port).await;
        let (keep, cancel) = Cancellation::new();
        let bulk = repo
            .batch_targets(source, 1, 1, cancel)
            .await
            .expect("bulk")
            .expect("не отменено");
        assert_eq!(bulk.targets.len(), 1);
        let row = bulk.targets.into_iter().next().expect("цель");
        let single = repo
            .resolve_profile(row.profile.id, 1)
            .await
            .expect("single");
        assert_eq!(row.profile.source_id, single.profile.source_id);
        assert_eq!(row.profile.name, single.profile.name);
        assert_eq!(row.profile.host, single.profile.host);
        assert_eq!(row.profile.port, single.profile.port);
        assert_eq!(bulk.source_generation, single.source_generation);
        assert_eq!(bulk.snapshot_generation, single.snapshot_generation);
        assert_eq!(bulk.revision, single.snapshot_revision);
        let (_keep, mut cancellation) = Cancellation::new();
        let first = crate::check_tcp_series(row.target, Duration::from_secs(2), &mut cancellation)
            .await
            .expect("bulk TCP");
        let second = crate::check_tcp_series(
            TcpProbeTarget::for_server(&single.server),
            Duration::from_secs(2),
            &mut cancellation,
        )
        .await
        .expect("single TCP");
        assert_eq!(first.status, crate::TcpProbeStatus::Reachable);
        assert_eq!(first.status, second.status);
        assert_eq!(first.selected_address, second.selected_address);
        assert_eq!(first.samples.len(), 3);
        assert_eq!(second.samples.len(), 3);
        server.await.expect("fixture");
        for (generation, revision) in [(2, 1), (1, 2)] {
            let (_sender, cancel) = Cancellation::new();
            assert!(matches!(
                repo.batch_targets(source, generation, revision, cancel)
                    .await,
                Err(RepositoryError::Conflict)
            ));
        }
        let (sender, cancel) = Cancellation::new();
        sender.send_replace(true);
        assert!(
            repo.batch_targets(source, 1, 1, cancel)
                .await
                .expect("cancel")
                .is_none()
        );
        repo.commit_snapshot(
            source,
            1,
            1,
            SnapshotInput {
                body: format!("trojan://fixture-secret@127.0.0.1:{port}#Тест").into_bytes(),
                metadata: vec![],
                validators: None,
            },
        )
        .await
        .expect("следующий snapshot");
        let (_sender, cancel) = Cancellation::new();
        assert!(matches!(
            repo.batch_targets(source, 1, 1, cancel).await,
            Err(RepositoryError::Conflict)
        ));
        assert!(matches!(
            repo.resolve_profile(row.profile.id, 1).await,
            Err(RepositoryError::Conflict)
        ));
        drop(keep);
    }
    #[tokio::test]
    async fn bulk_ten_thousand_projections_use_one_snapshot_pass() {
        let (_dir, repo, _source) = fixture(1).await;
        let fresh = repo
            .create_source(b"fresh".to_vec())
            .await
            .expect("источник");
        let body = (0..10_000)
            .map(|id| format!("trojan://synthetic-{id}@127.0.0.1:1#Цель{id}\n"))
            .collect::<String>();
        repo.commit_snapshot(
            fresh.id,
            1,
            0,
            SnapshotInput {
                body: body.into_bytes(),
                metadata: vec![],
                validators: None,
            },
        )
        .await
        .expect("snapshot");
        repo.profiles(fresh.id, ProfileQuery::default())
            .await
            .expect("каталог");
        let started = std::time::Instant::now();
        let (_sender, cancel) = Cancellation::new();
        let batch = repo
            .batch_targets(fresh.id, 1, 1, cancel)
            .await
            .expect("bulk")
            .expect("результат");
        assert_eq!(batch.targets.len(), 10_000);
        let ids: std::collections::HashSet<_> =
            batch.targets.iter().map(|row| row.profile.id).collect();
        assert_eq!(ids.len(), 10_000);
        eprintln!(
            "BULK_10K targets={} elapsed_ms={} projection_struct_bytes={}",
            batch.targets.len(),
            started.elapsed().as_millis(),
            std::mem::size_of::<BatchTargetProjection>()
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }
    #[tokio::test]
    async fn queued_bulk_cancellation_does_not_wait_for_database_gate() {
        let (_dir, repo, source) = fixture(1).await;
        let permit = repo.gate.clone().acquire_owned().await.expect("gate");
        let (sender, cancel) = Cancellation::new();
        let worker = repo.clone();
        let pending = tokio::spawn(async move { worker.batch_targets(source, 1, 1, cancel).await });
        tokio::task::yield_now().await;
        sender.send_replace(true);
        let result = tokio::time::timeout(Duration::from_millis(100), pending)
            .await
            .expect("отмена не ждёт БД")
            .expect("задача")
            .expect("cancel");
        assert!(result.is_none());
        drop(permit);
        let (_sender, cancel) = Cancellation::new();
        assert!(
            repo.batch_targets(source, 1, 1, cancel)
                .await
                .expect("повтор")
                .is_some()
        );
    }
    #[tokio::test]
    async fn bulk_rejects_profile_and_snapshot_aead_corruption() {
        for table in ["profiles", "snapshots"] {
            let (_dir, repo, source) = fixture(1).await;
            let id = repo
                .profiles(source, ProfileQuery::default())
                .await
                .expect("page")
                .profiles[0]
                .id;
            repo.run(move |db| {
                let sql = if table == "profiles" {
                    "UPDATE profiles SET payload=zeroblob(length(payload))"
                } else {
                    "UPDATE snapshots SET body=zeroblob(length(body))"
                };
                db.connection.execute(sql, []).map_err(storage)?;
                Ok(())
            })
            .await
            .expect("синтетическое повреждение своей БД");
            let (_sender, cancel) = Cancellation::new();
            assert!(matches!(
                repo.batch_targets(source, 1, 1, cancel).await,
                Err(RepositoryError::Corrupt)
            ));
            assert!(matches!(
                repo.resolve_profile(id, 1).await,
                Err(RepositoryError::Corrupt)
            ));
        }
    }
}
