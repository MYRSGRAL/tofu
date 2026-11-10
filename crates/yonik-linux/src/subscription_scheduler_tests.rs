//! Регрессии acknowledgement импорта и времени завершения review.
use super::*;
use crate::{SubscriptionScheduleEvent, SubscriptionScheduler};
use std::{fs, path::PathBuf, sync::Mutex as SyncMutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use yonik_subscription::scheduler::{Policy, Scheduler};
use zeroize::Zeroizing;

struct Hold {
    body: Vec<u8>,
    entered: tokio::sync::oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}
static HOLD: SyncMutex<Option<Hold>> = SyncMutex::new(None);
pub(super) fn hold_parser(body: &[u8]) {
    let hold = {
        let mut hook = HOLD.lock().unwrap();
        if hook.as_ref().is_some_and(|hold| hold.body == body) {
            hook.take()
        } else {
            None
        }
    };
    if let Some(hold) = hold {
        let _ = hold.entered.send(());
        hold.release.recv().unwrap();
    }
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
async fn service() -> (Directory, SubscriptionService) {
    let dir = Directory(std::env::temp_dir().join(format!(
        "yonik-review-{}-{}",
        std::process::id(),
        CONTROLLERS.fetch_add(1, Ordering::Relaxed)
    )));
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(&dir.0).unwrap();
    let repo = EncryptedSubscriptionRepository::open(dir.0.join("db"), Zeroizing::new([9; 32]))
        .await
        .unwrap();
    (dir, SubscriptionService::new(repo))
}
async fn source(service: &SubscriptionService, body: &'static str) -> i64 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/sub", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            assert_eq!(stream.read(&mut byte).await.unwrap(), 1);
            request.push(byte[0]);
        }
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    service
        .add(SubscriptionSource::new("Тест".into(), url, true, FetchRoute::Direct).unwrap())
        .await
        .unwrap()
}
fn policy() -> Policy {
    Policy {
        online: true,
        ui_visible: true,
        ..Policy::default()
    }
}
fn runtime() -> SubscriptionScheduler {
    SubscriptionScheduler::new(NonZeroU64::MIN, 2, 1).unwrap()
}
fn host() -> Arc<dyn SubscriptionFetch> {
    Arc::new(FetchHost::new(Duration::from_secs(2)).unwrap())
}

struct CountingHost(Arc<AtomicU64>);
impl SubscriptionFetch for CountingHost {
    fn fetch<'a>(
        &'a self,
        source: &'a Source,
        route: FetchRoute,
        validators: Option<&'a FetchValidators>,
        cancel: watch::Receiver<bool>,
    ) -> futures_util::future::BoxFuture<'a, Result<FetchResponse, FetchError>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::Relaxed);
            FetchHost::new(Duration::from_secs(2))?
                .fetch(source, route, validators, cancel)
                .await
        })
    }
}

#[tokio::test]
async fn cancel_ждёт_фактического_конца_parser_и_shutdown() {
    let (_dir, service) = service().await;
    let body = "socks5://127.0.0.1:1080#hold-regression";
    let first = source(&service, body).await;
    let second = source(&service, "socks5://127.0.0.1:1081#second").await;
    let (entered, mut started) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    *HOLD.lock().unwrap() = Some(Hold {
        body: body.as_bytes().to_vec(),
        entered,
        release: blocked,
    });
    let calls = Arc::new(AtomicU64::new(0));
    let transport: Arc<dyn SubscriptionFetch> = Arc::new(CountingHost(calls.clone()));
    let mut runtime = runtime();
    runtime
        .restore(
            first,
            1,
            SubscriptionRetryState {
                due: Some(Duration::ZERO),
                failures: 3,
            },
        )
        .unwrap();
    runtime.schedule(second, 1, Duration::ZERO).unwrap();
    runtime
        .dispatch(Duration::ZERO, policy(), &service, transport.clone())
        .unwrap();
    tokio::select! {
        result = runtime.next_event(|| Duration::ZERO) => panic!("parser ещё удерживается: {}", result.is_ok()),
        _ = &mut started => {}
    }
    runtime.cancel(first, Duration::ZERO).unwrap();
    let early = tokio::time::timeout(
        Duration::from_millis(30),
        runtime.next_event(|| Duration::ZERO),
    )
    .await;
    // Всегда освобождаем blocking thread даже на RED.
    if early.is_ok() {
        release.send(()).unwrap();
        panic!("Cancelled опубликован до завершения blocking parser");
    }
    runtime
        .dispatch(Duration::ZERO, policy(), &service, transport.clone())
        .unwrap();
    assert_eq!(runtime.next_deadline(policy()), None);
    assert!(
        tokio::time::timeout(
            Duration::from_millis(30),
            runtime.next_event(|| Duration::ZERO)
        )
        .await
        .is_err()
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "второй fetch не стартовал до release parser"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), runtime.shutdown(Duration::ZERO))
            .await
            .is_err()
    );
    release.send(()).unwrap();
    runtime.shutdown(Duration::ZERO).await.unwrap();
    assert_eq!(
        service
            .schedule_metadata(first)
            .await
            .unwrap()
            .unwrap()
            .state
            .unwrap()
            .failures,
        3
    );
    assert!(matches!(
        service.offline(first).await,
        Err(SubscriptionError::NoSnapshot)
    ));
}

