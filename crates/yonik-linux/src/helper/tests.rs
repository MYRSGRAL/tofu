use super::{AuthorizationError, NetworkAction, Prompt};

#[test]
fn actions_and_prompt_are_closed_and_noninteractive_by_default() {
    assert_eq!(
        NetworkAction::ManageNetwork.id(),
        "cloud.highpriority.tofu.manage-network"
    );
    assert_eq!(
        NetworkAction::UnlockNetwork.id(),
        "cloud.highpriority.tofu.unlock-network"
    );
    assert_eq!(Prompt::default(), Prompt::Never);
    assert_eq!(AuthorizationError::Denied, AuthorizationError::Denied);
}

use super::{AUTHORITY, INTERFACE, MAX_CHECKS, MAX_REPLY, MAX_REQUEST, PeerAuthorizer};
use crate::Cancellation;
use futures_util::StreamExt;
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
    .unwrap()
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

#[tokio::test]
async fn private_bus_allow_deny_challenge_and_closed_wire_contract() {
    let mut fixture = Fixture::new().await;
    for (action, prompt, allowed, challenge, expected) in [
        (
            NetworkAction::ManageNetwork,
            Prompt::Never,
            true,
            false,
            None,
        ),
        (
            NetworkAction::UnlockNetwork,
            Prompt::Allow,
            false,
            false,
            Some(AuthorizationError::Denied),
        ),
        (
            NetworkAction::ManageNetwork,
            Prompt::Never,
            false,
            true,
            Some(AuthorizationError::Challenge),
        ),
        (
            NetworkAction::ManageNetwork,
            Prompt::Never,
            true,
            true,
            Some(AuthorizationError::InvalidReply),
        ),
    ] {
        let (_owner, cancel) = cancellation();
        let query = fixture
            .helper
            .authorize(&fixture.incoming, action, prompt, cancel);
        let server = async {
            let call = next_call(&mut fixture.stream, "CheckAuthorization").await;
            let ((kind, values), id, details, flags, cancellation_id): Check =
                call.body().deserialize().unwrap();
            assert_eq!(
                call.header().destination().unwrap().as_str(),
                fixture.authority.unique_name().unwrap().as_str()
            );
            assert_eq!(kind, "system-bus-name");
            assert_eq!(values.len(), 1);
            assert_eq!(
                <&str>::try_from(values.get("name").unwrap()).unwrap(),
                fixture.peer.unique_name().unwrap().as_str()
            );
            assert_eq!(id, action.id());
            assert!(details.is_empty());
            assert_eq!(flags, u32::from(prompt == Prompt::Allow));
            assert!(cancellation_id.starts_with("yonik-"));
            reply(&fixture.authority, &call, allowed, challenge).await;
        };
        let (result, ()) = tokio::join!(query, server);
        if let Some(error) = expected {
            assert_eq!(result, Err(error));
        } else {
            let identity = result.unwrap();
            assert_eq!(identity.uid(), rustix::process::getuid().as_raw());
            assert_eq!(
                identity.sender(),
                fixture.peer.unique_name().unwrap().as_str()
            );
            assert_ne!(identity.uid(), 999_999);
        }
    }
}

#[tokio::test]
async fn missing_sender_non_call_and_oversized_message_are_rejected() {
    let fixture = Fixture::new().await;
    let missing = Message::method("/test", "Prepare")
        .unwrap()
        .build(&())
        .unwrap();
    let signal = Message::signal("/test", INTERFACE, "Prepare")
        .unwrap()
        .sender(":1.7")
        .unwrap()
        .build(&())
        .unwrap();
    let oversized = Message::method("/test", "Prepare")
        .unwrap()
        .sender(":1.7")
        .unwrap()
        .build(&vec![0u8; MAX_REQUEST + 1])
        .unwrap();
    for message in [missing, signal, oversized] {
        let (_owner, cancel) = cancellation();
        assert_eq!(
            fixture
                .helper
                .authorize(
                    &message,
                    NetworkAction::ManageNetwork,
                    Prompt::Never,
                    cancel
                )
                .await,
            Err(AuthorizationError::InvalidMessage)
        );
    }
    // zbus типизирует sender как UniqueName, well-known name не строится.
    assert!(
        Message::method("/test", "Prepare")
            .unwrap()
            .sender("org.impostor.Client")
            .is_err()
    );
}

