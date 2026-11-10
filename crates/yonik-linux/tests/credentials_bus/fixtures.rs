use std::{
    collections::HashMap,
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const SERVICE: &str = "/org/freedesktop/secrets";
const ITEM: &str = "/org/freedesktop/secrets/item/test";
const COLLECTION: &str = "/org/freedesktop/secrets/collection/test";
fn path(value: &str) -> OwnedObjectPath {
    value.try_into().unwrap()
}
type Calls = Arc<Mutex<Vec<&'static str>>>;

struct Service {
    case: String,
    calls: Calls,
}
#[zbus::interface(name = "org.freedesktop.Secret.Service")]
impl Service {
    async fn open_session(
        &self,
        algorithm: &str,
        _input: Value<'_>,
    ) -> (OwnedValue, OwnedObjectPath) {
        assert_eq!(algorithm, "dh-ietf1024-sha256-aes128-cbc-pkcs7");
        self.calls.lock().unwrap().push("OpenSession");
        (
            Value::from(vec![2u8]).try_to_owned().unwrap(),
            path("/org/freedesktop/secrets/session/test"),
        )
    }
    fn search_items(
        &self,
        attributes: HashMap<String, String>,
    ) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
        assert_eq!(
            attributes.get("application").unwrap(),
            "cloud.highpriority.tofu"
        );
        assert_eq!(
            attributes.get("purpose").unwrap(),
            "subscription-master-key"
        );
        assert_eq!(attributes.get("schema").unwrap(), "1");
        self.calls.lock().unwrap().push("SearchItems");
        match self.case.as_str() {
            "missing" | "collection" => (vec![], vec![]),
            "locked" | "cancel" => (vec![], vec![path(ITEM)]),
            "ambiguous" => (vec![path(ITEM)], vec![path(ITEM)]),
            _ => (vec![path(ITEM)], vec![]),
        }
    }
    fn unlock(&self, _objects: Vec<OwnedObjectPath>) -> (Vec<OwnedObjectPath>, OwnedObjectPath) {
        self.calls.lock().unwrap().push("Unlock");
        (vec![], path("/org/freedesktop/secrets/prompt/cancelled"))
    }
    fn set_alias(&self, _alias: &str, _collection: OwnedObjectPath) {
        self.calls.lock().unwrap().push("SetAlias");
    }
    fn create_collection(
        &self,
        _properties: HashMap<String, OwnedValue>,
        _alias: &str,
    ) -> zbus::fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        self.calls.lock().unwrap().push("CreateCollection");
        Err(zbus::fdo::Error::Failed("Запись запрещена".into()))
    }
    fn read_alias(&self, alias: &str) -> OwnedObjectPath {
        assert_eq!(alias, "default");
        self.calls.lock().unwrap().push("ReadAlias");
        path(COLLECTION)
    }
}
struct Collection {
    calls: Calls,
}
#[zbus::interface(name = "org.freedesktop.Secret.Collection")]
impl Collection {
    fn create_item(
        &self,
        _properties: HashMap<String, OwnedValue>,
        _secret: (OwnedObjectPath, Vec<u8>, Vec<u8>, String),
        _replace: bool,
    ) -> zbus::fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        self.calls.lock().unwrap().push("CreateItem");
        Err(zbus::fdo::Error::Failed("Запись запрещена".into()))
    }
    #[zbus(property)]
    fn locked(&self) -> bool {
        self.calls.lock().unwrap().push("Locked");
        true
    }
}
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.Secret.Error")]
enum Failure {
    IsLocked(String),
    Failed(String),
}
#[derive(serde::Serialize, zbus::zvariant::Type)]
struct Secret(OwnedObjectPath, Vec<u8>, Vec<u8>, String);
struct Item {
    case: String,
    calls: Calls,
}
#[zbus::interface(name = "org.freedesktop.Secret.Item")]
impl Item {
    fn get_secret(&self, session: OwnedObjectPath) -> Result<Secret, Failure> {
        self.calls.lock().unwrap().push("GetSecret");
        match self.case.as_str() {
            "race" => Err(Failure::IsLocked("Заблокировано после поиска".into())),
            "failing" => Err(Failure::Failed("Отказ чтения".into())),
            _ => Ok(Secret(
                session,
                if self.case == "cipher" {
                    vec![0; 16]
                } else {
                    vec![]
                },
                if self.case == "cipher" {
                    vec![0; 15]
                } else {
                    vec![0; 16]
                },
                "application/octet-stream".into(),
            )),
        }
    }
}

