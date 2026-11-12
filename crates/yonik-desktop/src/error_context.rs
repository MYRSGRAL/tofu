//! Закрытый контекст создаётся в точке отказа, без исходного текста ошибки.
use serde::{Deserialize, Serialize, Serializer};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    Error,
    Connection,
    Source,
    RoutingAck,
    MaintenanceAck,
    LogsRead,
    LogsSave,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    SchedulerInitialize,
    SchedulerAdmission,
    SchedulerService,
    SchedulerValidation,
    Startup,
    StorageOpen,
    EngineStart,
    EngineRun,
    Cleanup,
    HelperRpc,
    RoutingSave,
    MaintenanceAdmission,
    MaintenanceCopy,
    MaintenanceReopen,
    MaintenanceDrain,
    LogsRead,
    LogsSave,
    CatalogRead,
    StorageWrite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    SchedulerInitializeInvalidLimits,
    SchedulerInitializeExhausted,
    SchedulerInvalidLimits,
    SchedulerFull,
    SchedulerDuplicate,
    SchedulerExhausted,
    SchedulerStale,
    SchedulerIntervalInvalid,
    SchedulerServiceInvalidSource,
    SchedulerServiceCorruptSnapshot,
    SchedulerServiceNotFound,
    SchedulerServiceNoSnapshot,
    SchedulerServiceExhausted,
    SchedulerServiceForeignApproval,
    SchedulerServiceStaleUpdate,
    SchedulerServiceWorker,
    SchedulerRepositoryInvalidInput,
    SchedulerRepositoryUnsafePath,
    SchedulerRepositoryUnsupportedVersion,
    SchedulerRepositoryCorrupt,
    SchedulerRepositoryStorage,
    SchedulerRepositoryConflict,
    SchedulerRepositoryNotFound,

    DataDirectoryRelative,
    DataDirectoryCreate,
    DataDirectoryMetadata,
    DataDirectoryType,
    DataDirectoryOwner,
    DataDirectoryPermissions,
    DataDirectoryCanonicalize,
    DataDirectoryOpen,
    DataDirectoryLock,
    StorageTaskFailed,
    KeyringMissing,
    KeyringLocked,

    ListenPortUnavailable,
    EngineStartFailed,
    EngineStartExited,
    EngineExited,
    CleanupFailed,
    HelperUnsupported,
    HelperDenied,
    HelperCancelled,
    HelperBusy,
    HelperOwnerLost,
    HelperDeadline,
    HelperTimeout,
    HelperUnavailable,
    HelperInvalidRequest,
    HelperInvalidHandle,
    HelperStale,
    HelperProtocol,
    RoutingRequestIdentity,
    RoutingRevisionStale,
    RoutingRuleIdentity,
    RoutingRepositoryMissing,
    RoutingRepositoryConflict,
    RoutingRepositoryFailed,
    MaintenanceAdmissionBusy,
    MaintenanceAdmissionRevision,
    MaintenanceAdmissionIdentity,
    MaintenanceAdmissionInvalidInput,
    MaintenanceAdmissionUnavailable,
    MaintenanceAdmissionUnsupported,
    MaintenanceAdmissionDraining,
    MaintenanceAdmissionReopenUnavailable,
    MaintenanceAdmissionStale,
    MaintenanceAdmissionSchedulerUnavailable,
    MaintenanceCopyInvalidInput,
    MaintenanceCopyUnsafePath,
    MaintenanceCopyUnsupportedVersion,
    MaintenanceCopyCorrupt,
    MaintenanceCopyStorage,
    MaintenanceCopyConflict,
    MaintenanceCopyNotFound,
    MaintenanceReopenInvalidInput,
    MaintenanceReopenUnsafePath,
    MaintenanceReopenUnsupportedVersion,
    MaintenanceReopenCorrupt,
    MaintenanceReopenStorage,
    MaintenanceReopenConflict,
    MaintenanceReopenNotFound,
    MaintenanceAccessInvalidInput,
    MaintenanceAccessUnsafePath,
    MaintenanceAccessUnsupportedVersion,
    MaintenanceAccessCorrupt,
    MaintenanceAccessStorage,
    MaintenanceAccessConflict,
    MaintenanceAccessNotFound,
    MaintenanceCancelled,
    LogsReadBusy,
    LogsSaveBusy,
    LogsPageIdentity,
    LogsPageUnavailable,
    LogsDurabilityUnknown,
    LogsIoPermissionDenied,
    LogsIoAlreadyExists,
    LogsIoNotFound,
    LogsIoInvalidInput,
    LogsIoOther,
    BackendThreadStartFailed,
    RuntimePanicked,
    SignalsUnavailable,
    CatalogTaskFailed,
    CatalogQueryFailed,
    PreferencesIoPermissionDenied,
    PreferencesIoAlreadyExists,
    PreferencesIoNotFound,
    PreferencesIoInvalidInput,
    PreferencesIoOther,
    RepositoryOpenInvalidInput,
    RepositoryOpenUnsafePath,
    RepositoryOpenUnsupportedVersion,
    RepositoryOpenCorrupt,
    RepositoryOpenStorage,
    RepositoryOpenConflict,
    RepositoryOpenNotFound,
    KeyringUnavailable,
    KeyringAmbiguous,
    KeyringInvalidKey,
    KeyringRandom,
    KeyringTimeout,
    KeyringBusy,
}

