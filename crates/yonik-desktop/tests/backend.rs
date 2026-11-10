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
use yonik_desktop::{DesktopHost, HostConfig};

static NEXT: AtomicU64 = AtomicU64::new(1);
pub(crate) struct Directory(pub(crate) PathBuf);
impl Directory {
    pub(crate) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yonik-desktop-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("собственный каталог");
        Self(path)
    }
    pub(crate) fn config(&self) -> HostConfig {
        HostConfig::new(self.0.clone(), self.0.join("engines")).with_ephemeral_key([77; 32])
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("очистка только своего тестового каталога");
    }
}
pub(crate) struct Harness {
    pub(crate) host: DesktopHost,
    receiver: mpsc::Receiver<String>,
    snapshots: Vec<String>,
}
impl Harness {
    pub(crate) fn new(config: HostConfig) -> Self {
        Self::with_hook(config, |_| {})
    }
    fn with_hook(config: HostConfig, hook: impl Fn(&str) + Send + Sync + 'static) -> Self {
        let (sender, receiver) = mpsc::channel();
        let host = DesktopHost::start(config, move |value| {
            let _ = sender.send(value.clone());
            hook(&value);
        })
        .expect("backend thread");
        Self {
            host,
            receiver,
            snapshots: Vec::new(),
        }
    }
    pub(crate) fn wait(&mut self, predicate: impl Fn(&Value) -> bool) -> Value {
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
    pub(crate) fn send(&self, command: Value) {
        self.host.dispatch(&command.to_string()).expect("команда");
    }
    pub(crate) fn close(&mut self) {
        self.host.shutdown();
        self.wait(|value| value["closed"] == true);
        assert!(self.host.wait_closed(Duration::from_secs(2)));
    }
}

fn fixture(bodies: Vec<impl Into<String>>) -> (String, thread::JoinHandle<()>) {
    fixture_with_requests(bodies, |_| {})
}

fn fixture_with_requests(
    bodies: Vec<impl Into<String>>,
    inspect: impl Fn(&[u8]) + Send + 'static,
) -> (String, thread::JoinHandle<()>) {
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
            inspect(&request);
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
    app.send(json!({"command":"rename_source","id":added["source_id"],"name":"Переименован"}));
    let renamed = app.wait(|v| v["source_name"] == "Переименован");
    assert_eq!(renamed["source_id"], added["source_id"]);
    assert_eq!(renamed["selected_profile"], id);
    assert_eq!(renamed["revision"], revision);
    assert_eq!(renamed["profiles"], clean["profiles"]);
    assert_eq!(renamed["busy"], false);
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
fn listen_port_settings_are_strict_and_survive_reload() {
    let directory = Directory::new();
    let config = directory
        .config()
        .with_listen_port(std::num::NonZeroU16::new(18888).expect("порт"));
    let mut app = Harness::new(config);
    assert_eq!(app.wait(|v| v["ready"] == true)["port"], 18888);
    // Занятый порт допустим: применение настройки не открывает listener.
    let listener = TcpListener::bind("127.0.0.1:0").expect("свой listener");
    let port = listener.local_addr().expect("порт").port();
    app.send(json!({"command":"set_listen_port","port":port}));
    app.wait(|v| v["port"] == port);
    for (index, invalid) in [
        json!({"command":"set_listen_port","port":0}),
        json!({"command":"set_listen_port","port":65536}),
        json!({"command":"set_listen_port","port":-1}),
        json!({"command":"set_listen_port","port":"10808"}),
        json!({"command":"set_listen_port","port":true}),
        json!({"command":"set_listen_port","port":1.5}),
        json!({"command":"set_listen_port","port":null}),
        json!({"command":"set_listen_port"}),
        json!({"command":"set_listen_port","port":12345,"unknown":true}),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            app.host
                .dispatch(&invalid.to_string())
                .expect_err("строгая команда")
                .key(),
            "invalid_input"
        );
        app.send(json!({"command":"reload"}));
        // Смена ядра после reload служит FIFO-барьером: случайный сетевой
        // snapshot не должен выдаваться за подтверждение обработки reload.
        let engine = if index % 2 == 0 { "sing-box" } else { "xray" };
        app.send(json!({"command":"set_engine","engine":engine}));
        assert_eq!(app.wait(|v| v["engine"] == engine)["port"], port);
    }
    for boundary in [1, 65535] {
        app.send(json!({"command":"set_listen_port","port":boundary}));
        app.wait(|v| v["port"] == boundary);
    }
    app.close();
}

#[test]
fn next_connect_compiles_with_updated_listen_port() {
    let directory = Directory::new();
    let (url, server) = fixture(vec!["trojan://port-secret@127.0.0.1:18889#Порт"]);
    let config = directory
        .config()
        .with_listen_port(std::num::NonZeroU16::new(18888).expect("порт"));
    let mut app = Harness::new(config);
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"add_source","name":"Порт","url":url,"local_exception":true,"route":"direct"}));
    app.wait(|v| v["source_id"].is_number());
    app.send(json!({"command":"refresh"}));
    let imported = app.wait(|v| v["total"] == 1 && v["busy"] == false);
    app.send(json!({"command":"select_profile","id":imported["profiles"][0]["id"],"revision":imported["revision"]}));
    app.wait(|v| v["can_connect"] == true);
    app.send(json!({"command":"connect"}));
    app.wait(|v| v["error"] == "engine_missing" && v["can_stop"] == false);
    app.send(json!({"command":"set_listen_port","port":18889}));
    app.wait(|v| v["port"] == 18889);
    app.send(json!({"command":"reload"}));
    app.send(json!({"command":"set_engine","engine":"sing-box"}));
    assert_eq!(app.wait(|v| v["engine"] == "sing-box")["port"], 18889);
    app.send(json!({"command":"set_engine","engine":"xray"}));
    app.wait(|v| v["engine"] == "xray");
    // Совпадение loopback endpoint и listener отклоняется реальным compile
    // до загрузки ядра; прежний порт привёл бы к engine_missing.
    app.send(json!({"command":"connect"}));
    let rejected = app.wait(|v| v["error"] == "engine_config");
    assert_eq!(rejected["can_stop"], false);
    app.close();
    server.join().expect("HTTP fixture завершён");
}

