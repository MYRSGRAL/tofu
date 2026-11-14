//! Canonical app-UID inspector. Никаких RuntimeScope/TUN/Session permits.
use super::inspect_codec::{self as wire, Binding};
use super::lookup_channel::{Channel, Packet, credentials, fence};
use super::lookup_codec::FRAME;
use super::{DEADLINE, Unavailable};
use crate::attribution::{AttributionFailure, CapturedBusScope};
use crate::helper::native_bootstrap::{PeerVerifierLease, VerifiedStaticInstalledImage};
use std::{
    fs::{self, File},
    os::{
        fd::{AsFd, OwnedFd},
        unix::fs::MetadataExt,
    },
    sync::{Arc, Mutex},
    time::Instant,
};

pub(crate) struct VerifiedPeerSnapshot {
    files: [OwnedFd; 4],
    view: [(u64, u64); 3],
    executable: (u64, u64),
    identity: SnapshotIdentity,
}
#[derive(PartialEq, Eq)]
struct SnapshotIdentity {
    pid: u32,
    uid: u32,
    start: u64,
    observer: u32,
    invocation: [u8; 16],
    is_peer: bool,
}
impl VerifiedPeerSnapshot {
    fn from_packet(packet: Packet, binding: &Binding, is_peer: bool) -> Result<Self, Unavailable> {
        let files: [OwnedFd; 4] = packet
            .rights
            .try_into()
            .map_err(|_| Unavailable::Malformed)?;
        let mut view = [(0, 0); 3];
        for ((fd, expected), slot) in files[..3]
            .iter()
            .zip([
                yonik_nsfs::NamespaceType::Net,
                yonik_nsfs::NamespaceType::Pid,
                yonik_nsfs::NamespaceType::Mount,
            ])
            .zip(&mut view)
        {
            if yonik_nsfs::namespace_type(fd).map_err(|_| Unavailable::Coverage)? != expected {
                return Err(Unavailable::Coverage);
            }
            let st = rustix::fs::fstat(fd).map_err(|_| Unavailable::Coverage)?;
            *slot = (st.st_dev, st.st_ino);
        }
        let st = rustix::fs::fstat(&files[3]).map_err(|_| Unavailable::Coverage)?;
        if rustix::fs::FileType::from_raw_mode(st.st_mode) != rustix::fs::FileType::RegularFile {
            return Err(Unavailable::Coverage);
        }
        Ok(Self {
            files,
            view,
            executable: (st.st_dev, st.st_ino),
            identity: SnapshotIdentity {
                pid: if is_peer {
                    binding.caller
                } else {
                    binding.inspector
                },
                uid: binding.uid,
                start: if is_peer {
                    binding.caller_start
                } else {
                    binding.inspector_start
                },
                observer: binding.inspector,
                invocation: binding.invocation,
                is_peer,
            },
        })
    }
    pub(crate) fn namespace_view(&self) -> [(u64, u64); 3] {
        self.view
    }
    fn same(&self, other: &Self) -> bool {
        self.view == other.view
            && self.executable == other.executable
            && self.identity == other.identity
    }
    pub(crate) fn require_same_identity(&self, other: &Self) -> Result<(), AttributionFailure> {
        if self.same(other) {
            Ok(())
        } else {
            Err(AttributionFailure::ProcessChanged)
        }
    }
}
struct Protocol {
    channel: Channel,
    binding: Binding,
    id: u64,
}
struct PendingLease<'a> {
    lease: &'a PeerVerifierLease,
    armed: bool,
}
impl Drop for PendingLease<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.lease.revoke();
        }
    }
}
impl Protocol {
    fn observe(
        &mut self,
        deadline: Instant,
        lease: &PeerVerifierLease,
    ) -> Result<VerifiedPeerSnapshot, AttributionFailure> {
        self.id = self
            .id
            .checked_add(1)
            .ok_or(AttributionFailure::ProcUnavailable)?;
        self.channel
            .send(
                &self
                    .binding
                    .encode(wire::INSPECT, self.id)
                    .map_err(native)?,
                &[],
                deadline,
            )
            .map_err(native)?;
        let packet = self
            .channel
            .receive(FRAME, Some(deadline))
            .map_err(native)?;
        credentials(packet.credentials, lease.pid(), lease.uid()).map_err(native)?;
        self.binding
            .validate(&packet.bytes, wire::RESULT, self.id, packet.rights.len())
            .map_err(native)?;
        if packet.bytes[9] != 0 {
            return Err(
                match u16::from_be_bytes([packet.bytes[10], packet.bytes[11]]) {
                    1 => AttributionFailure::ProcUnavailable,
                    2 => AttributionFailure::ForeignUidDenied,
                    3 => AttributionFailure::ProcessChanged,
                    4 => AttributionFailure::ScanLimit,
                    6 => AttributionFailure::WorkerIsCaller,
                    _ => AttributionFailure::NativeMechanismUnverified,
                },
            );
        }
        let snapshot =
            VerifiedPeerSnapshot::from_packet(packet, &self.binding, true).map_err(native)?;
        fence(deadline).map_err(native)?;
        Ok(snapshot)
    }
}
pub(crate) struct PeerAttestation {
    protocol: Mutex<Protocol>,
    lease: PeerVerifierLease,
    source: VerifiedStaticInstalledImage,
    captured: Arc<CapturedBusScope>,
    snapshot: Arc<VerifiedPeerSnapshot>,
    _inspector: VerifiedPeerSnapshot,
}
impl PeerAttestation {
    pub(in crate::attribution) fn connect(
        socket: OwnedFd,
        lease: PeerVerifierLease,
        captured: Arc<CapturedBusScope>,
        source: VerifiedStaticInstalledImage,
    ) -> Result<Arc<Self>, AttributionFailure> {
        let mut pending = PendingLease {
            lease: &lease,
            armed: true,
        };
        captured.validate()?;
        source.revalidate().map_err(native)?;
        lease.validate().map_err(native)?;
        let deadline = source.deadline();
        fence(deadline).map_err(native)?;
        if captured.uid != lease.uid() {
            return Err(AttributionFailure::ProcessChanged);
        }
        let binding = Binding {
            uid: captured.uid,
            caller: captured.pid,
            caller_start: captured.start,
            nonce: captured.nonce,
            server: std::process::id(),
            server_start: crate::attribution::start_time(std::process::id())?,
            invocation: lease.invocation(),
            inspector: lease.pid(),
            inspector_start: lease.start(),
        };
        let channel = Channel::new(socket).map_err(native)?;
        channel
            .send(
                &binding.encode(wire::HELLO, 0).map_err(native)?,
                &[],
                deadline,
            )
            .map_err(native)?;
        let packet = channel.receive(FRAME, Some(deadline)).map_err(native)?;
        credentials(packet.credentials, lease.pid(), lease.uid()).map_err(native)?;
        binding
            .validate(&packet.bytes, wire::ACK, 0, packet.rights.len())
            .map_err(native)?;
        let inspector =
            VerifiedPeerSnapshot::from_packet(packet, &binding, false).map_err(native)?;
        let image =
            File::from(rustix::io::fcntl_dupfd_cloexec(&inspector.files[3], 3).map_err(native)?);
        lease.verify_image(&image).map_err(native)?;
        source.verify_image(&image).map_err(native)?;
        let mut protocol = Protocol {
            channel,
            binding,
            id: 0,
        };
        let observation_deadline = initial_observation_deadline(deadline, Instant::now());
        let snapshot = Arc::new(protocol.observe(observation_deadline, &lease)?);
        if snapshot.view != inspector.view {
            return Err(AttributionFailure::NativeMechanismUnverified);
        }
        captured.validate()?;
        source.validate().map_err(native)?;
        lease.validate().map_err(native)?;
        fence(observation_deadline).map_err(native)?;
        // Canonical supervisor сохраняет readonly snapshot до confirmed-empty.
        lease
            .retain_snapshot(Arc::clone(&snapshot))
            .map_err(native)?;
        fence(observation_deadline).map_err(native)?;
        pending.armed = false;
        drop(pending);
        Ok(Arc::new(Self {
            protocol: Mutex::new(protocol),
            lease,
            source,
            captured,
            snapshot,
            _inspector: inspector,
        }))
    }
    pub(in crate::attribution) fn matches(
        &self,
        lease: &PeerVerifierLease,
        source: &VerifiedStaticInstalledImage,
    ) -> bool {
        self.lease.same_observer(lease) && self.source.same_source(source)
    }
    pub(in crate::attribution) fn verify(&self) -> Result<(), AttributionFailure> {
        let deadline = Instant::now() + DEADLINE;
        self.captured.validate()?;
        self.source.validate().map_err(native)?;
        self.lease.validate().map_err(native)?;
        fence(deadline).map_err(native)?;
        let mut protocol = self
            .protocol
            .try_lock()
            .map_err(|_| AttributionFailure::ProcUnavailable)?;
        let fresh = match protocol.observe(deadline, &self.lease) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.lease.revoke();
                protocol.channel.shutdown();
                return Err(error);
            }
        };
        if !self.snapshot.same(&fresh) {
            self.lease.revoke();
            protocol.channel.shutdown();
            return Err(AttributionFailure::ProcessChanged);
        }
        self.captured.validate()?;
        self.source.validate().map_err(native)?;
        self.lease.validate().map_err(native)?;
        fence(deadline).map_err(native)
    }
    pub(in crate::attribution) fn view(&self) -> [(u64, u64); 3] {
        self.snapshot.namespace_view()
    }
    pub(in crate::attribution) fn append_identity(&self, hash: &mut sha2::Sha256) {
        use sha2::Digest;
        for (dev, ino) in self.snapshot.view {
            hash.update(dev.to_be_bytes());
            hash.update(ino.to_be_bytes());
        }
        hash.update(self.snapshot.executable.0.to_be_bytes());
        hash.update(self.snapshot.executable.1.to_be_bytes());
    }
}
fn native<E>(_: E) -> AttributionFailure {
    AttributionFailure::NativeMechanismUnverified
}
impl Drop for PeerAttestation {
    fn drop(&mut self) {
        self.lease.revoke();
        self.protocol
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .channel
            .shutdown();
    }
}
fn inspect_files(binding: &Binding, deadline: Instant) -> Result<[File; 4], AttributionFailure> {
    let pid = binding.caller;
    let verify = || -> Result<(), AttributionFailure> {
        if crate::attribution::start_time(pid)? != binding.caller_start
            || fs::metadata(format!("/proc/{pid}"))
                .map_err(|_| AttributionFailure::ProcUnavailable)?
                .uid()
                != binding.uid
        {
            return Err(AttributionFailure::ProcessChanged);
        }
        Ok(())
    };
    verify()?;
    crate::attribution::probe_process(pid)?;
    let denied = |e: std::io::Error| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            AttributionFailure::ForeignUidDenied
        } else {
            AttributionFailure::ProcUnavailable
        }
    };
    let files = [
        File::open(format!("/proc/{pid}/ns/net")).map_err(denied)?,
        File::open(format!("/proc/{pid}/ns/pid")).map_err(denied)?,
        File::open(format!("/proc/{pid}/ns/mnt")).map_err(denied)?,
        File::open(format!("/proc/{pid}/exe")).map_err(denied)?,
    ];
    for (fd, path) in files.iter().zip([
        format!("/proc/{pid}/ns/net"),
        format!("/proc/{pid}/ns/pid"),
        format!("/proc/{pid}/ns/mnt"),
        format!("/proc/{pid}/exe"),
    ]) {
        let before = fd.metadata().map_err(denied)?;
        let after = fs::metadata(path).map_err(denied)?;
        if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
            return Err(AttributionFailure::ProcessChanged);
        }
    }
    verify()?;
    fence(deadline).map_err(|_| AttributionFailure::ProcUnavailable)?;
    Ok(files)
}
pub(super) fn serve(
    channel: Channel,
    hello: Packet,
    bootstrap: Instant,
) -> Result<(), Unavailable> {
    if !hello.rights.is_empty() {
        return Err(Unavailable::Malformed);
    }
    let binding = Binding::hello(&hello.bytes)?;
    credentials(hello.credentials, binding.server, 0)?;
    if binding.inspector != std::process::id()
        || super::start(binding.server)? != binding.server_start
        || super::start(std::process::id())? != binding.inspector_start
    {
        return Err(Unavailable::Authority);
    }
    super::lookup_leaf::validate_identity(binding.uid)?;
    let files = [
        File::open("/proc/self/ns/net"),
        File::open("/proc/self/ns/pid"),
        File::open("/proc/self/ns/mnt"),
        File::open("/proc/self/exe"),
    ];
    let files: Vec<File> = files
        .into_iter()
        .collect::<std::io::Result<_>>()
        .map_err(|_| Unavailable::Coverage)?;
    channel.send(
        &binding.encode(wire::ACK, 0)?,
        &files.iter().map(AsFd::as_fd).collect::<Vec<_>>(),
        bootstrap,
    )?;
    drop(files);
    let mut last = 0;
    loop {
        let packet = match channel.receive(FRAME, None) {
            Ok(packet) => packet,
            Err(Unavailable::Disconnected) => return Ok(()),
            Err(error) => return Err(error),
        };
        let deadline = Instant::now() + DEADLINE;
        credentials(packet.credentials, binding.server, 0)?;
        let id = super::lookup_codec::u64_at(&packet.bytes, 88)?;
        if id <= last {
            return Err(Unavailable::Malformed);
        }
        last = id;
        binding.validate(&packet.bytes, wire::INSPECT, id, packet.rights.len())?;
        super::lookup_leaf::validate_identity(binding.uid)?;
        let mut reply = binding.encode(wire::RESULT, id)?;
        let files = inspect_files(&binding, deadline);
        match files {
            Ok(files) => channel.send(
                &reply,
                &files.iter().map(AsFd::as_fd).collect::<Vec<_>>(),
                deadline,
            )?,
            Err(error) => {
                reply[9] = 1;
                let reason: u16 = match error {
                    AttributionFailure::ProcUnavailable => 1,
                    AttributionFailure::ForeignUidDenied => 2,
                    AttributionFailure::ProcessChanged => 3,
                    AttributionFailure::ScanLimit => 4,
                    AttributionFailure::NativeMechanismUnverified => 5,
                    AttributionFailure::WorkerIsCaller => 6,
                };
                reply[10..12].copy_from_slice(&reason.to_be_bytes());
                channel.send(&reply, &[], deadline)?;
            }
        }
    }
}

fn initial_observation_deadline(admission: Instant, began: Instant) -> Instant {
    admission.min(began + DEADLINE)
}
#[cfg(test)]
mod deadline_tests {
    use super::*;
    #[test]
    fn pure_initial_attestation_finalization_never_uses_five_second_admission_budget() {
        let began = Instant::now();
        let admission = began + std::time::Duration::from_secs(5);
        let observation = initial_observation_deadline(admission, began);
        assert_eq!(observation, began + DEADLINE);
        let after_readonly_retain = began + DEADLINE + std::time::Duration::from_millis(1);
        assert_eq!(
            super::super::lookup_channel::fence_at(observation, after_readonly_retain),
            Err(Unavailable::Timeout)
        );
        assert_eq!(
            super::super::lookup_channel::fence_at(admission, after_readonly_retain),
            Ok(())
        );
        assert_eq!(
            super::super::lookup_channel::fence_at(observation, observation),
            Err(Unavailable::Timeout)
        );
        assert_eq!(
            super::super::lookup_channel::fence_at(
                observation,
                observation - std::time::Duration::from_nanos(1)
            ),
            Ok(())
        );
        let short = began + std::time::Duration::from_millis(1);
        assert_eq!(initial_observation_deadline(short, began), short);
    }
}
