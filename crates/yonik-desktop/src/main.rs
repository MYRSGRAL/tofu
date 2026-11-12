#![deny(unsafe_code)]

mod dispatch_error;

// Модуль содержит только декларацию сгенерированного FFI.
#[allow(unsafe_code)]
mod bridge;

#[forbid(unsafe_code)]
mod bridge_impl {
    use std::{
        pin::Pin,
        sync::{Arc, Mutex, OnceLock},
    };

    use crate::bridge::ffi;
    use cxx_qt::{CxxQtType, Threading};
    use cxx_qt_lib::QString;
    use yonik_desktop::{DesktopHost, HostConfig};

    static CONFIG: OnceLock<HostConfig> = OnceLock::new();

    pub fn configure(config: HostConfig) -> Result<(), HostConfig> {
        CONFIG.set(config)
    }

    pub struct DesktopBridgeRust {
        pub(crate) snapshot_json: QString,
        host: Option<DesktopHost>,
    }

    #[derive(Default)]
    struct PendingSnapshot {
        value: Option<String>,
        queued: bool,
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
            match DesktopHost::start(config.clone(), move |snapshot| {
                let Ok(mut state) = pending.lock() else {
                    log::error!("Повреждено состояние доставки GUI");
                    return;
                };
                state.value = Some(snapshot);
                if state.queued {
                    return;
                }
                state.queued = true;
                drop(state);
                let next = Arc::clone(&pending);
                // Отказ queue означает, что QObject уже уничтожен; worker не трогает Qt напрямую.
                if let Err(error) = thread.queue(move |object| {
                    let Ok(mut state) = next.lock() else {
                        log::error!("Повреждено состояние доставки GUI");
                        return;
                    };
                    let snapshot = state.value.take();
                    state.queued = false;
                    drop(state);
                    if let Some(snapshot) = snapshot {
                        object.set_snapshot_json(QString::from(&snapshot));
                    }
                }) {
                    if let Ok(mut state) = pending.lock() {
                        state.queued = false;
                        state.value = None;
                    }
                    log::debug!("Доставка события GUI завершена: {error:?}");
                }
            }) {
                Ok(host) => self.as_mut().rust_mut().host = Some(host),
                Err(_) => self.report_error("startup"),
            }
        }

        pub fn dispatch(mut self: Pin<&mut Self>, request: QString) {
            if request.len() > (6 * yonik_import::MAX_INPUT_BYTES + 4096) as isize {
                self.report_error("invalid_input");
                return;
            }
            let Some(host) = self.host.as_ref() else {
                self.report_dispatch_error(&request.to_string(), "startup");
                return;
            };
            if let Err(error) = host.dispatch(&request.to_string()) {
                self.as_mut()
                    .report_dispatch_error(&request.to_string(), error.key());
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
            let snapshot = crate::dispatch_error::snapshot(
                &self.snapshot_json().to_string(),
                request,
                message,
            );
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
}

use std::{num::NonZeroU16, os::unix::fs::DirBuilderExt, path::PathBuf};
use yonik_desktop::HostConfig;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut data_dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .ok_or("Не задан домашний каталог данных")?
        // Старый каталог сохраняет доступ к зашифрованной БД после ребрендинга.
        .join("tofu");
    let mut engine_dir = None;
    let mut listen_port = None;
    let mut explicit_data_dir = false;
    let mut smoke_test = false;
    let mut screenshot = None;
    let mut script = None;
    let mut trace = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--data-dir") => {
                data_dir = PathBuf::from(arguments.next().ok_or("Не задан --data-dir")?);
                explicit_data_dir = true;
            }
            Some("--smoke-test") => smoke_test = true,
            Some("--smoke-script") => {
                script = Some(PathBuf::from(
                    arguments.next().ok_or("Не задан --smoke-script")?,
                ));
            }
            Some("--smoke-trace") => {
                trace = Some(PathBuf::from(
                    arguments.next().ok_or("Не задан --smoke-trace")?,
                ));
            }
            Some("--smoke-screenshot") => {
                screenshot = Some(PathBuf::from(
                    arguments.next().ok_or("Не задан --smoke-screenshot")?,
                ));
            }
            Some("--engine-dir") => {
                engine_dir = Some(PathBuf::from(
                    arguments.next().ok_or("Не задан --engine-dir")?,
                ));
            }
            Some("--listen-port") => {
                let port = arguments.next().ok_or("Не задан --listen-port")?;
                listen_port = Some(
                    port.to_str()
                        .ok_or("Порт должен быть числом")?
                        .parse::<NonZeroU16>()
                        .map_err(|_| "Порт должен быть в диапазоне 1–65535")?,
                );
            }
            _ => return Err("Неизвестный параметр командной строки".into()),
        }
    }
    if (smoke_test && !explicit_data_dir)
        || ((screenshot.is_some() || script.is_some() || trace.is_some()) && !smoke_test)
    {
        return Err(
            "--smoke-test требует явный --data-dir; screenshot доступен только в smoke-test".into(),
        );
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&data_dir)?;
    let data_dir = std::fs::canonicalize(data_dir)?;
    let engine_dir = engine_dir.unwrap_or_else(|| data_dir.join("engines"));
    let mut config = HostConfig::new(data_dir.clone(), engine_dir);
    if let Some(port) = listen_port {
        config = config.with_listen_port(port);
    }
    if smoke_test {
        let mut key = zeroize::Zeroizing::new([0_u8; 32]);
        getrandom::getrandom(key.as_mut()).map_err(|_| "Не удалось создать тестовый ключ")?;
        config = config.with_ephemeral_key(*key);
    }
    bridge_impl::configure(config).map_err(|_| "Конфигурация уже задана")?;
    let data_dir = data_dir.to_str().ok_or("Путь данных должен быть UTF-8")?;
    let screenshot = screenshot
        .as_deref()
        .map(|path| path.to_str().ok_or("Путь screenshot должен быть UTF-8"))
        .transpose()?
        .unwrap_or("");
    let script = script
        .as_deref()
        .map(|path| path.to_str().ok_or("Путь script должен быть UTF-8"))
        .transpose()?
        .unwrap_or("");
    let trace = trace
        .as_deref()
        .map(|path| path.to_str().ok_or("Путь trace должен быть UTF-8"))
        .transpose()?
        .unwrap_or("");
    let code = bridge::ffi::run_application(
        &data_dir.into(),
        smoke_test,
        &screenshot.into(),
        &script.into(),
        &trace.into(),
    );
    if code == 0 {
        Ok(())
    } else {
        Err(format!("Qt завершился с кодом {code}").into())
    }
}
