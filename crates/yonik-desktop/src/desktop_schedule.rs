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
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.load_at(service, now, wall).await
    }
    pub async fn load_at(
        &mut self,
        service: &SubscriptionService,
        now: Duration,
        wall: u64,
    ) -> Result<()> {
        let mut offset = 0;
        loop {
            let page = service
                .schedule_metadata_page(offset)
                .await
                .map_err(super::map_subscription)?;
            let count = page.len();
            for token in page {
                let id = token.id;
                let generation = token.generation;
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    self.generations.entry(id)
                {
                    let state = match token.state {
                        Some(state) if wall < state.recorded_wall => {
                            let remaining = state
                                .due_wall
                                .map(|due| due.saturating_sub(state.recorded_wall).min(86400));
                            let state = yonik_linux::ScheduleState {
                                recorded_wall: wall,
                                due_wall: remaining.and_then(|delay| wall.checked_add(delay)),
                                ..state
                            };
                            service
                                .persist_schedule(token, state)
                                .await
                                .map_err(super::map_subscription)?;
                            Some(state)
                        }
                        Some(state) => Some(state),
                        None => {
                            let state = yonik_linux::ScheduleState {
                                version: 1,
                                recorded_wall: wall,
                                due_wall: wall.checked_add(60 + (id as u64 % 86341)),
                                failures: 0,
                                paused: false,
                            };
                            service
                                .persist_schedule(token, state)
                                .await
                                .map_err(super::map_subscription)?;
                            Some(state)
                        }
                    };
                    let due = state
                        .map(|state| state.deadline(wall, now))
                        .unwrap_or_else(|| {
                            now.saturating_add(Duration::from_secs(60 + (id as u64 % 86341)))
                        });
                    self.runtime
                        .restore(
                            id,
                            generation,
                            yonik_linux::SubscriptionRetryState {
                                due: if state.is_some_and(|state| state.paused) {
                                    None
                                } else {
                                    Some(due)
                                },
                                failures: state.map_or(0, |state| state.failures),
                            },
                        )
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
        now: Duration,
    ) -> Result<()> {
        if let Some(token) = service
            .schedule_metadata(id)
            .await
            .map_err(super::map_subscription)?
        {
            let generation = token.generation;
            let wall = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let state = token.state;
            if state.is_none() {
                service
                    .persist_schedule(
                        token,
                        yonik_linux::ScheduleState {
                            version: 1,
                            recorded_wall: wall,
                            due_wall: wall.checked_add(86400),
                            failures: 0,
                            paused: false,
                        },
                    )
                    .await
                    .map_err(super::map_subscription)?;
            }
            let due = state.map_or_else(
                || now.saturating_add(Duration::from_secs(86400)),
                |state| state.deadline(wall, now),
            );
            if let std::collections::btree_map::Entry::Vacant(entry) = self.generations.entry(id) {
                self.runtime
                    .restore(
                        id,
                        generation,
                        yonik_linux::SubscriptionRetryState {
                            due: if state.is_some_and(|state| state.paused) {
                                None
                            } else {
                                Some(due)
                            },
                            failures: state.map_or(0, |state| state.failures),
                        },
                    )
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

    #[tokio::test]
    async fn startup_восстанавливает_due_и_нормализует_rollback() {
        use std::os::unix::fs::DirBuilderExt;
        let path = std::env::temp_dir().join(format!("yonik-due-{}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let repo = yonik_linux::EncryptedSubscriptionRepository::open(
            path.join("db"),
            zeroize::Zeroizing::new([7; 32]),
        )
        .await
        .unwrap();
        let service = SubscriptionService::new(repo);
        let id = service
            .add(
                yonik_linux::SubscriptionSource::new(
                    "Тест".into(),
                    "http://127.0.0.1:9/sub".into(),
                    true,
                    yonik_linux::FetchRoute::Direct,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let mut initial = DesktopSchedule::new().unwrap();
        initial
            .load_at(&service, Duration::ZERO, 100)
            .await
            .unwrap();
        let state = service
            .schedule_metadata(id)
            .await
            .unwrap()
            .unwrap()
            .state
            .unwrap();
        assert!(state.due_wall.unwrap() > 100);
        let token = service.schedule_metadata(id).await.unwrap().unwrap();
        service
            .persist_schedule(
                token,
                yonik_linux::ScheduleState {
                    version: 1,
                    recorded_wall: 100,
                    due_wall: Some(86500),
                    failures: 3,
                    paused: false,
                },
            )
            .await
            .unwrap();
        let mut reopened = DesktopSchedule::new().unwrap();
        reopened
            .load_at(&service, Duration::from_secs(5), 101)
            .await
            .unwrap();
        reopened.visible = true;
        reopened.policy.online = true;
        assert_eq!(reopened.deadline(), Some(Duration::from_secs(86404)));
        let mut rolled = DesktopSchedule::new().unwrap();
        rolled.load_at(&service, Duration::ZERO, 0).await.unwrap();
        let normalized = service
            .schedule_metadata(id)
            .await
            .unwrap()
            .unwrap()
            .state
            .unwrap();
        assert_eq!(normalized.recorded_wall, 0);
        assert_eq!(normalized.due_wall, Some(86400));
        let mut again = DesktopSchedule::new().unwrap();
        again.load_at(&service, Duration::ZERO, 10).await.unwrap();
        again.visible = true;
        again.policy.online = true;
        assert_eq!(again.deadline(), Some(Duration::from_secs(86390)));
        service.close().await.unwrap();
        std::fs::remove_dir_all(path).unwrap();
    }

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
