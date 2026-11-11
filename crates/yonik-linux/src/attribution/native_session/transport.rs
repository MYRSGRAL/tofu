//! Ограниченный последовательный transport; ошибки всегда отзывают custody.
use super::{Header, Kind, Lookup, OwnedFd, Session, Unavailable, wire};
use rustix::net::{
    self, RecvAncillaryBuffer, RecvFlags, ReturnFlags, SendAncillaryBuffer, SendAncillaryMessage,
    SendFlags,
};
use std::{
    io::{IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::fd::AsFd,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const TICK: Duration = Duration::from_millis(1);

fn fence(deadline: Instant, cancel: &AtomicBool) -> Result<(), Unavailable> {
    if cancel.load(Ordering::Acquire) {
        return Err(Unavailable::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(Unavailable::Timeout);
    }
    Ok(())
}
fn receive(
    socket: &OwnedFd,
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, Unavailable> {
    loop {
        fence(deadline, cancel)?;
        let mut bytes = [0; wire::MAX_FRAME + 1];
        let mut space =
            [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1), ScmCredentials(1))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut space);
        match net::recvmsg(
            socket,
            &mut [IoSliceMut::new(&mut bytes)],
            &mut ancillary,
            RecvFlags::DONTWAIT | RecvFlags::CMSG_CLOEXEC,
        ) {
            Ok(msg) => {
                // drain владеет полученными FD и закрывает их даже при отказе.
                let unexpected = ancillary.drain().count() != 0;
                if unexpected
                    || msg
                        .flags
                        .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
                    || msg.bytes > wire::MAX_FRAME
                {
                    return Err(Unavailable::Malformed);
                }
                if msg.bytes == 0 {
                    return Err(Unavailable::Disconnected);
                }
                fence(deadline, cancel)?;
                return Ok(bytes[..msg.bytes].to_vec());
            }
            Err(rustix::io::Errno::AGAIN) => std::thread::sleep(TICK),
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => return Err(Unavailable::Disconnected),
        }
    }
}
fn send(
    socket: &OwnedFd,
    bytes: &[u8],
    fd: Option<&OwnedFd>,
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<(), Unavailable> {
    loop {
        fence(deadline, cancel)?;
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        let rights = fd.map(|f| [f.as_fd()]);
        if let Some(rights) = &rights {
            if !ancillary.push(SendAncillaryMessage::ScmRights(rights)) {
                return Err(Unavailable::Limit);
            }
        }
        match net::sendmsg(
            socket,
            &[IoSlice::new(bytes)],
            &mut ancillary,
            SendFlags::DONTWAIT | SendFlags::NOSIGNAL,
        ) {
            Ok(n) if n == bytes.len() => return fence(deadline, cancel),
            Err(rustix::io::Errno::AGAIN) => std::thread::sleep(TICK),
            Err(rustix::io::Errno::INTR) => continue,
            _ => return Err(Unavailable::Disconnected),
        }
    }
}
impl Session {
    /// Потребляет все роли. metadata — только от helper resource owner.
    /// cancel не является authority и может только отозвать существующую сессию.
    pub fn serve(mut self, metadata: &[u8; 32], cancel: &AtomicBool) -> Result<(), Unavailable> {
        let result = self.serve_inner(metadata, cancel);
        self.close();
        result
    }
    fn serve_inner(&mut self, metadata: &[u8; 32], cancel: &AtomicBool) -> Result<(), Unavailable> {
        let socket = rustix::io::fcntl_dupfd_cloexec(
            &self.roles.as_ref().ok_or(Unavailable::Authority)?.worker,
            0,
        )
        .map_err(|_| Unavailable::Disconnected)?;
        if net::sockopt::socket_type(&socket).map_err(|_| Unavailable::Malformed)?
            != net::SocketType::SEQPACKET
        {
            return Err(Unavailable::Malformed);
        }
        let expires = Instant::now() + Duration::from_nanos(wire::BOOTSTRAP_NS);
        let hello = self.hello().map_err(|_| Unavailable::Malformed)?;
        send(&socket, &hello, None, expires, cancel)?;
        let features = receive(&socket, expires, cancel)?;
        let (attach, fd) = self
            .attach(&features, metadata)
            .map_err(|_| Unavailable::Malformed)?;
        send(&socket, &attach, Some(&fd), expires, cancel)?;
        drop(fd);
        let ack = receive(&socket, expires, cancel)?;
        self.acknowledge(&ack).map_err(|_| Unavailable::Malformed)?;
        loop {
            let bytes = receive(&socket, expires, cancel)?;
            let header = self.header(&bytes).map_err(|_| Unavailable::Malformed)?;
            if header.kind == Kind::Revoke {
                self.revoke(&bytes).map_err(|_| Unavailable::Malformed)?;
                return Ok(());
            }
            let deadline = (Instant::now() + Duration::from_nanos(wire::OWNER_TTL_NS)).min(expires);
            let lookup = self.lookup(&bytes).map_err(|_| Unavailable::Malformed)?;
            fence(deadline, cancel)?;
            let reply = self.reply(
                header.request_id,
                lookup,
                wire::lookup_flow(&bytes).map_err(|_| Unavailable::Malformed)?,
            )?;
            send(&socket, &reply, None, deadline, cancel)?;
        }
    }
    fn reply(
        &self,
        id: u64,
        lookup: Lookup,
        flow: super::super::broker::Flow,
    ) -> Result<Vec<u8>, Unavailable> {
        let mut bytes = vec![0; 160];
        Header {
            kind: Kind::Reply,
            request_id: id,
            epoch: self.epoch,
            digest: self.digest,
        }
        .write(&mut bytes)
        .map_err(|_| Unavailable::Malformed)?;
        bytes[64..96].copy_from_slice(&self.nonce);
        let (status, reason) = match lookup {
            Lookup::UnknownFlow => (2, 0),
            Lookup::KnownOwner(snapshot) => {
                let projection = self
                    .roles
                    .as_ref()
                    .and_then(|roles| roles.broker.as_ref())
                    .ok_or(Unavailable::Authority)?
                    .project(flow, &snapshot, self.epoch);
                match projection {
                    Ok((owner, path, ttl)) => {
                        bytes.resize(160 + path.len(), 0);
                        Header {
                            kind: Kind::Reply,
                            request_id: id,
                            epoch: self.epoch,
                            digest: self.digest,
                        }
                        .write(&mut bytes)
                        .map_err(|_| Unavailable::Malformed)?;
                        bytes[100..102].copy_from_slice(&(path.len() as u16).to_be_bytes());
                        bytes[104..112].copy_from_slice(&(ttl.as_nanos() as u64).to_be_bytes());
                        bytes[112..120].copy_from_slice(&owner.cookie.to_be_bytes());
                        bytes[120..124].copy_from_slice(&owner.pid.to_be_bytes());
                        bytes[128..136].copy_from_slice(&owner.start_time.to_be_bytes());
                        bytes[136..144].copy_from_slice(&owner.executable.0.to_be_bytes());
                        bytes[144..152].copy_from_slice(&owner.executable.1.to_be_bytes());
                        bytes[160..].copy_from_slice(path.as_bytes());
                        (1, 0)
                    }
                    Err(reason) => {
                        return self.reply(id, Lookup::MechanismUnavailable(reason), flow);
                    }
                }
            }
            Lookup::MechanismUnavailable(reason) => (
                3,
                match reason {
                    Unavailable::Authority => 1,
                    Unavailable::Revoked | Unavailable::Cancelled | Unavailable::Disconnected => 2,
                    Unavailable::Timeout => 3,
                    Unavailable::Coverage | Unavailable::WorkerViewUnavailable => 4,
                    Unavailable::Limit | Unavailable::ReceiveOverflow => 5,
                    Unavailable::Malformed => 6,
                    Unavailable::AmbiguousSocket => 7,
                },
            ),
        };
        bytes[96] = status;
        bytes[97] = reason;
        Header::decode(&bytes, 0).map_err(|_| Unavailable::Malformed)?;
        Ok(bytes)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    fn fixture() -> (Session, OwnedFd, [u8; 32]) {
        let (mut session, _) = super::super::tests::fixture();
        let (worker, peer) = net::socketpair(
            net::AddressFamily::UNIX,
            net::SocketType::SEQPACKET,
            net::SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        session.roles.as_mut().unwrap().worker = worker;
        let mut metadata = [0; 32];
        metadata[..4].copy_from_slice(&1u32.to_be_bytes());
        metadata[4..8].copy_from_slice(&1u32.to_be_bytes());
        metadata[8..12].copy_from_slice(&0x1001u32.to_be_bytes());
        metadata[12..16].copy_from_slice(&1500u32.to_be_bytes());
        metadata[16..20].copy_from_slice(b"tun0");
        (session, peer, metadata)
    }
    fn handshake(peer: &OwnedFd, acknowledge: bool) -> Vec<u8> {
        let cancel = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(2);
        let hello = receive(peer, deadline, &cancel).unwrap();
        let mut features = hello[..112].to_vec();
        Header {
            kind: Kind::Features,
            request_id: 0,
            epoch: 1,
            digest: [2; 32],
        }
        .write(&mut features)
        .unwrap();
        features[96..104].copy_from_slice(&wire::FEATURES.to_be_bytes());
        features[104..112].copy_from_slice(&wire::OWNED_TUN.to_be_bytes());
        send(peer, &features, None, deadline, &cancel).unwrap();
        let mut bytes = [0; 128];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        loop {
            let mut ancillary = RecvAncillaryBuffer::new(&mut space);
            match net::recvmsg(
                peer,
                &mut [IoSliceMut::new(&mut bytes)],
                &mut ancillary,
                RecvFlags::CMSG_CLOEXEC | RecvFlags::DONTWAIT,
            ) {
                Ok(msg) => {
                    assert_eq!(msg.bytes, 128);
                    let fds: Vec<_> = ancillary
                        .drain()
                        .flat_map(|c| match c {
                            net::RecvAncillaryMessage::ScmRights(r) => r.collect::<Vec<_>>(),
                            _ => panic!("лишний ancillary"),
                        })
                        .collect();
                    assert_eq!(fds.len(), 1);
                    assert!(
                        rustix::io::fcntl_getfd(&fds[0])
                            .unwrap()
                            .contains(rustix::io::FdFlags::CLOEXEC)
                    );
                    assert_eq!(Header::decode(&bytes, 1).unwrap().kind, Kind::Attach);
                    break;
                }
                Err(rustix::io::Errno::AGAIN) => {
                    fence(deadline, &cancel).unwrap();
                    std::thread::sleep(TICK);
                }
                e => panic!("{e:?}"),
            }
        }
        let mut ack = hello[..104].to_vec();
        Header {
            kind: Kind::AttachAck,
            request_id: 0,
            epoch: 1,
            digest: [2; 32],
        }
        .write(&mut ack)
        .unwrap();
        ack[96..104].fill(0);
        ack[99] = 1;
        if acknowledge {
            send(peer, &ack, None, deadline, &cancel).unwrap();
        }
        hello
    }
    pub(in crate::attribution) fn known_roundtrip(
        broker: super::super::super::broker::Broker,
        flow: super::super::super::broker::Flow,
    ) {
        let (mut session, peer, metadata) = fixture();
        session.roles.as_mut().unwrap().broker = Some(broker);
        let thread = std::thread::spawn(move || session.serve(&metadata, &AtomicBool::new(false)));
        let hello = handshake(&peer, true);
        let mut lookup = vec![0; 144];
        Header {
            kind: Kind::Lookup,
            request_id: 43,
            epoch: 1,
            digest: [2; 32],
        }
        .write(&mut lookup)
        .unwrap();
        lookup[64..96].copy_from_slice(&hello[64..96]);
        lookup[96] = if flow.family == 2 { 4 } else { 6 };
        lookup[97] = flow.protocol;
        lookup[100..102].copy_from_slice(&flow.local_port.to_be_bytes());
        lookup[102..104].copy_from_slice(&flow.remote_port.to_be_bytes());
        lookup[104..120].copy_from_slice(&flow.local);
        lookup[120..136].copy_from_slice(&flow.remote);
        let deadline = Instant::now() + Duration::from_secs(1);
        let cancel = AtomicBool::new(false);
        send(&peer, &lookup, None, deadline, &cancel).unwrap();
        let reply = receive(&peer, deadline, &cancel).unwrap();
        assert_eq!(Header::decode(&reply, 0).unwrap().request_id, 43);
        assert_eq!(&reply[96..98], &[1, 0]);
        assert_eq!(
            &reply[160..],
            std::fs::read_link("/proc/self/exe")
                .unwrap()
                .to_str()
                .unwrap()
                .as_bytes()
        );
        drop(peer);
        assert_eq!(thread.join().unwrap(), Err(Unavailable::Disconnected));
    }
    #[test]
    fn socket_handshake_reply_correlation_and_replay_retire() {
        let (session, peer, metadata) = fixture();
        let thread = std::thread::spawn(move || session.serve(&metadata, &AtomicBool::new(false)));
        let hello = handshake(&peer, true);
        let mut lookup = vec![0; 144];
        Header {
            kind: Kind::Lookup,
            request_id: 42,
            epoch: 1,
            digest: [2; 32],
        }
        .write(&mut lookup)
        .unwrap();
        lookup[64..96].copy_from_slice(&hello[64..96]);
        lookup[96] = 4;
        lookup[97] = 6;
        lookup[101] = 1;
        lookup[103] = 1;
        let deadline = Instant::now() + Duration::from_secs(1);
        let cancel = AtomicBool::new(false);
        send(&peer, &lookup, None, deadline, &cancel).unwrap();
        let reply = receive(&peer, deadline, &cancel).unwrap();
        assert_eq!(Header::decode(&reply, 0).unwrap().request_id, 42);
        assert_eq!(&reply[96..98], &[3, 1]);
        send(&peer, &lookup, None, deadline, &cancel).unwrap();
        assert_eq!(thread.join().unwrap(), Err(Unavailable::Malformed));
        assert_eq!(
            receive(&peer, deadline, &cancel),
            Err(Unavailable::Disconnected)
        );
    }
    #[test]
    fn socket_negative_controls_and_cleanup() {
        for mode in 0..5 {
            let (session, peer, metadata) = fixture();
            let cancel = std::sync::Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let thread = std::thread::spawn(move || session.serve(&metadata, &flag));
            if mode == 0 {
                drop(peer);
                assert_eq!(thread.join().unwrap(), Err(Unavailable::Disconnected));
                continue;
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            receive(&peer, deadline, &AtomicBool::new(false)).unwrap();
            if mode == 1 {
                cancel.store(true, Ordering::Release);
            }
            if mode >= 2 {
                let data = if mode == 2 {
                    vec![0; wire::MAX_FRAME + 20]
                } else {
                    vec![0; 112]
                };
                let file = std::fs::File::open("/dev/null").unwrap();
                let fd: OwnedFd = file.into();
                send(
                    &peer,
                    &data,
                    if mode == 3 { Some(&fd) } else { None },
                    deadline,
                    &AtomicBool::new(false),
                )
                .unwrap();
            }
            let expected = if mode == 1 {
                Unavailable::Cancelled
            } else {
                Unavailable::Malformed
            };
            assert_eq!(thread.join().unwrap(), Err(expected));
            assert_eq!(
                receive(&peer, deadline, &AtomicBool::new(false)),
                Err(Unavailable::Disconnected)
            );
        }
    }
    #[test]
    fn expired_send_backpressure_and_wrong_socket_fail_closed() {
        let (session, _peer) = super::super::tests::fixture();
        assert_eq!(
            session.serve(&[0; 32], &AtomicBool::new(false)),
            Err(Unavailable::Malformed)
        );
        let (session, peer, _) = fixture();
        let worker = &session.roles.as_ref().unwrap().worker;
        let cancel = AtomicBool::new(false);
        assert_eq!(
            send(worker, &[0], None, Instant::now(), &cancel),
            Err(Unavailable::Timeout)
        );
        loop {
            match net::send(worker, &[0; 768], SendFlags::DONTWAIT | SendFlags::NOSIGNAL) {
                Ok(_) => (),
                Err(rustix::io::Errno::AGAIN) => break,
                e => panic!("{e:?}"),
            }
        }
        assert_eq!(
            send(
                worker,
                &[0; 768],
                None,
                Instant::now() + Duration::from_millis(5),
                &cancel
            ),
            Err(Unavailable::Timeout)
        );
        drop(peer);
    }
    #[test]
    fn foreign_epoch_and_wrong_phase_rejected_over_socket() {
        for kind in [Kind::Lookup, Kind::AttachAck] {
            let (session, peer, metadata) = fixture();
            let thread =
                std::thread::spawn(move || session.serve(&metadata, &AtomicBool::new(false)));
            let hello = handshake(&peer, true);
            let mut bytes = vec![0; kind.fixed_bytes().unwrap()];
            Header {
                kind,
                request_id: if kind == Kind::Lookup { 1 } else { 0 },
                epoch: 2,
                digest: [2; 32],
            }
            .write(&mut bytes)
            .unwrap();
            bytes[64..96].copy_from_slice(&hello[64..96]);
            send(
                &peer,
                &bytes,
                None,
                Instant::now() + Duration::from_secs(1),
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(thread.join().unwrap(), Err(Unavailable::Malformed));
        }
    }
    #[test]
    fn socket_ack_timeout_and_revoke() {
        let (session, peer, metadata) = fixture();
        let thread = std::thread::spawn(move || session.serve(&metadata, &AtomicBool::new(false)));
        handshake(&peer, false);
        // ATTACH отправлен; отсутствие ACK завершает роли по общему deadline.
        assert_eq!(thread.join().unwrap(), Err(Unavailable::Timeout));
        drop(peer);
        let (session, peer, metadata) = fixture();
        let thread = std::thread::spawn(move || session.serve(&metadata, &AtomicBool::new(false)));
        let hello = handshake(&peer, true);
        let mut revoke = hello[..104].to_vec();
        Header {
            kind: Kind::Revoke,
            request_id: 0,
            epoch: 1,
            digest: [2; 32],
        }
        .write(&mut revoke)
        .unwrap();
        revoke[96..104].fill(0);
        revoke[99] = 1;
        send(
            &peer,
            &revoke,
            None,
            Instant::now() + Duration::from_secs(1),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(thread.join().unwrap(), Ok(()));
    }
}
