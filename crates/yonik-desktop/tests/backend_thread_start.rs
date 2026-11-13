//! Реальный отказ pthread_create изолирован в дочернем процессе, без production hooks.
#![cfg(target_os = "linux")]

use std::{process::Command, sync::mpsc};
use yonik_desktop::{DesktopHost, HostConfig, diagnostics::export_snapshot_v2};

#[test]
fn отказ_spawn_публикуется_до_возврата() {
    const CHILD: &str = "YONIK_THREAD_START_FAILURE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = Command::new(std::env::current_exe().expect("тестовый binary"))
            .args([
                "--exact",
                "отказ_spawn_публикуется_до_возврата",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .expect("дочерний процесс");
        assert!(status.success());
        return;
    }
    let config = HostConfig::new("/unused-data".into(), "/unused-engine".into());
    let (tx, rx) = mpsc::channel();
    let original = rustix::process::getrlimit(rustix::process::Resource::Nproc);
    let mut restricted = original;
    restricted.current = Some(0);
    rustix::process::setrlimit(rustix::process::Resource::Nproc, restricted)
        .expect("лимит только своего дочернего процесса");
    let result = DesktopHost::start(config, move |raw| tx.send(raw).expect("приёмник"));
    rustix::process::setrlimit(rustix::process::Resource::Nproc, original)
        .expect("восстановление soft limit");
    let Err(error) = result else {
        panic!("ожидался реальный отказ spawn; запускать без CAP_SYS_RESOURCE");
    };
    assert_eq!(error.key(), "startup");
    let raw = rx.try_recv().expect("публикация до возврата start");
    let metadata: serde_json::Value =
        serde_json::from_str(&export_snapshot_v2(&raw).expect("V2")).expect("JSON");
    assert_eq!(
        metadata["errors"][0]["reason"],
        "backend_thread_start_failed"
    );
    assert_eq!(metadata["errors"][1]["slot"], "connection");
    assert!(rx.try_recv().is_err());
}
