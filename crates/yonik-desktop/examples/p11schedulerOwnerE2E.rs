//! Настоящее исчерпание локального owner allocator через production Bridge.
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
    let seed = match std::env::args().nth(2).as_deref() {
        Some("positive") => 7,
        Some("exhausted") => u64::MAX,
        _ => return Err("Нужен режим positive/exhausted".into()),
    };
    let config = yonik_desktop::HostConfig::ephemeral_scheduler_owner_example(
        data.clone(),
        root.join("engines"),
        *key,
        seed,
    )?;
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
