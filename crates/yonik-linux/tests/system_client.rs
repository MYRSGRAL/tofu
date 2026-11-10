#![cfg(target_os = "linux")]
use std::{
    io::BufRead,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::Mutex, time::Instant};
use yonik_linux::Cancellation;
use yonik_linux::system_client::{ClientError, SystemClient};
const SERVICE_NAME: &str = "cloud.highpriority.yonik.Network1";
const SERVICE_PATH: &str = "/cloud/highpriority/yonik/Network";
use yonik_linux::system_client::State;
struct Bus(std::process::Child, String);
impl Bus {
    fn new() -> Self {
        let mut child = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self(child, address.trim().into())
    }
    async fn connect(&self) -> zbus::Connection {
        zbus::connection::Builder::address(self.1.as_str())
            .unwrap()
            .build()
            .await
            .unwrap()
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "cloud.highpriority.yonik.Network1")]
enum WireError {
    Error(String),
}
struct Server {
    response: Arc<Mutex<String>>,
    effects: Arc<AtomicUsize>,
}
#[zbus::interface(name = "cloud.highpriority.yonik.Network1")]
impl Server {
    async fn state(&self, request: &str) -> Result<String, WireError> {
        assert_eq!(request, "{\"version\":1}");
        let response = self.response.lock().await.clone();
        if let Some(error) = response.strip_prefix("error:") {
            return Err(WireError::Error(error.into()));
        }
        Ok(response)
    }
    async fn capabilities(&self, _request: &str) -> String {
        "{\"version\":1,\"public_protection\":true,\"system_tun\":false,\"allow_lan\":false}".into()
    }
    async fn prepare(&self, request: &str) -> String {
        let value: serde_json::Value = serde_json::from_str(request).unwrap();
        assert_eq!(value["mode"], "public_protection");
        serde_json::json!({"version":1,"handle":vec![42;32]}).to_string()
    }
    async fn apply(&self, request: &str) -> String {
        let value: serde_json::Value = serde_json::from_str(request).unwrap();
        assert_eq!(value["handle"].as_array().unwrap().len(), 32);
        "{\"version\":1,\"state\":\"verified_rules\"}".into()
    }
    async fn stop(&self, _request: &str) -> String {
        self.effects.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(150)).await;
        "{\"version\":1,\"state\":\"verified_rules\"}".into()
    }
    async fn unlock(&self, _request: &str) -> String {
        "{\"version\":1,\"state\":\"no_managed_policy\"}".into()
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}
#[tokio::test]
async fn real_bus_contract_errors_cancellation_and_owner_fence() {
    let bus = Bus::new();
    let server = bus.connect().await;
    let response = Arc::new(Mutex::new(
        "{\"version\":1,\"state\":\"verified_rules\"}".into(),
    ));
    let effects = Arc::new(AtomicUsize::new(0));
    server
        .object_server()
        .at(
            SERVICE_PATH,
            Server {
                response: response.clone(),
                effects: effects.clone(),
            },
        )
        .await
        .unwrap();
    server.request_name(SERVICE_NAME).await.unwrap();
    let (keep, cancel) = Cancellation::new();
    let mut client = SystemClient::on_connection(bus.connect().await, deadline(), cancel.clone())
        .await
        .unwrap();
    assert_eq!(
        client.state(deadline(), cancel.clone()).await.unwrap(),
        State::VerifiedRules
    );
    let caps = client
        .capabilities(deadline(), cancel.clone())
        .await
        .unwrap();
    assert!(caps.public_protection && !caps.system_tun && !caps.allow_lan);
    let prepared = client.prepare(deadline(), cancel.clone()).await.unwrap();
    assert_eq!(
        client
            .apply(prepared, deadline(), cancel.clone())
            .await
            .unwrap(),
        State::VerifiedRules
    );
    assert_eq!(
        client.unlock(deadline(), cancel.clone()).await.unwrap(),
        State::NoManagedPolicy
    );
    for json in [
        "{\"version\":2,\"state\":\"unknown\"}".into(),
        "{\"version\":1,\"state\":\"future\"}".into(),
        "{\"version\":1,\"state\":42}".into(),
        "{\"version\":1,\"state\":\"unknown\",\"extra\":true}".into(),
        "x".repeat(65537),
    ] {
        *response.lock().await = json;
        assert_eq!(
            client.state(deadline(), cancel.clone()).await,
            Err(ClientError::Protocol)
        );
    }
    for (code, expected) in [
        ("busy", ClientError::Busy),
        ("timeout", ClientError::Timeout),
        ("authorization", ClientError::Denied),
        ("future", ClientError::Protocol),
    ] {
        *response.lock().await = format!("error:{{\"version\":1,\"error\":\"{code}\"}}");
        assert_eq!(
            client.state(deadline(), cancel.clone()).await,
            Err(expected)
        );
    }
    assert_eq!(
        client
            .stop(Instant::now() + Duration::from_millis(30), cancel.clone())
            .await,
        Err(ClientError::Deadline)
    );
    tokio::time::sleep(Duration::from_millis(180)).await;
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    let (stop, cancelled) = Cancellation::new();
    let operation = client.stop(deadline(), cancelled);
    let trigger = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        stop.send(true).unwrap();
    };
    let (result, _) = tokio::join!(operation, trigger);
    assert_eq!(result, Err(ClientError::Cancelled));
    tokio::time::sleep(Duration::from_millis(180)).await;
    assert_eq!(effects.load(Ordering::SeqCst), 2);
    let (already_stop, already_cancelled) = Cancellation::new();
    already_stop.send(true).unwrap();
    assert_eq!(
        client.stop(deadline(), already_cancelled).await,
        Err(ClientError::Cancelled)
    );
    assert_eq!(effects.load(Ordering::SeqCst), 2);
    let loss = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        server.release_name(SERVICE_NAME).await.unwrap();
        server.request_name(SERVICE_NAME).await.unwrap();
    };
    let (result, _) = tokio::join!(client.stop(deadline(), cancel.clone()), loss);
    assert_eq!(result, Err(ClientError::OwnerLost));
    tokio::time::sleep(Duration::from_millis(180)).await;
    assert_eq!(effects.load(Ordering::SeqCst), 3);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        client
            .capabilities(deadline(), cancel.clone())
            .await
            .unwrap_err(),
        ClientError::OwnerLost
    );
    let mut fresh = SystemClient::on_connection(bus.connect().await, deadline(), cancel.clone())
        .await
        .unwrap();
    assert!(
        fresh
            .capabilities(deadline(), cancel)
            .await
            .unwrap()
            .public_protection
    );
    drop(keep);
}

