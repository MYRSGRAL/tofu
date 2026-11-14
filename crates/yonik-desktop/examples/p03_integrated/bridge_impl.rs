use std::{
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
};

use crate::bridge::ffi;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use yonik_desktop::{DesktopHost, HostConfig};

#[path = "../../src/bridge_pending.rs"]
mod bridge_pending;
use bridge_pending::PendingSnapshot;

static CONFIG: OnceLock<HostConfig> = OnceLock::new();

pub fn configure(config: HostConfig) -> Result<(), Box<HostConfig>> {
    CONFIG.set(config).map_err(Box::new)
}

pub struct DesktopBridgeRust {
    pub(crate) snapshot_json: QString,
    host: Option<DesktopHost>,
}

impl Default for DesktopBridgeRust {
    fn default() -> Self {
        Self {
            snapshot_json: QString::from("{}"),
            host: None,
        }
    }
}

impl ffi::DesktopBridge {
    pub fn initialize(mut self: Pin<&mut Self>) {
        if self.host.is_some() {
            return;
        }
        let Some(config) = CONFIG.get() else {
            self.report_error("startup");
            return;
        };
        let thread = self.qt_thread();
        let pending = Arc::new(Mutex::new(PendingSnapshot::default()));
        let startup_pending = Arc::clone(&pending);
        match DesktopHost::start(config.clone(), move |snapshot| {
            let Ok(mut state) = pending.lock() else {
                log::error!("Повреждено состояние доставки GUI");
                return;
            };
            if !state.publish(snapshot) {
                return;
            }
            drop(state);
            let next = Arc::clone(&pending);
            // Отказ queue означает, что QObject уже уничтожен; worker не трогает Qt напрямую.
            if let Err(error) = thread.queue(move |object| {
                let Ok(mut state) = next.lock() else {
                    log::error!("Повреждено состояние доставки GUI");
                    return;
                };
                let snapshot = state.take_queued();
                drop(state);
                if let Some(snapshot) = snapshot {
                    object.set_snapshot_json(QString::from(&snapshot));
                }
            }) {
                if let Ok(mut state) = pending.lock() {
                    state.queue_rejected();
                }
                log::debug!("Доставка события GUI завершена: {error:?}");
            }
        }) {
            Ok(host) => self.as_mut().rust_mut().host = Some(host),
            Err(_) => {
                let snapshot = match startup_pending.lock() {
                    Ok(mut state) => state.take_start_failure(),
                    Err(_) => {
                        log::error!("Повреждено состояние доставки GUI");
                        None
                    }
                };
                if let Some(snapshot) = snapshot {
                    self.set_snapshot_json(QString::from(&snapshot));
                } else {
                    self.report_error("startup");
                }
            }
        }
    }

    pub fn dispatch(mut self: Pin<&mut Self>, request: QString) {
        let raw = request.to_string();
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("fixture JSON");
        let Some(host) = self.host.as_ref() else {
            self.report_dispatch_error(&raw, "startup");
            return;
        };
        if parsed["command"] == "subscription_visibility"
            && parsed["visible"] == false
            && std::env::var_os("P03_DROP_VISIBILITY").is_some()
        {
            crate::trace(serde_json::json!({"kind":"fault_drop", "request":parsed}));
            return;
        }
        match host.dispatch(&raw) {
            Ok(()) => {
                crate::trace(serde_json::json!({"kind":"admitted", "request":parsed}));
                if parsed["command"] == "subscription_visibility"
                    && std::env::var_os("P03_DUPLICATE_ADMISSION").is_some()
                {
                    host.dispatch(&raw).expect("fault duplicate admission");
                    crate::trace(serde_json::json!({"kind":"admitted", "request":parsed}));
                }
            }
            Err(error) => {
                crate::trace(
                    serde_json::json!({"kind":"rejected", "request":parsed, "error":error.key()}),
                );
                self.as_mut().report_dispatch_error(&raw, error.key());
            }
        }
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

    pub fn copy_diagnostics_v2(self: Pin<&mut Self>) -> bool {
        let Some(metadata) =
            yonik_desktop::diagnostics::export_snapshot_v2(&self.snapshot_json().to_string())
        else {
            return false;
        };
        ffi::copy_diagnostic_metadata(&QString::from(&metadata));
        true
    }

    pub fn shutdown(self: Pin<&mut Self>) {
        if let Some(host) = self.host.as_ref() {
            host.shutdown();
        } else {
            // При ошибке старта worker и его ресурсы ещё не созданы.
            self.set_snapshot_json(QString::from("{\"closed\":true}"));
        }
    }

    fn report_dispatch_error(self: Pin<&mut Self>, request: &str, message: &str) {
        let snapshot =
            crate::dispatch_error::snapshot(&self.snapshot_json().to_string(), request, message);
        let mut replacement: serde_json::Value =
            serde_json::from_str(&snapshot).unwrap_or_default();
        yonik_desktop::diagnostics::invalidate_contexts(&mut replacement, None);
        self.set_snapshot_json(QString::from(&replacement.to_string()));
    }

    fn report_error(self: Pin<&mut Self>, message: &str) {
        let snapshot = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
            &self.snapshot_json().to_string(),
        )
        .unwrap_or_default();
        let mut replacement = serde_json::Value::Object(snapshot);
        yonik_desktop::diagnostics::invalidate_contexts(
            &mut replacement,
            Some(yonik_desktop::diagnostics::context::Slot::Error),
        );
        let Some(snapshot) = replacement.as_object_mut() else {
            return;
        };
        snapshot.insert("error".into(), serde_json::Value::String(message.into()));
        self.set_snapshot_json(QString::from(&replacement.to_string()));
    }
}
