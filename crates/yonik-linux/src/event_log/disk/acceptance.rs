//! Закрытые одноразовые барьеры actual consumer, привязанные к owned fixture.
use super::{
    CAP, DiskLogGuard, DiskLogStatus, EventCode, EventContext, EventSource, LogEvent, NAMES,
    encode, tests, valid,
};
use crate::event_log::{EventLog, LogFilter};
use std::collections::HashMap;
use std::sync::LazyLock;
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Opened,
    Renamed,
}
struct Gate {
    phase: Phase,
    entered: mpsc::Sender<()>,
    release: Mutex<Receiver<()>>,
}
static GATES: LazyLock<Mutex<HashMap<PathBuf, Arc<Gate>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
pub(super) fn checkpoint(path: &Path, phase: Phase) {
    let gate = {
        let mut gates = GATES.lock().expect("barriers");
        if gates.get(path).is_some_and(|g| g.phase == phase) {
            gates.remove(path)
        } else {
            None
        }
    };
    if let Some(gate) = gate {
        gate.entered.send(()).expect("observer");
        gate.release
            .lock()
            .expect("release")
            .recv_timeout(Duration::from_secs(30))
            .expect("bounded release");
    }
}
fn gate(root: &Path, phase: Phase) -> (Receiver<()>, mpsc::Sender<()>) {
    let (entered, reached) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    assert!(
        GATES
            .lock()
            .expect("barriers")
            .insert(
                root.to_owned(),
                Arc::new(Gate {
                    phase,
                    entered,
                    release: Mutex::new(blocked),
                })
            )
            .is_none()
    );
    (reached, release)
}
fn emit(log: &EventLog) {
    log.record(EventSource::App, EventCode::StopRequested, Some(42));
}
fn finish(guard: &DiskLogGuard) {
    guard.request_shutdown();
    assert!(guard.wait_closed(Duration::from_secs(10)), "bounded drain");
}
// Валидный файл ровно CAP: следующий consumer write обязан вызвать ротацию.
fn seed(root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir(root.join("logs")).expect("logs");
    std::fs::set_permissions(root.join("logs"), std::fs::Permissions::from_mode(0o700))
        .expect("mode");
    let line = b"1 1 1 1 0 0 0 0 0\n";
    let count = CAP as usize / line.len() - 1;
    let mut bytes = line.repeat(count);
    let remaining = CAP as usize - bytes.len();
    bytes.extend_from_slice(b"1 ");
    bytes.extend(std::iter::repeat_n(b'0', remaining - line.len()));
    bytes.extend_from_slice(&line[2..]);
    assert_eq!(bytes.len() as u64, CAP);
    for name in NAMES {
        let path = root.join("logs").join(name);
        std::fs::write(&path, &bytes).expect("seed");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).expect("mode");
    }
}
#[test]
fn actual_consumer_stall_queue_and_export() {
    let root = tests::Fixture::new();
    let (entered, release) = gate(&root.0, Phase::Opened);
    let (log, guard) = EventLog::with_disk(root.0.clone());
    entered
        .recv_timeout(Duration::from_secs(10))
        .expect("actual open");
    let start = Instant::now();
    for _ in 0..1000 {
        emit(&log);
    }
    log.reject_native(b"FR16_SECRET://password\r\n\x1b[31m\0");
    log.record(EventSource::Engine, EventCode::ProcessExited, Some(-7));
    log.record(EventSource::App, EventCode::ProcessExited, Some(-8));
    log.record(EventSource::Engine, EventCode::StopRequested, None);
    guard.request_shutdown();
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "producer/shutdown ждут диск"
    );
    let stats = guard.stats();
    assert_eq!(
        (stats.enqueued, stats.written, stats.dropped_full),
        (128, 0, 875)
    );
    emit(&log);
    assert_eq!(guard.stats().dropped_closed, 1);
    assert!(
        !guard.wait_closed(Duration::from_millis(50)),
        "ложный drain"
    );
    assert_eq!(guard.stats().status, DiskLogStatus::Active);
    let page = log
        .read(
            0,
            LogFilter {
                source: Some(EventSource::Engine),
                code: Some(EventCode::ProcessExited),
            },
            256,
        )
        .expect("page");
    let export = page.export();
    assert_eq!(page.events.len(), 1);
    assert!(export.contains("Engine ProcessExited Some(-7)"));
    assert!(!export.contains("StopRequested"));
    assert!(!export.contains("FR16_SECRET"));
    assert!(!export.chars().any(|c| c.is_control() && c != '\n'));
    release.send(()).expect("release");
    finish(&guard);
    assert_eq!((guard.stats().written, guard.stats().io_errors), (128, 0));
    assert_eq!(guard.stats().status, DiskLogStatus::Closed);
    let bytes = std::fs::read(root.0.join("logs/events.log")).expect("disk");
    assert_eq!(
        bytes
            .split(|b| *b == b'\n')
            .filter(|s| !s.is_empty())
            .count(),
        128
    );
    assert!(
        bytes
            .split(|b| *b == b'\n')
            .filter(|s| !s.is_empty())
            .all(valid)
    );
}
#[test]
fn actual_consumer_rotation_restart_and_first_io_error() {
    let root = tests::Fixture::new();
    seed(&root.0);
    let (log, guard) = EventLog::with_disk(root.0.clone());
    emit(&log);
    finish(&guard);
    assert_eq!(guard.stats().written, 1);
    assert_eq!(guard.stats().status, DiskLogStatus::Closed);
    for name in NAMES {
        assert!(
            std::fs::metadata(root.0.join("logs").join(name))
                .expect("file")
                .len()
                <= CAP
        );
    }
    assert_eq!(
        std::fs::read_dir(root.0.join("logs"))
            .expect("files")
            .count(),
        5
    );
    let active = root.0.join("logs/events.log");
    File::options()
        .append(true)
        .open(&active)
        .expect("active")
        .write_all(b"partial")
        .expect("tail");
    let (log, guard) = EventLog::with_disk(root.0.clone());
    emit(&log);
    finish(&guard);
    assert_eq!(guard.stats().recovered_partial, 1);
    assert_eq!(guard.stats().written, 1);
    let recovered = std::fs::read(&active).expect("recovered bytes");
    assert!(
        recovered
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .all(valid)
    );
    assert!(!recovered.windows(7).any(|part| part == b"partial"));
    // Реальная ошибка rotation проверки после open; fd и consumer не заменяются.
    let fault = tests::Fixture::new();
    seed(&fault.0);
    let (entered, release) = gate(&fault.0, Phase::Opened);
    let (log, guard) = EventLog::with_disk(fault.0.clone());
    entered
        .recv_timeout(Duration::from_secs(10))
        .expect("opened");
    std::fs::remove_file(fault.0.join("logs/events.4.log")).expect("own archive");
    std::fs::create_dir(fault.0.join("logs/events.4.log")).expect("invalid archive");
    emit(&log);
    emit(&log);
    release.send(()).expect("release");
    assert!(guard.wait_closed(Duration::from_secs(10)));
    assert_eq!(guard.stats().status, DiskLogStatus::IoFailed);
    assert_eq!((guard.stats().io_errors, guard.stats().written), (1, 0));
    emit(&log);
    assert_eq!(guard.stats().dropped_closed, 1);
    assert_eq!(guard.stats().io_errors, 1);
    assert_eq!(
        std::fs::metadata(fault.0.join("logs/events.log"))
            .expect("unchanged")
            .len(),
        CAP
    );
}
#[test]
fn actual_consumer_crash_after_rename_chain() {
    const CHILD: &str = "YONIK_FR16_CRASH_OWN_ROOT";
    if let Some(path) = std::env::var_os(CHILD) {
        let root = PathBuf::from(path);
        let (entered, _release) = gate(&root, Phase::Renamed);
        let (log, guard) = EventLog::with_disk(root.clone());
        emit(&log);
        entered
            .recv_timeout(Duration::from_secs(10))
            .expect("rename chain reached");
        // Marker подтверждает завершение rename-chain, до create нового active.
        std::fs::write(
            root.join("renamed.marker"),
            b"after rename chain; before create",
        )
        .expect("marker");
        assert!(
            guard.wait_closed(Duration::from_secs(20)),
            "parent должен SIGKILL child"
        );
        panic!("child не должен завершаться штатно");
    }
    let root = tests::Fixture::new();
    seed(&root.0);
    let mut child = std::process::Command::new(std::env::current_exe().expect("libtest"))
        .args([
            "--exact",
            "event_log::disk::acceptance::actual_consumer_crash_after_rename_chain",
            "--test-threads=1",
        ])
        .env(CHILD, &root.0)
        .spawn()
        .expect("child");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !root.0.join("renamed.marker").exists() && Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("status") {
            panic!("early child: {status}");
        }
        thread::sleep(Duration::from_millis(20));
    }
    let reached = root.0.join("renamed.marker").exists();
    child.kill().expect("SIGKILL own child");
    let status = child.wait().expect("reap");
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(status.signal(), Some(9));
    assert!(reached, "не достигнут checkpoint");
    assert!(!root.0.join("logs/events.log").exists());
    for name in &NAMES[1..] {
        assert_eq!(
            std::fs::metadata(root.0.join("logs").join(name))
                .expect("archive")
                .len(),
            CAP
        );
    }
    let (log, guard) = EventLog::with_disk(root.0.clone());
    emit(&log);
    finish(&guard);
    assert_eq!(guard.stats().status, DiskLogStatus::Closed);
    assert_eq!((guard.stats().written, guard.stats().io_errors), (1, 0));
    assert_eq!(
        std::fs::read_dir(root.0.join("logs"))
            .expect("files")
            .count(),
        5
    );
}