#[tokio::test]
async fn untrusted_authority_is_refused_without_query() {
    let mut fixture = Fixture::new().await;
    fixture.helper.expected_authority_uid = rustix::process::getuid().as_raw().wrapping_add(1);
    let (_owner, cancel) = cancellation();
    assert_eq!(
        fixture
            .helper
            .authorize(
                &fixture.incoming,
                NetworkAction::ManageNetwork,
                Prompt::Never,
                cancel
            )
            .await,
        Err(AuthorizationError::UntrustedAuthority)
    );
}

#[tokio::test]
async fn malformed_and_oversized_polkit_replies_fail_closed() {
    let mut fixture = Fixture::new().await;
    for oversized in [false, true] {
        let (_owner, cancel) = cancellation();
        let query = fixture.helper.authorize(
            &fixture.incoming,
            NetworkAction::ManageNetwork,
            Prompt::Never,
            cancel,
        );
        let server = async {
            let call = next_call(&mut fixture.stream, "CheckAuthorization").await;
            if oversized {
                fixture
                    .authority
                    .reply(
                        &call,
                        &((
                            true,
                            false,
                            HashMap::from([("secret", "x".repeat(MAX_REPLY))]),
                        ),),
                    )
                    .await
                    .unwrap();
            } else {
                fixture.authority.reply(&call, &42u32).await.unwrap();
            }
        };
        let (result, ()) = tokio::join!(query, server);
        assert_eq!(result, Err(AuthorizationError::InvalidReply));
    }
}

#[tokio::test]
async fn peer_disconnected_during_check_cannot_receive_identity() {
    let mut fixture = Fixture::new().await;
    let (_owner, cancel) = cancellation();
    let query = fixture.helper.authorize(
        &fixture.incoming,
        NetworkAction::ManageNetwork,
        Prompt::Never,
        cancel,
    );
    let server = async {
        let call = next_call(&mut fixture.stream, "CheckAuthorization").await;
        fixture.peer.close().await.unwrap();
        reply(&fixture.authority, &call, true, false).await;
    };
    let (result, ()) = tokio::join!(query, server);
    assert_eq!(result, Err(AuthorizationError::PeerGone));
}

#[tokio::test]
async fn authority_replacement_during_check_cannot_authorize() {
    let mut fixture = Fixture::new().await;
    let replacement = fixture.bus.connect().await;
    let (_owner, cancel) = cancellation();
    let query = fixture.helper.authorize(
        &fixture.incoming,
        NetworkAction::ManageNetwork,
        Prompt::Never,
        cancel,
    );
    let server = async {
        let call = next_call(&mut fixture.stream, "CheckAuthorization").await;
        fixture.authority.release_name(AUTHORITY).await.unwrap();
        replacement.request_name(AUTHORITY).await.unwrap();
        reply(&fixture.authority, &call, true, false).await;
    };
    let (result, ()) = tokio::join!(query, server);
    assert_eq!(result, Err(AuthorizationError::AuthorityChanged));
}

#[tokio::test]
async fn cancellation_and_closed_owner_cancel_remote_check_and_ignore_late_allow() {
    for closed in [false, true] {
        let mut fixture = Fixture::new().await;
        let (owner, cancel) = cancellation();
        let query = fixture.helper.authorize(
            &fixture.incoming,
            NetworkAction::ManageNetwork,
            Prompt::Allow,
            cancel,
        );
        let server = async {
            let check = next_call(&mut fixture.stream, "CheckAuthorization").await;
            let (_, _, _, _, id): Check = check.body().deserialize().unwrap();
            if closed {
                drop(owner);
            } else {
                owner.send_replace(true);
            }
            let cancel = next_call(&mut fixture.stream, "CancelCheckAuthorization").await;
            assert_eq!(
                cancel.header().destination().unwrap().as_str(),
                fixture.authority.unique_name().unwrap().as_str()
            );
            let (cancel_id,): (String,) = cancel.body().deserialize().unwrap();
            assert_eq!(id, cancel_id);
            fixture.authority.reply(&cancel, &()).await.unwrap();
            reply(&fixture.authority, &check, true, false).await;
        };
        let (result, ()) = tokio::join!(query, server);
        assert_eq!(result, Err(AuthorizationError::Cancelled));
    }
}