#[test]
fn health_target_settings_are_validated_without_disclosing_url() {
    let directory = Directory::new();
    let mut app = Harness::new(directory.config());
    let initial = app.wait(|v| v["ready"] == true);
    assert_eq!(initial["probe_custom"], false);
    assert_eq!(initial["probe_expected_status"], 204);
    app.send(json!({"command":"set_probe_target","url":"https://health.invalid/private?health-token=hidden","expected_status":200}));
    let custom = app.wait(|v| v["probe_custom"] == true);
    assert_eq!(custom["probe_expected_status"], 200);
    assert_eq!(custom["probe_persistent"], false);
    let stored = fs::read_to_string(directory.0.join("preferences.json")).expect("настройки");
    assert!(!stored.contains("health-token"));
    assert!(!stored.contains("health.invalid"));
    for (url, status) in [
        ("http://health.invalid/", 204),
        ("https://user:secret@health.invalid/", 204),
        ("https://health.invalid/", 302),
    ] {
        app.send(json!({"command":"set_probe_target","url":url,"expected_status":status}));
        let invalid = app.wait(|v| v["error"] == "probe_target_invalid");
        assert_eq!(invalid["probe_custom"], true);
        assert_eq!(invalid["probe_expected_status"], 200);
    }
    app.send(json!({"command":"reset_probe_target"}));
    app.wait(|v| {
        v["probe_custom"] == false && v["probe_expected_status"] == 204 && v["error"].is_null()
    });
    app.close();
    for snapshot in &app.snapshots {
        for secret in ["health.invalid", "health-token", "hidden", "user:secret"] {
            assert!(!snapshot.contains(secret));
        }
    }
}

#[test]
fn tcp_check_is_separate_from_session_and_bound_to_selected_profile() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("TCP fixture");
    let port = listener.local_addr().expect("TCP адрес").port();
    listener.set_nonblocking(true).expect("nonblocking");
    let (closed_sender, closed) = mpsc::channel();
    let peer = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        for _ in 0..3 {
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline);
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("TCP accept: {error}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .expect("timeout");
            assert_eq!(socket.read(&mut [0]).expect("TCP EOF"), 0);
        }
        closed_sender.send(()).expect("EOF ack");
    });
    let directory = Directory::new();
    let (url, server) = fixture(vec![format!(
        "trojan://tcp-secret@127.0.0.1:{port}#Доступный\ntrojan://other-secret@127.0.0.1:{port}#Второй"
    )]);
    let blocked = Arc::new((Mutex::new(false), Condvar::new()));
    let gate = blocked.clone();
    let busy_count = AtomicU64::new(0);
    let mut app = Harness::with_hook(directory.config(), move |snapshot| {
        let value: Value = serde_json::from_str(snapshot).expect("DTO");
        if value["tcp_busy"] == true && busy_count.fetch_add(1, Ordering::Relaxed) == 1 {
            let (lock, condition) = &*gate;
            let mut released = lock.lock().expect("gate lock");
            while !*released {
                released = condition.wait(released).expect("gate wait");
            }
        }
    });
    app.wait(|v| v["ready"] == true);
    assert_eq!(
        app.host
            .dispatch(r#"{"command":"check_tcp","unexpected":true}"#)
            .expect_err("strict TCP command")
            .key(),
        "invalid_input"
    );
    app.send(json!({"command":"check_tcp"}));
    // Готовность наблюдателя может инвалидировать уже принятую команду.
    let missing = app.wait(|v| v["error"] == "profile_required" || v["error"] == "cancelled");
    assert_eq!(missing["tcp_busy"], false);
    assert!(missing["tcp_result"].is_null());
    app.send(json!({"command":"add_source","name":"TCP fixture","url":url,"local_exception":true,"route":"direct"}));
    app.wait(|v| v["source_id"].is_number());
    app.send(json!({"command":"refresh"}));
    let imported = app.wait(|v| v["total"] == 2 && v["busy"] == false);
    let id = imported["profiles"][0]["id"].as_i64().expect("id");
    let other = imported["profiles"][1]["id"].as_i64().expect("другой id");
    let revision = imported["revision"].as_i64().expect("revision");
    app.send(json!({"command":"select_profile","id":id,"revision":revision}));
    app.wait(|v| v["selected_profile"] == id);
    app.send(json!({"command":"check_tcp"}));
    let reachable = app.wait(|v| v["tcp_result"]["status"] == "reachable");
    assert_eq!(reachable["tcp_result"]["profile_id"], id);
    assert_eq!(reachable["tcp_result"]["revision"], revision);
    assert_eq!(reachable["tcp_result"]["dns_ms"], Value::Null);
    assert!(reachable["tcp_result"]["connect_ms"].is_u64());
    let samples = reachable["tcp_result"]["samples_ms"]
        .as_array()
        .expect("три TCP-выборки");
    assert_eq!(samples.len(), 3);
    assert_eq!(reachable["tcp_result"]["samples_requested"], 3);
    assert_eq!(
        reachable["tcp_result"]["selected_address"],
        format!("127.0.0.1:{port}")
    );
    let mut sorted: Vec<u64> = samples
        .iter()
        .map(|sample| sample.as_u64().expect("TCP время"))
        .collect();
    sorted.sort_unstable();
    assert_eq!(reachable["tcp_result"]["connect_ms"], sorted[1]);
    assert!(
        reachable["tcp_result"]["checked_at_ms"]
            .as_u64()
            .expect("время")
            > 0
    );
    assert_eq!(reachable["tcp_busy"], false);
    assert_eq!(reachable["status"], "Disconnected");
    assert_eq!(reachable["can_stop"], false);
    closed
        .recv_timeout(Duration::from_secs(3))
        .expect("сокет проверки закрыт");
    peer.join().expect("TCP peer завершён");
    // Останавливаем actor после spawn, до polling TCP future: выбор меняется
    // в очереди, пока callback старой проверки ещё не обработан.
    app.send(json!({"command":"check_tcp"}));
    app.wait(|v| v["tcp_busy"] == true);
    app.send(json!({"command":"select_profile","id":other,"revision":revision}));
    let (lock, condition) = &*blocked;
    *lock.lock().expect("gate lock") = true;
    condition.notify_all();
    let selected = app.wait(|v| v["selected_profile"] == other && v["tcp_busy"] == false);
    assert!(selected["tcp_result"].is_null());
    // Listener уже закрыт; повторная TCP-проверка не превращается в Connect ядра.
    app.send(json!({"command":"check_tcp"}));
    let refused = app.wait(|v| v["tcp_result"]["status"] == "connect_failed");
    assert_eq!(refused["tcp_result"]["profile_id"], other);
    assert_eq!(refused["status"], "Disconnected");
    app.close();
    server.join().expect("HTTP fixture завершён");
    for snapshot in &app.snapshots {
        assert!(!snapshot.contains("tcp-secret"));
        assert!(!snapshot.contains("other-secret"));
        assert!(!snapshot.contains("trojan://"));
    }
}

