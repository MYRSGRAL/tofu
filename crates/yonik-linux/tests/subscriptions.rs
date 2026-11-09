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
    RepositoryError, SubscriptionError, SubscriptionService, SubscriptionSource,
};
use zeroize::Zeroizing;

const OLD: &str = "trojan://invented-secret@example.invalid:443#old";
const NEW: &str = "trojan://invented-new-secret@example.invalid:443#new";
const PARTIAL: &str = "trojan://invented-new-secret@example.invalid:443#new\nunknown://invalid";
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
async fn assert_offline(
    service: &SubscriptionService,
    id: i64,
    original: &str,
    revision: i64,
    source_generation: i64,
    current_generation: i64,
) {
    let snapshot = service.offline(id).await.expect("offline snapshot");
    assert_eq!(snapshot.report.original().expose(), original);
    assert_eq!(snapshot.revision, revision);
    assert_eq!(snapshot.source_generation, source_generation);
    assert_eq!(snapshot.current_source_generation, current_generation);
}

#[tokio::test]
async fn local_pending_commit_reopen_and_offline_without_server() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD)]).await;
    assert!(
        SubscriptionSource::new(
            "Публичный".into(),
            fixture.url.clone(),
            false,
            FetchRoute::Direct
        )
        .is_err()
    );
    let service = SubscriptionService::new(directory.open().await);
    let id = service.add(fixture.source()).await.expect("add");
    let staged = pending(&service, id).await;
    assert_eq!(staged.summary().accepted, 1);
    assert!(!staged.summary().needs_review());
    assert!(matches!(
        service.offline(id).await,
        Err(SubscriptionError::NoSnapshot)
    ));
    assert_eq!(
        service
            .commit(staged.accept_clean().expect("согласие"))
            .await,
        Ok(1)
    );
    fixture.finish().await;
    drop(service);
    let reopened = SubscriptionService::new(directory.open().await);
    assert_offline(&reopened, id, OLD, 1, 1, 1).await;
}

#[tokio::test]
async fn partial_import_keeps_old_until_exact_report_is_approved() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD), ok(PARTIAL), ok(PARTIAL)]).await;
    let service = SubscriptionService::new(directory.open().await);
    let id = service.add(fixture.source()).await.expect("add");
    assert_eq!(install(&service, id).await, 1);
    let unreviewed = pending(&service, id).await;
    assert_eq!(unreviewed.summary().accepted, 1);
    assert_eq!(unreviewed.summary().rejected, 1);
    assert!(unreviewed.summary().needs_review());
    assert!(matches!(
        unreviewed.accept_clean(),
        Err(SubscriptionError::Import(
            yonik_subscription::Error::ReviewRequired
        ))
    ));
    assert_offline(&service, id, OLD, 1, 1, 1).await;
    let shown = pending(&service, id).await;
    assert_eq!(shown.report().original().expose(), PARTIAL);
    let approved = shown.approve_after_review();
    assert_offline(&service, id, OLD, 1, 1, 1).await;
    assert_eq!(service.commit(approved).await, Ok(2));
    assert_offline(&service, id, PARTIAL, 2, 1, 1).await;
    fixture.finish().await;
}

#[tokio::test]
async fn changed_source_rejects_stale_pending_and_preserves_old_snapshot() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD), ok(NEW)]).await;
    let service = SubscriptionService::new(directory.open().await);
    let id = service.add(fixture.source()).await.expect("add");
    install(&service, id).await;
    let old_pending = pending(&service, id).await.accept_clean().expect("approve");
    assert_eq!(
        service
            .change_source(
                id,
                1,
                spec(
                    "https://new.example.invalid/subscription",
                    FetchRoute::Direct
                )
            )
            .await,
        Ok(2)
    );
    assert_eq!(
        service.commit(old_pending).await,
        Err(SubscriptionError::StaleUpdate)
    );
    assert_offline(&service, id, OLD, 1, 1, 2).await;
    fixture.finish().await;
}

#[tokio::test]
async fn newer_pending_invalidates_previous_approval() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD), ok(NEW)]).await;
    let service = SubscriptionService::new(directory.open().await);
    let id = service.add(fixture.source()).await.expect("add");
    let first = pending(&service, id)
        .await
        .accept_clean()
        .expect("approve first");
    let second = pending(&service, id)
        .await
        .accept_clean()
        .expect("approve second");
    assert_eq!(
        service.commit(first).await,
        Err(SubscriptionError::StaleUpdate)
    );
    assert_eq!(service.commit(second).await, Ok(1));
    assert_offline(&service, id, NEW, 1, 1, 1).await;
    fixture.finish().await;
}

