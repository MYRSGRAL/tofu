//! Только приёмка: свежий State с bounded повтором исключительно typed Busy.
use crate::{
    Cancellation,
    helper::service::{SERVICE_INTERFACE, SERVICE_NAME, SERVICE_PATH},
};
use std::time::Duration;

const MAX_STATE_FRAME_BYTES: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum WaitError {
    Rpc,
    Protocol,
    Deadline,
    Attempts,
    Cancelled,
    State,
}
#[derive(Clone, Copy)]
pub(super) struct Policy {
    pub total: Duration,
    pub attempts: u32,
    pub delay: Duration,
    pub max_delay: Duration,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            total: Duration::from_secs(10),
            attempts: 32,
            delay: Duration::from_millis(25),
            max_delay: Duration::from_millis(400),
        }
    }
}
#[derive(Debug)]
pub(super) struct Evidence {
    pub state: String,
    pub busy: u32,
    pub calls: u32,
    pub elapsed: Duration,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorFrame {
    version: u32,
    error: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StateFrame {
    version: u32,
    state: String,
}
fn typed_busy(error: &zbus::Error) -> Result<bool, WaitError> {
    let zbus::Error::MethodError(name, _, reply) = error else {
        return Ok(false);
    };
    if name.as_str() != "cloud.highpriority.yonik.Network1.Error" {
        return Ok(false);
    }
    let (json,): (String,) = reply
        .body()
        .deserialize()
        .map_err(|_| WaitError::Protocol)?;
    if json.len() > MAX_STATE_FRAME_BYTES {
        return Err(WaitError::Protocol);
    }
    let frame: ErrorFrame = serde_json::from_str(&json).map_err(|_| WaitError::Protocol)?;
    if frame.version != 1 {
        return Err(WaitError::Protocol);
    }
    Ok(frame.error == "busy")
}
pub(super) async fn wait_for_tunnel_state(
    peer: &zbus::Connection,
    cancel: &mut Cancellation,
    policy: Policy,
) -> Result<Evidence, WaitError> {
    let started = tokio::time::Instant::now();
    let deadline = started + policy.total;
    let mut calls = 0;
    let mut busy = 0;
    let mut delay = policy.delay;
    let result=async {
        if policy.total.is_zero() || policy.attempts==0 || delay.is_zero() || delay>policy.max_delay { return Err(WaitError::Protocol); }
        loop {
            cancel.check().map_err(|_|WaitError::Cancelled)?;
            if tokio::time::Instant::now()>=deadline { return Err(WaitError::Deadline); }
            calls+=1;
            let reply=tokio::select! {
                biased;
                _=cancel.cancelled()=>return Err(WaitError::Cancelled),
                result=tokio::time::timeout_at(deadline,peer.call_method(Some(SERVICE_NAME),SERVICE_PATH,Some(SERVICE_INTERFACE),"State",&(r#"{"version":1}"#,)))=>result.map_err(|_|WaitError::Deadline)?,
            };
            match reply {
                Ok(reply)=>{
                    let (json,):(String,)=reply.body().deserialize().map_err(|_|WaitError::Protocol)?;
                    if json.len()>MAX_STATE_FRAME_BYTES { return Err(WaitError::Protocol); }
                    let frame:StateFrame=serde_json::from_str(&json).map_err(|_|WaitError::Protocol)?;
                    if frame.version!=1 { return Err(WaitError::Protocol); }
                    if frame.state!="tunnel_starting" { return Err(WaitError::State); }
                    cancel.check().map_err(|_|WaitError::Cancelled)?;
                    if tokio::time::Instant::now()>=deadline { return Err(WaitError::Deadline); }
                    return Ok(frame.state);
                },
                Err(error)=>{
                    if !typed_busy(&error)? { return Err(WaitError::Rpc); }
                    busy+=1;
                }
            }
            if calls>=policy.attempts { return Err(WaitError::Attempts); }
            let next=(tokio::time::Instant::now()+delay).min(deadline);
            tokio::select! {
                biased;
                _=cancel.cancelled()=>return Err(WaitError::Cancelled),
                _=tokio::time::sleep_until(next)=>{},
            }
            delay=delay.saturating_mul(2).min(policy.max_delay);
        }
    }.await;
    let elapsed = started.elapsed();
    eprintln!(
        "State acceptance: busy={busy} calls={calls} elapsed_ms={} result={:?}",
        elapsed.as_millis(),
        result
    );
    result.map(|state| Evidence {
        state,
        busy,
        calls,
        elapsed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use std::{
        collections::VecDeque,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    enum Reply {
        Busy,
        Success,
        SuccessBody(&'static str),
        Error(&'static str, &'static str),
        Hold,
    }
    struct BusFixture {
        _bus: super::super::super::PrivateBus,
        peer: zbus::Connection,
        task: tokio::task::JoinHandle<()>,
        calls: Arc<AtomicUsize>,
        entered: Arc<tokio::sync::Notify>,
    }
    impl Drop for BusFixture {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    async fn fixture(replies: Vec<Reply>) -> BusFixture {
        let bus = super::super::super::PrivateBus::start();
        let server = bus.connect().await;
        server.request_name(SERVICE_NAME).await.unwrap();
        let peer = bus.connect().await;
        let mut stream = zbus::MessageStream::from(&server);
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let entered = Arc::new(tokio::sync::Notify::new());
        let notified = entered.clone();
        let mut replies = VecDeque::from(replies);
        let task = tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                if message.header().message_type() != zbus::message::Type::MethodCall
                    || message.header().member().map(|v| v.as_str()) != Some("State")
                {
                    continue;
                }
                counted.fetch_add(1, Ordering::SeqCst);
                notified.notify_one();
                match replies.pop_front().unwrap_or(Reply::Busy) {
                    Reply::Busy => {
                        server
                            .reply_error(
                                &message,
                                "cloud.highpriority.yonik.Network1.Error",
                                &(r#"{"version":1,"error":"busy"}"#,),
                            )
                            .await
                            .unwrap();
                    }
                    Reply::Success => {
                        server
                            .reply(&message, &(r#"{"version":1,"state":"tunnel_starting"}"#,))
                            .await
                            .unwrap();
                    }
                    Reply::SuccessBody(body) => {
                        server.reply(&message, &(body,)).await.unwrap();
                    }
                    Reply::Error(name, body) => {
                        server.reply_error(&message, name, &(body,)).await.unwrap();
                    }
                    Reply::Hold => std::future::pending::<()>().await,
                }
            }
        });
        BusFixture {
            _bus: bus,
            peer,
            task,
            calls,
            entered,
        }
    }
    fn short() -> Policy {
        Policy {
            total: Duration::from_millis(200),
            attempts: 4,
            delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(4),
        }
    }
    #[tokio::test]
    async fn real_private_bus_busy_then_fresh_state() {
        let f = fixture(vec![Reply::Busy, Reply::Success]).await;
        let (_keep, mut cancel) = Cancellation::new();
        let result = wait_for_tunnel_state(&f.peer, &mut cancel, short())
            .await
            .unwrap();
        assert_eq!(result.state, "tunnel_starting");
        assert_eq!(result.busy, 1);
        assert_eq!(result.calls, 2);
        assert_eq!(f.calls.load(Ordering::SeqCst), 2);
    }
    #[tokio::test]
    async fn real_private_bus_constant_busy_is_bounded_by_attempts_and_one_deadline() {
        let f = fixture(vec![]).await;
        let (_keep, mut cancel) = Cancellation::new();
        let error = wait_for_tunnel_state(&f.peer, &mut cancel, short())
            .await
            .unwrap_err();
        assert_eq!(error, WaitError::Attempts);
        assert_eq!(f.calls.load(Ordering::SeqCst), 4);
        let f = fixture(vec![]).await;
        let (_keep, mut cancel) = Cancellation::new();
        let policy = Policy {
            total: Duration::from_millis(15),
            attempts: 100,
            ..short()
        };
        assert_eq!(
            wait_for_tunnel_state(&f.peer, &mut cancel, policy)
                .await
                .unwrap_err(),
            WaitError::Deadline
        );
        assert!(f.calls.load(Ordering::SeqCst) < 100);
    }
    #[tokio::test]
    async fn real_private_bus_non_busy_and_malformed_errors_never_retry() {
        for (name, body) in [
            (
                "cloud.highpriority.yonik.Network1.Error",
                r#"{"version":1,"error":"authorization"}"#,
            ),
            (
                "cloud.highpriority.yonik.Network1.Error",
                r#"{"version":1,"error":"stale"}"#,
            ),
            (
                "cloud.highpriority.yonik.Network1.Error",
                r#"{"version":1,"error":"unavailable"}"#,
            ),
            (
                "cloud.highpriority.yonik.Network1.Error",
                r#"{"version":1,"error":"timeout"}"#,
            ),
            (
                "cloud.highpriority.yonik.Network1.Error",
                r#"{"version":1,"error":"unknown"}"#,
            ),
            (
                "cloud.highpriority.yonik.Network1.Error",
                r#"{"version":2,"error":"busy"}"#,
            ),
            (
                "cloud.highpriority.yonik.Network1.Error",
                r#"{"version":1,"error":"busy","extra":true}"#,
            ),
            (
                "cloud.highpriority.yonik.Network1.Error",
                r#"{"version":1,"error":"busy","error":"busy"}"#,
            ),
            ("cloud.highpriority.yonik.Network1.Error", "busy"),
            (
                "org.freedesktop.DBus.Error.Failed",
                r#"{"version":1,"error":"busy"}"#,
            ),
        ] {
            let f = fixture(vec![Reply::Error(name, body), Reply::Success]).await;
            let (_keep, mut cancel) = Cancellation::new();
            assert!(
                wait_for_tunnel_state(&f.peer, &mut cancel, short())
                    .await
                    .is_err()
            );
            assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        }
    }
    #[tokio::test]
    async fn real_private_bus_success_must_be_fresh_valid_tunnel_state() {
        for body in [
            r#"{"version":2,"state":"tunnel_starting"}"#,
            r#"{"version":1,"error":"busy"}"#,
            r#"{"version":1,"state":"unknown"}"#,
            r#"{"version":1,"state":"tunnel_starting","extra":1}"#,
            "not json",
        ] {
            let f = fixture(vec![Reply::SuccessBody(body), Reply::Success]).await;
            let (_keep, mut cancel) = Cancellation::new();
            assert!(
                wait_for_tunnel_state(&f.peer, &mut cancel, short())
                    .await
                    .is_err()
            );
            assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        }
    }
    #[tokio::test]
    async fn real_private_bus_hung_call_times_out_and_cancels_without_retry() {
        let f = fixture(vec![Reply::Hold]).await;
        let (_keep, mut cancel) = Cancellation::new();
        assert_eq!(
            wait_for_tunnel_state(&f.peer, &mut cancel, short())
                .await
                .unwrap_err(),
            WaitError::Deadline
        );
        assert!(f.calls.load(Ordering::SeqCst) <= 1);
        let f = fixture(vec![Reply::Hold]).await;
        let (signal, mut cancel) = Cancellation::new();
        let request = wait_for_tunnel_state(&f.peer, &mut cancel, short());
        let cancellation = async {
            f.entered.notified().await;
            signal.send_replace(true);
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(request, cancellation)
        })
        .await
        .unwrap();
        assert_eq!(result.unwrap_err(), WaitError::Cancelled);
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn real_private_bus_transport_loss_is_not_retried() {
        let mut f = fixture(vec![Reply::Hold]).await;
        let (_keep, mut cancel) = Cancellation::new();
        let entered = f.entered.clone();
        let daemon = &mut f._bus.child;
        let disconnect = async {
            entered.notified().await;
            daemon.kill().unwrap();
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(
                wait_for_tunnel_state(&f.peer, &mut cancel, short()),
                disconnect
            )
        })
        .await
        .unwrap();
        assert_eq!(result.unwrap_err(), WaitError::Rpc);
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    }
}
