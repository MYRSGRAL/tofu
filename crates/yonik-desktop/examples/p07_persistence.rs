//! Normal Host/private Secret Service; production bridge и UI smoke driver.
#![deny(unsafe_code)]
#[cfg(feature = "gui")]
#[allow(unsafe_code)]
#[path = "../src/bridge.rs"]
mod bridge;
#[cfg(feature = "gui")]
#[path = "../src/bridge_impl.rs"]
mod bridge_impl;
#[cfg(feature = "gui")]
#[path = "../src/dispatch_error.rs"]
mod dispatch_error;
#[cfg(feature = "gui")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::PathBuf::from(std::env::args_os().nth(1).ok_or("Нет root")?);
    if std::env::var_os("HOME").as_deref() != Some(root.as_os_str())
        || !std::env::var("DBUS_SESSION_BUS_ADDRESS")?.starts_with("unix:")
        || std::fs::read_to_string(root.join("nonce"))? != std::env::var("P06_PRIVATE_NONCE")?
    {
        return Err("Private guard".into());
    }
    let script = std::env::args().nth(2).ok_or("Нет script")?;
    let trace = std::env::args().nth(3).ok_or("Нет trace")?;
    let data = root.join("data");
    bridge_impl::configure(yonik_desktop::HostConfig::new(
        data.clone(),
        root.join("no-engines"),
    ))
    .map_err(|_| "Конфигурация уже задана")?;
    let code = bridge::ffi::run_application(
        &data.to_str().ok_or("UTF8")?.into(),
        true,
        &"".into(),
        &script.into(),
        &trace.into(),
    );
    if code != 0 {
        return Err(format!("Qt exit {code}").into());
    }
    Ok(())
}
#[cfg(not(feature = "gui"))]
fn main() {
    eprintln!("Требуется gui");
    std::process::exit(1);
}
