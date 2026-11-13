//! Компонентный P-05: настоящий DesktopHost, только synthetic store и stdin control.
use serde_json::{Value, json};
use std::{
    error::Error,
    io::{BufRead, Write},
    num::NonZeroU16,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
use yonik_desktop::{DesktopHost, HostConfig};
use yonik_linux::{Cancellation, EncryptedSubscriptionRepository, SubscriptionService};
use zeroize::Zeroizing;
type Result<T> = std::result::Result<T, Box<dyn Error>>;
fn wait(rx: &mpsc::Receiver<String>, predicate: impl Fn(&Value) -> bool) -> Result<Value> {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let value: Value =
            serde_json::from_str(&rx.recv_timeout(end.saturating_duration_since(Instant::now()))?)?;
        if predicate(&value) {
            return Ok(value);
        }
    }
}
struct Guard(DesktopHost);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.shutdown();
        let _ = self.0.wait_closed(Duration::from_secs(10));
    }
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 7 {
        return Err("data engines engine profile probe port".into());
    }
    let data = PathBuf::from(&args[1]);
    let mut key = [0; 32];
    getrandom::getrandom(&mut key)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let id = runtime.block_on(async {
        let database = data.join("subscriptions.sqlite");
        let backup = data.join("backup.sqlite");
        EncryptedSubscriptionRepository::open(database.clone(), Zeroizing::new(key))
            .await?
            .close()
            .await?;
        EncryptedSubscriptionRepository::backup_closed(
            database,
            backup.clone(),
            Zeroizing::new(key),
        )
        .await?;
        let service = SubscriptionService::new(
            EncryptedSubscriptionRepository::open(
                data.join("subscriptions.sqlite"),
                Zeroizing::new(key),
            )
            .await?,
        );
        let report = yonik_import::import_bytes(args[4].as_bytes().to_vec())?;
        let (_source, cancel) = Cancellation::new();
        let id = service
            .approve_local_import("P05 synthetic".into(), &report, cancel, Some(backup))
            .await?;
        service.close().await?;
        Ok::<_, Box<dyn Error>>(id)
    })?;
    drop(runtime);
    let (tx, rx) = mpsc::channel();
    let host = Guard(DesktopHost::start(
        HostConfig::new(data, PathBuf::from(&args[2]))
            .with_ephemeral_key(key)
            .with_listen_port(args[6].parse::<NonZeroU16>()?),
        move |v| {
            let _ = tx.send(v);
        },
    )?);
    let send = |v: Value| -> Result<()> {
        host.0.dispatch(&v.to_string())?;
        Ok(())
    };
    wait(&rx, |v| v["ready"] == true)?;
    send(json!({"command":"set_engine","engine":args[3]}))?;
    wait(&rx, |v| v["engine"] == args[3])?;
    send(json!({"command":"select_source","id":id}))?;
    let page = wait(&rx, |v| v["source_id"] == id && v["total"] == 1)?;
    send(
        json!({"command":"select_profile","id":page["profiles"][0]["id"],"revision":page["revision"]}),
    )?;
    wait(&rx, |v| v["selected_profile"].is_i64())?;
    send(json!({"command":"set_probe_target","url":args[5],"expected_status":204}))?;
    wait(&rx, |v| v["probe_custom"] == true)?;
    writeln!(std::io::stdout(), "ready")?;
    std::io::stdout().flush()?;
    for line in std::io::stdin().lock().lines() {
        let command = line?;
        match command.as_str() {
            "connect" => {
                send(json!({"command":"connect"}))?;
                wait(&rx, |v| v["status"] == "Connected")?;
            }
            "stop" => {
                send(json!({"command":"stop"}))?;
                wait(&rx, |v| {
                    v["status"] == "Disconnected" && v["can_stop"] == false
                })?;
            }
            "logs" => {
                send(json!({"command":"logs_read","after":"0","source":"all","request_id":"1"}))?;
                let page = wait(&rx, |v| v["logs"]["request_id"] == "1")?;
                writeln!(std::io::stdout(), "{}", page["logs"])?;
                std::io::stdout().flush()?;
                continue;
            }
            "shutdown" => {
                host.0.shutdown();
                if !host.0.wait_closed(Duration::from_secs(10)) {
                    return Err("Нет closed ACK".into());
                }
                writeln!(std::io::stdout(), "closed")?;
                return Ok(());
            }
            _ => return Err("Неизвестная команда стенда".into()),
        }
        writeln!(std::io::stdout(), "{command}")?;
        std::io::stdout().flush()?;
    }
    Ok(())
}
