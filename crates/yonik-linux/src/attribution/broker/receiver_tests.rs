use super::{FRAME_BYTES, Process, TrustedContext, Unavailable, receive_frame};
use rustix::net::{AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType, socketpair};
use std::{
    os::{fd::OwnedFd, unix::process::CommandExt},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Fixture(Child);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn pair() -> (OwnedFd, OwnedFd) {
    let (a, b) = socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    rustix::net::sockopt::set_socket_passcred(&a, true).unwrap();
    (a, b)
}
fn receive(fd: &OwnedFd) -> rustix::net::UCred {
    let end = Instant::now() + Duration::from_secs(2);
    loop {
        match receive_frame(fd) {
            Ok((_, c)) => return c,
            Err(Unavailable::Timeout) if Instant::now() < end => {
                std::thread::sleep(Duration::from_millis(1))
            }
            other => panic!("receiver: {other:?}"),
        }
    }
}
fn valid_frame() -> [u8; FRAME_BYTES] {
    let mut b = [0; FRAME_BYTES];
    b[..4].copy_from_slice(b"YOB1");
    b[4] = 1;
    b[5] = 2;
    b[6] = 6;
    b[8..16].copy_from_slice(&7u64.to_be_bytes());
    b[16..48].fill(1);
    b[48..80].fill(2);
    b
}
#[test]
fn uid_sender() {
    if std::env::var_os("YONIK_RECEIVER_UID_SENDER").is_none() {
        return;
    }
    let mut trigger = [0; 1];
    rustix::net::recv(std::io::stdin(), &mut trigger, RecvFlags::empty()).unwrap();
    rustix::net::send(std::io::stdin(), &valid_frame(), SendFlags::empty()).unwrap();
    std::thread::sleep(Duration::from_secs(2));
}
#[test]
fn foreign_context_probe() {
    let Ok(worker) = std::env::var("YONIK_RECEIVER_FOREIGN_PID") else {
        return;
    };
    assert_eq!(rustix::process::getuid().as_raw(), 1000);
    let result =
        TrustedContext::from_host(worker.parse().unwrap(), 1001, 1000, 7, [1; 32], [2; 32]);
    assert!(
        matches!(result, Err(Unavailable::WorkerViewUnavailable)),
        "неожиданный admission result"
    );
    eprintln!(
        "typed negative WorkerViewUnavailable: app UID1000 не читает ns worker UID1001; CAP_SYS_PTRACE не выдаётся"
    );
}
#[test]
fn actual_foreign_uid_credentials_and_admission() {
    if std::env::var_os("YONIK_BROKER_ROOTMARKED").is_none() {
        return;
    }
    assert_eq!(rustix::process::getuid().as_raw(), 0);
    let map = std::fs::read_to_string("/proc/self/uid_map").unwrap();
    assert_ne!(
        map.split_whitespace().nth(1),
        Some("0"),
        "запрещено на host root"
    );
    let (rx, tx) = pair();
    let mut child = Fixture(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "attribution::broker::receiver_tests::uid_sender",
                "--nocapture",
            ])
            .uid(1001)
            .gid(1001)
            .env("YONIK_RECEIVER_UID_SENDER", "1")
            .stdin(Stdio::from(tx))
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let expected = Process::capture(child.0.id(), 1001).unwrap();
    rustix::net::send(&rx, b"x", SendFlags::empty()).unwrap();
    let c = receive(&rx);
    assert_eq!(c.pid.as_raw_pid() as u32, child.0.id());
    assert_eq!(c.uid.as_raw(), 1001);
    assert_eq!(expected.accept_credentials(c), Ok(()));
    // Тот же child PID, но другой реальный UID после host capture: kernel credentials обязаны дать отказ.
    let (rx2, tx2) = pair();
    use std::fmt::Write;
    let mut payload = String::with_capacity(FRAME_BYTES * 2);
    for byte in valid_frame() {
        write!(&mut payload, "{byte:02x}").unwrap();
    }
    let script = format!(
        "import os,socket,time\ns=socket.socket(fileno=0)\ns.recv(1)\nos.setgid(1001)\nos.setuid(1001)\ns.send(bytes.fromhex('{payload}'))\ntime.sleep(2)\n"
    );
    let mut changed = Fixture(
        Command::new("python")
            .args(["-c", &script])
            .stdin(Stdio::from(tx2))
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let original = Process::capture(changed.0.id(), 0).unwrap();
    rustix::net::send(&rx2, b"x", SendFlags::empty()).unwrap();
    let c = receive(&rx2);
    assert_eq!(c.pid.as_raw_pid() as u32, changed.0.id());
    assert_eq!(c.uid.as_raw(), 1001);
    assert_eq!(original.accept_credentials(c), Err(Unavailable::Authority));
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "attribution::broker::receiver_tests::foreign_context_probe",
            "--nocapture",
        ])
        .uid(1000)
        .gid(1000)
        .env("YONIK_RECEIVER_FOREIGN_PID", child.0.id().to_string())
        .output()
        .unwrap();
    eprintln!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success());
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    changed.0.kill().unwrap();
    changed.0.wait().unwrap();
}

#[test]
fn rejected_rights_and_truncation_release_every_received_fd() {
    use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, sendmsg};
    use std::{io::IoSlice, mem::MaybeUninit, os::fd::AsFd};
    let (rx, tx) = pair();
    let file = std::fs::File::open("/dev/null").unwrap();
    let fds = || std::fs::read_dir("/proc/self/fd").unwrap().count();
    let baseline = fds();
    for _ in 0..256 {
        let rights = [file.as_fd(); 4];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(4))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        assert!(ancillary.push(SendAncillaryMessage::ScmRights(&rights)));
        sendmsg(
            &tx,
            &[IoSlice::new(&[0; FRAME_BYTES])],
            &mut ancillary,
            SendFlags::empty(),
        )
        .unwrap();
        assert!(matches!(receive_frame(&rx), Err(Unavailable::Malformed)));
        assert_eq!(fds(), baseline, "SCM_RIGHTS leak");
    }
}

#[test]
fn oversized_kernel_queue_is_rejected() {
    let (rx, _tx) = pair();
    rustix::net::sockopt::set_socket_recv_buffer_size(&rx, 4 * 1024 * 1024).unwrap();
    let actual = rustix::net::sockopt::socket_recv_buffer_size(&rx).unwrap();
    assert!(
        actual > super::MAX_SOCKET_BUFFER,
        "fixture требует kernel rmem_max выше минимального buffer"
    );
    assert!(matches!(receive_frame(&rx), Err(Unavailable::Limit)));
}
