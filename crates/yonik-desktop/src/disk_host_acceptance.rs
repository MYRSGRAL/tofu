//! Actual backend/control loop с остановленным disk consumer.
use super::{DesktopHost, DiskTestInput, HostConfig, ProxyLeaseFixture};
use serde_json::{Value, json};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use yonik_linux::event_log::{DiskFixture, DiskLogStatus, EventLog};

const DEADLINE: Duration = Duration::from_secs(10);
struct Case {
    host: DesktopHost,
    fixture: DiskFixture,
    snapshots: mpsc::Receiver<String>,
    log: EventLog,
}
impl Case {
    fn new() -> Self {
        Self::with_proxy(None)
    }
    fn with_proxy(proxy: Option<ProxyLeaseFixture>) -> Self {
        let mut fixture = DiskFixture::new().expect("own fixture");
        assert!(fixture.wait_opened(DEADLINE));
        let config = HostConfig::new(fixture.path().to_owned(), fixture.path().join("engines"))
            .with_ephemeral_key([71; 32]);
        let (tx, rx) = mpsc::channel();
        let callback = move |raw| {
            let _sent = tx.send(raw);
        };
        let host = if let Some(proxy) = proxy {
            let sink = fixture.take_sink().expect("own fixture sink");
            DesktopHost::start_inner(
                config,
                callback,
                Some(DiskTestInput {
                    sink,
                    proxy: Some(proxy),
                }),
            )
        } else {
            DesktopHost::test_start_with_disk(config, &mut fixture, callback)
        }
        .expect("actual backend");
        let (tx, log_rx) = mpsc::channel();
        let mut case = Self {
            host,
            fixture,
            snapshots: rx,
            log: EventLog::new(),
        };
        case.wait(|value| value["ready"] == true);
        case.host.test_event_log(tx).expect("actual actor log");
        case.log = log_rx
            .recv_timeout(DEADLINE)
            .expect("log handle")
            .expect("actor initialized");
        case.log.test_fill();
        assert_eq!(case.fixture.stats().written, 0);
        assert_eq!(case.fixture.stats().enqueued, 128);
        case
    }
    fn wait(&mut self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let raw = self
                .snapshots
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("actual snapshot deadline");
            let value: Value = serde_json::from_str(&raw).expect("actual JSON");
            if predicate(&value) {
                return value;
            }
        }
    }
    fn pending(&mut self) {
        self.host.shutdown();
        let deadline = Instant::now() + DEADLINE;
        while !self.fixture.shutdown_requested() {
            assert!(Instant::now() < deadline, "backend не достиг disk drain");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!self.fixture.wait_consumer_closed(Duration::ZERO));
        assert!(
            !self.host.wait_closed(Duration::from_millis(100)),
            "ранний control.closed ACK"
        );
        for raw in self.snapshots.try_iter() {
            let value: Value = serde_json::from_str(&raw).expect("snapshot");
            assert_ne!(value["closed"], true, "ранний snapshot.closed");
        }
        assert_eq!(self.fixture.stats().written, 0);
    }
    fn close(&mut self) {
        self.fixture.release();
        self.wait(|value| value["closed"] == true);
        assert!(self.host.wait_closed(DEADLINE), "actual control.closed ACK");
        assert!(self.fixture.wait_consumer_closed(Duration::ZERO));
        assert_eq!(self.fixture.stats().status, DiskLogStatus::Closed);
        assert_eq!(self.fixture.stats().written, self.fixture.stats().enqueued);
        let bytes =
            std::fs::read(self.fixture.path().join("logs/events.log")).expect("actual disk file");
        assert_eq!(
            bytes.iter().filter(|b| **b == b'\n').count() as u64,
            self.fixture.stats().written
        );
    }
}
impl Drop for Case {
    fn drop(&mut self) {
        // Сначала unblock, затем bounded host cleanup, затем owned root Drop.
        self.fixture.release();
        self.host.shutdown();
        let closed = self.host.wait_closed(DEADLINE);
        if !std::thread::panicking() {
            assert!(closed, "fixture host cleanup");
        }
    }
}
#[test]
fn stop_and_final_ack_wait_for_actual_disk_drain() {
    let mut case = Case::new();
    let start = Instant::now();
    case.host.dispatch(r#"{"command":"stop"}"#).expect("Stop");
    case.host.dispatch(&json!({"command":"routing_default", "expected_revision":0, "request_id":410, "action":"block"}).to_string()).expect("routing after Stop");
    let ack = case.wait(|value| value["routing_ack"]["request_id"] == 410);
    assert!(
        ack["routing_ack"]["error"].is_null(),
        "actual loop продолжает команды после Stop"
    );
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "Stop/control loop ждут диск"
    );
    assert_eq!(case.fixture.stats().written, 0);
    assert!(!case.fixture.shutdown_requested());
    case.pending();
    case.close();
}
#[test]
fn drop_releases_after_open_and_constructor_refuses_foreign_root() {
    let mut fixture = DiskFixture::new().expect("fixture");
    assert!(fixture.wait_opened(DEADLINE));
    let bad = HostConfig::new(
        fixture.path().join("foreign"),
        fixture.path().join("engines"),
    )
    .with_ephemeral_key([72; 32]);
    assert!(DesktopHost::test_start_with_disk(bad, &mut fixture, |_| {}).is_err());
    let (_log, guard) = fixture.take_sink().expect("refusal не забрал sink");
    let path = fixture.path().to_owned();
    drop(fixture);
    assert!(
        guard.wait_closed(Duration::ZERO),
        "Drop освободил actual consumer"
    );
    assert_eq!(guard.stats().status, DiskLogStatus::Closed);
    assert!(!path.exists(), "own root cleanup");
}

