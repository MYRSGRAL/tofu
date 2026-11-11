#![cfg(all(target_os = "linux", feature = "startup-diagnostics"))]

use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};
use yonik_desktop::{DesktopHost, HostConfig, StartupPhase};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yonik-startup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("свой каталог");
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("очистка собственного fixture");
    }
}

#[test]
fn parallel_startup_reports_phases_and_keeps_owned_fds_close_on_exec() {
    // Все hosts запускаются до ожидания: тест сохраняет конкуренцию runtime/open gate.
    let hosts: Vec<_> = (0..16)
        .map(|_| {
            let fixture = Fixture::new();
            let phases = Arc::new(Mutex::new(Vec::new()));
            let observed = phases.clone();
            let config = HostConfig::new(fixture.0.clone(), fixture.0.join("engines"))
                .with_ephemeral_key([83; 32])
                .with_startup_observer(move |phase| observed.lock().expect("фазы").push(phase));
            let (tx, rx) = mpsc::channel();
            let host = DesktopHost::start(config, move |snapshot| {
                let _ = tx.send(snapshot);
            })
            .expect("host");
            (host, rx, phases, fixture)
        })
        .collect();
    for (_, rx, phases, fixture) in &hosts {
        loop {
            let snapshot = rx
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|error| {
                    panic!("{error:?}; phases={:?}", phases.lock().expect("фазы"))
                });
            let value: serde_json::Value = serde_json::from_str(&snapshot).expect("DTO");
            assert!(value["error"].is_null(), "startup error");
            if value["ready"] == true {
                break;
            }
        }
        assert_eq!(
            *phases.lock().expect("фазы"),
            [
                StartupPhase::RuntimeBuild,
                StartupPhase::SignalRegistration,
                StartupPhase::PortSelection,
                StartupPhase::DirectoryQueued,
                StartupPhase::DirectoryLock,
                StartupPhase::DirectoryReturned,
                StartupPhase::Credential,
                StartupPhase::RepositoryOpen,
                StartupPhase::RepositoryReturned,
                StartupPhase::Preferences,
                StartupPhase::NetworkInitialization,
                StartupPhase::NetworkReturned,
            ]
        );
        let mut directory_seen = false;
        let mut database_seen = false;
        for entry in fs::read_dir("/proc/self/fd").expect("fd inventory") {
            let entry = entry.expect("fd");
            let Ok(target) = fs::read_link(entry.path()) else {
                continue;
            };
            if target != fixture.0 && !target.starts_with(&fixture.0) {
                continue;
            }
            directory_seen |= target == fixture.0;
            database_seen |= target == fixture.0.join("subscriptions.sqlite");
            let info =
                fs::read_to_string(PathBuf::from("/proc/self/fdinfo").join(entry.file_name()))
                    .expect("свой fdinfo");
            let flags = info
                .lines()
                .find_map(|line| line.strip_prefix("flags:\t"))
                .expect("flags");
            assert_ne!(
                u64::from_str_radix(flags, 8).expect("octal") & 0o2000000,
                0,
                "свой startup FD должен иметь CLOEXEC"
            );
        }
        assert!(
            directory_seen && database_seen,
            "наблюдались directory lock и SQLite FD"
        );
    }
    for (host, _, _, _) in &hosts {
        host.shutdown();
    }
    for (host, _, _, _) in &hosts {
        assert!(host.wait_closed(Duration::from_secs(10)));
    }
}
