#![cfg(target_os = "linux")]

use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tofu_desktop::{DesktopHost, HostConfig};

const DEADLINE: Duration = Duration::from_secs(10);

struct IsolatedNetwork;
impl IsolatedNetwork {
    fn verify() -> Self {
        let host: u64 = std::env::var("TOFU_HOST_NETWORK_NS")
            .expect("обязателен inode host network namespace из родительского shell")
            .parse()
            .expect("TOFU_HOST_NETWORK_NS должен быть u64");
        assert_ne!(
            fs::metadata("/proc/self/ns/net")
                .expect("network namespace")
                .ino(),
            host,
            "запрещено изменять host network namespace"
        );
        let output = Command::new("ip")
            .args(["-j", "link", "show"])
            .output()
            .expect("ip link");
        assert!(output.status.success());
        let links: Value = serde_json::from_slice(&output.stdout).expect("JSON ip link");
        let links = links.as_array().expect("список интерфейсов");
        assert_eq!(links.len(), 1, "namespace должен содержать только lo");
        assert_eq!(links[0]["ifname"], "lo");
        Self
    }
    fn ip(&self, arguments: &[&str]) {
        let output = Command::new("ip")
            .args(arguments)
            .output()
            .expect("ip внутри namespace");
        assert!(
            output.status.success(),
            "ip: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("tofu-network-backend-{}", std::process::id()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("новый приватный каталог");
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("очистка только собственного каталога");
    }
}

#[derive(Default)]
struct Gate(Mutex<bool>, Condvar);
impl Gate {
    fn release(&self) {
        *self.0.lock().expect("gate lock") = true;
        self.1.notify_all();
    }
    fn wait(&self) {
        let (released, timeout) = self
            .1
            .wait_timeout_while(self.0.lock().expect("gate lock"), DEADLINE, |released| {
                !*released
            })
            .expect("gate wait");
        assert!(*released && !timeout.timed_out(), "callback gate deadline");
    }
}

struct Harness {
    host: DesktopHost,
    receiver: mpsc::Receiver<String>,
    gate: Arc<Gate>,
    block_next: Arc<AtomicBool>,
}
impl Harness {
    fn new(directory: &Directory) -> Self {
        let (sender, receiver) = mpsc::channel();
        let gate = Arc::new(Gate::default());
        let callback_gate = gate.clone();
        let block_next = Arc::new(AtomicBool::new(false));
        let callback_block = block_next.clone();
        let config = HostConfig::new(directory.0.clone(), directory.0.join("engines"))
            .with_ephemeral_key([77; 32]);
        let host = DesktopHost::start(config, move |snapshot| {
            let value: Value = serde_json::from_str(&snapshot).expect("JSON DTO");
            let block = value["tcp_busy"] == true && callback_block.swap(false, Ordering::SeqCst);
            let _ = sender.send(snapshot);
            if block {
                callback_gate.wait();
            }
        })
        .expect("DesktopHost");
        Self {
            host,
            receiver,
            gate,
            block_next,
        }
    }
    fn send(&self, value: Value) {
        self.host
            .dispatch(&value.to_string())
            .expect("команда backend");
    }
    fn wait(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let snapshot = self
                .receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("snapshot до deadline");
            let value: Value = serde_json::from_str(&snapshot).expect("JSON DTO");
            if predicate(&value) {
                return value;
            }
        }
    }
    fn close(&self) -> Value {
        self.host.shutdown();
        let closed = self.wait(|v| v["closed"] == true);
        assert!(self.host.wait_closed(DEADLINE));
        closed
    }
    fn stays_invalidated(&self) {
        let deadline = Instant::now() + Duration::from_millis(200);
        loop {
            match self
                .receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(snapshot) => {
                    assert_invalidated(&serde_json::from_str(&snapshot).expect("JSON DTO"))
                }
                Err(mpsc::RecvTimeoutError::Timeout) => return,
                Err(error) => panic!("backend должен оставаться открытым: {error}"),
            }
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.gate.release();
        self.host.shutdown();
        assert!(self.host.wait_closed(DEADLINE), "backend завершён");
    }
}

