//! Ограниченный best-effort журнал; никакие произвольные строки сюда не поступают.
use super::{EventCode, EventContext, EventSource, LogEvent};
use rustix::fs::{self, AtFlags, Mode, OFlags};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const CAP: u64 = 4 * 1024 * 1024;
const NAMES: [&str; 5] = [
    "events.log",
    "events.1.log",
    "events.2.log",
    "events.3.log",
    "events.4.log",
];
const INTERVAL: Duration = Duration::from_secs(1);
type Record = (LogEvent, Option<EventContext>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskLogStatus {
    Starting,
    Active,
    Closed,
    Refused,
    IoFailed,
    ThreadFailed,
}
#[derive(Clone, Copy, Debug)]
pub struct DiskLogStats {
    pub status: DiskLogStatus,
    pub enqueued: u64,
    pub written: u64,
    pub dropped_full: u64,
    pub dropped_closed: u64,
    pub io_errors: u64,
    pub recovered_partial: u64,
}
#[derive(Default)]
struct State {
    #[cfg(any(test, feature = "test-support"))]
    after_open: Option<Arc<test_support::AfterOpen>>,
    stop: AtomicBool,
    admission: Mutex<()>,
    status: AtomicU8,
    enqueued: AtomicU64,
    written: AtomicU64,
    full: AtomicU64,
    closed: AtomicU64,
    errors: AtomicU64,
    recovered: AtomicU64,
    done: Mutex<bool>,
    changed: Condvar,
}
fn inc(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
        Some(n.saturating_add(1))
    });
}
pub(super) struct Producer {
    tx: SyncSender<Option<Record>>,
    state: Arc<State>,
}
impl Producer {
    pub(super) fn record(&self, event: LogEvent, context: Option<EventContext>) {
        let Ok(_admission) = self.state.admission.try_lock() else {
            inc(&self.state.full);
            return;
        };
        if self.state.stop.load(Ordering::Acquire) {
            inc(&self.state.closed);
            return;
        }
        match self.tx.try_send(Some((event, context))) {
            Ok(()) => inc(&self.state.enqueued),
            Err(TrySendError::Full(_)) => inc(&self.state.full),
            Err(TrySendError::Disconnected(_)) => inc(&self.state.closed),
        }
    }
}
pub struct DiskLogGuard {
    tx: SyncSender<Option<Record>>,
    state: Arc<State>,
    thread: Option<JoinHandle<()>>,
}
impl DiskLogGuard {
    pub fn stats(&self) -> DiskLogStats {
        let s = &self.state;
        DiskLogStats {
            status: match s.status.load(Ordering::Acquire) {
                0 => DiskLogStatus::Starting,
                1 => DiskLogStatus::Active,
                2 => DiskLogStatus::Closed,
                3 => DiskLogStatus::Refused,
                4 => DiskLogStatus::IoFailed,
                _ => DiskLogStatus::ThreadFailed,
            },
            enqueued: s.enqueued.load(Ordering::Relaxed),
            written: s.written.load(Ordering::Relaxed),
            dropped_full: s.full.load(Ordering::Relaxed),
            dropped_closed: s.closed.load(Ordering::Relaxed),
            io_errors: s.errors.load(Ordering::Relaxed),
            recovered_partial: s.recovered.load(Ordering::Relaxed),
        }
    }
    pub fn request_shutdown(&self) {
        self.state.stop.store(true, Ordering::Release);
        // Полная очередь уже разбудила consumer. Disconnected означает завершение.
        let _ = self.tx.try_send(None);
    }
    pub fn wait_closed(&self, timeout: Duration) -> bool {
        let Ok(done) = self.state.done.lock() else {
            return false;
        };
        if *done {
            return true;
        }
        self.state
            .changed
            .wait_timeout_while(done, timeout, |done| !*done)
            .is_ok_and(|(done, _)| *done)
    }
}
impl Drop for DiskLogGuard {
    fn drop(&mut self) {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(barrier) = &self.state.after_open {
            barrier.release();
        }
        self.request_shutdown();
        // Join разрешён только после завершения, Drop не подтверждает drain.
        if self.thread.as_ref().is_some_and(JoinHandle::is_finished) {
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}
pub(super) fn start(path: PathBuf) -> (Producer, DiskLogGuard) {
    start_inner(
        path,
        #[cfg(any(test, feature = "test-support"))]
        None,
    )
}
fn start_inner(
    path: PathBuf,
    #[cfg(any(test, feature = "test-support"))] after_open: Option<Arc<test_support::AfterOpen>>,
) -> (Producer, DiskLogGuard) {
    let (tx, rx) = mpsc::sync_channel(128);
    let state = Arc::new(State {
        #[cfg(any(test, feature = "test-support"))]
        after_open,
        ..State::default()
    });
    let worker = Arc::clone(&state);
    let thread = thread::Builder::new()
        .name("yonik-event-log".into())
        .spawn(move || {
            // Даже panic consumer не должен навсегда удерживать pending drain.
            let outcome = std::panic::catch_unwind(|| consume(&path, rx, &worker));
            if outcome.is_err() {
                worker.status.store(5, Ordering::Release);
                inc(&worker.errors);
            }
            worker.stop.store(true, Ordering::Release);
            if let Ok(mut done) = worker.done.lock() {
                *done = true;
                worker.changed.notify_all();
            }
        });
    let thread = match thread {
        Ok(thread) => Some(thread),
        Err(_) => {
            state.status.store(5, Ordering::Release);
            inc(&state.errors);
            state.stop.store(true, Ordering::Release);
            if let Ok(mut done) = state.done.lock() {
                *done = true;
            }
            None
        }
    };
    (
        Producer {
            tx: tx.clone(),
            state: Arc::clone(&state),
        },
        DiskLogGuard { tx, state, thread },
    )
}
fn bad() -> io::Error {
    io::Error::from(io::ErrorKind::PermissionDenied)
}
fn directory(path: &Path) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(bad());
    }
    let mut dir = File::from(fs::open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    for component in path.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(name) => {
                dir = File::from(fs::openat(
                    &dir,
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )?);
                let meta = dir.metadata()?;
                // Системные ancestors допустимы, но writable чужим каталог — нет.
                if ![0, rustix::process::geteuid().as_raw()].contains(&meta.uid())
                    || meta.mode() & 0o022 != 0
                {
                    return Err(bad());
                }
            }
            _ => return Err(bad()),
        }
    }
    let meta = dir.metadata()?;
    if meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o7777 != 0o700 {
        return Err(bad());
    }
    Ok(dir)
}
struct Sink {
    #[cfg(test)]
    test_path: PathBuf,
    dir: File,
    file: File,
    size: u64,
}
impl Sink {
    fn open(path: &Path, state: &State) -> io::Result<Self> {
        let root = directory(path)?;
        match fs::mkdirat(&root, "logs", Mode::RUSR | Mode::WUSR | Mode::XUSR) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => (),
            Err(e) => return Err(e.into()),
        }
        let dir = File::from(fs::openat(
            &root,
            "logs",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let meta = dir.metadata()?;
        if meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o7777 != 0o700 {
            return Err(bad());
        }
        fs::flock(&dir, fs::FlockOperation::NonBlockingLockExclusive)?;
        let files = NAMES
            .map(|name| checked(&dir, name))
            .into_iter()
            .collect::<io::Result<Vec<_>>>()?;
        for mut file in files.into_iter().flatten() {
            recover(&mut file, state)?;
        }
        let file = match checked(&dir, NAMES[0])? {
            Some(file) => file,
            None => create(&dir)?,
        };
        let size = file.metadata()?.len();
        Ok(Self {
            #[cfg(test)]
            test_path: path.to_owned(),
            dir,
            file,
            size,
        })
    }
    fn write(&mut self, record: Record) -> io::Result<()> {
        let text = encode(record);
        if self.size + text.len() as u64 > CAP {
            self.file.sync_data()?;
            // Полная проверка перед первой модификацией цепочки имён.
            for name in NAMES {
                checked(&self.dir, name)?;
            }
            match fs::unlinkat(&self.dir, NAMES[4], AtFlags::empty()) {
                Ok(()) | Err(rustix::io::Errno::NOENT) => (),
                Err(e) => return Err(e.into()),
            }
            for index in (0..4).rev() {
                match fs::renameat(&self.dir, NAMES[index], &self.dir, NAMES[index + 1]) {
                    Ok(()) | Err(rustix::io::Errno::NOENT) => (),
                    Err(e) => return Err(e.into()),
                }
            }
            #[cfg(test)]
            acceptance::checkpoint(&self.test_path, acceptance::Phase::Renamed);
            self.file = create(&self.dir)?;
            self.size = 0;
            self.dir.sync_all()?;
        }
        self.file.write_all(text.as_bytes())?;
        self.size += text.len() as u64;
        Ok(())
    }
}
fn regular(stat: &fs::Stat) -> bool {
    fs::FileType::from_raw_mode(stat.st_mode) == fs::FileType::RegularFile
        && stat.st_uid == rustix::process::geteuid().as_raw()
        && stat.st_nlink == 1
        && stat.st_mode & 0o7777 == 0o600
        && stat.st_size >= 0
        && stat.st_size as u64 <= CAP
}
fn checked(dir: &File, name: &str) -> io::Result<Option<File>> {
    let stat = match fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !regular(&stat) {
        return Err(bad());
    }

    let file = match fs::openat(
        dir,
        name,
        OFlags::RDWR | OFlags::APPEND | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !regular(&fs::fstat(&file)?) {
        return Err(bad());
    }
    Ok(Some(file))
}
fn create(dir: &File) -> io::Result<File> {
    let file = File::from(fs::openat(
        dir,
        NAMES[0],
        OFlags::RDWR
            | OFlags::APPEND
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )?);
    fs::fchmod(&file, Mode::RUSR | Mode::WUSR)?;
    Ok(file)
}

fn encode((e, context): Record) -> String {
    let source = match e.source {
        EventSource::App => 1,
        EventSource::Engine => 2,
    };
    let code = match e.code {
        EventCode::StartRequested => 1,
        EventCode::ProcessSpawned => 2,
        EventCode::StopRequested => 3,
        EventCode::CleanupConfirmed => 4,
        EventCode::CleanupFailed => 5,
        EventCode::ProcessExited => 6,
        EventCode::NativeSingStarted => 7,
        EventCode::NativeSingMissingInterface => 8,
        EventCode::NativeXrayStarted => 9,
        EventCode::NativeXrayLocalDns => 10,
    };
    let (present, session, generation) =
        context.map_or((0, 0, 0), |c| (1, c.session_id, c.generation));
    format!(
        "1 {} {source} {code} {} {} {present} {session} {generation}\n",
        e.id,
        u8::from(e.value.is_some()),
        e.value.unwrap_or(0)
    )
}
fn valid(line: &[u8]) -> bool {
    if line.len() > 192 || !line.is_ascii() {
        return false;
    }
    let Ok(text) = std::str::from_utf8(line) else {
        return false;
    };
    let fields: Vec<_> = text.split(' ').collect();
    if fields.len() != 9 || fields[0] != "1" {
        return false;
    }
    let code = match fields[3] {
        "1" | "2" | "3" | "4" | "5" | "6" => None,
        "7" => Some(EventCode::NativeSingStarted),
        "8" => Some(EventCode::NativeSingMissingInterface),
        "9" => Some(EventCode::NativeXrayStarted),
        "10" => Some(EventCode::NativeXrayLocalDns),
        _ => return false,
    };
    let Ok(value) = fields[5].parse::<i32>() else {
        return false;
    };
    let optional = match fields[4] {
        "0" => None,
        "1" => Some(value),
        _ => return false,
    };
    let native_valid = code.is_none_or(|code| {
        fields[2] == "2"
            && fields[6] == "1"
            && fields[7].parse::<u64>().is_ok_and(|n| n != 0)
            && fields[8].parse::<u64>().is_ok_and(|n| n != 0)
            && (optional.is_some() || value == 0)
            && super::native_details::project(EventSource::Engine, code, optional).is_some()
    });
    native_valid
        && fields[1].parse::<u64>().is_ok()
        && matches!(fields[2], "1" | "2")
        && matches!(fields[6], "0" | "1")
        && fields[7].parse::<u64>().is_ok()
        && fields[8].parse::<u64>().is_ok()
}
fn recover(file: &mut File, state: &State) -> io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(CAP + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > CAP {
        return Err(bad());
    }
    let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |n| n + 1);
    if bytes.len() - complete > 192 {
        return Err(bad());
    }
    if !(complete == 0 || bytes[..complete - 1].split(|b| *b == b'\n').all(valid)) {
        return Err(bad());
    }
    if complete != bytes.len() {
        file.set_len(complete as u64)?;
        file.sync_data()?;
        inc(&state.recovered);
    }
    Ok(())
}
fn consume(path: &Path, rx: Receiver<Option<Record>>, state: &State) {
    let mut sink = match Sink::open(path, state) {
        Ok(sink) => sink,
        Err(_) => {
            state.status.store(3, Ordering::Release);
            inc(&state.errors);
            return;
        }
    };
    state.status.store(1, Ordering::Release);
    #[cfg(any(test, feature = "test-support"))]
    if let Some(barrier) = &state.after_open {
        barrier.enter();
    }
    #[cfg(test)]
    acceptance::checkpoint(path, acceptance::Phase::Opened);
    let result = drain(&mut sink, &rx, state);
    conclude(state, result);
}
fn conclude(state: &State, result: io::Result<()>) {
    if result.is_err() {
        inc(&state.errors);
        state.status.store(4, Ordering::Release);
    } else {
        state.status.store(2, Ordering::Release);
    }
}
fn drain(sink: &mut Sink, rx: &Receiver<Option<Record>>, state: &State) -> io::Result<()> {
    let mut dirty = false;
    let mut deadline = Instant::now() + INTERVAL;
    loop {
        if dirty && Instant::now() >= deadline {
            sink.file.sync_data()?;
            dirty = false;
        }
        let message = if state.stop.load(Ordering::Acquire) {
            let _admission = state.admission.lock().map_err(|_| bad())?;
            match rx.try_recv() {
                Ok(message) => Some(message),
                Err(_) => None,
            }
        } else if dirty {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(message) => Some(message),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    sink.file.sync_data()?;
                    dirty = false;
                    continue;
                }
                Err(_) => None,
            }
        } else {
            rx.recv().ok()
        };
        match message {
            Some(Some(record)) => {
                sink.write(record)?;
                inc(&state.written);
                if !dirty {
                    deadline = Instant::now() + INTERVAL;
                }
                dirty = true;
            }
            Some(None) => (),
            None => {
                if dirty {
                    sink.file.sync_data()?;
                }
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    pub(super) struct Fixture(pub(super) PathBuf);
    impl Fixture {
        pub(super) fn new() -> Self {
            let root =
                PathBuf::from(std::env::var_os("HOME").expect("домашний каталог")).join(format!(
                    ".disk-log-test-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&root)
                .expect("fixture");
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).expect("собственная fixture");
        }
    }
    fn record() -> Record {
        (
            LogEvent {
                id: u64::MAX,
                source: EventSource::Engine,
                code: EventCode::CleanupFailed,
                value: Some(i32::MIN),
            },
            Some(EventContext {
                session_id: u64::MAX,
                generation: u64::MAX,
            }),
        )
    }
    #[test]
    fn rotation_restart_and_bounds() {
        const CHILD: &str = "YONIK_DISK_ROTATION_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // Соседний fork может удерживать OFD до exec при CLOEXEC и drop(sink).
            // Exact child не запускает соседние тесты в окне drop → reopen.
            let status = std::process::Command::new(std::env::current_exe().expect("libtest"))
                .args([
                    "--exact",
                    "event_log::disk::tests::rotation_restart_and_bounds",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .status()
                .expect("изолированный тест ротации");
            assert!(status.success(), "дочерний тест ротации: {status}");
            return;
        }
        let root = Fixture::new();
        let state = State::default();
        let mut sink = Sink::open(&root.0, &state).expect("sink");
        let line = encode(record());
        assert!(line.len() <= 192);
        let count = CAP as usize * 6 / line.len();
        for _ in 0..count {
            sink.write(record()).expect("write");
        }
        sink.file.sync_data().expect("sync");
        drop(sink);
        let sink = Sink::open(&root.0, &state).expect("restart");
        assert!(sink.size > 0);
        for name in NAMES {
            let meta = std::fs::metadata(root.0.join("logs").join(name)).expect("rotated");
            assert!(meta.len() <= CAP);
            assert_eq!(meta.mode() & 0o777, 0o600);
        }
        assert_eq!(
            std::fs::read_dir(root.0.join("logs"))
                .expect("entries")
                .count(),
            5
        );
        assert!(Sink::open(&root.0, &state).is_err(), "exclusive consumer");
    }
    #[test]
    fn recovery_and_invalid_complete_record() {
        let root = Fixture::new();
        let state = State::default();
        let mut sink = Sink::open(&root.0, &state).expect("sink");
        sink.write(record()).expect("record");
        sink.file.write_all(b"partial").expect("partial");
        drop(sink);
        let mut sink = Sink::open(&root.0, &state).expect("recover");
        assert_eq!(state.recovered.load(Ordering::Relaxed), 1);
        assert_eq!(sink.size, encode(record()).len() as u64);
        sink.file.write_all(b"SECRET\n").expect("negative control");
        drop(sink);
        assert!(Sink::open(&root.0, &state).is_err());
    }
    #[test]
    fn refuse_links_permissions_types_and_oversize() {
        for kind in 0..6 {
            let root = Fixture::new();
            let state = State::default();
            drop(Sink::open(&root.0, &state).expect("init"));
            let active = root.0.join("logs/events.log");
            match kind {
                0 => {
                    std::fs::remove_file(&active).expect("unlink");
                    symlink(root.0.join("outside"), &active).expect("symlink");
                }
                1 => std::fs::hard_link(&active, root.0.join("outside")).expect("hardlink"),
                2 => std::fs::set_permissions(&active, std::fs::Permissions::from_mode(0o644))
                    .expect("mode"),
                3 => File::options()
                    .write(true)
                    .open(&active)
                    .expect("file")
                    .set_len(CAP + 1)
                    .expect("size"),
                4 => {
                    std::fs::remove_file(&active).expect("unlink");
                    std::fs::create_dir(&active).expect("directory");
                }
                _ => std::fs::set_permissions(
                    root.0.join("logs"),
                    std::fs::Permissions::from_mode(0o755),
                )
                .expect("mode"),
            }
            assert!(Sink::open(&root.0, &state).is_err(), "control {kind}");
        }
    }
    #[test]
    fn bounded_queue_stop_and_secrets() {
        let (tx, rx) = mpsc::sync_channel(128);
        let state = Arc::new(State::default());
        let producer = Producer {
            tx: tx.clone(),
            state: Arc::clone(&state),
        };
        for _ in 0..1000 {
            producer.record(record().0, record().1);
        }
        assert_eq!(state.enqueued.load(Ordering::Relaxed), 128);
        assert_eq!(state.full.load(Ordering::Relaxed), 872);
        let guard = DiskLogGuard {
            tx,
            state,
            thread: None,
        };
        guard.request_shutdown();
        assert!(!guard.wait_closed(Duration::ZERO));
        producer.record(record().0, None);
        assert_eq!(guard.stats().dropped_closed, 1);
        assert_eq!(rx.try_iter().count(), 128);
        let root = Fixture::new();
        let (log, guard) = super::super::EventLog::with_disk(root.0.clone());
        log.reject_native(b"SECRET config://password\n\x1b");
        // Очередь принимается ещё до открытия sink.
        log.record(EventSource::App, EventCode::StopRequested, None);
        guard.request_shutdown();
        assert!(guard.wait_closed(Duration::from_secs(5)));
        assert_eq!(guard.stats().status, DiskLogStatus::Closed);
        assert_eq!(guard.stats().written, 1);
        let bytes = std::fs::read(root.0.join("logs/events.log")).expect("disk");
        assert_eq!(
            bytes,
            encode((
                LogEvent {
                    id: 1,
                    source: EventSource::App,
                    code: EventCode::StopRequested,
                    value: None
                },
                None
            ))
            .as_bytes()
        );
    }
    #[test]
    fn stalled_consumer_retains_guard_until_done() {
        let root = Fixture::new();
        let state = Arc::new(State::default());
        let (tx, rx) = mpsc::sync_channel(128);
        let (release, blocked) = mpsc::channel();
        let worker = Arc::clone(&state);
        let path = root.0.clone();
        let thread = thread::spawn(move || {
            blocked.recv().expect("разрешение fixture");
            consume(&path, rx, &worker);
            *worker.done.lock().expect("done") = true;
            worker.changed.notify_all();
        });
        let producer = Producer {
            tx: tx.clone(),
            state: Arc::clone(&state),
        };
        let guard = DiskLogGuard {
            tx,
            state,
            thread: Some(thread),
        };
        for _ in 0..129 {
            producer.record(record().0, record().1);
        }
        guard.request_shutdown();
        assert!(!guard.wait_closed(Duration::from_millis(100)));
        assert!(guard.thread.as_ref().is_some_and(|t| !t.is_finished()));
        assert_eq!(guard.stats().dropped_full, 1);
        release.send(()).expect("release");
        assert!(guard.wait_closed(Duration::from_secs(5)));
        assert_eq!(guard.stats().written, 128);
        assert_eq!(guard.stats().status, DiskLogStatus::Closed);
    }
    #[test]
    fn archived_symlink_and_root_symlink_refuse_before_recovery() {
        let root = Fixture::new();
        let state = State::default();
        let mut sink = Sink::open(&root.0, &state).expect("sink");
        sink.file.write_all(b"partial").expect("partial");
        drop(sink);
        symlink(root.0.join("outside"), root.0.join("logs/events.4.log")).expect("archive link");
        assert!(Sink::open(&root.0, &state).is_err());
        assert_eq!(
            std::fs::read(root.0.join("logs/events.log")).expect("unchanged"),
            b"partial"
        );
        symlink(&root.0, root.0.join("alias")).expect("alias");
        assert!(Sink::open(&root.0.join("alias"), &state).is_err());
    }
    #[test]
    fn idle_shutdown_and_write_failure() {
        let root = Fixture::new();
        let (_log, guard) = super::super::EventLog::with_disk(root.0.clone());
        // Не требуется событие, чтобы разбудить idle consumer.
        guard.request_shutdown();
        assert!(guard.wait_closed(Duration::from_secs(5)));
        assert_eq!(guard.stats().status, DiskLogStatus::Closed);
        drop(guard);
        let state = State::default();
        let mut sink = Sink::open(&root.0, &state).expect("sink");
        sink.file = File::open(root.0.join("logs/events.log")).expect("readonly fixture");
        let (tx, rx) = mpsc::sync_channel(128);
        tx.send(Some(record())).expect("record");
        drop(tx);
        assert!(drain(&mut sink, &rx, &state).is_err());
    }
    #[test]
    fn foreign_owner_metadata_is_refused_without_privileges() {
        let root = Fixture::new();
        let sink = Sink::open(&root.0, &State::default()).expect("sink");
        let mut stat = fs::fstat(&sink.file).expect("metadata");
        assert!(regular(&stat));
        // Отрицательный metadata fixture не требует chown/capabilities.
        stat.st_uid = stat.st_uid.wrapping_add(1);
        assert!(!regular(&stat));
    }
    #[test]
    fn sync_failure_has_closed_status_and_counter() {
        let root = Fixture::new();
        let state = State::default();
        let mut sink = Sink::open(&root.0, &state).expect("sink");
        // Test-only устройство принимает write, но не поддерживает fsync.
        // Production checked() запрещает любые non-regular файлы.
        sink.file = File::options()
            .write(true)
            .open("/dev/null")
            .expect("negative control");
        let (tx, rx) = mpsc::sync_channel(128);
        tx.send(Some(record())).expect("record");
        drop(tx);
        let result = drain(&mut sink, &rx, &state);
        assert!(result.is_err());
        assert_eq!(
            state.written.load(Ordering::Relaxed),
            1,
            "write прошёл до fsync"
        );
        conclude(&state, result);
        assert_eq!(state.status.load(Ordering::Acquire), 4);
        assert_eq!(state.errors.load(Ordering::Relaxed), 1);
    }
}

#[cfg(test)]
mod acceptance;

#[cfg(any(test, feature = "test-support"))]
pub(super) mod test_support;

#[cfg(test)]
mod native_codec_tests {
    use super::*;
    #[test]
    fn native_numeric_records_revalidate_and_legacy_reader_rejects_new_codes() {
        for (code, value) in [
            (EventCode::NativeSingStarted, Some(123)),
            (EventCode::NativeSingMissingInterface, None),
            (EventCode::NativeXrayStarted, None),
            (EventCode::NativeXrayLocalDns, None),
        ] {
            let line = encode((
                LogEvent {
                    id: u64::MAX,
                    source: EventSource::Engine,
                    code,
                    value,
                },
                Some(EventContext {
                    session_id: u64::MAX,
                    generation: u64::MAX,
                }),
            ));
            assert!(line.len() <= 192);
            assert_eq!(line.split_whitespace().count(), 9);
            assert!(valid(line.trim_end().as_bytes()));
            // Точный старый code allowlist, не запуск прежнего binary.
            let old_code = line.split_whitespace().nth(3).unwrap();
            assert!(!matches!(old_code, "1" | "2" | "3" | "4" | "5" | "6"));
            assert!(!line.contains("started"));
        }
        for line in [
            "1 1 2 7 0 0 1 1 1",
            "1 1 2 7 1 -1 1 1 1",
            "1 1 2 7 1 8640001 1 1 1",
            "1 1 1 9 0 0 1 1 1",
            "1 1 2 9 1 0 1 1 1",
            "1 1 2 9 0 42 1 1 1",
            "1 1 2 9 0 0 0 0 0",
            "1 1 2 11 0 0 1 1 1",
        ] {
            assert!(!valid(line.as_bytes()));
        }
    }
}
