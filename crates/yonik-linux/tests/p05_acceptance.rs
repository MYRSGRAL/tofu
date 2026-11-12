#![cfg(target_os = "linux")]

use std::{
    fs::{self, DirBuilder},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::watch,
};
use yonik_linux::{
    EncryptedSubscriptionRepository, FetchRoute, PendingRefresh, RefreshResult, SubscriptionError,
    SubscriptionService, SubscriptionSource,
};
use zeroize::Zeroizing;

const OLD: &str = "trojan://invented-secret@example.invalid:443#old";
static NEXT: AtomicU64 = AtomicU64::new(0);

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
async fn refresh(
    service: &SubscriptionService,
    id: i64,
) -> Result<RefreshResult, SubscriptionError> {
    let (_sender, receiver) = watch::channel(false);
    service.refresh(id, Duration::from_secs(2), receiver).await
}
async fn pending(service: &SubscriptionService, id: i64) -> PendingRefresh {
    match refresh(service, id).await.expect("загрузка") {
        RefreshResult::Pending(value) => *value,
        RefreshResult::NotModified => panic!("ожидалось новое тело"),
    }
}
async fn install(service: &SubscriptionService, id: i64) -> i64 {
    service
        .commit(
            pending(service, id)
                .await
                .accept_clean()
                .expect("чистый импорт"),
        )
        .await
        .expect("commit")
}

use std::{num::NonZeroU64, sync::Arc};
use yonik_linux::{FetchHost, SubscriptionScheduleEvent, SubscriptionScheduler};
use yonik_subscription::scheduler::Policy;

#[tokio::test]
async fn ротация_источника_manual_scheduled_матрица() {
    for scheduled in [false, true] {
        for case in ["full", "partial", "empty", "corrupt", "http", "304", "size"] {
            let dir = TestDirectory::new();
            let first = Fixture::start(vec![ok(OLD)]).await;
            let body = match case {
                "full" => OLD.replace("#old", "#renamed"),
                "partial" => format!("{OLD}\nunknown://broken"),
                "corrupt" => "unknown://broken".into(),
                _ => String::new(),
            };
            let response = match case {
                "http" => "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n".into(),
                "304" => "HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".into(),
                "size" => {
                    "HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\nConnection: close\r\n\r\n"
                        .into()
                }
                _ => ok(&body),
            };
            let second = Fixture::start(vec![response]).await;
            let service = SubscriptionService::new(dir.open().await);
            let id = service.add(first.source()).await.expect("источник");
            install(&service, id).await;
            first.finish().await;
            let before = service
                .profiles(id, Default::default())
                .await
                .expect("профили");
            let profile = before.profiles[0].id;
            service
                .set_favorite(profile, true)
                .await
                .expect("избранное");
            service
                .change_source(
                    id,
                    1,
                    spec(
                        &format!("{}?token=synthetic-rotation", second.url),
                        FetchRoute::Direct,
                    ),
                )
                .await
                .expect("ротация");
            let mut runtime = SubscriptionScheduler::new(NonZeroU64::MIN, 1, 1).expect("scheduler");
            let pending = if scheduled {
                runtime.schedule(id, 2, Duration::ZERO).expect("due");
                runtime
                    .dispatch(
                        Duration::ZERO,
                        Policy {
                            online: true,
                            ui_visible: true,
                            ..Default::default()
                        },
                        &service,
                        Arc::new(FetchHost::new(Duration::from_secs(2)).expect("transport")),
                    )
                    .expect("dispatch");
                let event = tokio::time::timeout(
                    Duration::from_secs(5),
                    runtime.next_event(|| Duration::ZERO),
                )
                .await
                .expect("bounded")
                .expect("event")
                .expect("работа");
                if matches!(case, "full" | "partial") {
                    assert!(
                        matches!(event, SubscriptionScheduleEvent::Review { id: actual } if actual == id)
                    );
                    Some(runtime.review(id).expect("review").summary())
                } else {
                    assert!(
                        matches!(event, SubscriptionScheduleEvent::Failed { id: actual, .. } if actual == id),
                        "случай {case}"
                    );
                    None
                }
            } else {
                match refresh(&service, id).await {
                    Ok(RefreshResult::Pending(pending)) => {
                        assert!(matches!(case, "full" | "partial"));
                        let summary = pending.summary();
                        service.discard_refresh(*pending).await;
                        Some(summary)
                    }
                    Err(_) => {
                        assert!(!matches!(case, "full" | "partial"));
                        None
                    }
                    Ok(RefreshResult::NotModified) => {
                        panic!("304 после смены generation не имеет валидного snapshot")
                    }
                }
            };
            if let Some(summary) = pending {
                assert_eq!(summary.accepted, 1);
                assert_eq!(summary.rejected, usize::from(case == "partial"));
            }
            let snapshot = service.offline(id).await.expect("рабочий snapshot");
            assert_eq!(snapshot.revision, 1);
            assert_eq!(snapshot.source_generation, 1);
            assert_eq!(snapshot.current_source_generation, 2);
            assert!(snapshot.report.original().expose() == OLD);
            if scheduled && pending.is_some() {
                runtime
                    .finish_review(&service, id, true, || Duration::ZERO)
                    .await
                    .expect("явное согласие");
                assert_eq!(
                    service.offline(id).await.expect("новый snapshot").revision,
                    2
                );
            }
            let profiles = service
                .profiles(id, Default::default())
                .await
                .expect("профили");
            assert_eq!(profiles.profiles.len(), 1);
            assert_eq!(profiles.profiles[0].id, profile);
            assert!(profiles.profiles[0].favorite);
            let requests = second.finish().await;
            assert_eq!(requests.len(), 1);
            assert!(!requests[0].to_ascii_lowercase().contains("if-none-match:"));
            runtime.shutdown(Duration::ZERO).await.expect("shutdown");
            service.close().await.expect("закрытие");
        }
    }
}

