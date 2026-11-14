//! Изолированный P-03 с production bridge/host/qrc; без сетевых команд.
#![deny(unsafe_code)]
#[cfg(all(feature = "gui", feature = "test-support"))]
#[allow(unsafe_code)]
#[path = "../src/bridge.rs"]
mod bridge;
#[cfg(all(feature = "gui", feature = "test-support"))]
#[path = "../src/bridge_impl.rs"]
mod bridge_impl;
#[cfg(all(feature = "gui", feature = "test-support"))]
#[path = "../src/dispatch_error.rs"]
mod dispatch_error;
#[cfg(all(feature = "gui", feature = "test-support"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{os::unix::fs::DirBuilderExt, path::PathBuf};
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("root")?);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(&root)?;
    bridge_impl::configure(
        yonik_desktop::HostConfig::new(root.clone(), root.join("no-engines"))
            .with_ephemeral_key([77; 32]),
    )
    .map_err(|_| "config")?;
    let code = bridge::ffi::run_application(
        &root.to_str().ok_or("UTF8")?.into(),
        false,
        &"".into(),
        &"".into(),
        &"".into(),
    );
    if code != 0 {
        return Err(format!("Qt {code}").into());
    }
    Ok(())
}
#[cfg(not(all(feature = "gui", feature = "test-support")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("Нужны gui,test-support".into())
}
