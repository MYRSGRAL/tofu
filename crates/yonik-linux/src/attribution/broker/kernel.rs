use super::{
    DEADLINE, Flow, Instant, IoSliceMut, Lookup, OwnerSnapshot, Process, RecvAncillaryBuffer,
    RecvFlags, ReturnFlags, TrustedContext, Unavailable, fs, namespaces, recvmsg,
};
use rustix::net::{
    AddressFamily, SendFlags, SocketFlags, SocketType,
    netlink::{SOCK_DIAG, SocketAddrNetlink},
    socket_with,
};
use std::os::unix::fs::MetadataExt;

fn u32n(b: &[u8]) -> Result<u32, Unavailable> {
    Ok(u32::from_ne_bytes(
        b.try_into().map_err(|_| Unavailable::Malformed)?,
    ))
}
/// Полный bounded SOCK_DIAG dump: DUMP_INTR, truncation и overflow запрещают proof.
fn witness(flow: Flow, deadline: Instant) -> Result<Option<(u64, u64)>, Unavailable> {
    let socket = socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        Some(SOCK_DIAG),
    )
    .map_err(|_| Unavailable::Coverage)?;
    let mut request = [0u8; 72];
    request[..4].copy_from_slice(&72u32.to_ne_bytes());
    request[4..6].copy_from_slice(&20u16.to_ne_bytes());
    request[6..8].copy_from_slice(&0x301u16.to_ne_bytes());
    request[8..12].copy_from_slice(&1u32.to_ne_bytes());
    request[16] = flow.family;
    request[17] = flow.protocol;
    request[20..24].fill(255);
    request[64..72].fill(255);
    rustix::net::sendto(
        &socket,
        &request,
        SendFlags::empty(),
        &SocketAddrNetlink::new(0, 0),
    )
    .map_err(|_| Unavailable::Coverage)?;
    let mut found = None;
    let mut count = 0;
    loop {
        if Instant::now() >= deadline {
            return Err(Unavailable::Timeout);
        }
        let mut b = [0u8; 32768];
        let mut space = [];
        let mut ancillary = RecvAncillaryBuffer::new(&mut space);
        let msg = match recvmsg(
            &socket,
            &mut [IoSliceMut::new(&mut b)],
            &mut ancillary,
            RecvFlags::DONTWAIT,
        ) {
            Ok(m) => m,
            Err(rustix::io::Errno::AGAIN) => {
                std::thread::yield_now();
                continue;
            }
            Err(_) => return Err(Unavailable::Coverage),
        };
        let sender = msg
            .address
            .as_ref()
            .and_then(|a| SocketAddrNetlink::try_from(a.clone()).ok())
            .ok_or(Unavailable::Coverage)?;
        if sender.pid() != 0 || sender.groups() != 0 {
            return Err(Unavailable::Authority);
        }
        if msg
            .flags
            .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
        {
            return Err(Unavailable::Coverage);
        }
        let mut offset = 0;
        while offset < msg.bytes {
            let h = b.get(offset..offset + 16).ok_or(Unavailable::Malformed)?;
            let len = u32n(&h[..4])? as usize;
            if len < 16 || offset + len > msg.bytes || u32n(&h[8..12])? != 1 {
                return Err(Unavailable::Malformed);
            }
            let flags = u16::from_ne_bytes([h[6], h[7]]);
            if flags & 0x10 != 0 {
                return Err(Unavailable::Coverage);
            }
            let kind = u16::from_ne_bytes([h[4], h[5]]);
            let body = &b[offset + 16..offset + len];
            if kind == 3 {
                if body.len() >= 4 && u32n(&body[..4])? != 0 {
                    return Err(Unavailable::Coverage);
                }
                return Ok(found);
            }
            if kind != 20 || body.len() < 72 {
                return Err(Unavailable::Coverage);
            }
            count += 1;
            if count > 65536 {
                return Err(Unavailable::Limit);
            }
            let cookie = u64::from(u32n(&body[44..48])?) | (u64::from(u32n(&body[48..52])?) << 32);
            if body[0] == flow.family
                && body[4..6] == flow.local_port.to_be_bytes()
                && body[6..8] == flow.remote_port.to_be_bytes()
                && body[8..24] == flow.local
                && body[24..40] == flow.remote
                && flow.cookie.is_none_or(|c| c == cookie)
            {
                if found.is_some() {
                    return Err(Unavailable::Coverage);
                }
                found = Some((u64::from(u32n(&body[68..72])?), cookie));
            }
            offset += (len + 3) & !3;
        }
    }
}

