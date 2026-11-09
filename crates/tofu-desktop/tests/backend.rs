#![cfg(target_os = "linux")]

use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tofu_desktop::{DesktopHost, HostConfig};

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tofu-desktop-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("собственный каталог");
        Self(path)
    }
    fn config(&self) -> HostConfig {
        HostConfig::new(self.0.clone(), self.0.join("engines")).with_ephemeral_key([77; 32])
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("очистка только своего тестового каталога");
    }
}
struct Harness {
    host: DesktopHost,
    receiver: mpsc::Receiver<String>,
    snapshots: Vec<String>,
}
impl Harness {
    fn new(config: HostConfig) -> Self {
        let (sender, receiver) = mpsc::channel();
        let host = DesktopHost::start(config, move |value| {
            let _ = sender.send(value);
        })
        .expect("backend thread");
        Self {
            host,
            receiver,
            snapshots: Vec::new(),
        }
    }
    fn wait(&mut self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let snapshot = self
                .receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("snapshot до deadline");
            let value: Value = serde_json::from_str(&snapshot).expect("JSON DTO");
            self.snapshots.push(snapshot);
            if predicate(&value) {
                return value;
            }
        }
    }
    fn send(&self, command: Value) {
        self.host.dispatch(&command.to_string()).expect("команда");
    }
    fn close(&mut self) {
        self.host.shutdown();
        self.wait(|value| value["closed"] == true);
        assert!(self.host.wait_closed(Duration::from_secs(2)));
    }
}

fn fixture(bodies: Vec<impl Into<String>>) -> (String, thread::JoinHandle<()>) {
    let bodies: Vec<String> = bodies.into_iter().map(Into::into).collect();
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback");
    listener.set_nonblocking(true).expect("nonblocking fixture");
    let url = format!(
        "http://{}/subscription?fixture-secret",
        listener.local_addr().expect("адрес")
    );
    let task = thread::spawn(move || {
        for body in bodies {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "ожидание локального запроса");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept fixture: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("timeout");
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                assert_eq!(stream.read(&mut byte).expect("HTTP запрос"), 1);
                request.push(byte[0]);
                assert!(request.len() < 16384);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("ответ fixture");
        }
    });
    (url, task)
}

#[test]
fn real_subscription_approval_revision_and_favorite() {
    const CLEAN: &str = "trojan://fixture-password@example.invalid:443#Первый";
    const PARTIAL: &str =
        "trojan://fixture-next-password@example.invalid:443#Второй\nunknown://broken";
    let directory = Directory::new();
    let (url, server) = fixture(vec![CLEAN, PARTIAL]);
    let mut app = Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"add_source","name":"Тестовый источник","url":url,"local_exception":true,"route":"direct"}));
    let added = app.wait(|v| v["source_id"].is_i64());
    assert_eq!(added["profiles"], json!([]));
    assert_eq!(added["sources_total"], 1);
    app.send(json!({"command":"refresh"}));
    let clean = app.wait(|v| v["profiles"].as_array().is_some_and(|p| p.len() == 1));
    assert!(clean["pending"].is_null());
    let id = clean["profiles"][0]["id"].as_i64().expect("profile id");
    let revision = clean["revision"].as_i64().expect("revision");
    app.send(json!({"command":"select_profile","id":id,"revision":revision}));
    assert_eq!(
        app.wait(|v| v["can_connect"] == true)["selected_name"],
        "Первый"
    );
    app.send(json!({"command":"favorite","id":id,"value":true}));
    app.wait(|v| v["profiles"][0]["favorite"] == true);
    app.send(json!({"command":"query","search":"Первый","favorites_only":true,"offset":0}));
    assert_eq!(
        app.wait(|v| v["total"] == 1)["profiles"][0]["favorite"],
        true
    );
    app.send(json!({"command":"query","search":"","favorites_only":false,"offset":0}));
    app.wait(|v| v["offset"] == 0);
    app.send(json!({"command":"refresh"}));
    let pending = app.wait(|v| !v["pending"].is_null());
    assert_eq!(pending["pending"]["accepted"], 1);
    assert_eq!(pending["pending"]["rejected"], 1);
    assert_eq!(pending["revision"], revision);
    assert_eq!(pending["profiles"][0]["name"], "Первый");
    assert_eq!(pending["can_connect"], false);
    app.send(json!({"command":"approve_refresh"}));
    let approved = app.wait(|v| v["revision"].as_i64().is_some_and(|r| r > revision));
    assert!(approved["selected_profile"].is_null());
    let available = approved["profiles"]
        .as_array()
        .expect("профили")
        .iter()
        .find(|profile| profile["available"] == true)
        .expect("доступный профиль");
    assert_eq!(available["name"], "Второй");
    app.send(json!({"command":"select_profile","id":id,"revision":revision}));
    app.wait(|v| v["error"] == "profile_unavailable");
    // Отсутствующее ядро не создаёт процесс и не мешает подтверждённому закрытию.
    let current = available["id"].as_i64().expect("новый id");
    app.send(json!({"command":"select_profile","id":current,"revision":approved["revision"]}));
    app.wait(|v| v["can_connect"] == true);
    app.send(json!({"command":"connect"}));
    app.wait(|v| v["error"] == "engine_missing" && v["can_stop"] == false);
    app.close();
    server.join().expect("HTTP fixture завершён");
    for snapshot in &app.snapshots {
        assert!(!snapshot.contains("fixture-password"));
        assert!(!snapshot.contains("fixture-next-password"));
        assert!(!snapshot.contains("fixture-secret"));
        assert!(!snapshot.contains("trojan://"));
    }
    let stored = fs::read(directory.0.join("subscriptions.sqlite")).expect("зашифрованная БД");
    assert!(
        !stored
            .windows(CLEAN.len())
            .any(|bytes| bytes == CLEAN.as_bytes())
    );
}

