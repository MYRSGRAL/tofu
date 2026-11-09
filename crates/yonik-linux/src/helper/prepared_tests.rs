use super::super::{AUTHORITY, MAX_CHECKS, PeerAuthorizer};
use super::*;
use crate::Cancellation;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::Duration};
use tokio::sync::{Semaphore, watch};
use zbus::{Connection, Message, MessageStream, message::Type, zvariant::OwnedValue};

type Subject = (String, HashMap<String, OwnedValue>);
type Check = (Subject, String, HashMap<String, String>, u32, String);

struct PrivateBus {
    child: std::process::Child,
    address: String,
}
impl PrivateBus {
    fn start() -> Self {
        use std::io::BufRead;
        let mut child = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self {
            child,
            address: address.trim().to_owned(),
        }
    }
    async fn connect(&self) -> Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap()
    }
}
impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Fixture {
    bus: PrivateBus,
    helper: PeerAuthorizer,
    peer: Connection,
    authority: Connection,
    stream: MessageStream,
    incoming: Message,
}
impl Fixture {
    async fn new() -> Self {
        let bus = PrivateBus::start();
        let server = bus.connect().await;
        let peer = bus.connect().await;
        let authority = bus.connect().await;
        authority.request_name(AUTHORITY).await.unwrap();
        let stream = MessageStream::from(&authority);
        let mut server_stream = MessageStream::from(&server);
        let call = Message::method("/cloud/highpriority/yonik", "Prepare")
            .unwrap()
            .destination(server.unique_name().unwrap())
            .unwrap()
            .interface("cloud.highpriority.yonik.NetworkHelper1")
            .unwrap()
            .build(&(999_999u32, "untrusted.client.message", "/untrusted/path"))
            .unwrap();
        peer.send(&call).await.unwrap();
        let incoming = next_call(&mut server_stream, "Prepare").await;
        let helper = PeerAuthorizer {
            connection: server,
            expected_authority_uid: rustix::process::getuid().as_raw(),
            checks: Semaphore::new(MAX_CHECKS),
            noninteractive_timeout: Duration::from_secs(5),
            interactive_timeout: Duration::from_secs(5),
        };
        Self {
            bus,
            helper,
            peer,
            authority,
            stream,
            incoming,
        }
    }
}

async fn next_call(stream: &mut MessageStream, member: &str) -> Message {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = stream.next().await.unwrap().unwrap();
            if message.header().message_type() == Type::MethodCall
                && message.header().member().map(|name| name.as_str()) == Some(member)
            {
                return message;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("Нет D-Bus вызова {member}"))
}
async fn reply(connection: &Connection, call: &Message, allowed: bool, challenge: bool) {
    connection
        .reply(call, &((allowed, challenge, HashMap::<&str, &str>::new()),))
        .await
        .unwrap();
}
fn cancellation() -> (watch::Sender<bool>, Cancellation) {
    Cancellation::new()
}

#[derive(Debug)]
struct Artifact(std::sync::Arc<Mutex<Vec<u8>>>);
impl PreparedArtifact for Artifact {
    fn current_digest(&self) -> ConfigDigest {
        ConfigDigest::new(Sha256::digest(&*self.0.lock().unwrap()).into())
    }
    fn byte_len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}
