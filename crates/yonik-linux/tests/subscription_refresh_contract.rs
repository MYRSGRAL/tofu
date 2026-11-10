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
    EncryptedSubscriptionRepository, FetchError, FetchRoute, PendingRefresh, RefreshResult,
    SubscriptionError, SubscriptionService, SubscriptionSource,
};
use zeroize::Zeroizing;

const OLD: &str = "trojan://invented-secret@example.invalid:443#old";
const NEW: &str = "trojan://invented-new-secret@example.invalid:443#new";
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
#[tokio::test]
async fn failed_wire_responses_preserve_every_persisted_snapshot_field() {
    let directory = TestDirectory::new();
    let responses = vec![
        ok(OLD),
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{NEW}", NEW.len() + 100),
        format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{NEW}", NEW.len()),
        format!("HTTP/1.1 200 OK\r\nContent-Range: bytes 0-1/99\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{NEW}", NEW.len()),
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        "HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        "HTTP/1.1 302 Found\r\nLocation: file:///forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
    ];
    let count = responses.len();
    let fixture = Fixture::start(responses).await;
    let repository = directory.open().await;
    let service = SubscriptionService::new(repository.clone());
    let id = service.add(fixture.source()).await.expect("источник");
    install(&service, id).await;
    let before = repository
        .load(id)
        .await
        .expect("БД")
        .expect("источник")
        .snapshot
        .expect("snapshot");
    for index in 1..count {
        let error = match refresh(&service, id).await {
            Err(error) => error,
            Ok(_) => panic!("ответ обязан быть отклонён"),
        };
        let expected = match index {
            1 => FetchError::Transport,
            6 => FetchError::InvalidSource,
            _ => FetchError::HttpStatus,
        };
        assert!(
            matches!(error, SubscriptionError::Fetch(actual) if actual == expected),
            "случай {index}: {error:?}"
        );
        let after = repository
            .load(id)
            .await
            .expect("БД")
            .expect("источник")
            .snapshot
            .expect("snapshot");
        assert_eq!(after.body, before.body);
        assert_eq!(after.metadata, before.metadata);
        assert_eq!(after.validators, before.validators);
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.source_generation, before.source_generation);
    }
    let requests = fixture.finish().await;
    assert_eq!(requests.len(), count);
    assert!(
        requests
            .iter()
            .all(|request| request.starts_with("GET /subscription HTTP/1.1\r\n"))
    );
    assert!(requests[1..].iter().all(|request| {
        request
            .to_ascii_lowercase()
            .contains("if-none-match: initial\r\n")
    }));
}
#[tokio::test]
async fn source_token_rotation_preserves_profile_id_and_favorite() {
    let directory = TestDirectory::new();
    let first = Fixture::start(vec![ok(OLD)]).await;
    let second = Fixture::start(vec![ok(&OLD.replace("#old", "#renamed"))]).await;
    let service = SubscriptionService::new(directory.open().await);
    let id = service.add(first.source()).await.expect("источник");
    install(&service, id).await;
    assert_eq!(first.finish().await.len(), 1);
    let before = service
        .profiles(id, Default::default())
        .await
        .expect("профили");
    assert_eq!(before.profiles.len(), 1);
    let profile_id = before.profiles[0].id;
    service
        .set_favorite(profile_id, true)
        .await
        .expect("избранное");
    let rotated = spec(
        &format!("{}?token=invented-rotated-token", second.url),
        FetchRoute::Direct,
    );
    service
        .change_source(id, 1, rotated)
        .await
        .expect("ротация URL");
    install(&service, id).await;
    let after = service
        .profiles(id, Default::default())
        .await
        .expect("профили");
    assert_eq!(after.profiles.len(), 1);
    assert_eq!(after.profiles[0].id, profile_id);
    assert!(after.profiles[0].favorite);
    assert_eq!(after.profiles[0].name, "renamed");
    let requests = second.finish().await;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /subscription?token=invented-rotated-token HTTP/1.1\r\n"));
    assert!(!requests[0].to_ascii_lowercase().contains("if-none-match:"));
}

#[tokio::test]
async fn cancelled_body_cannot_publish_a_late_complete_response() {
    let directory = TestDirectory::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let url = format!(
        "http://{}/subscription",
        listener.local_addr().expect("адрес")
    );
    let (started, waiting) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut started = Some(started);
        let mut released = Some(released);
        for index in 0..2 {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .expect("запрос")
                .expect("accept");
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                assert_eq!(stream.read(&mut byte).await.expect("HTTP"), 1);
                request.push(byte[0]);
                assert!(request.len() < 32768);
            }
            if index == 0 {
                stream
                    .write_all(ok(OLD).as_bytes())
                    .await
                    .expect("snapshot");
            } else {
                let response = ok(NEW);
                stream
                    .write_all(&response.as_bytes()[..response.len() - 1])
                    .await
                    .expect("часть тела");
                started.take().expect("сигнал").send(()).expect("читатель");
                released
                    .take()
                    .expect("барьер")
                    .await
                    .expect("разрешение позднего ответа");
                // После отмены клиент вправе закрыть соединение до этой записи.
                let _ = stream
                    .write_all(&response.as_bytes()[response.len() - 1..])
                    .await;
            }
        }
    });
    let service = SubscriptionService::new(directory.open().await);
    let id = service
        .add(spec(&url, FetchRoute::Direct))
        .await
        .expect("источник");
    install(&service, id).await;
    let worker = service.clone();
    let refreshing = tokio::spawn(async move { refresh(&worker, id).await });
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("тело отправлено")
        .expect("сигнал");
    assert!(service.cancel(id).await);
    let result = tokio::time::timeout(Duration::from_secs(5), refreshing)
        .await
        .expect("отмена без конца тела")
        .expect("worker");
    assert!(matches!(
        result,
        Err(SubscriptionError::Fetch(FetchError::Cancelled)) | Err(SubscriptionError::StaleUpdate)
    ));
    release.send(()).expect("поздний ответ");
    server.await.expect("fixture");
    let snapshot = service.offline(id).await.expect("offline");
    assert_eq!(snapshot.report.original().expose(), OLD);
    assert_eq!(snapshot.revision, 1);
}
