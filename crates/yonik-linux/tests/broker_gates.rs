//! Actual broker regressions: выполнять только в собственном user/net/pid namespace.
use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use yonik_linux::attribution::broker::{Broker, Flow, Lookup, TrustedContext};

struct Fixture(Child);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn enabled() -> bool {
    std::env::var_os("YONIK_BROKER_GATES").is_some()
}
fn broker() -> Broker {
    let uid = rustix::process::getuid().as_raw();
    Broker::new(
        TrustedContext::from_host(std::process::id(), uid, uid, 7, [1; 32], [2; 32]).unwrap(),
    )
    .unwrap()
}
fn udp(port: u16) -> UdpSocket {
    use rustix::net::{AddressFamily, SocketFlags, SocketType, socket_with};
    let fd = socket_with(
        AddressFamily::INET,
        SocketType::DGRAM,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    rustix::net::sockopt::set_socket_reuseport(&fd, true).unwrap();
    rustix::net::bind(&fd, &SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).unwrap();
    UdpSocket::from(fd)
}
fn flow(socket: &UdpSocket) -> Flow {
    let mut local = [0; 16];
    local[..4].copy_from_slice(&[127, 0, 0, 1]);
    Flow {
        family: 2,
        protocol: 17,
        local_port: socket.local_addr().unwrap().port(),
        remote_port: socket.peer_addr().unwrap().port(),
        local,
        remote: local,
        cookie: None,
    }
}
#[test]
fn task_private_fd_table_is_covered() {
    if !enabled() {
        return;
    }
    let script = std::env::var("YONIK_BROKER_THREAD_FIXTURE").unwrap();
    let mut child = Fixture(
        Command::new("python")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(child.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let record: serde_json::Value = serde_json::from_str(&line).unwrap();
    let mut local = [0; 16];
    local[..4].copy_from_slice(&[127, 0, 0, 1]);
    let f = Flow {
        family: 2,
        protocol: 17,
        local_port: record["port"].as_u64().unwrap() as u16,
        remote_port: 45678,
        local,
        remote: local,
        cookie: None,
    };
    let mut broker = broker();
    let result = broker.lookup(f);
    assert!(
        matches!(result,Lookup::KnownOwner(ref s) if s.pid==child.0.id()),
        "private task FD пропущен: {result:?}"
    );
    child.0.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(child.0.wait().unwrap().success());
}
#[test]
fn tuple_only_cache_rechecks_reuseport_uniqueness() {
    if !enabled() {
        return;
    }
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    let first = udp(0);
    first.connect(peer.local_addr().unwrap()).unwrap();
    let f = flow(&first);
    let mut broker = broker();
    assert!(matches!(broker.lookup(f), Lookup::KnownOwner(_)));
    let second = udp(first.local_addr().unwrap().port());
    second.connect(peer.local_addr().unwrap()).unwrap();
    let before = Instant::now();
    let hit = broker.lookup(f);
    assert!(
        matches!(hit, Lookup::MechanismUnavailable(_)),
        "tuple-only cache скрыл reuseport: {hit:?}"
    );
    std::thread::sleep(Duration::from_millis(21));
    let cold = broker.lookup(f);
    assert!(
        matches!(cold, Lookup::MechanismUnavailable(_)),
        "cold/hit расходятся: {cold:?}"
    );
    eprintln!("reuseport detection {:?}", before.elapsed());
}

#[test]
fn fd_cap_and_cancel_teardown_are_bounded() {
    if !enabled() {
        return;
    }
    use yonik_linux::attribution::broker::Unavailable;
    let fds = || std::fs::read_dir("/proc/self/fd").unwrap().count();
    let baseline = fds();
    {
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        let socket = udp(0);
        socket.connect(peer.local_addr().unwrap()).unwrap();
        let mut b = broker();
        let opened: Vec<_> = (0..260)
            .map(|_| std::fs::File::open("/dev/null").unwrap())
            .collect();
        assert!(matches!(
            b.lookup(flow(&socket)),
            Lookup::MechanismUnavailable(Unavailable::Limit)
        ));
        drop(opened);
        drop(b);
        let mut b = broker();
        assert!(matches!(b.lookup(flow(&socket)), Lookup::KnownOwner(_)));
        let cancel = b.cancellation();
        let started = Instant::now();
        std::thread::spawn(move || cancel.cancel()).join().unwrap();
        assert!(matches!(
            b.lookup(flow(&socket)),
            Lookup::MechanismUnavailable(Unavailable::Cancelled)
        ));
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_secs(1));
        drop(b);
        eprintln!("cancellation+broker drop {:?}", started.elapsed());
    }
    assert_eq!(fds(), baseline, "cache/context/output FD teardown");
}

#[test]
fn actual_exec_replacement_never_returns_cached_executable() {
    if !enabled() {
        return;
    }
    use std::os::unix::fs::MetadataExt;
    let script = std::env::var("YONIK_BROKER_EXEC_FIXTURE").unwrap();
    let mut child = Fixture(
        Command::new("python")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(child.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let record: serde_json::Value = serde_json::from_str(&line).unwrap();
    let mut local = [0; 16];
    local[..4].copy_from_slice(&[127, 0, 0, 1]);
    let f = Flow {
        family: 2,
        protocol: 17,
        local_port: record["port"].as_u64().unwrap() as u16,
        remote_port: 45679,
        local,
        remote: local,
        cookie: None,
    };
    let mut b = broker();
    let Lookup::KnownOwner(old) = b.lookup(f) else {
        panic!("initial snapshot")
    };
    child.0.stdin.take().unwrap().write_all(b"x").unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let current = std::fs::metadata(format!("/proc/{}/exe", child.0.id())).unwrap();
        if (current.dev(), current.ino()) != old.executable {
            break;
        }
        assert!(Instant::now() < deadline, "exec deadline");
        std::thread::yield_now();
    }
    assert!(child.0.try_wait().unwrap().is_none());
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.0.id())).unwrap();
    let start = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert_eq!(start, old.start_time, "exec должен сохранить starttime");
    if let Lookup::KnownOwner(new) = b.lookup(f) {
        assert_ne!(new.executable, old.executable, "stale cached executable");
        assert_eq!(new.pid, old.pid);
        assert_eq!(new.start_time, old.start_time);
    }
    let held = old.executable_fd.metadata().unwrap();
    assert_eq!(
        (held.dev(), held.ino()),
        old.executable,
        "opened exe witness должен остаться прежним"
    );
}

#[test]
fn inaccessible_holder_is_unavailable_not_unknown_or_known() {
    if !enabled() {
        return;
    }
    use yonik_linux::attribution::broker::Unavailable;
    let script = std::env::var("YONIK_BROKER_THREAD_FIXTURE").unwrap();
    let mut child = Fixture(
        Command::new("python")
            .arg(script)
            .env("YONIK_BROKER_NON_DUMPABLE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(child.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let record: serde_json::Value = serde_json::from_str(&line).unwrap();
    let mut local = [0; 16];
    local[..4].copy_from_slice(&[127, 0, 0, 1]);
    let f = Flow {
        family: 2,
        protocol: 17,
        local_port: record["port"].as_u64().unwrap() as u16,
        remote_port: 45678,
        local,
        remote: local,
        cookie: None,
    };
    assert!(matches!(
        broker().lookup(f),
        Lookup::MechanismUnavailable(Unavailable::Coverage)
    ));
    child.0.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(child.0.wait().unwrap().success());
}

#[test]
fn eof_revokes_cache_and_drop_latches_cancellation() {
    if !enabled() {
        return;
    }
    use rustix::net::{AddressFamily, SocketFlags, SocketType, socketpair};
    use yonik_linux::attribution::broker::Unavailable;
    let fds = || std::fs::read_dir("/proc/self/fd").unwrap().count();
    let baseline = fds();
    {
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        let socket = udp(0);
        socket.connect(peer.local_addr().unwrap()).unwrap();
        let mut b = broker();
        assert!(matches!(b.lookup(flow(&socket)), Lookup::KnownOwner(_)));
        let token = b.cancellation();
        let (rx, tx) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        rustix::net::sockopt::set_socket_passcred(&rx, true).unwrap();
        drop(tx);
        assert_eq!(b.receive(&rx), Err(Unavailable::Disconnected));
        assert!(token.is_cancelled());
        assert!(matches!(
            b.lookup(flow(&socket)),
            Lookup::MechanismUnavailable(Unavailable::Revoked)
        ));
        drop(b);
        drop(rx);
        let b = broker();
        let token = b.cancellation();
        assert!(!token.is_cancelled());
        drop(b);
        assert!(token.is_cancelled());
    }
    assert_eq!(fds(), baseline);
}
