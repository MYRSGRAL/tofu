//! Только fixture: exact source142 API, same-UID private namespace и Secret Service.
use std::{collections::HashMap, fs, os::unix::fs::MetadataExt, path::Path};
use yonik_linux::{EncryptedSubscriptionRepository, SubscriptionService, subscription_master_key};

const SOURCE: &str = "14229843f9a11bf98ec4f5992df00f3247102ea2";
const HOME: &str = "/home/fixture";
const DATA: &str = "/home/fixture/storage";
const BUS: &str = "unix:path=/home/fixture/runtime/bus";

fn guarded() -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let uid = rustix::process::getuid().as_raw();
    if uid == 0
        || rustix::process::geteuid().as_raw() != uid
        || std::env::var_os("YONIK_CREDENTIAL_VM").is_some()
        || std::env::var("HOME").as_deref() != Ok(HOME)
        || std::env::var("DBUS_SESSION_BUS_ADDRESS").as_deref() != Ok(BUS)
    {
        return Err("Неверная own-prefix среда".into());
    }
    let path = Path::new("/home/fixture/runtime/witness-guard.json");
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o7777 != 0o600
        || metadata.len() > 8192
    {
        return Err("Неверный guard file".into());
    }
    let guard: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    let nonce = guard["nonce"].as_str().ok_or("Нет nonce guard")?;
    if guard["source"].as_str() != Some(SOURCE)
        || guard["uid"].as_u64() != Some(u64::from(uid))
        || nonce.len() != 64
        || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
        || std::env::var("A4_WITNESS_NONCE").ok().as_deref() != Some(nonce)
    {
        return Err("Неверный source/UID/nonce guard".into());
    }
    for name in ["user", "mnt", "pid", "net"] {
        let current = fs::read_link(format!("/proc/self/ns/{name}"))?;
        let current = current.to_str().ok_or("Некорректный namespace")?;
        if guard["namespace"][name].as_str() != Some(current)
            || guard["host_namespace"][name].as_str() == Some(current)
        {
            return Err("Namespace не принадлежит own stand".into());
        }
    }
    for (path, label) in [("/", "root"), (DATA, "data")] {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir()
            || metadata.uid() != uid
            || guard[label]["dev"].as_u64() != Some(metadata.dev())
            || guard[label]["ino"].as_u64() != Some(metadata.ino())
        {
            return Err("Root/data identity mismatch".into());
        }
    }
    for directory in [HOME, DATA, "/home/fixture/runtime"] {
        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o7777 != 0o700 {
            return Err("Own directory permissions mismatch".into());
        }
    }
    Ok(guard)
}

async fn identity(
    connection: &zbus::Connection,
    guard: &serde_json::Value,
) -> Result<(), &'static str> {
    let owner: String = connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetNameOwner",
            &("org.freedesktop.secrets",),
        )
        .await
        .map_err(|_| "Provider owner недоступен")?
        .body()
        .deserialize()
        .map_err(|_| "Неверный owner")?;
    for (method, field) in [
        ("GetConnectionUnixUser", "uid"),
        ("GetConnectionUnixProcessID", "provider_pid"),
    ] {
        let value: u32 = connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                method,
                &(owner.as_str(),),
            )
            .await
            .map_err(|_| "Provider identity недоступна")?
            .body()
            .deserialize()
            .map_err(|_| "Неверная provider identity")?;
        if guard[field].as_u64() != Some(u64::from(value)) {
            return Err("Provider UID/PID mismatch");
        }
    }
    if guard["provider_owner"].as_str() != Some(owner.as_str()) {
        return Err("Provider owner изменился");
    }
    let attributes = HashMap::from([
        ("application", "cloud.highpriority.tofu"),
        ("purpose", "subscription-master-key"),
        ("schema", "1"),
    ]);
    let (unlocked, locked): (
        Vec<zbus::zvariant::OwnedObjectPath>,
        Vec<zbus::zvariant::OwnedObjectPath>,
    ) = connection
        .call_method(
            Some("org.freedesktop.secrets"),
            "/org/freedesktop/secrets",
            Some("org.freedesktop.Secret.Service"),
            "SearchItems",
            &(attributes,),
        )
        .await
        .map_err(|_| "Item search отказал")?
        .body()
        .deserialize()
        .map_err(|_| "Неверный item search")?;
    if unlocked.len() != 1
        || !locked.is_empty()
        || guard["item_path"].as_str() != Some(unlocked[0].as_str())
    {
        return Err("Нет единственного прежнего unlocked item");
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let guard = guarded()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        let connection = zbus::Connection::session()
            .await
            .map_err(|_| "Private bus недоступен")?;
        identity(&connection, &guard).await?;
        // Единственный путь ключа — production Secret Service backend; create=false.
        let key = subscription_master_key(false)
            .await
            .map_err(|_| "Production custody отказал")?;
        let repository = EncryptedSubscriptionRepository::open(
            Path::new(DATA).join("subscriptions.sqlite"),
            key,
        )
        .await
        .map_err(|_| "Production repository отказал")?;
        let service = SubscriptionService::new(repository);
        let page = service
            .sources_page(0)
            .await
            .map_err(|_| "Production metadata отказал")?;
        let mut original_matches = page.total == 2 && page.sources.len() == 2;
        let mut revision_matches = true;
        let mut metadata_matches = original_matches;
        for (id, name) in [(1, "text.txt"), (2, "file.txt")] {
            let expected =
                fs::read(Path::new(DATA).join(name)).map_err(|_| "Fixture input недоступен")?;
            let snapshot = service
                .offline(id)
                .await
                .map_err(|_| "Production offline отказал")?;
            original_matches &= snapshot.report.original().expose().as_bytes() == expected;
            revision_matches &= snapshot.revision == 1;
            metadata_matches &= snapshot.source_generation == 1
                && snapshot.current_source_generation == 1
                && snapshot.report.servers.len() == 1
                && snapshot.report.native_json.is_none();
            let source = service
                .source_summary(id)
                .await
                .map_err(|_| "Source metadata отказал")?;
            metadata_matches &= source.id == id
                && !source.name.trim().is_empty()
                && !source.hwid_enabled
                && service
                    .source_is_local(id)
                    .await
                    .map_err(|_| "Source kind отказал")?;
        }
        service
            .close()
            .await
            .map_err(|_| "Production close отказал")?;
        identity(&connection, &guard).await?;
        if !(original_matches && revision_matches && metadata_matches) {
            return Err("Production API assertions отказали");
        }
        Ok(
            serde_json::json!({"source": SOURCE, "original_matches": true,
            "revision_matches": true, "metadata_matches": true, "count": 2,
            "same_unlocked_item": true, "provider_uid_pid_verified": true}),
        )
    });
    match result {
        Ok(value) => {
            println!("{value}");
            Ok(())
        }
        Err(reason) => Err(reason.into()),
    }
}
