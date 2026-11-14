//! Собственный private fixture: никаких caller-supplied путей, fd и I/O callbacks.
use super::{DiskLogGuard, DiskLogStats, start_inner};
use crate::event_log::EventLog;
use std::{
    io,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
pub(super) struct AfterOpen {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}
impl AfterOpen {
    pub(super) fn enter(&self) {
        let mut state = self.state.lock().expect("fixture barrier");
        state.0 = true;
        self.changed.notify_all();
        let _guard = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(30), |state| !state.1)
            .expect("bounded fixture barrier");
    }
    pub(super) fn release(&self) {
        let mut state = self.state.lock().expect("fixture release");
        state.1 = true;
        self.changed.notify_all();
    }
}
/// Drop освобождает after-open, запрашивает shutdown и ограниченно ждёт cleanup.
/// При непрекращающемся kernel I/O private root сохраняется для диагностики.
pub struct DiskFixture {
    path: PathBuf,
    barrier: Arc<AfterOpen>,
    sink: Option<(EventLog, DiskLogGuard)>,
    observer: DiskLogGuard,
}
impl DiskFixture {
    pub fn new() -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let home =
            std::env::var_os("HOME").ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let path = PathBuf::from(home).join(format!(
            ".yonik-disk-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        let barrier = Arc::new(AfterOpen::default());
        let (producer, guard) = start_inner(path.clone(), Some(Arc::clone(&barrier)));
        let observer = DiskLogGuard {
            tx: guard.tx.clone(),
            state: Arc::clone(&guard.state),
            thread: None,
        };
        Ok(Self {
            path,
            barrier,
            sink: Some((EventLog::build(Some(producer)), guard)),
            observer,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn take_sink(&mut self) -> Option<(EventLog, DiskLogGuard)> {
        self.sink.take()
    }
    pub fn wait_opened(&self, timeout: Duration) -> bool {
        let Ok(state) = self.barrier.state.lock() else {
            return false;
        };
        self.barrier
            .changed
            .wait_timeout_while(state, timeout, |state| !state.0)
            .is_ok_and(|(state, _)| state.0)
    }
    pub fn release(&self) {
        self.barrier.release();
    }
    pub fn stats(&self) -> DiskLogStats {
        self.observer.stats()
    }
    pub fn shutdown_requested(&self) -> bool {
        self.observer.state.stop.load(Ordering::Acquire)
    }
    pub fn wait_consumer_closed(&self, timeout: Duration) -> bool {
        self.observer.wait_closed(timeout)
    }
}
impl Drop for DiskFixture {
    fn drop(&mut self) {
        self.release();
        self.observer.request_shutdown();
        if self.observer.wait_closed(Duration::from_secs(5)) {
            // Только собственный, созданный new() root; host должен быть закрыт caller-ом.
            if let Err(error) = std::fs::remove_dir_all(&self.path) {
                eprintln!(
                    "Не удалось очистить собственный fixture {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}
