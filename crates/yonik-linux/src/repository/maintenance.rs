//! Непрозрачный доступ к maintenance без передачи ключа desktop-слою.
use super::*;

/// Не содержит SQLite handle и намеренно не реализует Debug/Serialize/Clone.
/// Владелец обязан закрыть все repository handles до запуска копирования.
pub struct MaintenanceAccess {
    source: PathBuf,
    key: Zeroizing<[u8; 32]>,
}

impl EncryptedSubscriptionRepository {
    pub async fn maintenance_access(&self) -> Result<MaintenanceAccess> {
        self.run(|database| {
            let source = database
                .connection
                .path()
                .ok_or(RepositoryError::UnsafePath)?;
            let source = PathBuf::from(source);
            secure_path(&source)?;
            if !source.is_absolute() {
                return Err(RepositoryError::UnsafePath);
            }
            Ok(MaintenanceAccess {
                source,
                key: database.key.clone(),
            })
        })
        .await
    }
}

impl MaintenanceAccess {
    /// Путь получен из открытого соединения, а не из GUI-запроса.
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// Reopen не получает instance lock повторно: его удерживает supervisor.
    pub async fn reopen(&self) -> Result<EncryptedSubscriptionRepository> {
        EncryptedSubscriptionRepository::open(self.source.clone(), self.key.clone()).await
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaintenanceError {
    Cancelled,
    Repository(RepositoryError),
}

use std::sync::atomic::{AtomicU8, Ordering};

const ACTIVE: u8 = 0;
const CANCELLED: u8 = 1;
const PUBLISHING: u8 = 2;
const PUBLISHED: u8 = 3;

#[derive(Clone)]
pub struct MaintenanceControl(Arc<Control>);
struct Control {
    admission: AtomicU8,
    state: Mutex<ControlState>,
}
struct ControlState {
    cleanup_failed: bool,
    interrupt: Option<rusqlite::InterruptHandle>,
}
// Drop снимает interrupt handle ДО sqlite3_close: cancel не попадает в
// внутренний mutex rusqlite, удерживаемый во время checkpoint при close.
pub(super) struct InterruptScope<'a>(&'a MaintenanceControl);
impl Drop for InterruptScope<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.0.state.lock() {
            state.interrupt = None;
        }
    }
}
impl Default for MaintenanceControl {
    fn default() -> Self {
        Self(Arc::new(Control {
            admission: AtomicU8::new(ACTIVE),
            state: Mutex::new(ControlState {
                cleanup_failed: false,
                interrupt: None,
            }),
        }))
    }
}
impl MaintenanceControl {
    pub(super) fn interrupt_scope(&self) -> InterruptScope<'_> {
        InterruptScope(self)
    }
    /// Отмена не ждёт mutex или syscall публикации. CAS задаёт её границу.
    pub fn cancel(&self) {
        let _admission = self.0.admission.compare_exchange(
            ACTIVE,
            CANCELLED,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        if self.0.admission.load(Ordering::SeqCst) == CANCELLED {
            if let Ok(state) = self.0.state.try_lock() {
                if let Some(interrupt) = &state.interrupt {
                    interrupt.interrupt();
                }
            }
        }
    }
    pub(super) fn check(&self) -> Result<()> {
        if self.0.admission.load(Ordering::SeqCst) == CANCELLED {
            Err(RepositoryError::Storage)
        } else {
            Ok(())
        }
    }
    pub(super) fn connection(&self, connection: &Connection) -> Result<()> {
        self.check()?;
        self.0
            .state
            .lock()
            .map_err(|_| RepositoryError::Storage)?
            .interrupt = Some(connection.get_interrupt_handle());
        self.check()
    }
    pub(super) fn publish(&self, operation: impl FnOnce() -> Result<()>) -> Result<()> {
        self.0
            .admission
            .compare_exchange(ACTIVE, PUBLISHING, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| RepositoryError::Storage)?;
        operation()?;
        self.0.admission.store(PUBLISHED, Ordering::SeqCst);
        Ok(())
    }
    pub(super) fn cleanup_result(&self, result: &Result<()>) {
        if result.is_err() {
            if let Ok(mut state) = self.0.state.lock() {
                state.cleanup_failed = true;
            }
        }
    }
    fn result(&self, result: Result<()>) -> std::result::Result<(), MaintenanceError> {
        let mut state = self
            .0
            .state
            .lock()
            .map_err(|_| MaintenanceError::Repository(RepositoryError::Storage))?;
        state.interrupt = None;
        if state.cleanup_failed {
            Err(MaintenanceError::Repository(RepositoryError::Storage))
        } else if self.0.admission.load(Ordering::SeqCst) == CANCELLED {
            Err(MaintenanceError::Cancelled)
        } else {
            result.map_err(MaintenanceError::Repository)
        }
    }
}
impl MaintenanceAccess {
    /// Вызывается только после ACK закрытия всех repository handles.
    /// Один AEAD-вызов ограничен размером записи, но не прерывается.
    pub async fn copy_closed(
        &self,
        candidate: Option<PathBuf>,
        destination: PathBuf,
        control: MaintenanceControl,
    ) -> std::result::Result<(), MaintenanceError> {
        let source = candidate.unwrap_or_else(|| self.source.clone());
        if destination == self.source || destination == source {
            return Err(MaintenanceError::Repository(RepositoryError::UnsafePath));
        }
        let permit = tokio::time::timeout(std::time::Duration::from_secs(30), OPEN_GATE.acquire())
            .await
            .map_err(|_| MaintenanceError::Repository(RepositoryError::Storage))?
            .map_err(|_| MaintenanceError::Repository(RepositoryError::Storage))?;
        let key = self.key.clone();
        let worker_control = control.clone();
        let mut task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let result =
                super::backup::copy_controlled(&source, &destination, &key, &worker_control);
            worker_control.result(result)
        });
        tokio::select! {
            result = &mut task => result.map_err(|_| MaintenanceError::Repository(RepositoryError::Storage))?,
            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                control.cancel();
                task.await.map_err(|_| MaintenanceError::Repository(RepositoryError::Storage))?
            }
        }
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;
    #[test]
    fn cancellation_never_waits_for_admitted_publication() {
        let control = MaintenanceControl::default();
        let worker_control = control.clone();
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            worker_control
                .publish(|| {
                    entered.send(()).expect("admitted");
                    released
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .expect("release");
                    Ok(())
                })
                .expect("publication");
            worker_control.result(Ok(()))
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("admission");
        let before = std::time::Instant::now();
        control.cancel();
        assert!(before.elapsed() < std::time::Duration::from_millis(100));
        release.send(()).expect("release");
        assert_eq!(worker.join().expect("join"), Ok(()));
    }
    #[test]
    fn cleanup_failure_during_cancel_remains_storage_uncertainty() {
        let control = MaintenanceControl::default();
        control.cancel();
        control.cleanup_result(&Err(RepositoryError::Storage));
        assert_eq!(
            control.result(Err(RepositoryError::Storage)),
            Err(MaintenanceError::Repository(RepositoryError::Storage))
        );
    }
    #[test]
    fn published_result_survives_late_cancel() {
        let control = MaintenanceControl::default();
        control.publish(|| Ok(())).expect("publication fixture");
        control.cancel();
        assert_eq!(control.result(Ok(())), Ok(()));
        assert_eq!(
            control.result(Err(RepositoryError::Storage)),
            Err(MaintenanceError::Repository(RepositoryError::Storage))
        );
    }
}
