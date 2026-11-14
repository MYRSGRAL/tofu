//! P07: настоящий qrc Main/DesktopHost, частный bridge только для producer журнала.
#![deny(unsafe_code)]
#[cfg(all(feature = "gui", feature = "test-support"))]
#[allow(unsafe_code)]
#[path = "../src/bridge.rs"]
mod bridge;
#[cfg(all(feature = "gui", feature = "test-support"))]
#[path = "p07_gui/bridge_impl.rs"]
mod bridge_impl;
#[cfg(all(feature = "gui", feature = "test-support"))]
#[path = "../src/dispatch_error.rs"]
mod dispatch_error;
#[cfg(all(feature = "gui", feature = "test-support"))]
use std::{os::unix::fs::DirBuilderExt, path::PathBuf, sync::Mutex, time::Duration};
#[cfg(all(feature = "gui", feature = "test-support"))]
static PRODUCER: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
#[cfg(all(feature = "gui", feature = "test-support"))]
fn start_logs(log: yonik_linux::event_log::EventLog) {
    let mut producer = PRODUCER.lock().expect("producer lock");
    assert!(producer.is_none(), "один producer");
    *producer = Some(std::thread::spawn(move || {
        let start = std::time::Instant::now();
        for _ in 0..500 {
            log.test_fill();
            std::thread::sleep(Duration::from_millis(10));
        }
        let page = log.read(0, Default::default(), usize::MAX).expect("page");
        assert_eq!(page.events.len(), 256);
        assert!(page.evicted > 0);
        eprintln!(
            "P07 LOGS {}",
            serde_json::json!({"attempted":1002500,"batches":500,"period_ms":10,"duration_ms":start.elapsed().as_secs_f64()*1000.0,"page_len":page.events.len(),"evicted":page.evicted,"rejected":page.rejected})
        );
    }));
}
#[cfg(all(feature = "gui", feature = "test-support"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("root")?);
    let engines = PathBuf::from(std::env::args_os().nth(2).ok_or("cores")?);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(root.join("data"))?;
    bridge_impl::configure(
        yonik_desktop::HostConfig::new(root.join("data"), engines).with_ephemeral_key([77; 32]),
    )
    .map_err(|_| "config")?;
    let code = bridge::ffi::run_application(
        &root.join("data").to_str().ok_or("UTF8")?.into(),
        true,
        &"".into(),
        &"".into(),
        &"".into(),
    );
    if let Some(producer) = PRODUCER.lock().map_err(|_| "lock")?.take() {
        producer.join().map_err(|_| "producer panic")?;
    }
    if code != 0 {
        return Err(format!("Qt {code}").into());
    }
    Ok(())
}

#[cfg(not(all(feature = "gui", feature = "test-support")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("Для GUI fixture нужны features gui,test-support".into())
}
