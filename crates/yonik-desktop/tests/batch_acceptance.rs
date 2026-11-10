//! Приёмка DesktopHost через тот же JSON-протокол, что использует Qt.
#![cfg(target_os = "linux")]
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use yonik_desktop::{DesktopHost, HostConfig};
const DEADLINE: Duration = Duration::from_secs(180);
const TARGETS: usize = 10_000;
const LISTENERS: usize = 16;

struct Fixture {
    directory: PathBuf,
    stop: Arc<AtomicBool>,
    counts: Vec<Arc<AtomicU64>>,
    threads: Vec<thread::JoinHandle<()>>,
    url: String,
}
impl Fixture {
    fn new(targets: usize) -> Self {
        let directory =
            std::env::temp_dir().join(format!("yonik-batch-acceptance-{}", std::process::id()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .expect("свой каталог");
        let stop = Arc::new(AtomicBool::new(false));
        let mut counts = Vec::new();
        let mut threads = Vec::new();
        let mut ports = Vec::new();
        for _ in 0..LISTENERS {
            let listener = TcpListener::bind("127.0.0.1:0").expect("loopback TCP");
            ports.push(listener.local_addr().expect("адрес").port());
            listener.set_nonblocking(true).expect("nonblocking");
            let count = Arc::new(AtomicU64::new(0));
            counts.push(count.clone());
            let stop = stop.clone();
            threads.push(thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(5)))
                                .expect("deadline");
                            assert_eq!(stream.read(&mut [0]).expect("TCP EOF"), 0, "нет payload");
                            count.fetch_add(1, Ordering::SeqCst);
                            // Достаточное окно для RTNETLINK при активных TCP futures.
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(1))
                        }
                        Err(error) => panic!("accept: {error}"),
                    }
                }
            }));
        }
        let http = TcpListener::bind("127.0.0.1:0").expect("loopback HTTP");
        let url = format!("http://{}/fixture", http.local_addr().expect("адрес"));
        let body = (0..targets)
            .map(|id| {
                format!(
                    "trojan://synthetic-credential-{id}@127.0.0.1:{}#Цель{id}\n",
                    ports[id % LISTENERS]
                )
            })
            .collect::<String>();
        threads.push(thread::spawn(move || {
            let (mut stream, _) = http.accept().expect("HTTP запрос");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("deadline");
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                assert_eq!(stream.read(&mut byte).expect("заголовок"), 1);
                request.push(byte[0]);
                assert!(request.len() <= 16384);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("ответ");
        }));
        Self {
            directory,
            stop,
            counts,
            threads,
            url,
        }
    }
    fn accepted(&self) -> u64 {
        self.counts.iter().map(|n| n.load(Ordering::SeqCst)).sum()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for task in self.threads.drain(..) {
            task.join().expect("fixture завершена");
        }
        fs::remove_dir_all(&self.directory).expect("очистка своего каталога");
    }
}
struct App {
    host: DesktopHost,
    receiver: mpsc::Receiver<String>,
}
impl App {
    fn new(fixture: &Fixture) -> Self {
        let (sender, receiver) = mpsc::channel();
        let config = HostConfig::new(
            fixture.directory.clone(),
            fixture.directory.join("empty-engines"),
        )
        .with_ephemeral_key([61; 32]);
        let host = DesktopHost::start(config, move |snapshot| {
            let value: Value = serde_json::from_str(&snapshot).expect("DTO");
            let batch = &value["batch"];
            assert!(batch["rows"].as_array().expect("страница").len() <= 32);
            assert!(!snapshot.contains("synthetic-credential-"));
            sender.send(snapshot).expect("приёмник жив");
        })
        .expect("DesktopHost");
        Self { host, receiver }
    }
    fn send(&self, value: Value) {
        self.host
            .dispatch(&value.to_string())
            .expect("GUI protocol");
    }
    fn wait(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let snapshot = self
                .receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("snapshot deadline");
            let value = serde_json::from_str(&snapshot).expect("DTO");
            if predicate(&value) {
                return value;
            }
        }
    }
    fn invalidated(&self, old_epoch: &Value) -> Value {
        let value =
            self.wait(|v| v["network_epoch"] != *old_epoch && v["batch"]["invalidated"] == true);
        assert_eq!(value["batch"]["busy"], false);
        assert!(
            value["batch"]["rows"]
                .as_array()
                .expect("строки")
                .is_empty()
        );
        // Отличимый offset подтверждает обработку команды после network event.
        self.send(json!({"command":"batch_page","offset":32}));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snapshot = self
                .receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("page acknowledgement");
            let snapshot: Value = serde_json::from_str(&snapshot).expect("DTO");
            assert_eq!(snapshot["batch"]["invalidated"], true);
            assert!(
                snapshot["batch"]["rows"]
                    .as_array()
                    .expect("строки")
                    .is_empty()
            );
            if snapshot["batch"]["offset"] == 32 {
                break;
            }
        }
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            match self
                .receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(snapshot) => {
                    let snapshot: Value = serde_json::from_str(&snapshot).expect("DTO");
                    assert_eq!(snapshot["batch"]["invalidated"], true);
                    assert!(
                        snapshot["batch"]["rows"]
                            .as_array()
                            .expect("строки")
                            .is_empty()
                    );
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(error) => panic!("backend остаётся открыт: {error}"),
            }
        }
        value
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.host.shutdown();
        assert!(self.host.wait_closed(DEADLINE));
    }
}
fn ip(args: &[&str]) {
    let output = Command::new("ip")
        .args(args)
        .output()
        .expect("ip внутри namespace");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn isolated_network() -> u64 {
    let host_ns: u64 = std::env::var("YONIK_HOST_NETWORK_NS")
        .expect("host namespace inode")
        .parse()
        .expect("inode");
    assert_ne!(
        fs::metadata("/proc/self/ns/net").expect("namespace").ino(),
        host_ns
    );
    let links: Value = serde_json::from_slice(
        &Command::new("ip")
            .args(["-j", "link", "show"])
            .output()
            .expect("ip")
            .stdout,
    )
    .expect("links");
    assert_eq!(links.as_array().expect("links").len(), 1);
    assert_eq!(links[0]["ifname"], "lo");
    ip(&["link", "set", "lo", "up"]);
    host_ns
}

#[test]
#[ignore = "только unshare -Urn и YONIK_HOST_NETWORK_NS"]
fn actual_10k_tcp_connections() {
    isolated_network();
    let fixture = Fixture::new(TARGETS);
    let app = App::new(&fixture);
    app.wait(|v| v["ready"] == true && v["network_routes"] == true);
    app.send(json!({"command":"import_source","name":"10k loopback","url":fixture.url,"local_exception":true,"route":"direct"}));
    let imported = app.wait(|v| v["total"] == TARGETS && v["busy"] == false);
    let mut ids = HashSet::new();
    for offset in (0..TARGETS).step_by(200) {
        app.send(json!({"command":"query","search":"","favorites_only":false,"offset":offset}));
        let page = app.wait(|v| v["offset"] == offset && v["total"] == TARGETS);
        for row in page["profiles"].as_array().expect("профили") {
            assert!(ids.insert(row["id"].as_i64().expect("profile ID")));
        }
    }
    assert_eq!(ids.len(), TARGETS);
    let started = Instant::now();
    app.send(json!({"command":"check_batch"}));
    app.wait(|v| {
        v["batch"]["busy"] == true
            && v["batch"]["completed"]
                .as_u64()
                .is_some_and(|n| n >= 32 && n < TARGETS as u64)
    });
    let done = app.wait(|v| v["batch"]["completed"] == TARGETS && v["batch"]["busy"] == false);
    let mut results = HashSet::new();
    for offset in (0..TARGETS).step_by(32) {
        app.send(json!({"command":"batch_page","offset":offset}));
        let page =
            app.wait(|v| v["batch"]["offset"] == offset && v["batch"]["completed"] == TARGETS);
        for row in page["batch"]["rows"].as_array().expect("результаты") {
            assert_eq!(row["status"], "reachable");
            assert_eq!(row["samples_ms"].as_array().expect("пробы").len(), 3);
            assert_eq!(row["revision"], imported["revision"]);
            assert_eq!(row["network_epoch"], done["network_epoch"]);
            assert!(results.insert(row["profile_id"].as_i64().expect("ID")));
        }
    }
    assert_eq!(results, ids);
    let deadline = Instant::now() + Duration::from_secs(5);
    while fixture.accepted() < 30_000 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(fixture.accepted(), 30_000);
    let per_listener: Vec<_> = fixture
        .counts
        .iter()
        .map(|n| n.load(Ordering::SeqCst))
        .collect();
    assert!(per_listener.iter().all(|n| *n == 1875));
    eprintln!(
        "ACTUAL_10K profiles={} results={} tcp_eof={} listeners={:?} elapsed_ms={}",
        ids.len(),
        results.len(),
        fixture.accepted(),
        per_listener,
        started.elapsed().as_millis()
    );
    app.send(json!({"command":"check_batch"}));
    thread::sleep(Duration::from_millis(50));
    let cancel = Instant::now();
    app.send(json!({"command":"cancel_batch"}));
    app.wait(|v| {
        v["error"] == "cancelled"
            || (v["batch"]["cancelled"] == true && v["batch"]["busy"] == false)
    });
    assert!(cancel.elapsed() < Duration::from_secs(2));
    eprintln!(
        "PREPARATION_CANCEL elapsed_ms={} tcp_eof={}",
        cancel.elapsed().as_millis(),
        fixture.accepted()
    );
}

#[test]
#[ignore = "только unshare -Urn и YONIK_HOST_NETWORK_NS"]
fn real_batch_network_epoch() {
    let host_ns = isolated_network();
    let fixture = Fixture::new(128);
    let app = App::new(&fixture);
    app.wait(|v| v["ready"] == true && v["network_routes"] == true);
    app.send(json!({"command":"import_source","name":"epoch fixture","url":fixture.url,"local_exception":true,"route":"direct"}));
    app.wait(|v| v["total"] == 128 && v["busy"] == false);
    app.send(json!({"command":"check_batch"}));
    let done = app.wait(|v| v["batch"]["completed"] == 128 && v["batch"]["busy"] == false);
    let deadline = Instant::now() + Duration::from_secs(5);
    while fixture.accepted() < 384 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    ip(&["route", "add", "198.51.100.0/24", "dev", "lo"]);
    let completed_stale = app.invalidated(&done["network_epoch"]);
    assert_eq!(
        fixture.accepted(),
        384,
        "смена сети не запускает новую серию"
    );
    app.send(json!({"command":"check_batch"}));
    let inflight = app.wait(|v| {
        v["batch"]["busy"] == true && v["batch"]["completed"].as_u64().is_some_and(|n| n >= 32)
    });
    let active_connections = fixture.accepted();
    assert!(active_connections > 384 && active_connections < 768);
    assert!(inflight["batch"]["completed"].as_u64().expect("progress") < 128);
    eprintln!(
        "INFLIGHT completed={} accepted={active_connections}",
        inflight["batch"]["completed"]
    );
    ip(&["route", "add", "203.0.113.0/24", "dev", "lo"]);
    let inflight_stale = app.invalidated(&inflight["network_epoch"]);
    let after_epoch = fixture.accepted();
    thread::sleep(Duration::from_millis(250));
    assert!(
        fixture.accepted() <= after_epoch + 32,
        "нет автоматического продолжения batch"
    );
    app.send(json!({"command":"check_batch"}));
    app.wait(|v| v["batch"]["busy"] == true);
    let cancel = Instant::now();
    app.send(json!({"command":"cancel_batch"}));
    app.wait(|v| v["batch"]["cancelled"] == true && v["batch"]["busy"] == false);
    assert!(cancel.elapsed() < Duration::from_secs(2));
    eprintln!(
        "REAL_RTNETLINK completed_epoch={} invalidated_epoch={} inflight_epoch={} next_epoch={} cancel_ms={} host_ns={} isolated_ns={}",
        done["network_epoch"],
        completed_stale["network_epoch"],
        inflight["network_epoch"],
        inflight_stale["network_epoch"],
        cancel.elapsed().as_millis(),
        host_ns,
        fs::metadata("/proc/self/ns/net").expect("namespace").ino()
    );
}
