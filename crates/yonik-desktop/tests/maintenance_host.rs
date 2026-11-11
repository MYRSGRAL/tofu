//! Запускать через dbus-run-session; только ephemeral БД и приватные пути.
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    sync::mpsc,
    time::{Duration, Instant},
};
use yonik_desktop::{DesktopHost, HostConfig};

fn wait(
    receiver: &mpsc::Receiver<serde_json::Value>,
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("snapshot до deadline");
        if predicate(&state) {
            return state;
        }
    }
}

#[test]
#[ignore = "требует private D-Bus: YONIK_PRIVATE_MAINTENANCE_TEST=1 dbus-run-session"]
fn desktop_host_backup_restore_new_path_and_shutdown() {
    assert!(
        std::env::var_os("YONIK_PRIVATE_MAINTENANCE_TEST").is_some(),
        "тест требует явного private D-Bus fixture запуска"
    );
    let root = std::env::temp_dir().join(format!("yonik-maintenance-host-{}", std::process::id()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .expect("fixture");
    let destination = root.join("copies");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&destination)
        .expect("private destination");
    let (sender, receiver) = mpsc::channel();
    let host = DesktopHost::start(
        HostConfig::new(root.join("data"), root.join("engines")).with_ephemeral_key([61; 32]),
        move |raw| {
            if let Ok(value) = serde_json::from_str(&raw) {
                let _closed_receiver = sender.send(value);
            }
        },
    )
    .expect("host");
    let state = wait(&receiver, |state| state["ready"] == true);
    assert_eq!(state["maintenance_available"], true);
    let backup = destination.join("backup.sqlite");
    let restored = destination.join("restored.sqlite");
    for (id, operation, source, target) in [
        (1, "backup", None, &backup),
        (2, "restore", Some(&backup), &restored),
    ] {
        let mut command = serde_json::json!({"command":"maintenance_start", "request_id":id, "operation":operation, "destination":target, "revision":state["revision"]});
        if let Some(source) = source {
            command["source"] = serde_json::json!(source);
        }
        host.dispatch(&command.to_string())
            .expect("maintenance command");
        let finished = wait(&receiver, |state| {
            state["maintenance_ack"]["request_id"] == id
                && matches!(
                    state["maintenance_ack"]["phase"].as_str(),
                    Some("complete" | "failed")
                )
        });
        assert_eq!(
            finished["maintenance_ack"]["error"],
            serde_json::Value::Null
        );
        assert_eq!(finished["ready"], true);
        assert!(target.exists());
    }
    host.shutdown();
    assert!(host.wait_closed(Duration::from_secs(10)));
    drop(host);
    for directory in [root.join("data"), destination] {
        for entry in fs::read_dir(&directory).expect("fixture") {
            fs::remove_file(entry.expect("файл").path()).expect("очистка");
        }
        fs::remove_dir(directory).expect("очистка каталога");
    }
    fs::remove_dir(root).expect("очистка fixture");
}
