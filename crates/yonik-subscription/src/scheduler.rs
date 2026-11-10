//! Чистое планирование автоматических обновлений. Host передаёт монотонное время,
//! исполняет выданные работы и подтверждает даже отменённые завершения.

use std::{collections::BTreeMap, num::NonZeroU64, time::Duration};

/// Политика приходит событиями host; видимость UI сама не создаёт работу.
#[derive(Debug, Clone, Copy, Default)]
pub struct Policy {
    pub online: bool,
    pub metered: bool,
    pub battery_saver: bool,
    pub ui_visible: bool,
    pub background_window: bool,
}
impl Policy {
    fn allowed(self) -> bool {
        self.online
            && !self.metered
            && !self.battery_saver
            && (self.ui_visible || self.background_window)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidLimits,
    Full,
    Duplicate,
    Exhausted,
    Stale,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidLimits => "Некорректные лимиты планировщика",
            Self::Full => "Очередь обновлений заполнена",
            Self::Duplicate => "Подписка уже запланирована",
            Self::Exhausted => "Исчерпаны идентификаторы работ",
            Self::Stale => "Завершение работы устарело",
        })
    }
}
impl std::error::Error for Error {}

/// Токен планирования, не заменяет Ticket импорта или CAS Repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    scheduler: NonZeroU64,
    serial: u64,
    id: i64,
    generation: i64,
}
impl Work {
    pub fn id(self) -> i64 {
        self.id
    }
    pub fn generation(self) -> i64 {
        self.generation
    }
}

#[derive(Debug)]
struct Entry {
    generation: i64,
    due: Option<Duration>,
    failures: u32,
    active: Option<Work>,
    cancelled: bool,
}

/// Один экземпляр на controller. `identity` уникален среди экземпляров, callbacks
/// которых ещё живы. ID записей i64 совпадают с SubscriptionService.
/// Лимит включает ожидающие, активные и отменённые, но ещё не завершённые работы.
#[derive(Debug)]
pub struct Scheduler {
    identity: NonZeroU64,
    capacity: usize,
    parallelism: usize,
    serial: u64,
    entries: BTreeMap<i64, Entry>,
}
impl Scheduler {
    pub const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
    pub const RETRY_CAP: Duration = Duration::from_secs(60);
    pub const MAX_FAILURES: u32 = 5;

    pub fn new(identity: NonZeroU64, capacity: usize, parallelism: usize) -> Result<Self, Error> {
        if capacity == 0 || parallelism == 0 || parallelism > capacity {
            return Err(Error::InvalidLimits);
        }
        Ok(Self {
            identity,
            capacity,
            parallelism,
            serial: 0,
            entries: BTreeMap::new(),
        })
    }

    /// `due` — первое разрешённое время; host восстанавливает его из метаданных.
    /// Повторное добавление не меняет уже ожидающую/активную работу.
    pub fn schedule(&mut self, id: i64, generation: i64, due: Duration) -> Result<(), Error> {
        if self.entries.contains_key(&id) {
            return Err(Error::Duplicate);
        }
        if self.entries.len() >= self.capacity {
            return Err(Error::Full);
        }
        self.entries.insert(
            id,
            Entry {
                generation,
                due: Some(due),
                failures: 0,
                active: None,
                cancelled: false,
            },
        );
        Ok(())
    }

    /// Host отменяет соответствующий fetch. До acknowledgement новый fetch того
    /// же ID запрещён; смена source/delete использует этот же путь.
    pub fn cancel(&mut self, id: i64) -> Option<Work> {
        let entry = self.entries.get_mut(&id)?;
        if let Some(work) = entry.active {
            entry.cancelled = true;
            entry.due = None;
            Some(work)
        } else {
            self.entries.remove(&id);
            None
        }
    }

    /// Один общий deadline; None означает ожидание host-события/завершения.
    pub fn next_deadline(&self, policy: Policy) -> Option<Duration> {
        if !policy.allowed()
            || self.entries.values().filter(|e| e.active.is_some()).count() >= self.parallelism
        {
            return None;
        }
        self.entries
            .values()
            .filter(|e| e.active.is_none())
            .filter_map(|e| e.due)
            .min()
    }

