use yonik_engine::presets::Candidate;

#[test]
fn rejects_mixed_revision_and_invalid_lists() {
    assert!(Candidate::parse("a", "b", b"example.org\n", b"1.1.1.0/24\n").is_err());
    assert!(Candidate::parse("a", "a", b"https://example.org\n", b"1.1.1.0/24\n").is_err());
    assert!(Candidate::parse("a", "a", b"example.org\n", b"1.1.1.1/24\n").is_err());
    assert!(Candidate::parse("a", "a", b"", b"1.1.1.0/24\n").is_err());
}

use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Duration};
use yonik_linux::{
    FetchHost,
    presets::{Error, Metadata, Resource, Store},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let mut id = [0; 16];
        getrandom::getrandom(&mut id).expect("случайный путь");
        Self(std::env::temp_dir().join(format!("yonik-presets-{:x}", Sha256::digest(id))))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("удаление собственной fixture");
    }
}
fn metadata(version: &str, url: &str, domains: &[u8], ips: &[u8]) -> Metadata {
    let resource = |body: &[u8]| Resource {
        url: url.into(),
        revision: version.into(),
        sha256: format!("{:x}", Sha256::digest(body)),
    };
    Metadata {
        source_id: "fixture".into(),
        version: version.into(),
        format: "plain-domain-cidr-v1".into(),
        checked_at_unix: 1,
        enabled: true,
        domains: resource(domains),
        ips: resource(ips),
    }
}
const DOMAINS: &[u8] = b"example.org\n";
const IPS: &[u8] = b"0.0.0.0/0\n::/0\n";

