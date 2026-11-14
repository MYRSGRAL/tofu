#![cfg(all(target_os = "linux", feature = "test-support"))]

use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
use yonik_desktop::{DesktopHost, HostConfig};
use yonik_linux::{FetchHost, SubscriptionPolicy};

struct App {
    host: DesktopHost,
    rx: mpsc::Receiver<String>,
}
impl App {
    fn start() -> Self {
        let data = PathBuf::from(std::env::var_os("P05_DATA").expect("private fixture data"));
        let endpoint = std::env::var("P05_SOCKS")
            .expect("owned SOCKS")
            .parse()
            .expect("endpoint");
        let fetch = Arc::new(
            FetchHost::new(Duration::from_secs(3))
                .expect("timeout")
                .with_socks_endpoint(endpoint)
                .expect("loopback"),
        );
        let config = HostConfig::new(data.clone(), data.join("no-engines"))
            .with_system_subscription_policy(false)
            .with_test_subscription_fetch(fetch);
        let (tx, rx) = mpsc::channel();
        let host = DesktopHost::start(config, move |snapshot| {
            let _ = tx.send(snapshot);
        })
        .expect("normal Host");
        let mut app = Self { host, rx };
        app.wait(|v| v["ready"] == true);
        app
    }
    fn send(&self, value: Value) {
        self.host.dispatch(&value.to_string()).expect("command");
    }
    #[track_caller]
    fn wait(&mut self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(22);
        let caller = std::panic::Location::caller();
        let mut last = json!({});
        loop {
            let raw = self
                .rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|_| {
                    panic!(
                        "bounded snapshot at line {}; safe flags: {last}",
                        caller.line()
                    )
                });
            assert!(!raw.contains("invented-priority-secret"), "DTO redaction");
            let value: Value = serde_json::from_str(&raw).expect("snapshot");
            last = json!({"ready":value["ready"],"error":value["error"],"source_error":value["source_error"],"closed":value["closed"],"busy":value["busy"],"pending":value["pending"].is_object(),"revision":value["revision"],"total":value["total"],"source_id":value["source_id"]});
            if predicate(&value) {
                return value;
            }
        }
    }
    fn close(self) {
        self.host.shutdown();
        assert!(self.host.wait_closed(Duration::from_secs(8)));
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.host.shutdown();
        assert!(self.host.wait_closed(Duration::from_secs(8)));
    }
}

fn control(path: &str) -> Value {
    let mut stream = TcpStream::connect(std::env::var("P05_CONTROL").expect("control address"))
        .expect("own control socket");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    write!(stream, "GET /{path} HTTP/1.0\r\nHost: fixture\r\n\r\n").expect("control request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("control response");
    serde_json::from_str(response.split_once("\r\n\r\n").expect("HTTP framing").1)
        .expect("control JSON")
}
fn inspect_due(overdue: bool) -> (u32, u64) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let key = yonik_linux::subscription_master_key(false)
                .await
                .expect("actual private SS key");
            let path = PathBuf::from(std::env::var_os("P05_DATA").expect("data"))
                .join("subscriptions.sqlite");
            let service = yonik_linux::SubscriptionService::new(
                yonik_linux::EncryptedSubscriptionRepository::open(path, key)
                    .await
                    .expect("same encrypted repository"),
            );
            let token = service
                .schedule_metadata_page(0)
                .await
                .expect("metadata")
                .pop()
                .expect("source");
            let state = token.state.expect("persisted schedule");
            let result = (state.failures, state.due_wall.expect("due"));
            if overdue {
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
                    .expect("existing metadata API, not clock injection");
            }
            service.close().await.expect("close");
            result
        })
}