    /// Выдаёт одну работу без выделения очереди результатов. Host повторяет до None.
    pub fn take_due(&mut self, now: Duration, policy: Policy) -> Result<Option<Work>, Error> {
        if self.next_deadline(policy).is_none_or(|due| due > now) {
            return Ok(None);
        }
        let candidate = self
            .entries
            .iter()
            .filter(|(_, e)| e.active.is_none())
            .filter_map(|(&id, e)| e.due.filter(|due| *due <= now).map(|due| (due, id)))
            .min();
        let Some((_, id)) = candidate else {
            return Ok(None);
        };
        let serial = self.serial.checked_add(1).ok_or(Error::Exhausted)?;
        let entry = self.entries.get_mut(&id).ok_or(Error::Stale)?;
        let work = Work {
            scheduler: self.identity,
            serial,
            id,
            generation: entry.generation,
        };
        self.serial = serial;
        entry.active = Some(work);
        entry.due = None;
        Ok(Some(work))
    }

    /// Вызывается после завершения всей service-операции, включая review/commit.
    /// Не публикует snapshot: его по-прежнему защищают Subscription и Repository.
    /// После 5 ошибок или overflow времени требуется явный resume.
    pub fn complete(&mut self, work: Work, now: Duration, success: bool) -> Result<(), Error> {
        let entry = self.entries.get_mut(&work.id).ok_or(Error::Stale)?;
        if entry.active != Some(work) {
            return Err(Error::Stale);
        }
        if entry.cancelled {
            self.entries.remove(&work.id);
            return Ok(());
        }
        entry.active = None;
        let delay = if success {
            entry.failures = 0;
            Some(Self::INTERVAL)
        } else {
            entry.failures = entry.failures.saturating_add(1);
            (entry.failures < Self::MAX_FAILURES).then(|| Self::retry_delay(entry.failures))
        };
        entry.due = delay.and_then(|delay| now.checked_add(delay));
        Ok(())
    }

    fn retry_delay(failures: u32) -> Duration {
        Duration::from_secs(
            1_u64
                .checked_shl(failures.saturating_sub(1))
                .unwrap_or(u64::MAX),
        )
        .min(Self::RETRY_CAP)
    }