#[test]
fn immutable_install_rollback_reopen_and_rejected_candidates() {
    let temp = Temp::new();
    let store = Store::open(&temp.0).expect("store");
    let m = metadata("v1", "https://example.org/list", DOMAINS, IPS);
    store
        .install(m.clone(), DOMAINS.to_vec(), IPS.to_vec())
        .expect("v1");
    let mut mixed = m.clone();
    mixed.ips.revision = "v2".into();
    assert_eq!(
        store.install(mixed, DOMAINS.to_vec(), IPS.to_vec()),
        Err(Error::Invalid)
    );
    assert_eq!(
        store.install(m.clone(), b"invalid/path".to_vec(), IPS.to_vec()),
        Err(Error::Invalid)
    );
    assert_eq!(
        store.install(m.clone(), vec![b'a'; 4 * 1024 * 1024 + 1], IPS.to_vec()),
        Err(Error::Invalid)
    );
    assert_eq!(
        store.install(
            metadata("bad", "https://example.org/list", b"https://bad/", IPS),
            b"https://bad/".to_vec(),
            IPS.to_vec()
        ),
        Err(Error::Invalid)
    );
    let mut secret = m;
    secret.domains.url = "https://example.org/list?token=secret".into();
    assert_eq!(
        store.install(secret, DOMAINS.to_vec(), IPS.to_vec()),
        Err(Error::Invalid)
    );
    store
        .install(
            metadata("v2", "https://example.org/list", DOMAINS, IPS),
            DOMAINS.to_vec(),
            IPS.to_vec(),
        )
        .expect("v2");
    // Остаток прерванного staging не участвует в выборе активной версии.
    std::fs::write(temp.0.join("stage-interrupted"), b"incomplete").expect("crash fixture");
    std::fs::set_permissions(
        temp.0.join("stage-interrupted"),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("private stage");
    let reopened = Store::open(&temp.0).expect("reopen");
    assert_eq!(
        reopened.active().expect("active").expect("some").0.version,
        "v2"
    );
    reopened.rollback().expect("rollback");
    assert_eq!(
        Store::open(&temp.0)
            .expect("reopen")
            .active()
            .expect("active")
            .expect("some")
            .0
            .version,
        "v1"
    );
    use std::os::unix::fs::PermissionsExt;
    for entry in std::fs::read_dir(&temp.0).expect("entries") {
        assert_eq!(
            entry
                .expect("entry")
                .metadata()
                .expect("metadata")
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}

#[tokio::test]
async fn loopback_update_and_failure_of_second_resource_preserve_active() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let temp = Temp::new();
    let store = Store::open(&temp.0).expect("store");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let url = format!("http://{}/list", listener.local_addr().expect("addr"));
    let server = tokio::spawn(async move {
        for body in [Some(DOMAINS), Some(IPS), Some(DOMAINS), None] {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.expect("request byte");
                request.push(byte[0]);
                assert!(request.len() <= 8192);
            }
            if let Some(body) = body {
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(header.as_bytes()).await.expect("header");
                socket.write_all(body).await.expect("body");
            } else {
                socket
                    .write_all(
                        b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .expect("failure");
            }
        }
    });
    let host = FetchHost::new(Duration::from_secs(3)).expect("host");
    let (_sender, cancel) = tokio::sync::watch::channel(false);
    store
        .update(
            &host,
            metadata("v1", &url, DOMAINS, IPS),
            true,
            cancel.clone(),
        )
        .await
        .expect("update");
    assert_eq!(
        store
            .update(&host, metadata("v2", &url, DOMAINS, IPS), true, cancel)
            .await,
        Err(Error::Fetch)
    );
    assert_eq!(
        Store::open(&temp.0)
            .expect("reopen")
            .active()
            .expect("active")
            .expect("some")
            .0
            .version,
        "v1"
    );
    server.await.expect("fixture");
}

#[test]
fn corrupt_snapshot_does_not_disable_healthy_pointer() {
    for damaged in ["active", "previous"] {
        let temp = Temp::new();
        let store = Store::open(&temp.0).expect("store");
        for version in ["v1", "v2"] {
            store
                .install(
                    metadata(version, "https://example.org/list", DOMAINS, IPS),
                    DOMAINS.to_vec(),
                    IPS.to_vec(),
                )
                .expect("install");
        }
        let index: serde_json::Value =
            serde_json::from_slice(&std::fs::read(temp.0.join("index")).expect("index"))
                .expect("index JSON");
        let path = temp.0.join(index[damaged].as_str().expect("snapshot ID"));
        std::fs::write(&path, b"damaged").expect("corrupt own fixture");
        let reopened =
            Store::open(&temp.0).expect("открытие не требует исправности обоих snapshots");
        if damaged == "active" {
            assert!(matches!(reopened.active(), Err(Error::Invalid)));
            reopened
                .rollback()
                .expect("восстановление исправного previous");
            assert_eq!(
                reopened
                    .active()
                    .expect("active")
                    .expect("snapshot")
                    .0
                    .version,
                "v1"
            );
            let index: serde_json::Value =
                serde_json::from_slice(&std::fs::read(temp.0.join("index")).expect("index"))
                    .expect("JSON");
            assert!(
                index["previous"].is_null(),
                "повреждённый active не становится резервом"
            );
        } else {
            assert_eq!(
                reopened
                    .active()
                    .expect("active")
                    .expect("snapshot")
                    .0
                    .version,
                "v2"
            );
            assert_eq!(reopened.rollback(), Err(Error::Invalid));
            reopened
                .install(
                    metadata("v3", "https://example.org/list", DOMAINS, IPS),
                    DOMAINS.to_vec(),
                    IPS.to_vec(),
                )
                .expect("update не требует исправного previous");
            reopened.rollback().expect("новый резерв v2");
            assert_eq!(
                reopened
                    .active()
                    .expect("active")
                    .expect("snapshot")
                    .0
                    .version,
                "v2"
            );
        }
        assert_eq!(std::fs::read(path).expect("original"), b"damaged");
    }
}

#[test]
fn install_preserves_healthy_previous_when_active_is_corrupt() {
    let temp = Temp::new();
    let store = Store::open(&temp.0).expect("store");
    for version in ["v1", "v2"] {
        store
            .install(
                metadata(version, "https://example.org/list", DOMAINS, IPS),
                DOMAINS.to_vec(),
                IPS.to_vec(),
            )
            .expect("install");
    }
    let index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.0.join("index")).expect("index"))
            .expect("index JSON");
    std::fs::write(
        temp.0.join(index["active"].as_str().expect("ID")),
        b"damaged",
    )
    .expect("corrupt own fixture");
    store
        .install(
            metadata("v3", "https://example.org/list", DOMAINS, IPS),
            DOMAINS.to_vec(),
            IPS.to_vec(),
        )
        .expect("install");
    store.rollback().expect("сохранён исправный резерв");
    assert_eq!(
        store.active().expect("active").expect("snapshot").0.version,
        "v1"
    );
}

#[test]
fn product_catalog_reads_previous_independently_of_corrupt_active() {
    let temp = Temp::new();
    let store = Store::open(&temp.0).expect("store");
    for version in ["v1", "v2"] {
        store
            .install(
                metadata(version, "https://example.org/list", DOMAINS, IPS),
                DOMAINS.to_vec(),
                IPS.to_vec(),
            )
            .expect("install");
    }
    let index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.0.join("index")).expect("index")).expect("json");
    let active = index["active"].as_str().expect("active");
    std::fs::write(temp.0.join(active), b"corrupt").expect("fault");
    assert!(store.active().is_err());
    assert_eq!(
        store
            .previous()
            .expect("independent previous")
            .expect("v1")
            .0
            .version,
        "v1"
    );
    store.rollback().expect("recover");
    assert_eq!(
        store
            .active()
            .expect("healthy active")
            .expect("v1")
            .0
            .version,
        "v1"
    );
    assert!(
        store
            .previous()
            .expect("corrupt old active excluded")
            .is_none()
    );
}