#[test]
fn actual_consumer_six_segments_real_writes() {
    let root = tests::Fixture::new();
    let (log, guard) = EventLog::with_disk(root.0.clone());
    let log = log.with_context(EventContext {
        session_id: u64::MAX,
        generation: u64::MAX,
    });
    let bytes_per_record = encode((
        LogEvent {
            id: 1,
            source: EventSource::App,
            code: EventCode::StopRequested,
            value: Some(42),
        },
        Some(EventContext {
            session_id: u64::MAX,
            generation: u64::MAX,
        }),
    ))
    .len();
    let count = CAP as usize * 6 / bytes_per_record + 1;
    let deadline = Instant::now() + Duration::from_secs(60);
    for batch in (0..count).step_by(128) {
        for _ in batch..(batch + 128).min(count) {
            emit(&log);
        }
        let admitted = guard.stats().enqueued;
        while guard.stats().written < admitted {
            assert!(Instant::now() < deadline, "consumer progress deadline");
            assert_eq!(guard.stats().io_errors, 0);
            thread::sleep(Duration::from_millis(1));
        }
    }
    finish(&guard);
    assert_eq!(guard.stats().status, DiskLogStatus::Closed);
    assert_eq!(guard.stats().written, count as u64);
    assert_eq!(guard.stats().dropped_full, 0);
    let mut previous = 0;
    let mut retained = 0;
    for name in NAMES.iter().rev() {
        let bytes = std::fs::read(root.0.join("logs").join(name)).expect("segment");
        assert!(bytes.len() as u64 <= CAP);
        if *name != NAMES[0] {
            assert!(bytes.len() as u64 > CAP - 192);
        }
        retained += bytes.len() as u64;
        for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
            assert!(valid(line));
            let id = std::str::from_utf8(line)
                .expect("ascii")
                .split(' ')
                .nth(1)
                .expect("id")
                .parse::<u64>()
                .expect("number");
            assert!(id > previous, "rotation history order");
            previous = id;
        }
    }
    assert_eq!(previous, count as u64);
    assert!(retained > CAP * 4);
}
