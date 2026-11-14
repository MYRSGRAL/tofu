//! Узкий normal-host witness; только собственный private runtime harness.
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};
use yonik_desktop::{DesktopHost, HostConfig};
const ORIGINAL: &str = "trojan://p06-synthetic@example.invalid:443#p06";
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn wait(rx: &mpsc::Receiver<String>, gate: impl Fn(&Value) -> bool) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let value: Value = serde_json::from_str(
            &rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))?,
        )?;
        eprintln!(
            "{}",
            json!({"ready":value["ready"],"closed":value["closed"],"revision":value["revision"],"sources_total":value["sources_total"],"total":value["total"],"maintenance_ack":value["maintenance_ack"],"error":value["error"],"pending":value["pending"].is_object()})
        );
        if gate(&value) {
            return Ok(value);
        }
    }
}
fn start(dir: &Path) -> Result<(DesktopHost, mpsc::Receiver<String>, Value)> {
    let (tx, rx) = mpsc::channel();
    let host = DesktopHost::start(
        HostConfig::new(dir.into(), dir.join("no-engines")),
        move |s| {
            let _ = tx.send(s);
        },
    )?;
    let ready = wait(&rx, |v| v["ready"] == true)?;
    Ok((host, rx, ready))
}
fn close(host: DesktopHost) -> Result<()> {
    host.shutdown();
    if !host.wait_closed(Duration::from_secs(5)) {
        return Err("Нет Closed ACK".into());
    }
    println!("{}", json!({"closed_ack":true}));
    Ok(())
}
fn maintenance(
    host: &DesktopHost,
    rx: &mpsc::Receiver<String>,
    mut request: Value,
    revision: &Value,
    phase: &str,
) -> Result<Value> {
    request["command"] = json!("maintenance_start");
    request["revision"] = revision.clone();
    host.dispatch(&request.to_string())?;
    let id = request["request_id"].clone();
    let value = wait(rx, |v| {
        v["maintenance_ack"]["request_id"] == id
            && matches!(
                v["maintenance_ack"]["phase"].as_str(),
                Some("complete" | "failed")
            )
    })?;
    if value["maintenance_ack"]["phase"] != phase {
        return Err("Неожиданный matching maintenance ACK".into());
    }
    Ok(value)
}
fn verify(dir: &Path) -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            use yonik_linux::{
                EncryptedSubscriptionRepository, SubscriptionService, subscription_master_key,
            };
            let key = subscription_master_key(false).await?;
            let service = SubscriptionService::new(
                EncryptedSubscriptionRepository::open(dir.join("subscriptions.sqlite"), key)
                    .await?,
            );
            let sources = service.sources_page(0).await?;
            let snapshot = service.offline(1).await?;
            let good = sources.total == 1
                && sources.sources.len() == 1
                && sources.sources[0].name == "P06 synthetic"
                && snapshot.revision == 1
                && snapshot.report.original().expose() == ORIGINAL;
            service.close().await?;
            if !good {
                return Err("Не совпали metadata/original/revision".into());
            }
            println!("{}", json!({"original_matches":true,"metadata_matches":true,"revision":snapshot.revision,"source_count":sources.total}));
            Ok(())
        })
}
fn run() -> Result<()> {
    let root = std::path::PathBuf::from(std::env::args_os().nth(1).ok_or("Нет root")?);
    let nonce = std::env::var("P06_PRIVATE_NONCE")?;
    if nonce.len() != 32
        || std::fs::read_to_string(root.join("nonce"))? != nonce
        || std::env::var("HOME")? != root.to_str().ok_or("root")?
        || !std::env::var("DBUS_SESSION_BUS_ADDRESS")?.starts_with("unix:")
    {
        return Err("Private guard".into());
    }
    let mode = std::env::args().nth(2).ok_or("Нет mode")?;
    if mode == "verify" {
        verify(&root.join("restored"))?;
    } else if mode == "reopen" {
        if !root.join("restored/subscriptions.sqlite").is_file() {
            return Err("Нет restore target; normal create запрещён".into());
        }
        let (host, _, ready) = start(&root.join("restored"))?;
        let good = ready["sources_total"] == 1 && ready["revision"] == 1 && ready["total"] == 1;
        close(host)?;
        if !good {
            return Err("Fresh ready не содержит профиль".into());
        }
    } else if matches!(mode.as_str(), "backup" | "restore" | "negative" | "seed") {
        let live = root.join("live");
        if mode != "seed" && !live.join("subscriptions.sqlite").is_file() {
            return Err("Нет существующей live DB".into());
        }
        let (host, rx, initial) = start(&live)?;
        if mode == "seed" {
            maintenance(
                &host,
                &rx,
                json!({"request_id":100,"operation":"backup","destination":root.join("copies/schema3.sqlite")}),
                &initial["revision"],
                "complete",
            )?;
            host.dispatch(
                &json!({"command":"import_text","name":"P06 synthetic","text":ORIGINAL})
                    .to_string(),
            )?;
            wait(&rx, |v| v["pending"].is_object())?;
            host.dispatch(r#"{"command":"approve_refresh"}"#)?;
            wait(&rx, |v| v["sources_total"] == 1 && v["revision"] == 1)?;
            close(host)?;
            verify(&live)?;
            println!("{}", json!({"stage":mode,"passed":true}));
            return Ok(());
        }
        let request = match mode.as_str() {
            "backup" => {
                json!({"request_id":101,"operation":"backup","destination":root.join("copies/backup.sqlite")})
            }
            "restore" => {
                json!({"request_id":102,"operation":"restore","source":root.join("copies/backup.sqlite"),"destination":root.join("restored/subscriptions.sqlite")})
            }
            "negative" => {
                json!({"request_id":103,"operation":"restore","source":root.join("missing.sqlite"),"destination":root.join("negative/absent.sqlite")})
            }
            _ => return Err("Неизвестный режим".into()),
        };
        maintenance(
            &host,
            &rx,
            request,
            &initial["revision"],
            if mode == "negative" {
                "failed"
            } else {
                "complete"
            },
        )?;
        if root.join("negative/absent.sqlite").exists() {
            return Err("Negative опубликовал файл".into());
        }
        close(host)?;
        verify(&live)?;
    } else {
        return Err("Неизвестный mode".into());
    }
    println!("{}", json!({"stage":mode,"passed":true}));
    Ok(())
}
fn main() {
    if run().is_err() {
        eprintln!("P06 witness gate failed (без credential payload)");
        std::process::exit(1);
    }
}
