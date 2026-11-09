use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn pair() -> (
    ResolverWire,
    UnixStream,
    tokio::sync::watch::Sender<bool>,
    Cancellation,
) {
    let (client, server) = UnixStream::pair().expect("локальная пара сокетов");
    let (sender, cancel) = Cancellation::new();
    (ResolverWire::new(client), server, sender, cancel)
}

const SNAPSHOT: &[u8] = b"{\"parameters\":{\"configuration\":[]}}\0";
const UPDATE: &[u8] = b"{\"continues\":true,\"parameters\":{\"configuration\":[]}}\0";

async fn assert_closed(server: &mut UnixStream) {
    let mut data = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(1), server.read_to_end(&mut data))
        .await
        .expect("fd должен закрыться");
    // Закрытие сокета с непрочитанной частью кадра может вернуть ECONNRESET.
    assert!(
        result.is_ok()
            || result.is_err_and(|error| error.kind() == std::io::ErrorKind::ConnectionReset)
    );
}

#[tokio::test]
async fn split_bytewise_snapshot_and_fixed_request() {
    let (mut wire, mut server, _sender, mut cancel) = pair();
    let fake = async {
        let mut request = Vec::new();
        loop {
            let byte = server.read_u8().await.expect("запрос");
            if byte == 0 {
                break;
            }
            request.push(byte);
        }
        assert_eq!(
            serde_json::from_slice::<Value>(&request).expect("JSON запроса"),
            serde_json::json!({"method":"io.systemd.Resolve.DumpDNSConfiguration","parameters":{}})
        );
        for byte in SNAPSHOT {
            server.write_all(&[*byte]).await.expect("ответ");
            tokio::task::yield_now().await;
        }
        assert_closed(&mut server).await;
    };
    let (result, ()) = tokio::join!(
        wire.snapshot(&mut cancel, Instant::now() + Duration::from_secs(2)),
        fake
    );
    assert!(result.is_ok());
    assert!(wire.is_closed());
}

#[tokio::test]
async fn subscription_preserves_coalesced_frames() {
    let (mut wire, mut server, _sender, mut cancel) = pair();
    server
        .write_all(&[UPDATE, UPDATE].concat())
        .await
        .expect("ответы");
    assert!(
        wire.subscribe(&mut cancel, Instant::now() + Duration::from_secs(1))
            .await
            .is_ok()
    );
    assert!(wire.next(&mut cancel).await.is_ok());
    let mut request = vec![0; 512];
    let n = server.read(&mut request).await.expect("запрос");
    assert_eq!(
        serde_json::from_slice::<Value>(&request[..n - 1]).expect("JSON"),
        serde_json::json!({
            "method":"io.systemd.Resolve.Monitor.SubscribeDNSConfiguration",
            "parameters":{"allowInteractiveAuthentication":false},"more":true
        })
    );
}

#[tokio::test]
async fn strict_json_rejects_nested_and_top_level_duplicates() {
    for frame in [
        br#"{"parameters":{"configuration":[]},"parameters":{"configuration":[]}}"#.as_slice(),
        br#"{"parameters":{"configuration":[],"configuration":[]}}"#.as_slice(),
        br#"{"parameters":{"configuration":[]},"extra":{"x":1,"x":2}}"#.as_slice(),
    ] {
        let (mut wire, mut server, _sender, mut cancel) = pair();
        server
            .write_all(&[frame, &[0]].concat())
            .await
            .expect("ответ");
        assert_eq!(
            wire.snapshot(&mut cancel, Instant::now() + Duration::from_secs(1))
                .await
                .err(),
            Some(ResolverError::Malformed)
        );
        assert_closed(&mut server).await;
    }
}

#[tokio::test]
async fn errors_are_classified_and_close_fd() {
    for (error, expected) in [
        (
            "org.varlink.service.UnknownMethod",
            ResolverError::Unsupported,
        ),
        (
            "org.varlink.service.UnknownInterface",
            ResolverError::Unsupported,
        ),
        (
            "io.systemd.InteractiveAuthenticationRequired",
            ResolverError::PermissionDenied,
        ),
        ("io.systemd.NotAuthorized", ResolverError::PermissionDenied),
        (
            "io.systemd.PermissionDenied",
            ResolverError::PermissionDenied,
        ),
    ] {
        let (mut wire, mut server, _sender, mut cancel) = pair();
        server
            .write_all(format!("{{\"error\":\"{error}\"}}\0").as_bytes())
            .await
            .expect("ошибка");
        assert_eq!(
            wire.snapshot(&mut cancel, Instant::now() + Duration::from_secs(1))
                .await
                .err(),
            Some(expected)
        );
        assert_closed(&mut server).await;
    }
}