#[test]
fn tcp_cancel_bypasses_full_queue_and_invalidates_queued_probe() {
    let directory = Directory::new();
    let blocked = Arc::new((Mutex::new(false), Condvar::new()));
    let gate = blocked.clone();
    let mut app = Harness::with_hook(directory.config(), move |_| {
        let (lock, condition) = &*gate;
        let mut released = lock.lock().expect("gate lock");
        while !*released {
            released = condition.wait(released).expect("gate wait");
        }
    });
    app.wait(|_| true);
    for _ in 0..127 {
        app.send(json!({"command":"reload"}));
    }
    app.send(json!({"command":"check_tcp"}));
    assert_eq!(
        app.host
            .dispatch(r#"{"command":"reload"}"#)
            .expect_err("queue limit")
            .key(),
        "busy"
    );
    app.send(json!({"command":"cancel_tcp"}));
    let (lock, condition) = &*blocked;
    *lock.lock().expect("gate lock") = true;
    condition.notify_all();
    let cancelled = app.wait(|v| v["error"] == "cancelled");
    assert_eq!(cancelled["tcp_busy"], false);
    assert_eq!(cancelled["status"], "Disconnected");
    assert!(cancelled["tcp_result"].is_null());
    app.close();
}

#[test]
fn stop_and_shutdown_finish_a_pending_tcp_task_before_acknowledgement() {
    for shutdown in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("свой TCP listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().expect("порт").port();
        let directory = Directory::new();
        let (url, server) = fixture(vec![format!(
            "trojan://pending-secret@127.0.0.1:{port}#Pending"
        )]);
        let blocked = Arc::new((Mutex::new(false), Condvar::new()));
        let gate = blocked.clone();
        let mut app = Harness::with_hook(directory.config(), move |snapshot| {
            let value: Value = serde_json::from_str(snapshot).expect("DTO");
            if value["tcp_busy"] == true {
                let (lock, condition) = &*gate;
                let mut released = lock.lock().expect("gate lock");
                while !*released {
                    released = condition.wait(released).expect("gate wait");
                }
            }
        });
        app.wait(|v| v["ready"] == true);
        app.send(json!({"command":"add_source","name":"Pending","url":url,"local_exception":true,"route":"direct"}));
        app.wait(|v| v["source_id"].is_number());
        app.send(json!({"command":"refresh"}));
        let imported = app.wait(|v| v["total"] == 1 && v["busy"] == false);
        app.send(json!({"command":"select_profile","id":imported["profiles"][0]["id"],"revision":imported["revision"]}));
        app.wait(|v| v["selected_profile"].is_number());
        app.send(json!({"command":"check_tcp"}));
        app.wait(|v| v["tcp_busy"] == true);
        if shutdown {
            app.host.shutdown();
        } else {
            app.send(json!({"command":"stop"}));
        }
        let (lock, condition) = &*blocked;
        *lock.lock().expect("gate lock") = true;
        condition.notify_all();
        if shutdown {
            let closed = app.wait(|v| v["closed"] == true);
            assert_eq!(closed["tcp_busy"], false);
            assert!(app.host.wait_closed(Duration::from_secs(2)));
        } else {
            let cancelled =
                app.wait(|v| v["tcp_busy"] == false && v["tcp_result"]["status"] == "cancelled");
            assert_eq!(cancelled["status"], "Disconnected");
            app.close();
        }
        assert_eq!(
            listener
                .accept()
                .expect_err("отмена до polling connect")
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        server.join().expect("HTTP fixture завершён");
    }
}

#[test]
fn xray_json_auto_import_exposes_profiles_only_after_review() {
    let directory = Directory::new();
    let body = json!([{"remarks":"JSON-сервер","outbounds":[{
        "protocol":"vless", "settings":{"vnext":[{
            "address":"example.invalid", "port":443,
            "users":[{"id":"11111111-1111-4111-8111-111111111111","encryption":"none"}]
        }]}, "streamSettings":{"network":"tcp","security":"tls",
            "tlsSettings":{"serverName":"example.invalid","enableSessionResumption":false}}
    }]}])
    .to_string();
    let (url, server) = fixture(vec![body]);
    let mut app = Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"import_source","name":"JSON","url":url,"local_exception":true,"route":"direct"}));
    let pending = app.wait(|v| v["pending"]["native_json"] == true && v["busy"] == false);
    assert_eq!(pending["pending"]["accepted"], 1);
    assert_eq!(pending["pending"]["rejected"], 0);
    assert_eq!(pending["total"], 0);
    assert_eq!(pending["can_connect"], false);
    app.send(json!({"command":"approve_refresh"}));
    let imported = app.wait(|v| v["total"] == 1 && v["pending"].is_null());
    app.send(json!({"command":"select_profile","id":imported["profiles"][0]["id"],"revision":imported["revision"]}));
    app.wait(|v| v["can_connect"] == true);
    app.send(json!({"command":"connect"}));
    // До engine lookup прошла настоящая компиляция нормализованного профиля.
    app.wait(|v| v["error"] == "engine_missing" && v["can_stop"] == false);
    app.close();
    server.join().expect("fixture");
    assert!(
        app.snapshots
            .iter()
            .all(|v| !v.contains("11111111-1111-4111-8111-111111111111"))
    );
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
        r#"{"command":"rename_source","id":1,"name":" "}"#,
        r#"{"command":"rename_source","id":1,"name":"Имя","url":"hidden"}"#,
        r#"{"command":"rename_source","id":1,"name":"Имя\n"}"#,
        r#"{"command":"delete_source","id":0}"#,
        r#"{"command":"delete_source","id":1,"confirmed":true}"#,
        r#"{"command":"cancel_refresh","id":-1}"#,
        r#"{"command":"cancel_refresh","id":1,"unknown":true}"#,
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