#[tokio::test]
async fn импорт_10x_с_потолком_и_постраничным_чтением() {
    for count in [200, 2000] {
        let dir = TestDirectory::new();
        let body = (0..count)
            .map(|i| format!("trojan://synthetic-{i}@example.invalid:443#Профиль-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(body.len() < 256 * 1024);
        let fixture = Fixture::start(vec![ok(&body)]).await;
        let service = SubscriptionService::new(dir.open().await);
        let id = service.add(fixture.source()).await.expect("источник");
        tokio::time::timeout(Duration::from_secs(15), install(&service, id))
            .await
            .expect("bounded импорт");
        let mut seen = std::collections::BTreeSet::new();
        for offset in (0..count).step_by(100) {
            let page = service
                .profiles(
                    id,
                    yonik_linux::ProfileQuery {
                        offset,
                        limit: 100,
                        ..Default::default()
                    },
                )
                .await
                .expect("страница");
            assert_eq!(page.total, count);
            assert_eq!(page.profiles.len(), 100);
            for profile in page.profiles {
                assert!(seen.insert(profile.id));
            }
        }
        assert_eq!(seen.len(), count);
        fixture.finish().await;
        service.close().await.expect("закрытие");
    }
}

#[tokio::test]
async fn proxy_отказ_после_ротации_не_касается_direct_и_snapshot() {
    let dir = TestDirectory::new();
    let first = Fixture::start(vec![ok(OLD)]).await;
    let service = SubscriptionService::new(dir.open().await);
    let id = service.add(first.source()).await.expect("источник");
    install(&service, id).await;
    first.finish().await;
    let direct = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("direct oracle");
    let url = format!(
        "http://{}/rotated?token=synthetic",
        direct.local_addr().expect("адрес")
    );
    service
        .change_source(id, 1, spec(&url, FetchRoute::Proxy))
        .await
        .expect("ротация");
    for scheduled in [false, true] {
        if scheduled {
            let mut runtime = SubscriptionScheduler::new(NonZeroU64::MIN, 1, 1).expect("scheduler");
            runtime.schedule(id, 2, Duration::ZERO).expect("due");
            runtime
                .dispatch(
                    Duration::ZERO,
                    Policy {
                        online: true,
                        ui_visible: true,
                        ..Default::default()
                    },
                    &service,
                    Arc::new(FetchHost::new(Duration::from_secs(2)).expect("host")),
                )
                .expect("dispatch");
            let event = runtime.next_event(|| Duration::ZERO).await.expect("event");
            assert!(
                matches!(event, Some(SubscriptionScheduleEvent::Failed { id: actual, .. }) if actual == id)
            );
            runtime.shutdown(Duration::ZERO).await.expect("shutdown");
        } else {
            assert!(refresh(&service, id).await.is_err());
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), direct.accept())
                .await
                .is_err()
        );
        let snapshot = service.offline(id).await.expect("offline");
        assert_eq!(snapshot.revision, 1);
        assert!(snapshot.report.original().expose() == OLD);
    }
    service.close().await.expect("закрытие");
}
