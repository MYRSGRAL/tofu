//! Fixture-only issuer: живой Session и Journal owner, без production admission.
use super::{BoundTun, Kind, TunRole, Unavailable};
use rustix::net::{self, SocketFlags, SocketType};

struct Consumer(std::process::Child);
impl Drop for Consumer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
use std::{
    io::IoSlice,
    os::fd::AsFd,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) fn run(bound: Arc<BoundTun>, socket: &Path, binary: &Path, mode: &str) {
    let baseline = std::fs::read_dir("/proc/self/fd").unwrap().count();
    let owners = Arc::strong_count(&bound);
    let listener = net::socket_with(
        net::AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    )
    .unwrap();
    net::bind(&listener, &net::SocketAddrUnix::new(socket).unwrap()).unwrap();
    net::listen(&listener, 1).unwrap();
    let mut child = Consumer(
        std::process::Command::new(binary)
            .args(["-test.run=^TestN4Consumer$", "-test.v", "-test.timeout=15s"])
            .env("YONIK_N4_SOCKET", socket)
            .env("YONIK_N4_MODE", mode)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let control = loop {
        match net::accept_with(&listener, SocketFlags::CLOEXEC | SocketFlags::NONBLOCK) {
            Ok(fd) => break fd,
            Err(rustix::io::Errno::AGAIN) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1))
            }
            other => {
                child.0.kill().ok();
                child.0.wait().unwrap();
                panic!("fixture accept: {other:?}");
            }
        }
    };
    let (worker, peer) = net::socketpair(
        net::AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let (mut session, unused) = super::tests::fixture();
    drop(unused);
    session.roles.as_mut().unwrap().tun = TunRole::Bound(bound.clone());
    session.roles.as_mut().unwrap().worker = worker;
    let pidfd = rustix::process::pidfd_open(
        rustix::process::getpid(),
        rustix::process::PidfdFlags::NONBLOCK,
    )
    .unwrap();
    let mut record = [0u8; 192];
    record[..8].copy_from_slice(b"YNKROLE1");
    record[8..40].copy_from_slice(&session.nonce);
    record[40..72].copy_from_slice(&session.digest);
    record[72..80].copy_from_slice(&session.epoch.to_be_bytes());
    record[80..96].copy_from_slice(&session.session);
    record[96..104].copy_from_slice(&session.generation.to_be_bytes());
    record[104..120].copy_from_slice(&session.invocation);
    record[120..124].copy_from_slice(
        &rustix::process::getpid()
            .as_raw_nonzero()
            .get()
            .to_be_bytes(),
    );
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let start: u64 = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .parse()
        .unwrap();
    record[128..136].copy_from_slice(&start.to_be_bytes());
    record[144..146].copy_from_slice(&2u16.to_be_bytes());
    record[146..148].copy_from_slice(&1u16.to_be_bytes());
    record[148..150].copy_from_slice(&2u16.to_be_bytes());
    let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
    let mut ancillary = net::SendAncillaryBuffer::new(&mut space);
    let rights = [peer.as_fd(), pidfd.as_fd()];
    assert!(ancillary.push(net::SendAncillaryMessage::ScmRights(&rights)));
    assert_eq!(
        net::sendmsg(
            &control,
            &[IoSlice::new(&record)],
            &mut ancillary,
            net::SendFlags::NOSIGNAL
        )
        .unwrap(),
        192
    );
    drop(peer);
    drop(pidfd);
    let read = |deadline: Instant| -> Option<u8> {
        let mut byte = [0];
        loop {
            match net::recv(&control, &mut byte, net::RecvFlags::DONTWAIT) {
                Ok((1, _)) => return Some(byte[0]),
                Ok(_) => return None,
                Err(rustix::io::Errno::AGAIN) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                _ => return None,
            }
        }
    };
    assert_eq!(read(deadline), Some(b'R'));
    let revoke_socket =
        rustix::io::fcntl_dupfd_cloexec(&session.roles.as_ref().unwrap().worker, 0).unwrap();
    let mut revoke_socket = Some(revoke_socket);
    let mut revoke = session.frame(Kind::Revoke).unwrap();
    revoke[96..100].copy_from_slice(&1u32.to_be_bytes());
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let thread = std::thread::spawn(move || session.serve(&flag));
    if read(deadline) == Some(b'T') {
        if mode == "revoke" {
            assert_eq!(
                net::send(
                    revoke_socket.as_ref().unwrap(),
                    &revoke,
                    net::SendFlags::NOSIGNAL
                )
                .unwrap(),
                revoke.len()
            );
        } else {
            cancel.store(true, Ordering::Release);
        }
        drop(revoke_socket.take());
    }
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "Go consumer: {status}");
            break;
        }
        if Instant::now() >= deadline {
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            panic!("consumer deadline");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    cancel.store(true, Ordering::Release);
    let result = thread.join().unwrap();
    eprintln!("N4 {mode}: confirmed consumer exit; serve={result:?}");
    assert!(
        matches!(
            result,
            Err(Unavailable::Cancelled | Unavailable::Disconnected)
        ),
        "terminal не должен подменяться bootstrap TTL: {result:?}"
    );
    drop(revoke_socket);
    drop(control);
    drop(listener);
    std::fs::remove_file(socket).unwrap();
    assert_eq!(Arc::strong_count(&bound), owners);
    assert!(bound.attach_metadata().is_ok());
    assert_eq!(
        std::fs::read_dir("/proc/self/fd").unwrap().count(),
        baseline
    );
}
