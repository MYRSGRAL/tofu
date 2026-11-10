use super::*;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn daemon() -> Process {
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1", "--address"])
        .arg(std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(line.starts_with("unix:"));
    Process(child)
}
struct Provider {
    process: Process,
    output: BufReader<std::process::ChildStdout>,
}
impl Provider {
    fn new() -> Self {
        let mut process = Command::new("python")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/p10_bus.py"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = BufReader::new(process.stdout.take().unwrap());
        let mut result = Self {
            process: Process(process),
            output,
        };
        result.ack("ready");
        result
    }
    fn ack(&mut self, expected: &str) {
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), expected);
    }
    fn clients(&mut self) -> usize {
        writeln!(self.process.0.stdin.as_mut().unwrap(), "clients").unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        line.trim().parse().unwrap()
    }
    fn set(&mut self, command: &str) {
        writeln!(self.process.0.stdin.as_mut().unwrap(), "{command}").unwrap();
        self.ack("ack");
        thread::sleep(Duration::from_millis(200));
    }
}
#[test]
fn private_bus_desktop_gate() {
    if std::env::var_os("YONIK_P10_CHILD").is_none() {
        let dir = Directory::new();
        let mut child = Process(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "p10_e2e::private_bus_desktop_gate",
                    "--nocapture",
                ])
                .env("YONIK_P10_CHILD", "1")
                .env(
                    "DBUS_SYSTEM_BUS_ADDRESS",
                    format!("unix:path={}/bus", dir.0.display()),
                )
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(50);
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "deadline subprocess");
            thread::sleep(Duration::from_millis(100));
        }
        return;
    }
    let mut bus = daemon();
    let mut provider = Provider::new();
    let dir = Directory::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/sub", listener.local_addr().unwrap());
    let mut seed = Harness::new(dir.config());
    seed.wait(|v| v["ready"] == true);
    seed.send(json!({"command":"add_source","name":"Приёмка","url":url,"local_exception":true,"route":"direct"}));
    seed.wait(|v| v["source_id"].is_i64());
    seed.close();
    сделать_overdue(&dir);
    let config = || dir.config().with_system_subscription_policy(true);
    let mut app = Harness::new(config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"subscription_visibility","visible":true}));
    let blocked = || {
        thread::sleep(Duration::from_millis(350));
        assert!(matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
    };
    blocked(); // unknown NM snapshot
    provider.set("1");
    blocked();
    provider.set("power-saver");
    provider.set("2");
    blocked();
    provider.set("loss");
    provider.set("balanced");
    blocked();
    app.send(json!({"command":"subscription_visibility","visible":false}));
    thread::sleep(Duration::from_millis(200));
    provider.set("acquire");
    provider.set("1");
    provider.set("2");
    blocked();
    app.send(json!({"command":"subscription_visibility","visible":true}));
    accept_review(&listener, &mut app, 0);
    app.close();
    assert_eq!(provider.clients(), 1);
    сделать_overdue(&dir);
    app = Harness::new(config());
    app.wait(|v| v["ready"] == true);
    thread::sleep(Duration::from_millis(300));
    drop(provider);
    bus.0.kill().unwrap();
    bus.0.wait().unwrap();
    app.send(json!({"command":"subscription_visibility","visible":true}));
    blocked();
    // Реальный outage пересекает первую retry-задержку provider (1 секунда).
    thread::sleep(Duration::from_millis(1400));
    blocked();
    app.send(json!({"command":"subscription_visibility","visible":false}));
    bus = daemon();
    provider = Provider::new();
    provider.set("2");
    blocked();
    app.send(json!({"command":"subscription_visibility","visible":true}));
    accept_review(&listener, &mut app, 1);
    app.close();
    assert_eq!(provider.clients(), 1, "shutdown закрыл все bus connections");
    let mut reopened = Harness::new(config());
    reopened.wait(|v| v["ready"] == true);
    reopened.send(json!({"command":"subscription_visibility","visible":true}));
    blocked();
    reopened.close();
    assert_eq!(
        provider.clients(),
        1,
        "reopen shutdown без утечки соединений"
    );
    drop(provider);
    drop(bus);
}

fn accept_review(listener: &TcpListener, app: &mut Harness, previous_total: u64) {
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "provider recovery fetch");
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("{e}"),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut request = [0; 4096];
    assert!(stream.read(&mut request).unwrap() > 0);
    let body = "socks5://127.0.0.1:1080#Приёмка";
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
    drop(stream);
    let review = app.wait(|v| !v["pending"].is_null());
    assert_eq!(review["total"], previous_total);
    app.send(json!({"command":"approve_refresh"}));
    app.wait(|v| v["pending"].is_null() && v["total"] == 1);
}
