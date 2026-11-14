//! Закрытый lifecycle producer. Подключение compile/issuer требует отдельного leaf.
use super::{ServiceError, SessionArtifact};
use crate::helper::{run_authorization::RunAuthority, systemd_worker::AuthorizedRun};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct NativeArtifactStamp {
    session: [u8; 16],
    generation: u64,
}

// Поля удерживают exact immutable artifact/queue и целый authorized run.
// Пока runtime caller запрещён, эти типы создаются только закрытым producer.
#[allow(dead_code)]
struct NativeOwnerLease<A, Q, R> {
    artifact: A,
    queue: Q,
    network_stamp: crate::NetworkStamp,
    network_epoch: u64,
    authority: RunAuthority,
    run: Option<R>,
}

pub(in super::super) struct NativeOwnerState<
    A = Arc<SessionArtifact>,
    Q = Arc<crate::managed_network::tun::BoundTun>,
    R = AuthorizedRun,
> {
    session: Option<[u8; 16]>,
    last_generation: u64,
    consumed: BTreeSet<u64>,
    lease: Option<NativeOwnerLease<A, Q, R>>,
}
impl<A, Q, R> NativeOwnerState<A, Q, R> {
    pub(in super::super) fn fresh() -> Self {
        let mut session = [0; 16];
        let valid = getrandom::getrandom(&mut session).is_ok() && session != [0; 16];
        Self::from_session(valid.then_some(session))
    }
    fn from_session(session: Option<[u8; 16]>) -> Self {
        Self {
            session: session.filter(|id| *id != [0; 16]),
            last_generation: 0,
            consumed: BTreeSet::new(),
            lease: None,
        }
    }
    // Нет production caller: legacy compile намеренно оставляет stamp=None.
    #[allow(dead_code)]
    pub(super) fn reserve(&mut self) -> Result<NativeArtifactStamp, ServiceError> {
        let session = self.session.ok_or(ServiceError::Unavailable)?;
        let Some(generation) = self.last_generation.checked_add(1) else {
            self.close();
            return Err(ServiceError::Unavailable);
        };
        self.last_generation = generation;
        Ok(NativeArtifactStamp {
            session,
            generation,
        })
    }
    #[allow(dead_code)] // Используется только будущим verified commit, не caller flags.
    fn consume(&mut self, stamp: &NativeArtifactStamp) -> Result<(), ServiceError> {
        if self.session != Some(stamp.session)
            || stamp.generation == 0
            || stamp.generation > self.last_generation
        {
            return Err(ServiceError::Unavailable);
        }
        if self.busy() {
            return Err(ServiceError::Busy);
        }
        if !self.consumed.insert(stamp.generation) {
            return Err(ServiceError::Unavailable);
        }
        Ok(())
    }
    pub(in super::super) fn busy(&self) -> bool {
        self.lease.is_some()
    }
    pub(in super::super) fn revoke(&mut self) {
        if let Some(lease) = &self.lease {
            lease.authority.revoke();
        }
        // Неизвестный stop сохраняет queue/artifact/run; Busy до confirmed recover.
    }
    pub(in super::super) fn close(&mut self) {
        self.revoke();
        self.session = None;
    }
    pub(in super::super) fn release_confirmed(&mut self) {
        self.revoke();
        self.lease = None;
    }
}
impl<A, Q, R> Drop for NativeOwnerState<A, Q, R> {
    fn drop(&mut self) {
        self.revoke();
    }
}

// Нет constructor из caller данных: record создаётся только после async проверок.
#[allow(dead_code)]
struct VerifiedNativeNetworkCommit {
    artifact: Arc<SessionArtifact>,
    queue: Arc<crate::managed_network::tun::BoundTun>,
    stamp: crate::NetworkStamp,
    epoch: u64,
    owner: crate::managed_network::Ownership,
}