impl Cause {
    pub fn allows(self, slot: Slot) -> bool {
        if matches!(
            self,
            Self::MaintenanceAdmissionUnsupported | Self::MaintenanceAdmissionReopenUnavailable
        ) {
            return matches!(slot, Slot::MaintenanceAck | Slot::Error | Slot::Connection);
        }
        match self.identity().2 {
            Stage::MaintenanceAdmission
            | Stage::MaintenanceCopy
            | Stage::MaintenanceDrain
            | Stage::MaintenanceReopen => slot == Slot::MaintenanceAck,
            Stage::LogsRead => slot == Slot::LogsRead,
            Stage::LogsSave => slot == Slot::LogsSave,
            Stage::SchedulerAdmission | Stage::SchedulerService | Stage::SchedulerValidation => {
                matches!(slot, Slot::Error | Slot::Source)
            }
            Stage::SchedulerInitialize => matches!(slot, Slot::Error | Slot::Source),
            Stage::StorageWrite => matches!(slot, Slot::Error | Slot::Connection | Slot::Source),
            Stage::RoutingSave => matches!(slot, Slot::Error | Slot::Connection | Slot::RoutingAck),
            _ => matches!(slot, Slot::Error | Slot::Connection),
        }
    }
    pub fn needs_generation(self) -> bool {
        !matches!(
            self.identity().2,
            Stage::Startup | Stage::StorageOpen | Stage::SchedulerInitialize
        )
    }
    fn identity(self) -> (&'static str, &'static str, Stage) {
        match self {
            Self::SchedulerInitializeInvalidLimits | Self::SchedulerInitializeExhausted => {
                ("STARTUP", "startup", Stage::SchedulerInitialize)
            }
            Self::SchedulerInvalidLimits
            | Self::SchedulerFull
            | Self::SchedulerDuplicate
            | Self::SchedulerExhausted
            | Self::SchedulerStale => ("STORAGE", "storage", Stage::SchedulerAdmission),
            Self::SchedulerIntervalInvalid | Self::SchedulerServiceInvalidSource => {
                ("INVALID_INPUT", "invalid_input", Stage::SchedulerValidation)
            }
            Self::SchedulerServiceCorruptSnapshot => {
                ("STORAGE", "storage", Stage::SchedulerService)
            }
            Self::SchedulerServiceNotFound => ("STORAGE", "storage", Stage::SchedulerService),
            Self::SchedulerServiceNoSnapshot => ("STORAGE", "storage", Stage::SchedulerService),
            Self::SchedulerServiceExhausted => ("STORAGE", "storage", Stage::SchedulerService),
            Self::SchedulerServiceForeignApproval => {
                ("STORAGE", "storage", Stage::SchedulerService)
            }
            Self::SchedulerServiceStaleUpdate => ("STORAGE", "storage", Stage::SchedulerService),
            Self::SchedulerServiceWorker => ("STORAGE", "storage", Stage::SchedulerService),
            Self::SchedulerRepositoryInvalidInput => {
                ("STORAGE", "storage", Stage::SchedulerService)
            }
            Self::SchedulerRepositoryUnsafePath => ("STORAGE", "storage", Stage::SchedulerService),
            Self::SchedulerRepositoryUnsupportedVersion => {
                ("STORAGE", "storage", Stage::SchedulerService)
            }
            Self::SchedulerRepositoryCorrupt => ("STORAGE", "storage", Stage::SchedulerService),
            Self::SchedulerRepositoryStorage => ("STORAGE", "storage", Stage::SchedulerService),
            Self::SchedulerRepositoryConflict => ("STORAGE", "storage", Stage::SchedulerService),
            Self::SchedulerRepositoryNotFound => ("STORAGE", "storage", Stage::SchedulerService),
            Self::RepositoryOpenInvalidInput => ("STORAGE", "storage", Stage::StorageOpen),
            Self::RepositoryOpenUnsafePath => ("STORAGE", "storage", Stage::StorageOpen),
            Self::RepositoryOpenUnsupportedVersion => ("STORAGE", "storage", Stage::StorageOpen),
            Self::RepositoryOpenCorrupt => ("STORAGE", "storage", Stage::StorageOpen),
            Self::RepositoryOpenStorage => ("STORAGE", "storage", Stage::StorageOpen),
            Self::RepositoryOpenConflict => ("STORAGE", "storage", Stage::StorageOpen),
            Self::RepositoryOpenNotFound => ("STORAGE", "storage", Stage::StorageOpen),
            Self::KeyringUnavailable => ("KEYRING_LOCKED", "keyring_locked", Stage::StorageOpen),
            Self::KeyringAmbiguous => ("KEYRING_LOCKED", "keyring_locked", Stage::StorageOpen),
            Self::KeyringInvalidKey => ("KEYRING_LOCKED", "keyring_locked", Stage::StorageOpen),
            Self::KeyringRandom => ("KEYRING_LOCKED", "keyring_locked", Stage::StorageOpen),
            Self::KeyringTimeout => ("KEYRING_LOCKED", "keyring_locked", Stage::StorageOpen),
            Self::KeyringBusy => ("KEYRING_LOCKED", "keyring_locked", Stage::StorageOpen),
            Self::BackendThreadStartFailed => ("STARTUP", "startup", Stage::Startup),
            Self::RuntimePanicked => ("STARTUP", "startup", Stage::Startup),
            Self::SignalsUnavailable => ("STARTUP", "startup", Stage::Startup),
            Self::CatalogTaskFailed => ("CATALOG", "catalog", Stage::CatalogRead),
            Self::CatalogQueryFailed => ("CATALOG", "catalog", Stage::CatalogRead),
            Self::PreferencesIoPermissionDenied => (
                "PREFERENCES_WRITE",
                "preferences_write",
                Stage::StorageWrite,
            ),
            Self::PreferencesIoAlreadyExists => (
                "PREFERENCES_WRITE",
                "preferences_write",
                Stage::StorageWrite,
            ),
            Self::PreferencesIoNotFound => (
                "PREFERENCES_WRITE",
                "preferences_write",
                Stage::StorageWrite,
            ),
            Self::PreferencesIoInvalidInput => (
                "PREFERENCES_WRITE",
                "preferences_write",
                Stage::StorageWrite,
            ),
            Self::PreferencesIoOther => (
                "PREFERENCES_WRITE",
                "preferences_write",
                Stage::StorageWrite,
            ),
            Self::MaintenanceAdmissionBusy => ("BUSY", "busy", Stage::MaintenanceAdmission),
            Self::MaintenanceAdmissionRevision => ("BUSY", "busy", Stage::MaintenanceAdmission),
            Self::MaintenanceAdmissionIdentity => (
                "INVALID_INPUT",
                "invalid_input",
                Stage::MaintenanceAdmission,
            ),
            Self::MaintenanceAdmissionInvalidInput => (
                "INVALID_INPUT",
                "invalid_input",
                Stage::MaintenanceAdmission,
            ),
            Self::MaintenanceAdmissionUnavailable => ("BUSY", "busy", Stage::MaintenanceAdmission),
            Self::MaintenanceAdmissionUnsupported => {
                ("STARTUP", "startup", Stage::MaintenanceAdmission)
            }
            Self::MaintenanceAdmissionDraining => ("BUSY", "busy", Stage::MaintenanceAdmission),
            Self::MaintenanceAdmissionReopenUnavailable => {
                ("BUSY", "busy", Stage::MaintenanceAdmission)
            }
            Self::MaintenanceAdmissionStale => {
                ("CANCELLED", "cancelled", Stage::MaintenanceAdmission)
            }
            Self::MaintenanceAdmissionSchedulerUnavailable => {
                ("STARTUP", "startup", Stage::MaintenanceAdmission)
            }
            Self::MaintenanceCopyInvalidInput => {
                ("INVALID_INPUT", "invalid_input", Stage::MaintenanceCopy)
            }
            Self::MaintenanceCopyUnsafePath => {
                ("INVALID_INPUT", "invalid_input", Stage::MaintenanceCopy)
            }
            Self::MaintenanceCopyUnsupportedVersion => {
                ("STORAGE", "storage", Stage::MaintenanceCopy)
            }
            Self::MaintenanceCopyCorrupt => ("STORAGE", "storage", Stage::MaintenanceCopy),
            Self::MaintenanceCopyStorage => ("STORAGE", "storage", Stage::MaintenanceCopy),
            Self::MaintenanceCopyConflict => ("BUSY", "busy", Stage::MaintenanceCopy),
            Self::MaintenanceCopyNotFound => ("STORAGE", "storage", Stage::MaintenanceCopy),
            Self::MaintenanceReopenInvalidInput => ("STORAGE", "storage", Stage::MaintenanceReopen),
            Self::MaintenanceReopenUnsafePath => ("STORAGE", "storage", Stage::MaintenanceReopen),
            Self::MaintenanceReopenUnsupportedVersion => {
                ("STORAGE", "storage", Stage::MaintenanceReopen)
            }
            Self::MaintenanceReopenCorrupt => ("STORAGE", "storage", Stage::MaintenanceReopen),
            Self::MaintenanceReopenStorage => ("STORAGE", "storage", Stage::MaintenanceReopen),
            Self::MaintenanceReopenConflict => ("STORAGE", "storage", Stage::MaintenanceReopen),
            Self::MaintenanceReopenNotFound => ("STORAGE", "storage", Stage::MaintenanceReopen),
            Self::MaintenanceAccessInvalidInput => ("STORAGE", "storage", Stage::MaintenanceDrain),
            Self::MaintenanceAccessUnsafePath => ("STORAGE", "storage", Stage::MaintenanceDrain),
            Self::MaintenanceAccessUnsupportedVersion => {
                ("STORAGE", "storage", Stage::MaintenanceDrain)
            }
            Self::MaintenanceAccessCorrupt => ("STORAGE", "storage", Stage::MaintenanceDrain),
            Self::MaintenanceAccessStorage => ("STORAGE", "storage", Stage::MaintenanceDrain),
            Self::MaintenanceAccessConflict => ("STORAGE", "storage", Stage::MaintenanceDrain),
            Self::MaintenanceAccessNotFound => ("STORAGE", "storage", Stage::MaintenanceDrain),
            Self::MaintenanceCancelled => ("CANCELLED", "cancelled", Stage::MaintenanceCopy),
            Self::LogsReadBusy => ("BUSY", "busy", Stage::LogsRead),
            Self::LogsSaveBusy => ("BUSY", "busy", Stage::LogsSave),
            Self::LogsPageIdentity => ("LOGS_SAVE_FAILED", "logs_save_failed", Stage::LogsSave),
            Self::LogsPageUnavailable => ("LOGS_SAVE_FAILED", "logs_save_failed", Stage::LogsSave),
            Self::LogsDurabilityUnknown => (
                "LOGS_SAVE_DURABILITY_UNKNOWN",
                "logs_save_durability_unknown",
                Stage::LogsSave,
            ),
            Self::LogsIoPermissionDenied => {
                ("LOGS_SAVE_FAILED", "logs_save_failed", Stage::LogsSave)
            }
            Self::LogsIoAlreadyExists => ("LOGS_SAVE_FAILED", "logs_save_failed", Stage::LogsSave),
            Self::LogsIoNotFound => ("LOGS_SAVE_FAILED", "logs_save_failed", Stage::LogsSave),
            Self::LogsIoInvalidInput => ("LOGS_SAVE_FAILED", "logs_save_failed", Stage::LogsSave),
            Self::LogsIoOther => ("LOGS_SAVE_FAILED", "logs_save_failed", Stage::LogsSave),
            Self::RoutingRequestIdentity => ("INVALID_INPUT", "invalid_input", Stage::RoutingSave),
            Self::RoutingRevisionStale | Self::RoutingRepositoryConflict => {
                ("ROUTING_STALE", "routing_stale", Stage::RoutingSave)
            }
            Self::RoutingRuleIdentity => ("ID", "id", Stage::RoutingSave),
            Self::RoutingRepositoryMissing | Self::RoutingRepositoryFailed => {
                ("ROUTING_STORAGE", "routing_storage", Stage::RoutingSave)
            }
            Self::DataDirectoryRelative
            | Self::DataDirectoryCreate
            | Self::DataDirectoryMetadata
            | Self::DataDirectoryType
            | Self::DataDirectoryOwner
            | Self::DataDirectoryPermissions
            | Self::DataDirectoryCanonicalize
            | Self::DataDirectoryOpen
            | Self::StorageTaskFailed => ("STORAGE", "storage", Stage::StorageOpen),
            Self::DataDirectoryLock => ("BUSY", "busy", Stage::StorageOpen),
            Self::KeyringMissing => ("KEYRING_MISSING", "keyring_missing", Stage::StorageOpen),
            Self::KeyringLocked => ("KEYRING_LOCKED", "keyring_locked", Stage::StorageOpen),
            Self::ListenPortUnavailable => ("LISTEN_PORT_BUSY", "listen_port_busy", Stage::Startup),
            Self::EngineStartFailed | Self::EngineStartExited => {
                ("ENGINE_FAILED", "engine_failed", Stage::EngineStart)
            }
            Self::EngineExited => ("ENGINE_FAILED", "engine_failed", Stage::EngineRun),
            Self::CleanupFailed => ("CLEANUP_FAILED", "cleanup_failed", Stage::Cleanup),
            Self::HelperUnsupported => {
                ("SYSTEM_UNSUPPORTED", "system_unsupported", Stage::HelperRpc)
            }
            Self::HelperDenied => ("SYSTEM_DENIED", "system_denied", Stage::HelperRpc),
            Self::HelperCancelled => ("CANCELLED", "cancelled", Stage::HelperRpc),
            Self::HelperBusy => ("BUSY", "busy", Stage::HelperRpc),
            Self::HelperOwnerLost => ("SYSTEM_OWNER_LOST", "system_owner_lost", Stage::HelperRpc),
            Self::HelperDeadline | Self::HelperTimeout => {
                ("SYSTEM_TIMEOUT", "system_timeout", Stage::HelperRpc)
            }
            Self::HelperUnavailable
            | Self::HelperInvalidRequest
            | Self::HelperInvalidHandle
            | Self::HelperStale
            | Self::HelperProtocol => {
                ("SYSTEM_UNAVAILABLE", "system_unavailable", Stage::HelperRpc)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct SafeParams {}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ErrorContext {
    pub slot: Slot,
    pub code: &'static str,
    pub message_key: &'static str,
    #[serde(serialize_with = "decimal_id")]
    pub request_id: Option<u64>,
    #[serde(serialize_with = "decimal_id")]
    pub generation: Option<u64>,
    pub stage: Stage,
    pub reason: Cause,
    pub retryable: bool,
    pub params: SafeParams,
}

impl ErrorContext {
    pub fn new(slot: Slot, reason: Cause, request_id: Option<u64>) -> Self {
        let (code, message_key, stage) = reason.identity();
        Self {
            slot,
            code,
            message_key,
            request_id,
            generation: None,
            stage,
            reason,
            retryable: false,
            params: SafeParams {},
        }
    }
}

/// Фиксированная ёмкость; replacement всегда заменяет также причину и request ID.
#[derive(Default)]
pub struct Contexts([Option<ErrorContext>; 7]);
impl Contexts {
    pub fn set(&mut self, slot: Slot, cause: Cause, request_id: Option<u64>) {
        self.0[slot as usize] = Some(ErrorContext::new(slot, cause, request_id));
    }
    pub fn set_ack(&mut self, slot: Slot, cause: Cause, request_id: u64, generation: u64) {
        let mut context = ErrorContext::new(slot, cause, Some(request_id));
        context.generation = Some(generation);
        self.0[slot as usize] = Some(context);
    }
    pub fn set_generation(&mut self, slot: Slot, cause: Cause, generation: Option<u64>) {
        let mut context = ErrorContext::new(slot, cause, None);
        context.generation = generation;
        self.0[slot as usize] = Some(context);
    }
    pub fn clear(&mut self, slot: Slot) {
        self.0[slot as usize] = None;
    }
    pub fn matching(
        &self,
        slot: Slot,
        key: Option<&str>,
        request_id: Option<u64>,
    ) -> Option<ErrorContext> {
        self.0[slot as usize]
            .filter(|context| Some(context.message_key) == key && context.request_id == request_id)
    }
}

fn decimal_id<S: Serializer>(id: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error> {
    id.map(|id| id.to_string()).serialize(serializer)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_clear_and_ack_identity() {
        let mut contexts = Contexts::default();
        contexts.set(Slot::RoutingAck, Cause::RepositoryOpenStorage, Some(7));
        assert!(
            contexts
                .matching(Slot::RoutingAck, Some("storage"), Some(8))
                .is_none()
        );
        assert!(
            contexts
                .matching(Slot::RoutingAck, Some("busy"), Some(7))
                .is_none()
        );
        contexts.set(Slot::RoutingAck, Cause::DataDirectoryPermissions, Some(8));
        let current = contexts
            .matching(Slot::RoutingAck, Some("storage"), Some(8))
            .expect("текущий ACK");
        assert_eq!(current.reason, Cause::DataDirectoryPermissions);
        assert!(!current.retryable);
        contexts.clear(Slot::RoutingAck);
        assert!(
            contexts
                .matching(Slot::RoutingAck, Some("storage"), Some(8))
                .is_none()
        );
    }
    #[test]
    fn same_key_does_not_determine_reason() {
        let permissions =
            ErrorContext::new(Slot::Connection, Cause::DataDirectoryPermissions, None);
        let repository = ErrorContext::new(Slot::Connection, Cause::RepositoryOpenStorage, None);
        assert_eq!(permissions.message_key, repository.message_key);
        assert_ne!(permissions.reason, repository.reason);
        assert_eq!(permissions.stage, Stage::StorageOpen);
    }
}
