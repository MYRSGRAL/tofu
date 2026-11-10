use crate::backend::{Directory, Harness, сделать_overdue};
use rustix::process::{self, Pid, Signal, WaitOptions};
use serde_json::json;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    os::unix::{fs::DirBuilderExt, process::CommandExt},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use yonik_linux::{SubscriptionPolicy, policy_test_support::PolicyBus};

/// Владеет только собственной group; отдельный harness — subreaper, без
/// изменения process-wide атрибутов обычного параллельного Rust test binary.
struct Group {
    child: Child,
    pgid: Pid,
    stopped: bool,
}
impl Group {
    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        for (signal, budget) in [(Signal::TERM, 300), (Signal::KILL, 2000)] {
            let _ = process::kill_process_group(self.pgid, signal);
            let deadline = Instant::now() + Duration::from_millis(budget);
            loop {
                match process::waitpgid(self.pgid, WaitOptions::NOHANG) {
                    Ok(Some(_)) => continue,
                    Err(rustix::io::Errno::CHILD) => {
                        self.stopped = true;
                        return;
                    }
                    Ok(None) => {}
                    Err(e) => panic!("reap собственной group: {e}"),
                }
                assert!(
                    signal != Signal::KILL || Instant::now() < deadline,
                    "reap deadline"
                );
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
impl Drop for Group {
    fn drop(&mut self) {
        self.stop();
    }
}

fn supervised(mode: &str, timeout: Duration) -> (bool, std::path::PathBuf, Pid) {
    let dir = Directory::new();
    let path = dir.0.clone();
    let tools = path.join("tools");
    fs::create_dir(&tools).unwrap();
    for name in ["dbus-daemon", "sh", "sleep"] {
        std::os::unix::fs::symlink(format!("/usr/bin/{name}"), tools.join(name)).unwrap();
    }
    let child = Command::new(std::env::current_exe().unwrap())
        .arg(mode)
        .current_dir(&path)
        .env("PATH", &tools)
        .process_group(0)
        .env(
            "DBUS_SYSTEM_BUS_ADDRESS",
            format!("unix:path={}/bus", path.display()),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let pgid = Pid::from_raw(child.id() as i32).unwrap();
    let mut group = Group {
        child,
        pgid,
        stopped: false,
    };
    let output = group.child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + timeout;
    let mut created = false;
    let completed = loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(line)) if line == "done" => break true,
            Ok(Ok(line)) => {
                created |= line == "fixture-created";
            }
            _ => break false,
        }
    };
    // Даже normal worker обязан завершиться до общего deadline.
    let success = if completed {
        loop {
            if let Some(status) = group.child.try_wait().unwrap() {
                break status.success();
            }
            if Instant::now() >= deadline {
                break false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    } else {
        false
    };
    group.stop();
    if mode == "--hang-before-ready" {
        assert!(created, "регрессия реально создала daemon и DB");
    }
    reader.join().unwrap(); // EOF гарантирован после уничтожения всех writers group.
    assert_eq!(
        process::test_kill_process_group(pgid),
        Err(rustix::io::Errno::SRCH)
    );
    drop(group);
    drop(dir); // Только parent удаляет своё дерево, включая DB/WAL и socket.
    (success, path, pgid)
}

fn daemon() -> Child {
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1", "--address"])
        .arg(std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(output).read_line(&mut line);
        let _ = tx.send((result, line));
    });
    let (result, line) = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("bounded daemon ready");
    result.unwrap();
    assert!(line.starts_with("unix:"));
    child
}

pub fn run() {
    match std::env::args().nth(1).as_deref() {
        Some("--worker") => {
            worker();
            println!("done");
        }
        Some("--hang-before-ready") => {
            // Реальная DB и daemon остаются живы до parent timeout.
            let mut bus = daemon();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let _repo = yonik_linux::EncryptedSubscriptionRepository::open(
                    std::env::current_dir().unwrap().join("own.sqlite"),
                    zeroize::Zeroizing::new([77; 32]),
                )
                .await
                .unwrap();
            });
            let mut stubborn = Command::new("sh")
                .args(["-c", "trap '' TERM; exec sleep 300"])
                .spawn()
                .unwrap();
            println!("fixture-created");
            // Worker ожидает собственного ребёнка; при timeout parent становится
            // его subreaper и завершает/reaps всю group, включая daemon.
            stubborn.wait().unwrap();
            bus.wait().unwrap();
        }
        None => {
            process::set_child_subreaper(Some(process::getpid())).unwrap();
            let (success, path, pgid) = supervised("--hang-before-ready", Duration::from_secs(2));
            assert!(!success, "ready не пришёл — timeout обязателен");
            assert!(!path.exists(), "parent удалил own DB/tree");
            assert_eq!(
                process::test_kill_process_group(pgid),
                Err(rustix::io::Errno::SRCH)
            );
            println!("P2 orphan timeout: GREEN");
            let (success, path, _) = supervised("--worker", Duration::from_secs(45));
            assert!(success, "сквозная приёмка");
            assert!(!path.exists());
            println!("P2 barrier и dependency-free Rust provider: GREEN");
        }
        _ => panic!("неизвестный режим собственного harness"),
    }
}

