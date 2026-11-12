//! Каждая попытка initialize владеет отдельным pending: старая очередь не видит новую.
#[derive(Default)]
pub(super) struct PendingSnapshot {
    value: Option<String>,
    queued: bool,
}

impl PendingSnapshot {
    pub(super) fn publish(&mut self, snapshot: String) -> bool {
        self.value = Some(snapshot);
        if self.queued {
            return false;
        }
        self.queued = true;
        true
    }

    pub(super) fn take_queued(&mut self) -> Option<String> {
        self.queued = false;
        self.value.take()
    }

    pub(super) fn take_start_failure(&mut self) -> Option<String> {
        // Qt callback уже может ждать в очереди; после take он должен стать no-op.
        self.value.take()
    }

    pub(super) fn queue_rejected(&mut self) {
        // Синхронный start Err ещё может забрать typed failure после отказа queue.
        self.queued = false;
    }
}
