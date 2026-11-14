//! Helper-only binding, живой owning token и Journal под общей operation lease.
use super::{
    Cancellation, DEADLINE, Journal, Ownership, TunError, TunExecutor, config, lease, name,
    read_topology, topology,
};
use crate::LinuxTopology;
use yonik_nsfs::tun::{Framing, Queue};

pub(crate) struct BoundTun {
    pub(super) queue: Queue,
    operation: Option<std::sync::Arc<super::OperationLease>>,
    owner: Ownership,
    pub(super) tun_id: u64,
    index: u32,
    pub(super) mtu: u16,
    pub(super) framing: Framing,
}
impl BoundTun {
    #[cfg(all(test, feature = "test-support"))]
    pub(crate) fn queue_fd_for_test(&self) -> std::os::fd::BorrowedFd<'_> {
        self.queue.as_fd()
    }

    #[cfg(all(test, feature = "test-support"))]
    pub(crate) fn duplicate_same_kernel_queue_for_test(&self) -> Result<Self, TunError> {
        Ok(Self {
            queue: self
                .queue
                .test_duplicate()
                .map_err(|_| TunError::Evidence)?,
            operation: self.operation.clone(),
            owner: self.owner,
            tun_id: self.tun_id,
            index: self.index,
            mtu: self.mtu,
            framing: self.framing,
        })
    }

    pub(crate) fn duplicate_for_attach(&self) -> Result<std::os::fd::OwnedFd, TunError> {
        rustix::io::fcntl_dupfd_cloexec(self.queue.as_fd(), 0).map_err(|_| TunError::Io)
    }
    pub(crate) fn attach_metadata(&self) -> Result<[u8; 32], TunError> {
        if self.framing != Framing::Plain || self.index == 0 || self.mtu < 68 {
            return Err(TunError::Rejected);
        }
        self.queue.verify().map_err(|_| TunError::Evidence)?;
        let mut metadata = [0; 32];
        metadata[..4].copy_from_slice(&1u32.to_be_bytes());
        metadata[4..8].copy_from_slice(&self.index.to_be_bytes());
        // IFF_PERSIST — kernel state, wire передаёт только semantic framing.
        metadata[8..12].copy_from_slice(&0x1001u32.to_be_bytes());
        metadata[12..16].copy_from_slice(&u32::from(self.mtu).to_be_bytes());
        let name = self.queue.name().as_bytes();
        metadata[16..16 + name.len()].copy_from_slice(name);
        Ok(metadata)
    }

    pub(crate) fn transferred(&mut self) {
        self.operation = None;
    }
    pub(super) fn check_binding(
        &self,
        journal: &Journal,
        snapshot: &LinuxTopology,
    ) -> Result<(), TunError> {
        if journal.ownership() != self.owner
            || (snapshot.namespace.device, snapshot.namespace.inode) != self.owner.namespace()
        {
            return Err(TunError::Foreign);
        }
        let record = journal
            .records()
            .iter()
            .find(|r| r.id() == self.tun_id)
            .ok_or(TunError::Evidence)?;
        let super::Resource::Tun { name_id } = record.resource() else {
            return Err(TunError::Evidence);
        };
        if super::observed(
            &snapshot.links,
            *name_id,
            &super::alias(self.owner, self.tun_id)?,
        )? != Some(self.index)
        {
            return Err(TunError::Foreign);
        }
        self.queue.verify().map_err(|_| TunError::Evidence)
    }
    pub(crate) fn project(
        &self,
        snapshot: LinuxTopology,
        journal: &Journal,
    ) -> Result<LinuxTopology, TunError> {
        topology::attached_topology(snapshot, journal, self)
    }
}
impl TunExecutor {
    pub(crate) async fn bind_session_queue(
        &self,
        journal: &Journal,
        tun_id: u64,
        mtu: u16,
        framing: Framing,
        cancel: &mut Cancellation,
    ) -> Result<BoundTun, TunError> {
        let operation = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(TunError::Rejected),
            result = lease() => result?,
        };
        cancel.check().map_err(|_| TunError::Rejected)?;
        self.check_namespace()?;
        config::ownership(self, journal)?;
        let before = read_topology(cancel, DEADLINE)
            .await
            .map_err(|_| TunError::Evidence)?;
        let index = config::parent_index(self, journal, tun_id, &before, true)?
            .ok_or(TunError::Evidence)?;
        let link = before
            .links
            .iter()
            .find(|l| l.index == index)
            .ok_or(TunError::Evidence)?;
        // Для single queue счётчик netlink отсутствует. Carrier-off — prefilter;
        // отсутствие занятой очереди атомарно обеспечивает TUNSETIFF (иначе EBUSY).
        if !link.unknown.iter().any(|a| a.kind == 18)
            || link.flags & 0x10000 != 0
            || !link.unknown.iter().any(|a| a.kind == 33 && a.value == [0])
        {
            return Err(TunError::Foreign);
        }
        let record = journal
            .records()
            .iter()
            .find(|r| r.id() == tun_id)
            .ok_or(TunError::Evidence)?;
        let super::Resource::Tun { name_id } = record.resource() else {
            return Err(TunError::Evidence);
        };
        topology::detached_parent(&before, journal, tun_id, mtu, framing == Framing::Vnet10)?;
        cancel.check().map_err(|_| TunError::Rejected)?;
        self.check_namespace()?;
        let bound = BoundTun {
            queue: Queue::open(&name(*name_id)?, framing).map_err(|_| TunError::Io)?,
            operation: Some(operation),
            owner: self.owner,
            tun_id,
            index,
            mtu,
            framing,
        };
        #[cfg(test)]
        tests::after_attach().await;
        // Attach сам порождает carrier/route events. Только один новый dump при
        // Changed, без повторного ioctl; общий deadline ограничивает оба чтения.
        let after = tokio::time::timeout(DEADLINE, async {
            match read_topology(cancel, DEADLINE).await {
                Err(crate::NetworkReadError::Changed) => read_topology(cancel, DEADLINE).await,
                result => result,
            }
        })
        .await
        .map_err(|_| TunError::Timeout)?
        .map_err(|_| TunError::Evidence)?;
        self.check_namespace()?;
        if config::parent_index(self, journal, tun_id, &after, true)? != Some(index) {
            return Err(TunError::Foreign);
        }
        bound.project(after, journal)?;
        cancel.check().map_err(|_| TunError::Rejected)?;
        // Никакого await между последней проверкой и transfer единственного owner.
        Ok(bound)
    }
}

#[cfg(test)]
#[path = "tun_custody_tests.rs"]
mod tests;
