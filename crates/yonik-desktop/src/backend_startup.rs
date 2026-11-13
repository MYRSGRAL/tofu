//! Фазы startup: только статические метки, без путей, ключей и содержимого БД.

use crate::diagnostics::context::{Cause, ErrorContext, Slot};

/// До запуска worker существует только отказ старта, а не состояние actor.
enum StartupFailure {
    BackendThreadStartFailed,
}

impl StartupFailure {
    fn snapshot(self) -> String {
        let cause = match self {
            Self::BackendThreadStartFailed => Cause::BackendThreadStartFailed,
        };
        serde_json::json!({
            "ready": false,
            "closed": false,
            "error": "startup",
            "connection_error": "startup",
            "error_contexts": [
                ErrorContext::new(Slot::Error, cause, None),
                ErrorContext::new(Slot::Connection, cause, None),
            ],
        })
        .to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupPhase {
    RuntimeBuild,
    SignalRegistration,
    PortSelection,
    DirectoryQueued,
    DirectoryLock,
    DirectoryReturned,
    Credential,
    RepositoryOpen,
    RepositoryReturned,
    Preferences,
    NetworkInitialization,
    NetworkReturned,
}

#[derive(Clone, Default)]
pub(super) struct StartupObserver {
    #[cfg(feature = "startup-diagnostics")]
    callback: Option<std::sync::Arc<dyn Fn(StartupPhase) + Send + Sync>>,
}

impl StartupObserver {
    pub(super) fn finish_thread_start(
        &self,
        result: std::io::Result<std::thread::JoinHandle<()>>,
        callback: &super::Callback,
    ) -> super::Result<()> {
        match result {
            Ok(_) => Ok(()),
            Err(_) => {
                self.publish_failure(StartupFailure::BackendThreadStartFailed, callback);
                Err(super::DesktopError("startup"))
            }
        }
    }

    fn publish_failure(&self, failure: StartupFailure, callback: &super::Callback) {
        callback(failure.snapshot());
    }

    #[cfg(feature = "startup-diagnostics")]
    pub(super) fn new(callback: impl Fn(StartupPhase) + Send + Sync + 'static) -> Self {
        Self {
            callback: Some(std::sync::Arc::new(callback)),
        }
    }

    pub(super) fn emit(&self, phase: StartupPhase) {
        #[cfg(feature = "startup-diagnostics")]
        if let Some(callback) = &self.callback {
            callback(phase);
        }
        #[cfg(not(feature = "startup-diagnostics"))]
        let _ = phase;
    }
}

#[cfg(test)]
mod tests {
    use super::StartupObserver;
    use crate::diagnostics::{export_snapshot, export_snapshot_v2};
    use serde_json::{Value, json};
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    #[test]
    fn thread_start_failure_publishes_closed_context_before_return() {
        let published = Arc::new(Mutex::new(Vec::new()));
        let capture = Arc::clone(&published);
        let callback: super::super::Callback =
            Arc::new(move |raw| capture.lock().expect("публикации").push(raw));
        let result = StartupObserver::default()
            .finish_thread_start(Err(io::Error::other("/private/CANARY secret")), &callback);
        assert_eq!(result.expect_err("отказ").key(), "startup");
        let values = published.lock().expect("публикации");
        assert_eq!(values.len(), 1);
        let raw = &values[0];
        assert!(!raw.contains("CANARY"));
        let snapshot: Value = serde_json::from_str(raw).expect("DTO");
        assert_eq!(snapshot["ready"], false);
        assert_eq!(snapshot["closed"], false);
        assert_eq!(snapshot["error"], "startup");
        assert_eq!(snapshot["connection_error"], "startup");
        let metadata: Value =
            serde_json::from_str(&export_snapshot_v2(raw).expect("V2")).expect("JSON");
        assert_eq!(metadata["errors"].as_array().expect("ошибки").len(), 2);
        for (error, slot) in metadata["errors"]
            .as_array()
            .expect("ошибки")
            .iter()
            .zip(["error", "connection"])
        {
            assert_eq!(error["slot"], slot);
            assert_eq!(error["reason"], "backend_thread_start_failed");
            assert_eq!(error["stage"], "startup");
            assert_eq!(error["code"], "STARTUP");
            assert_eq!(error["message_key"], "startup");
            assert_eq!(error["params"], json!({}));
            assert!(error["generation"].is_null());
        }
        let mut validator = std::process::Command::new("python3")
            .arg("../../tools/check_diagnostic_export_v2.py")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("валидатор");
        use std::io::Write;
        validator
            .stdin
            .take()
            .expect("stdin")
            .write_all(metadata.to_string().as_bytes())
            .expect("запись");
        assert!(validator.wait().expect("статус").success());
        assert!(export_snapshot(raw).is_some());
        let qml = include_str!("../qml/Main.qml");
        assert!(qml.contains(
            "startup: [\"Не удалось запустить приложение.\", \"Could not start the application.\"]"
        ));
    }

    #[test]
    fn successful_thread_start_does_not_publish_failure() {
        let callback: super::super::Callback = Arc::new(|_| panic!("лишняя публикация"));
        let worker = std::thread::spawn(|| ());
        assert!(
            StartupObserver::default()
                .finish_thread_start(Ok(worker), &callback)
                .is_ok()
        );
    }
}