fn context() -> PreparedContext {
    PreparedContext::new(
        NonZeroU64::new(1).unwrap(),
        NonZeroU64::new(1).unwrap(),
        EnvironmentStamp {
            network: 1,
            applications: 1,
            authorization: 1,
            engine: 1,
            authorized: true,
        },
    )
    .unwrap()
}
fn artifact() -> Artifact {
    Artifact(std::sync::Arc::new(Mutex::new(vec![42])))
}
async fn auth(stream: &mut MessageStream, authority: &Connection, allowed: bool, challenge: bool) {
    let call = next_call(stream, "CheckAuthorization").await;
    let (_, action, details, _, _): Check = call.body().deserialize().unwrap();
    assert_eq!(action, NetworkAction::ManageNetwork.id());
    assert!(details.is_empty());
    reply(authority, &call, allowed, challenge).await;
}
async fn prepared(
    store: &PreparedRequests<Artifact>,
    message: &Message,
    stream: &mut MessageStream,
    authority: &Connection,
) -> PreparedHandle {
    let (_owner, cancel) = cancellation();
    let (result, ()) = tokio::join!(
        store.prepare(message, Prompt::Never, cancel, || async {
            Ok((artifact(), context()))
        }),
        auth(stream, authority, true, false)
    );
    result.unwrap()
}
async fn claimed(
    store: &PreparedRequests<Artifact>,
    message: &Message,
    handle: PreparedHandle,
    stream: &mut MessageStream,
    authority: &Connection,
) -> Result<Artifact, PreparedError> {
    let (_owner, cancel) = cancellation();
    let (result, ()) = tokio::join!(
        store.claim_for_apply(message, handle, Prompt::Never, cancel, || Ok(context())),
        auth(stream, authority, true, false)
    );
    result
}
#[tokio::test]
async fn fresh_auth_revoke_challenge_replay_and_wrong_unique_peer() {
    let Fixture {
        bus,
        helper,
        peer: _peer,
        authority,
        mut stream,
        incoming,
    } = Fixture::new().await;
    let store = PreparedRequests::with_authorizer(helper);
    let handle = prepared(&store, &incoming, &mut stream, &authority).await;
    for challenge in [false, true] {
        let (_owner, cancel) = cancellation();
        let (result, ()) = tokio::join!(
            store.claim_for_apply(&incoming, handle, Prompt::Never, cancel, || panic!(
                "Контекст нельзя читать без авторизации"
            )),
            auth(&mut stream, &authority, false, challenge)
        );
        assert_eq!(
            result.unwrap_err(),
            PreparedError::Authorization(if challenge {
                AuthorizationError::Challenge
            } else {
                AuthorizationError::Denied
            })
        );
    }
    let other = bus.connect().await;
    let mut calls = MessageStream::from(store.connection());
    let call = Message::method("/cloud/highpriority/yonik", "Apply")
        .unwrap()
        .destination(store.connection().unique_name().unwrap())
        .unwrap()
        .build(&())
        .unwrap();
    other.send(&call).await.unwrap();
    let other_call = next_call(&mut calls, "Apply").await;
    assert_eq!(
        claimed(&store, &other_call, handle, &mut stream, &authority)
            .await
            .unwrap_err(),
        PreparedError::InvalidHandle
    );
    claimed(&store, &incoming, handle, &mut stream, &authority)
        .await
        .unwrap();
    assert_eq!(
        claimed(&store, &incoming, handle, &mut stream, &authority)
            .await
            .unwrap_err(),
        PreparedError::InvalidHandle
    );
}
#[tokio::test]
async fn stale_context_digest_ttl_and_forget_invalidate() {
    let Fixture {
        bus: _bus,
        helper,
        peer: _peer,
        authority,
        mut stream,
        incoming,
    } = Fixture::new().await;
    let store = PreparedRequests::with_authorizer(helper);
    for field in 0..5 {
        let handle = prepared(&store, &incoming, &mut stream, &authority).await;
        let mut current = context();
        match field {
            0 => current.session = NonZeroU64::new(2).unwrap(),
            1 => current.generation = NonZeroU64::new(2).unwrap(),
            2 => current.environment.network = 2,
            3 => current.environment.authorization = 2,
            _ => current.environment.authorized = false,
        }
        let (_owner, cancel) = cancellation();
        let (result, ()) = tokio::join!(
            store.claim_for_apply(&incoming, handle, Prompt::Never, cancel, || Ok(current)),
            auth(&mut stream, &authority, true, false)
        );
        assert_eq!(result.unwrap_err(), PreparedError::Stale);
        assert_eq!(
            claimed(&store, &incoming, handle, &mut stream, &authority)
                .await
                .unwrap_err(),
            PreparedError::InvalidHandle
        );
    }
    let handle = prepared(&store, &incoming, &mut stream, &authority).await;
    store
        .state
        .lock()
        .unwrap()
        .entries
        .get(&handle)
        .unwrap()
        .artifact
        .0
        .lock()
        .unwrap()[0] = 7;
    assert_eq!(
        claimed(&store, &incoming, handle, &mut stream, &authority)
            .await
            .unwrap_err(),
        PreparedError::Stale
    );
    let handle = prepared(&store, &incoming, &mut stream, &authority).await;
    store
        .clock_offset_ms
        .store(61_000, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        claimed(&store, &incoming, handle, &mut stream, &authority)
            .await
            .unwrap_err(),
        PreparedError::InvalidHandle
    );
    let handle = prepared(&store, &incoming, &mut stream, &authority).await;
    store
        .forget_peer(incoming.header().sender().unwrap().as_str())
        .unwrap();
    assert_eq!(
        claimed(&store, &incoming, handle, &mut stream, &authority)
            .await
            .unwrap_err(),
        PreparedError::InvalidHandle
    );
}
#[tokio::test]
async fn caps_and_cancel_reclaim_slots_before_work() {
    let Fixture {
        bus: _bus,
        helper,
        peer: _peer,
        authority,
        mut stream,
        incoming,
    } = Fixture::new().await;
    let store = PreparedRequests::with_authorizer(helper);
    for _ in 0..MAX_PENDING_PER_PEER {
        prepared(&store, &incoming, &mut stream, &authority).await;
    }
    let (_owner, cancel) = cancellation();
    assert_eq!(
        store
            .prepare(&incoming, Prompt::Never, cancel, || async {
                panic!("Лимит до factory")
            })
            .await
            .unwrap_err(),
        PreparedError::Capacity
    );
    store
        .forget_peer(incoming.header().sender().unwrap().as_str())
        .unwrap();
    let (owner, cancel) = cancellation();
    owner.send(true).unwrap();
    assert_eq!(
        store
            .prepare(&incoming, Prompt::Never, cancel, || async {
                panic!("Отмена до factory")
            })
            .await
            .unwrap_err(),
        PreparedError::Authorization(AuthorizationError::Cancelled)
    );
    assert!(store.state.lock().unwrap().reservations.is_empty());
    let (owner, cancel) = cancellation();
    let operation = store.prepare(&incoming, Prompt::Never, cancel, || async {
        owner.send(true).unwrap();
        std::future::pending().await
    });
    let (result, ()) = tokio::join!(operation, auth(&mut stream, &authority, true, false));
    assert_eq!(result.unwrap_err(), PreparedError::Cancelled);
    assert!(store.state.lock().unwrap().reservations.is_empty());
    prepared(&store, &incoming, &mut stream, &authority).await;
}
#[tokio::test]
async fn simultaneous_claims_release_exactly_once() {
    let Fixture {
        bus: _bus,
        helper,
        peer: _peer,
        authority,
        mut stream,
        incoming,
    } = Fixture::new().await;
    let store = PreparedRequests::with_authorizer(helper);
    let handle = prepared(&store, &incoming, &mut stream, &authority).await;
    let (_owner, cancel) = cancellation();
    let server = async {
        for _ in 0..2 {
            auth(&mut stream, &authority, true, false).await;
        }
    };
    let (one, two, ()) = tokio::join!(
        store.claim_for_apply(&incoming, handle, Prompt::Never, cancel.clone(), || Ok(
            context()
        )),
        store.claim_for_apply(&incoming, handle, Prompt::Never, cancel, || Ok(context())),
        server
    );
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
}
#[test]
fn exact_wire_handle_and_redacted_debug() {
    assert!(PreparedHandle::try_from(&[0; 31][..]).is_err());
    assert!(PreparedHandle::try_from(&[0; 33][..]).is_err());
    let handle = PreparedHandle::try_from(&[97; 32][..]).unwrap();
    assert_eq!(format!("{handle:?}"), "PreparedHandle(<скрыто>)");
    assert!(
        PreparedContext::new(
            NonZeroU64::new(1).unwrap(),
            NonZeroU64::new(1).unwrap(),
            EnvironmentStamp {
                network: 0,
                ..context().environment
            }
        )
        .is_err()
    );
}
#[tokio::test]
async fn global_and_inflight_limits_expiry_reclaim() {
    let Fixture {
        bus,
        helper,
        peer: _peer,
        authority,
        mut stream,
        incoming: _incoming,
    } = Fixture::new().await;
    let store = PreparedRequests::with_authorizer(helper);
    let mut server_stream = MessageStream::from(store.connection());
    let mut peers = Vec::new();
    let mut messages = Vec::new();
    for _ in 0..5 {
        let peer = bus.connect().await;
        let call = Message::method("/cloud/highpriority/yonik", "Prepare")
            .unwrap()
            .destination(store.connection().unique_name().unwrap())
            .unwrap()
            .build(&())
            .unwrap();
        peer.send(&call).await.unwrap();
        messages.push(next_call(&mut server_stream, "Prepare").await);
        peers.push(peer);
    }
    drop(server_stream);
    for message in messages.iter().take(4) {
        for _ in 0..MAX_PENDING_PER_PEER {
            prepared(&store, message, &mut stream, &authority).await;
        }
    }
    let (_owner, cancel) = cancellation();
    assert_eq!(
        store
            .prepare(&messages[4], Prompt::Never, cancel, || async {
                panic!("Глобальный лимит до factory")
            })
            .await
            .unwrap_err(),
        PreparedError::Capacity
    );
    store
        .clock_offset_ms
        .store(61_000, std::sync::atomic::Ordering::Relaxed);
    prepared(&store, &messages[4], &mut stream, &authority).await;
    assert_eq!(store.state.lock().unwrap().entries.len(), 1);
    let permits = store
        .in_flight
        .acquire_many(MAX_IN_FLIGHT as u32)
        .await
        .unwrap();
    let (_owner, cancel) = cancellation();
    assert_eq!(
        store
            .prepare(&messages[0], Prompt::Never, cancel, || async {
                panic!("In-flight лимит до auth")
            })
            .await
            .unwrap_err(),
        PreparedError::Capacity
    );
    drop(permits);
    prepared(&store, &messages[0], &mut stream, &authority).await;
}
#[tokio::test]
async fn factory_failure_size_disconnect_and_forget_during_work() {
    let Fixture {
        bus: _bus,
        helper,
        peer,
        authority,
        mut stream,
        incoming,
    } = Fixture::new().await;
    let store = PreparedRequests::with_authorizer(helper);
    for oversized in [false, true] {
        let (_owner, cancel) = cancellation();
        let operation = store.prepare(&incoming, Prompt::Never, cancel, || async {
            if oversized {
                Ok((
                    Artifact(std::sync::Arc::new(Mutex::new(vec![
                        0;
                        MAX_ARTIFACT_BYTES + 1
                    ]))),
                    context(),
                ))
            } else {
                Err(PreparedError::Unavailable)
            }
        });
        let (result, ()) = tokio::join!(operation, auth(&mut stream, &authority, true, false));
        assert_eq!(
            result.unwrap_err(),
            if oversized {
                PreparedError::ArtifactTooLarge
            } else {
                PreparedError::Unavailable
            }
        );
        assert!(store.state.lock().unwrap().reservations.is_empty());
    }
    let (_owner, cancel) = cancellation();
    let operation = store.prepare(&incoming, Prompt::Never, cancel, || async {
        store
            .forget_peer(incoming.header().sender().unwrap().as_str())
            .unwrap();
        Ok((artifact(), context()))
    });
    let (result, ()) = tokio::join!(operation, auth(&mut stream, &authority, true, false));
    assert_eq!(result.unwrap_err(), PreparedError::Stale);
    let prior = prepared(&store, &incoming, &mut stream, &authority).await;
    let (_owner, cancel) = cancellation();
    let operation = store.prepare(&incoming, Prompt::Never, cancel, || async {
        peer.close().await.unwrap();
        Ok((artifact(), context()))
    });
    let (result, ()) = tokio::join!(operation, auth(&mut stream, &authority, true, false));
    assert_eq!(
        result.unwrap_err(),
        PreparedError::Authorization(AuthorizationError::PeerGone)
    );
    let (_owner, cancel) = cancellation();
    assert_eq!(
        store
            .claim_for_apply(&incoming, prior, Prompt::Never, cancel, || panic!(
                "Отключённый peer не читает context"
            ))
            .await
            .unwrap_err(),
        PreparedError::Authorization(AuthorizationError::PeerGone)
    );
    store
        .forget_peer(incoming.header().sender().unwrap().as_str())
        .unwrap();
    assert!(store.state.lock().unwrap().entries.is_empty());
    assert!(store.state.lock().unwrap().reservations.is_empty());
}
#[tokio::test]
async fn denied_and_cancelled_auth_never_run_factory_and_reclaim_reservation() {
    let Fixture {
        bus: _bus,
        helper,
        peer: _peer,
        authority,
        mut stream,
        incoming,
    } = Fixture::new().await;
    let store = PreparedRequests::<Artifact>::with_authorizer(helper);
    let (_owner, cancel) = cancellation();
    let operation = store.prepare(&incoming, Prompt::Never, cancel, || async {
        panic!("Отказ до factory")
    });
    let (result, ()) = tokio::join!(operation, auth(&mut stream, &authority, false, false));
    assert_eq!(
        result.unwrap_err(),
        PreparedError::Authorization(AuthorizationError::Denied)
    );
    assert!(store.state.lock().unwrap().reservations.is_empty());
    let (owner, cancel) = cancellation();
    let operation = store.prepare(&incoming, Prompt::Never, cancel, || async {
        panic!("Отмена до factory")
    });
    let server = async {
        let _call = next_call(&mut stream, "CheckAuthorization").await;
        owner.send(true).unwrap();
        let cancelled = next_call(&mut stream, "CancelCheckAuthorization").await;
        authority.reply(&cancelled, &()).await.unwrap();
    };
    let (result, ()) = tokio::join!(operation, server);
    assert_eq!(
        result.unwrap_err(),
        PreparedError::Authorization(AuthorizationError::Cancelled)
    );
    assert!(store.state.lock().unwrap().reservations.is_empty());
    // Закрытый sender отменяет незавершённый config-check тем же каналом.
    let (owner, cancel) = cancellation();
    let operation = store.prepare(&incoming, Prompt::Never, cancel, || async {
        drop(owner);
        std::future::pending().await
    });
    let (result, ()) = tokio::join!(operation, auth(&mut stream, &authority, true, false));
    assert_eq!(result.unwrap_err(), PreparedError::Cancelled);
    assert!(store.state.lock().unwrap().reservations.is_empty());
    prepared(&store, &incoming, &mut stream, &authority).await;
}
