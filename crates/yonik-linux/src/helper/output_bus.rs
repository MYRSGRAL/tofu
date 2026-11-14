//! Закрытый transport helper: pinned zbus не ставит CLOEXEC при SCM_RIGHTS receive.
//! Получаем owned FD атомарно; строгий DATA importer ничего не исправляет.
use std::{
    future::Future,
    io::{self, IoSliceMut},
    mem::MaybeUninit,
    os::fd::{AsFd, OwnedFd},
    pin::Pin,
};
use zbus::connection::socket::{BoxedSplit, ReadHalf};

#[derive(Debug)]
struct Receiver(tokio::net::UnixStream);
impl ReadHalf for Receiver {
    fn recvmsg<'a, 'b, 'f>(
        &'a mut self,
        bytes: &'b mut [u8],
    ) -> Pin<Box<dyn Future<Output = io::Result<(usize, Vec<OwnedFd>)>> + Send + 'f>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            use rustix::net::{
                RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, recvmsg,
            };
            self.0
                .async_io(tokio::io::Interest::READABLE, || {
                    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(64))];
                    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
                    let message = recvmsg(
                        &self.0,
                        &mut [IoSliceMut::new(bytes)],
                        &mut ancillary,
                        RecvFlags::CMSG_CLOEXEC,
                    )?;
                    let mut fds = Vec::new();
                    for control in ancillary.drain() {
                        match control {
                            RecvAncillaryMessage::ScmRights(rights) => fds.extend(rights),
                            _ => return Err(io::Error::from(io::ErrorKind::InvalidData)),
                        }
                    }
                    if message
                        .flags
                        .intersects(ReturnFlags::CTRUNC | ReturnFlags::TRUNC)
                    {
                        return Err(io::Error::from(io::ErrorKind::InvalidData));
                    }
                    if message.bytes == 0 {
                        return Err(io::Error::from(io::ErrorKind::BrokenPipe));
                    }
                    Ok((message.bytes, fds))
                })
                .await
        })
    }
    fn can_pass_unix_fd(&self) -> bool {
        true
    }
    fn peer_credentials<'a, 'f>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = io::Result<zbus::fdo::ConnectionCredentials>> + Send + 'f>>
    where
        'a: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            let peer = self.0.peer_cred()?;
            let credentials = zbus::fdo::ConnectionCredentials::default()
                .set_unix_user_id(peer.uid())
                .add_unix_group_id(peer.gid());
            Ok(match peer.pid() {
                Some(pid) => credentials.set_process_id(pid as u32),
                None => credentials,
            })
        })
    }
}

// Только helper system bus и собственный private-bus fixture; наружу не экспортируется.
pub(super) fn builder(
    stream: tokio::net::UnixStream,
) -> io::Result<zbus::connection::Builder<'static>> {
    let duplicate = stream.as_fd().try_clone_to_owned()?;
    let receiver =
        tokio::net::UnixStream::from_std(std::os::unix::net::UnixStream::from(duplicate))?;
    let mut socket: BoxedSplit = stream.into();
    *socket.read_mut() = Box::new(Receiver(receiver));
    Ok(zbus::connection::Builder::socket(socket))
}
