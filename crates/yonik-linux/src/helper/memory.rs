//! Процессный учёт удерживаемых buffers и memfd, включая текущие reservations.

use std::sync::{
    Arc, Mutex, OnceLock, Weak,
    atomic::{AtomicUsize, Ordering},
};

pub const MAX_CONFIG_MEMORY_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_EXECUTABLE_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_RETAINED_CONFIG_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_CONFIG_STORES: usize = 16;

pub(crate) type Reclaimer = dyn Fn() + Send + Sync;

static CONFIG: OnceLock<ByteBudget> = OnceLock::new();
static EXECUTABLE: OnceLock<ByteBudget> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryUsage {
    pub reserved_config_bytes: usize,
    pub reserved_executable_bytes: usize,
}

/// Reservations включают ещё не заполненные buffers; это не измеритель RSS.
pub fn memory_usage() -> MemoryUsage {
    MemoryUsage {
        reserved_config_bytes: CONFIG.get().map_or(0, ByteBudget::used),
        reserved_executable_bytes: EXECUTABLE.get().map_or(0, ByteBudget::used),
    }
}

pub(crate) fn config_budget() -> &'static ByteBudget {
    CONFIG.get_or_init(|| ByteBudget::new(MAX_CONFIG_MEMORY_BYTES))
}

pub(crate) fn executable_budget() -> &'static ByteBudget {
    EXECUTABLE.get_or_init(|| ByteBudget::new(MAX_EXECUTABLE_BYTES))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryLimit;

struct Pool {
    limit: usize,
    used: AtomicUsize,
    reclaimers: Mutex<Vec<Weak<Reclaimer>>>,
}

#[derive(Clone)]
pub(crate) struct ByteBudget(Arc<Pool>);

impl std::fmt::Debug for ByteBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ByteBudget(<учёт ресурсов>)")
    }
}

impl ByteBudget {
    pub(crate) fn new(limit: usize) -> Self {
        Self(Arc::new(Pool {
            limit,
            used: AtomicUsize::new(0),
            reclaimers: Mutex::new(Vec::new()),
        }))
    }

    pub(crate) fn used(&self) -> usize {
        self.0.used.load(Ordering::Acquire)
    }

    fn add(&self, bytes: usize) -> Result<(), MemoryLimit> {
        self.0
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.0.limit)
            })
            .map(|_| ())
            .map_err(|_| MemoryLimit)
    }

    pub(crate) fn reserve(&self, bytes: usize) -> Result<ByteLease, MemoryLimit> {
        if self.add(bytes).is_err() {
            // Не держим lock списка во время удаления payload из stores.
            let callbacks: Vec<_> = {
                let mut entries = self.0.reclaimers.lock().map_err(|_| MemoryLimit)?;
                entries.retain(|entry| entry.strong_count() != 0);
                entries.iter().filter_map(Weak::upgrade).collect()
            };
            for reclaim in callbacks {
                reclaim();
            }
            self.add(bytes)?;
        }
        Ok(ByteLease {
            budget: self.clone(),
            bytes,
        })
    }

    pub(crate) fn register(&self, callback: &Arc<Reclaimer>) -> Result<(), MemoryLimit> {
        let mut entries = self.0.reclaimers.lock().map_err(|_| MemoryLimit)?;
        entries.retain(|entry| entry.strong_count() != 0);
        if entries.len() >= MAX_CONFIG_STORES {
            return Err(MemoryLimit);
        }
        entries.push(Arc::downgrade(callback));
        Ok(())
    }
}

/// Не Clone: единственный владелец освобождает учёт после освобождения данных.
pub(crate) struct ByteLease {
    budget: ByteBudget,
    bytes: usize,
}

impl std::fmt::Debug for ByteLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ByteLease(<резерв>)")
    }
}

impl ByteLease {
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn resize(&mut self, bytes: usize) -> Result<(), MemoryLimit> {
        if bytes > self.bytes {
            self.budget.add(bytes - self.bytes)?;
        } else {
            self.budget
                .0
                .used
                .fetch_sub(self.bytes - bytes, Ordering::AcqRel);
        }
        self.bytes = bytes;
        Ok(())
    }
}

impl Drop for ByteLease {
    fn drop(&mut self) {
        self.budget.0.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::ByteBudget;
    use std::sync::Barrier;

    #[test]
    fn reservation_resize_overflow_and_drop_preserve_exact_usage() {
        let budget = ByteBudget::new(16);
        let mut first = budget.reserve(10).unwrap();
        assert!(budget.reserve(7).is_err());
        assert_eq!(budget.used(), 10);
        assert!(first.resize(17).is_err());
        assert_eq!(first.bytes(), 10);
        first.resize(4).unwrap();
        let second = budget.reserve(12).unwrap();
        assert_eq!(budget.used(), 16);
        assert!(budget.reserve(usize::MAX).is_err());
        drop(first);
        assert_eq!(budget.used(), 12);
        drop(second);
        assert_eq!(budget.used(), 0);
        let zero = budget.reserve(0).unwrap();
        drop(zero);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn simultaneous_reservations_never_exceed_shared_limit() {
        let budget = ByteBudget::new(4);
        let barrier = Barrier::new(16);
        std::thread::scope(|scope| {
            for _ in 0..16 {
                let budget = &budget;
                let barrier = &barrier;
                scope.spawn(move || {
                    let lease = budget.reserve(1);
                    barrier.wait();
                    assert_eq!(budget.used(), 4);
                    barrier.wait();
                    drop(lease);
                });
            }
        });
        assert_eq!(budget.used(), 0);
    }
    #[test]
    fn reclaimer_registry_is_bounded_and_does_not_keep_owners_alive() {
        use super::{MAX_CONFIG_STORES, Reclaimer};
        use std::sync::Arc;
        let pool = ByteBudget::new(4);
        let mut owners: Vec<Arc<Reclaimer>> = Vec::new();
        for _ in 0..MAX_CONFIG_STORES {
            let owner: Arc<Reclaimer> = Arc::new(|| {});
            pool.register(&owner).unwrap();
            owners.push(owner);
        }
        let extra: Arc<Reclaimer> = Arc::new(|| {});
        assert!(pool.register(&extra).is_err());
        drop(owners.pop());
        pool.register(&extra).unwrap();
        drop(owners);
        assert_eq!(Arc::strong_count(&extra), 1);
    }
}
