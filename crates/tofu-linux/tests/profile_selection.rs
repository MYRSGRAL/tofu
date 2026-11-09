#![cfg(target_os = "linux")]

use std::{
    fs::{self, DirBuilder},
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tofu_linux::{
    EncryptedSubscriptionRepository, FetchRoute, ProfileQuery, ProfileSort, RefreshResult,
    RepositoryError, SubscriptionError, SubscriptionService, SubscriptionSource,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::watch,
};
use zeroize::Zeroizing;

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tofu-selection-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("приватный каталог");
        Self(path)
    }
    async fn service(&self) -> SubscriptionService {
        let repository = EncryptedSubscriptionRepository::open(
            self.0.join("profiles.sqlite"),
            Zeroizing::new([33; 32]),
        )
        .await
        .expect("БД");
        SubscriptionService::new(repository)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("очистка своего каталога");
    }
}
async fn fixture(bodies: Vec<String>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let url = format!(
        "http://{}/subscription",
        listener.local_addr().expect("адрес")
    );
    let task = tokio::spawn(async move {
        for body in bodies {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
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
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.expect("ответ");
        }
    });
    (url, task)
}
async fn save(service: &SubscriptionService, id: i64) {
    let (_sender, cancel) = watch::channel(false);
    let result = service
        .refresh(id, Duration::from_secs(3), cancel)
        .await
        .expect("refresh");
    let RefreshResult::Pending(pending) = result else {
        panic!("ожидался новый snapshot")
    };
    service
        .commit(pending.accept_clean().expect("чистый отчёт"))
        .await
        .expect("commit");
}
fn source(url: String) -> SubscriptionSource {
    SubscriptionSource::new("Тест".into(), url, true, FetchRoute::Direct).expect("источник")
}
fn uri(key: &str, name: &str) -> String {
    format!("trojan://{key}@example.invalid:443#{name}")
}

#[tokio::test]
async fn service_selects_identity_after_rename_and_reorder_not_old_ordinal() {
    let dir = Directory::new();
    let service = dir.service().await;
    let (url, task) = fixture(vec![
        format!("{}\n{}", uri("first", "Первое"), uri("second", "Второе")),
        format!("{}\n{}", uri("second", "Второе"), uri("first", "Новое имя")),
    ])
    .await;
    let id = service.add(source(url)).await.expect("add");
    save(&service, id).await;
    let page = service
        .profiles(id, ProfileQuery::default())
        .await
        .expect("страница");
    let selected = page.profiles[0].id;
    service
        .set_favorite(selected, true)
        .await
        .expect("избранное");
    let immutable = service
        .select_profile(selected, 1)
        .await
        .expect("первый выбор");
    save(&service, id).await;
    assert!(matches!(
        service.select_profile(selected, 1).await,
        Err(SubscriptionError::Repository(RepositoryError::Conflict))
    ));
    let resolved = service
        .select_profile(selected, 2)
        .await
        .expect("актуальный выбор");
    assert_eq!(resolved.profile.ordinal, 1);
    assert!(resolved.profile.favorite);
    assert_eq!(resolved.profile.name, "Новое имя");
    assert_eq!(resolved.server.source().expose(), uri("first", "Новое имя"));
    // Уже подготовленная сессия вправе использовать прежний immutable server.
    assert_eq!(immutable.server.source().expose(), uri("first", "Первое"));
    task.await.expect("fixture");
    service.close().await.expect("закрытие");
}