#[tokio::test]
async fn review_deadline_после_commit_и_retry_отменённого_ожидания() {
    let (_dir, service) = service().await;
    let id = source(&service, "socks5://127.0.0.1:1080#clock").await;
    let mut runtime = runtime();
    runtime.schedule(id, 1, Duration::ZERO).unwrap();
    runtime
        .dispatch(Duration::ZERO, policy(), &service, host())
        .unwrap();
    assert_eq!(
        runtime.next_event(|| Duration::ZERO).await.unwrap(),
        Some(SubscriptionScheduleEvent::Review { id })
    );
    let guard = service.requests.lock().await;
    let now = std::cell::Cell::new(Duration::from_secs(10));
    assert!(
        tokio::time::timeout(
            Duration::from_millis(30),
            runtime.finish_review(&service, id, true, || now.get())
        )
        .await
        .is_err()
    );
    drop(guard);
    // При повторе нельзя заменить уже принятое approve=true на false.
    let decision = runtime.finish_review(&service, id, false, || now.get());
    now.set(Duration::from_secs(90));
    assert_eq!(decision.await.unwrap(), Some(1));
    assert_eq!(
        runtime.next_deadline(policy()),
        Some(Duration::from_secs(90) + Scheduler::INTERVAL)
    );
    runtime.shutdown(now.get()).await.unwrap();
}

#[tokio::test]
async fn review_backoff_после_задержанной_ошибки_commit() {
    let (_dir, service) = service().await;
    let id = source(&service, "socks5://127.0.0.1:1080#error-clock").await;
    let mut runtime = runtime();
    runtime.schedule(id, 1, Duration::ZERO).unwrap();
    runtime
        .dispatch(Duration::ZERO, policy(), &service, host())
        .unwrap();
    runtime.next_event(|| Duration::ZERO).await.unwrap();
    let mut guard = service.requests.lock().await;
    let now = std::cell::Cell::new(Duration::from_secs(10));
    {
        let decision = runtime.finish_review(&service, id, true, || now.get());
        tokio::pin!(decision);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), decision.as_mut())
                .await
                .is_err()
        );
        guard.remove(&id);
        drop(guard);
        now.set(Duration::from_secs(90));
        assert_eq!(decision.await, Err(SubscriptionError::StaleUpdate));
    }
    assert_eq!(
        runtime.next_deadline(policy()),
        Some(Duration::from_secs(91))
    );
    runtime.shutdown(now.get()).await.unwrap();
}

