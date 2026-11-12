//! Private-process receiver controls; не запускают helper/native dataplane.
use rustix::net::{AddressFamily, SendFlags, SocketFlags, SocketType, socketpair};
use std::{
    os::fd::OwnedFd,
    process::{Child, Command, Stdio},
    time::Duration,
};
use yonik_linux::attribution::broker::{
    Broker, FRAME_BYTES, Flow, Lookup, TrustedContext, Unavailable,
};

struct Sender(Child);
impl Drop for Sender {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn frame() -> [u8; FRAME_BYTES] {
    let mut b = [0; FRAME_BYTES];
    b[..4].copy_from_slice(b"YOB1");
    b[4] = 1;
    b[5] = 2;
    b[6] = 6;
    b[8..16].copy_from_slice(&7u64.to_be_bytes());
    b[16..48].fill(1);
    b[48..80].fill(2);
    b[80..82].copy_from_slice(&1234u16.to_be_bytes());
    b[82..84].copy_from_slice(&443u16.to_be_bytes());
    b[84..88].copy_from_slice(&[127, 0, 0, 1]);
    b[100..104].copy_from_slice(&[127, 0, 0, 1]);
    b
}
fn pair() -> (OwnedFd, OwnedFd) {
    socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap()
}
fn assert_sender(socket: &OwnedFd, pid: u32, uid: u32) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let mut bytes = [0; FRAME_BYTES];
        let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmCredentials(1))];
        let mut ancillary = rustix::net::RecvAncillaryBuffer::new(&mut space);
        match rustix::net::recvmsg(
            socket,
            &mut [std::io::IoSliceMut::new(&mut bytes)],
            &mut ancillary,
            rustix::net::RecvFlags::PEEK | rustix::net::RecvFlags::DONTWAIT,
        ) {
            Err(rustix::io::Errno::AGAIN) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(message) => {
                assert_eq!(message.bytes, FRAME_BYTES);
                let Some(rustix::net::RecvAncillaryMessage::ScmCredentials(c)) =
                    ancillary.drain().next()
                else {
                    panic!("нет actual SCM_CREDENTIALS: pid={pid}, uid={uid}");
                };
                eprintln!(
                    "actual credentials={c:?}; ожидается pid={pid}, uid={uid}; socket={:?}, passcred={:?}",
                    rustix::net::sockopt::socket_type(socket),
                    rustix::net::sockopt::socket_passcred(socket)
                );
                assert_eq!(c.pid.as_raw_pid() as u32, pid);
                assert_eq!(c.uid.as_raw(), uid);
                return;
            }
            result => panic!("MSG_PEEK: {result:?}; ожидается pid={pid}, uid={uid}"),
        }
    }
}

