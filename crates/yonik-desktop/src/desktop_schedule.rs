use super::{DesktopError, Result};
use std::{
    collections::BTreeMap,
    num::NonZeroU64,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use yonik_linux::{
    SubscriptionPolicy, SubscriptionScheduleEvent, SubscriptionScheduler, SubscriptionService,
};

/// Один bounded владелец; один fetch/review исключает очередь скрытых отчётов.
pub(super) struct DesktopSchedule {
    pub runtime: SubscriptionScheduler,
    generations: BTreeMap<i64, i64>,
    pub policy: SubscriptionPolicy,
    pub visible: bool,
    pub review: Option<i64>,
    pub fetching: bool,
    pub deferred: Option<SubscriptionScheduleEvent>,
}
impl DesktopSchedule {
    pub fn new() -> Result<Self> {
        static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
        let identity = NEXT_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .ok()
            .and_then(NonZeroU64::new)
            .ok_or(DesktopError("startup"))?;
        Ok(Self {
            runtime: SubscriptionScheduler::new(identity, 20_000, 1)
                .map_err(|_| DesktopError("startup"))?,
            generations: BTreeMap::new(),
            policy: SubscriptionPolicy::default(),
            visible: false,
            review: None,
            fetching: false,
            deferred: None,
        })
    }
    pub fn policy(&self) -> SubscriptionPolicy {
        SubscriptionPolicy {
            ui_visible: self.visible,
            background_window: false,
            ..self.policy
        }
    }
    pub fn deadline(&self) -> Option<Duration> {
        self.runtime.next_deadline(self.policy())
    }
    pub async fn load(&mut self, service: &SubscriptionService, now: Duration) -> Result<()> {
        let mut offset = 0;
        loop {
            let page = service
                .schedule_sources(offset)
                .await
                .map_err(super::map_subscription)?;
            let count = page.len();
            for (id, generation) in page {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    self.generations.entry(id)
                {
                    self.runtime
                        .schedule(id, generation, now)
                        .map_err(|_| DesktopError("storage"))?;
                    entry.insert(generation);
                }
            }
            if count < 64 {
                break;
            }
            offset += count;
            tokio::task::yield_now().await;
        }
        Ok(())
    }
    pub async fn reload_id(
        &mut self,
        service: &SubscriptionService,
        id: i64,
        due: Duration,
    ) -> Result<()> {
        if let Some(generation) = service
            .schedule_generation(id)
            .await
            .map_err(super::map_subscription)?
        {
            if let std::collections::btree_map::Entry::Vacant(entry) = self.generations.entry(id) {
                self.runtime
                    .schedule(id, generation, due)
                    .map_err(|_| DesktopError("storage"))?;
                entry.insert(generation);
            }
        }
        Ok(())
    }
    pub async fn cancel(&mut self, id: i64, clock: impl Fn() -> Duration) -> Result<()> {
        self.runtime
            .cancel(id, clock())
            .map_err(|_| DesktopError("storage"))?;
        // Queued ID уже удалён: чужой fetch остаётся в общем select, где доступны
        // приоритетные Stop/Shutdown. Только собственная active/review работа ждёт ack.
        if self.runtime.cancellation_pending(id) {
            if let Some(event) = self
                .runtime
                .next_event(clock)
                .await
                .map_err(|_| DesktopError("storage"))?
            {
                if !matches!(event, SubscriptionScheduleEvent::Cancelled { id: current } if current == id)
                {
                    self.deferred = Some(event);
                }
            }
            self.fetching = false;
        }
        if self.review == Some(id) {
            self.review = None;
        }
        self.generations.remove(&id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn неизвестная_policy_не_даёт_deadline() {
        let mut owner = DesktopSchedule::new().expect("scheduler");
        owner
            .runtime
            .schedule(1, 1, Duration::ZERO)
            .expect("schedule");
        assert_eq!(owner.deadline(), None);
        owner.visible = true;
        assert_eq!(owner.deadline(), None);
        owner.policy = SubscriptionPolicy {
            online: true,
            ..Default::default()
        };
        assert_eq!(owner.deadline(), Some(Duration::ZERO));
        owner.policy.metered = true;
        assert_eq!(owner.deadline(), None);
        owner.policy.metered = false;
        owner.policy.battery_saver = true;
        assert_eq!(owner.deadline(), None);
        owner.policy.battery_saver = false;
        owner.visible = false;
        assert_eq!(owner.deadline(), None);
    }
}