#[tokio::test]
async fn service_favorites_survive_offline_reopen_removed_and_returned_profile() {
    let dir = Directory::new();
    let service = dir.service().await;
    let a = uri("first", "Первая запись");
    let b = uri("second", "Вторая запись");
    let (url, task) = fixture(vec![
        format!("{a}\n{b}"),
        b.clone(),
        format!("{b}\n{}", uri("first", "Возвращённая")),
    ])
    .await;
    let id = service.add(source(url)).await.expect("add");
    save(&service, id).await;
    let favorite = service
        .profiles(id, ProfileQuery::default())
        .await
        .expect("страница")
        .profiles[0]
        .id;
    service
        .set_favorite(favorite, true)
        .await
        .expect("избранное");
    save(&service, id).await;
    service.close().await.expect("закрытие");
    let service = dir.service().await;
    let query = ProfileQuery {
        favorites_only: true,
        include_unavailable: true,
        ..Default::default()
    };
    let page = service.profiles(id, query).await.expect("офлайн избранное");
    assert_eq!(page.total, 1);
    assert_eq!(page.profiles[0].id, favorite);
    assert!(!page.profiles[0].available);
    assert!(service.select_profile(favorite, 2).await.is_err());
    save(&service, id).await;
    let query = ProfileQuery {
        search: "ВОЗВРАЩЁННАЯ".into(),
        sort: ProfileSort::Name,
        ..Default::default()
    };
    let page = service.profiles(id, query).await.expect("поиск");
    assert_eq!(page.total, 1);
    assert_eq!(page.profiles[0].id, favorite);
    assert!(page.profiles[0].favorite);
    assert!(page.profiles[0].available);
    task.await.expect("fixture");
    service.close().await.expect("закрытие");
}

#[tokio::test]
async fn service_collapses_duplicate_labels_and_does_not_transfer_favorite_to_new_credentials() {
    let dir = Directory::new();
    let service = dir.service().await;
    let (url, task) = fixture(vec![
        format!("{}\n{}", uri("first", "Первое"), uri("first", "Дубликат")),
        uri("changed", "Первое"),
    ])
    .await;
    let id = service.add(source(url)).await.expect("add");
    save(&service, id).await;
    let page = service
        .profiles(id, ProfileQuery::default())
        .await
        .expect("страница");
    assert_eq!(page.raw_count, 2);
    assert_eq!(page.unique_count, 1);
    assert_eq!(page.duplicate_count, 1);
    assert_eq!(page.profiles[0].name, "Первое");
    let old = page.profiles[0].id;
    service.set_favorite(old, true).await.expect("избранное");
    save(&service, id).await;
    let page = service
        .profiles(id, ProfileQuery::default())
        .await
        .expect("новый профиль");
    assert_eq!(page.total, 1);
    assert_ne!(page.profiles[0].id, old);
    assert!(!page.profiles[0].favorite);
    task.await.expect("fixture");
    service.close().await.expect("закрытие");
}

#[tokio::test]
async fn service_rejects_unbounded_and_control_character_queries() {
    let dir = Directory::new();
    let service = dir.service().await;
    for query in [
        ProfileQuery {
            limit: 0,
            ..Default::default()
        },
        ProfileQuery {
            limit: 201,
            ..Default::default()
        },
        ProfileQuery {
            offset: 40_001,
            ..Default::default()
        },
        ProfileQuery {
            search: "x".repeat(513),
            ..Default::default()
        },
        ProfileQuery {
            search: "строка\nс управлением".into(),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            service.profiles(1, query).await,
            Err(SubscriptionError::Repository(RepositoryError::InvalidInput))
        ));
    }
    service.close().await.expect("закрытие");
}

#[tokio::test]
async fn favorite_changed_after_report_is_not_lost_at_snapshot_commit() {
    let dir = Directory::new();
    let service = dir.service().await;
    let (url, task) = fixture(vec![uri("first", "Старое имя"), uri("first", "Новое имя")]).await;
    let id = service.add(source(url)).await.expect("add");
    save(&service, id).await;
    let selected = service
        .profiles(id, ProfileQuery::default())
        .await
        .expect("список")
        .profiles[0]
        .id;
    let (_sender, cancel) = watch::channel(false);
    let RefreshResult::Pending(pending) = service
        .refresh(id, Duration::from_secs(3), cancel)
        .await
        .expect("refresh")
    else {
        panic!("pending")
    };
    service
        .set_favorite(selected, true)
        .await
        .expect("избранное после загрузки");
    service
        .commit(pending.accept_clean().expect("чистый отчёт"))
        .await
        .expect("commit");
    let page = service
        .profiles(id, ProfileQuery::default())
        .await
        .expect("список");
    assert!(page.profiles[0].favorite);
    assert_eq!(page.profiles[0].id, selected);
    assert_eq!(page.profiles[0].name, "Новое имя");
    task.await.expect("fixture");
    service.close().await.expect("закрытие");
}
