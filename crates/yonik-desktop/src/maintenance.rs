//! Контракт maintenance: один запрос до подтверждённого освобождения ресурсов.
//! Отмена не обещает прерывания текущей AEAD-операции.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Backup,
    Restore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Busy,
    Revision,
    Identity,
    InvalidInput,
}
impl AdmissionError {
    pub fn key(self) -> &'static str {
        match self {
            Self::Busy | Self::Revision => "busy",
            Self::Identity | Self::InvalidInput => "invalid_input",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request_id: u64,
    pub operation: Operation,
    pub source: Option<PathBuf>,
    pub destination: PathBuf,
    pub revision: i64,
}

impl Request {
    /// Проверка формы до I/O; secure_path/nofollow и NOREPLACE остаются в repository.
    pub fn validate(&self, live: &Path, revision: i64) -> Result<(), &'static str> {
        self.validate_typed(live, revision)
            .map_err(AdmissionError::key)
    }
    pub fn validate_typed(&self, live: &Path, revision: i64) -> Result<(), AdmissionError> {
        if self.revision != revision {
            return Err(AdmissionError::Revision);
        }
        let source = match (self.operation, self.source.as_deref()) {
            (Operation::Backup, None) => live,
            (Operation::Restore, Some(source)) => source,
            _ => return Err(AdmissionError::InvalidInput),
        };
        for path in [source, self.destination.as_path()] {
            if !path.is_absolute()
                || path.as_os_str().len() > 4096
                || path.components().any(|part| {
                    matches!(
                        part,
                        std::path::Component::ParentDir | std::path::Component::CurDir
                    )
                })
                || path.file_name().is_none()
            {
                return Err(AdmissionError::InvalidInput);
            }
        }
        if self.destination == live || self.destination == source {
            return Err(AdmissionError::InvalidInput);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Draining,
    Running,
    Cancelling,
    Complete,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct Ack {
    pub request_id: u64,
    pub phase: Phase,
    pub error: Option<&'static str>,
}

#[derive(Default)]
pub struct Controller {
    ack: Option<Ack>,
    last_request_id: Option<u64>,
}

impl Controller {
    pub fn ack(&self) -> Option<&Ack> {
        self.ack.as_ref()
    }

    pub fn busy(&self) -> bool {
        self.ack.as_ref().is_some_and(|ack| {
            matches!(
                ack.phase,
                Phase::Draining | Phase::Running | Phase::Cancelling
            )
        })
    }

    pub fn begin(
        &mut self,
        request: &Request,
        live: &Path,
        revision: i64,
    ) -> Result<(), &'static str> {
        self.begin_typed(request, live, revision)
            .map_err(AdmissionError::key)
    }
    pub fn begin_typed(
        &mut self,
        request: &Request,
        live: &Path,
        revision: i64,
    ) -> Result<(), AdmissionError> {
        if self.busy() {
            return Err(AdmissionError::Busy);
        }
        request.validate_typed(live, revision)?;
        if self
            .last_request_id
            .is_some_and(|last| request.request_id <= last)
        {
            return Err(AdmissionError::Identity);
        }
        self.last_request_id = Some(request.request_id);
        self.ack = Some(Ack {
            request_id: request.request_id,
            phase: Phase::Draining,
            error: None,
        });
        Ok(())
    }

    pub fn running(&mut self, request_id: u64) -> bool {
        self.transition(request_id, Phase::Draining, Phase::Running)
    }

    pub fn cancel(&mut self, request_id: u64) -> bool {
        if !self.busy() {
            return false;
        }
        let Some(ack) = self.ack.as_mut().filter(|ack| ack.request_id == request_id) else {
            return false;
        };
        ack.phase = Phase::Cancelling;
        true
    }

    /// Вызывать только после join job и освобождения всех repository handles.
    pub fn drained(&mut self, request_id: u64, result: Result<(), &'static str>) -> bool {
        if !self.busy() {
            return false;
        }
        let Some(ack) = self.ack.as_mut().filter(|ack| ack.request_id == request_id) else {
            return false;
        };
        ack.phase = if result.is_ok() {
            Phase::Complete
        } else {
            Phase::Failed
        };
        ack.error = result.err();
        true
    }

    fn transition(&mut self, request_id: u64, from: Phase, to: Phase) -> bool {
        let Some(ack) = self
            .ack
            .as_mut()
            .filter(|ack| ack.request_id == request_id && ack.phase == from)
        else {
            return false;
        };
        ack.phase = to;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{Controller, Operation, Phase, Request};
    use std::path::Path;

    fn request() -> Request {
        Request {
            request_id: 7,
            operation: Operation::Backup,
            source: None,
            destination: "/private/backup.sqlite".into(),
            revision: 3,
        }
    }

    #[test]
    fn request_contract_rejects_unknown_fields_and_operations() {
        for raw in [
            r#"{"request_id":1,"operation":"export_key","source":"/x","destination":"/y","revision":1}"#,
            r#"{"request_id":1,"operation":"backup","source":"/x","destination":"/y","revision":1,"overwrite":true}"#,
        ] {
            assert!(serde_json::from_str::<Request>(raw).is_err());
        }
    }

    #[test]
    fn cancellation_retains_capacity_until_matching_drain() {
        let mut controller = Controller::default();
        let request = request();
        let live = Path::new("/private/live.sqlite");
        assert_eq!(controller.begin(&request, live, 3), Ok(()));
        assert!(controller.running(7));
        assert!(!controller.cancel(8));
        assert!(controller.cancel(7));
        assert_eq!(controller.begin(&request, live, 3), Err("busy"));
        assert!(!controller.drained(8, Ok(())));
        assert!(controller.busy());
        assert!(controller.drained(7, Err("cancelled")));
        assert_eq!(controller.ack().map(|ack| ack.phase), Some(Phase::Failed));
        assert!(!controller.drained(7, Ok(())));
        assert_eq!(controller.begin(&request, live, 3), Err("invalid_input"));
    }

    #[test]
    fn refuses_live_destination_stale_revision_and_unbounded_paths() {
        let live = Path::new("/private/live.sqlite");
        let mut request = request();
        assert_eq!(request.validate(live, 4), Err("busy"));
        request.destination = live.into();
        assert_eq!(request.validate(live, 3), Err("invalid_input"));
        request.destination = "/private/../live.sqlite".into();
        assert_eq!(request.validate(live, 3), Err("invalid_input"));
        request.destination = format!("/{}", "x".repeat(4096)).into();
        assert_eq!(request.validate(live, 3), Err("invalid_input"));
        request.destination = "/private/new.sqlite".into();
        request.source = Some("/other.sqlite".into());
        assert_eq!(request.validate(live, 3), Err("invalid_input"));
        request.operation = Operation::Restore;
        assert_eq!(request.validate(live, 3), Ok(()));
    }
}
