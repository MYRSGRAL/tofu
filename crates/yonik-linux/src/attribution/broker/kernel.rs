use super::{
    DEADLINE, Flow, Instant, IoSliceMut, Lookup, OwnerSnapshot, Process, RecvAncillaryBuffer,
    RecvFlags, ReturnFlags, TrustedContext, Unavailable, fs, namespaces, recvmsg,
};
use rustix::net::{
    AddressFamily, SendFlags, SocketFlags, SocketType,
    netlink::{SOCK_DIAG, SocketAddrNetlink},
    socket_with,
};
use std::{
    os::unix::fs::MetadataExt,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_PROCESSES: usize = 256;
const MAX_TASKS: usize = 256;
const MAX_FDS_PER_TASK: usize = 256;
const MAX_DIAG_RECORDS: usize = 65536;

fn u32n(b: &[u8]) -> Result<u32, Unavailable> {
    Ok(u32::from_ne_bytes(
        b.try_into().map_err(|_| Unavailable::Malformed)?,
    ))
}
/// Полный bounded SOCK_DIAG dump: DUMP_INTR, truncation и overflow запрещают proof.
fn witness(
    flow: Flow,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<Option<(u64, u64)>, Unavailable> {
    let socket = socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        Some(SOCK_DIAG),
    )
    .map_err(|_| Unavailable::Coverage)?;
    rustix::net::sockopt::set_socket_recv_buffer_size(&socket, 32768)
        .map_err(|_| Unavailable::Coverage)?;
    rustix::net::sockopt::set_socket_send_buffer_size(&socket, 4096)
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
    let mut dump = DiagDump {
        flow,
        found: None,
        count: 0,
    };
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(Unavailable::Cancelled);
        }
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
            Err(error) => return Err(diag_error(error)),
        };
        let sender = msg
            .address
            .as_ref()
            .and_then(|a| SocketAddrNetlink::try_from(a.clone()).ok())
            .ok_or(Unavailable::Coverage)?;
        if sender.pid() != 0 || sender.groups() != 0 {
            return Err(Unavailable::Authority);
        }
        if let DumpProgress::Complete(result) = dump.consume(&b[..msg.bytes], msg.flags)? {
            return Ok(result);
        }
    }
}