#[tokio::test]
async fn wrong_dbus_type_is_protocol_error() {
    use futures_util::StreamExt;
    let bus = Bus::new();
    let server = bus.connect().await;
    server.request_name(SERVICE_NAME).await.unwrap();
    let mut calls = zbus::MessageStream::from(&server);
    let (keep, cancel) = Cancellation::new();
    let mut client = SystemClient::on_connection(bus.connect().await, deadline(), cancel.clone())
        .await
        .unwrap();
    let respond = async {
        while let Some(Ok(message)) = calls.next().await {
            if message.header().message_type() == zbus::message::Type::MethodCall {
                server.reply(&message, &(true,)).await.unwrap();
                break;
            }
        }
    };
    let (result, _) = tokio::join!(client.state(deadline(), cancel), respond);
    assert_eq!(result, Err(ClientError::Protocol));
    drop(keep);
}

#[tokio::test]
async fn malformed_helper_error_is_protocol_error() {
    use futures_util::StreamExt;
    let bus = Bus::new();
    let server = bus.connect().await;
    server.request_name(SERVICE_NAME).await.unwrap();
    let mut calls = zbus::MessageStream::from(&server);
    let (keep, cancel) = Cancellation::new();
    let mut client = SystemClient::on_connection(bus.connect().await, deadline(), cancel.clone())
        .await
        .unwrap();
    let respond = async {
        while let Some(Ok(message)) = calls.next().await {
            if message.header().message_type() == zbus::message::Type::MethodCall {
                server
                    .reply_error(
                        &message,
                        "cloud.highpriority.yonik.Network1.Error",
                        &(42_u32,),
                    )
                    .await
                    .unwrap();
                break;
            }
        }
    };
    let (result, _) = tokio::join!(client.state(deadline(), cancel), respond);
    assert_eq!(result, Err(ClientError::Protocol));
    drop(keep);
}
