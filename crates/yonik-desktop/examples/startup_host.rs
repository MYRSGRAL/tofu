//! Owned software fixture: normal Host, настоящий private Secret Service, без ephemeral key.
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
use yonik_desktop::{DesktopHost, HostConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("Нет fixture root")?);
    if !root.is_absolute()
        || !root.join("startup-fixture.marker").is_file()
        || std::env::var_os("HOME").as_deref() != Some(root.as_os_str())
    {
        return Err("Требуется собственный fixture HOME с marker".into());
    }
    let engines = PathBuf::from(std::env::args_os().nth(2).ok_or("Нет engines")?);
    let delay = std::env::args().nth(3).as_deref() == Some("barrier");
    let observed_root = root.clone();
    let config = HostConfig::new(root.join("data"), engines)
        .with_system_subscription_policy(false)
        .with_startup_observer(move |phase| {
            if delay && phase == yonik_desktop::StartupPhase::Preferences {
                std::fs::write(observed_root.join("barrier-ready"), b"ready")
                    .expect("fixture marker");
                let deadline = Instant::now() + Duration::from_secs(8);
                while !observed_root.join("barrier-release").exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        });
    let host = DesktopHost::start(config, |snapshot| {
        let mut stdout = io::stdout().lock();
        let _ = writeln!(stdout, "{snapshot}");
        let _ = stdout.flush();
    })?;
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.len() > 65536 {
            return Err("Лимит fixture command".into());
        }
        if let Err(error) = host.dispatch(&line) {
            writeln!(
                io::stdout(),
                "{}",
                serde_json::json!({"dispatch_error":error.key()})
            )?;
        }
    }
    host.shutdown();
    if !host.wait_closed(Duration::from_secs(10)) {
        return Err("Нет shutdown ACK".into());
    }
    Ok(())
}
