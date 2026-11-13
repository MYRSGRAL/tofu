//! Внутренний результат ручной операции; legacy DTO остаётся прежним.
use super::{Cause, DesktopError, FetchError, SubscriptionError, map_subscription};
use yonik_linux::RepositoryError;

#[derive(Debug)]
pub(super) struct SourceFailure {
    pub error: DesktopError,
    pub context: Option<FailureContext>,
}
#[derive(Clone, Copy, Debug)]
pub(super) enum FailureContext {
    Source(Cause),
    Scheduler(Cause, u64),
}
pub(super) type SourceResult<T> = std::result::Result<T, SourceFailure>;
impl SourceFailure {
    pub fn key(&self) -> &'static str {
        self.error.key()
    }
    pub fn subscription(error: SubscriptionError) -> Self {
        use FetchError as F;
        use RepositoryError as R;
        use SubscriptionError as E;
        let cause = match error {
            E::InvalidSource => Cause::SourceServiceInvalidSource,
            E::CorruptSnapshot => Cause::SourceServiceCorruptSnapshot,
            E::NotFound => Cause::SourceServiceNotFound,
            E::NoSnapshot => Cause::SourceServiceNoSnapshot,
            E::Exhausted => Cause::SourceServiceExhausted,
            E::ForeignApproval => Cause::SourceServiceForeignApproval,
            E::StaleUpdate => Cause::SourceServiceStaleUpdate,
            E::Worker => Cause::SourceServiceWorker,
            E::Repository(R::InvalidInput) => Cause::SourceRepositoryInvalidInput,
            E::Repository(R::UnsafePath) => Cause::SourceRepositoryUnsafePath,
            E::Repository(R::UnsupportedVersion) => Cause::SourceRepositoryUnsupportedVersion,
            E::Repository(R::Corrupt) => Cause::SourceRepositoryCorrupt,
            E::Repository(R::Storage) => Cause::SourceRepositoryStorage,
            E::Repository(R::Conflict) => Cause::SourceRepositoryConflict,
            E::Repository(R::NotFound) => Cause::SourceRepositoryNotFound,
            E::Fetch(F::HwidRequired) => Cause::SourceFetchHwidRequired,
            E::Fetch(F::HwidLimit) => Cause::SourceFetchHwidLimit,
            E::Fetch(F::HwidRejected) => Cause::SourceFetchHwidRejected,
            E::Fetch(F::HwidRedirect) => Cause::SourceFetchHwidRedirect,
            E::Fetch(F::UnsupportedPeerVerification) => {
                Cause::SourceFetchUnsupportedPeerVerification
            }
            E::Fetch(_) => Cause::SourceFetchFailed,
            E::Import(_) => Cause::SourceServiceImportFailed,
        };
        Self {
            error: map_subscription(error),
            context: Some(FailureContext::Source(cause)),
        }
    }
    pub fn selection(error: SubscriptionError) -> Self {
        Self {
            error: DesktopError("profile_unavailable"),
            context: matches!(error, SubscriptionError::Repository(_)).then_some(
                FailureContext::Source(Cause::SourceSelectionRepositoryFailed),
            ),
        }
    }
    pub fn schedule(failure: super::desktop_schedule::ScheduleFailure, generation: u64) -> Self {
        Self {
            error: failure.error,
            context: failure
                .cause
                .map(|cause| FailureContext::Scheduler(cause, generation)),
        }
    }
}
impl From<DesktopError> for SourceFailure {
    fn from(error: DesktopError) -> Self {
        Self {
            error,
            context: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Snapshot;
    use super::*;
    use crate::diagnostics::{
        context::{Slot, Stage},
        export_snapshot_v2,
    };
    use serde_json::{Value, json};

    fn snapshot(state: &Snapshot) -> Value {
        let value = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let copy = value.clone();
        let callback: super::super::Callback = std::sync::Arc::new(move |snapshot| {
            *copy.lock().expect("снимок") = snapshot;
        });
        state.emit(&callback);
        let snapshot = value.lock().expect("снимок").clone();
        serde_json::from_str(&snapshot).expect("DTO")
    }
    #[test]
    fn source_batch_сохраняет_категории_ключи_и_fences() {
        use FetchError as F;
        use RepositoryError as R;
        use SubscriptionError as E;
        let cases = [
            (E::InvalidSource, Cause::SourceServiceInvalidSource),
            (E::CorruptSnapshot, Cause::SourceServiceCorruptSnapshot),
            (E::NotFound, Cause::SourceServiceNotFound),
            (E::NoSnapshot, Cause::SourceServiceNoSnapshot),
            (E::Exhausted, Cause::SourceServiceExhausted),
            (E::ForeignApproval, Cause::SourceServiceForeignApproval),
            (E::StaleUpdate, Cause::SourceServiceStaleUpdate),
            (E::Worker, Cause::SourceServiceWorker),
            (
                E::Repository(R::InvalidInput),
                Cause::SourceRepositoryInvalidInput,
            ),
            (
                E::Repository(R::UnsafePath),
                Cause::SourceRepositoryUnsafePath,
            ),
            (
                E::Repository(R::UnsupportedVersion),
                Cause::SourceRepositoryUnsupportedVersion,
            ),
            (E::Repository(R::Corrupt), Cause::SourceRepositoryCorrupt),
            (E::Repository(R::Storage), Cause::SourceRepositoryStorage),
            (E::Repository(R::Conflict), Cause::SourceRepositoryConflict),
            (E::Repository(R::NotFound), Cause::SourceRepositoryNotFound),
            (E::Fetch(F::InvalidSource), Cause::SourceFetchFailed),
            (E::Fetch(F::InvalidHwid), Cause::SourceFetchFailed),
            (E::Fetch(F::HwidRequired), Cause::SourceFetchHwidRequired),
            (E::Fetch(F::HwidLimit), Cause::SourceFetchHwidLimit),
            (E::Fetch(F::HwidRejected), Cause::SourceFetchHwidRejected),
            (E::Fetch(F::HwidRedirect), Cause::SourceFetchHwidRedirect),
            (E::Fetch(F::InvalidValidator), Cause::SourceFetchFailed),
            (E::Fetch(F::InvalidTimeout), Cause::SourceFetchFailed),
            (
                E::Fetch(F::UnsupportedPeerVerification),
                Cause::SourceFetchUnsupportedPeerVerification,
            ),
            (E::Fetch(F::ProxyDnsRequired), Cause::SourceFetchFailed),
            (E::Fetch(F::Cancelled), Cause::SourceFetchFailed),
            (E::Fetch(F::Timeout), Cause::SourceFetchFailed),
            (E::Fetch(F::Dns), Cause::SourceFetchFailed),
            (E::Fetch(F::BlockedAddress), Cause::SourceFetchFailed),
            (E::Fetch(F::PeerMismatch), Cause::SourceFetchFailed),
            (E::Fetch(F::Transport), Cause::SourceFetchFailed),
            (E::Fetch(F::Redirect), Cause::SourceFetchFailed),
            (E::Fetch(F::HttpStatus), Cause::SourceFetchFailed),
            (E::Fetch(F::UnexpectedNotModified), Cause::SourceFetchFailed),
            (E::Fetch(F::UnsupportedEncoding), Cause::SourceFetchFailed),
            (E::Fetch(F::BodyTooLarge), Cause::SourceFetchFailed),
        ];
        for (error, cause) in cases {
            let mut state = Snapshot::new(true);
            state.command_generation = 17;
            let failure = SourceFailure::subscription(error);
            assert_eq!(failure.key(), map_subscription(error).key());
            state.source_failure(true, failure, 17);
            let dto = snapshot(&state);
            assert_eq!(
                dto["error_contexts"].as_array().expect("контексты").len(),
                2
            );
            for slot in [Slot::Error, Slot::Source] {
                let context = state
                    .error_contexts
                    .matching(slot, state.error, None)
                    .expect("причина");
                assert_eq!(context.reason, cause);
                assert_eq!(context.generation, Some(17));
                assert!(context.request_id.is_none());
                assert!(!context.retryable);
            }
            assert!(export_snapshot_v2(&dto.to_string()).is_some(), "{error:?}");
            for (field, value) in [
                ("generation", json!("16")),
                ("slot", json!("routing_ack")),
                ("request_id", json!("9")),
                ("code", json!("STORAGE_OTHER")),
                ("stage", json!("scheduler_service")),
            ] {
                let mut wrong = dto.clone();
                wrong["error_contexts"][0][field] = value;
                assert!(
                    export_snapshot_v2(&wrong.to_string()).is_none(),
                    "{error:?}: {field}"
                );
            }
            // Другой command с тем же key не наследует причину предыдущего.
            let key = state.error.expect("ошибка");
            state.source_failure(true, DesktopError(key).into(), 17);
            assert!(
                state
                    .error_contexts
                    .matching(Slot::Error, Some(key), None)
                    .is_none()
            );
            assert!(
                state
                    .error_contexts
                    .matching(Slot::Source, Some(key), None)
                    .is_none()
            );
            assert!(export_snapshot_v2(&snapshot(&state).to_string()).is_none());
        }
    }
    #[test]
    fn source_selection_и_connection_используют_фактические_slots() {
        for error in [
            RepositoryError::InvalidInput,
            RepositoryError::UnsafePath,
            RepositoryError::UnsupportedVersion,
            RepositoryError::Corrupt,
            RepositoryError::Storage,
            RepositoryError::Conflict,
            RepositoryError::NotFound,
        ] {
            let mut state = Snapshot::new(true);
            state.command_generation = 19;
            state.source_failure(
                true,
                SourceFailure::selection(SubscriptionError::Repository(error)),
                19,
            );
            assert!(export_snapshot_v2(&snapshot(&state).to_string()).is_some());
            let context = state
                .error_contexts
                .matching(Slot::Source, Some("profile_unavailable"), None)
                .expect("selection");
            assert_eq!(context.reason, Cause::SourceSelectionRepositoryFailed);
            assert_eq!(context.stage, Stage::SourceSelection);
        }
        let mut state = Snapshot::new(true);
        state.command_generation = 19;
        state.source_failure(
            false,
            SourceFailure::subscription(SubscriptionError::NotFound),
            19,
        );
        assert!(export_snapshot_v2(&snapshot(&state).to_string()).is_some());
        assert!(Cause::SourceServiceNotFound.allows(Slot::Connection));
        assert!(!Cause::SourceFetchFailed.allows(Slot::Connection));
        assert!(!Cause::SourceSelectionRepositoryFailed.allows(Slot::Connection));
        state.command_generation = 20;
        assert!(export_snapshot_v2(&snapshot(&state).to_string()).is_none());
    }
    #[test]
    fn source_scheduler_companion_сохраняет_своё_поколение_и_precedence() {
        let mut state = Snapshot::new(true);
        state.command_generation = 31;
        let schedule = super::super::desktop_schedule::ScheduleFailure::service(
            SubscriptionError::Repository(RepositoryError::Corrupt),
        );
        state.source_failure(true, SourceFailure::schedule(schedule, 77), 31);
        assert!(export_snapshot_v2(&snapshot(&state).to_string()).is_some());
        let context = state
            .error_contexts
            .matching(Slot::Source, Some("storage"), None)
            .expect("scheduler");
        assert_eq!(context.reason, Cause::SchedulerRepositoryCorrupt);
        assert_eq!(context.generation, Some(77));
        assert_eq!(state.scheduler_generation, Some(77));
        // Primary manual outcome заменяет более поздний reload с тем же legacy key.
        state.source_failure(
            true,
            SourceFailure::subscription(SubscriptionError::Worker),
            31,
        );
        let context = state
            .error_contexts
            .matching(Slot::Source, Some("storage"), None)
            .expect("manual");
        assert_eq!(context.reason, Cause::SourceServiceWorker);
        assert_eq!(context.generation, Some(31));
        assert!(export_snapshot_v2(&snapshot(&state).to_string()).is_some());
        // Async completion публикуется раньше reconcile: последний failure владеет slots.
        state.schedule_error(
            super::super::desktop_schedule::ScheduleFailure::service(
                SubscriptionError::Repository(RepositoryError::NotFound),
            ),
            Some(78),
        );
        let context = state
            .error_contexts
            .matching(Slot::Source, Some("storage"), None)
            .expect("reconcile");
        assert_eq!(context.reason, Cause::SchedulerRepositoryNotFound);
        assert_eq!(context.generation, Some(78));
        assert!(export_snapshot_v2(&snapshot(&state).to_string()).is_some());
    }
}
