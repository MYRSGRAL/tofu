use std::{num::NonZeroU64, time::Duration};
use yonik_linux::SubscriptionScheduler;

#[test]
fn очередь_ограничена() {
    let mut runtime = SubscriptionScheduler::new(NonZeroU64::MIN, 1, 1).unwrap();
    runtime.schedule(1, 1, Duration::ZERO).unwrap();
    assert!(runtime.schedule(2, 1, Duration::ZERO).is_err());
}

static NEXT: AtomicU64 = AtomicU64::new(1);
struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yonik-service-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // create не принимает уже существующий каталог: очистка принадлежит только этому тесту.
        DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("приватный каталог теста");
        assert_eq!(
            fs::metadata(&path).expect("права").permissions().mode() & 0o777,
            0o700
        );
        Self(path)
    }
    async fn open(&self) -> EncryptedSubscriptionRepository {
        EncryptedSubscriptionRepository::open(
            self.0.join("subscriptions.sqlite"),
            Zeroizing::new([42; 32]),
        )
        .await
        .expect("открытие БД")
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("очистка своего каталога");
    }
}

use std::{
    fs::{self, DirBuilder},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
use tokio::sync::watch;
use yonik_linux::*;
use yonik_subscription::scheduler::Policy;
use zeroize::Zeroizing;
fn policy() -> Policy {
    Policy {
        online: true,
        ui_visible: true,
        ..Policy::default()
    }
}
struct Fake {
    calls: AtomicUsize,
    wait: bool,
}
impl SubscriptionFetch for Fake {
    fn fetch<'a>(
        &'a self,
        _: &'a Source,
        route: FetchRoute,
        _: Option<&'a FetchValidators>,
        mut cancel: watch::Receiver<bool>,
    ) -> futures_util::future::BoxFuture<'a, Result<FetchResponse, FetchError>> {
        Box::pin(async move {
            assert_eq!(route, FetchRoute::Proxy);
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.wait {
                let _ = cancel.changed().await;
                return Err(FetchError::Cancelled);
            }
            Err(FetchError::Transport)
        })
    }
}
async fn setup() -> (TestDirectory, SubscriptionService, i64) {
    let dir = TestDirectory::new();
    let service = SubscriptionService::new(dir.open().await);
    let id = service
        .add(
            SubscriptionSource::new(
                "Тест".into(),
                "https://example.invalid/secret".into(),
                false,
                FetchRoute::Proxy,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    (dir, service, id)
}
#[tokio::test]
async fn ошибка_транспорта_даёт_backoff_без_direct() {
    let (_dir, service, id) = setup().await;
    let host = Arc::new(Fake {
        calls: AtomicUsize::new(0),
        wait: false,
    });
    let mut runtime = SubscriptionScheduler::new(NonZeroU64::MIN, 2, 1).unwrap();
    runtime.schedule(id, 1, Duration::ZERO).unwrap();
    runtime
        .dispatch(Duration::ZERO, policy(), &service, host.clone())
        .unwrap();
    assert_eq!(runtime.next_deadline(policy()), None);
    assert_eq!(
        runtime
            .next_event(|| Duration::from_secs(10))
            .await
            .unwrap(),
        Some(SubscriptionScheduleEvent::Failed {
            id,
            error: SubscriptionError::Fetch(FetchError::Transport)
        })
    );
    assert_eq!(host.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        runtime.next_deadline(policy()),
        Some(Duration::from_secs(11))
    );
    assert!(matches!(
        service.offline(id).await,
        Err(SubscriptionError::NoSnapshot)
    ));
    runtime.shutdown(Duration::ZERO).await.unwrap();
}
#[tokio::test]
async fn отмена_до_запуска_и_устаревшее_поколение() {
    let (_dir, service, id) = setup().await;
    let host = Arc::new(Fake {
        calls: AtomicUsize::new(0),
        wait: false,
    });
    let mut runtime = SubscriptionScheduler::new(NonZeroU64::MIN, 1, 1).unwrap();
    runtime.schedule(id, 1, Duration::ZERO).unwrap();
    runtime
        .dispatch(Duration::ZERO, policy(), &service, host.clone())
        .unwrap();
    runtime.cancel(id, Duration::ZERO).unwrap();
    assert!(runtime.schedule(id, 2, Duration::ZERO).is_err());
    assert_eq!(
        runtime.next_event(|| Duration::ZERO).await.unwrap(),
        Some(SubscriptionScheduleEvent::Cancelled { id })
    );
    runtime.schedule(id, 2, Duration::ZERO).unwrap();
    runtime
        .dispatch(Duration::ZERO, policy(), &service, host.clone())
        .unwrap();
    assert_eq!(
        runtime.next_event(|| Duration::ZERO).await.unwrap(),
        Some(SubscriptionScheduleEvent::Failed {
            id,
            error: SubscriptionError::StaleUpdate
        })
    );
    runtime.shutdown(Duration::ZERO).await.unwrap();
    assert_eq!(runtime.next_deadline(policy()), None);
}

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
struct Fixture {
    url: String,
    task: tokio::task::JoinHandle<Vec<String>>,
}
impl Fixture {
    async fn start(responses: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("локальный listener");
        let url = format!(
            "http://{}/subscription",
            listener.local_addr().expect("адрес")
        );
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) =
                    tokio::time::timeout(Duration::from_secs(5), listener.accept())
                        .await
                        .expect("ожидание запроса")
                        .expect("accept");
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    assert_eq!(stream.read(&mut byte).await.expect("чтение"), 1);
                    request.push(byte[0]);
                    assert!(request.len() < 32 * 1024);
                }
                requests.push(String::from_utf8(request).expect("HTTP"));
                stream.write_all(response.as_bytes()).await.expect("ответ");
            }
            requests
        });
        Self { url, task }
    }
    fn source(&self) -> SubscriptionSource {
        spec(&self.url, FetchRoute::Direct)
    }
    async fn finish(self) -> Vec<String> {
        self.task.await.expect("HTTP fixture")
    }
}
fn spec(url: &str, route: FetchRoute) -> SubscriptionSource {
    SubscriptionSource::new("Тестовый источник".into(), url.into(), true, route).expect("источник")
}
fn ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: initial\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