#[test]
fn import_and_hwid_toggle_stop_and_shutdown_cancel_fetch_and_reject_concurrent_changes() {
    for (shutdown, toggle) in [(false, false), (true, false), (false, true), (true, true)] {
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
        if toggle {
            app.send(json!({"command":"add_source","name":"Slow","url":url,"local_exception":true,"route":"direct"}));
            app.wait(|v| v["sources_total"] == 1);
            app.send(json!({"command":"set_source_hwid","enabled":true}));
        } else {
            app.send(json!({"command":"import_source","name":"Slow","url":url,"local_exception":true,"route":"direct"}));
        }
        app.wait(|v| v["busy"] == true);
        request
            .recv_timeout(Duration::from_secs(5))
            .expect("fetch started");
        app.send(json!({"command":"import_source","name":"Concurrent","url":url,"local_exception":true,"route":"direct"}));
        let rejected = app.wait(|v| v["error"] == "busy");
        app.send(json!({"command":"set_source_hwid","enabled":!toggle}));
        let toggle_rejected = app.wait(|v| v["error"] == "busy");
        assert_eq!(toggle_rejected["source_hwid_enabled"], toggle);
        assert_eq!(rejected["sources_total"], 1);
        assert_eq!(rejected["busy"], true);
        let started = Instant::now();
        if !shutdown {
            app.send(json!({"command":"stop"}));
            let stopped = app.wait(|v| v["busy"] == false);
            assert_eq!(stopped["total"], 0);
            assert!(stopped["pending"].is_null());
            assert_eq!(stopped["sources_total"], 1);
        }
        app.close();
        assert!(started.elapsed() < Duration::from_secs(3));
        server.join().expect("fetch socket closed");
    }
}

#[test]
fn import_source_loads_profiles_and_clears_stale_filters() {
    let directory = Directory::new();
    let (url, server) = fixture(vec!["trojan://import-password@example.invalid:443#Новый"]);
    let mut app = Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    app.send(
        json!({"command":"query","search":"Скрывающий фильтр","favorites_only":true,"offset":0}),
    );
    app.wait(|v| v["busy"] == false);
    app.send(json!({"command":"import_source","name":"Новый источник","url":url,"local_exception":true,"route":"direct"}));
    let loaded = app.wait(|v| v["total"] == 1 && v["busy"] == false);
    assert_eq!(loaded["profiles"][0]["name"], "Новый");
    assert_eq!(loaded["search"], "");
    assert_eq!(loaded["favorites_only"], false);
    assert!(loaded["pending"].is_null());
    assert_eq!(loaded["sources_total"], 1);
    app.close();
    server.join().expect("fixture");
    assert!(
        app.snapshots
            .iter()
            .all(|v| !v.contains("import-password") && !v.contains("fixture-secret"))
    );
}

#[test]
fn automatic_port_avoids_occupied_default_and_remains_stable_on_reload() {
    let occupied = TcpListener::bind("127.0.0.1:10808").ok();
    let directory = Directory::new();
    let mut app = Harness::new(directory.config());
    let ready = app.wait(|v| v["ready"] == true);
    let port = ready["port"].as_u64().expect("порт");
    assert!((1..=65535).contains(&port));
    if occupied.is_some() {
        assert_ne!(port, 10808);
    }
    assert_eq!(ready["listen_address"], format!("127.0.0.1:{port}"));
    let probe =
        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port as u16)).expect("порт доступен");
    app.send(json!({"command":"reload"}));
    assert_eq!(app.wait(|v| v["ready"] == true)["port"], port);
    app.close();
    drop(probe);
}

#[test]
fn import_source_partial_requires_review_and_failed_fetch_keeps_sources() {
    let directory = Directory::new();
    let (url, server) = fixture(vec![
        "trojan://partial-password@example.invalid:443#Частичный\nunknown://broken",
    ]);
    let mut app = Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"import_source","name":"Частичный","url":url,"local_exception":true,"route":"direct"}));
    let partial = app.wait(|v| !v["pending"].is_null() && v["busy"] == false);
    assert_eq!(partial["pending"]["accepted"], 1);
    assert_eq!(partial["pending"]["rejected"], 1);
    assert_eq!(partial["total"], 0);
    app.send(json!({"command":"approve_refresh"}));
    app.wait(|v| v["total"] == 1);
    let refused = TcpListener::bind("127.0.0.1:0").expect("адрес");
    let url = format!(
        "http://{}/failed-secret",
        refused.local_addr().expect("адрес")
    );
    drop(refused);
    app.send(json!({"command":"import_source","name":"Недоступный","url":url,"local_exception":true,"route":"direct"}));
    let failed = app.wait(|v| v["error"] == "fetch" && v["busy"] == false);
    assert_eq!(failed["sources_total"], 2);
    assert_eq!(failed["total"], 0);
    app.send(json!({"command":"select_source","id":partial["source_id"]}));
    assert_eq!(
        app.wait(|v| v["total"] == 1)["profiles"][0]["name"],
        "Частичный"
    );
    app.close();
    server.join().expect("fixture");
}