#[tokio::test]
async fn conditional_304_updates_validator_without_replacing_body() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![
        ok(OLD),
        "HTTP/1.1 304 Not Modified\r\nETag: updated\r\nConnection: close\r\n\r\n".into(),
        "HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".into(),
    ])
    .await;
    let service = SubscriptionService::new(directory.open().await);
    let id = service.add(fixture.source()).await.expect("add");
    install(&service, id).await;
    assert!(matches!(
        refresh(&service, id).await,
        Ok(RefreshResult::NotModified)
    ));
    assert_offline(&service, id, OLD, 2, 1, 1).await;
    assert!(matches!(
        refresh(&service, id).await,
        Ok(RefreshResult::NotModified)
    ));
    assert_offline(&service, id, OLD, 3, 1, 1).await;
    let requests = fixture.finish().await;
    assert!(!requests[0].contains("if-none-match"));
    assert!(requests[1].contains("if-none-match: initial"));
    assert!(requests[2].contains("if-none-match: updated"));
}

#[tokio::test]
async fn failed_http_and_empty_refresh_preserve_committed_snapshot() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![
        ok(OLD),
        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .into(),
        ok(""),
    ])
    .await;
    let service = SubscriptionService::new(directory.open().await);
    let id = service.add(fixture.source()).await.expect("add");
    install(&service, id).await;
    assert!(matches!(
        refresh(&service, id).await,
        Err(SubscriptionError::Fetch(FetchError::HttpStatus))
    ));
    assert_offline(&service, id, OLD, 1, 1, 1).await;
    assert!(matches!(
        refresh(&service, id).await,
        Err(SubscriptionError::Import(_))
    ));
    assert_offline(&service, id, OLD, 1, 1, 1).await;
    fixture.finish().await;
}

#[tokio::test]
async fn proxy_policy_never_falls_back_to_direct() {
    let directory = TestDirectory::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let url = format!(
        "http://{}/subscription",
        listener.local_addr().expect("адрес")
    );
    let service = SubscriptionService::new(directory.open().await);
    let id = service
        .add(spec(&url, FetchRoute::Proxy))
        .await
        .expect("add");
    assert!(matches!(
        refresh(&service, id).await,
        Err(SubscriptionError::Fetch(
            FetchError::UnsupportedPeerVerification
        ))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
    assert!(matches!(
        service.offline(id).await,
        Err(SubscriptionError::NoSnapshot)
    ));
}

#[tokio::test]
async fn approval_cannot_be_committed_to_another_repository_with_same_source_id() {
    let first_directory = TestDirectory::new();
    let second_directory = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(NEW)]).await;
    let first = SubscriptionService::new(first_directory.open().await);
    let second = SubscriptionService::new(second_directory.open().await);
    let first_id = first.add(fixture.source()).await.expect("первый источник");
    let second_id = second.add(fixture.source()).await.expect("второй источник");
    assert_eq!(first_id, second_id);
    let approved = pending(&first, first_id)
        .await
        .accept_clean()
        .expect("approve");
    assert_eq!(
        second.commit(approved).await,
        Err(SubscriptionError::ForeignApproval)
    );
    assert!(matches!(
        first.offline(first_id).await,
        Err(SubscriptionError::NoSnapshot)
    ));
    assert!(matches!(
        second.offline(second_id).await,
        Err(SubscriptionError::NoSnapshot)
    ));
    fixture.finish().await;
}

#[tokio::test]
async fn independent_services_still_enforce_repository_revision_cas() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD), ok(NEW)]).await;
    let first_service = SubscriptionService::new(directory.open().await);
    let id = first_service.add(fixture.source()).await.expect("add");
    let second_service = SubscriptionService::new(directory.open().await);
    let first = pending(&first_service, id)
        .await
        .accept_clean()
        .expect("approve first");
    let second = pending(&second_service, id)
        .await
        .accept_clean()
        .expect("approve second");
    assert_eq!(first_service.commit(first).await, Ok(1));
    assert_eq!(
        second_service.commit(second).await,
        Err(SubscriptionError::Repository(RepositoryError::Conflict))
    );
    assert_offline(&second_service, id, OLD, 1, 1, 1).await;
    fixture.finish().await;
}

#[tokio::test]
async fn cancel_after_pending_prevents_commit() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD), ok(NEW)]).await;
    let service = SubscriptionService::new(directory.open().await);
    let id = service.add(fixture.source()).await.expect("add");
    install(&service, id).await;
    let approved = pending(&service, id).await.accept_clean().expect("approve");
    assert!(service.cancel(id).await);
    assert_eq!(
        service.commit(approved).await,
        Err(SubscriptionError::StaleUpdate)
    );
    assert_offline(&service, id, OLD, 1, 1, 1).await;
    fixture.finish().await;
}

