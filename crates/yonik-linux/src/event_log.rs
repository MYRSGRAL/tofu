//! Ограниченный журнал без строк из конфигурации или native stdout/stderr.
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicU64, Ordering},
    },
};

pub const MAX_EVENTS: usize = 2_000;
pub const MAX_PAGE: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventSource {
    App,
    Engine,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventCode {
    StartRequested,
    ProcessSpawned,
    StopRequested,
    CleanupConfirmed,
    ProcessExited,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogEvent {
    pub id: u64,
    pub source: EventSource,
    pub code: EventCode,
    pub value: Option<i32>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct LogFilter {
    pub source: Option<EventSource>,
    pub code: Option<EventCode>,
}
#[derive(Debug)]
pub struct LogPage {
    pub events: Vec<LogEvent>,
    /// Последний просмотренный ID, включая отфильтрованные записи.
    pub cursor: u64,
    pub evicted: u64,
    pub rejected: u64,
    pub missed: u64,
}
struct Ring {
    events: VecDeque<LogEvent>,
    next: u64,
    evicted: u64,
}
struct Shared {
    ring: Mutex<Ring>,
    rejected: AtomicU64,
}
#[derive(Clone)]
pub struct EventLog(Arc<Shared>);
impl Default for EventLog {
    fn default() -> Self {
        Self::new()
    }
}
impl EventLog {
    pub fn new() -> Self {
        Self(Arc::new(Shared {
            ring: Mutex::new(Ring {
                events: VecDeque::with_capacity(MAX_EVENTS),
                next: 1,
                evicted: 0,
            }),
            rejected: AtomicU64::new(0),
        }))
    }
    /// Producer никогда не ждёт consumer. Переполнение счётчиков насыщается.
    pub(crate) fn record(&self, source: EventSource, code: EventCode, value: Option<i32>) {
        let Ok(mut ring) = self.0.ring.try_lock() else {
            self.reject();
            return;
        };
        if ring.next == u64::MAX {
            self.reject();
            return;
        }
        let event = LogEvent {
            id: ring.next,
            source,
            code,
            value,
        };
        ring.next += 1;
        if ring.events.len() == MAX_EVENTS {
            ring.events.pop_front();
            ring.evicted = ring.evicted.saturating_add(1);
        }
        ring.events.push_back(event);
    }
    pub(crate) fn reject(&self) {
        let _ = self
            .0
            .rejected
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_add(1))
            });
    }
    /// Неизвестные native bytes не классифицируются и не сохраняются.
    pub fn reject_native(&self, _bytes: &[u8]) {
        self.reject();
    }
    /// Не более 2000 просмотренных записей и 256 результатов на запрос.
    /// Busy возвращает None; caller повторяет по явному запросу, без polling.
    pub fn read(&self, after: u64, filter: LogFilter, limit: usize) -> Option<LogPage> {
        let ring = match self.0.ring.try_lock() {
            Ok(ring) => ring,
            Err(TryLockError::WouldBlock | TryLockError::Poisoned(_)) => return None,
        };
        let oldest = ring.events.front().map_or(ring.next, |e| e.id);
        let mut page = LogPage {
            events: Vec::with_capacity(limit.min(MAX_PAGE)),
            cursor: after.min(ring.next - 1),
            evicted: ring.evicted,
            rejected: self.0.rejected.load(Ordering::Relaxed),
            missed: oldest.saturating_sub(after.saturating_add(1)),
        };
        if limit == 0 {
            return Some(page);
        }
        for event in ring.events.iter().filter(|e| e.id > after) {
            page.cursor = event.id;
            if filter.source.is_none_or(|s| s == event.source)
                && filter.code.is_none_or(|c| c == event.code)
            {
                page.events.push(*event);
                if page.events.len() == limit.min(MAX_PAGE) {
                    break;
                }
            }
        }
        Some(page)
    }
}
impl LogPage {
    /// Экспорт одной ограниченной страницы. Только статические коды и числа.
    pub fn export(&self) -> String {
        let mut text = format!(
            "yonik-events-v1 cursor={} evicted={} rejected={} missed={}\n",
            self.cursor, self.evicted, self.rejected, self.missed
        );
        for event in self.events.iter().take(MAX_PAGE) {
            use std::fmt::Write;
            let _ = writeln!(
                text,
                "{} {:?} {:?} {:?}",
                event.id, event.source, event.code, event.value
            );
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_cursor_filter_and_secret_canary() {
        let log = EventLog::new();
        log.reject_native(b"https://user:SECRET@host/\x1b[31m\n");
        for _ in 0..MAX_EVENTS + 5 {
            log.record(EventSource::App, EventCode::StartRequested, None);
        }
        let page = log.read(0, LogFilter::default(), usize::MAX).expect("page");
        assert_eq!(page.events.len(), MAX_PAGE);
        assert_eq!((page.evicted, page.missed, page.rejected), (5, 5, 1));
        assert!(!page.export().contains("SECRET"));
        assert!(!page.export().contains('\x1b'));
        let empty = log
            .read(
                0,
                LogFilter {
                    source: Some(EventSource::Engine),
                    code: None,
                },
                1,
            )
            .expect("page");
        assert!(empty.events.is_empty());
        assert_eq!(empty.cursor, (MAX_EVENTS + 5) as u64);
    }
    #[test]
    fn busy_producer_drops_without_waiting() {
        let log = EventLog::new();
        let guard = log.0.ring.lock().expect("lock");
        log.record(EventSource::App, EventCode::StopRequested, None);
        drop(guard);
        assert_eq!(
            log.read(0, LogFilter::default(), 1).expect("page").rejected,
            1
        );
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn exhausted_ids_are_observable_without_wrap() {
        let log = EventLog::new();
        log.0.ring.lock().expect("lock").next = u64::MAX;
        log.record(EventSource::App, EventCode::StartRequested, None);
        let page = log.read(u64::MAX, LogFilter::default(), 1).expect("page");
        assert!(page.events.is_empty());
        assert_eq!(page.rejected, 1);
    }
}
