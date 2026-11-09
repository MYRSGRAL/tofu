#![cfg(target_os = "linux")]

use std::{
    fs::{self, DirBuilder},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tofu_linux::{
    EncryptedSubscriptionRepository, FetchError, FetchHost, FetchResponse, FetchRoute,
    FetchValidators, PendingRefresh, RefreshResult, Source, SubscriptionError, SubscriptionService,
    SubscriptionSource,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::watch,
};
use zeroize::Zeroizing;

const OLD: &str = "trojan://invented-secret@example.invalid:443#old";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tofu-hwid-{}-{}",
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
fn hwid(request: &str) -> Option<&str> {
    request.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("x-hwid").then_some(value.trim())
    })
}

async fn fetch_source(
    source: &Source,
    validators: Option<&FetchValidators>,
) -> Result<FetchResponse, FetchError> {
    let (_sender, cancel) = watch::channel(false);
    FetchHost::new(Duration::from_secs(2))
        .expect("host")
        .fetch(source, FetchRoute::Direct, validators, cancel)
        .await
}

#[tokio::test]
async fn opt_in_is_stable_private_and_reused_after_restart_and_disable() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD); 6]).await;
    let repository = directory.open().await;
    let service = SubscriptionService::new(repository.clone());
    let id = service.add(fixture.source()).await.expect("add");
    assert!(!service.source_hwid_enabled(id).await.expect("disabled"));
    let disabled = repository.load(id).await.expect("record").expect("source");
    let disabled_data: serde_json::Value =
        serde_json::from_slice(&disabled.source.payload).expect("payload");
    assert!(disabled_data["hwid"].is_null());
    install(&service, id).await;
    service.set_source_hwid(id, true).await.expect("opt-in");
    assert!(service.sources_page(0).await.expect("page").sources[0].hwid_enabled);
    let changed = repository.load(id).await.expect("record").expect("source");
    assert_eq!(changed.source.generation, 2);
    assert!(changed.snapshot.expect("old snapshot").validators.is_none());
    install(&service, id).await;
    install(&service, id).await;
    service.set_source_hwid(id, false).await.expect("disable");
    install(&service, id).await;
    service.set_source_hwid(id, true).await.expect("re-enable");
    install(&service, id).await;
    drop(repository);
    service.close().await.expect("close");
    let reopened = SubscriptionService::new(directory.open().await);
    assert!(reopened.source_hwid_enabled(id).await.expect("enabled"));
    install(&reopened, id).await;
    reopened.close().await.expect("close");
    let requests = fixture.finish().await;
    assert!(hwid(&requests[0]).is_none());
    assert!(hwid(&requests[3]).is_none());
    let secret = hwid(&requests[1]).expect("opt-in header");
    assert_eq!(secret.len(), 32);
    assert!(secret.bytes().all(|byte| byte.is_ascii_hexdigit()));
    for index in [2, 4, 5] {
        assert_eq!(hwid(&requests[index]), Some(secret));
    }
    assert!(!requests[1].to_ascii_lowercase().contains("if-none-match"));
    assert!(requests[2].to_ascii_lowercase().contains("if-none-match"));
    assert!(!requests[3].to_ascii_lowercase().contains("if-none-match"));
    assert!(!requests[4].to_ascii_lowercase().contains("if-none-match"));
    let source = Source::local("http://localhost/subscription")
        .expect("source")
        .with_hwid(secret)
        .expect("hwid");
    assert!(!format!("{source:?}").contains(secret));
    for entry in fs::read_dir(&directory.0).expect("files") {
        let data = fs::read(entry.expect("file").path()).expect("read");
        assert!(
            !data
                .windows(secret.len())
                .any(|part| part == secret.as_bytes())
        );
    }
}

