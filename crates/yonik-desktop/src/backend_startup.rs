//! Фазы startup: только статические метки, без путей, ключей и содержимого БД.

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
