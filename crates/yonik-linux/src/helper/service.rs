//! Ограниченный versioned D-Bus transport: идентичность берётся только у daemon.

use std::{future::Future, sync::Arc, time::Duration};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{Mutex, Semaphore},
    task::JoinSet,
};
use zbus::{MatchRule, Message, MessageStream, message::Type};

use super::{
    AuthorizationError, MAX_IN_FLIGHT, NetworkAction, PreparedArtifact, PreparedContext,
    PreparedError, PreparedHandle, PreparedRequests, Prompt,
};
use crate::Cancellation;

pub const SERVICE_NAME: &str = "cloud.highpriority.yonik.Network1";
pub const SERVICE_PATH: &str = "/cloud/highpriority/yonik/Network";
pub const SERVICE_INTERFACE: &str = SERVICE_NAME;
const MAX_WIRE_BYTES: usize = 64 * 1024;
const INTROSPECTION: &str = r#"<node><interface name="cloud.highpriority.yonik.Network1"><method name="Prepare"><arg name="request" type="s" direction="in"/><arg name="response" type="s" direction="out"/></method><method name="Apply"><arg name="request" type="s" direction="in"/><arg name="response" type="s" direction="out"/></method><method name="Stop"><arg name="request" type="s" direction="in"/><arg name="response" type="s" direction="out"/></method><method name="Unlock"><arg name="request" type="s" direction="in"/><arg name="response" type="s" direction="out"/></method><method name="State"><arg name="request" type="s" direction="in"/><arg name="response" type="s" direction="out"/></method><method name="Capabilities"><arg name="request" type="s" direction="in"/><arg name="response" type="s" direction="out"/></method></interface><interface name="org.freedesktop.DBus.Introspectable"><method name="Introspect"><arg name="xml" type="s" direction="out"/></method></interface></node>"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionState {
    NoManagedPolicy,
    VerifiedRules,
    RecoveryRequired,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceError {
    InvalidRequest,
    Unsupported,
    Busy,
    Authorization(AuthorizationError),
    Prepared(PreparedError),
    Unavailable,
    Cancelled,
}
impl From<PreparedError> for ServiceError {
    fn from(error: PreparedError) -> Self {
        Self::Prepared(error)
    }
}
impl From<AuthorizationError> for ServiceError {
    fn from(error: AuthorizationError) -> Self {
        Self::Authorization(error)
    }
}
impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for ServiceError {}
impl ServiceError {
    fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Unsupported => "unsupported",
            Self::Busy | Self::Prepared(PreparedError::Capacity) => "busy",
            Self::Authorization(_) | Self::Prepared(PreparedError::Authorization(_)) => {
                "authorization"
            }
            Self::Prepared(PreparedError::InvalidHandle) => "invalid_handle",
            Self::Prepared(PreparedError::Stale) => "stale",
            Self::Cancelled | Self::Prepared(PreparedError::Cancelled) => "cancelled",
            Self::Prepared(_) | Self::Unavailable => "unavailable",
        }
    }
}