#[test]
fn automatic_port_collision_reselects_and_fixed_override_reports_busy() {
    for automatic in [true, false] {
        let directory = Directory::new();
        let (url, server) = fixture(vec!["trojan://port-password@example.invalid:443#Порт"]);
        let fixed = TcpListener::bind("127.0.0.1:0").expect("порт");
        let port = fixed.local_addr().expect("адрес").port();
        let config = if automatic {
            directory.config()
        } else {
            directory
                .config()
                .with_listen_port(std::num::NonZeroU16::new(port).expect("порт"))
        };
        let mut app = Harness::new(config);
        let ready = app.wait(|v| v["ready"] == true);
        let occupied = if automatic {
            TcpListener::bind((
                std::net::Ipv4Addr::LOCALHOST,
                ready["port"].as_u64().expect("порт") as u16,
            ))
            .expect("коллизия")
        } else {
            fixed
        };
        app.send(json!({"command":"import_source","name":"Порт","url":url,"local_exception":true,"route":"direct"}));
        let imported = app.wait(|v| v["total"] == 1 && v["busy"] == false);
        app.send(json!({"command":"select_profile","id":imported["profiles"][0]["id"],"revision":imported["revision"]}));
        app.wait(|v| v["can_connect"] == true);
        app.send(json!({"command":"connect"}));
        if automatic {
            let failed = app.wait(|v| v["error"] == "engine_missing" && v["can_stop"] == false);
            assert_ne!(failed["port"], ready["port"]);
        } else {
            let failed = app.wait(|v| v["error"] == "listen_port_busy");
            assert_eq!(failed["port"], port);
            assert_eq!(failed["can_stop"], false);
        }
        assert!(occupied.local_addr().is_ok());
        app.close();
        server.join().expect("fixture");
    }
}

#[test]
fn hwid_preferences_are_opt_in_persisted_and_refresh_selected_source() {
    let directory = Directory::new();
    let (url, server) = fixture(vec![
        "trojan://fixture@example.invalid:443#После переключения",
    ]);
    let mut app = Harness::new(directory.config());
    let initial = app.wait(|v| v["ready"] == true);
    assert_eq!(initial["source_hwid_enabled"], false);
    assert!(
        initial["build_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty() && id != "0.1.0")
    );
    app.send(json!({"command":"add_source","name":"По умолчанию","url":url,"local_exception":true,"route":"direct"}));
    let first = app.wait(|v| v["sources_total"] == 1);
    assert_eq!(first["source_hwid_enabled"], false);
    let first_id = first["source_id"].clone();
    app.send(json!({"command":"add_source","name":"Opt-in","url":url,"local_exception":true,"route":"direct","hwid_enabled":true}));
    let second = app.wait(|v| v["sources_total"] == 2);
    assert_eq!(second["source_hwid_enabled"], true);
    let second_id = second["source_id"].clone();
    app.send(json!({"command":"select_source","id":first_id}));
    app.wait(|v| v["source_id"] == first_id && v["source_hwid_enabled"] == false);
    app.send(json!({"command":"select_source","id":second_id}));
    app.wait(|v| v["source_id"] == second_id && v["source_hwid_enabled"] == true);
    // Отключение автоматически обновляет выбранную подписку без передачи идентификатора.
    app.send(json!({"command":"set_source_hwid","enabled":false}));
    let refreshed = app.wait(|v| v["total"] == 1 && v["busy"] == false);
    assert_eq!(refreshed["source_hwid_enabled"], false);
    assert_eq!(refreshed["profiles"][0]["name"], "После переключения");
    app.close();
    server.join().expect("автоматическое обновление завершено");
    let mut restarted = Harness::new(directory.config());
    let loaded = restarted.wait(|v| v["ready"] == true);
    assert_eq!(loaded["source_hwid_enabled"], false);
    assert_eq!(loaded["sources"][1]["hwid_enabled"], false);
    restarted.close();
    for snapshot in app.snapshots.iter().chain(&restarted.snapshots) {
        let value: Value = serde_json::from_str(snapshot).expect("DTO");
        assert!(value.get("hwid").is_none());
        assert!(value.get("hwid_id").is_none());
        assert!(!snapshot.contains("fixture-secret"));
    }
}

#[test]
fn hwid_commands_reject_non_boolean_and_require_selection() {
    let directory = Directory::new();
    let mut app = Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    for value in [json!(null), json!("true"), json!(1)] {
        for command in [
            json!({"command":"set_source_hwid","enabled":value}),
            json!({"command":"add_source","name":"x","url":"https://example.invalid","local_exception":false,"route":"direct","hwid_enabled":value}),
            json!({"command":"import_source","name":"x","url":"https://example.invalid","local_exception":false,"route":"direct","hwid_enabled":value}),
        ] {
            assert_eq!(
                app.host
                    .dispatch(&command.to_string())
                    .expect_err("только bool")
                    .key(),
                "invalid_input"
            );
        }
    }
    app.send(json!({"command":"set_source_hwid","enabled":true}));
    app.wait(|v| v["error"] == "source_required");
    app.close();
}

#[test]
fn import_hwid_opt_in_sends_stable_private_id_and_toggle_refreshes() {
    let directory = Directory::new();
    let (sender, requests) = mpsc::channel();
    let (url, server) = fixture_with_requests(
        vec!["trojan://fixture@example.invalid:443#HWID"; 4],
        move |request| {
            let request = String::from_utf8_lossy(request);
            let hwid = request.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("x-hwid")
                    .then(|| value.trim().to_owned())
            });
            sender.send(hwid).expect("заголовок тестового запроса");
        },
    );
    let mut app = Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"import_source","name":"HWID","url":url,"local_exception":true,"route":"direct","hwid_enabled":true}));
    let imported = app.wait(|v| v["total"] == 1 && v["busy"] == false);
    assert_eq!(imported["source_hwid_enabled"], true);
    let hwid = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("запрос")
        .expect("opt-in HWID");
    assert_eq!(hwid.len(), 32);
    app.send(json!({"command":"set_source_hwid","enabled":false}));
    app.wait(|v| v["source_hwid_enabled"] == false && v["busy"] == false);
    assert!(
        requests
            .recv_timeout(Duration::from_secs(2))
            .expect("без HWID")
            .is_none()
    );
    app.send(json!({"command":"set_source_hwid","enabled":true}));
    app.wait(|v| v["source_hwid_enabled"] == true && v["busy"] == false);
    // Не используем assert_eq: при ошибке тест не должен напечатать идентификатор.
    assert!(
        requests
            .recv_timeout(Duration::from_secs(2))
            .expect("повторный opt-in")
            .as_deref()
            == Some(hwid.as_str())
    );
    app.close();
    let mut restarted = Harness::new(directory.config());
    let loaded = restarted.wait(|v| v["ready"] == true);
    assert_eq!(loaded["source_hwid_enabled"], true);
    restarted.send(json!({"command":"refresh"}));
    restarted.wait(|v| v["busy"] == true);
    restarted.wait(|v| v["busy"] == false);
    assert!(
        requests
            .recv_timeout(Duration::from_secs(2))
            .expect("после перезапуска")
            .as_deref()
            == Some(hwid.as_str())
    );
    restarted.close();
    server.join().expect("все обновления завершены");
    for snapshot in app.snapshots.iter().chain(&restarted.snapshots) {
        assert!(!snapshot.contains(&hwid));
    }
}