fn diag_error(error: rustix::io::Errno) -> Unavailable {
    if error == rustix::io::Errno::NOBUFS {
        Unavailable::ReceiveOverflow
    } else {
        Unavailable::Coverage
    }
}
type SocketWitness = (u64, u64);
enum DumpProgress {
    Pending,
    Complete(Option<SocketWitness>),
}
struct DiagDump {
    flow: Flow,
    found: Option<(u64, u64)>,
    count: usize,
}
impl DiagDump {
    fn consume(&mut self, bytes: &[u8], flags: ReturnFlags) -> Result<DumpProgress, Unavailable> {
        if flags.intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC) {
            return Err(Unavailable::Coverage);
        }
        let mut offset = 0;
        while offset < bytes.len() {
            let h = bytes
                .get(offset..offset + 16)
                .ok_or(Unavailable::Malformed)?;
            let len = u32n(&h[..4])? as usize;
            if len < 16 || offset + len > bytes.len() || u32n(&h[8..12])? != 1 {
                return Err(Unavailable::Malformed);
            }
            let flags = u16::from_ne_bytes([h[6], h[7]]);
            if flags & 0x10 != 0 {
                return Err(Unavailable::Coverage);
            }
            let kind = u16::from_ne_bytes([h[4], h[5]]);
            let body = &bytes[offset + 16..offset + len];
            if kind == 4 {
                return Err(Unavailable::ReceiveOverflow);
            }
            if kind == 2 {
                let code = u32n(body.get(..4).ok_or(Unavailable::Malformed)?)? as i32;
                return Err(if code == -rustix::io::Errno::NOBUFS.raw_os_error() {
                    Unavailable::ReceiveOverflow
                } else {
                    Unavailable::Coverage
                });
            }
            if kind == 3 {
                if body.len() < 4 || offset + ((len + 3) & !3) != bytes.len() {
                    return Err(Unavailable::Malformed);
                }
                if u32n(&body[..4])? != 0 {
                    return Err(Unavailable::Coverage);
                }
                return Ok(DumpProgress::Complete(self.found));
            }
            if kind != 20 || body.len() < 72 {
                return Err(Unavailable::Coverage);
            }
            self.count += 1;
            if self.count > MAX_DIAG_RECORDS {
                return Err(Unavailable::Limit);
            }
            let cookie = u64::from(u32n(&body[44..48])?) | (u64::from(u32n(&body[48..52])?) << 32);
            if body[0] == self.flow.family
                && body[4..6] == self.flow.local_port.to_be_bytes()
                && body[6..8] == self.flow.remote_port.to_be_bytes()
                && body[8..24] == self.flow.local
                && body[24..40] == self.flow.remote
                && self.flow.cookie.is_none_or(|c| c == cookie)
            {
                if self.found.is_some() {
                    return Err(Unavailable::AmbiguousSocket);
                }
                self.found = Some((u64::from(u32n(&body[68..72])?), cookie));
            }
            offset += (len + 3) & !3;
        }
        Ok(DumpProgress::Pending)
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
    if context.cancelled.load(Ordering::Acquire) {
        return Err(Unavailable::Cancelled);
    }
    context.worker.validate()?;
    context.lookup_view.validate()?;
    if ![2, 10].contains(&flow.family) || ![6, 17].contains(&flow.protocol) {
        return Err(Unavailable::Malformed);
    }
    if flow.remote_port == 0 || flow.remote.iter().all(|v| *v == 0) {
        return Ok(None);
    }
    let begun = Instant::now();
    let deadline = begun + DEADLINE;
    let Some((inode, cookie)) = witness(flow, deadline, &context.cancelled)? else {
        return Ok(None);
    };
    let mut owner = None;
    let entries = fs::read_dir("/proc").map_err(|_| Unavailable::Coverage)?;
    let mut pids = 0;
    let mut task_count = 0;
    for entry in entries {
        if context.cancelled.load(Ordering::Acquire) {
            return Err(Unavailable::Cancelled);
        }
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
        pids += 1;
        if pids > MAX_PROCESSES {
            return Err(Unavailable::Limit);
        }
        let leader_uid = entry.metadata().map_err(|_| Unavailable::Coverage)?.uid();
        let proof = if leader_uid == context.target_uid {
            let process = Process::capture(pid, context.target_uid)?;
            let exe =
                fs::File::open(format!("/proc/{pid}/exe")).map_err(|_| Unavailable::Coverage)?;
            let before = exe.metadata().map_err(|_| Unavailable::Coverage)?;
            Some((process, exe, before))
        } else {
            None
        };
        // Метаданные task проверяются и у foreign leader: UID task может отличаться.
        let tasks = fs::read_dir(format!("/proc/{pid}/task")).map_err(|_| Unavailable::Coverage)?;
        let mut candidate = None;
        for task in tasks {
            task_count += 1;
            if task_count > MAX_TASKS {
                return Err(Unavailable::Limit);
            }
            if context.cancelled.load(Ordering::Acquire) {
                return Err(Unavailable::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(Unavailable::Timeout);
            }
            let task = task.map_err(|_| Unavailable::Coverage)?;
            if task.metadata().map_err(|_| Unavailable::Coverage)?.uid() != context.target_uid {
                continue;
            }
            let tid = task
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or(Unavailable::Coverage)?;
            // Разные credentials или namespace у task требуют отдельного authority.
            if fs::metadata(format!("/proc/{pid}"))
                .map_err(|_| Unavailable::Coverage)?
                .uid()
                != context.target_uid
                || namespaces(tid)? != context.lookup_view.namespaces
            {
                return Err(Unavailable::Coverage);
            }
            let task_start = super::start(tid)?;
            for (n, fd) in fs::read_dir(task.path().join("fd"))
                .map_err(|_| Unavailable::Coverage)?
                .enumerate()
            {
                if n >= MAX_FDS_PER_TASK {
                    return Err(Unavailable::Limit);
                }
                if context.cancelled.load(Ordering::Acquire) {
                    return Err(Unavailable::Cancelled);
                }
                if Instant::now() >= deadline {
                    return Err(Unavailable::Timeout);
                }
                let fd = fd.map_err(|_| Unavailable::Coverage)?;
                let number = fd
                    .file_name()
                    .to_str()
                    .and_then(|s| s.parse::<u32>().ok())
                    .ok_or(Unavailable::Coverage)?;
                if fs::read_link(fd.path())
                    .map_err(|_| Unavailable::Coverage)?
                    .as_os_str()
                    == format!("socket:[{inode}]").as_str()
                {
                    candidate = Some((tid, task_start, number));
                }
            }
            if task.metadata().map_err(|_| Unavailable::Coverage)?.uid() != context.target_uid
                || super::start(tid)? != task_start
                || namespaces(tid)? != context.lookup_view.namespaces
            {
                return Err(Unavailable::Coverage);
            }
        }
        let Some((task_id, task_start, fd_number)) = candidate else {
            continue;
        };
        let (process, exe, before) = proof.ok_or(Unavailable::Coverage)?;
        process.validate()?;
        let after = fs::metadata(format!("/proc/{pid}/exe")).map_err(|_| Unavailable::Coverage)?;
        if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
            return Err(Unavailable::Coverage);
        }
        {
            if owner.is_some() {
                return Ok(None);
            }
            #[cfg(test)]
            capture_hook();
            owner = Some(OwnerSnapshot {
                socket_inode: inode,
                task_id,
                task_start,
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
    if witness(flow, deadline, &context.cancelled)? != Some((inode, cookie)) {
        return Ok(None);
    }
    context.worker.validate()?;
    if context.cancelled.load(Ordering::Acquire) {
        return Err(Unavailable::Cancelled);
    }
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
        task_id: s.task_id,
        task_start: s.task_start,
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
    context.lookup_view.validate()?;
    let process = Process::capture(s.pid, context.target_uid)?;
    if process.start != s.start_time || process.namespaces != context.lookup_view.namespaces {
        return Err(Unavailable::Authority);
    }
    let exe = fs::metadata(format!("/proc/{}/exe", s.pid)).map_err(|_| Unavailable::Coverage)?;
    if (exe.dev(), exe.ino()) != s.executable {
        return Err(Unavailable::Authority);
    }
    if super::start(s.task_id)? != s.task_start
        || namespaces(s.task_id)? != context.lookup_view.namespaces
    {
        return Err(Unavailable::Authority);
    }
    let target = fs::read_link(format!(
        "/proc/{}/task/{}/fd/{}",
        s.pid, s.task_id, s.socket_fd
    ))
    .map_err(|_| Unavailable::Coverage)?;
    if target.as_os_str() != format!("socket:[{}]", s.socket_inode).as_str() {
        return Err(Unavailable::Authority);
    }
    if witness(flow, s.observed_from + DEADLINE, &context.cancelled)?
        != Some((s.socket_inode, s.cookie))
    {
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
thread_local! {static CAPTURE_HOOK:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=std::cell::RefCell::new(None);}
#[cfg(test)]
fn capture_hook() {
    CAPTURE_HOOK.with(|h| {
        if let Some(hook) = h.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{Flow, Unavailable};
    fn witness(
        flow: Flow,
        deadline: std::time::Instant,
    ) -> Result<Option<(u64, u64)>, Unavailable> {
        super::witness(flow, deadline, &std::sync::atomic::AtomicBool::new(false))
    }
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
    fn cancellation_during_actual_capture_releases_partial_proof() {
        if std::env::var_os("YONIK_BROKER_GATES").is_none() {
            return;
        }
        use crate::attribution::broker::{Broker, Lookup, TrustedContext};
        use std::{net::UdpSocket, sync::mpsc, time::Duration};
        let fds = || std::fs::read_dir("/proc/self/fd").unwrap().count();
        let baseline = fds();
        let begun = Instant::now();
        {
            let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
            socket.connect(peer.local_addr().unwrap()).unwrap();
            let uid = rustix::process::getuid().as_raw();
            let mut b = Broker::new(
                TrustedContext::from_host(std::process::id(), uid, uid, 7, [1; 32], [2; 32])
                    .unwrap(),
            )
            .unwrap();
            let cancel = b.cancellation();
            let (go, ready) = mpsc::channel();
            let (ack, done) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                ready.recv_timeout(Duration::from_secs(1)).unwrap();
                cancel.cancel();
                ack.send(()).unwrap();
            });
            super::CAPTURE_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    go.send(()).unwrap();
                    done.recv_timeout(Duration::from_secs(1)).unwrap();
                }))
            });
            let result = b.lookup(flow(
                socket.local_addr().unwrap(),
                socket.peer_addr().unwrap(),
                17,
            ));
            worker.join().unwrap();
            assert!(matches!(
                result,
                Lookup::MechanismUnavailable(Unavailable::Cancelled)
            ));
            assert!(b.cache.is_empty());
        }
        assert_eq!(fds(), baseline);
        assert!(begun.elapsed() < Duration::from_secs(1));
        eprintln!("in-flight cancel+teardown {:?}", begun.elapsed());
    }
    #[test]
    fn retained_projection_positive_and_stale_replaced_deleted_epoch() {
        retained_projection_retry(
            Instant::now() + Duration::from_secs(1),
            retained_projection_attempt,
        )
        .unwrap();
    }
    fn retained_projection_retry(
        deadline: Instant,
        mut observe: impl FnMut(Instant) -> Result<(), Unavailable>,
    ) -> Result<(), Unavailable> {
        let begun = Instant::now();
        for attempt in 1..=3 {
            if Instant::now() >= deadline {
                return Err(Unavailable::Timeout);
            }
            let result = observe(deadline);
            eprintln!(
                "retained attempt={attempt}, elapsed={:?}, result={result:?}",
                begun.elapsed()
            );
            match result {
                Ok(()) if Instant::now() <= deadline => return Ok(()),
                Ok(()) => return Err(Unavailable::Timeout),
                Err(Unavailable::Timeout) if attempt < 3 => {}
                other => return other,
            }
        }
        Err(Unavailable::Timeout)
    }
    #[test]
    fn retained_retry_is_bounded_and_only_timeout_is_retryable() {
        let mut calls = 0;
        let deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            retained_projection_retry(deadline, |_| {
                calls += 1;
                Err(Unavailable::Timeout)
            }),
            Err(Unavailable::Timeout)
        );
        assert_eq!(calls, 3);
        calls = 0;
        assert_eq!(
            retained_projection_retry(deadline, |_| {
                calls += 1;
                Err(Unavailable::Authority)
            }),
            Err(Unavailable::Authority)
        );
        assert_eq!(calls, 1);
        calls = 0;
        assert_eq!(
            retained_projection_retry(Instant::now(), |_| {
                calls += 1;
                Ok(())
            }),
            Err(Unavailable::Timeout)
        );
        assert_eq!(calls, 0);
    }
    fn retained_projection_attempt(deadline: Instant) -> Result<(), Unavailable> {
        use super::super::{Broker, Process, TrustedContext};
        use std::{
            collections::VecDeque,
            fs,
            os::{fd::AsRawFd, unix::fs::MetadataExt},
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client =
            std::sync::Arc::new(TcpStream::connect(listener.local_addr().unwrap()).unwrap());
        let (_peer, _) = listener.accept().unwrap();
        let f = flow(client.local_addr().unwrap(), client.peer_addr().unwrap(), 6);
        let pid = std::process::id();
        let uid = rustix::process::getuid().as_raw();
        let context = TrustedContext::from_host(pid, uid, uid, 7, [1; 32], [2; 32]).unwrap();
        fn observe(
            client: &TcpStream,
            f: Flow,
            epoch: u64,
            deadline: Instant,
        ) -> Result<super::OwnerSnapshot, Unavailable> {
            let observed_from = Instant::now();
            let pid = std::process::id();
            let uid = rustix::process::getuid().as_raw();
            let process = Process::capture(pid, uid).unwrap();
            let exe = fs::File::open("/proc/self/exe").unwrap();
            let m = exe.metadata().unwrap();
            let result = witness(f, (observed_from + super::DEADLINE).min(deadline));
            eprintln!(
                "retained capture/witness elapsed={:?}, result={result:?}",
                observed_from.elapsed()
            );
            let (inode, cookie) = result?.expect("socket fixture должен присутствовать");
            assert_eq!(
                fs::read_link(format!("/proc/{pid}/task/{pid}/fd/{}", client.as_raw_fd()))
                    .unwrap()
                    .to_str()
                    .unwrap(),
                format!("socket:[{inode}]")
            );
            process.validate().unwrap();
            Ok(super::OwnerSnapshot {
                pidfd: process.pidfd,
                executable_fd: exe,
                pid,
                start_time: process.start,
                executable: (m.dev(), m.ino()),
                cookie,
                epoch,
                socket_inode: inode,
                task_id: pid,
                task_start: process.start,
                socket_fd: client.as_raw_fd() as u32,
                observed_from,
                observed_until: Instant::now(),
            })
        }
        let mut snapshot = observe(&client, f, 7, deadline)?;
        let mut broker = Broker {
            context,
            cache: VecDeque::new(),
            last_cold: None,
        };
        let projected = broker.project(f, &snapshot, 7);
        eprintln!(
            "retained direct project elapsed={:?}, result={:?}",
            snapshot.observed_from.elapsed(),
            projected.as_ref().map(|(_, _, ttl)| ttl)
        );
        let (_, path, ttl) = projected?;
        assert_eq!(
            path,
            fs::read_link("/proc/self/exe").unwrap().to_str().unwrap()
        );
        assert!(ttl > Duration::ZERO && ttl <= Duration::from_millis(20));
        let wire_client = std::sync::Arc::clone(&client);
        let wire_broker = Broker {
            context: TrustedContext::from_host(pid, uid, uid, 1, [3; 32], [2; 32]).unwrap(),
            cache: VecDeque::new(),
            last_cold: None,
        };
        crate::attribution::native_session::known_roundtrip(
            wire_broker,
            move |broker| {
                let snapshot = observe(&wire_client, f, 1, deadline)?;
                // Lookup и project повторно проверяют реальный snapshot; лишняя
                // тестовая проверка здесь расходовала тот же production TTL.
                broker.cache = VecDeque::from([(f, snapshot)]);
                Ok(())
            },
            f,
            deadline,
        )?;

        let stale = super::duplicate(&snapshot).unwrap();
        std::thread::sleep(super::DEADLINE.saturating_sub(stale.observed_from.elapsed()));
        assert!(matches!(
            broker.project(f, &stale, 7),
            Err(Unavailable::Timeout)
        ));
        snapshot = observe(&client, f, 7, deadline)?;
        assert!(broker.project(f, &snapshot, 8).is_err());
        snapshot = observe(&client, f, 7, deadline)?;
        snapshot.executable.1 ^= 1;
        assert!(broker.project(f, &snapshot, 7).is_err());
        snapshot = observe(&client, f, 7, deadline)?;
        let path = std::env::temp_dir().join(format!("yonik-deleted-exe-{}", pid));
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        snapshot.executable_fd = file;
        // Реально заменённый retained FD не соответствует исходному executable.
        assert!(broker.project(f, &snapshot, 7).is_err());
        snapshot = observe(&client, f, 7, deadline)?;
        snapshot.executable_fd = fs::File::open(&path).unwrap();
        let metadata = snapshot.executable_fd.metadata().unwrap();
        snapshot.executable = (metadata.dev(), metadata.ino());
        fs::remove_file(&path).unwrap();
        assert!(matches!(
            broker.project(f, &snapshot, 7),
            Err(Unavailable::Coverage)
        ));
        snapshot = observe(&client, f, 7, deadline)?;
        broker.revoke();
        assert!(broker.project(f, &snapshot, 7).is_err());
        Ok(())
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
            task_id: pid,
            task_start: process.start,
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

#[cfg(test)]
#[path = "diag_tests.rs"]
mod diag_tests;
