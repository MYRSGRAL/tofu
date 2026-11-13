//! Контролируемый HTTP body проходит настоящий FetchHost, parser и scheduler.
use std::{num::NonZeroU64, os::unix::fs::DirBuilderExt, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use yonik_linux::{
    EncryptedSubscriptionRepository, FetchHost, FetchRoute, ProfileQuery, SubscriptionError,
    SubscriptionScheduleEvent, SubscriptionScheduler, SubscriptionService, SubscriptionSource,
};
use yonik_subscription::scheduler::Policy;

#[tokio::test]
async fn отказ_импорта_сохраняет_рабочий_snapshot() {
    let root = std::env::temp_dir().join(format!("yonik-import-failure-{}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .expect("свой каталог");
    let repository =
        EncryptedSubscriptionRepository::open(root.join("db"), zeroize::Zeroizing::new([53; 32]))
            .await
            .expect("БД");
    let service = SubscriptionService::new(repository.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("loopback");
    let id = service
        .add(
            SubscriptionSource::new(
                "Импорт".into(),
                format!("http://{}/sub", listener.local_addr().expect("адрес")),
                true,
                FetchRoute::Direct,
            )
            .expect("источник"),
        )
        .await
        .expect("add");
    let server = tokio::spawn(async move {
        for body in ["socks5://127.0.0.1:1080#Рабочий", "{broken json"] {
            let (mut stream, _) = listener.accept().await.expect("запрос");
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                assert_eq!(stream.read(&mut byte).await.expect("HTTP"), 1);
                request.push(byte[0]);
                assert!(request.len() < 32768);
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
                .expect("body");
        }
    });
    let workflow = async {
        let mut scheduler = SubscriptionScheduler::new(NonZeroU64::MIN, 1, 1).expect("scheduler");
        let host = Arc::new(FetchHost::new(Duration::from_secs(3)).expect("host"));
        let policy = Policy {
            online: true,
            ui_visible: true,
            ..Policy::default()
        };
        scheduler.schedule(id, 1, Duration::ZERO).expect("schedule");
        scheduler
            .dispatch(Duration::ZERO, policy, &service, host.clone())
            .expect("dispatch");
        assert!(
            matches!(scheduler.next_event(|| Duration::ZERO).await.expect("event"), Some(SubscriptionScheduleEvent::Review { id: current }) if current == id)
        );
        scheduler
            .finish_review(&service, id, true, || Duration::ZERO)
            .await
            .expect("commit");
        let saved = repository
            .load(id)
            .await
            .expect("load")
            .expect("source")
            .snapshot
            .expect("original");
        let before = service
            .profiles(id, ProfileQuery::default())
            .await
            .expect("snapshot");
        assert_eq!(before.profiles.len(), 1);
        let selected = before.profiles[0].id;
        let original = service.select_profile(selected, 1).await.expect("original");
        let mut failed = SubscriptionScheduler::new(NonZeroU64::new(2).expect("identity"), 1, 1)
            .expect("scheduler");
        failed.schedule(id, 1, Duration::ZERO).expect("schedule");
        failed
            .dispatch(Duration::ZERO, policy, &service, host)
            .expect("dispatch");
        assert!(
            matches!(failed.next_event(|| Duration::ZERO).await.expect("event"), Some(SubscriptionScheduleEvent::Failed { id: current, error: SubscriptionError::Import(_) }) if current == id)
        );
        let after = service
            .profiles(id, ProfileQuery::default())
            .await
            .expect("snapshot после отказа");
        assert_eq!(after.profiles.len(), 1);
        assert_eq!(after.profiles[0].id, selected);
        let retained = service
            .select_profile(selected, 1)
            .await
            .expect("original после отказа");
        assert_eq!(retained.profile.name, original.profile.name);
        let unchanged = repository
            .load(id)
            .await
            .expect("load")
            .expect("source")
            .snapshot
            .expect("original");
        assert_eq!(unchanged.revision, saved.revision);
        assert_eq!(unchanged.body.as_slice(), saved.body.as_slice());
        assert_eq!(unchanged.metadata.as_slice(), saved.metadata.as_slice());
        scheduler.shutdown(Duration::ZERO).await.expect("shutdown");
        failed.shutdown(Duration::ZERO).await.expect("shutdown");
    };
    tokio::time::timeout(Duration::from_secs(15), workflow)
        .await
        .expect("deadline");
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .expect("server deadline")
        .expect("server joined");
    tokio::time::timeout(Duration::from_secs(3), service.close())
        .await
        .expect("close deadline")
        .expect("close");
    std::fs::remove_dir_all(root).expect("очистка своего каталога");
}