#[test]
fn rename_and_delete_preserve_other_sources_and_choose_first_remaining() {
    let directory = Directory::new();
    let mut app = Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    for name in ["Первый", "Второй"] {
        app.send(json!({"command":"add_source","name":name,"url":"https://example.invalid/private","local_exception":false,"route":"direct","hwid_enabled":true}));
        app.wait(|v| v["source_name"] == name);
    }
    app.send(json!({"command":"reload"}));
    let sources = app.wait(|v| v["sources_total"] == 2);
    let first = sources["sources"][0]["id"].clone();
    let second = sources["source_id"].clone();
    app.send(json!({"command":"rename_source","id":second,"name":"Новое имя"}));
    let renamed = app.wait(|v| v["source_name"] == "Новое имя");
    assert_eq!(renamed["source_id"], second);
    assert_eq!(renamed["source_hwid_enabled"], true);
    assert_eq!(renamed["busy"], false);
    app.send(json!({"command":"delete_source","id":second}));
    let remaining = app.wait(|v| v["sources_total"] == 1);
    assert_eq!(remaining["source_id"], first);
    assert_eq!(remaining["source_name"], "Первый");
    app.send(json!({"command":"delete_source","id":first}));
    let empty = app.wait(|v| v["sources_total"] == 0);
    assert!(empty["source_id"].is_null());
    assert!(empty["source_name"].is_null());
    assert_eq!(empty["profiles"], json!([]));
    assert!(empty["selected_profile"].is_null());
    app.close();
    assert!(
        !app.snapshots
            .iter()
            .any(|v| v.contains("example.invalid/private"))
    );
}

#[test]
fn cancel_refresh_checks_id_retains_busy_until_terminal_and_preserves_sources() {
    let directory = Directory::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback");
    let url = format!("http://{}/slow", listener.local_addr().expect("адрес"));
    let (accepted, request) = mpsc::channel();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("локальный запрос");
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .expect("deadline");
            let mut header = Vec::new();
            let mut byte = [0];
            while !header.ends_with(b"\r\n\r\n") {
                assert_eq!(stream.read(&mut byte).expect("заголовки"), 1);
                header.push(byte[0]);
                assert!(header.len() < 16384);
            }
            accepted.send(()).expect("начат запрос");
            assert_eq!(stream.read(&mut byte).expect("отмена закрывает сокет"), 0);
        }
    });
    let mut app = Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"import_source","name":"Обновляемый","url":url,"local_exception":true,"route":"direct"}));
    let busy = app.wait(|v| v["busy"] == true);
    let id = busy["refresh_source_id"]
        .as_i64()
        .expect("ID текущего refresh");
    request
        .recv_timeout(Duration::from_secs(5))
        .expect("загрузка началась");
    let token = busy["refresh_token"].as_str().expect("строковый token");
    assert!(token.bytes().all(|byte| byte.is_ascii_digit()));
    app.send(json!({"command":"cancel_refresh","id":id+1,"token":token}));
    let wrong = app.wait(|v| v["error"] == "refresh_required");
    assert_eq!(wrong["busy"], true);
    assert_eq!(wrong["refresh_source_id"], id);
    app.send(json!({"command":"delete_source","id":id}));
    assert_eq!(app.wait(|v| v["error"] == "busy")["sources_total"], 1);
    app.send(json!({"command":"cancel_refresh","id":id,"token":token}));
    let requested = app.wait(|v| v["error"].is_null());
    assert_eq!(requested["busy"], true);
    let terminal = app.wait(|v| v["busy"] == false);
    assert!(terminal["error"].is_null());
    assert!(terminal["refresh_source_id"].is_null());
    assert_eq!(terminal["sources_total"], 1);
    assert_eq!(terminal["source_id"], id);
    assert_eq!(terminal["status"], "Disconnected");
    app.send(json!({"command":"cancel_refresh","id":id,"token":token}));
    assert_eq!(
        app.wait(|v| v["error"] == "refresh_required")["busy"],
        false
    );
    app.send(json!({"command":"refresh"}));
    let next = app.wait(|v| v["busy"] == true);
    let next_token = next["refresh_token"].as_str().expect("новый token");
    assert_ne!(next_token, token);
    request
        .recv_timeout(Duration::from_secs(5))
        .expect("новая загрузка");
    for stale in [
        json!({"command":"cancel_refresh","id":id,"token":token}),
        json!({"command":"cancel_refresh","id":id}),
    ] {
        app.send(stale);
        let rejected = app.wait(|v| v["error"] == "refresh_required");
        assert_eq!(rejected["busy"], true);
        assert_eq!(rejected["refresh_token"], next_token);
    }
    app.send(json!({"command":"cancel_refresh","id":id,"token":next_token}));
    let finished = app.wait(|v| v["busy"] == false);
    assert!(finished["error"].is_null());
    assert!(finished["refresh_token"].is_null());
    app.close();
    server.join().expect("сокет закрыт");
}