#[test]
#[ignore = "только owned private Secret Service + loopback fixture runner"]
fn actual_normal_host_subscription_priority() {
    let url = std::env::var("P05_URL").expect("own HTTPS source");
    let mut app = App::start();
    app.send(json!({"command":"add_source","name":"Synthetic","url":url,"local_exception":true,"route":"proxy"}));
    app.wait(|v| v["source_id"].is_i64());
    app.send(json!({"command":"refresh"}));
    // Чистый remote refresh коммитится существующим producer автоматически.
    app.wait(|v| v["revision"] == 1 && v["total"] == 1 && v["busy"] == false);
    app.close();
    inspect_due(true);
    control("mode/error");
    let mut app = App::start();
    app.send(json!({"command":"subscription_visibility","visible":true}));
    app.host
        .set_subscription_policy(SubscriptionPolicy {
            online: true,
            ui_visible: true,
            ..Default::default()
        })
        .expect("existing policy API");
    let failed = app.wait(|v| {
        v["error_contexts"].as_array().is_some_and(|items| {
            items
                .iter()
                .any(|c| c["reason"] == "scheduler_service_fetch_failed")
        })
    });
    assert_eq!(failed["revision"], 1);
    assert!(failed["pending"].is_null());
    assert!(
        failed["scheduler_generation"]
            .as_u64()
            .is_some_and(|v| v > 0)
    );
    app.close();
    let (failures, due) = inspect_due(false);
    assert!(failures > 0, "HTTP503 persist backoff");
    assert!(
        due > std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("wall")
            .as_secs()
    );
    let mut app = App::start();
    // Явный manual не ждёт следующего автоматического due; policy не подменяет результат fetch.
    control("mode/partial");
    let started = Instant::now();
    app.send(json!({"command":"refresh"}));
    let manual = app.wait(|v| v["busy"] == true);
    assert!(manual["refresh_token"].is_string());
    let review = app.wait(|v| v["pending"]["accepted"] == 1 && v["pending"]["rejected"] == 1);
    assert_eq!(review["revision"], 1);
    assert!(started.elapsed() < Duration::from_millis(900));
    app.send(json!({"command":"discard_refresh"}));
    app.wait(|v| v["pending"].is_null() && v["busy"] == false);
    app.close();
    let (failures, due) = inspect_due(false);
    assert_eq!(failures, 0, "успешный manual review сбрасывает backoff");
    assert!(due > 1);
    control("mode/held");
    let mut app = App::start();
    app.send(json!({"command":"refresh"}));
    let busy = app.wait(|v| v["busy"] == true);
    let held_deadline = Instant::now() + Duration::from_secs(3);
    while control("stats")["held_started"] != true {
        assert!(Instant::now() < held_deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let token = busy["refresh_token"].clone();
    app.send(json!({"command":"cancel_refresh","id":busy["source_id"],"token":token}));
    app.wait(|v| v["busy"] == false && v["pending"].is_null());
    let closed_deadline = Instant::now() + Duration::from_secs(3);
    while control("stats")["held_closed"] != true {
        assert!(Instant::now() < closed_deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    // Stop отзывает тот же transport watch, даже без NativeRunning/ProxyLease.
    control("mode/held");
    app.send(json!({"command":"refresh"}));
    app.wait(|v| v["busy"] == true);
    let deadline = Instant::now() + Duration::from_secs(3);
    while control("stats")["held_started"] != true {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    app.send(json!({"command":"stop"}));
    app.wait(|v| v["busy"] == false && v["pending"].is_null());
    let deadline = Instant::now() + Duration::from_secs(3);
    while control("stats")["held_closed"] != true {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    control("mode/partial");
    app.send(json!({"command":"refresh"}));
    app.wait(|v| v["pending"].is_object());
    app.send(json!({"command":"source_change","request_id":700,"id":busy["source_id"],"expected_generation":1,"name":"Changed","url":url,"local_exception":true,"route":"proxy"}));
    let changed = app.wait(|v| v["source_change_ack"]["request_id"] == 700);
    assert!(changed["source_change_ack"]["error"].is_null());
    assert_eq!(changed["source_change_ack"]["generation"], 2);
    app.send(json!({"command":"approve_refresh"}));
    let rejected = app.wait(|v| !v["error"].is_null());
    assert_eq!(rejected["revision"], 1);
    assert!(rejected["pending"].is_null());
    app.close();
    let mut app = App::start();
    app.send(json!({"command":"select_source","id":1}));
    app.wait(|v| v["revision"] == 1);
    control("proxy/stop");
    let before = control("stats")["origin_requests"].clone();
    assert_eq!(control("origin-control")["available"], true);
    app.send(json!({"command":"refresh"}));
    app.wait(|v| v["busy"] == false && !v["error"].is_null());
    assert_eq!(control("stats")["origin_requests"], before);
    app.close();
    println!("P05/P10 actual NormalHost/private-SS witness: PASS");
}