#[tokio::test]
async fn disabled_source_still_rejects_malformed_stored_hwid() {
    let directory = TestDirectory::new();
    let repository = directory.open().await;
    let malformed = serde_json::json!({"version":1,"name":"Malformed", "url":"https://example.invalid/",
        "local_exception":false,"proxy":false,"hwid_enabled":false,"hwid":"short"});
    let id = repository
        .create_source(serde_json::to_vec(&malformed).expect("payload"))
        .await
        .expect("source")
        .id;
    let service = SubscriptionService::new(repository);
    assert!(service.source_hwid_enabled(id).await.is_err());
    assert!(service.sources_page(0).await.is_err());
    service.close().await.expect("close");
}

#[tokio::test]
async fn independent_sources_and_legacy_payload_start_disabled() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD); 3]).await;
    let repository = directory.open().await;
    let legacy = serde_json::json!({"version":1,"name":"Legacy", "url":fixture.url,
        "local_exception":true,"proxy":false});
    let id = repository
        .create_source(serde_json::to_vec(&legacy).expect("payload"))
        .await
        .expect("legacy")
        .id;
    let service = SubscriptionService::new(repository);
    assert!(
        !service
            .source_hwid_enabled(id)
            .await
            .expect("legacy disabled")
    );
    install(&service, id).await;
    let first = service
        .add(fixture.source().with_hwid(true).expect("opt-in"))
        .await
        .expect("add");
    let second = service
        .add(fixture.source().with_hwid(true).expect("opt-in"))
        .await
        .expect("add");
    install(&service, first).await;
    install(&service, second).await;
    let requests = fixture.finish().await;
    assert!(hwid(&requests[0]).is_none());
    assert_ne!(hwid(&requests[1]), hwid(&requests[2]));
    service.close().await.expect("close");
}

#[tokio::test]
async fn provider_flags_precede_status_and_not_modified() {
    for status in ["200 OK", "404 Not Found", "304 Not Modified"] {
        for (headers, expected) in [
            (
                "X-HwId-NoT-SuPpOrTeD: TrUe\r\nx-hwid-limit: true\r\n",
                FetchError::HwidRequired,
            ),
            (
                "x-hwid-max-devices-reached: true\r\nx-hwid-limit: true\r\n",
                FetchError::HwidLimit,
            ),
            ("x-hwid-limit: TRUE\r\n", FetchError::HwidRejected),
        ] {
            let fixture = Fixture::start(vec![format!(
                "HTTP/1.1 {status}\r\n{headers}Content-Length: 0\r\nConnection: close\r\n\r\n"
            )])
            .await;
            let source = Source::local(&fixture.url).expect("source");
            assert!(matches!(fetch_source(&source, None).await, Err(error) if error == expected));
            fixture.finish().await;
        }
    }
    let fixture = Fixture::start(vec![ok(OLD).replace(
        "ETag: initial",
        "x-hwid-active: true\r\nx-hwid-limit: false\r\nETag: initial",
    )])
    .await;
    assert!(
        fetch_source(&Source::local(&fixture.url).expect("source"), None)
            .await
            .is_ok()
    );
    fixture.finish().await;
}

#[tokio::test]
async fn redirect_sends_hwid_only_within_same_origin() {
    let destination = TcpListener::bind("127.0.0.1:0").await.expect("destination");
    let fixture = Fixture::start(vec![format!("HTTP/1.1 302 Found\r\nLocation: http://{}/target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", destination.local_addr().expect("address"))]).await;
    let source = Source::local(&fixture.url)
        .expect("source")
        .with_hwid("0123456789abcdef")
        .expect("hwid");
    assert!(matches!(
        fetch_source(&source, None).await,
        Err(FetchError::HwidRedirect)
    ));
    fixture.finish().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), destination.accept())
            .await
            .is_err()
    );
    let fixture = Fixture::start(vec![
        "HTTP/1.1 302 Found\r\nLocation: /target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .into(),
        ok(OLD),
    ])
    .await;
    let source = Source::local(&fixture.url)
        .expect("source")
        .with_hwid("0123456789abcdef")
        .expect("hwid");
    assert!(fetch_source(&source, None).await.is_ok());
    let requests = fixture.finish().await;
    assert_eq!(hwid(&requests[0]), hwid(&requests[1]));
}

