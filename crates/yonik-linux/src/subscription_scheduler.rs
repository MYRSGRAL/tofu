//! Событийный исполнитель: host объединяет next_deadline с сетевыми событиями.
//! Один владелец управляет schedule/cancel и review; фоновых таймеров нет.
use crate::{
    PendingRefresh, RefreshResult, ScheduleState, SubscriptionError, SubscriptionFetch,
    SubscriptionService,
};
use futures_util::{StreamExt, future::BoxFuture, stream::FuturesUnordered};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU64,
    sync::Arc,
    time::Duration,
};
use tokio::sync::watch;
pub use yonik_subscription::scheduler::Error as SubscriptionSchedulerError;
use yonik_subscription::scheduler::{Error, Policy, Scheduler, Work};

/// Не содержит URL, имён, тела ответа или диагностик транспорта.
#[derive(Debug, PartialEq, Eq)]
pub enum SubscriptionScheduleEvent {
    Review { id: i64 },
    NotModified { id: i64 },
    Failed { id: i64, error: SubscriptionError },
    Cancelled { id: i64 },
}
struct Active {
    service: SubscriptionService,
    work: Work,
    cancel: watch::Sender<bool>,
    cancelled: bool,
    review: Option<Box<PendingRefresh>>,
    decision: Option<BoxFuture<'static, Decision>>,
}
type Decision = (Result<Option<i64>, SubscriptionError>, bool);
type Completion = (Work, Result<RefreshResult, SubscriptionError>, bool);