pub(super) fn lookup(context: &TrustedContext, flow: Flow) -> Lookup {
    match capture(context, flow) {
        Ok(Some(s)) => Lookup::KnownOwner(s),
        Ok(None) => Lookup::UnknownFlow,
        Err(e) => Lookup::MechanismUnavailable(e),
    }
}
fn capture(context: &TrustedContext, flow: Flow) -> Result<Option<OwnerSnapshot>, Unavailable> {
    if context.revoked {
        return Err(Unavailable::Revoked);
    }
    context.worker.validate()?;
    if ![2, 10].contains(&flow.family) || ![6, 17].contains(&flow.protocol) {
        return Err(Unavailable::Malformed);
    }
    if flow.remote_port == 0 || flow.remote.iter().all(|v| *v == 0) {
        return Ok(None);
    }
    let begun = Instant::now();
    let deadline = begun + DEADLINE;
    if namespaces(std::process::id())? != context.worker.namespaces {
        return Err(Unavailable::Coverage);
    }
    let Some((inode, cookie)) = witness(flow, deadline)? else {
        return Ok(None);
    };
    let mut owner = None;
    let entries = fs::read_dir("/proc").map_err(|_| Unavailable::Coverage)?;
    let mut pids = 0;
    for entry in entries {
        if Instant::now() >= deadline {
            return Err(Unavailable::Timeout);
        }
        let entry = entry.map_err(|_| Unavailable::Coverage)?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let meta = entry.metadata().map_err(|_| Unavailable::Coverage)?;
        if meta.uid() != context.target_uid {
            continue;
        }
        pids += 1;
        if pids > 256 {
            return Err(Unavailable::Limit);
        }
        let process = Process::capture(pid, context.target_uid)?;
        if process.namespaces != context.worker.namespaces {
            return Err(Unavailable::Coverage);
        }
        let exe = fs::File::open(format!("/proc/{pid}/exe")).map_err(|_| Unavailable::Coverage)?;
        let before = exe.metadata().map_err(|_| Unavailable::Coverage)?;
        let mut matched = None;
        for (n, fd) in fs::read_dir(format!("/proc/{pid}/fd"))
            .map_err(|_| Unavailable::Coverage)?
            .enumerate()
        {
            if n >= 256 {
                return Err(Unavailable::Limit);
            }
            if Instant::now() >= deadline {
                return Err(Unavailable::Timeout);
            }
            let entry = fd.map_err(|_| Unavailable::Coverage)?;
            let fd_number = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or(Unavailable::Coverage)?;
            let path = entry.path();
            let target = fs::read_link(path).map_err(|_| Unavailable::Coverage)?;
            if target.as_os_str() == format!("socket:[{inode}]").as_str() {
                matched = Some(fd_number);
            }
        }
        process.validate()?;
        let after = fs::metadata(format!("/proc/{pid}/exe")).map_err(|_| Unavailable::Coverage)?;
        if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
            return Err(Unavailable::Coverage);
        }
        if let Some(fd_number) = matched {
            if owner.is_some() {
                return Ok(None);
            }
            owner = Some(OwnerSnapshot {
                socket_inode: inode,
                socket_fd: fd_number,
                pidfd: process.pidfd,
                executable_fd: exe,
                observed_from: begun,
                observed_until: Instant::now(),
                pid,
                start_time: process.start,
                executable: (before.dev(), before.ino()),
                cookie,
                epoch: context.epoch,
            });
        }
    }
    if witness(flow, deadline)? != Some((inode, cookie)) {
        return Ok(None);
    }
    context.worker.validate()?;
    if Instant::now() >= deadline {
        return Err(Unavailable::Timeout);
    }
    if let Some(snapshot) = owner.as_mut() {
        snapshot.observed_until = Instant::now();
    }
    Ok(owner)
}