struct Fixtures {
    stop: Arc<AtomicBool>,
    accepted: Arc<AtomicU64>,
    tasks: Vec<thread::JoinHandle<()>>,
    url: String,
}
impl Fixtures {
    fn new() -> Self {
        let tcp = TcpListener::bind("127.0.0.1:0").expect("TCP fixture");
        let port = tcp.local_addr().expect("TCP адрес").port();
        let http = TcpListener::bind("127.0.0.1:0").expect("HTTP fixture");
        let url = format!(
            "http://{}/subscription",
            http.local_addr().expect("HTTP адрес")
        );
        let stop = Arc::new(AtomicBool::new(false));
        let accepted = Arc::new(AtomicU64::new(0));
        let mut tasks = Vec::new();
        for (listener, subscription) in [(tcp, false), (http, true)] {
            listener
                .set_nonblocking(true)
                .expect("nonblocking listener");
            let stop = stop.clone();
            let accepted = accepted.clone();
            tasks.push(thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(30);
                while !stop.load(Ordering::SeqCst) {
                    assert!(Instant::now() < deadline, "fixture deadline");
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_read_timeout(Some(Duration::from_secs(2))).expect("read timeout");
                            stream.set_write_timeout(Some(Duration::from_secs(2))).expect("write timeout");
                            if subscription {
                                read_headers(&mut stream);
                                let body = format!("trojan://fixture-secret@127.0.0.1:{port}#Локальный");
                                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).expect("HTTP ответ");
                            } else {
                                assert_eq!(stream.read(&mut [0]).expect("TCP EOF"), 0);
                                accepted.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
                        Err(error) => panic!("fixture accept: {error}"),
                    }
                }
            }));
        }
        Self {
            stop,
            accepted,
            tasks,
            url,
        }
    }
    fn wait_accepted(&self, expected: u64) {
        let deadline = Instant::now() + DEADLINE;
        while self.accepted.load(Ordering::SeqCst) < expected {
            assert!(Instant::now() < deadline, "TCP fixture завершила проверку");
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(self.accepted.load(Ordering::SeqCst), expected);
    }
}
impl Drop for Fixtures {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for task in self.tasks.drain(..) {
            task.join().expect("fixture thread завершён");
        }
    }
}

fn read_headers(stream: &mut TcpStream) {
    let mut request = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !request.ends_with(b"\r\n\r\n") {
        assert!(Instant::now() < deadline, "HTTP request deadline");
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).expect("HTTP запрос"), 1);
        request.push(byte[0]);
        assert!(request.len() <= 16384, "ограниченный HTTP запрос");
    }
}

fn assert_invalidated(value: &Value) {
    assert_eq!(value["tcp_busy"], false);
    assert_eq!(value["tcp_result"], Value::Null);
    assert_eq!(value["tcp_invalidated"], true);
    assert_eq!(value["status"], "Disconnected");
    assert_eq!(value["ready"], true);
}

#[test]
#[ignore = "только отдельный unshare network namespace с TOFU_HOST_NETWORK_NS"]
fn route_changes_invalidate_completed_and_inflight_tcp_checks() {
    let network = IsolatedNetwork::verify();
    network.ip(&["link", "set", "lo", "up"]);
    let directory = Directory::new();
    let fixtures = Fixtures::new();
    let app = Harness::new(&directory);
    let ready = app.wait(|v| v["ready"] == true && v["network_routes"] == true);
    assert!(ready["network_epoch"].is_string());
    assert_eq!(ready["network_routes"], true);
    assert!(ready["network_observed"].is_boolean());
    app.send(json!({"command":"add_source","name":"Локальный","url":fixtures.url,"local_exception":true,"route":"direct"}));
    app.wait(|v| v["source_id"].is_number());
    app.send(json!({"command":"refresh"}));
    let imported = app.wait(|v| v["total"] == 1 && v["busy"] == false);
    app.send(json!({"command":"select_profile","id":imported["profiles"][0]["id"],"revision":imported["revision"]}));
    app.wait(|v| v["selected_profile"].is_number());
    app.send(json!({"command":"check_tcp"}));
    let first = app.wait(|v| v["tcp_result"]["status"] == "reachable");
    assert_eq!(first["tcp_result"]["network_epoch"], first["network_epoch"]);
    assert_eq!(
        first["tcp_result"]["samples_ms"]
            .as_array()
            .expect("выборки")
            .len(),
        3
    );
    fixtures.wait_accepted(3);
    network.ip(&["route", "add", "198.51.100.0/24", "dev", "lo"]);
    let changed =
        app.wait(|v| v["network_epoch"] != first["network_epoch"] && v["tcp_invalidated"] == true);
    assert_invalidated(&changed);
    app.stays_invalidated();
    assert_eq!(fixtures.accepted.load(Ordering::SeqCst), 3);
    app.send(json!({"command":"check_tcp"}));
    let second = app.wait(|v| v["tcp_result"]["status"] == "reachable");
    assert_eq!(
        second["tcp_result"]["network_epoch"],
        changed["network_epoch"]
    );
    assert_eq!(second["tcp_invalidated"], false);
    assert_eq!(
        second["tcp_result"]["samples_ms"]
            .as_array()
            .expect("выборки")
            .len(),
        3
    );
    fixtures.wait_accepted(6);
    app.block_next.store(true, Ordering::SeqCst);
    app.send(json!({"command":"check_tcp"}));
    app.wait(|v| v["tcp_busy"] == true);
    network.ip(&["route", "add", "203.0.113.0/24", "dev", "lo"]);
    app.gate.release();
    let invalidated = app.wait(|v| {
        v["network_epoch"] != second["network_epoch"]
            && v["tcp_busy"] == false
            && v["tcp_invalidated"] == true
    });
    assert_invalidated(&invalidated);
    app.stays_invalidated();
    let closed = app.close();
    assert_eq!(closed["tcp_busy"], false);
    assert_eq!(closed["tcp_result"], Value::Null);
    assert_eq!(closed["tcp_invalidated"], true);
    assert!(
        fixtures.accepted.load(Ordering::SeqCst) <= 9,
        "нет автоматических TCP проверок"
    );
}