pub struct Fixture {
    pub address: String,
    pub runtime: std::path::PathBuf,
    _daemon: Daemon,
    _connection: zbus::Connection,
    calls: Calls,
    case: String,
}
impl Fixture {
    pub async fn start(case: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let runtime = std::env::temp_dir().join(format!(
            "yonik-credentials-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&runtime).unwrap();
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Нет servicedir: отсутствие имени не активирует настоящий keyring.
        let config = runtime.join("bus.conf");
        std::fs::write(&config, r#"<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>"#).unwrap();
        let daemon = Command::new("dbus-daemon")
            .arg("--config-file")
            .arg(&config)
            .args(["--nofork", "--nopidfile", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut daemon = Daemon {
            child: daemon,
            runtime: runtime.clone(),
        };
        let address = daemon_address(&mut daemon.child).await.unwrap();
        let calls = Calls::default();
        let mut builder = zbus::connection::Builder::address(address.as_str()).unwrap();
        if case != "absent" {
            builder = builder
                .name("org.freedesktop.secrets")
                .unwrap()
                .serve_at(
                    SERVICE,
                    Service {
                        case: case.into(),
                        calls: calls.clone(),
                    },
                )
                .unwrap()
                .serve_at(
                    COLLECTION,
                    Collection {
                        calls: calls.clone(),
                    },
                )
                .unwrap()
                .serve_at(
                    ITEM,
                    Item {
                        case: case.into(),
                        calls: calls.clone(),
                    },
                )
                .unwrap();
        }
        let connection = tokio::time::timeout(std::time::Duration::from_secs(5), builder.build())
            .await
            .unwrap()
            .unwrap();
        Self {
            address,
            runtime,
            _daemon: daemon,
            _connection: connection,
            calls,
            case: case.into(),
        }
    }
    pub fn assert_readonly(&self) {
        let calls = self.calls.lock().unwrap();
        if self.case != "absent" {
            assert!(calls.contains(&"SearchItems"));
        }
        assert!(calls.iter().all(|call| matches!(
            *call,
            "OpenSession" | "SearchItems" | "ReadAlias" | "Locked" | "GetSecret"
        )));
    }
}
struct Daemon {
    child: Child,
    runtime: std::path::PathBuf,
}
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        std::fs::remove_dir_all(&self.runtime).unwrap();
    }
}

async fn daemon_address(child: &mut Child) -> std::io::Result<String> {
    let stdout = tokio::process::ChildStdout::from_std(child.stdout.take().unwrap())?;
    let mut reader = tokio::io::BufReader::new(stdout.take(4097));
    let mut address = String::new();
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        reader.read_line(&mut address),
    )
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "адрес D-Bus не получен"))??;
    if bytes == 0 || bytes > 4096 || !address.ends_with('\n') || !address.starts_with("unix:") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "неверный адрес fixture D-Bus",
        ));
    }
    Ok(address.trim().to_owned())
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    #[test]
    fn cipher_failure_is_not_a_valid_aes_block() {
        let item = Item {
            case: "cipher".into(),
            calls: Calls::default(),
        };
        let secret = item.get_secret(path("/session/test")).unwrap();
        assert_eq!(secret.1.len(), 16, "IV корректного размера");
        assert_ne!(
            secret.2.len() % 16,
            0,
            "ciphertext должен гарантированно отклоняться"
        );
    }
    #[tokio::test(flavor = "current_thread")]
    async fn startup_read_is_interruptible_and_guard_reaps_child() {
        let runtime =
            std::env::temp_dir().join(format!("yonik-credentials-startup-{}", std::process::id()));
        std::fs::create_dir(&runtime).unwrap();
        let child = Command::new("python3")
            .args([
                "-c",
                "import time; time.sleep(0.3); print('unix:path=/never-connect')",
            ])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut daemon = Daemon {
            child,
            runtime: runtime.clone(),
        };
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            daemon_address(&mut daemon.child),
        )
        .await;
        drop(daemon);
        assert!(
            result.is_err(),
            "отсутствующая строка адреса не блокирует runtime"
        );
        assert!(!runtime.exists());
        assert!(
            !std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "child reaped"
        );
    }
}