#[tokio::test]
async fn persisted_due_reopen_cas_generation_delete() {
    let (dir, service) = service().await;
    let id = service
        .add(
            SubscriptionSource::new(
                "Тест".into(),
                "http://127.0.0.1:9/sub".into(),
                true,
                FetchRoute::Direct,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let token = service.schedule_metadata(id).await.unwrap().unwrap();
    assert_eq!(token.state, None);
    let state = ScheduleState {
        version: 1,
        due_wall: Some(86500),
        recorded_wall: 100,
        failures: 4,
        paused: false,
    };
    service.persist_schedule(token, state).await.unwrap();
    let stale = service.schedule_metadata(id).await.unwrap().unwrap();
    service.rename_source(id, "Новое имя".into()).await.unwrap();
    assert!(matches!(
        service.persist_schedule(stale, state).await,
        Err(SubscriptionError::Repository(RepositoryError::Conflict))
    ));
    service.close().await.unwrap();
    let repo = EncryptedSubscriptionRepository::open(dir.0.join("db"), Zeroizing::new([9; 32]))
        .await
        .unwrap();
    let reopened = SubscriptionService::new(repo);
    let token = reopened.schedule_metadata(id).await.unwrap().unwrap();
    assert_eq!(token.generation, 1);
    assert_eq!(token.state, Some(state));
    reopened
        .change_source(
            id,
            1,
            SubscriptionSource::new(
                "Другой".into(),
                "http://127.0.0.1:9/new".into(),
                true,
                FetchRoute::Direct,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(matches!(
        reopened.persist_schedule(token, state).await,
        Err(SubscriptionError::Repository(RepositoryError::Conflict))
    ));
    assert_eq!(
        reopened.schedule_metadata(id).await.unwrap().unwrap().state,
        None
    );
    let token = reopened.schedule_metadata(id).await.unwrap().unwrap();
    reopened.delete_source(id).await.unwrap();
    assert!(reopened.persist_schedule(token, state).await.is_err());
    reopened.close().await.unwrap();
}

#[test]
fn schedule_clock_bounds() {
    let state = ScheduleState {
        version: 1,
        recorded_wall: 100,
        due_wall: Some(86500),
        failures: 0,
        paused: false,
    };
    assert_eq!(
        state.deadline(100, Duration::from_secs(2)),
        Duration::from_secs(86402)
    );
    assert_eq!(
        state.deadline(0, Duration::ZERO),
        Duration::from_secs(86400)
    );
    assert_eq!(
        state.deadline(u64::MAX, Duration::from_secs(2)),
        Duration::from_secs(2)
    );
    assert_eq!(state.deadline(0, Duration::MAX), Duration::MAX);
    for bad in [
        ScheduleState {
            version: 2,
            ..state
        },
        ScheduleState {
            failures: 6,
            ..state
        },
        ScheduleState {
            paused: true,
            ..state
        },
        ScheduleState {
            due_wall: Some(u64::MAX),
            ..state
        },
        ScheduleState {
            due_wall: Some(99),
            ..state
        },
    ] {
        assert!(bad.validate().is_err());
    }
}

#[tokio::test]
async fn completion_due_survives_reopen_for_approve_and_discard() {
    for approve in [false, true] {
        let (dir, service) = service().await;
        let id = source(&service, "socks5://127.0.0.1:1080#persisted").await;
        let wall = Arc::new(AtomicU64::new(100));
        let clock = wall.clone();
        let mut scheduler = SubscriptionScheduler::with_wall_clock(
            NonZeroU64::MIN,
            2,
            1,
            Arc::new(move || clock.load(Ordering::Relaxed)),
        )
        .unwrap();
        scheduler.schedule(id, 1, Duration::ZERO).unwrap();
        scheduler
            .dispatch(Duration::ZERO, policy(), &service, host())
            .unwrap();
        assert_eq!(
            scheduler.next_event(|| Duration::ZERO).await.unwrap(),
            Some(SubscriptionScheduleEvent::Review { id })
        );
        wall.store(200, Ordering::Relaxed);
        scheduler
            .finish_review(&service, id, approve, || Duration::from_secs(50))
            .await
            .unwrap();
        service.close().await.unwrap();
        let repo = EncryptedSubscriptionRepository::open(dir.0.join("db"), Zeroizing::new([9; 32]))
            .await
            .unwrap();
        let reopened = SubscriptionService::new(repo);
        let token = reopened.schedule_metadata(id).await.unwrap().unwrap();
        let state = token.state.unwrap();
        assert_eq!(state.due_wall, Some(86600));
        assert_eq!(state.failures, 0);
        let mut runtime = runtime();
        runtime
            .restore(
                id,
                token.generation,
                SubscriptionRetryState {
                    due: Some(state.deadline(201, Duration::ZERO)),
                    failures: state.failures,
                },
            )
            .unwrap();
        let calls = Arc::new(AtomicU64::new(0));
        runtime
            .dispatch(
                Duration::ZERO,
                policy(),
                &reopened,
                Arc::new(CountingHost(calls.clone())),
            )
            .unwrap();
        assert_eq!(runtime.next_event(|| Duration::ZERO).await.unwrap(), None);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        reopened.close().await.unwrap();
    }
}

struct ОшибкаFetch;
impl SubscriptionFetch for ОшибкаFetch {
    fn fetch<'a>(
        &'a self,
        _: &'a Source,
        _: FetchRoute,
        _: Option<&'a FetchValidators>,
        _: watch::Receiver<bool>,
    ) -> futures_util::future::BoxFuture<'a, Result<FetchResponse, FetchError>> {
        Box::pin(async { Err(FetchError::Transport) })
    }
}

#[tokio::test]
async fn retry_budget_reopens_and_pauses_after_five() {
    let (dir, mut service) = service().await;
    let id = service
        .add(
            SubscriptionSource::new(
                "Тест".into(),
                "http://127.0.0.1:9/sub".into(),
                true,
                FetchRoute::Direct,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    for failure in 1..=5 {
        let token = service.schedule_metadata(id).await.unwrap().unwrap();
        let mut runtime =
            SubscriptionScheduler::with_wall_clock(NonZeroU64::MIN, 1, 1, Arc::new(|| 100))
                .unwrap();
        runtime
            .restore(
                id,
                token.generation,
                SubscriptionRetryState {
                    due: Some(Duration::ZERO),
                    failures: token.state.map_or(0, |s| s.failures),
                },
            )
            .unwrap();
        runtime
            .dispatch(Duration::ZERO, policy(), &service, Arc::new(ОшибкаFetch))
            .unwrap();
        assert!(matches!(
            runtime.next_event(|| Duration::ZERO).await.unwrap(),
            Some(SubscriptionScheduleEvent::Failed { .. })
        ));
        service.close().await.unwrap();
        let repo = EncryptedSubscriptionRepository::open(dir.0.join("db"), Zeroizing::new([9; 32]))
            .await
            .unwrap();
        service = SubscriptionService::new(repo);
        let state = service
            .schedule_metadata(id)
            .await
            .unwrap()
            .unwrap()
            .state
            .unwrap();
        assert_eq!(state.failures, failure);
        assert_eq!(state.paused, failure == 5);
        assert_eq!(
            state.due_wall,
            if failure == 5 {
                None
            } else {
                Some(100 + (1 << (failure - 1)))
            }
        );
    }
    service.close().await.unwrap();
}

#[tokio::test]
async fn storage_failure_blocks_dispatch_without_fetch() {
    let (dir, service) = service().await;
    let id = service
        .add(
            SubscriptionSource::new(
                "Тест".into(),
                "http://127.0.0.1:9/sub".into(),
                true,
                FetchRoute::Direct,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let calls = Arc::new(AtomicU64::new(0));
    let mut runtime = runtime();
    runtime.schedule(id, 1, Duration::ZERO).unwrap();
    let db = rusqlite::Connection::open(dir.0.join("db")).unwrap();
    db.execute(
        "UPDATE sources SET payload=zeroblob(length(payload)) WHERE id=?1",
        [id],
    )
    .unwrap();
    drop(db);
    runtime
        .dispatch(
            Duration::ZERO,
            policy(),
            &service,
            Arc::new(CountingHost(calls.clone())),
        )
        .unwrap();
    assert!(matches!(
        runtime.next_event(|| Duration::ZERO).await.unwrap(),
        Some(SubscriptionScheduleEvent::Failed {
            error: SubscriptionError::Repository(_),
            ..
        })
    ));
    assert_eq!(runtime.next_deadline(policy()), None);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    service.close().await.unwrap();
}

#[tokio::test]
async fn crash_checkpoint_reopens_without_immediate_fetch() {
    let (dir, service) = service().await;
    let id = source(&service, "socks5://127.0.0.1:1080#crash").await;
    let mut runtime =
        SubscriptionScheduler::with_wall_clock(NonZeroU64::MIN, 1, 1, Arc::new(|| 100)).unwrap();
    runtime
        .restore(
            id,
            1,
            SubscriptionRetryState {
                due: Some(Duration::ZERO),
                failures: 3,
            },
        )
        .unwrap();
    runtime
        .dispatch(Duration::ZERO, policy(), &service, host())
        .unwrap();
    assert_eq!(
        runtime.next_event(|| Duration::ZERO).await.unwrap(),
        Some(SubscriptionScheduleEvent::Review { id })
    );
    // Имитируем потерю процесса до решения review: checkpoint уже записан.
    drop(runtime);
    service.close().await.unwrap();
    let repo = EncryptedSubscriptionRepository::open(dir.0.join("db"), Zeroizing::new([9; 32]))
        .await
        .unwrap();
    let reopened = SubscriptionService::new(repo);
    let state = reopened
        .schedule_metadata(id)
        .await
        .unwrap()
        .unwrap()
        .state
        .unwrap();
    assert_eq!(state.due_wall, Some(86500));
    assert_eq!(state.failures, 3);
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn not_modified_persists_daily_due() {
    let (_dir, service) = service().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/sub", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for second in [false, true] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut byte = [0];
            while !bytes.ends_with(b"\r\n\r\n") {
                assert_eq!(stream.read(&mut byte).await.unwrap(), 1);
                bytes.push(byte[0]);
            }
            if second {
                assert!(
                    String::from_utf8_lossy(&bytes)
                        .to_ascii_lowercase()
                        .contains("if-none-match")
                );
                stream
                    .write_all(
                        b"HTTP/1.1 304 Not Modified\r\nETag: \"one\"\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
            } else {
                let body = "socks5://127.0.0.1:1080#etag";
                stream.write_all(format!("HTTP/1.1 200 OK\r\nETag: \"one\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        }
    });
    let id = service
        .add(SubscriptionSource::new("Тест".into(), url, true, FetchRoute::Direct).unwrap())
        .await
        .unwrap();
    let (_cancel, receiver) = watch::channel(false);
    let RefreshResult::Pending(pending) = service
        .refresh(id, Duration::from_secs(2), receiver)
        .await
        .unwrap()
    else {
        panic!("pending")
    };
    service
        .commit(pending.accept_clean().unwrap())
        .await
        .unwrap();
    let mut runtime =
        SubscriptionScheduler::with_wall_clock(NonZeroU64::MIN, 1, 1, Arc::new(|| 200)).unwrap();
    runtime.schedule(id, 1, Duration::ZERO).unwrap();
    runtime
        .dispatch(Duration::ZERO, policy(), &service, host())
        .unwrap();
    assert_eq!(
        runtime
            .next_event(|| Duration::from_secs(10))
            .await
            .unwrap(),
        Some(SubscriptionScheduleEvent::NotModified { id })
    );
    assert_eq!(
        service
            .schedule_metadata(id)
            .await
            .unwrap()
            .unwrap()
            .state
            .unwrap()
            .due_wall,
        Some(86600)
    );
    server.await.unwrap();
    service.close().await.unwrap();
}

#[tokio::test]
async fn schedule_ciphertext_wrong_key_and_corruption_fail_closed() {
    let (dir, service) = service().await;
    let id = service
        .add(
            SubscriptionSource::new(
                "Тест".into(),
                "http://127.0.0.1:9/sub".into(),
                true,
                FetchRoute::Direct,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    service
        .record_schedule_completion(SubscriptionIdentity { id, generation: 1 }, true, 100)
        .await
        .unwrap();
    service.close().await.unwrap();
    assert!(
        EncryptedSubscriptionRepository::open(dir.0.join("db"), Zeroizing::new([8; 32]))
            .await
            .is_err()
    );
    let db = rusqlite::Connection::open(dir.0.join("db")).unwrap();
    db.execute(
        "UPDATE sources SET payload=zeroblob(length(payload)) WHERE id=?1",
        [id],
    )
    .unwrap();
    drop(db);
    let repo = EncryptedSubscriptionRepository::open(dir.0.join("db"), Zeroizing::new([9; 32]))
        .await
        .unwrap();
    let reopened = SubscriptionService::new(repo);
    assert!(matches!(
        reopened.schedule_metadata_page(0).await,
        Err(SubscriptionError::Repository(RepositoryError::Corrupt))
    ));
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn legacy_database_and_source_without_schedule_migrate_and_reopen() {
    let (dir, service) = service().await;
    let id = service
        .add(
            SubscriptionSource::new(
                "Старый источник".into(),
                "http://127.0.0.1:9/sub".into(),
                true,
                FetchRoute::Direct,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    service.close().await.unwrap();
    // Только собственная пустая fixture: воспроизводим schema v1 без новых таблиц.
    let db = rusqlite::Connection::open(dir.0.join("db")).unwrap();
    db.execute_batch("DROP TABLE profiles; DROP TABLE profile_catalog; DROP TABLE source_sequence; PRAGMA user_version=1;").unwrap();
    drop(db);
    let repo = EncryptedSubscriptionRepository::open(dir.0.join("db"), Zeroizing::new([9; 32]))
        .await
        .unwrap();
    let migrated = SubscriptionService::new(repo);
    assert_eq!(
        migrated.schedule_metadata(id).await.unwrap().unwrap().state,
        None
    );
    migrated
        .record_schedule_completion(SubscriptionIdentity { id, generation: 1 }, true, 100)
        .await
        .unwrap();
    migrated.close().await.unwrap();
    let repo = EncryptedSubscriptionRepository::open(dir.0.join("db"), Zeroizing::new([9; 32]))
        .await
        .unwrap();
    let reopened = SubscriptionService::new(repo);
    assert_eq!(
        reopened
            .schedule_metadata(id)
            .await
            .unwrap()
            .unwrap()
            .state
            .unwrap()
            .due_wall,
        Some(86500)
    );
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn p2_старое_manual_completion_не_изменяет_новое_поколение() {
    let (_dir, service) = service().await;
    let id = service
        .add(
            SubscriptionSource::new(
                "Тест".into(),
                "http://127.0.0.1:9/sub".into(),
                true,
                FetchRoute::Direct,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    service
        .reserve_manual_schedule(SubscriptionIdentity { id, generation: 1 }, 100)
        .await
        .unwrap();
    service
        .change_source(
            id,
            1,
            SubscriptionSource::new(
                "Новый".into(),
                "http://127.0.0.1:9/new".into(),
                true,
                FetchRoute::Direct,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(matches!(
        service
            .reserve_manual_schedule(SubscriptionIdentity { id, generation: 1 }, 200)
            .await,
        Err(SubscriptionError::StaleUpdate)
    ));
    let result = service
        .record_schedule_completion(SubscriptionIdentity { id, generation: 1 }, true, 200)
        .await;
    assert!(
        matches!(result, Err(SubscriptionError::StaleUpdate)),
        "old completion: {result:?}"
    );
    let current = service.schedule_metadata(id).await.unwrap().unwrap();
    assert_eq!(current.generation, 2);
    assert_eq!(current.state, None);
    service.close().await.unwrap();
}

#[tokio::test]
async fn p2_hwid_сбрасывает_paused_в_переходе_поколения() {
    let (_dir, service) = service().await;
    let id = service
        .add(
            SubscriptionSource::new(
                "Тест".into(),
                "http://127.0.0.1:9/sub".into(),
                true,
                FetchRoute::Direct,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let token = service.schedule_metadata(id).await.unwrap().unwrap();
    service
        .persist_schedule(
            token,
            ScheduleState {
                version: 1,
                due_wall: None,
                recorded_wall: 100,
                failures: 5,
                paused: true,
            },
        )
        .await
        .unwrap();
    service.set_source_hwid(id, true).await.unwrap();
    let current = service.schedule_metadata(id).await.unwrap().unwrap();
    assert_eq!(current.generation, 2);
    assert_eq!(current.state, None, "новый ресурс не наследует paused");
    service.close().await.unwrap();
}