#[test]
fn preferences_reopen_applies_engine_port_probe_and_language() {
    let directory = Directory::new();
    let mut first = Harness::new(directory.config());
    first.wait(|v| v["ready"] == true);
    for (command, field, expected) in [
        (
            json!({"command":"set_engine","engine":"sing-box"}),
            "engine",
            json!("sing-box"),
        ),
        (
            json!({"command":"set_listen_port","port":18889}),
            "port",
            json!(18889),
        ),
        (
            json!({"command":"set_probe_target","url":"https://example.com/health","expected_status":200}),
            "probe_expected_status",
            json!(200),
        ),
        (
            json!({"command":"set_language","language":"en"}),
            "language",
            json!("en"),
        ),
    ] {
        first.send(command);
        first.wait(|v| v[field] == expected);
    }
    first.close();
    let mut second = Harness::new(directory.config());
    let state = second.wait(|v| v["ready"] == true);
    assert_eq!(state["engine"], "sing-box");
    assert_eq!(state["port"], 18889);
    assert_eq!(state["probe_expected_status"], 200);
    assert_eq!(state["language"], "en");
    assert_eq!(state["probe_custom"], true);
    second.send(json!({"command":"reset_listen_port"}));
    second.wait(|v| v["port"] != 18889);
    second.close();
    let raw = fs::read_to_string(directory.0.join("preferences.json")).expect("настройки");
    assert!(raw.contains("\"port\":null"));
}

#[test]
fn damaged_preferences_notify_and_survive_settings_commands() {
    let directory = Directory::new();
    let path = directory.0.join("preferences.json");
    fs::write(&path, b"{").expect("fixture");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("fixture");
    let mut app = Harness::new(directory.config());
    let state = app.wait(|v| v["ready"] == true);
    assert_eq!(state["preferences_warning"], true);
    assert_eq!(state["engine"], "xray");
    app.send(json!({"command":"set_engine","engine":"sing-box"}));
    let state = app.wait(|v| v["error"] == "preferences_write");
    assert_eq!(state["engine"], "xray");
    app.close();
    assert_eq!(fs::read(path).expect("исходник"), b"{");
}

// Старые integration-сценарии явно создают overdue metadata вместо ожидания суток.
pub(crate) fn сделать_overdue(dir: &Directory) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let repo = yonik_linux::EncryptedSubscriptionRepository::open(
                dir.0.join("subscriptions.sqlite"),
                zeroize::Zeroizing::new([77; 32]),
            )
            .await
            .unwrap();
            let service = yonik_linux::SubscriptionService::new(repo);
            for token in service.schedule_metadata_page(0).await.unwrap() {
                service
                    .persist_schedule(
                        token,
                        yonik_linux::ScheduleState {
                            version: 1,
                            due_wall: Some(1),
                            recorded_wall: 1,
                            failures: 0,
                            paused: false,
                        },
                    )
                    .await
                    .unwrap();
            }
            service.close().await.unwrap();
        });
}

#[test]
fn scheduled_clean_import_requires_review_before_commit_or_discard() {
    for approve in [false, true] {
        let dir = Directory::new();
        let (url, server) = fixture(vec!["socks5://127.0.0.1:1080#Плановый"]);
        let mut seed = Harness::new(dir.config());
        seed.wait(|v| v["ready"] == true);
        seed.send(json!({"command":"add_source", "name":"Плановый", "url":url,
            "local_exception":true,"route":"direct"}));
        seed.wait(|v| v["source_id"].is_i64());
        seed.close();
        сделать_overdue(&dir);
        let mut app = Harness::new(dir.config());
        app.wait(|v| v["ready"] == true);
        app.send(json!({"command":"subscription_visibility","visible":true}));
        app.host
            .set_subscription_policy(yonik_linux::SubscriptionPolicy {
                online: true,
                ..Default::default()
            })
            .expect("доверенный тестовый policy provider");
        let review = app.wait(|v| !v["pending"].is_null());
        assert_eq!(review["total"], 0);
        assert_eq!(review["pending"]["accepted"], 1);
        assert_eq!(review["can_connect"], false);
        app.send(json!({"command": if approve {"approve_refresh"} else {"discard_refresh"}}));
        let done =
            app.wait(|v| v["pending"].is_null() && v["total"] == if approve { 1 } else { 0 });
        assert!(done["error"].is_null());
        app.close();
        server.join().expect("loopback fixture");
    }
}

#[test]
fn scheduled_delete_and_shutdown_acknowledge_held_fetch() {
    for action in ["shutdown", "delete", "hwid"] {
        let dir = Directory::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback");
        let url = format!("http://{}/held", listener.local_addr().expect("адрес"));
        let mut seed = Harness::new(dir.config());
        seed.wait(|v| v["ready"] == true);
        seed.send(
            json!({"command":"add_source", "name":"Удерживаемый", "url":url,
            "local_exception":true,"route":"direct"}),
        );
        let id = seed.wait(|v| v["source_id"].is_i64())["source_id"].clone();
        seed.close();
        сделать_overdue(&dir);
        let (accepted, receive) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("fetch");
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .expect("timeout");
            let mut bytes = [0; 4096];
            assert!(stream.read(&mut bytes).expect("request") > 0);
            accepted.send(()).expect("сигнал fetch");
            loop {
                if stream.read(&mut bytes).expect("cancel закрыл соединение") == 0 {
                    break;
                }
            }
            if action == "hwid" {
                let (mut next, _) = listener.accept().expect("refresh нового поколения");
                assert!(next.read(&mut bytes).expect("request") > 0);
                let body = "socks5://127.0.0.1:1081#Новое поколение";
                write!(
                    next,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .expect("ответ");
            }
        });
        let mut app = Harness::new(dir.config());
        app.wait(|v| v["ready"] == true);
        app.send(json!({"command":"subscription_visibility","visible":true}));
        app.host
            .set_subscription_policy(yonik_linux::SubscriptionPolicy {
                online: true,
                ..Default::default()
            })
            .expect("policy");
        receive
            .recv_timeout(Duration::from_secs(10))
            .expect("реальный scheduled запрос");
        if action == "delete" {
            app.send(json!({"command":"delete_source","id":id}));
            let removed = app.wait(|v| v["sources_total"] == 0);
            assert!(removed["pending"].is_null());
        }
        if action == "hwid" {
            app.send(json!({"command":"set_source_hwid","enabled":true}));
            let refreshed = app.wait(|v| v["source_hwid_enabled"] == true && v["total"] == 1);
            assert!(refreshed["error"].is_null());
        }
        app.close();
        server.join().expect("fetch освобождён до closed");
    }
}

