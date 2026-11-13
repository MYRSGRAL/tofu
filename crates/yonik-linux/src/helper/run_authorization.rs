//! Одноразовое разрешение владельца helper; не является namespace admission.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunAuthorizationError {
    InvalidContext,
    Revoked,
    Consumed,
}

// Producer подключается W4 после проверки реального owner/network context.
#[allow(dead_code)]
pub(crate) struct RunAuthority {
    session: [u8; 16],
    generation: u64,
    network_epoch: u64,
    issued: AtomicBool,
    revoked: Arc<AtomicBool>,
}
#[allow(dead_code)]
impl RunAuthority {
    pub(crate) fn new(
        session: [u8; 16],
        generation: u64,
        network_epoch: u64,
    ) -> Result<Self, RunAuthorizationError> {
        if session == [0; 16] || generation == 0 || network_epoch == 0 {
            return Err(RunAuthorizationError::InvalidContext);
        }
        Ok(Self {
            session,
            generation,
            network_epoch,
            issued: AtomicBool::new(false),
            revoked: Arc::new(AtomicBool::new(false)),
        })
    }
    pub(crate) fn authorize(
        &self,
        nonce: [u8; 32],
    ) -> Result<RunAuthorization, RunAuthorizationError> {
        if nonce == [0; 32] {
            return Err(RunAuthorizationError::InvalidContext);
        }
        if self.revoked.load(Ordering::Acquire) {
            return Err(RunAuthorizationError::Revoked);
        }
        self.issued
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| RunAuthorizationError::Consumed)?;
        let token = RunAuthorization {
            session: self.session,
            nonce,
            generation: self.generation,
            network_epoch: self.network_epoch,
            revoked: Arc::clone(&self.revoked),
        };
        token.validate()?;
        Ok(token)
    }
    pub(crate) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }
}
impl Drop for RunAuthority {
    fn drop(&mut self) {
        self.revoke();
    }
}

pub(crate) struct RunAuthorization {
    session: [u8; 16],
    nonce: [u8; 32],
    generation: u64,
    network_epoch: u64,
    revoked: Arc<AtomicBool>,
}
impl RunAuthorization {
    pub(super) fn validate(&self) -> Result<(), RunAuthorizationError> {
        if self.revoked.load(Ordering::Acquire) {
            return Err(RunAuthorizationError::Revoked);
        }
        if self.session == [0; 16]
            || self.nonce == [0; 32]
            || self.generation == 0
            || self.network_epoch == 0
        {
            return Err(RunAuthorizationError::InvalidContext);
        }
        Ok(())
    }
    pub(super) fn guard(&self) -> RunRevocationGuard {
        RunRevocationGuard(Arc::clone(&self.revoked))
    }
}

/// Drop future отзывает даже token, уже переданный blocking task.
pub(super) struct RunRevocationGuard(Arc<AtomicBool>);
impl Drop for RunRevocationGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
