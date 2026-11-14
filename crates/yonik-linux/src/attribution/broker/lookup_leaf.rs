//! Fixed worker --lookup: только scoped kernel lookup под app UID/caps0.
use super::lookup_channel::{Channel, credentials, fence};
use super::lookup_codec::{self as wire, FRAME, HELLO_ACK, LOOKUP, REVOKE};
use super::{Broker, DEADLINE, LookupBinding, ProjectedLookup, Unavailable};
use std::{
    fs::File,
    os::fd::AsFd,
    time::{Duration, Instant},
};

pub(super) fn run() -> Result<(), Unavailable> {
    // До HELLO: same-UID ptrace_scope=0 не должен позволить изменить leaf.
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)
        .map_err(|_| Unavailable::Authority)?;
    let socket = rustix::io::fcntl_dupfd_cloexec(std::io::stdin().as_fd(), 3)
        .map_err(|_| Unavailable::Authority)?;
    let channel = Channel::new(socket)?;
    let bootstrap = Instant::now() + Duration::from_secs(1);
    let packet = channel.receive(FRAME, Some(bootstrap))?;
    if !packet.rights.is_empty() {
        return Err(Unavailable::Malformed);
    }
    if packet.bytes.get(..8) == Some(super::inspect_codec::MAGIC.as_slice()) {
        return super::inspect_peer::serve(channel, packet, bootstrap);
    }
    let binding = LookupBinding::decode_hello(&packet.bytes)?;
    credentials(packet.credentials, binding.server_pid, 0)?;
    if super::start(binding.server_pid)? != binding.server_start {
        return Err(Unavailable::Authority);
    }
    validate_identity(binding.target_uid)?;
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
    for (file, expected) in files[..3].iter().zip(broker.context.lookup_view.namespaces) {
        let metadata = file.metadata().map_err(|_| Unavailable::Coverage)?;
        use std::os::unix::fs::MetadataExt;
        if (metadata.dev(), metadata.ino()) != expected {
            return Err(Unavailable::Coverage);
        }
    }
    broker.context.validate_scope()?;
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
        validate_identity(binding.target_uid)?;
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
                let value = wire::KnownData {
                    pid: owner.pid,
                    task_id: owner.task_id,
                    start: owner.start_time,
                    task_start: owner.task_start,
                    socket_inode: owner.socket_inode,
                    socket_fd: owner.socket_fd,
                    cookie: owner.cookie,
                    executable: owner.executable,
                    nanos: ttl.as_nanos() as u64,
                    path,
                };
                let response = wire::known_frame(&binding, id, &value)?;
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

pub(super) fn validate_identity(uid: u32) -> Result<(), Unavailable> {
    let status = super::bounded("/proc/self/status")?;
    validate_status(&status, uid)?;
    if rustix::process::getuid().as_raw() != uid || rustix::process::geteuid().as_raw() != uid {
        return Err(Unavailable::Authority);
    }
    Ok(())
}
fn validate_status(status: &str, uid: u32) -> Result<(), Unavailable> {
    if uid == 0 {
        return Err(Unavailable::Authority);
    }
    for name in [
        "Uid:",
        "CapInh:",
        "CapPrm:",
        "CapEff:",
        "CapBnd:",
        "CapAmb:",
        "NoNewPrivs:",
    ] {
        let mut values = status.lines().filter_map(|line| line.strip_prefix(name));
        let value = values.next().ok_or(Unavailable::Authority)?;
        if values.next().is_some() {
            return Err(Unavailable::Authority);
        }
        let valid = if name == "Uid:" {
            let ids = value
                .split_whitespace()
                .map(str::parse::<u32>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| Unavailable::Authority)?;
            ids == [uid; 4]
        } else if name == "NoNewPrivs:" {
            value.trim() == "1"
        } else {
            u64::from_str_radix(value.trim(), 16).ok() == Some(0)
        };
        if !valid {
            return Err(Unavailable::Authority);
        }
    }
    Ok(())
}
#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn pure_leaf_status_checks_saved_fs_uid_bounding_caps_and_nnp() {
        let status = "Uid: 1000 1000 1000 1000\nCapInh: 0\nCapPrm: 0\nCapEff: 0\nCapBnd: 0\nCapAmb: 0\nNoNewPrivs: 1\n";
        assert_eq!(validate_status(status, 1000), Ok(()));
        for bad in [
            status.replace("1000 1000 1000 1000", "1000 1000 0 1000"),
            status.replace("CapBnd: 0", "CapBnd: 1"),
            status.replace("NoNewPrivs: 1", "NoNewPrivs: 0"),
            format!("{status}CapEff: 1\n"),
        ] {
            assert!(validate_status(&bad, 1000).is_err());
        }
    }
}
