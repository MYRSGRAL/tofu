use super::*;
use crate::helper::{MAX_CHECKS, PeerAuthorizer};
use std::{
    collections::HashMap,
    num::NonZeroU64,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::watch;
use yonik_application::{ConfigDigest, EnvironmentStamp};

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
    async fn connect(&self) -> zbus::Connection {
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
struct Artifact;
impl PreparedArtifact for Artifact {
    fn current_digest(&self) -> ConfigDigest {
        ConfigDigest::new([7; 32])
    }
    fn byte_len(&self) -> usize {
        6
    }
    fn retained_bytes(&self) -> usize {
        6
    }
}
struct Host {
    effects: Arc<AtomicUsize>,
    generation: Arc<AtomicUsize>,
}
impl ProtectionHost for Host {
    type Artifact = Artifact;
    async fn prepare(
        &mut self,
        _: u32,
        _: bool,
        _: Cancellation,
    ) -> Result<(Artifact, PreparedContext), ServiceError> {
        Ok((Artifact, self.context(0)?))
    }
    fn context(&self, _: u32) -> Result<PreparedContext, ServiceError> {
        Ok(PreparedContext::new(
            NonZeroU64::new(1).unwrap(),
            NonZeroU64::new(self.generation.load(Ordering::SeqCst) as u64).unwrap(),
            EnvironmentStamp {
                network: 1,
                applications: 1,
                authorization: 1,
                engine: 1,
                authorized: true,
            },
        )?)
    }
    async fn apply(
        &mut self,
        _: u32,
        _: &Artifact,
        _: Cancellation,
    ) -> Result<ProtectionState, ServiceError> {
        self.effects.fetch_add(1, Ordering::SeqCst);
        Ok(ProtectionState::VerifiedRules)
    }
    async fn stop(&mut self, _: u32, _: Cancellation) -> Result<ProtectionState, ServiceError> {
        Ok(ProtectionState::VerifiedRules)
    }
    async fn unlock(&mut self, _: u32, _: Cancellation) -> Result<ProtectionState, ServiceError> {
        self.effects.fetch_add(1, Ordering::SeqCst);
        Ok(ProtectionState::NoManagedPolicy)
    }
    async fn state(&mut self, _: u32, _: Cancellation) -> Result<ProtectionState, ServiceError> {
        Ok(ProtectionState::Unknown)
    }
}
pub(in crate::helper) struct Fixture {
    _bus: PrivateBus,
    peer: zbus::Connection,
    stop: watch::Sender<bool>,
    server: tokio::task::JoinHandle<Result<(), ServiceError>>,
    auth_stop: watch::Sender<bool>,
    auth: tokio::task::JoinHandle<()>,
    checks: Arc<AtomicUsize>,
    effects: Arc<AtomicUsize>,
    allowed: Arc<AtomicBool>,
    challenge: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    gate: Arc<Semaphore>,
    generation: Arc<AtomicUsize>,
    actions: Arc<std::sync::Mutex<Vec<String>>>,
}
impl Fixture {
    async fn new() -> Self {
        let effects = Arc::new(AtomicUsize::new(0));
        let generation = Arc::new(AtomicUsize::new(1));
        let host = Host {
            effects: effects.clone(),
            generation: generation.clone(),
        };
        Self::with_host(host, effects, generation).await
    }
    pub(in crate::helper) async fn real_host(host: impl ProtectionHost) -> Self {
        Self::with_host(
            host,
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(1)),
        )
        .await
    }
    async fn with_host<H: ProtectionHost>(
        host: H,
        effects: Arc<AtomicUsize>,
        generation: Arc<AtomicUsize>,
    ) -> Self {
        let bus = PrivateBus::start();
        let connection = bus.connect().await;
        let peer = bus.connect().await;
        let authority = bus.connect().await;
        authority
            .request_name(crate::helper::AUTHORITY)
            .await
            .unwrap();
        let mut stream = MessageStream::from(&authority);
        let checks = Arc::new(AtomicUsize::new(0));
        let allowed = Arc::new(AtomicBool::new(true));
        let challenge = Arc::new(AtomicBool::new(false));
        let actions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let pause = Arc::new(AtomicBool::new(false));
        let gate = Arc::new(Semaphore::new(0));
        let paused = pause.clone();
        let wait = gate.clone();
        let (auth_stop, mut auth_cancel) = Cancellation::new();
        let ac = checks.clone();
        let aa = allowed.clone();
        let ach = challenge.clone();
        let actions_copy = actions.clone();
        let auth = tokio::spawn(async move {
            loop {
                tokio::select! { _=auth_cancel.cancelled()=>break, message=stream.next()=> {
                    let Some(Ok(message))=message else {break};
                    if message.header().message_type()!=Type::MethodCall {continue;}
                    if message.header().member().map(|v|v.as_str())==Some("CheckAuthorization") {
                        type Subject=(String,HashMap<String,zbus::zvariant::OwnedValue>);
                        let (_,action,_,_,_):(Subject,String,HashMap<String,String>,u32,String)=message.body().deserialize().unwrap();
                        actions_copy.lock().unwrap().push(action);
                        ac.fetch_add(1,Ordering::SeqCst);
                        if paused.load(Ordering::SeqCst) { let permit=wait.acquire().await.unwrap(); permit.forget(); }
                        authority.reply(&message,&((aa.load(Ordering::SeqCst),ach.load(Ordering::SeqCst),HashMap::<&str,&str>::new()),)).await.unwrap();
                    }
                } }
            }
        });
        let authorizer = PeerAuthorizer {
            connection,
            expected_authority_uid: rustix::process::getuid().as_raw(),
            checks: Semaphore::new(MAX_CHECKS),
            noninteractive_timeout: Duration::from_secs(5),
            interactive_timeout: Duration::from_secs(5),
        };
        let broker = PreparedRequests::with_authorizer(authorizer).unwrap();
        let (stop, cancel) = Cancellation::new();
        let server = tokio::spawn(serve_with_broker(host, broker, cancel));
        // Барьер готовности через daemon, не фиксированная задержка.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if peer
                    .call_method(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        Some("org.freedesktop.DBus"),
                        "GetNameOwner",
                        &(SERVICE_NAME,),
                    )
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        Self {
            _bus: bus,
            peer,
            stop,
            server,
            auth_stop,
            auth,
            checks,
            effects,
            allowed,
            challenge,
            pause,
            gate,
            generation,
            actions,
        }
    }
    pub(in crate::helper) async fn call(
        &self,
        member: &str,
        json: &str,
    ) -> Result<serde_json::Value, zbus::Error> {
        let reply = self
            .peer
            .call_method(
                Some(SERVICE_NAME),
                SERVICE_PATH,
                Some(SERVICE_INTERFACE),
                member,
                &(json,),
            )
            .await?;
        let (json,): (String,) = reply.body().deserialize()?;
        Ok(serde_json::from_str(&json).unwrap())
    }
    pub(in crate::helper) fn check_count(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }
    pub(in crate::helper) async fn shutdown(self) {
        self.stop.send(true).unwrap();
        self.server.await.unwrap().unwrap();
        self.auth_stop.send(true).unwrap();
        self.auth.await.unwrap();
    }
}
#[tokio::test]
async fn real_bus_prepare_apply_replay_and_unlock_action() {
    let f = Fixture::new().await;
    let capabilities = f.call("Capabilities", r#"{"version":1}"#).await.unwrap();
    assert_eq!(capabilities["system_tun"], false);
    assert_eq!(capabilities["allow_lan"], false);
    assert_eq!(capabilities["public_protection"], true);
    let xml = f
        .peer
        .call_method(
            Some(SERVICE_NAME),
            SERVICE_PATH,
            Some("org.freedesktop.DBus.Introspectable"),
            "Introspect",
            &(),
        )
        .await
        .unwrap();
    let (xml,): (String,) = xml.body().deserialize().unwrap();
    assert!(xml.contains("method name=\"Apply\""));

    let result = f
        .call(
            "Prepare",
            r#"{"version":1,"mode":"public_protection","allow_lan":false}"#,
        )
        .await
        .unwrap();
    assert_eq!(f.checks.load(Ordering::SeqCst), 1);
    let request = serde_json::json!({"version":1,"handle":result["handle"]}).to_string();
    assert_eq!(
        f.call("Apply", &request).await.unwrap()["state"],
        "verified_rules"
    );
    assert_eq!(f.effects.load(Ordering::SeqCst), 1);
    assert!(f.call("Apply", &request).await.is_err());
    assert_eq!(f.effects.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.call("Stop", r#"{"version":1}"#).await.unwrap()["state"],
        "verified_rules"
    );
    f.call("Unlock", r#"{"version":1}"#).await.unwrap();
    assert_eq!(f.effects.load(Ordering::SeqCst), 2);
    assert_eq!(
        f.actions.lock().unwrap().last().unwrap(),
        crate::helper::NetworkAction::UnlockNetwork.id()
    );
    assert_eq!(f.checks.load(Ordering::SeqCst), 5);
    f.shutdown().await;
}
#[tokio::test]
async fn invalid_denied_challenged_stale_and_wrong_peer_have_no_effect() {
    let f = Fixture::new().await;
    for json in [
        r#"{"version":2,"mode":"public_protection","allow_lan":false}"#,
        r#"{"version":1,"mode":"public_protection","allow_lan":false,"uid":0}"#,
        r#"{"version":1,"mode":"public_protection","allow_lan":true}"#,
    ] {
        assert!(f.call("Prepare", json).await.is_err());
    }
    assert!(
        f.peer
            .call_method(
                Some(SERVICE_NAME),
                SERVICE_PATH,
                Some(SERVICE_INTERFACE),
                "Prepare",
                &(123u32,)
            )
            .await
            .is_err()
    );
    assert_eq!(f.checks.load(Ordering::SeqCst), 0);
    let prep = r#"{"version":1,"mode":"public_protection","allow_lan":false}"#;
    f.allowed.store(false, Ordering::SeqCst);
    assert!(f.call("Prepare", prep).await.is_err());
    f.challenge.store(true, Ordering::SeqCst);
    assert!(f.call("Prepare", prep).await.is_err());
    f.allowed.store(true, Ordering::SeqCst);
    f.challenge.store(false, Ordering::SeqCst);
    let value = f.call("Prepare", prep).await.unwrap();
    let apply = serde_json::json!({"version":1,"handle":value["handle"]}).to_string();
    let other = f._bus.connect().await;
    assert!(
        other
            .call_method(
                Some(SERVICE_NAME),
                SERVICE_PATH,
                Some(SERVICE_INTERFACE),
                "Apply",
                &(apply.as_str(),)
            )
            .await
            .is_err()
    );
    f.generation.store(2, Ordering::SeqCst);
    assert!(f.call("Apply", &apply).await.is_err());
    assert_eq!(f.effects.load(Ordering::SeqCst), 0);
    f.shutdown().await;
}

#[tokio::test]
async fn pending_authorization_does_not_lock_host_state() {
    for member in ["Apply", "Stop", "Unlock"] {
        let f = Fixture::new().await;
        let value = f
            .call(
                "Prepare",
                r#"{"version":1,"mode":"public_protection","allow_lan":false}"#,
            )
            .await
            .unwrap();
        let apply = if member == "Apply" {
            serde_json::json!({"version":1,"handle":value["handle"]}).to_string()
        } else {
            r#"{"version":1}"#.to_owned()
        };
        f.pause.store(true, Ordering::SeqCst);
        let peer = f.peer.clone();
        let pending = tokio::spawn(async move {
            peer.call_method(
                Some(SERVICE_NAME),
                SERVICE_PATH,
                Some(SERVICE_INTERFACE),
                member,
                &(apply,),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while f.checks.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let result = tokio::time::timeout(
            Duration::from_millis(500),
            f.call("State", r#"{"version":1}"#),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap()["state"], "unknown");
        assert_eq!(f.effects.load(Ordering::SeqCst), 0);
        f.gate.add_permits(1);
        pending.await.unwrap().unwrap();
        f.shutdown().await;
    }
}