#[tokio::test]
async fn eof_and_trailing_partial_are_not_successful_frames() {
    for bytes in [b"".as_slice(), b"{\"parameters\":", b"\0"] {
        let (mut wire, mut server, _sender, mut cancel) = pair();
        server.write_all(bytes).await.expect("частичный ответ");
        server.shutdown().await.expect("EOF");
        let expected = if bytes == b"\0" {
            ResolverError::Malformed
        } else {
            ResolverError::Closed
        };
        assert_eq!(
            wire.snapshot(&mut cancel, Instant::now() + Duration::from_secs(1))
                .await
                .err(),
            Some(expected)
        );
        assert!(wire.is_closed());
    }
    let (mut wire, mut server, _sender, mut cancel) = pair();
    server
        .write_all(&[UPDATE, b"{"].concat())
        .await
        .expect("ответ");
    server.shutdown().await.expect("EOF");
    assert!(
        wire.subscribe(&mut cancel, Instant::now() + Duration::from_secs(1))
            .await
            .is_ok()
    );
    assert_eq!(
        wire.next(&mut cancel).await.err(),
        Some(ResolverError::Closed)
    );
    assert!(wire.is_closed());
}

#[tokio::test]
async fn subscription_requires_continuing_full_configuration() {
    for frame in [
        SNAPSHOT,
        b"{\"continues\":false,\"parameters\":{\"configuration\":[]}}\0",
        b"{\"continues\":true,\"parameters\":{}}\0",
    ] {
        let (mut wire, mut server, _sender, mut cancel) = pair();
        server.write_all(frame).await.expect("ответ");
        assert_eq!(
            wire.subscribe(&mut cancel, Instant::now() + Duration::from_secs(1))
                .await
                .err(),
            Some(ResolverError::Malformed)
        );
        assert_closed(&mut server).await;
    }
}

#[tokio::test]
async fn invalid_operation_order_closes_stream() {
    let (mut wire, mut server, _sender, mut cancel) = pair();
    assert_eq!(
        wire.next(&mut cancel).await.err(),
        Some(ResolverError::Closed)
    );
    assert_closed(&mut server).await;
    let (mut wire, mut server, _sender, mut cancel) = pair();
    server.write_all(SNAPSHOT).await.expect("ответ");
    assert!(
        wire.snapshot(&mut cancel, Instant::now() + Duration::from_secs(1))
            .await
            .is_ok()
    );
    assert_eq!(
        wire.snapshot(&mut cancel, Instant::now() + Duration::from_secs(1))
            .await
            .err(),
        Some(ResolverError::Closed)
    );
}

#[tokio::test]
async fn cancellation_during_idle_and_partial_frame_closes_fd() {
    for partial in [false, true] {
        let (mut wire, mut server, sender, mut cancel) = pair();
        server.write_all(UPDATE).await.expect("ответ");
        assert!(
            wire.subscribe(&mut cancel, Instant::now() + Duration::from_secs(1))
                .await
                .is_ok()
        );
        if partial {
            server.write_all(b"{").await.expect("часть");
        }
        let stop = async {
            tokio::task::yield_now().await;
            sender.send(true).expect("отмена");
        };
        let (result, ()) = tokio::join!(wire.next(&mut cancel), stop);
        assert_eq!(result.err(), Some(ResolverError::Cancelled));
        assert_closed(&mut server).await;
    }
}

#[tokio::test]
async fn startup_deadline_and_oversize_close_fd() {
    let (mut wire, mut server, _sender, mut cancel) = pair();
    assert_eq!(
        wire.snapshot(&mut cancel, Instant::now() + Duration::from_millis(10))
            .await
            .err(),
        Some(ResolverError::Timeout)
    );
    assert_closed(&mut server).await;
    let (mut wire, mut server, _sender, mut cancel) = pair();
    let send = async {
        let _ = server.write_all(&vec![b' '; MAX_FRAME + 1]).await;
    };
    let (result, ()) = tokio::join!(
        wire.snapshot(&mut cancel, Instant::now() + Duration::from_secs(2)),
        send
    );
    assert_eq!(result.err(), Some(ResolverError::Limit));
    assert!(wire.is_closed());
}

#[tokio::test]
async fn snapshot_rejects_continuation_and_deep_json() {
    let deep = format!(
        "{{\"parameters\":{{\"configuration\":[]}},\"extra\":{}0{}}}\0",
        "[".repeat(129),
        "]".repeat(129)
    );
    for frame in [UPDATE, deep.as_bytes()] {
        let (mut wire, mut server, _sender, mut cancel) = pair();
        server.write_all(frame).await.expect("ответ");
        assert_eq!(
            wire.snapshot(&mut cancel, Instant::now() + Duration::from_secs(1))
                .await
                .err(),
            Some(ResolverError::Malformed)
        );
        assert_closed(&mut server).await;
    }
}