/// Единственный владелец привилегированных эффектов; dispatcher сериализует доступ.
pub trait ProtectionHost: Send + 'static {
    type Artifact: PreparedArtifact;
    fn prepare(
        &mut self,
        uid: u32,
        allow_lan: bool,
        cancel: Cancellation,
    ) -> impl Future<Output = Result<(Self::Artifact, PreparedContext), ServiceError>> + Send;
    fn context(&self, uid: u32) -> Result<PreparedContext, ServiceError>;
    fn apply(
        &mut self,
        uid: u32,
        artifact: &Self::Artifact,
        cancel: Cancellation,
    ) -> impl Future<Output = Result<ProtectionState, ServiceError>> + Send;
    fn stop(
        &mut self,
        uid: u32,
        cancel: Cancellation,
    ) -> impl Future<Output = Result<ProtectionState, ServiceError>> + Send;
    fn unlock(
        &mut self,
        uid: u32,
        cancel: Cancellation,
    ) -> impl Future<Output = Result<ProtectionState, ServiceError>> + Send;
    fn state(
        &mut self,
        uid: u32,
        cancel: Cancellation,
    ) -> impl Future<Output = Result<ProtectionState, ServiceError>> + Send;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareRequest {
    version: u32,
    mode: String,
    allow_lan: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyRequest {
    version: u32,
    handle: [u8; 32],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyRequest {
    version: u32,
}

enum Request {
    Prepare(bool),
    Apply(PreparedHandle),
    Stop,
    Unlock,
    State,
    Capabilities,
    Introspect,
}
fn parse(message: &Message) -> Result<Request, ServiceError> {
    let header = message.header();
    if header.path().map(|v| v.as_str()) == Some(SERVICE_PATH)
        && header.interface().map(|v| v.as_str()) == Some("org.freedesktop.DBus.Introspectable")
        && header.member().map(|v| v.as_str()) == Some("Introspect")
        && message.body().is_empty()
    {
        return Ok(Request::Introspect);
    }
    if header.path().map(|v| v.as_str()) != Some(SERVICE_PATH)
        || header.interface().map(|v| v.as_str()) != Some(SERVICE_INTERFACE)
        || message.body().len() > MAX_WIRE_BYTES
    {
        return Err(ServiceError::InvalidRequest);
    }
    let body = message.body();
    let (json,): (&str,) = body
        .deserialize()
        .map_err(|_| ServiceError::InvalidRequest)?;
    match header.member().map(|v| v.as_str()) {
        Some("Prepare") => {
            let dto: PrepareRequest =
                serde_json::from_str(json).map_err(|_| ServiceError::InvalidRequest)?;
            if dto.version != 1 {
                return Err(ServiceError::InvalidRequest);
            }
            if dto.mode != "public_protection" || dto.allow_lan {
                return Err(ServiceError::Unsupported);
            }
            Ok(Request::Prepare(dto.allow_lan))
        }
        Some("Apply") => {
            let dto: ApplyRequest =
                serde_json::from_str(json).map_err(|_| ServiceError::InvalidRequest)?;
            if dto.version != 1 {
                return Err(ServiceError::InvalidRequest);
            }
            Ok(Request::Apply(PreparedHandle::try_from(
                dto.handle.as_slice(),
            )?))
        }
        Some(member) => {
            let dto: EmptyRequest =
                serde_json::from_str(json).map_err(|_| ServiceError::InvalidRequest)?;
            if dto.version != 1 {
                return Err(ServiceError::InvalidRequest);
            }
            match member {
                "Stop" => Ok(Request::Stop),
                "Unlock" => Ok(Request::Unlock),
                "State" => Ok(Request::State),
                "Capabilities" => Ok(Request::Capabilities),
                _ => Err(ServiceError::Unsupported),
            }
        }
        None => Err(ServiceError::InvalidRequest),
    }
}

pub async fn run_system_service(
    host: impl ProtectionHost,
    cancel: Cancellation,
) -> Result<(), ServiceError> {
    let broker = PreparedRequests::system(cancel.clone()).await?;
    serve_with_broker(host, broker, cancel).await
}

pub(super) async fn serve_with_broker<H: ProtectionHost>(
    host: H,
    broker: PreparedRequests<H::Artifact>,
    mut cancel: Cancellation,
) -> Result<(), ServiceError> {
    let broker = Arc::new(broker);
    let connection = broker.connection().clone();
    let mut calls = MessageStream::from(&connection);
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|_| ServiceError::Unavailable)?
        .interface("org.freedesktop.DBus")
        .map_err(|_| ServiceError::Unavailable)?
        .path("/org/freedesktop/DBus")
        .map_err(|_| ServiceError::Unavailable)?
        .member("NameOwnerChanged")
        .map_err(|_| ServiceError::Unavailable)?
        .build();
    let mut changes = MessageStream::for_match_rule(rule, &connection, Some(64))
        .await
        .map_err(|_| ServiceError::Unavailable)?;
    connection
        .request_name(SERVICE_NAME)
        .await
        .map_err(|_| ServiceError::Unavailable)?;
    let host = Arc::new(Mutex::new(host));
    let permits = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    let (stop, worker_cancel) = Cancellation::new();
    let mut tasks = JoinSet::new();
    let result = loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break Ok(()),
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            changed = changes.next() => match changed {
                Some(Ok(message)) => if let Err(error) = forget_disconnected(&broker, &message) { break Err(error); },
                _ => break Err(ServiceError::Unavailable),
            },
            call = calls.next() => {
                let message = match call { Some(Ok(message)) => message, _ => break Err(ServiceError::Unavailable) };
                if message.header().message_type() != Type::MethodCall { continue; }
                let permit = match permits.clone().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => { send_result(&connection, &message, Err(ServiceError::Busy)).await; continue; }
                };
                let broker = broker.clone(); let host = host.clone(); let child_cancel = worker_cancel.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    let result = dispatch(&broker, &host, &message, child_cancel).await;
                    send_result(broker.connection(), &message, result).await;
                });
            }
        }
    };
    let _ = stop.send(true);
    while tasks.join_next().await.is_some() {}
    let _ = connection.release_name(SERVICE_NAME).await;
    result
}