#[tokio::test]
async fn newer_refresh_cancels_previous_request_waiting_for_http_headers() {
    let directory = TestDirectory::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let url = format!(
        "http://{}/subscription",
        listener.local_addr().expect("адрес")
    );
    let service = SubscriptionService::new(directory.open().await);
    let id = service
        .add(spec(&url, FetchRoute::Direct))
        .await
        .expect("add");
    let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.expect("первый запрос");
        let mut byte = [0];
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            assert_eq!(first.read(&mut byte).await.expect("первый HTTP"), 1);
            request.push(byte[0]);
            assert!(request.len() < 32 * 1024);
        }
        started_sender.send(()).expect("начало ожидания ответа");
        let (mut second, _) = listener.accept().await.expect("второй запрос");
        request.clear();
        while !request.ends_with(b"\r\n\r\n") {
            assert_eq!(second.read(&mut byte).await.expect("второй HTTP"), 1);
            request.push(byte[0]);
            assert!(request.len() < 32 * 1024);
        }
        second
            .write_all(ok(NEW).as_bytes())
            .await
            .expect("ответ новому запросу");
        // Старому запросу ответ не посылается: завершение обязано прийти от отмены.
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), first.read(&mut byte))
                .await
                .expect("закрытие старого сокета")
                .expect("чтение закрытия"),
            0
        );
    });
    let old_service = service.clone();
    let first = tokio::spawn(async move { refresh(&old_service, id).await });
    tokio::time::timeout(Duration::from_secs(2), started_receiver)
        .await
        .expect("старый fetch начал ждать")
        .expect("сигнал fixture");
    let newest = pending(&service, id)
        .await
        .accept_clean()
        .expect("approve newest");
    let cancelled = tokio::time::timeout(Duration::from_secs(2), first)
        .await
        .expect("отмена старого fetch")
        .expect("задача fetch");
    assert!(matches!(
        cancelled,
        Err(SubscriptionError::Fetch(FetchError::Cancelled)) | Err(SubscriptionError::StaleUpdate)
    ));
    assert_eq!(service.commit(newest).await, Ok(1));
    assert_offline(&service, id, NEW, 1, 1, 1).await;
    server.await.expect("fixture");
}

#[tokio::test]
async fn rename_preserves_hwid_credentials_snapshot_and_validators_without_fetch() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD)]).await;
    let repository = directory.open().await;
    let service = SubscriptionService::new(repository.clone());
    let id = service
        .add(fixture.source().with_hwid(true).expect("HWID"))
        .await
        .expect("источник");
    install(&service, id).await;
    fixture.finish().await;
    let before = repository.load(id).await.expect("БД").expect("источник");
    service
        .rename_source(id, "Новое имя".into())
        .await
        .expect("переименование без сети");
    let after = repository.load(id).await.expect("БД").expect("источник");
    let mut expected: serde_json::Value =
        serde_json::from_slice(&before.source.payload).expect("metadata");
    expected["name"] = serde_json::json!("Новое имя");
    let actual: serde_json::Value =
        serde_json::from_slice(&after.source.payload).expect("metadata");
    assert_eq!(actual, expected);
    assert_eq!(after.source.generation, before.source.generation);
    let before = before.snapshot.expect("snapshot");
    let after = after.snapshot.expect("snapshot");
    assert_eq!(after.body, before.body);
    assert_eq!(after.metadata, before.metadata);
    assert_eq!(after.validators, before.validators);
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.source_generation, before.source_generation);
    assert!(service.rename_source(id, "\n".into()).await.is_err());
}

#[tokio::test]
async fn delete_removes_only_target_and_invalidates_pending_commit() {
    let directory = TestDirectory::new();
    let fixture = Fixture::start(vec![ok(OLD), ok(NEW)]).await;
    let repository = directory.open().await;
    let service = SubscriptionService::new(repository.clone());
    let first = service.add(fixture.source()).await.expect("первый");
    install(&service, first).await;
    let second = service.add(fixture.source()).await.expect("второй");
    let pending = pending(&service, second).await;
    fixture.finish().await;
    service.delete_source(second).await.expect("удаление");
    assert!(repository.load(second).await.expect("БД").is_none());
    assert!(repository.load(first).await.expect("БД").is_some());
    assert_eq!(service.sources_page(0).await.expect("источники").total, 1);
    assert!(service.delete_source(second).await.is_err());
    let replacement = service
        .add(
            SubscriptionSource::new(
                "Новый".into(),
                "https://example.invalid/private".into(),
                false,
                FetchRoute::Direct,
            )
            .expect("источник"),
        )
        .await
        .expect("добавление после удаления");
    assert!(replacement > second);
    assert!(matches!(
        service
            .commit(pending.accept_clean().expect("импорт"))
            .await,
        Err(SubscriptionError::StaleUpdate)
    ));

    service
        .delete_source(replacement)
        .await
        .expect("удаление максимального ID");
    service
        .delete_source(first)
        .await
        .expect("удаление snapshot и профилей");
    assert_eq!(service.sources_page(0).await.expect("источники").total, 0);
}