#[tokio::test]
async fn maximum_payload_is_accepted() {
    let (mut wire, mut server, _sender, mut cancel) = pair();
    let send = async {
        let mut frame = SNAPSHOT[..SNAPSHOT.len() - 1].to_vec();
        frame.resize(MAX_FRAME, b' ');
        frame.push(0);
        server.write_all(&frame).await.expect("максимальный frame");
    };
    let (result, ()) = tokio::join!(
        wire.snapshot(&mut cancel, Instant::now() + Duration::from_secs(2)),
        send
    );
    assert!(result.is_ok());
}

#[tokio::test]
async fn partial_frame_uses_absolute_five_second_deadline() {
    let (mut wire, mut server, _sender, mut cancel) = pair();
    server.write_all(UPDATE).await.expect("ответ");
    assert!(
        wire.subscribe(&mut cancel, Instant::now() + Duration::from_secs(1))
            .await
            .is_ok()
    );
    server.write_all(b"{").await.expect("часть");
    let started = Instant::now();
    let drip = async {
        tokio::time::sleep(Duration::from_secs(3)).await;
        server.write_all(b" ").await.expect("продолжение");
    };
    let (result, ()) = tokio::join!(wire.next(&mut cancel), drip);
    assert_eq!(result.err(), Some(ResolverError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(7));
    assert_closed(&mut server).await;
}

#[tokio::test]
async fn subscription_idle_has_no_five_second_timeout() {
    let (mut wire, mut server, _sender, mut cancel) = pair();
    server.write_all(UPDATE).await.expect("ответ");
    assert!(
        wire.subscribe(&mut cancel, Instant::now() + Duration::from_secs(1))
            .await
            .is_ok()
    );
    let send = async {
        tokio::time::sleep(Duration::from_millis(5100)).await;
        server
            .write_all(UPDATE)
            .await
            .expect("обновление после idle");
    };
    let (result, ()) = tokio::join!(wire.next(&mut cancel), send);
    assert!(result.is_ok());
}

#[tokio::test]
async fn already_visible_half_close_is_not_an_active_subscription() {
    let (mut wire, mut server, _sender, mut cancel) = pair();
    server.write_all(UPDATE).await.expect("ответ");
    server.shutdown().await.expect("half-close");
    assert_eq!(
        wire.subscribe(&mut cancel, Instant::now() + Duration::from_secs(1))
            .await
            .err(),
        Some(ResolverError::Closed)
    );
    assert!(wire.is_closed());
}

#[tokio::test]
async fn final_snapshot_accepts_half_close() {
    let (mut wire, mut server, _sender, mut cancel) = pair();
    server.write_all(SNAPSHOT).await.expect("ответ");
    server.shutdown().await.expect("half-close");
    assert!(
        wire.snapshot(&mut cancel, Instant::now() + Duration::from_secs(1))
            .await
            .is_ok()
    );
    assert!(wire.is_closed());
}

#[tokio::test]
async fn coalesced_partial_tail_has_its_own_deadline() {
    let (mut wire, mut server, _sender, mut cancel) = pair();
    // A уже принимается четыре секунды; последний read принесёт также начало B.
    let split = UPDATE.len() / 2;
    wire.state = State::Following;
    wire.buffer.extend_from_slice(&UPDATE[..split]);
    wire.partial_since = Some(Instant::now() - Duration::from_secs(4));
    server
        .write_all(&[&UPDATE[split..], &UPDATE[..split]].concat())
        .await
        .expect("конец A и начало B");
    assert!(wire.next(&mut cancel).await.is_ok());
    tokio::time::sleep(Duration::from_millis(1100)).await;
    server.write_all(&UPDATE[split..]).await.expect("конец B");
    assert!(wire.next(&mut cancel).await.is_ok());
}

#[tokio::test]
async fn complete_coalesced_frame_survives_delayed_consumer() {
    let (mut wire, mut server, _sender, mut cancel) = pair();
    server
        .write_all(&[UPDATE, UPDATE].concat())
        .await
        .expect("полные сообщения");
    assert!(
        wire.subscribe(&mut cancel, Instant::now() + Duration::from_secs(1))
            .await
            .is_ok()
    );
    assert_eq!(wire.buffer, UPDATE);
    tokio::time::sleep(Duration::from_millis(5100)).await;
    assert!(wire.next(&mut cancel).await.is_ok());
}
