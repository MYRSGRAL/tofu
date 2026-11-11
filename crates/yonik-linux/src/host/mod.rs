mod binary;
mod probe;
mod process;
mod session;
#[cfg(test)]
mod tests;

pub(crate) use binary::SharedImage;
pub use binary::{BinarySnapshot, Sha256};
pub use probe::{ProbeResult, ProbeTarget};
pub use session::{CheckedConfig, LocalSession};

use std::fmt;
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostError {
    InvalidBinary,
    BinaryIo,
    BinaryHash,
    Snapshot,
    ResourceLimit,
    Version,
    Process,
    ConfigRejected,
    Timeout,
    Cancelled,
    EngineExited,
    Listener,
    Cleanup,
    State,
    ProbeTarget,
}
impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidBinary => "Нужен исполняемый ELF regular file не больше 128 MiB",
            Self::BinaryIo => "Не удалось прочитать бинарник ядра",
            Self::BinaryHash => "SHA-256 бинарника не совпадает",
            Self::Snapshot => "Не удалось создать неизменяемый executable snapshot",
            Self::ResourceLimit => "Достигнут бюджет исполняемых снимков",
            Self::Version => "Версия ядра не соответствует контракту",
            Self::Process => "Не удалось выполнить операцию процесса",
            Self::ConfigRejected => "Ядро отклонило конфигурацию",
            Self::Timeout => "Превышено время операции",
            Self::Cancelled => "Операция отменена",
            Self::EngineExited => "Процесс ядра завершился",
            Self::Listener => "Listener не принадлежит запущенному ядру",
            Self::Cleanup => "Не подтверждено завершение процесса ядра",
            Self::State => "Нарушен порядок операций сессии",
            Self::ProbeTarget => "Некорректный HTTPS endpoint или timeout",
        })
    }
}
impl std::error::Error for HostError {}

/// Закрытие источника тоже отменяет операцию: потеря владельца не оставляет core.
#[derive(Clone, Debug)]
pub struct Cancellation(watch::Receiver<bool>, Option<watch::Receiver<bool>>);
impl Cancellation {
    pub fn new() -> (watch::Sender<bool>, Self) {
        let (sender, receiver) = watch::channel(false);
        (sender, Self(receiver, None))
    }
    pub fn check(&self) -> Result<(), HostError> {
        if *self.0.borrow()
            || self.0.has_changed().is_err()
            || self
                .1
                .as_ref()
                .is_some_and(|other| *other.borrow() || other.has_changed().is_err())
        {
            Err(HostError::Cancelled)
        } else {
            Ok(())
        }
    }
    pub async fn cancelled(&mut self) {
        while self.check().is_ok() {
            tokio::select! {
                _ = self.0.changed() => {},
                _ = async {
                    if let Some(other) = &mut self.1 { let _ = other.changed().await; }
                    else { std::future::pending::<()>().await; }
                } => {},
            }
        }
    }
}

impl Cancellation {
    /// Использует существующий watch-источник; закрытие означает отмену.
    pub fn from_receiver(receiver: watch::Receiver<bool>) -> Self {
        Self(receiver, None)
    }

    pub(crate) fn with_other(mut self, receiver: watch::Receiver<bool>) -> Self {
        self.1 = Some(receiver);
        self
    }

    /// Чтение под watch-lock — точка допуска необратимого COMMIT.
    /// send_replace до чтения запрещает допуск; после допуска worker обязан
    /// вернуть фактический результат COMMIT, даже если пришла отмена.
    pub(crate) fn admit_commit(&self) -> Result<(), HostError> {
        let cancelled = self.0.borrow();
        let other = self.1.as_ref().map(|receiver| receiver.borrow());
        if *cancelled
            || self.0.has_changed().is_err()
            || other.as_ref().is_some_and(|value| **value)
            || self
                .1
                .as_ref()
                .is_some_and(|receiver| receiver.has_changed().is_err())
        {
            Err(HostError::Cancelled)
        } else {
            Ok(())
        }
    }
}
