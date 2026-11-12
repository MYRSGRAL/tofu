//! Настоящий scheduler service failure через общий production Bridge.
#![deny(unsafe_code)]
#[allow(unsafe_code)]
#[path = "../src/bridge.rs"]
mod bridge;
#[path = "../src/bridge_impl.rs"]
mod bridge_impl;
#[path = "../src/dispatch_error.rs"]
mod dispatch_error;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{os::unix::fs::DirBuilderExt, path::PathBuf};
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("Нужен отдельный root")?);
    std::fs::DirBuilder::new().mode(0o700).create(&root)?;
    let data = root.join("data");
    std::fs::DirBuilder::new().mode(0o700).create(&data)?;
    let mut key = zeroize::Zeroizing::new([0_u8; 32]);
    getrandom::getrandom(key.as_mut()).map_err(|_| "Ошибка random")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let repo = yonik_linux::EncryptedSubscriptionRepository::open(
            data.join("subscriptions.sqlite"),
            zeroize::Zeroizing::new(*key),
        )
        .await?;
        repo.create_source(b"{}".to_vec()).await?;
        Ok::<_, yonik_linux::RepositoryError>(())
    })?;
    let status = std::process::Command::new("python3")
        .args(["-c", "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('UPDATE sources SET payload=zeroblob(40)'); c.commit()"])
        .arg(data.join("subscriptions.sqlite")).status()?;
    if !status.success() {
        return Err("Повреждение synthetic payload не выполнено".into());
    }
    drop(runtime);
    let config =
        yonik_desktop::HostConfig::new(data.clone(), root.join("engines")).with_ephemeral_key(*key);
    bridge_impl::configure(config).map_err(|_| "Конфигурация уже задана")?;
    let code = bridge::ffi::run_application(
        &data.to_str().ok_or("UTF-8")?.into(),
        true,
        &"".into(),
        &"".into(),
        &"".into(),
    );
    if code != 0 {
        return Err(format!("Qt: {code}").into());
    }
    Ok(())
}
