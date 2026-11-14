#![cfg(all(target_os = "linux", feature = "test-support"))]

use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
use yonik_desktop::{DesktopHost, HostConfig};
use yonik_linux::SubscriptionPolicy;

struct App {
    host: DesktopHost,
    rx: mpsc::Receiver<String>,
    trace: mpsc::Receiver<Value>,
    overflow: Arc<AtomicBool>,
    rows: Vec<Value>,
}
impl App {
    fn start() -> Self {
        let data = PathBuf::from(std::env::var_os("P05_DATA").expect("private fixture data"));
        let (trace_tx, trace) = mpsc::sync_channel(512);
        let overflow = Arc::new(AtomicBool::new(false));
        let config = HostConfig::new(
            data.clone(),
            PathBuf::from(std::env::var_os("P05_ENGINES").expect("pinned engines")),
        )
        .with_system_subscription_policy(false)
        .with_test_schedule_trace(trace_tx, overflow.clone());
        let (tx, rx) = mpsc::channel();
        let host = DesktopHost::start(config, move |snapshot| {
            let _ = tx.send(snapshot);
        })
        .expect("normal Host");
        let mut app = Self {
            host,
            rx,
            trace,
            overflow,
            rows: Vec::new(),
        };
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
    fn event(&mut self, predicate: impl Fn(&Value) -> bool) -> Value {
        let until = Instant::now() + Duration::from_secs(25);
        loop {
            assert!(
                !self.overflow.load(Ordering::Acquire),
                "trace overflow/disconnect"
            );
            let row = self
                .trace
                .recv_timeout(until.saturating_duration_since(Instant::now()))
                .expect("bounded actual transition");
            self.rows.push(row.clone());
            if predicate(&row) {
                return row;
            }
        }
    }
    fn connect(&mut self) {
        self.send(
            json!({"command":"set_engine","engine":std::env::var("P05_ENGINE").expect("engine")}),
        );
        self.wait(|v| v["engine"] == std::env::var("P05_ENGINE").expect("engine"));
        self.send(json!({"command":"select_source","id":1}));
        let page = self.wait(|v| v["source_id"] == 1 && v["total"] == 1);
        self.send(json!({"command":"select_profile","id":page["profiles"][0]["id"],"revision":page["revision"]}));
        self.wait(|v| v["selected_profile"].is_i64());
        self.send(json!({"command":"set_probe_target","url":std::env::var("P05_URL").expect("url").replace("/subscription","/health"),"expected_status":204}));
        self.wait(|v| v["probe_custom"] == true);
        self.send(json!({"command":"connect"}));
        self.wait(|v| v["status"] == "Connected");
    }
    fn close(mut self) {
        self.host.shutdown();
        assert!(self.host.wait_closed(Duration::from_secs(8)));
        self.rows.extend(self.trace.try_iter());
        assert!(
            !self.overflow.load(Ordering::Acquire),
            "trace overflow/disconnect"
        );
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.host.shutdown();
        assert!(self.host.wait_closed(Duration::from_secs(8)));
        self.rows.extend(self.trace.try_iter());
        let path = std::env::var("P05_TRACE").expect("safe trace output");
        use std::fs::OpenOptions;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("trace file");
        for row in &self.rows {
            writeln!(file, "{row}").expect("trace");
        }
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

fn schedules(prepare: bool) -> Vec<(i64, yonik_linux::ScheduleState)> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let key = yonik_linux::subscription_master_key(false)
                .await
                .expect("actual SS");
            let svc = yonik_linux::SubscriptionService::new(
                yonik_linux::EncryptedSubscriptionRepository::open(
                    PathBuf::from(std::env::var_os("P05_DATA").expect("data"))
                        .join("subscriptions.sqlite"),
                    key,
                )
                .await
                .expect("repository"),
            );
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("wall")
                .as_secs();
            let mut result = Vec::new();
            for token in svc.schedule_metadata_page(0).await.expect("metadata") {
                let id = token.id;
                if prepare && id >= 2 {
                    svc.persist_schedule(
                        token,
                        yonik_linux::ScheduleState {
                            version: 1,
                            due_wall: Some(if id == 2 { 1 } else { now + 90 }),
                            recorded_wall: if id == 2 { 1 } else { now },
                            failures: 0,
                            paused: false,
                        },
                    )
                    .await
                    .expect("due API");
                } else {
                    result.push((id, token.state.expect("schedule")));
                }
            }
            svc.close().await.expect("close");
            result
        })
}
#[test]
#[ignore = "owned private SS + pinned LocalSocks fixture"]
fn actual_legacy_stop_window() {
    let url = std::env::var("P05_URL").expect("fixture");
    let mut app = App::start();
    app.send(json!({"command":"import_source","name":"Seed","url":url.replace("/subscription","/seed"),"local_exception":true,"route":"direct"}));
    app.wait(|v| v["total"] == 1 && v["busy"] == false);
    app.connect();
    for (index, path) in ["held", "second"].into_iter().enumerate() {
        app.send(json!({"command":"add_source","name":path,"url":url.replace("/subscription",&format!("/{path}")),"local_exception":true,"route":"proxy"}));
        app.wait(|v| v["source_id"] == (index as i64 + 2));
        app.send(json!({"command":"refresh"}));
        app.wait(|v| v["total"] == 1 && v["revision"] == 1 && v["busy"] == false);
    }
    app.close();
    schedules(true);
    control("mode/held");
    let mut app = App::start();
    app.connect();
    app.send(
        json!({"command":"set_subscription_schedule","interval_seconds":86400,"background":true}),
    );
    app.wait(|v| v["subscription_background"] == true);
    app.send(json!({"command":"subscription_visibility","visible":false}));
    app.host
        .set_subscription_policy(SubscriptionPolicy {
            online: true,
            ..Default::default()
        })
        .expect("logical policy");
    let opened = app.event(|e| {
        e["phase"] == "dispatch"
            && e["window_deadline_ns"].is_number()
            && e["lease_generation"].is_number()
    });
    let deadline = opened["window_deadline_ns"].clone();
    let lease = opened["lease_generation"].as_u64().expect("actual lease");
    let until = Instant::now() + Duration::from_secs(5);
    while control("stats")["held_started"] != true {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    app.send(json!({"command":"stop"}));
    let begin = app.event(|e| e["phase"] == "stop_drain_begin");
    let ack = app.event(|e| e["phase"] == "stop_drain_complete");
    assert_eq!(ack["drain_id"], begin["drain_id"]);
    assert_eq!(ack["scheduled_generation"], opened["scheduled_generation"]);
    app.wait(|v| v["status"] == "Disconnected");
    assert_eq!(control("stats")["held_closed"], true);
    let replacement = app.event(|e| {
        e["phase"] == "state" && e["scheduled_generation"] != ack["scheduled_generation"]
    });
    assert_eq!(
        replacement["window_deadline_ns"], deadline,
        "actual Stop обязан сохранить original window deadline"
    );
    app.connect();
    let newlease = app.event(|e| e["lease_generation"].as_u64().is_some_and(|g| g > lease));
    assert_eq!(newlease["window_deadline_ns"], deadline);
    control("mode/full");
    // Положительный control нового actual lease без подмены default transport.
    app.send(json!({"command":"select_source","id":2}));
    app.wait(|v| v["source_id"] == 2);
    app.send(json!({"command":"refresh"}));
    app.wait(|v| v["revision"] == 2 && v["busy"] == false);
    let second_before = control("stats")["second_requests"].clone();
    std::thread::sleep(Duration::from_secs(95));
    let closed = app.event(|e| e["phase"] == "state" && e["window_deadline_ns"].is_null());
    assert!(closed["at_ns"].as_u64().expect("monotonic") >= deadline.as_u64().expect("deadline"));
    assert_eq!(
        control("stats")["second_requests"],
        second_before,
        "second due source не открывает новое окно после original deadline"
    );
    app.send(json!({"command":"select_source","id":2}));
    let page = app.wait(|v| v["source_id"] == 2);
    assert_eq!(page["revision"], 2);
    assert!(page["pending"].is_null());
    app.close();
    let persisted = schedules(false);
    assert!(
        persisted
            .iter()
            .any(|(id, s)| *id == 3 && s.due_wall.is_some())
    );
    let mut app = App::start();
    app.send(json!({"command":"select_source","id":2}));
    app.wait(|v| v["source_id"] == 2 && v["revision"] == 2);
    app.close();
}