fn forget_disconnected<A: PreparedArtifact>(
    broker: &PreparedRequests<A>,
    message: &Message,
) -> Result<(), ServiceError> {
    let h = message.header();
    if h.message_type() != Type::Signal
        || h.sender().map(|v| v.as_str()) != Some("org.freedesktop.DBus")
        || h.path().map(|v| v.as_str()) != Some("/org/freedesktop/DBus")
        || h.interface().map(|v| v.as_str()) != Some("org.freedesktop.DBus")
        || h.member().map(|v| v.as_str()) != Some("NameOwnerChanged")
    {
        return Ok(());
    }
    let body = message.body();
    let (name, old, new): (&str, &str, &str) = body
        .deserialize()
        .map_err(|_| ServiceError::InvalidRequest)?;
    if name.starts_with(':') && name == old && new.is_empty() {
        broker.forget_peer(name)?;
    }
    Ok(())
}

async fn dispatch<H: ProtectionHost>(
    broker: &PreparedRequests<H::Artifact>,
    host: &Mutex<H>,
    message: &Message,
    mut cancel: Cancellation,
) -> Result<String, ServiceError> {
    let request = parse(message)?;
    let sender = super::sender(message)?;
    let uid = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(ServiceError::Cancelled),
        result = tokio::time::timeout(Duration::from_secs(5), broker.authorizer().uid(&sender)) =>
            result.map_err(|_| ServiceError::Unavailable)??,
    };
    let value = match request {
        Request::Introspect => return Ok(INTROSPECTION.to_owned()),
        Request::Capabilities => {
            serde_json::json!({"version":1,"public_protection":true,"system_tun":false,"allow_lan":false})
        }
        Request::Prepare(allow_lan) => {
            let handle = broker
                .prepare(message, Prompt::Allow, cancel.clone(), || async {
                    let mut host = host.try_lock().map_err(|_| PreparedError::Capacity)?;
                    host.prepare(uid, allow_lan, cancel.clone())
                        .await
                        .map_err(as_prepared)
                })
                .await?;
            serde_json::json!({"version":1,"handle":handle.as_bytes()})
        }
        Request::Apply(handle) => {
            let mut guard = None;
            let artifact = broker
                .claim_for_apply(message, handle, Prompt::Allow, cancel.clone(), || {
                    let locked = host.try_lock().map_err(|_| PreparedError::Capacity)?;
                    let context = locked.context(uid).map_err(as_prepared)?;
                    guard = Some(locked);
                    Ok(context)
                })
                .await?;
            let mut host = guard.ok_or(ServiceError::Unavailable)?;
            let state = host.apply(uid, artifact.as_ref(), cancel).await?;
            serde_json::json!({"version":1,"state":state})
        }
        Request::State => {
            let mut host = host.try_lock().map_err(|_| ServiceError::Busy)?;
            let state = host.state(uid, cancel).await?;
            serde_json::json!({"version":1,"state":state})
        }
        Request::Stop | Request::Unlock => {
            let action = if matches!(request, Request::Unlock) {
                NetworkAction::UnlockNetwork
            } else {
                NetworkAction::ManageNetwork
            };
            let peer = broker
                .authorizer()
                .authorize(message, action, Prompt::Allow, cancel.clone())
                .await?;
            if peer.uid() != uid {
                return Err(ServiceError::Authorization(AuthorizationError::PeerGone));
            }
            let mut host = host.try_lock().map_err(|_| ServiceError::Busy)?;
            let state = if matches!(request, Request::Unlock) {
                host.unlock(uid, cancel).await?
            } else {
                host.stop(uid, cancel).await?
            };
            serde_json::json!({"version":1,"state":state})
        }
    };
    let json = serde_json::to_string(&value).map_err(|_| ServiceError::Unavailable)?;
    if json.len() > MAX_WIRE_BYTES {
        return Err(ServiceError::Unavailable);
    }
    Ok(json)
}
fn as_prepared(error: ServiceError) -> PreparedError {
    match error {
        ServiceError::Prepared(error) => error,
        ServiceError::Authorization(error) => PreparedError::Authorization(error),
        ServiceError::Busy => PreparedError::Capacity,
        ServiceError::Cancelled => PreparedError::Cancelled,
        _ => PreparedError::Unavailable,
    }
}
async fn send_result(
    connection: &zbus::Connection,
    message: &Message,
    result: Result<String, ServiceError>,
) {
    let operation = async {
        match result {
            Ok(json) => connection.reply(message, &(json,)).await,
            Err(error) => {
                let json = serde_json::json!({"version":1,"error":error.code()}).to_string();
                connection
                    .reply_error(message, "cloud.highpriority.yonik.Network1.Error", &(json,))
                    .await
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(5), operation).await;
}

#[cfg(test)]
#[path = "service_tests.rs"]
pub(super) mod tests;