    /// Явное новое событие сети/действие пользователя после исчерпания retries.
    /// Не сокращает обычный интервал и не прерывает активную работу.
    pub fn resume(&mut self, id: i64, now: Duration) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        if entry.active.is_some() || entry.due.is_some() {
            return false;
        }
        entry.failures = 0;
        entry.due = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, Policy, Scheduler};
    use std::{num::NonZeroU64, time::Duration};

    fn scheduler() -> Scheduler {
        // Константа заведомо ненулевая.
        Scheduler::new(NonZeroU64::new(1).unwrap(), 2, 1).unwrap()
    }
    fn allowed() -> Policy {
        Policy {
            online: true,
            ui_visible: true,
            ..Policy::default()
        }
    }

    #[test]
    fn due_and_daily_interval() {
        let mut s = scheduler();
        s.schedule(10, 7, Duration::from_secs(10)).unwrap();
        assert_eq!(s.take_due(Duration::from_secs(9), allowed()), Ok(None));
        let w = s
            .take_due(Duration::from_secs(10), allowed())
            .unwrap()
            .unwrap();
        assert_eq!((w.id(), w.generation()), (10, 7));
        assert_eq!(s.take_due(Duration::MAX, allowed()), Ok(None));
        s.complete(w, Duration::from_secs(20), true).unwrap();
        assert_eq!(
            s.next_deadline(allowed()),
            Some(Duration::from_secs(20) + Scheduler::INTERVAL)
        );
        assert!(!s.resume(10, Duration::ZERO));
    }

    #[test]
    fn retry_budget_and_resume() {
        let mut s = scheduler();
        s.schedule(1, 0, Duration::ZERO).unwrap();
        let mut now = Duration::ZERO;
        for failure in 1..=5 {
            let w = s.take_due(now, allowed()).unwrap().unwrap();
            s.complete(w, now, false).unwrap();
            if failure < 5 {
                now += Duration::from_secs(1 << (failure - 1));
                assert_eq!(s.next_deadline(allowed()), Some(now));
            }
        }
        assert_eq!(s.next_deadline(allowed()), None);
        assert!(s.resume(1, now));
        assert!(s.take_due(now, allowed()).unwrap().is_some());
        assert_eq!(Scheduler::retry_delay(u32::MAX), Scheduler::RETRY_CAP);
        assert_eq!(Scheduler::retry_delay(7), Scheduler::RETRY_CAP);
    }

    #[test]
    fn time_and_serial_overflow_do_not_wrap() {
        let mut s = scheduler();
        s.schedule(1, 0, Duration::ZERO).unwrap();
        let w = s.take_due(Duration::ZERO, allowed()).unwrap().unwrap();
        s.complete(w, Duration::MAX, false).unwrap();
        assert_eq!(s.next_deadline(allowed()), None);
        assert!(s.resume(1, Duration::ZERO));
        s.serial = u64::MAX;
        assert_eq!(s.take_due(Duration::ZERO, allowed()), Err(Error::Exhausted));
        assert_eq!(s.next_deadline(allowed()), Some(Duration::ZERO));
    }

    #[test]
    fn cancelled_work_retains_slot_and_stale_completion_is_rejected() {
        let mut s = scheduler();
        s.schedule(1, 1, Duration::ZERO).unwrap();
        s.schedule(2, 0, Duration::ZERO).unwrap();
        let old = s.take_due(Duration::ZERO, allowed()).unwrap().unwrap();
        assert_eq!(s.cancel(1), Some(old));
        assert_eq!(s.take_due(Duration::ZERO, allowed()), Ok(None));
        assert_eq!(s.schedule(1, 2, Duration::ZERO), Err(Error::Duplicate));
        s.complete(old, Duration::ZERO, true).unwrap();
        s.schedule(1, 2, Duration::ZERO).unwrap();
        let new = s.take_due(Duration::ZERO, allowed()).unwrap().unwrap();
        assert_eq!(new.generation(), 2);
        assert_eq!(s.complete(old, Duration::ZERO, true), Err(Error::Stale));
        assert_eq!(s.take_due(Duration::ZERO, allowed()), Ok(None));
        s.complete(new, Duration::ZERO, true).unwrap();
        assert_eq!(s.complete(new, Duration::ZERO, true), Err(Error::Stale));
    }

    #[test]
    fn bounded_queue_and_pending_cancellation() {
        let mut s = scheduler();
        s.schedule(1, 0, Duration::ZERO).unwrap();
        s.schedule(2, 0, Duration::ZERO).unwrap();
        assert_eq!(s.schedule(3, 0, Duration::ZERO), Err(Error::Full));
        assert_eq!(s.schedule(1, 0, Duration::ZERO), Err(Error::Duplicate));
        assert_eq!(s.cancel(1), None);
        s.schedule(3, 0, Duration::ZERO).unwrap();
        assert!(Scheduler::new(NonZeroU64::MIN, 0, 1).is_err());
        assert!(Scheduler::new(NonZeroU64::MIN, 1, 2).is_err());
    }

    #[test]
    fn policy_events_resume_without_ui_polling() {
        let mut s = scheduler();
        s.schedule(1, 0, Duration::ZERO).unwrap();
        for policy in [
            Policy::default(),
            Policy {
                metered: true,
                ..allowed()
            },
            Policy {
                battery_saver: true,
                ..allowed()
            },
            Policy {
                ui_visible: false,
                ..allowed()
            },
        ] {
            assert_eq!(s.next_deadline(policy), None);
            assert_eq!(s.take_due(Duration::MAX, policy), Ok(None));
        }
        let background = Policy {
            ui_visible: false,
            background_window: true,
            ..allowed()
        };
        assert_eq!(s.next_deadline(background), Some(Duration::ZERO));
        assert!(s.take_due(Duration::ZERO, background).unwrap().is_some());
        assert_eq!(s.next_deadline(background), None);
        assert_eq!(s.entries.len(), 1);
    }
}