#[test]
fn stop_revokes_same_host_watch_lease_while_disk_is_blocked() {
    use super::{FetchError, FetchRoute, watch};
    use yonik_linux::{Source, SubscriptionFetch};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("fixture runtime");
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("owned loopback");
        let endpoint = listener.local_addr().expect("endpoint");
        let (observer, observed) = mpsc::channel();
        let mut case = Case::with_proxy(Some(ProxyLeaseFixture { endpoint, observer }));
        let (mut current, fetch) = observed
            .recv_timeout(DEADLINE)
            .expect("same production channel");
        assert_eq!(*current.borrow_and_update(), Some((17, endpoint)));
        assert_eq!(fetch.lease, Some((17, endpoint)));
        let (_cancel, cancelled) = watch::channel(false);
        let source = Source::local("http://127.0.0.1:1/sub").expect("typed source");
        let operation = fetch.fetch(&source, FetchRoute::Proxy, None, cancelled);
        tokio::pin!(operation);
        let (mut stream, _) = tokio::select! {
            accepted = listener.accept() => accepted.expect("SOCKS accept"),
            result = &mut operation => panic!("fetch завершился до Stop: {result:?}"),
            _ = tokio::time::sleep(DEADLINE) => panic!("loopback accept deadline"),
        };
        // Получаем greeting без ответа: операция действительно ожидает SOCKS I/O.
        let mut greeting = [0; 3];
        use tokio::io::AsyncReadExt;
        tokio::select! {
            read = stream.read_exact(&mut greeting) => { read.expect("SOCKS greeting"); },
            result = &mut operation => panic!("fetch завершился до greeting: {result:?}"),
            _ = tokio::time::sleep(DEADLINE) => panic!("greeting deadline"),
        }
        assert_eq!(greeting, [5, 1, 0]);
        assert_eq!(*current.borrow(), Some((17, endpoint)));
        let start = Instant::now();
        case.host
            .dispatch(r#"{"command":"stop"}"#)
            .expect("actual priority Stop");
        tokio::time::timeout(Duration::from_secs(2), current.changed())
            .await
            .expect("host revoke deadline")
            .expect("channel remains open");
        assert_eq!(*current.borrow_and_update(), None);
        let result = tokio::time::timeout(Duration::from_secs(2), &mut operation)
            .await
            .expect("pending fetch revoked");
        assert_eq!(
            result.expect_err("revoked operation"),
            FetchError::Cancelled
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(case.fixture.stats().written, 0);
        assert!(!case.fixture.shutdown_requested());
        case.pending();
        case.close();
    });
}

#[test]
fn dropping_sink_guard_also_releases_its_fixture_barrier() {
    let mut fixture = DiskFixture::new().expect("fixture");
    assert!(fixture.wait_opened(DEADLINE));
    let (_log, guard) = fixture.take_sink().expect("own sink");
    drop(guard);
    assert!(
        fixture.wait_consumer_closed(DEADLINE),
        "DiskLogGuard Drop должен unblock consumer"
    );
    assert_eq!(fixture.stats().status, DiskLogStatus::Closed);
    let path = fixture.path().to_owned();
    drop(fixture);
    assert!(!path.exists());
}
