//! Ожидание existing NetworkMonitor без фоновой задачи, очереди и polling.
use super::{PrivateBindings, current};
use crate::{Cancellation, network::NetworkWatch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingInvalidation {
    NetworkChanged,
    ObservationLost,
    Cancelled,
}

impl PrivateBindings {
    /// Защёлкивает invalidation до возврата: прежний witness не оживает даже
    /// при восстановлении topology. Host затем обязан применить resolve guard.
    /// Метод не владеет monitor; его bounded shutdown остаётся у владельца.
    pub async fn wait_invalidation(
        &mut self,
        watch: &mut NetworkWatch,
        cancel: &mut Cancellation,
    ) -> BindingInvalidation {
        let reason = loop {
            if cancel.check().is_err() {
                break BindingInvalidation::Cancelled;
            }
            let snapshot = watch.current();
            if !snapshot.coverage.routes || snapshot.exhausted {
                break BindingInvalidation::ObservationLost;
            }
            if self.invalidated.load(std::sync::atomic::Ordering::Acquire)
                || !current(snapshot, self.stamp)
            {
                break BindingInvalidation::NetworkChanged;
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break BindingInvalidation::Cancelled,
                changed = watch.changed() => {
                    if changed.is_err() { break BindingInvalidation::ObservationLost; }
                }
            }
        };
        self.invalidated
            .store(true, std::sync::atomic::Ordering::Release);
        reason
    }
}