#[tokio::test]
async fn timeout_cancels_remote_check_and_ignores_late_allow() {
    let mut fixture = Fixture::new().await;
    fixture.helper.noninteractive_timeout = Duration::from_millis(100);
    let (_owner, cancel) = cancellation();
    let query = fixture.helper.authorize(
        &fixture.incoming,
        NetworkAction::ManageNetwork,
        Prompt::Never,
        cancel,
    );
    let server = async {
        let check = next_call(&mut fixture.stream, "CheckAuthorization").await;
        let (_, _, _, _, id): Check = check.body().deserialize().unwrap();
        let cancel = next_call(&mut fixture.stream, "CancelCheckAuthorization").await;
        let (cancel_id,): (String,) = cancel.body().deserialize().unwrap();
        assert_eq!(id, cancel_id);
        fixture.authority.reply(&cancel, &()).await.unwrap();
        reply(&fixture.authority, &check, true, false).await;
    };
    let (result, ()) = tokio::join!(query, server);
    assert_eq!(result, Err(AuthorizationError::Timeout));
}

#[tokio::test]
async fn queued_checks_are_cancellable_before_start() {
    let fixture = Fixture::new().await;
    let _permits = fixture
        .helper
        .checks
        .acquire_many(MAX_CHECKS as u32)
        .await
        .unwrap();
    let (owner, cancel) = cancellation();
    let query = fixture.helper.authorize(
        &fixture.incoming,
        NetworkAction::ManageNetwork,
        Prompt::Never,
        cancel,
    );
    let signal = async {
        owner.send_replace(true);
    };
    let (result, ()) = tokio::join!(query, signal);
    assert_eq!(result, Err(AuthorizationError::Cancelled));
}

#[tokio::test]
async fn concurrent_checks_have_distinct_ids_and_do_not_cross_cancel() {
    let mut fixture = Fixture::new().await;
    let (owner_a, cancel_a) = cancellation();
    let (_owner_b, cancel_b) = cancellation();
    let first = fixture.helper.authorize(
        &fixture.incoming,
        NetworkAction::ManageNetwork,
        Prompt::Never,
        cancel_a,
    );
    let second = fixture.helper.authorize(
        &fixture.incoming,
        NetworkAction::UnlockNetwork,
        Prompt::Never,
        cancel_b,
    );
    let server = async {
        let one = next_call(&mut fixture.stream, "CheckAuthorization").await;
        let two = next_call(&mut fixture.stream, "CheckAuthorization").await;
        let (_, action_one, _, _, id_one): Check = one.body().deserialize().unwrap();
        let (_, _, _, _, id_two): Check = two.body().deserialize().unwrap();
        assert_ne!(id_one, id_two);
        let (cancelled_check, cancelled_id, surviving_check) =
            if action_one == NetworkAction::ManageNetwork.id() {
                (&one, &id_one, &two)
            } else {
                (&two, &id_two, &one)
            };
        owner_a.send_replace(true);
        let cancel = next_call(&mut fixture.stream, "CancelCheckAuthorization").await;
        let (id,): (String,) = cancel.body().deserialize().unwrap();
        assert_eq!(&id, cancelled_id);
        fixture.authority.reply(&cancel, &()).await.unwrap();
        reply(&fixture.authority, surviving_check, true, false).await;
        reply(&fixture.authority, cancelled_check, true, false).await;
    };
    let (a, b, ()) = tokio::join!(first, second, server);
    assert_eq!(a, Err(AuthorizationError::Cancelled));
    assert!(b.is_ok());
}

