//! Событийный исполнитель: host объединяет next_deadline с сетевыми событиями.
//! Один владелец управляет schedule/cancel и review; фоновых таймеров нет.
use crate::{
    PendingRefresh, RefreshResult, SubscriptionError, SubscriptionFetch, SubscriptionService,
};
use futures_util::{StreamExt, future::BoxFuture, stream::FuturesUnordered};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU64,
    sync::Arc,
    time::Duration,
};
use tokio::sync::watch;
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
    decision: Option<BoxFuture<'static, Result<Option<i64>, SubscriptionError>>>,
}
type Completion = (Work, Result<RefreshResult, SubscriptionError>);

pub struct SubscriptionScheduler {
    scheduler: Scheduler,
    scheduled: BTreeSet<i64>,
    active: BTreeMap<i64, Active>,
    futures: FuturesUnordered<BoxFuture<'static, Completion>>,
}
impl SubscriptionScheduler {
    pub fn new(identity: NonZeroU64, capacity: usize, parallelism: usize) -> Result<Self, Error> {
        Ok(Self {
            scheduled: BTreeSet::new(),
            scheduler: Scheduler::new(identity, capacity, parallelism)?,
            active: BTreeMap::new(),
            futures: FuturesUnordered::new(),
        })
    }
    pub fn schedule(&mut self, id: i64, generation: i64, due: Duration) -> Result<(), Error> {
        self.scheduler.schedule(id, generation, due)?;
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
            self.futures.push(Box::pin(async move {
                let result = service
                    .refresh_with_host(work.id(), Some(work.generation()), host.as_ref(), receiver)
                    .await;
                (work, result)
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
                        (work, Err(SubscriptionError::StaleUpdate))
                    }));
                }
                if let Some(pending) = active.review.take() {
                    let service = active.service.clone();
                    self.futures.push(Box::pin(async move {
                        service.discard_refresh(*pending).await;
                        (work, Err(SubscriptionError::StaleUpdate))
                    }));
                }
            }
        }
        Ok(())
    }
    /// Возвращает один bounded event. Отмена ожидания безопасна; повторить при
    /// следующем событии. Нет отдельного task или очереди на каждый сервер.
    pub async fn next_event(
        &mut self,
        clock: impl Fn() -> Duration,
    ) -> Result<Option<SubscriptionScheduleEvent>, Error> {
        loop {
            let Some((work, result)) = self.futures.next().await else {
                return Ok(None);
            };
            let now = clock();
            let active = self.active.get_mut(&work.id()).ok_or(Error::Stale)?;
            if active.cancelled {
                if let Ok(RefreshResult::Pending(pending)) = result {
                    let service = active.service.clone();
                    self.futures.push(Box::pin(async move {
                        service.discard_refresh(*pending).await;
                        (work, Err(SubscriptionError::StaleUpdate))
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
        now: Duration,
    ) -> Result<Option<i64>, SubscriptionError> {
        let active = self
            .active
            .get_mut(&id)
            .ok_or(SubscriptionError::StaleUpdate)?;
        if active.decision.is_none() {
            let pending = active.review.take().ok_or(SubscriptionError::StaleUpdate)?;
            let service = service.clone();
            active.decision = Some(Box::pin(async move {
                if approve {
                    service
                        .commit(pending.approve_after_review())
                        .await
                        .map(Some)
                } else {
                    service.discard_refresh(*pending).await;
                    Ok(None)
                }
            }));
        }
        let result = active
            .decision
            .as_mut()
            .ok_or(SubscriptionError::StaleUpdate)?
            .await;
        let work = active.work;
        self.active.remove(&id);
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