// Закрытая граница: production caller и compile stamping ещё не подключаются.
#[allow(dead_code)]
impl super::Backend {
    async fn verify_native_network(
        &mut self,
        artifact: Arc<SessionArtifact>,
        index: u32,
        link: &crate::LinuxLink,
        cancel: &mut crate::Cancellation,
    ) -> Result<VerifiedNativeNetworkCommit, ServiceError> {
        let before = self.snapshot()?;
        let epoch = self.epoch;
        let queue = Arc::clone(
            &self
                .tun_custody
                .as_ref()
                .ok_or(ServiceError::Unavailable)?
                .bound,
        );
        crate::attribution::preflight(
            artifact.attribution_required,
            artifact.peer.as_deref(),
            artifact.worker_uid,
        )
        .await
        .map_err(|_| ServiceError::Unsupported)?;
        self.verify_network_facts(&artifact, index, link, cancel)
            .await?;
        let (current, _) = crate::network::read_topology_captured(cancel, super::DEADLINE)
            .await
            .map_err(super::unavailable)?;
        let metadata = queue.attach_metadata().map_err(super::unavailable)?;
        if metadata[4..8] != index.to_be_bytes() {
            return Err(super::stale());
        }
        let journal = self.journal.as_ref().ok_or(ServiceError::Unavailable)?;
        let owner = journal.ownership();
        queue
            .project(current, journal)
            .map_err(super::unavailable)?;
        let snapshot = self.snapshot()?;
        if !snapshot.fully_observed() || !snapshot.is_current(before.stamp) || epoch != self.epoch {
            return Err(ServiceError::Unavailable);
        }
        let record = VerifiedNativeNetworkCommit {
            artifact,
            queue,
            stamp: snapshot.stamp,
            epoch: self.epoch,
            owner,
        };
        self.revalidate_native(&record)?;
        Ok(record)
    }
    fn revalidate_native(&self, record: &VerifiedNativeNetworkCommit) -> Result<(), ServiceError> {
        if self.worker_uncertain || self.active.is_some() || self.native_owner.busy() {
            return Err(ServiceError::Busy);
        }
        if record.epoch != self.epoch
            || !self.snapshot()?.is_current(record.stamp)
            || !self.snapshot()?.fully_observed()
            || self.journal.as_ref().map(|j| j.ownership()) != Some(record.owner)
            || !self
                .tun_custody
                .as_ref()
                .is_some_and(|c| Arc::ptr_eq(&c.bound, &record.queue))
            || self.context(record.artifact.uid)? != record.artifact.context
        {
            return Err(super::stale());
        }
        record.queue.attach_metadata().map_err(super::unavailable)?;
        // UID берётся только из retained peer после identity recheck.
        let peer = record
            .artifact
            .peer
            .as_ref()
            .ok_or(ServiceError::Unsupported)?;
        if peer.verified_uid().map_err(|_| ServiceError::Unsupported)? != record.artifact.uid {
            return Err(ServiceError::Unsupported);
        }
        Ok(())
    }
    fn commit_native_owner(
        &mut self,
        record: VerifiedNativeNetworkCommit,
    ) -> Result<(), ServiceError> {
        self.revalidate_native(&record)?;
        let stamp = record
            .artifact
            .native_owner_stamp
            .as_ref()
            .ok_or(ServiceError::Unavailable)?;
        self.native_owner.consume(stamp)?;
        let authority = RunAuthority::new(stamp.session, stamp.generation, record.epoch)
            .map_err(|_| ServiceError::Unavailable)?;
        self.native_owner.lease = Some(NativeOwnerLease {
            artifact: record.artifact,
            queue: record.queue,
            network_stamp: record.stamp,
            network_epoch: record.epoch,
            authority,
            run: None,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestOwner = NativeOwnerState<Arc<()>, Arc<()>, Arc<()>>;
    #[test]
    fn failed_compile_burns_generation_and_prepared_order_is_not_authority() {
        let mut owner = TestOwner::from_session(Some([1; 16]));
        let first = owner.reserve().unwrap();
        let failed = owner.reserve().unwrap();
        let third = owner.reserve().unwrap();
        assert_eq!(
            (first.generation, failed.generation, third.generation),
            (1, 2, 3)
        );
        owner.consume(&third).unwrap();
        owner.consume(&first).unwrap();
        assert!(owner.consume(&first).is_err());
    }
    #[test]
    fn foreign_session_and_unreserved_generation_cannot_consume() {
        let mut owner = TestOwner::from_session(Some([1; 16]));
        let first = owner.reserve().unwrap();
        for stamp in [
            NativeArtifactStamp {
                session: [2; 16],
                generation: 1,
            },
            NativeArtifactStamp {
                session: [1; 16],
                generation: 0,
            },
            NativeArtifactStamp {
                session: [1; 16],
                generation: 2,
            },
        ] {
            assert!(owner.consume(&stamp).is_err());
        }
        owner.consume(&first).unwrap();
    }
    #[test]
    fn zero_entropy_and_overflow_permanently_close_admission() {
        for session in [None, Some([0; 16])] {
            assert!(TestOwner::from_session(session).reserve().is_err());
        }
        let mut owner = TestOwner::from_session(Some([1; 16]));
        owner.last_generation = u64::MAX - 1;
        let last = owner.reserve().unwrap();
        assert_eq!(last.generation, u64::MAX);
        assert!(owner.reserve().is_err());
        assert!(owner.consume(&last).is_err());
        assert!(owner.reserve().is_err());
    }
    #[tokio::test]
    async fn unknown_stop_revokes_but_retains_all_ownership_until_confirmation() {
        let mut owner = TestOwner::from_session(Some([1; 16]));
        let artifact = Arc::new(());
        let queue = Arc::new(());
        let run = Arc::new(());
        let authority = RunAuthority::new([1; 16], 1, 1).unwrap();
        let token = authority.authorize([2; 32]).unwrap();
        // NetworkStamp не proof; fixture берёт snapshot штатного monitor.
        let monitor = crate::NetworkMonitor::start().unwrap();
        owner.lease = Some(NativeOwnerLease {
            artifact: Arc::clone(&artifact),
            queue: Arc::clone(&queue),
            run: Some(Arc::clone(&run)),
            authority,
            network_stamp: monitor.watch().current().stamp,
            network_epoch: 1,
        });
        assert!(token.challenge_context().is_ok());
        owner.revoke();
        assert!(token.challenge_context().is_err());
        assert!(owner.busy());
        assert_eq!(
            (
                Arc::strong_count(&artifact),
                Arc::strong_count(&queue),
                Arc::strong_count(&run)
            ),
            (2, 2, 2)
        );
        owner.release_confirmed();
        assert!(!owner.busy());
        assert_eq!(
            (
                Arc::strong_count(&artifact),
                Arc::strong_count(&queue),
                Arc::strong_count(&run)
            ),
            (1, 1, 1)
        );
        let authority = RunAuthority::new([1; 16], 2, 1).unwrap();
        let pending_token = authority.authorize([3; 32]).unwrap();
        owner.lease = Some(NativeOwnerLease {
            artifact: Arc::clone(&artifact),
            queue: Arc::clone(&queue),
            run: None,
            authority,
            network_stamp: monitor.watch().current().stamp,
            network_epoch: 1,
        });
        drop(owner);
        assert!(pending_token.challenge_context().is_err());
        assert_eq!(
            (Arc::strong_count(&artifact), Arc::strong_count(&queue)),
            (1, 1)
        );
    }
}