const OLD: &str = "socks5://127.0.0.1:1080#Старый";
const NEW: &str = "socks5://127.0.0.1:1081#Новый";
#[tokio::test]
async fn review_сохраняет_прежний_snapshot_до_явного_approval() {
    let dir = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD), ok(NEW)]).await;
    let service = SubscriptionService::new(dir.open().await);
    let id = service.add(fixture.source()).await.unwrap();
    let host = Arc::new(FetchHost::new(Duration::from_secs(2)).unwrap());
    let mut runtime = SubscriptionScheduler::new(NonZeroU64::MIN, 1, 1).unwrap();
    runtime.schedule(id, 1, Duration::ZERO).unwrap();
    runtime
        .dispatch(Duration::ZERO, policy(), &service, host.clone())
        .unwrap();
    assert_eq!(
        runtime.next_event(|| Duration::ZERO).await.unwrap(),
        Some(SubscriptionScheduleEvent::Review { id })
    );
    assert!(matches!(
        service.offline(id).await,
        Err(SubscriptionError::NoSnapshot)
    ));
    assert_eq!(
        runtime
            .finish_review(&service, id, true, Duration::ZERO)
            .await
            .unwrap(),
        Some(1)
    );
    let deadline = runtime.next_deadline(policy()).unwrap();
    runtime
        .dispatch(deadline, policy(), &service, host)
        .unwrap();
    assert_eq!(
        runtime.next_event(|| deadline).await.unwrap(),
        Some(SubscriptionScheduleEvent::Review { id })
    );
    assert_eq!(
        runtime.review(id).unwrap().report().original().expose(),
        NEW
    );
    assert_eq!(
        service
            .offline(id)
            .await
            .unwrap()
            .report
            .original()
            .expose(),
        OLD
    );
    assert_eq!(runtime.next_deadline(policy()), None);
    runtime
        .finish_review(&service, id, false, deadline)
        .await
        .unwrap();
    assert!(runtime.review(id).is_none());
    assert_eq!(service.offline(id).await.unwrap().revision, 1);
    fixture.finish().await;
    runtime.shutdown(deadline).await.unwrap();
    assert_eq!(runtime.next_deadline(policy()), None);
}
#[tokio::test]
async fn отмена_активного_fetch_освобождает_слот() {
    let (_dir, service, id) = setup().await;
    let host = Arc::new(Fake {
        calls: AtomicUsize::new(0),
        wait: true,
    });
    let mut runtime = SubscriptionScheduler::new(NonZeroU64::MIN, 1, 1).unwrap();
    runtime.schedule(id, 1, Duration::ZERO).unwrap();
    runtime
        .dispatch(Duration::ZERO, policy(), &service, host.clone())
        .unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            runtime.next_event(|| Duration::ZERO)
        )
        .await
        .is_err()
    );
    assert_eq!(host.calls.load(Ordering::SeqCst), 1);
    runtime.cancel(id, Duration::ZERO).unwrap();
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(2),
            runtime.next_event(|| Duration::ZERO)
        )
        .await
        .unwrap()
        .unwrap(),
        Some(SubscriptionScheduleEvent::Cancelled { id })
    );
    runtime.schedule(id, 1, Duration::ZERO).unwrap();
    runtime.shutdown(Duration::ZERO).await.unwrap();
    assert_eq!(runtime.next_deadline(policy()), None);
}