#[tokio::test]
async fn forged_replies_cancel_pending_real_check_and_ignore_late_allow() {
    for (use_error, expected) in [
        (false, AuthorizationError::InvalidReply),
        (true, AuthorizationError::Unavailable),
    ] {
        let mut fixture = Fixture::new().await;
        let attacker = fixture.bus.connect().await;
        let (_owner, cancel) = cancellation();
        let query = fixture.helper.authorize(
            &fixture.incoming,
            NetworkAction::ManageNetwork,
            Prompt::Never,
            cancel,
        );
        let server = async {
            let call = next_call(&mut fixture.stream, "CheckAuthorization").await;
            if use_error {
                attacker
                    .reply_error(&call, "org.freedesktop.DBus.Error.Failed", &"fixture")
                    .await
                    .unwrap();
            } else {
                reply(&attacker, &call, true, false).await;
            }
            // Барьер подтверждает обработку предыдущей записи тем же bus daemon.
            zbus::fdo::DBusProxy::new(&attacker)
                .await
                .unwrap()
                .get_id()
                .await
                .unwrap();
            let (_, _, _, _, expected_id): Check = call.body().deserialize().unwrap();
            let cancel = next_call(&mut fixture.stream, "CancelCheckAuthorization").await;
            assert_eq!(
                cancel.header().destination().unwrap().as_str(),
                fixture.authority.unique_name().unwrap().as_str()
            );
            let (id,): (String,) = cancel.body().deserialize().unwrap();
            assert_eq!(id, expected_id);
            fixture.authority.reply(&cancel, &()).await.unwrap();
            reply(&fixture.authority, &call, true, false).await;
        };
        let (result, ()) = tokio::join!(query, server);
        assert_eq!(result, Err(expected));
    }
}

#[tokio::test]
async fn cancellation_after_owner_replacement_still_targets_original_authority() {
    let mut fixture = Fixture::new().await;
    let replacement = fixture.bus.connect().await;
    let (owner, cancel) = cancellation();
    let query = fixture.helper.authorize(
        &fixture.incoming,
        NetworkAction::ManageNetwork,
        Prompt::Never,
        cancel,
    );
    let server = async {
        let check = next_call(&mut fixture.stream, "CheckAuthorization").await;
        let (_, _, _, _, check_id): Check = check.body().deserialize().unwrap();
        fixture.authority.release_name(AUTHORITY).await.unwrap();
        replacement.request_name(AUTHORITY).await.unwrap();
        owner.send_replace(true);
        let cancel = next_call(&mut fixture.stream, "CancelCheckAuthorization").await;
        assert_eq!(
            cancel.header().destination().unwrap().as_str(),
            fixture.authority.unique_name().unwrap().as_str()
        );
        let (cancel_id,): (String,) = cancel.body().deserialize().unwrap();
        assert_eq!(check_id, cancel_id);
        fixture.authority.reply(&cancel, &()).await.unwrap();
    };
    let (result, ()) = tokio::join!(query, server);
    assert_eq!(result, Err(AuthorizationError::Cancelled));
}

#[test]
fn system_polkit_account_is_closed_and_unambiguous() {
    use super::parse_polkit_uid;
    assert_eq!(
        parse_polkit_uid(
            "root:x:0:0:root:/root:/bin/sh\npolkitd:x:102:102:PolicyKit:/:/usr/bin/nologin\n"
        ),
        Ok(102)
    );
    assert_eq!(parse_polkit_uid("root:x:0:0:root:/root:/bin/sh\n"), Ok(0));
    for bad in [
        "polkitd:x:0:0::/:/bin/false",
        "polkitd:x:1000:1000::/:/bin/false",
        "polkitd:x:abc:102::/:/bin/false",
        "polkitd:x:102:102::/:/bin/false\npolkitd:x:103:103::/:/bin/false",
        "polkitd:x:102:102::/:/bin/false\nintruder:x:102:102::/:/bin/false",
        "polkitd:x:102",
    ] {
        assert!(parse_polkit_uid(bad).is_err(), "{bad}");
    }
}
#[test]
fn polkit_account_lookup_failure_is_closed() {
    assert!(
        super::trusted_polkit_uid(std::path::Path::new("/nonexistent-yonik-polkit-passwd"))
            .is_err()
    );
    assert!(super::trusted_polkit_uid(std::path::Path::new("/dev/null")).is_err());
}
