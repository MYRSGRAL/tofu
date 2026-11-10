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
    runtime.schedule(first, 1, Duration::ZERO).unwrap();
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