/// Cache хранит прежний observation interval; каждый hit вновь проверяет owner/exe.
pub(super) fn duplicate(s: &OwnerSnapshot) -> Result<OwnerSnapshot, Unavailable> {
    Ok(OwnerSnapshot {
        socket_inode: s.socket_inode,
        socket_fd: s.socket_fd,
        pidfd: s.pidfd.try_clone().map_err(|_| Unavailable::Limit)?,
        executable_fd: s
            .executable_fd
            .try_clone()
            .map_err(|_| Unavailable::Limit)?,
        pid: s.pid,
        start_time: s.start_time,
        executable: s.executable,
        cookie: s.cookie,
        epoch: s.epoch,
        observed_from: s.observed_from,
        observed_until: s.observed_until,
    })
}
pub(super) fn revalidate(
    context: &TrustedContext,
    flow: Flow,
    s: &OwnerSnapshot,
) -> Result<OwnerSnapshot, Unavailable> {
    if s.epoch != context.epoch || s.observed_from.elapsed() >= DEADLINE {
        return Err(Unavailable::Timeout);
    }
    context.worker.validate()?;
    let process = Process::capture(s.pid, context.target_uid)?;
    if process.start != s.start_time || process.namespaces != context.worker.namespaces {
        return Err(Unavailable::Authority);
    }
    let exe = fs::metadata(format!("/proc/{}/exe", s.pid)).map_err(|_| Unavailable::Coverage)?;
    if (exe.dev(), exe.ino()) != s.executable {
        return Err(Unavailable::Authority);
    }
    let target = fs::read_link(format!("/proc/{}/fd/{}", s.pid, s.socket_fd))
        .map_err(|_| Unavailable::Coverage)?;
    if target.as_os_str() != format!("socket:[{}]", s.socket_inode).as_str() {
        return Err(Unavailable::Authority);
    }
    let mut precise = flow;
    precise.cookie = Some(s.cookie);
    if witness(precise, s.observed_from + DEADLINE)? != Some((s.socket_inode, s.cookie)) {
        return Err(Unavailable::Coverage);
    }
    process.validate()?;
    let after = fs::metadata(format!("/proc/{}/exe", s.pid)).map_err(|_| Unavailable::Coverage)?;
    if (after.dev(), after.ino()) != s.executable {
        return Err(Unavailable::Authority);
    }
    if s.observed_from.elapsed() >= DEADLINE {
        return Err(Unavailable::Timeout);
    }
    let mut result = duplicate(s)?;
    result.observed_until = Instant::now();
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::{Flow, Unavailable, witness};
    use std::{
        net::{SocketAddr, TcpListener, TcpStream, UdpSocket},
        time::{Duration, Instant},
    };
    fn flow(local: SocketAddr, remote: SocketAddr, protocol: u8) -> Flow {
        let mut a = [0; 16];
        let mut b = [0; 16];
        let family = match (local, remote) {
            (SocketAddr::V4(l), SocketAddr::V4(r)) => {
                a[..4].copy_from_slice(&l.ip().octets());
                b[..4].copy_from_slice(&r.ip().octets());
                2
            }
            (SocketAddr::V6(l), SocketAddr::V6(r)) => {
                a = l.ip().octets();
                b = r.ip().octets();
                10
            }
            _ => panic!("семейства fixture должны совпадать"),
        };
        Flow {
            family,
            protocol,
            local_port: local.port(),
            remote_port: remote.port(),
            local: a,
            remote: b,
            cookie: None,
        }
    }
    #[test]
    fn cache_rejects_reuse_executable_and_epoch_without_churn() {
        use super::{OwnerSnapshot, Process, TrustedContext, revalidate};
        use std::{fs, os::unix::fs::MetadataExt};
        let pid = std::process::id();
        let uid = rustix::process::getuid().as_raw();
        let context = TrustedContext::from_host(pid, uid, uid, 7, [1; 32], [2; 32]).unwrap();
        let process = Process::capture(pid, uid).unwrap();
        let exe = fs::File::open(format!("/proc/{pid}/exe")).unwrap();
        let m = exe.metadata().unwrap();
        let mut s = OwnerSnapshot {
            pidfd: process.pidfd,
            executable_fd: exe,
            pid,
            start_time: process.start,
            executable: (m.dev(), m.ino()),
            cookie: 1,
            epoch: 7,
            socket_inode: 1,
            socket_fd: 0,
            observed_from: Instant::now(),
            observed_until: Instant::now(),
        };
        let f = Flow {
            family: 2,
            protocol: 6,
            local_port: 1,
            remote_port: 2,
            local: [0; 16],
            remote: [0; 16],
            cookie: None,
        };
        s.start_time = s.start_time.wrapping_add(1);
        assert!(matches!(
            revalidate(&context, f, &s),
            Err(Unavailable::Authority)
        ));
        s.start_time = process.start;
        s.executable.1 ^= 1;
        assert!(matches!(
            revalidate(&context, f, &s),
            Err(Unavailable::Authority)
        ));
        s.executable.1 ^= 1;
        s.epoch = 8;
        assert!(matches!(
            revalidate(&context, f, &s),
            Err(Unavailable::Timeout)
        ));
    }
    #[test]
    fn diag_tcp_udp_v4_v6_cookie_and_deadline() {
        for address in ["127.0.0.1:0", "[::1]:0"] {
            let listener = TcpListener::bind(address).unwrap();
            let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (_peer, _) = listener.accept().unwrap();
            let tcp = flow(client.local_addr().unwrap(), client.peer_addr().unwrap(), 6);
            let proof = witness(tcp, Instant::now() + Duration::from_secs(1))
                .unwrap()
                .unwrap();
            assert_ne!(proof.0, 0);
            let mut wrong = tcp;
            wrong.cookie = Some(proof.1.wrapping_add(1));
            assert_eq!(
                witness(wrong, Instant::now() + Duration::from_secs(1)),
                Ok(None)
            );
            let a = UdpSocket::bind(address).unwrap();
            let b = UdpSocket::bind(address).unwrap();
            a.connect(b.local_addr().unwrap()).unwrap();
            let udp = flow(a.local_addr().unwrap(), b.local_addr().unwrap(), 17);
            assert!(
                witness(udp, Instant::now() + Duration::from_secs(1))
                    .unwrap()
                    .is_some()
            );
            assert_eq!(witness(udp, Instant::now()), Err(Unavailable::Timeout));
        }
    }
}
