//! Fixed worker --lookup: только scoped kernel lookup под app UID/caps0.
use super::lookup_channel::{Channel, credentials, fence};
use super::lookup_codec::{self as wire, FRAME, HELLO_ACK, LOOKUP, REVOKE};
use super::{Broker, DEADLINE, LookupBinding, ProjectedLookup, TrustedContext, Unavailable};
use std::{
    fs::File,
    os::fd::AsFd,
    time::{Duration, Instant},
};

pub(super) fn run() -> Result<(), Unavailable> {
    let socket = rustix::io::fcntl_dupfd_cloexec(std::io::stdin().as_fd(), 3)
        .map_err(|_| Unavailable::Authority)?;
    let channel = Channel::new(socket)?;
    let bootstrap = Instant::now() + Duration::from_secs(1);
    let packet = channel.receive(FRAME, Some(bootstrap))?;
    if !packet.rights.is_empty() {
        return Err(Unavailable::Malformed);
    }
    let binding = LookupBinding::decode_hello(&packet.bytes)?;
    credentials(packet.credentials, binding.server_pid, 0)?;
    if super::start(binding.server_pid)? != binding.server_start {
        return Err(Unavailable::Authority);
    }
    let mut broker = Broker::lookup_leaf(&binding)?;
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
    let rights: Vec<_> = files.iter().map(AsFd::as_fd).collect();
    channel.send(&binding.encode(HELLO_ACK, 0)?, &rights, bootstrap)?;
    drop(files);
    let mut last = 0u64;
    loop {
        // Idle блокируется на owned endpoint. EOF/revoke завершает leaf; нет idle timer.
        let packet = match channel.receive(FRAME, None) {
            Ok(packet) => packet,
            Err(Unavailable::Disconnected) => {
                broker.revoke();
                return Ok(());
            }
            Err(reason) => return Err(reason),
        };
        credentials(packet.credentials, binding.server_pid, 0)?;
        if !packet.rights.is_empty() {
            return Err(Unavailable::Malformed);
        }
        if packet.bytes[8] == REVOKE {
            binding.validate(&packet.bytes, REVOKE, 0, FRAME)?;
            if packet.bytes[148..].iter().any(|b| *b != 0) {
                return Err(Unavailable::Malformed);
            }
            broker.revoke();
            return Ok(());
        }
        let id = wire::u64_at(&packet.bytes, 24)?;
        if id <= last {
            return Err(Unavailable::Malformed);
        }
        last = id;
        binding.validate(&packet.bytes, LOOKUP, id, FRAME)?;
        let flow = binding.flow(&packet.bytes, id)?;
        let deadline = Instant::now() + DEADLINE;
        let result = broker.lookup_and_project(flow, binding.epoch);
        let (mut response, owner) = match result {
            ProjectedLookup::Unknown => (wire::result_header(&binding, id, 0, 0)?, None),
            ProjectedLookup::Unavailable(reason) => (
                wire::result_header(&binding, id, 2, wire::reason_code(reason))?,
                None,
            ),
            ProjectedLookup::Known(projection) => {
                let (owner, path, ttl) = projection.into_parts();
                let ttl = ttl.min(deadline.saturating_duration_since(Instant::now()));
                if ttl.is_zero() {
                    return Err(Unavailable::Timeout);
                }
                let mut response = wire::result_header(&binding, id, 1, 0)?;
                for (offset, value) in [
                    (148, owner.pid),
                    (152, owner.task_id),
                    (180, owner.socket_fd),
                ] {
                    response[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
                }
                for (offset, value) in [
                    (156, owner.start_time),
                    (164, owner.task_start),
                    (172, owner.socket_inode),
                    (184, owner.cookie),
                    (192, owner.executable.0),
                    (200, owner.executable.1),
                    (208, ttl.as_nanos() as u64),
                ] {
                    response[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
                }
                response[216..218].copy_from_slice(&(path.len() as u16).to_be_bytes());
                response[256..256 + path.len()].copy_from_slice(path.as_bytes());
                (response, Some(owner))
            }
        };
        // FDs только validated pidfd/executable_fd, никогда TUN/activation roles.
        let rights = owner
            .as_ref()
            .map(|o| vec![o.pidfd.as_fd(), o.executable_fd.as_fd()])
            .unwrap_or_default();
        fence(deadline)?;
        channel.send(&response, &rights, deadline)?;
        response.fill(0);
    }
}