#[test]
fn review_reasons_are_paginated_and_do_not_expose_source_data() {
    let directory = Directory::new();
    let body = format!(
        "trojan://review-secret@example.invalid:443?allowInsecure=1&private-secret=hidden-value#Секрет\n{}",
        vec!["unknown://rejected-secret"; 103].join("\n")
    );
    let (url, server) = fixture(vec![body]);
    let mut app = Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"add_source","name":"Fixture","url":url,"local_exception":true,"route":"direct"}));
    app.wait(|v| v["source_id"].is_number());
    app.send(json!({"command":"refresh"}));
    let first = app.wait(|v| !v["pending"].is_null());
    assert_eq!(first["pending"]["rows_total"], 105);
    assert_eq!(
        first["pending"]["rows"].as_array().expect("строки").len(),
        100
    );
    assert_eq!(
        first["pending"]["rows"][0],
        json!({"kind":"rejected","position":2,"code":"unsupported_protocol","field":"source"})
    );
    app.send(json!({"command":"review_page","offset":100}));
    let last = app.wait(|v| v["pending"]["rows_offset"] == 100);
    let rows = last["pending"]["rows"]
        .as_array()
        .expect("последняя страница");
    assert_eq!(rows.len(), 5);
    assert_eq!(
        rows[3],
        json!({"kind":"flagged","position":1,"code":"unknown_parameter","field":null})
    );
    assert_eq!(rows[4]["code"], "unsafe_option");
    app.send(json!({"command":"review_page","offset":106}));
    app.wait(|v| v["error"] == "invalid_input");
    app.send(json!({"command":"discard_refresh"}));
    app.wait(|v| v["pending"].is_null() && v["error"].is_null());
    app.close();
    server.join().expect("HTTP fixture завершён");
    for snapshot in &app.snapshots {
        for secret in [
            "review-secret",
            "private-secret",
            "hidden-value",
            "rejected-secret",
            "example.invalid",
            "trojan://",
            "fixture-secret",
        ] {
            assert!(!snapshot.contains(secret), "секрет не попадает в DTO");
        }
    }
}

#[test]
fn native_json_requires_approval_and_never_becomes_profile() {
    let directory = Directory::new();
    let (url, server) = fixture(vec![r#"{"outbounds":[{"type":"direct"}]}"#]);
    let mut app = Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"add_source","name":"Opaque","url":url,"local_exception":true,"route":"direct"}));
    app.wait(|v| v["source_id"].is_i64());
    app.send(json!({"command":"refresh"}));
    app.wait(|v| v["pending"]["native_json"] == true);
    app.send(json!({"command":"approve_refresh"}));
    let value = app.wait(|v| v["revision"].as_i64().is_some_and(|r| r > 0));
    assert_eq!(value["profiles"], json!([]));
    assert_eq!(value["can_connect"], false);
    app.close();
    server.join().expect("fixture");
}