pub struct SubscriptionScheduler {
    wall_clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    scheduler: Scheduler,
    scheduled: BTreeSet<i64>,
    active: BTreeMap<i64, Active>,
    futures: FuturesUnordered<BoxFuture<'static, Completion>>,
}
impl SubscriptionScheduler {
    pub fn new(identity: NonZeroU64, capacity: usize, parallelism: usize) -> Result<Self, Error> {
        Self::with_wall_clock(
            identity,
            capacity,
            parallelism,
            Arc::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    .min(i64::MAX as u64 - u32::MAX as u64)
            }),
        )
    }
    pub fn with_wall_clock(
        identity: NonZeroU64,
        capacity: usize,
        parallelism: usize,
        wall_clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Result<Self, Error> {
        Ok(Self {
            wall_clock,
            scheduled: BTreeSet::new(),
            scheduler: Scheduler::new(identity, capacity, parallelism)?,
            active: BTreeMap::new(),
            futures: FuturesUnordered::new(),
        })
    }
    pub fn set_interval(&mut self, interval: Duration) -> Result<(), Error> {
        self.scheduler.set_interval(interval)
    }
    pub fn schedule(&mut self, id: i64, generation: i64, due: Duration) -> Result<(), Error> {
        self.scheduler.schedule(id, generation, due)?;
        self.scheduled.insert(id);
        Ok(())
    }
    pub fn restore(
        &mut self,
        id: i64,
        generation: i64,
        state: yonik_subscription::scheduler::RetryState,
    ) -> Result<(), Error> {
        self.scheduler.restore(id, generation, state)?;
        self.scheduled.insert(id);
        Ok(())
    }
    pub fn next_deadline(&self, policy: Policy) -> Option<Duration> {
        self.scheduler.next_deadline(policy)
    }
    pub fn resume(&mut self, id: i64, now: Duration) -> bool {
        self.scheduler.resume(id, now)
    }
    /// Вызывается на общем deadline или событии host. Futures исполняются в next_event.
    pub fn dispatch(
        &mut self,
        now: Duration,
        policy: Policy,
        service: &SubscriptionService,
        host: Arc<dyn SubscriptionFetch>,
    ) -> Result<(), Error> {
        while let Some(work) = self.scheduler.take_due(now, policy)? {
            let service = service.clone();
            let host = host.clone();
            let (cancel, receiver) = watch::channel(false);
            self.active.insert(
                work.id(),
                Active {
                    service: service.clone(),
                    work,
                    cancel,
                    cancelled: false,
                    review: None,
                    decision: None,
                },
            );
            let failures = self.scheduler.failures(work.id()).ok_or(Error::Stale)?;
            let wall = self.wall_clock.clone();
            self.futures.push(Box::pin(async move {
                if let Err(error) = persist(
                    &service,
                    work,
                    failures,
                    Some(work.interval().as_secs()),
                    wall(),
                )
                .await
                {
                    return (work, Err(error), false);
                }
                let cancelled = receiver.clone();
                let result = service
                    .refresh_with_host(work.id(), Some(work.generation()), host.as_ref(), receiver)
                    .await;
                if !*cancelled.borrow() && !matches!(result, Ok(RefreshResult::Pending(_))) {
                    let success = matches!(result, Ok(RefreshResult::NotModified));
                    let state = Scheduler::completion_state_with_interval(
                        failures,
                        Duration::ZERO,
                        success,
                        work.interval(),
                    );
                    let failures = state.failures;
                    let delay = state.due.map(|due| due.as_secs());
                    if let Err(error) = persist(&service, work, failures, delay, wall()).await {
                        return (work, Err(error), false);
                    }
                }
                (work, result, true)
            }));
        }
        Ok(())
    }
    /// До acknowledgement слот отменённого fetch остаётся занят.
    /// Для delete/change_source сначала cancel, затем операция сервиса; новое
    /// schedule разрешено после Cancelled. now задаёт host, включая время review.
    pub fn cancel(&mut self, id: i64, _now: Duration) -> Result<(), Error> {
        self.scheduled.remove(&id);
        if let Some(work) = self.scheduler.cancel(id) {
            if let Some(active) = self.active.get_mut(&id) {
                active.cancelled = true;
                active.cancel.send_replace(true);
                if let Some(decision) = active.decision.take() {
                    self.futures.push(Box::pin(async move {
                        let _ = decision.await;
                        (work, Err(SubscriptionError::StaleUpdate), true)
                    }));
                }
                if let Some(pending) = active.review.take() {
                    let service = active.service.clone();
                    self.futures.push(Box::pin(async move {
                        service.discard_refresh(*pending).await;
                        (work, Err(SubscriptionError::StaleUpdate), true)
                    }));
                }
            }
        }
        Ok(())
    }
    /// Только активная отмена требует acknowledgement; queued запись удалена сразу.
    pub fn cancellation_pending(&self, id: i64) -> bool {
        self.active.get(&id).is_some_and(|active| active.cancelled)
    }
    /// Возвращает один bounded event. Отмена ожидания безопасна; повторить при
    /// следующем событии. Нет отдельного task или очереди на каждый сервер.
    pub async fn next_event(
        &mut self,
        clock: impl Fn() -> Duration,
    ) -> Result<Option<SubscriptionScheduleEvent>, Error> {
        loop {
            let Some((work, result, persisted)) = self.futures.next().await else {
                return Ok(None);
            };
            let now = clock();
            let active = self.active.get_mut(&work.id()).ok_or(Error::Stale)?;
            if active.cancelled {
                if let Ok(RefreshResult::Pending(pending)) = result {
                    let service = active.service.clone();
                    self.futures.push(Box::pin(async move {
                        service.discard_refresh(*pending).await;
                        (work, Err(SubscriptionError::StaleUpdate), true)
                    }));
                    continue;
                }
                self.active.remove(&work.id());
                self.scheduler.complete(work, now, false)?;
                return Ok(Some(SubscriptionScheduleEvent::Cancelled { id: work.id() }));
            }
            let event = match result {
                Ok(RefreshResult::Pending(pending)) => {
                    active.review = Some(pending);
                    return Ok(Some(SubscriptionScheduleEvent::Review { id: work.id() }));
                }
                Ok(RefreshResult::NotModified) => {
                    SubscriptionScheduleEvent::NotModified { id: work.id() }
                }
                Err(error) => SubscriptionScheduleEvent::Failed {
                    id: work.id(),
                    error,
                },
            };
            self.active.remove(&work.id());
            if !persisted {
                self.scheduler.cancel(work.id());
            }
            self.scheduler.complete(
                work,
                now,
                matches!(event, SubscriptionScheduleEvent::NotModified { .. }),
            )?;
            return Ok(Some(event));
        }
    }
    pub fn review(&self, id: i64) -> Option<&PendingRefresh> {
        self.active.get(&id)?.review.as_deref()
    }
    /// Явное решение UI. Только approve=true разрешает commit показанного отчёта.
    pub async fn finish_review(
        &mut self,
        service: &SubscriptionService,
        id: i64,
        approve: bool,
        clock: impl Fn() -> Duration,
    ) -> Result<Option<i64>, SubscriptionError> {
        let active = self
            .active
            .get_mut(&id)
            .ok_or(SubscriptionError::StaleUpdate)?;
        if active.decision.is_none() {
            let pending = active.review.take().ok_or(SubscriptionError::StaleUpdate)?;
            let service = service.clone();
            let wall = self.wall_clock.clone();
            let work = active.work;
            let cancelled = active.cancel.subscribe();
            let failures = self
                .scheduler
                .failures(id)
                .ok_or(SubscriptionError::StaleUpdate)?;
            active.decision = Some(Box::pin(async move {
                let result = if approve {
                    service
                        .commit(pending.approve_after_review())
                        .await
                        .map(Some)
                } else {
                    service.discard_refresh(*pending).await;
                    Ok(None)
                };
                if *cancelled.borrow() {
                    return (result, true);
                }
                let state = Scheduler::completion_state_with_interval(
                    failures,
                    Duration::ZERO,
                    result.is_ok(),
                    work.interval(),
                );
                let failures = state.failures;
                let delay = state.due.map(|due| due.as_secs());
                if let Err(error) = persist(&service, work, failures, delay, wall()).await {
                    return (Err(error), false);
                }
                (result, true)
            }));
        }
        let (result, persisted) = active
            .decision
            .as_mut()
            .ok_or(SubscriptionError::StaleUpdate)?
            .await;
        let now = clock();
        let work = active.work;
        self.active.remove(&id);
        if !persisted {
            self.scheduler.cancel(id);
        }
        self.scheduler
            .complete(work, now, result.is_ok())
            .map_err(|_| SubscriptionError::StaleUpdate)?;
        result
    }
    /// Завершение host: отменить и дождаться acknowledgement всех fetch.
    pub async fn shutdown(&mut self, now: Duration) -> Result<(), Error> {
        let ids: Vec<_> = self.scheduled.iter().copied().collect();
        for id in ids {
            self.cancel(id, now)?;
        }
        while self.next_event(|| now).await?.is_some() {}
        Ok(())
    }
}

async fn persist(
    service: &SubscriptionService,
    work: Work,
    failures: u32,
    delay: Option<u64>,
    wall: u64,
) -> Result<(), SubscriptionError> {
    let token = service
        .schedule_metadata(work.id())
        .await?
        .ok_or(SubscriptionError::NotFound)?;
    if token.generation != work.generation() {
        return Err(SubscriptionError::StaleUpdate);
    }
    let due_wall = delay.and_then(|delay| wall.checked_add(delay));
    service
        .persist_schedule(
            token,
            ScheduleState {
                version: 1,
                recorded_wall: wall,
                due_wall,
                failures,
                paused: due_wall.is_none(),
            },
        )
        .await
}
