#![cfg(feature = "test-support")]
//! Только собственный ephemeral fixture: настройки не запускают Connect.
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    sync::mpsc,
    time::{Duration, Instant},
};
use yonik_desktop::{DesktopHost, HostConfig};
fn until(rx: &mpsc::Receiver<String>, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let snapshot: Value = serde_json::from_str(
            &rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("deadline"),
        )
        .expect("JSON");
        if predicate(&snapshot) {
            return snapshot;
        }
    }
}
#[test]
fn startup_independent_cas_ephemeral_capture_and_no_immediate_connect() {
    let root = std::env::temp_dir().join(format!("yonik-startup-settings-{}", std::process::id()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .expect("fixture");
    let config = root.join("config");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&config)
        .expect("fixture");
    let (tx, rx) = mpsc::channel();
    let host = DesktopHost::start(
        HostConfig::new(root.clone(), root.join("engines"))
            .with_ephemeral_key([19; 32])
            .with_startup_fixture_config(config.clone()),
        move |snapshot| {
            let _ = tx.send(snapshot);
        },
    )
    .expect("host");
    until(&rx, |s| s["ready"] == true);
    for (index, (a, c)) in [(false, false), (true, false), (false, true), (true, true)]
        .into_iter()
        .enumerate()
    {
        let request_id = (index + 1).to_string();
        host.dispatch(&json!({"command":"startup_preferences_set","request_id":request_id,"expected_revision":index.to_string(),"autostart_into_tray":a,"autoconnect":c}).to_string()).expect("dispatch");
        let snapshot = until(&rx, |s| {
            s["startup"]["request_id"].as_str() == Some(request_id.as_str())
        });
        assert_eq!(snapshot["startup"]["autostart_into_tray"], a);
        assert_eq!(snapshot["startup"]["autoconnect"], c);
        assert_eq!(snapshot["can_stop"], false);
    }
    host.dispatch(r#"{"command":"startup_preferences_set","request_id":"5","expected_revision":"0","autostart_into_tray":true,"autoconnect":false}"#).expect("dispatch");
    let stale = until(&rx, |s| s["startup"]["request_id"] == "5");
    assert_eq!(stale["startup"]["error"], "startup_conflict");
    host.dispatch(
        r#"{"command":"autoconnect_target_set","request_id":"6","expected_revision":"4"}"#,
    )
    .expect("dispatch");
    let capture = until(&rx, |s| s["startup"]["request_id"] == "6");
    assert_eq!(capture["startup"]["error"], "startup_binding");
    for bad in [json!(1), json!("01"), json!("18446744073709551616")] {
        assert!(host.dispatch(&json!({"command":"autoconnect_target_set","request_id":bad,"expected_revision":"4"}).to_string()).is_err());
    }
    host.shutdown();
    assert!(host.wait_closed(Duration::from_secs(10)));
    let (tx, rx) = mpsc::channel();
    let host = DesktopHost::start(
        HostConfig::new(root.clone(), root.join("engines"))
            .with_ephemeral_key([19; 32])
            .with_startup_fixture_config(config),
        move |snapshot| {
            let _ = tx.send(snapshot);
        },
    )
    .expect("restart");
    host.dispatch(r#"{"command":"stop"}"#)
        .expect("priority Stop");
    let restarted = until(&rx, |s| s["startup"]["attempt"] == "blocked");
    assert_eq!(restarted["can_stop"], false);
    assert_eq!(restarted["startup"]["autoconnect"], true);
    host.shutdown();
    assert!(host.wait_closed(Duration::from_secs(10)));
    fs::remove_dir_all(root).expect("fixture cleanup");
}
