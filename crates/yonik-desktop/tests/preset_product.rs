//! Проверка responsive actor, одного задания и matching ACK без настоящей сети.
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    net::TcpListener,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    sync::mpsc,
    time::{Duration, Instant},
};
use yonik_desktop::{DesktopHost, HostConfig};

#[test]
fn single_job_cancel_and_stop_join_before_terminal_ack() {
    for stop in [false, true] {
        let mut random = [0; 16];
        getrandom::getrandom(&mut random).expect("путь");
        let root = std::env::temp_dir().join(format!("yonik-preset-product-{random:x?}"));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .expect("fixture");
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let (entered, observed) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .expect("timeout");
            let mut buffer = [0; 4096];
            let mut received: usize = 0;
            loop {
                let start = received.saturating_sub(3);
                let count = stream.read(&mut buffer[received..]).expect("request");
                assert!(count > 0);
                received += count;
                if buffer[start..received]
                    .windows(4)
                    .any(|bytes| bytes == b"\r\n\r\n")
                {
                    break;
                }
                assert!(received < buffer.len(), "bounded request headers");
            }
            entered.send(()).expect("entered");
            assert_eq!(
                stream
                    .read(&mut buffer)
                    .expect("connection closed after cancel"),
                0
            );
        });
        let (sender, snapshots) = mpsc::channel();
        let host = DesktopHost::start(
            HostConfig::new(root.clone(), root.join("engines")).with_ephemeral_key([47; 32]),
            move |snapshot| {
                let _ = sender.send(snapshot);
            },
        )
        .expect("host");
        let wait = |predicate: &dyn Fn(&Value) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let raw = snapshots
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .expect("snapshot deadline");
                let value: Value = serde_json::from_str(&raw).expect("DTO");
                if predicate(&value) {
                    break value;
                }
            }
        };
        wait(&|v| v["ready"] == true);
        let resource = |hash: &str| json!({"url":format!("http://{address}/list"),"revision":"v1","sha256":hash});
        let update = |request_id| {
            json!({"command":"preset_update","request_id":request_id,"route":"direct","local_exception":true,"metadata":{
            "source_id":"fixture","version":"v1","format":"plain-domain-cidr-v1","checked_at_unix":1,"enabled":false,
            "domains":resource("04e512879df7acb18392404d982a1b179e94c20b53ee03b75538624863d89e96"),
            "ips":resource("1056040935d29c506274717d77a7388683810fd7ae430176f81a59de1270c994")}})
        };
        host.dispatch(&update(1).to_string()).expect("update");
        observed
            .recv_timeout(Duration::from_secs(10))
            .expect("fetch entered");
        host.dispatch(
            &json!({"command":"maintenance_start","request_id":81,
            "operation":"backup","destination":root.join("backup.sqlite"),"revision":0})
            .to_string(),
        )
        .expect("maintenance admission");
        let refused = wait(&|v| v["maintenance_ack"]["request_id"] == 81);
        assert_eq!(refused["maintenance_ack"]["error"], "busy");
        assert_eq!(refused["ready"], true);
        host.dispatch(&update(2).to_string())
            .expect("second update");
        let busy = wait(&|v| v["presets"]["ack"]["request_id"] == 2);
        assert_eq!(busy["presets"]["ack"]["error"], "busy");
        host.dispatch(r#"{"command":"set_language","language":"en"}"#)
            .expect("responsive command");
        wait(&|v| v["language"] == "en");
        if stop {
            host.dispatch(r#"{"command":"stop"}"#).expect("stop");
        } else {
            host.dispatch(r#"{"command":"preset_cancel","request_id":1}"#)
                .expect("cancel");
        }
        let ack = wait(&|v| v["presets"]["ack"]["request_id"] == 1);
        assert_eq!(ack["presets"]["ack"]["error"], "cancelled");
        assert_eq!(ack["presets"]["busy"], false);
        assert!(ack["presets"]["current"].is_null());
        server.join().expect("server joined before terminal");
        host.shutdown();
        assert!(host.wait_closed(Duration::from_secs(10)));
        drop(host);
        fs::remove_dir_all(root).expect("fixture");
    }
}

#[test]
fn unavailable_controller_update_returns_matching_storage_ack() {
    let mut id = [0; 16];
    getrandom::getrandom(&mut id).expect("path");
    let root = std::env::temp_dir().join(format!("yonik-unavailable-preset-{id:x?}"));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .expect("root");
    fs::DirBuilder::new()
        .mode(0o755)
        .create(root.join("presets"))
        .expect("unavailable catalog");
    fs::set_permissions(root.join("presets"), fs::Permissions::from_mode(0o755))
        .expect("exact unsafe fixture mode");
    let (sender, snapshots) = mpsc::channel();
    let host = DesktopHost::start(
        HostConfig::new(root.clone(), root.join("engines")).with_ephemeral_key([48; 32]),
        move |value| {
            let _ = sender.send(value);
        },
    )
    .expect("host");
    let ready_deadline = Instant::now() + Duration::from_secs(10);
    while !serde_json::from_str::<Value>(
        &snapshots
            .recv_timeout(ready_deadline.saturating_duration_since(Instant::now()))
            .expect("ready"),
    )
    .expect("DTO")["ready"]
        .as_bool()
        .unwrap_or(false)
    {}
    let resource =
        json!({"url":"https://example.org/list","revision":"v1","sha256":"a".repeat(64)});
    host.dispatch(&json!({"command":"preset_update","request_id":71,"route":"proxy","local_exception":false,"metadata":{"source_id":"fixture","version":"v1","format":"plain-domain-cidr-v1","checked_at_unix":1,"enabled":false,"domains":resource,"ips":resource}}).to_string()).expect("update");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut matching = false;
    while let Ok(raw) = snapshots.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        let value: Value = serde_json::from_str(&raw).expect("DTO");
        if value["presets"]["ack"]["request_id"] == 71 {
            matching = value["presets"]["ack"]["error"] == "storage";
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    host.shutdown();
    assert!(host.wait_closed(Duration::from_secs(10)));
    drop(host);
    fs::remove_dir_all(root).expect("fixture");
    assert!(matching, "нет matching storage ACK");
}