#[test]
fn private_sender() {
    if std::env::var_os("YONIK_BROKER_SENDER").is_none() {
        return;
    }
    let mut trigger = [0; 1];
    rustix::net::recv(
        std::io::stdin(),
        &mut trigger,
        rustix::net::RecvFlags::empty(),
    )
    .unwrap();
    assert_eq!(trigger, [1]);
    rustix::net::send(std::io::stdin(), &frame(), SendFlags::empty()).unwrap();
    rustix::net::recv(
        std::io::stdin(),
        &mut trigger,
        rustix::net::RecvFlags::empty(),
    )
    .unwrap();
}
#[test]
fn credentials_epoch_revoke_and_malformed() {
    if std::env::var_os("YONIK_PRIVATE_CLEAN_CAPS").is_none() {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        for line in status
            .lines()
            .filter(|line| line.starts_with("Cap") || line.starts_with("Uid:"))
        {
            eprintln!("исходные credentials: {line}");
        }
        let status = Command::new("setpriv")
            .args(["--inh-caps=-all", "--ambient-caps=-all", "--"])
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "credentials_epoch_revoke_and_malformed",
                "--nocapture",
            ])
            .env("YONIK_PRIVATE_CLEAN_CAPS", "1")
            .status()
            .expect("fixture требует util-linux setpriv");
        assert!(status.success(), "изолированная fixture: {status}");
        return;
    }
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    for field in ["CapInh:", "CapPrm:", "CapEff:", "CapAmb:"] {
        let value = status
            .lines()
            .find_map(|line| line.strip_prefix(field))
            .unwrap();
        assert_eq!(u64::from_str_radix(value.trim(), 16).unwrap(), 0, "{field}");
    }
    let uid = rustix::process::getuid().as_raw();
    assert_ne!(uid, 0, "fixture требует обычного пользователя");
    let (rx, tx) = pair();
    rustix::net::sockopt::set_socket_passcred(&rx, true).unwrap();
    let impostor = tx.try_clone().unwrap();
    let mut child = Sender(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "private_sender", "--nocapture"])
            .env("YONIK_BROKER_SENDER", "1")
            .stdin(Stdio::from(tx))
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let context = TrustedContext::from_host(child.0.id(), uid, uid, 7, [1; 32], [2; 32]).unwrap();
    let mut broker = Broker::new(context).unwrap();
    rustix::net::send(&rx, &[1], SendFlags::empty()).unwrap();
    assert_sender(&rx, child.0.id(), uid);
    // socketpair SO_PEERCRED здесь указывает creator PID; actual sender — child.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let flow = loop {
        assert!(std::time::Instant::now() < deadline, "receiver deadline");
        match broker.receive(&rx) {
            Err(Unavailable::Timeout) => std::thread::sleep(Duration::from_millis(5)),
            result => break result.expect("actual child frame должен пройти admission"),
        }
    };
    assert_eq!(flow.protocol, 6);
    rustix::net::send(&impostor, &frame(), SendFlags::empty()).unwrap();
    assert_sender(&rx, std::process::id(), uid);
    assert_eq!(broker.receive(&rx), Err(Unavailable::Authority));
    broker.revoke();
    assert_eq!(broker.receive(&rx), Err(Unavailable::Revoked));
    assert!(matches!(
        broker.lookup(flow),
        Lookup::MechanismUnavailable(Unavailable::Revoked)
    ));
    let death_context =
        TrustedContext::from_host(child.0.id(), uid, uid, 7, [1; 32], [2; 32]).unwrap();
    let mut death_broker = Broker::new(death_context).unwrap();
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert!(matches!(
        death_broker.receive(&rx),
        Err(Unavailable::Authority | Unavailable::Coverage)
    ));
    let context =
        TrustedContext::from_host(std::process::id(), uid, uid, 7, [1; 32], [2; 32]).unwrap();
    let mut broker = Broker::new(context).unwrap();
    let (rx, tx) = pair();
    rustix::net::sockopt::set_socket_passcred(&rx, true).unwrap();
    assert_eq!(broker.receive(&rx), Err(Unavailable::Timeout));
    for offset in [8, 16, 48, 124, 4, 5, 6, 7] {
        let mut b = frame();
        b[offset] = 255;
        rustix::net::send(&tx, &b, SendFlags::empty()).unwrap();
        assert_eq!(broker.receive(&rx), Err(Unavailable::Malformed));
    }
    rustix::net::send(&tx, &[0; FRAME_BYTES + 1], SendFlags::empty()).unwrap();
    assert_eq!(broker.receive(&rx), Err(Unavailable::Malformed));
    let flow = Flow {
        family: 2,
        protocol: 17,
        local_port: 1,
        remote_port: 0,
        local: [0; 16],
        remote: [0; 16],
        cookie: None,
    };
    assert!(matches!(broker.lookup(flow), Lookup::UnknownFlow));
}

#[test]
fn isolated_snapshot_controls() {
    if std::env::var_os("YONIK_BROKER_ISOLATED").is_none() {
        return;
    }
    use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
    let uid = rustix::process::getuid().as_raw();
    let mut broker = Broker::new(
        TrustedContext::from_host(std::process::id(), uid, uid, 7, [1; 32], [2; 32]).unwrap(),
    )
    .unwrap();
    let count = std::env::var("YONIK_BROKER_SCALE")
        .unwrap_or("1".into())
        .parse::<usize>()
        .unwrap();
    let fds = || std::fs::read_dir("/proc/self/fd").unwrap().count();
    let baseline = fds();
    let to_flow = |local: SocketAddr, remote: SocketAddr, protocol| {
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
            _ => unreachable!(),
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
    };
    for _ in 0..count {
        for address in ["127.0.0.1:0", "[::1]:0"] {
            let listener = TcpListener::bind(address).unwrap();
            let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (_peer, _) = listener.accept().unwrap();
            let udp = UdpSocket::bind(address).unwrap();
            let peer = UdpSocket::bind(address).unwrap();
            udp.connect(peer.local_addr().unwrap()).unwrap();
            for flow in [
                to_flow(client.local_addr().unwrap(), client.peer_addr().unwrap(), 6),
                to_flow(udp.local_addr().unwrap(), udp.peer_addr().unwrap(), 17),
            ] {
                std::thread::sleep(Duration::from_millis(21));
                let result = broker.lookup(flow);
                let Lookup::KnownOwner(snapshot) = result else {
                    panic!("ожидался actual own-UID snapshot: {result:?}")
                };
                assert_eq!(snapshot.pid, std::process::id());
                assert_eq!(snapshot.epoch, 7);
                assert_ne!(snapshot.cookie, 0);
                assert!(matches!(broker.lookup(flow), Lookup::KnownOwner(_)));
                let mut wrong = flow;
                wrong.cookie = Some(snapshot.cookie.wrapping_add(1));
                std::thread::sleep(Duration::from_millis(21));
                assert!(matches!(broker.lookup(wrong), Lookup::UnknownFlow));
            }
        }
    }
    assert_eq!(fds(), baseline, "FD leak");
}
