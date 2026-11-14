//! Одноразовый instance-bound барьер consumer; только отдельный test-support стенд.
use super::{Cancellation, HostError};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleStage {
    Validate,
    Apply,
}

type Held = (
    LifecycleStage,
    oneshot::Sender<(u64, u32)>,
    oneshot::Receiver<()>,
);
type Fixture = (
    LifecycleGate,
    oneshot::Receiver<(u64, u32)>,
    oneshot::Sender<()>,
    oneshot::Receiver<u64>,
);
#[derive(Clone, Debug)]
pub struct LifecycleGate {
    held: Arc<Mutex<Option<Held>>>,
    stopped: Arc<Mutex<Option<oneshot::Sender<u64>>>>,
}
impl LifecycleGate {
    pub fn new(stage: LifecycleStage) -> Fixture {
        let (reached, receiver) = oneshot::channel();
        let (release, held) = oneshot::channel();
        let (stopped, ack) = oneshot::channel();
        (
            Self {
                held: Arc::new(Mutex::new(Some((stage, reached, held)))),
                stopped: Arc::new(Mutex::new(Some(stopped))),
            },
            receiver,
            release,
            ack,
        )
    }
    /// Наблюдение только после успешного join actual producer и cleanup.
    pub fn confirm_stopped(&self, generation: u64) {
        if let Ok(mut stopped) = self.stopped.lock() {
            if let Some(stopped) = stopped.take() {
                let _ = stopped.send(generation);
            }
        }
    }
    pub(super) async fn reach(
        &self,
        stage: LifecycleStage,
        pid: u32,
        generation: u64,
        cancel: &mut Cancellation,
    ) -> Result<(), HostError> {
        let gate = {
            let mut guard = self.held.lock().map_err(|_| HostError::State)?;
            if guard.as_ref().is_some_and(|entry| entry.0 == stage) {
                guard.take()
            } else {
                None
            }
        };
        if let Some((_, reached, release)) = gate {
            reached
                .send((generation, pid))
                .map_err(|_| HostError::State)?;
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(HostError::Cancelled),
                result = release => result.map_err(|_| HostError::State)?,
            }
        }
        cancel.check()
    }
}
