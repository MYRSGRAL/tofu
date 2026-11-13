#![cfg(target_os = "linux")]

use std::{
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
use yonik_desktop::{DesktopHost, HostConfig};

#[test]
fn production_host_owns_sink_until_final_shutdown() {
    let root = PathBuf::from(std::env::var_os("HOME").expect("домашний каталог"))
        .join(format!(".yonik-disk-host-test-{}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .expect("fixture");
    let (tx, rx) = mpsc::channel();
    let host = DesktopHost::start(
        HostConfig::new(root.clone(), root.join("engines")).with_ephemeral_key([77; 32]),
        move |snapshot| {
            let _ = tx.send(snapshot);
        },
    )
    .expect("host");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("ready deadline");
        let value: serde_json::Value = serde_json::from_str(&snapshot).expect("snapshot");
        if value["ready"] == true {
            break;
        }
        assert_ne!(value["closed"], true, "startup завершился до ready");
    }
    #[cfg(feature = "test-support")]
    {
        let (tx, rx) = mpsc::channel();
        host.test_event_log(tx).expect("log handle request");
        let log = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("handle")
            .expect("log");
        log.reject_native(b"SECRET native/config/path");
        log.test_fill();
    }
    host.shutdown();
    assert!(host.wait_closed(Duration::from_secs(10)), "final drain");
    let bytes = std::fs::read(root.join("logs/events.log")).expect("production sink");
    #[cfg(feature = "test-support")]
    assert!(
        !bytes.is_empty(),
        "реальные типизированные события достигли диска"
    );
    assert!(!bytes.windows(6).any(|w| w == b"SECRET"));
    // После acknowledgement consumer уже освободил directory lock.
    let (_log, guard) = yonik_linux::event_log::EventLog::with_disk(root.clone());
    guard.request_shutdown();
    assert!(guard.wait_closed(Duration::from_secs(5)));
    assert_eq!(
        guard.stats().status,
        yonik_linux::event_log::DiskLogStatus::Closed
    );
    drop(guard);
    drop(host);
    std::fs::remove_dir_all(root).expect("очистка собственной fixture");
}
