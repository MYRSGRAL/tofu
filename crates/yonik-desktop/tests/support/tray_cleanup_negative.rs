//! Только отрицательный Qt-стенд: настоящий DesktopHost и удержанные ресурсы.
#![deny(unsafe_code)]

#[allow(unsafe_code)]
#[path = "../../src/bridge.rs"]
mod bridge;

mod bridge_impl {
    use crate::bridge::ffi;
    use cxx_qt::Threading;
    use cxx_qt_lib::QString;
    use std::{
        fs::OpenOptions,
        io::Write,
        pin::Pin,
        sync::{Arc, OnceLock},
    };
    use yonik_desktop::{DesktopHost, HostConfig};

    pub static CONFIG: OnceLock<HostConfig> = OnceLock::new();
    pub static HOST: OnceLock<Arc<DesktopHost>> = OnceLock::new();

    pub struct DesktopBridgeRust {
        pub(crate) snapshot_json: QString,
    }
    impl Default for DesktopBridgeRust {
        fn default() -> Self {
            Self {
                snapshot_json: QString::from("{}"),
            }
        }
    }
    impl ffi::DesktopBridge {
        pub fn initialize(self: Pin<&mut Self>) {
            let config = CONFIG.get().expect("trusted config");
            let backend_trace = config
                .data_dir()
                .parent()
                .expect("root")
                .join("backend.jsonl");
            let diagnostics = config
                .data_dir()
                .parent()
                .expect("root")
                .join("diagnostics.json");
            let thread = self.qt_thread();
            let host = DesktopHost::start(config.clone(), move |snapshot| {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&backend_trace)
                    .expect("backend trace");
                writeln!(file, "{snapshot}").expect("backend snapshot");
                let value: serde_json::Value = serde_json::from_str(&snapshot).expect("actual DTO");
                if value["error"] == "cleanup_failed" {
                    let exported = yonik_desktop::diagnostics::export_snapshot(&snapshot)
                        .expect("diagnostic metadata");
                    std::fs::write(&diagnostics, exported).expect("diagnostics");
                }
                thread
                    .queue(move |mut object| {
                        object.as_mut().set_snapshot_json(QString::from(&snapshot));
                    })
                    .expect("Qt queue");
            })
            .expect("actual DesktopHost");
            assert!(HOST.set(Arc::new(host)).is_ok());
        }
        pub fn dispatch(self: Pin<&mut Self>, request: QString) {
            // QML visibleChanged может прийти до Component.onCompleted/initialize.
            if let Some(host) = HOST.get() {
                if let Err(error) = host.dispatch(&request.to_string()) {
                    log::error!("Отказ команды отрицательного стенда: {error}");
                }
            }
        }
        pub fn shutdown(self: Pin<&mut Self>) {
            if let Some(host) = HOST.get() {
                host.shutdown();
            }
        }
        pub fn copy_diagnostics_v2(self: Pin<&mut Self>) -> bool {
            let Some(metadata) =
                yonik_desktop::diagnostics::export_snapshot_v2(&self.snapshot_json().to_string())
            else {
                return false;
            };
            ffi::copy_diagnostic_metadata(&QString::from(&metadata));
            true
        }

        pub fn copy_diagnostics(self: Pin<&mut Self>) -> bool {
            let Some(metadata) =
                yonik_desktop::diagnostics::export_snapshot(&self.snapshot_json().to_string())
            else {
                return false;
            };
            ffi::copy_diagnostic_metadata(&QString::from(&metadata));
            true
        }
    }
}

use cxx_qt_lib::QString;
use std::{
    io::Read,
    num::NonZeroU16,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use yonik_desktop::HostConfig;
use yonik_engine::Engine;
use yonik_linux::BinarySnapshot;

struct CleanupGuard(Arc<AtomicBool>);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
        if let Some(host) = bridge_impl::HOST.get() {
            host.shutdown();
            let _ = host.wait_closed(std::time::Duration::from_secs(5));
        }
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let argument = |name: &str| -> String {
        arguments
            .windows(2)
            .find(|pair| pair[0] == name)
            .expect("fixture argument")[1]
            .clone()
    };
    let data = PathBuf::from(argument("--data-dir"));
    let engines = PathBuf::from(argument("--engine-dir"));
    let script = argument("--smoke-script");
    let trace = argument("--smoke-trace");
    let port: NonZeroU16 = argument("--listen-port").parse().expect("port");
    // Никакой альтернативы pin и никакой подмены исполняемого файла.
    let _preverified = BinarySnapshot::load(
        &engines.join("xray-26.10.31/xray"),
        "cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1"
            .parse()
            .expect("pin"),
        Engine::Xray,
    )
    .expect("preverified pinned Xray");
    let mut key = [0; 32];
    getrandom::getrandom(&mut key).expect("ephemeral key");
    let fault = Arc::new(AtomicBool::new(true));
    let _cleanup = CleanupGuard(Arc::clone(&fault));
    let config = HostConfig::new(data.clone(), engines)
        .with_ephemeral_key(key)
        .with_listen_port(port)
        .with_cleanup_fault(Arc::clone(&fault))
        .expect("ephemeral-only fault");
    assert!(bridge_impl::CONFIG.set(config).is_ok());
    let control_fault = Arc::clone(&fault);
    // EOF/ошибка также снимают fault; внешний supervisor ограничивает lifetime группы.
    let control = std::thread::spawn(move || {
        let mut byte = [0];
        let _ = std::io::stdin().read_exact(&mut byte);
        control_fault.store(false, Ordering::Release);
        if let Some(host) = bridge_impl::HOST.get() {
            host.shutdown();
        }
    });
    let code = bridge::ffi::run_application(
        &QString::from(data.to_string_lossy().as_ref()),
        true,
        &QString::from(""),
        &QString::from(&script),
        &QString::from(&trace),
    );
    fault.store(false, Ordering::Release);
    if let Some(host) = bridge_impl::HOST.get() {
        host.shutdown();
        assert!(
            host.wait_closed(std::time::Duration::from_secs(5)),
            "final Stop"
        );
    }
    // При нормальном ходе supervisor прислал release до выхода Qt.
    if control.is_finished() {
        control.join().expect("control thread");
    }
    assert_eq!(code, 0, "Qt smoke");
}