#[test]
fn strict_bounded_commands_and_priority_shutdown() {
    let directory = Directory::new();
    let blocked = Arc::new((Mutex::new(false), Condvar::new()));
    let gate = blocked.clone();
    let (sender, receiver) = mpsc::channel();
    let host = DesktopHost::start(directory.config(), move |snapshot| {
        let _ = sender.send(snapshot);
        let (lock, condition) = &*gate;
        let mut released = lock.lock().expect("lock");
        while !*released {
            released = condition.wait(released).expect("wait");
        }
    })
    .expect("start");
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("готовность actor");
    for invalid in [
        r#"{"command":"reload","unknown":1}"#,
        r#"{"command":"select_profile","id":1,"revision":0}"#,
        r#"{"command":"query","search":"\u0000","favorites_only":false,"offset":0}"#,
        r#"{"command":"set_engine","engine":"invalid"}"#,
        "{}",
    ] {
        assert_eq!(
            host.dispatch(invalid).expect_err("strict command").key(),
            "invalid_input"
        );
    }
    assert_eq!(
        host.dispatch(&" ".repeat(16385)).expect_err("size").key(),
        "invalid_input"
    );
    for _ in 0..64 {
        host.dispatch(r#"{"command":"connect"}"#)
            .expect("queued connect");
        host.dispatch(r#"{"command":"refresh"}"#)
            .expect("queued refresh");
    }
    assert_eq!(
        host.dispatch(r#"{"command":"reload"}"#)
            .expect_err("bounded queue")
            .key(),
        "busy"
    );
    host.dispatch(r#"{"command":"stop"}"#)
        .expect("priority Stop bypasses full queue");
    host.shutdown();
    let (lock, condition) = &*blocked;
    *lock.lock().expect("lock") = true;
    condition.notify_all();
    assert!(host.wait_closed(Duration::from_secs(10)));
    let snapshots: Vec<Value> = receiver
        .try_iter()
        .map(|s| serde_json::from_str(&s).expect("DTO"))
        .collect();
    assert!(snapshots.iter().any(|s| s["closed"] == true));
    assert!(
        snapshots
            .iter()
            .all(|s| s["error"].is_null() && s["status"] != "Preparing")
    );
}

#[test]
fn startup_failure_remains_closable_and_lock_is_exclusive() {
    let directory = Directory::new();
    let mut first = Harness::new(directory.config());
    first.wait(|v| v["ready"] == true);
    let mut second = Harness::new(directory.config());
    second.wait(|v| v["error"] == "busy");
    second.close();
    first.close();
    let mut reopened = Harness::new(directory.config());
    reopened.wait(|v| v["ready"] == true);
    reopened.close();
    let invalid = HostConfig::new(
        PathBuf::from("relative-forbidden"),
        PathBuf::from("missing"),
    )
    .with_ephemeral_key([1; 32]);
    let mut failed = Harness::new(invalid);
    failed.wait(|v| v["error"] == "storage");
    failed.close();
}

#[test]
fn existing_directory_permissions_are_never_rewritten() {
    let directory = Directory::new();
    fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o755)).expect("fixture права");
    let mut app = Harness::new(directory.config());
    app.wait(|value| value["error"] == "storage");
    app.close();
    assert_eq!(
        fs::metadata(&directory.0)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}

#[test]
fn proxy_source_has_explicit_capability_error_and_custom_port_is_reported() {
    let directory = Directory::new();
    let config = directory
        .config()
        .with_listen_port(std::num::NonZeroU16::new(18888).expect("порт"));
    let mut app = Harness::new(config);
    assert_eq!(app.wait(|value| value["ready"] == true)["port"], 18888);
    app.send(json!({"command":"add_source","name":"Proxy","url":"https://fixture.invalid/invented-secret","local_exception":false,"route":"proxy"}));
    let rejected = app.wait(|value| value["error"] == "unsupported_proxy");
    assert_eq!(rejected["sources_total"], 0);
    app.close();
}

#[test]
fn shutdown_cancels_inflight_fetch_before_close_ack() {
    let directory = Directory::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback");
    listener.set_nonblocking(true).expect("fixture listener");
    let url = format!("http://{}/slow", listener.local_addr().expect("address"));
    let (accepted, request) = mpsc::channel();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("deadline");
        let mut bytes = [0; 4096];
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let count = stream.read(&mut bytes).expect("request");
            assert!(count > 0);
            request.extend_from_slice(&bytes[..count]);
            assert!(request.len() <= 16384);
        }
        accepted.send(()).expect("accepted");
        // Ни ответа, ни sleep: отмена FetchHost закрывает соединение.
        assert_eq!(stream.read(&mut bytes).expect("client close"), 0);
    });
    let mut app = Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"add_source","name":"Slow","url":url,"local_exception":true,"route":"direct"}));
    app.wait(|v| v["source_id"].is_i64());
    app.send(json!({"command":"refresh"}));
    app.wait(|v| v["busy"] == true);
    request
        .recv_timeout(Duration::from_secs(5))
        .expect("fetch started");
    let started = Instant::now();
    app.close();
    assert!(started.elapsed() < Duration::from_secs(3));
    server.join().expect("fetch socket closed");
}