#[tokio::test]
async fn смена_источника_и_удаление_блокируют_устаревший_review() {
    let dir = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD), ok(NEW)]).await;
    let service = SubscriptionService::new(dir.open().await);
    let id = service.add(fixture.source()).await.unwrap();
    let host = Arc::new(FetchHost::new(Duration::from_secs(2)).unwrap());
    let mut runtime = SubscriptionScheduler::new(NonZeroU64::MIN, 1, 1).unwrap();
    runtime.schedule(id, 1, Duration::ZERO).unwrap();
    runtime
        .dispatch(Duration::ZERO, policy(), &service, host.clone())
        .unwrap();
    runtime.next_event(|| Duration::ZERO).await.unwrap();
    runtime
        .finish_review(&service, id, true, Duration::ZERO)
        .await
        .unwrap();
    let due = runtime.next_deadline(policy()).unwrap();
    runtime
        .dispatch(due, policy(), &service, host.clone())
        .unwrap();
    runtime.next_event(|| due).await.unwrap();
    runtime.cancel(id, due).unwrap();
    let generation = service
        .change_source(
            id,
            1,
            spec("https://example.invalid/new", FetchRoute::Proxy),
        )
        .await
        .unwrap();
    assert_eq!(
        runtime.next_event(|| due).await.unwrap(),
        Some(SubscriptionScheduleEvent::Cancelled { id })
    );
    assert!(runtime.review(id).is_none());
    assert_eq!(
        service
            .offline(id)
            .await
            .unwrap()
            .report
            .original()
            .expose(),
        OLD
    );
    runtime.schedule(id, generation, due).unwrap();
    service.delete_source(id).await.unwrap();
    runtime.dispatch(due, policy(), &service, host).unwrap();
    assert_eq!(
        runtime.next_event(|| due).await.unwrap(),
        Some(SubscriptionScheduleEvent::Failed {
            id,
            error: SubscriptionError::NotFound
        })
    );
    runtime.shutdown(due).await.unwrap();
    fixture.finish().await;
}
