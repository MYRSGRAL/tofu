mod binary;
mod probe;
mod process;
mod session;
#[cfg(test)]
mod tests;

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
pub struct Cancellation(watch::Receiver<bool>);
impl Cancellation {
    pub fn new() -> (watch::Sender<bool>, Self) {
        let (sender, receiver) = watch::channel(false);
        (sender, Self(receiver))
    }
    pub fn check(&self) -> Result<(), HostError> {
        if *self.0.borrow() || self.0.has_changed().is_err() {
            Err(HostError::Cancelled)
        } else {
            Ok(())
        }
    }
    pub async fn cancelled(&mut self) {
        while self.check().is_ok() {
            if self.0.changed().await.is_err() {
                break;
            }
        }
    }
}
