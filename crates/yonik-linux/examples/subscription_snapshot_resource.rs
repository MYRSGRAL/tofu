//! P-04 delta: 20 реальных snapshots в собственной ephemeral БД, без сети.
use rusqlite::{Connection, OpenFlags};
use std::{
    fmt::Write as _,
    fs,
    num::{NonZeroU16, NonZeroU64},
    path::Path,
    time::{Duration, Instant},
};
use yonik_linux::{EncryptedSubscriptionRepository, RepositoryError, SnapshotInput};
use yonik_subscription::Subscription;
use zeroize::Zeroizing;

const SOURCES: usize = 20;
const ROUNDS: usize = 4;
fn body(source: usize, count: usize, round: usize) -> Result<Vec<u8>, std::fmt::Error> {
    let mut text = String::new();
    for i in 0..count {
        writeln!(
            text,
            "trojan://fixture-407@r{round}-s{source}-{i}.example.invalid:443"
        )?;
    }
    Ok(text.into_bytes())
}
fn rss() -> Result<u64, Box<dyn std::error::Error>> {
    let status = fs::read_to_string("/proc/self/status")?;
    Ok(status
        .lines()
        .find(|s| s.starts_with("VmRSS:"))
        .ok_or("Нет RSS")?
        .split_whitespace()
        .nth(1)
        .ok_or("Нет RSS")?
        .parse()?)
}
fn inspect(path: &Path) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let snapshots: i64 = db.query_row("SELECT count(*) FROM snapshots", [], |r| r.get(0))?;
    let profiles: i64 = db.query_row("SELECT count(*) FROM profiles", [], |r| r.get(0))?;
    let bytes: i64 = db.query_row(
        "SELECT coalesce(sum(length(body)),0) FROM snapshots",
        [],
        |r| r.get(0),
    )?;
    let pages: i64 = db.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    Ok(
        serde_json::json!({"snapshots":snapshots,"profiles":profiles,"encrypted_body_bytes":bytes,"pages":pages,"rss_kib":rss()?}),
    )
}
async fn exercise(
    root: &Path,
    count: usize,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let path = root.join("snapshots.sqlite");
    let repo =
        EncryptedSubscriptionRepository::open(path.clone(), Zeroizing::new([31; 32])).await?;
    let mut controllers = Vec::new();
    let mut ids = Vec::new();
    for i in 0..SOURCES {
        ids.push(
            repo.create_source(format!("offline-{i}").into_bytes())
                .await?
                .id,
        );
        controllers.push(Subscription::new(
            NonZeroU64::new(i as u64 + 1).ok_or("identity")?,
        ));
    }
    let mut phases = Vec::new();
    let mut config_bytes = [0usize; 2];
    let mut staged = 0;
    let mut disk_imported = 0;
    let mut compiled = [0usize; 2];
    let port = NonZeroU16::new(10808).ok_or("port")?;
    let mut out = Vec::new();
    for round in 0..ROUNDS {
        let started = Instant::now();
        for (s, controller) in controllers.iter_mut().enumerate() {
            let ticket = controller.begin()?;
            let update = controller
                .stage(ticket, body(s, count / SOURCES, round)?)?
                .accept_clean()?;
            let report = update.report();
            assert_eq!(report.servers.len(), count / SOURCES);
            staged += report.servers.len();
            let input = SnapshotInput {
                body: report.original().expose().as_bytes().to_vec(),
                metadata: vec![],
                validators: None,
            };
            assert_eq!(
                repo.commit_snapshot(ids[s], 1, round as i64, input).await?,
                round as i64 + 1
            );
            controller.commit(update)?;
            // Читаем утверждённый snapshot, а не случайно оставленный staged report.
            let stored = repo
                .load(ids[s])
                .await?
                .ok_or("Нет source")?
                .snapshot
                .ok_or("Нет snapshot")?;
            assert_eq!(stored.revision, round as i64 + 1);
            let imported = yonik_import::import_bytes(stored.body.to_vec())?;
            assert_eq!(imported.servers.len(), count / SOURCES);
            assert!(imported.issues.is_empty());
            disk_imported += imported.servers.len();
            for (e, engine) in [yonik_engine::Engine::Xray, yonik_engine::Engine::SingBox]
                .into_iter()
                .enumerate()
            {
                for server in &imported.servers {
                    out.clear();
                    yonik_engine::compile(server, engine, port)?.write_sensitive(&mut out)?;
                    compiled[e] += 1;
                    config_bytes[e] += out.len();
                }
            }
        }
        let retained_servers: usize = controllers
            .iter()
            .filter_map(Subscription::snapshot)
            .map(|r| r.servers.len())
            .sum();
        let retained_source_bytes: usize = controllers
            .iter()
            .filter_map(Subscription::snapshot)
            .map(|r| r.original().expose().len())
            .sum();
        assert_eq!(retained_servers, count);
        assert!(retained_source_bytes < count * 128);
        let state = inspect(&path)?;
        assert_eq!(state["snapshots"], SOURCES);
        assert_eq!(state["profiles"], count);
        phases.push(
            serde_json::json!({"round":round,"wall_ns":started.elapsed().as_nanos(),"state":state,"retained_servers":retained_servers,"retained_source_bytes":retained_source_bytes}),
        );
    }
    // InvalidInput и CAS refusal обязаны сохранить точные предыдущие bytes/revision.
    let before = repo
        .load(ids[0])
        .await?
        .ok_or("source")?
        .snapshot
        .ok_or("snapshot")?;
    for (revision, bytes) in [
        (ROUNDS as i64, "не URI".as_bytes().to_vec()),
        (0, body(0, count / SOURCES, 99)?),
    ] {
        let result = repo
            .commit_snapshot(
                ids[0],
                1,
                revision,
                SnapshotInput {
                    body: bytes,
                    metadata: vec![],
                    validators: None,
                },
            )
            .await;
        assert!(matches!(
            result,
            Err(RepositoryError::InvalidInput | RepositoryError::Conflict)
        ));
        let after = repo
            .load(ids[0])
            .await?
            .ok_or("source")?
            .snapshot
            .ok_or("snapshot")?;
        assert_eq!(after.revision, before.revision);
        assert_eq!(*after.body, *before.body);
    }
    // Actual failing cancellation probe: abort JoinHandle не отменяет начатый spawn_blocking.
    let lock = Connection::open(&path)?;
    lock.execute_batch("BEGIN IMMEDIATE")?;
    let worker = repo.clone();
    let id = ids[0];
    let future = tokio::spawn(async move {
        worker
            .commit_snapshot(
                id,
                1,
                ROUNDS as i64,
                SnapshotInput {
                    body: body(0, count / SOURCES, 100).expect("fixture"),
                    metadata: vec![],
                    validators: None,
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let cancel_started = Instant::now();
    future.abort();
    assert!(future.await.expect_err("abort").is_cancelled());
    lock.execute_batch("ROLLBACK")?;
    drop(lock);
    // load ждёт тот же repository gate и потому наблюдает завершение blocking job.
    let after = repo
        .load(id)
        .await?
        .ok_or("source")?
        .snapshot
        .ok_or("snapshot")?;
    let continued = after.revision == ROUNDS as i64 + 1;
    assert!(
        continued,
        "Probe не достиг blocking commit: отмена не доказана"
    );
    let cancel = serde_json::json!({"aborted_future_committed":continued,"settled_ns":cancel_started.elapsed().as_nanos(),"revision":after.revision});
    drop(before);
    drop(after);
    drop(controllers);
    let final_state = inspect(&path)?;
    repo.close().await?;
    Ok(
        serde_json::json!({"sources":SOURCES,"rounds":ROUNDS,"successful_commits":SOURCES*ROUNDS,"staged_imported":staged,"disk_imported":disk_imported,"compiled":compiled,"config_bytes":config_bytes,"phases":phases,"refusal_preserved":true,"cancel_probe":cancel,"final_state":final_state}),
    )
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::DirBuilderExt;
    let count: usize = std::env::args().nth(1).ok_or("count")?.parse()?;
    if ![1000, 10000].contains(&count) {
        return Err("count вне границ".into());
    }
    let root = std::env::temp_dir().join(format!("yonik-p04-snapshots-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&root)?;
    let result = exercise(&root, count).await;
    fs::remove_dir_all(&root)?;
    println!("{}", result?);
    Ok(())
}