#[tokio::test]
async fn changing_fetch_identity_invalidates_conditional_headers() {
    let fixture = Fixture::start(vec![ok(OLD); 2]).await;
    let source = Source::local(&fixture.url)
        .expect("source")
        .with_hwid("0123456789abcdef")
        .expect("hwid");
    let validators = FetchValidators::new(&source, Some("initial"), None).expect("validators");
    let different = Source::local(&fixture.url)
        .expect("source")
        .with_hwid("fedcba9876543210")
        .expect("hwid");
    fetch_source(&different, Some(&validators))
        .await
        .expect("different id");
    fetch_source(
        &Source::local(&fixture.url).expect("source"),
        Some(&validators),
    )
    .await
    .expect("no id");
    for request in fixture.finish().await {
        assert!(!request.to_ascii_lowercase().contains("if-none-match"));
    }
    for value in [
        "short",
        "contains space",
        "contains\r\nheader",
        "contains_underscore",
    ] {
        assert!(matches!(
            Source::local("http://localhost/")
                .expect("source")
                .with_hwid(value),
            Err(FetchError::InvalidHwid)
        ));
    }
}

#[tokio::test]
async fn provider_rejection_keeps_last_snapshot() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![
        ok(OLD),
        "HTTP/1.1 200 OK\r\nx-hwid-limit: true\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .into(),
    ])
    .await;
    let service = SubscriptionService::new(directory.open().await);
    let id = service
        .add(fixture.source().with_hwid(true).expect("opt-in"))
        .await
        .expect("add");
    let revision = install(&service, id).await;
    assert!(matches!(
        refresh(&service, id).await,
        Err(SubscriptionError::Fetch(FetchError::HwidRejected))
    ));
    let snapshot = service.offline(id).await.expect("offline");
    assert_eq!(snapshot.revision, revision);
    assert_eq!(snapshot.report.original().expose(), OLD);
    fixture.finish().await;
    service.close().await.expect("close");
}

#[tokio::test]
async fn opt_in_cancels_pending_approval_and_preserves_profiles_atomically() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD); 2]).await;
    let repository = directory.open().await;
    let service = SubscriptionService::new(repository.clone());
    let id = service.add(fixture.source()).await.expect("add");
    let revision = install(&service, id).await;
    let approval = pending(&service, id)
        .await
        .accept_clean()
        .expect("approval");
    let profiles = service
        .profiles(id, Default::default())
        .await
        .expect("profiles");
    service.set_source_hwid(id, true).await.expect("enable");
    service
        .set_source_hwid(id, true)
        .await
        .expect("повторный opt-in");
    assert!(matches!(
        service.commit(approval).await,
        Err(SubscriptionError::StaleUpdate)
    ));
    let snapshot = service.offline(id).await.expect("snapshot");
    assert_eq!(snapshot.revision, revision);
    assert_eq!(snapshot.source_generation, 1);
    assert_eq!(snapshot.current_source_generation, 2);
    let unchanged = service
        .profiles(id, Default::default())
        .await
        .expect("profiles");
    assert_eq!(profiles.total, unchanged.total);
    let record = repository.load(id).await.expect("record").expect("source");
    assert!(record.snapshot.expect("snapshot").validators.is_none());
    fixture.finish().await;
    drop(repository);
    service.close().await.expect("close");
}

#[tokio::test]
async fn enabled_identity_keeps_validators_after_restart() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![
        ok(OLD),
        "HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".into(),
    ])
    .await;
    let service = SubscriptionService::new(directory.open().await);
    let id = service
        .add(fixture.source().with_hwid(true).expect("opt-in"))
        .await
        .expect("add");
    install(&service, id).await;
    service.close().await.expect("close");
    let service = SubscriptionService::new(directory.open().await);
    assert!(matches!(
        refresh(&service, id).await,
        Ok(RefreshResult::NotModified)
    ));
    let requests = fixture.finish().await;
    assert_eq!(hwid(&requests[0]), hwid(&requests[1]));
    assert!(
        requests[1]
            .to_ascii_lowercase()
            .contains("if-none-match: initial")
    );
    service.close().await.expect("close");
}
