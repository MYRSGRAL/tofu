//! Проверка responsive actor, одного задания и matching ACK без настоящей сети.
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    net::TcpListener,
    os::unix::fs::DirBuilderExt,
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
