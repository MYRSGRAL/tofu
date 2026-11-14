//! Lifetime ровно actual unique D-Bus owner. Не permission/native permit.
use super::{MAX_IN_FLIGHT, PreparedError};
use crate::helper::MAX_PENDING;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) struct PeerPresence {
    sender: String,
    uid: u32,
    revoked: AtomicBool,
    capture: Arc<Mutex<Weak<crate::attribution::CapturedBusScope>>>,
}
pub(crate) struct BusPeerIdentity {
    presence: Arc<PeerPresence>,
    pid: u32,
}
impl BusPeerIdentity {
    pub(crate) fn uid(&self) -> u32 {
        self.presence.uid
    }
    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }
    pub(crate) fn owner(&self) -> &str {
        &self.presence.sender
    }
    pub(crate) fn validate(&self) -> bool {
        !self.presence.revoked.load(Ordering::Acquire)
    }
    pub(crate) fn capture_slot(&self) -> Arc<Mutex<Weak<crate::attribution::CapturedBusScope>>> {
        Arc::clone(&self.presence.capture)
    }
    pub(crate) fn same_owner(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.presence, &other.presence)
    }
}
#[derive(Default)]
pub(super) struct PeerTracker {
    entries: Mutex<HashMap<String, Weak<PeerPresence>>>,
}
impl PeerTracker {
    pub(super) fn track(&self, sender: &str, uid: u32) -> Result<Arc<PeerPresence>, PreparedError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| PreparedError::Unsupported)?;
        entries.retain(|_, v| v.strong_count() > 0);
        if let Some(existing) = entries.get(sender).and_then(Weak::upgrade) {
            if existing.uid != uid || existing.revoked.load(Ordering::Acquire) {
                return Err(PreparedError::Unsupported);
            }
            return Ok(existing);
        }
        if !sender.starts_with(':')
            || sender.len() > 255
            || entries.len() >= MAX_PENDING + MAX_IN_FLIGHT
        {
            return Err(PreparedError::Capacity);
        }
        let value = Arc::new(PeerPresence {
            sender: sender.into(),
            uid,
            revoked: AtomicBool::new(false),
            capture: Arc::new(Mutex::new(Weak::new())),
        });
        entries.insert(sender.into(), Arc::downgrade(&value));
        Ok(value)
    }
    pub(super) fn revoke(&self, sender: &str) -> Result<(), PreparedError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| PreparedError::Unsupported)?;
        if let Some(presence) = entries.remove(sender).and_then(|v| v.upgrade()) {
            presence.revoked.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub(super) fn revoke_all(&self) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for presence in entries.values().filter_map(Weak::upgrade) {
            presence.revoked.store(true, Ordering::Release);
        }
        entries.clear();
    }
}
impl PeerPresence {
    /// Только service после fresh authoritative GetConnectionCredentials(pid,uid).
    pub(super) fn bind(self: Arc<Self>, pid: u32) -> Result<BusPeerIdentity, PreparedError> {
        if pid == 0 || self.revoked.load(Ordering::Acquire) {
            return Err(PreparedError::Unsupported);
        }
        Ok(BusPeerIdentity {
            presence: self,
            pid,
        })
    }
}
impl Drop for PeerTracker {
    fn drop(&mut self) {
        self.revoke_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pure_bus_owner_revoke_denies_live_shared_identity_immediately() {
        let tracker = PeerTracker::default();
        let first = tracker.track(":1.2", 1000).unwrap().bind(42).unwrap();
        let second = tracker.track(":1.2", 1000).unwrap().bind(42).unwrap();
        assert!(first.same_owner(&second));
        tracker.revoke(":1.2").unwrap();
        assert!(!first.validate());
        assert!(!second.validate());
    }
    #[test]
    fn pure_bus_owner_uid_change_and_disconnect_before_pid_reply_are_rejected() {
        let tracker = PeerTracker::default();
        let pending = tracker.track(":1.2", 1000).unwrap();
        assert!(tracker.track(":1.2", 1001).is_err());
        tracker.revoke(":1.2").unwrap();
        assert!(pending.bind(42).is_err());
    }
}
