//! Root Session → canonical app-UID leaf. Нет local root lookup fallback.
use super::lookup_channel::{Channel, credentials, fence};
use super::lookup_codec::{self as wire, FRAME, HELLO, HELLO_ACK, REPLY};
use super::{
    DEADLINE, Lookup, LookupBinding, OwnerSnapshot, ProjectedLookup, Unavailable,
    ValidatedProjection,
};
use crate::helper::native_bootstrap::{LookupLeafLease, RuntimeScopeStamp, VerifiedLookupLeaf};
use std::{
    fs::{self, File},
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::fs::MetadataExt,
    },
    time::{Duration, Instant},
};

pub(crate) struct RemoteLookup {
    channel: Channel,
    lease: LookupLeafLease,
    scope: RuntimeScopeStamp,
    binding: LookupBinding,
    next: u64,
    revoked: bool,
    _view: [OwnedFd; 3],
    _image: File,
}
impl RemoteLookup {
    /// Только owning PID1 launch token, не caller socket/PID/UID/FD constructor.
    pub(crate) fn from_verified(
        leaf: VerifiedLookupLeaf,
        scope: RuntimeScopeStamp,
    ) -> Result<Self, Unavailable> {
        scope
            .validate_admission()
            .map_err(|_| Unavailable::Authority)?;
        let deadline = scope.admission_deadline();
        let binding = scope.lookup_binding_data();
        let (socket, lease) = leaf.into_parts();
        lease.validate().map_err(|_| Unavailable::Authority)?;
        if lease.uid() != binding.target_uid || lease.invocation() == [0; 16] {
            return Err(Unavailable::Authority);
        }
        let channel = Channel::new(socket)?;
        channel.send(&binding.encode(HELLO, 0)?, &[], deadline)?;
        let packet = channel.receive(FRAME, Some(deadline))?;
        credentials(packet.credentials, lease.pid(), lease.uid())?;
        binding.validate(&packet.bytes, HELLO_ACK, 0, FRAME)?;
        if packet.rights.len() != 4 {
            return Err(Unavailable::Malformed);
        }
        let mut rights = packet.rights.into_iter();
        let mut view = Vec::with_capacity(3);
        let expected_view = scope
            .application_view()
            .map_err(|_| Unavailable::Authority)?;
        for (expected, identity) in [
            yonik_nsfs::NamespaceType::Net,
            yonik_nsfs::NamespaceType::Pid,
            yonik_nsfs::NamespaceType::Mount,
        ]
        .into_iter()
        .zip(expected_view)
        {
            let fd = rights.next().ok_or(Unavailable::Malformed)?;
            if yonik_nsfs::namespace_type(&fd).map_err(|_| Unavailable::Coverage)? != expected {
                return Err(Unavailable::Coverage);
            }
            let m = rustix::fs::fstat(&fd).map_err(|_| Unavailable::Coverage)?;
            if (m.st_dev, m.st_ino) != identity
                || expected == yonik_nsfs::NamespaceType::Net && identity != binding.network
            {
                return Err(Unavailable::Coverage);
            }
            view.push(fd);
        }
        let image = File::from(rights.next().ok_or(Unavailable::Malformed)?);
        lease
            .verify_image(&image)
            .map_err(|_| Unavailable::Authority)?;
        lease.validate().map_err(|_| Unavailable::Authority)?;
        scope
            .validate_admission()
            .map_err(|_| Unavailable::Authority)?;
        scope.validate().map_err(|_| Unavailable::Revoked)?;
        fence(deadline)?;
        Ok(Self {
            channel,
            lease,
            scope,
            binding,
            next: 0,
            revoked: false,
            _view: view.try_into().map_err(|_| Unavailable::Malformed)?,
            _image: image,
        })
    }
    pub(crate) fn validate_scope(&self, scope: &RuntimeScopeStamp) -> Result<(), Unavailable> {
        if !self.scope.same_scope(scope) {
            return Err(Unavailable::Authority);
        }
        self.validate()
    }
    fn validate(&self) -> Result<(), Unavailable> {
        if self.revoked {
            return Err(Unavailable::Revoked);
        }
        self.scope.validate().map_err(|_| Unavailable::Revoked)?;
        self.lease.validate().map_err(|_| Unavailable::Authority)
    }
    pub(crate) fn revoke(&mut self) {
        if !self.revoked {
            self.revoked = true;
            // Shutdown затрагивает все duplicates; child получает EOF. Unit custody
            // остаётся в supervisor до отдельного canonical confirmed-empty.
            self.channel.shutdown();
        }
    }
    pub(in crate::attribution) fn lookup(&mut self, flow: super::Flow) -> Lookup {
        match self.lookup_and_project(flow, self.binding.epoch) {
            ProjectedLookup::Known(projection) => Lookup::KnownOwner(projection.into_parts().0),
            ProjectedLookup::Unknown => Lookup::UnknownFlow,
            ProjectedLookup::Unavailable(reason) => Lookup::MechanismUnavailable(reason),
        }
    }
    pub(in crate::attribution) fn lookup_and_project(
        &mut self,
        flow: super::Flow,
        epoch: u64,
    ) -> ProjectedLookup {
        let result = self.exchange(flow, epoch);
        match result {
            Ok(value) => value,
            Err(reason) => {
                self.revoke();
                ProjectedLookup::Unavailable(reason)
            }
        }
    }
    fn exchange(&mut self, flow: super::Flow, epoch: u64) -> Result<ProjectedLookup, Unavailable> {
        let began = Instant::now();
        let deadline = began + DEADLINE;
        self.validate()?;
        fence(deadline)?;
        if epoch != self.binding.epoch {
            return Err(Unavailable::Authority);
        }
        self.next = self.next.checked_add(1).ok_or(Unavailable::Limit)?;
        self.channel
            .send(&self.binding.request(self.next, flow)?, &[], deadline)?;
        let packet = self.channel.receive(REPLY, Some(deadline))?;
        credentials(packet.credentials, self.lease.pid(), self.lease.uid())?;
        self.validate()?;
        let value = match wire::decode_result(
            &self.binding,
            &packet.bytes,
            self.next,
            packet.rights.len(),
        )? {
            wire::ResultData::Unknown => {
                fence(deadline)?;
                return Ok(ProjectedLookup::Unknown);
            }
            wire::ResultData::Unavailable(reason) => {
                fence(deadline)?;
                return Ok(ProjectedLookup::Unavailable(reason));
            }
            wire::ResultData::Known(value) => value,
        };
        let wire::KnownData {
            pid,
            task_id,
            start,
            task_start,
            socket_inode,
            socket_fd,
            cookie,
            executable,
            nanos,
            path,
        } = value;
        let mut rights = packet.rights.into_iter();
        let pidfd = rights.next().ok_or(Unavailable::Malformed)?;
        let executable_fd = File::from(rights.next().ok_or(Unavailable::Malformed)?);
        validate_owner_fds(
            pid,
            start,
            self.binding.target_uid,
            &pidfd,
            &executable_fd,
            executable,
            &path,
        )?;
        // Root не делает cross-UID ptrace/proc-exe fallback. Live app/exe/socket
        // kernel revalidation выполнена immutable app-UID leaf непосредственно
        // перед reply. Здесь проверяются retained FDs, identity и исходный бюджет.
        self.validate()?;
        fence(deadline)?;
        // TTL не перезапускается при receive: весь roundtrip уже израсходован.
        // Root-start + leaf-remaining <= leaf-send + leaf-remaining, включая cache.
        let expiry = conservative_expiry(began, nanos)?;
        fence(expiry)?;
        let ttl = expiry
            .checked_duration_since(Instant::now())
            .filter(|v| !v.is_zero())
            .ok_or(Unavailable::Timeout)?;
        let observed_from = expiry.checked_sub(DEADLINE).ok_or(Unavailable::Timeout)?;
        let owner = OwnerSnapshot {
            socket_inode,
            task_id,
            task_start,
            socket_fd,
            pidfd,
            executable_fd,
            observed_from,
            observed_until: Instant::now(),
            pid,
            start_time: start,
            executable,
            cookie,
            epoch,
        };
        Ok(ProjectedLookup::Known(ValidatedProjection {
            owner,
            path,
            ttl,
        }))
    }
}
fn validate_owner_fds(
    pid: u32,
    start: u64,
    uid: u32,
    pidfd: &OwnedFd,
    exe: &File,
    identity: (u64, u64),
    path: &str,
) -> Result<(), Unavailable> {
    let info = super::bounded(&format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd()))?;
    if info.lines().find_map(|line| {
        line.strip_prefix("Pid:")
            .and_then(|v| v.trim().parse::<u32>().ok())
    }) != Some(pid)
        || super::start(pid)? != start
        || fs::metadata(format!("/proc/{pid}"))
            .map_err(|_| Unavailable::Coverage)?
            .uid()
            != uid
    {
        return Err(Unavailable::Authority);
    }
    let metadata = exe.metadata().map_err(|_| Unavailable::Coverage)?;
    if !metadata.is_file() || (metadata.dev(), metadata.ino()) != identity {
        return Err(Unavailable::Authority);
    }
    let actual = fs::read_link(format!("/proc/self/fd/{}", exe.as_raw_fd()))
        .map_err(|_| Unavailable::Coverage)?;
    if actual.to_str() != Some(path) {
        return Err(Unavailable::Authority);
    }
    let named = fs::metadata(path).map_err(|_| Unavailable::Coverage)?;
    if (named.dev(), named.ino()) != identity {
        return Err(Unavailable::Authority);
    }
    if super::start(pid)? != start {
        return Err(Unavailable::Authority);
    }
    Ok(())
}
impl Drop for RemoteLookup {
    fn drop(&mut self) {
        self.revoke();
    }
}

fn conservative_expiry(began: Instant, nanos: u64) -> Result<Instant, Unavailable> {
    if nanos == 0 || nanos > DEADLINE.as_nanos() as u64 {
        return Err(Unavailable::Malformed);
    }
    began
        .checked_add(Duration::from_nanos(nanos))
        .ok_or(Unavailable::Timeout)
}
#[cfg(test)]
mod ttl_tests {
    use super::*;
    #[test]
    fn pure_remote_ttl_never_restarts_at_receive_or_extends_cached_snapshot() {
        let began = Instant::now();
        let transport = Duration::from_millis(3);
        let leaf_ttl = Duration::from_millis(1);
        let expiry = conservative_expiry(began, leaf_ttl.as_nanos() as u64).unwrap();
        assert!(expiry < began + transport);
        assert_eq!(
            conservative_expiry(began, DEADLINE.as_nanos() as u64).unwrap(),
            began + DEADLINE
        );
        assert!(conservative_expiry(began, 0).is_err());
        assert!(conservative_expiry(began, DEADLINE.as_nanos() as u64 + 1).is_err());
    }
}
