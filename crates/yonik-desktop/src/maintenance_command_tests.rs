//! Admission проверяется без запуска DesktopHost и системных служб.
use super::{Command, validate};

#[test]
fn startup_failure_rejects_each_maintenance_request_with_matching_ack() {
    use super::{DesktopHost, HostConfig};
    use std::os::unix::fs::DirBuilderExt;
    use std::time::Duration;
    let directory =
        std::env::temp_dir().join(format!("yonik-maintenance-startup-{}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .expect("fixture");
    let data = directory.join("data");
    std::fs::write(&data, b"not a directory").expect("startup fault");
    let (sender, receiver) = std::sync::mpsc::channel();
    let host = DesktopHost::start(
        HostConfig::new(data.clone(), directory.join("engines")).with_ephemeral_key([42; 32]),
        move |raw| {
            let _sent = sender.send(raw);
        },
    )
    .expect("host");
    loop {
        let raw = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("startup snapshot");
        let state: serde_json::Value = serde_json::from_str(&raw).expect("JSON");
        if !state["error"].is_null() {
            break;
        }
    }
    for request_id in [7, 8] {
        let command = if request_id == 7 {
            serde_json::json!({"command":"maintenance_start","request_id":request_id,"operation":"backup","destination":directory.join("new.sqlite"),"revision":0})
        } else {
            serde_json::json!({"command":"maintenance_cancel","request_id":request_id})
        };
        host.dispatch(&command.to_string()).expect("admission");
        let raw = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("matching ACK");
        let state: serde_json::Value = serde_json::from_str(&raw).expect("JSON");
        assert_eq!(state["maintenance_available"], false);
        assert_eq!(state["maintenance_ack"]["request_id"], request_id);
        assert_eq!(state["maintenance_ack"]["phase"], "failed");
        assert_eq!(state["maintenance_ack"]["error"], "startup");
        super::maintenance_job::assert_admission_context(
            &state,
            "maintenance_admission_unsupported",
        );
    }
    host.shutdown();
    assert!(host.wait_closed(Duration::from_secs(5)));
    drop(host);
    std::fs::remove_file(data).expect("очистка fixture");
    std::fs::remove_dir(directory).expect("очистка каталога");
}

#[test]
fn backup_accepts_only_authoritative_source_selection() {
    let parse = |raw: &str| serde_json::from_str::<Command>(raw).expect("JSON fixture");
    let allowed = parse(
        r#"{"command":"maintenance_start","request_id":1,"operation":"backup","destination":"/private/new.sqlite","revision":0}"#,
    );
    assert!(validate(&allowed).is_ok());
    let supplied = parse(
        r#"{"command":"maintenance_start","request_id":1,"operation":"backup","source":"/private/live.sqlite","destination":"/private/new.sqlite","revision":0}"#,
    );
    assert_eq!(
        validate(&supplied)
            .expect_err("источник Backup не задаётся GUI")
            .key(),
        "invalid_input"
    );
}

#[test]
fn rejects_nonlocal_restore_and_inexact_request_identity() {
    for raw in [
        r#"{"command":"maintenance_start","request_id":1,"operation":"restore","source":"file:///candidate","destination":"/private/new.sqlite","revision":0}"#,
        r#"{"command":"maintenance_cancel","request_id":9007199254740992}"#,
    ] {
        let command = serde_json::from_str::<Command>(raw).expect("JSON fixture");
        assert_eq!(
            validate(&command).expect_err("отказ admission").key(),
            "invalid_input"
        );
    }
}