struct Observations(mpsc::Receiver<(SubscriptionPolicy, bool)>);
impl Observations {
    fn wait(&self, expected: SubscriptionPolicy) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let (policy, loaded) = self
                .0
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("effective policy barrier");
            if loaded && policy_fields(policy) == policy_fields(expected) {
                break;
            }
        }
    }
}
fn policy_fields(p: SubscriptionPolicy) -> (bool, bool, bool, bool, bool) {
    (
        p.online,
        p.metered,
        p.battery_saver,
        p.ui_visible,
        p.background_window,
    )
}
fn expected(online: bool, metered: bool, saver: bool, visible: bool) -> SubscriptionPolicy {
    SubscriptionPolicy {
        online,
        metered,
        battery_saver: saver,
        ui_visible: visible,
        ..Default::default()
    }
}
fn start(dir: &Directory) -> (Harness, Observations) {
    let (tx, rx) = mpsc::channel();
    let config = dir
        .config()
        .with_system_subscription_policy(true)
        .with_schedule_observer(move |policy, loaded| {
            let _ = tx.send((policy, loaded));
        });
    let mut app = Harness::new(config);
    app.wait(|v| v["ready"] == true);
    (app, Observations(rx))
}
fn bounded<F: std::future::Future>(runtime: &tokio::runtime::Handle, future: F) -> F::Output {
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), future)
            .await
            .expect("bounded provider operation")
    })
}
fn worker() {
    for name in ["python", "python3"] {
        assert_eq!(
            Command::new(name).spawn().unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }
    let mut bus = daemon();
    let driver = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let runtime = driver.handle().clone();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let driver_thread = thread::spawn(move || {
        driver.block_on(async {
            let _ = stopped.await;
        })
    });
    let address = std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap();
    let mut provider = bounded(&runtime, PolicyBus::new(&address)).unwrap();
    let app_path = std::env::current_dir().unwrap().join("app");
    fs::DirBuilder::new().mode(0o700).create(&app_path).unwrap();
    // У child нет права удалять cwd или путь из inherited env; cleanup у parent.
    let dir = std::mem::ManuallyDrop::new(Directory(app_path));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/sub", listener.local_addr().unwrap());
    let mut seed = Harness::new(dir.config());
    seed.wait(|v| v["ready"] == true);
    seed.send(json!({"command":"add_source","name":"Приёмка","url":url,"local_exception":true,"route":"direct"}));
    seed.wait(|v| v["source_id"].is_i64());
    seed.close();
    сделать_overdue(&dir);
    let (mut app, mut obs) = start(&dir);
    obs.wait(expected(true, false, false, false)); // healthy + schedule loaded
    let blocked = || {
        thread::sleep(Duration::from_millis(150)); // Окно наблюдения после реального barrier.
        assert!(matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
    };
    blocked();
    for (metered, saver, policy) in [
        (0, false, expected(false, true, false, true)),
        (1, false, expected(false, true, false, true)),
        (2, true, expected(true, false, true, true)),
    ] {
        // Перед каждой новой проверкой вернуться в наблюдённый healthy hidden.
        app.send(json!({"command":"subscription_visibility","visible":false}));
        bounded(&runtime, provider.set(2, false)).unwrap();
        obs.wait(expected(true, false, false, false));
        if metered == 0 {
            provider.hold_next_snapshot();
        }
        bounded(&runtime, provider.set(metered, saver)).unwrap();
        if metered == 0 {
            bounded(&runtime, provider.snapshot_entered());
            let target = expected(policy.online, policy.metered, policy.battery_saver, false);
            let (applied, observed) = mpsc::channel();
            let barrier = thread::spawn(move || {
                obs.wait(target);
                applied.send(()).unwrap();
                obs
            });
            assert!(
                matches!(
                    observed.recv_timeout(Duration::from_millis(600)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ),
                "negative не засчитывается до delayed GetAll"
            );
            provider.release_snapshot();
            observed.recv_timeout(Duration::from_secs(8)).unwrap();
            obs = barrier.join().unwrap();
        } else {
            obs.wait(expected(
                policy.online,
                policy.metered,
                policy.battery_saver,
                false,
            ));
        }
        app.send(json!({"command":"subscription_visibility","visible":true}));
        obs.wait(policy);
        blocked();
    }
    bounded(&runtime, provider.owner(false)).unwrap();
    obs.wait(expected(false, false, false, true));
    blocked();
    app.send(json!({"command":"subscription_visibility","visible":false}));
    obs.wait(expected(false, false, false, false));
    bounded(&runtime, provider.set(2, false)).unwrap();
    bounded(&runtime, provider.owner(true)).unwrap();
    obs.wait(expected(true, false, false, false));
    blocked();
    app.send(json!({"command":"subscription_visibility","visible":true}));
    obs.wait(expected(true, false, false, true));
    accept_review(&listener, &mut app, 0);
    app.close();
    assert_eq!(bounded(&runtime, provider.clients()).unwrap(), 1);
    сделать_overdue(&dir);
    let (next, next_obs) = start(&dir);
    app = next;
    next_obs.wait(expected(true, false, false, false));
    drop(provider);
    bus.kill().unwrap();
    bus.wait().unwrap();
    next_obs.wait(expected(false, false, false, false));
    app.send(json!({"command":"subscription_visibility","visible":true}));
    next_obs.wait(expected(false, false, false, true));
    blocked();
    thread::sleep(Duration::from_millis(1400));
    blocked();
    app.send(json!({"command":"subscription_visibility","visible":false}));
    next_obs.wait(expected(false, false, false, false));
    bus = daemon();
    provider = bounded(&runtime, PolicyBus::new(&address)).unwrap();
    next_obs.wait(expected(true, false, false, false));
    blocked();
    app.send(json!({"command":"subscription_visibility","visible":true}));
    next_obs.wait(expected(true, false, false, true));
    accept_review(&listener, &mut app, 1);
    app.close();
    assert_eq!(bounded(&runtime, provider.clients()).unwrap(), 1);
    // Deliberate delayed GetAll > старого окна 350 мс. Reopen не имеет права
    // засчитать absence до healthy+visible+loadschedule.
    provider.hold_next_snapshot();
    let (mut reopened, reopen_obs) = start(&dir);
    bounded(&runtime, async {
        tokio::time::timeout(Duration::from_secs(3), provider.snapshot_entered())
            .await
            .unwrap();
    });
    reopened.send(json!({"command":"subscription_visibility","visible":true}));
    let (applied, observed) = mpsc::channel();
    let barrier = thread::spawn(move || {
        reopen_obs.wait(expected(true, false, false, true));
        applied.send(()).unwrap();
    });
    assert!(
        matches!(
            observed.recv_timeout(Duration::from_millis(600)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ),
        "barrier не может принять initial unknown за healthy"
    );
    provider.release_snapshot();
    observed.recv_timeout(Duration::from_secs(8)).unwrap();
    barrier.join().unwrap();
    blocked();
    reopened.close();
    assert_eq!(bounded(&runtime, provider.clients()).unwrap(), 1);
    drop(provider);
    bus.kill().unwrap();
    bus.wait().unwrap();
    stop.send(()).unwrap();
    driver_thread.join().unwrap();
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
