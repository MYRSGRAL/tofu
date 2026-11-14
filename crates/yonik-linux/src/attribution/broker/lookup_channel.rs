//! Один owned SEQPACKET endpoint; credentials проверяются на КАЖДОМ packet.
use super::Unavailable;
use rustix::net::{
    self, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags,
};
use std::{
    io::{IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::fd::{BorrowedFd, OwnedFd},
    time::Instant,
};

pub(super) struct Packet {
    pub(super) bytes: Vec<u8>,
    pub(super) rights: Vec<OwnedFd>,
    pub(super) credentials: net::UCred,
}
pub(super) struct Channel {
    socket: OwnedFd,
}
impl Channel {
    pub(super) fn new(socket: OwnedFd) -> Result<Self, Unavailable> {
        if net::sockopt::socket_type(&socket).map_err(|_| Unavailable::Authority)?
            != net::SocketType::SEQPACKET
            || rustix::fs::fcntl_getfl(&socket)
                .map_err(|_| Unavailable::Authority)?
                .contains(rustix::fs::OFlags::NONBLOCK)
        {
            return Err(Unavailable::Authority);
        }
        // Launcher обязан включить PASSCRED на обоих концах ДО передачи child.
        if !net::sockopt::socket_passcred(&socket).map_err(|_| Unavailable::Authority)? {
            return Err(Unavailable::Authority);
        }
        Ok(Self { socket })
    }
    fn bound(
        &self,
        which: net::sockopt::Timeout,
        deadline: Option<Instant>,
    ) -> Result<(), Unavailable> {
        let remaining = deadline
            .map(|d| {
                d.checked_duration_since(Instant::now())
                    .filter(|d| !d.is_zero())
                    .ok_or(Unavailable::Timeout)
            })
            .transpose()?;
        net::sockopt::set_socket_timeout(&self.socket, which, remaining)
            .map_err(|_| Unavailable::Authority)
    }
    pub(super) fn send(
        &self,
        bytes: &[u8],
        rights: &[BorrowedFd<'_>],
        deadline: Instant,
    ) -> Result<(), Unavailable> {
        if bytes.len() > super::lookup_codec::REPLY || rights.len() > 4 {
            return Err(Unavailable::Malformed);
        }
        loop {
            self.bound(net::sockopt::Timeout::Send, Some(deadline))?;
            let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(4))];
            let mut ancillary = SendAncillaryBuffer::new(&mut space);
            if !rights.is_empty() && !ancillary.push(SendAncillaryMessage::ScmRights(rights)) {
                return Err(Unavailable::Malformed);
            }
            match net::sendmsg(
                &self.socket,
                &[IoSlice::new(bytes)],
                &mut ancillary,
                SendFlags::NOSIGNAL,
            ) {
                Ok(count) if count == bytes.len() => return fence(deadline),
                Err(rustix::io::Errno::INTR) => continue,
                Err(rustix::io::Errno::AGAIN) => return Err(Unavailable::Timeout),
                _ => return Err(Unavailable::Disconnected),
            }
        }
    }
    pub(super) fn receive(
        &self,
        size: usize,
        deadline: Option<Instant>,
    ) -> Result<Packet, Unavailable> {
        if size > super::lookup_codec::REPLY {
            return Err(Unavailable::Malformed);
        }
        loop {
            self.bound(net::sockopt::Timeout::Recv, deadline)?;
            let mut bytes = [0u8; super::lookup_codec::REPLY + 1];
            let mut space =
                [MaybeUninit::uninit(); rustix::cmsg_space!(ScmCredentials(1), ScmRights(4))];
            let mut ancillary = RecvAncillaryBuffer::new(&mut space);
            let msg = match net::recvmsg(
                &self.socket,
                &mut [IoSliceMut::new(&mut bytes[..size + 1])],
                &mut ancillary,
                RecvFlags::CMSG_CLOEXEC,
            ) {
                Ok(msg) => msg,
                Err(rustix::io::Errno::INTR) => continue,
                Err(rustix::io::Errno::AGAIN) => return Err(Unavailable::Timeout),
                Err(_) => return Err(Unavailable::Disconnected),
            };
            let mut rights = Vec::with_capacity(4);
            let mut credentials = None;
            let mut bad = false;
            for part in ancillary.drain() {
                match part {
                    RecvAncillaryMessage::ScmRights(fds) => rights.extend(fds),
                    RecvAncillaryMessage::ScmCredentials(value) if credentials.is_none() => {
                        credentials = Some(value)
                    }
                    _ => bad = true,
                }
            }
            if bad
                || msg
                    .flags
                    .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
                || msg.bytes != size
                || rights.len() > 4
            {
                return Err(if msg.bytes == 0 {
                    Unavailable::Disconnected
                } else {
                    Unavailable::Malformed
                });
            }
            if let Some(deadline) = deadline {
                fence(deadline)?;
            }
            return Ok(Packet {
                bytes: bytes[..msg.bytes].to_vec(),
                rights,
                credentials: credentials.ok_or(Unavailable::Authority)?,
            });
        }
    }
    pub(super) fn shutdown(&self) {
        let _closed = net::shutdown(&self.socket, net::Shutdown::Both);
    }
}
pub(super) fn credentials(value: net::UCred, pid: u32, uid: u32) -> Result<(), Unavailable> {
    if value.pid.as_raw_pid() as u32 != pid || value.uid.as_raw() != uid {
        return Err(Unavailable::Authority);
    }
    Ok(())
}
pub(super) fn fence(deadline: Instant) -> Result<(), Unavailable> {
    if Instant::now() >= deadline {
        Err(Unavailable::Timeout)
    } else {
        Ok(())
    }
}
impl Drop for Channel {
    fn drop(&mut self) {
        self.shutdown();
    }
}
