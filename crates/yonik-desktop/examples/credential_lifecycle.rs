//! Запускать только в одноразовой VM с отдельной private bus.
use std::{path::PathBuf, sync::mpsc, time::Duration};
use yonik_desktop::{DesktopHost, HostConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("YONIK_CREDENTIAL_VM").as_deref() != Ok("disposable-v1")
        || std::fs::read_to_string("/sys/class/dmi/id/sys_vendor")?.trim() != "QEMU"
    {
        return Err("Требуется одноразовая QEMU VM".into());
    }
    if std::env::args().nth(2).as_deref() == Some("diagnose") {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        match runtime.block_on(yonik_linux::subscription_master_key(false)) {
            Ok(_) => println!("credential_available"),
            Err(error) => println!("credential_error:{error:?}"),
        }
        return Ok(());
    }
    let directory = PathBuf::from(std::env::args_os().nth(1).ok_or("Нет пути fixture")?);
    let (sender, receiver) = mpsc::channel();
    let host = DesktopHost::start(
        HostConfig::new(directory.clone(), directory.join("engines")),
        move |event| {
            let _ = sender.send(event);
        },
    )?;
    let deadline = std::time::Instant::now() + Duration::from_secs(22);
    let outcome = loop {
        let event =
            receiver.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))?;
        let value: serde_json::Value = serde_json::from_str(&event)?;
        let snapshot = &value;
        if snapshot["ready"] == true {
            break "ready";
        }
        if snapshot["closed"] == true || snapshot["error"].is_string() {
            break match snapshot["error"].as_str() {
                Some("keyring_missing") => "keyring_missing",
                Some("keyring_locked") => "keyring_locked",
                _ => "other_failure",
            };
        }
    };
    host.shutdown();
    if !host.wait_closed(Duration::from_secs(5)) {
        return Err("Нет bounded shutdown ACK".into());
    }
    println!("{outcome}");
    Ok(())
}