#[test]
fn queued_source_delete_does_not_wait_for_another_fetch_before_shutdown() {
    let dir = Directory::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback");
    let url = format!("http://{}/held", listener.local_addr().expect("адрес"));
    let mut seed = Harness::new(dir.config());
    seed.wait(|v| v["ready"] == true);
    let mut ids = Vec::new();
    for name in ["Первый", "Второй"] {
        seed.send(json!({"command":"add_source", "name":name, "url":url,
            "local_exception":true,"route":"direct"}));
        let state = seed.wait(|v| v["sources_total"] == ids.len() + 1);
        ids.push(state["source_id"].as_i64().expect("source id"));
    }
    seed.close();
    сделать_overdue(&dir);
    let (accepted, receive) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("fetch");
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .expect("timeout");
        let mut bytes = [0; 4096];
        assert!(stream.read(&mut bytes).expect("request") > 0);
        accepted.send(()).expect("fetch начался");
        assert_eq!(stream.read(&mut bytes).expect("shutdown отменяет fetch"), 0);
    });
    let mut app = Harness::new(dir.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"subscription_visibility","visible":true}));
    app.host
        .set_subscription_policy(yonik_linux::SubscriptionPolicy {
            online: true,
            ..Default::default()
        })
        .expect("policy");
    receive
        .recv_timeout(Duration::from_secs(10))
        .expect("fetch первого источника");
    app.send(json!({"command":"delete_source","id":ids[1]}));
    // Удаление queued ID завершается, пока первый fetch удерживается.
    app.wait(|v| v["sources_total"] == 1);
    app.close();
    server.join().expect("fetch отменён shutdown");
}

#[test]
fn scheduler_metadata_pages_and_generation_follow_source_changes() {
    let dir = Directory::new();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let repository = yonik_linux::EncryptedSubscriptionRepository::open(
                dir.0.join("subscriptions.sqlite"),
                zeroize::Zeroizing::new([77; 32]),
            )
            .await
            .expect("отдельная БД");
            let service = yonik_linux::SubscriptionService::new(repository);
            let mut ids = Vec::new();
            for index in 0..65 {
                ids.push(
                    service
                        .add(
                            yonik_linux::SubscriptionSource::new(
                                format!("Источник {index}"),
                                "https://example.invalid/metadata".into(),
                                false,
                                yonik_linux::FetchRoute::Proxy,
                            )
                            .expect("source"),
                        )
                        .await
                        .expect("add"),
                );
            }
            let first = service.schedule_sources(0).await.expect("первая страница");
            assert_eq!(first.len(), 64);
            assert_eq!(
                service
                    .schedule_sources(64)
                    .await
                    .expect("вторая страница")
                    .len(),
                1
            );
            assert!(
                service
                    .schedule_sources(65)
                    .await
                    .expect("конец")
                    .is_empty()
            );
            assert!(service.schedule_sources(20_001).await.is_err());
            let id = ids[0];
            let old = service
                .schedule_generation(id)
                .await
                .expect("generation")
                .expect("id");
            service
                .set_source_hwid(id, true)
                .await
                .expect("source change");
            assert!(
                service
                    .schedule_generation(id)
                    .await
                    .expect("generation")
                    .expect("id")
                    > old
            );
            service.delete_source(id).await.expect("delete");
            assert_eq!(
                service.schedule_generation(id).await.expect("deleted"),
                None
            );
            service.close().await.expect("close");
        });
}

#[test]
fn p2_ручной_b_не_пересекается_с_плановым_a_и_не_теряет_review() {
    let dir = Directory::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/held", listener.local_addr().unwrap());
    let mut seed = Harness::new(dir.config());
    seed.wait(|v| v["ready"] == true);
    seed.send(json!({"command":"add_source", "name":"A", "url":url, "local_exception":true,"route":"direct"}));
    let a = seed.wait(|v| v["source_id"].is_i64())["source_id"].clone();
    seed.send(json!({"command":"add_source", "name":"B", "url":"http://127.0.0.1:9/b", "local_exception":true,"route":"direct"}));
    let b = seed.wait(|v| v["sources_total"] == 2)["source_id"].clone();
    seed.close();
    сделать_overdue(&dir);
    let (entered, receive) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = [0; 4096];
        assert!(stream.read(&mut bytes).unwrap() > 0);
        entered.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        let body = "socks5://127.0.0.1:1080#A";
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    });
    let mut app = Harness::new(dir.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"subscription_visibility","visible":true}));
    app.host
        .set_subscription_policy(yonik_linux::SubscriptionPolicy {
            online: true,
            ..Default::default()
        })
        .unwrap();
    receive.recv_timeout(Duration::from_secs(10)).unwrap();
    app.send(json!({"command":"select_source","id":b}));
    app.wait(|v| v["source_id"] == b);
    app.send(json!({"command":"refresh"}));
    let refused = app.wait(|v| !v["error"].is_null());
    // Освобождаем loopback даже в RED, прежде чем проверять guard.
    release.send(()).unwrap();
    let guarded = refused["error"] == "busy" && refused["busy"] == false;
    if !guarded {
        app.close();
        server.join().unwrap();
        panic!("ручной B должен ждать владельца A: {refused}");
    }
    let review = app.wait(|v| !v["pending"].is_null());
    assert_eq!(review["source_id"], a);
    app.send(json!({"command":"refresh"}));
    let blocked = app.wait(|v| v["error"] == "review_required");
    assert!(!blocked["pending"].is_null());
    app.send(json!({"command":"discard_refresh"}));
    app.wait(|v| v["pending"].is_null());
    app.close();
    server.join().unwrap();
}
