//! Закрытый RTNETLINK write transport: ACK не заменяет последующую сверку.
use super::{OperationLease, TunError};
use rustix::net::{
    self, AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType, netlink::SocketAddrNetlink,
};
use std::{io, os::fd::OwnedFd, sync::Arc};
use tokio::io::unix::AsyncFd;

const MAX_BYTES: usize = 64 * 1024;
const HEADER: usize = 16;

pub(super) enum Action {
    SetAlias { index: u32, alias: Vec<u8> },
    Delete { index: u32 },
}
impl Action {
    fn kind(&self) -> u16 {
        match self {
            Self::SetAlias { .. } => 19,
            Self::Delete { .. } => 17,
        }
    }
    fn request(&self, sequence: u32, port: u32) -> Result<Vec<u8>, TunError> {
        let index = match self {
            Self::SetAlias { index, .. } | Self::Delete { index } => *index,
        };
        if index == 0 || index > i32::MAX as u32 {
            return Err(TunError::Invalid);
        }
        let mut body = vec![0; 16];
        body[4..8].copy_from_slice(&index.to_ne_bytes());
        if let Self::SetAlias { alias, .. } = self {
            if alias.is_empty() || alias.len() > 254 || alias.contains(&0) {
                return Err(TunError::Invalid);
            }
            let length = 4 + alias.len() + 1;
            body.extend_from_slice(&(length as u16).to_ne_bytes());
            body.extend_from_slice(&20_u16.to_ne_bytes());
            body.extend_from_slice(alias);
            body.push(0);
            body.resize((body.len() + 3) & !3, 0);
        }
        let mut bytes = Vec::with_capacity(HEADER + body.len());
        bytes.extend_from_slice(&((HEADER + body.len()) as u32).to_ne_bytes());
        bytes.extend_from_slice(&self.kind().to_ne_bytes());
        bytes.extend_from_slice(&5_u16.to_ne_bytes()); // REQUEST | ACK, без replace.
        bytes.extend_from_slice(&sequence.to_ne_bytes());
        bytes.extend_from_slice(&port.to_ne_bytes());
        bytes.extend(body);
        Ok(bytes)
    }
}

fn value_u32(bytes: &[u8]) -> Result<u32, TunError> {
    Ok(u32::from_ne_bytes(
        bytes.try_into().map_err(|_| TunError::Evidence)?,
    ))
}
fn value_u16(bytes: &[u8]) -> Result<u16, TunError> {
    Ok(u16::from_ne_bytes(
        bytes.try_into().map_err(|_| TunError::Evidence)?,
    ))
}
fn accept(
    bytes: &[u8],
    sequence: u32,
    port: u32,
    kind: u16,
    request_length: usize,
) -> Result<bool, TunError> {
    let mut rest = bytes;
    let mut acknowledged = false;
    while !rest.is_empty() {
        if rest.len() < HEADER {
            return Err(TunError::Evidence);
        }
        let length = value_u32(&rest[..4])? as usize;
        if length < HEADER || length > rest.len() {
            return Err(TunError::Evidence);
        }
        let size = (length + 3) & !3;
        if size > rest.len() {
            return Err(TunError::Evidence);
        }
        if value_u32(&rest[8..12])? == sequence {
            if acknowledged
                || value_u16(&rest[6..8])? & !0x0300 != 0
                || value_u16(&rest[4..6])? != 2
                || value_u32(&rest[12..16])? != port
                || length < 36
            {
                return Err(TunError::Evidence);
            }
            let original = &rest[20..36];
            if value_u32(&original[..4])? as usize != request_length
                || value_u16(&original[6..8])? != 5
                || value_u16(&original[4..6])? != kind
                || value_u32(&original[8..12])? != sequence
                || value_u32(&original[12..16])? != port
            {
                return Err(TunError::Evidence);
            }
            let error =
                i32::from_ne_bytes(rest[16..20].try_into().map_err(|_| TunError::Evidence)?);
            if error < 0 {
                return Err(TunError::Rejected);
            }
            if error != 0 {
                return Err(TunError::Evidence);
            }
            acknowledged = true;
        }
        rest = &rest[size..];
    }
    Ok(acknowledged)
}

pub(super) async fn run(action: Action, lease: Arc<OperationLease>) -> Result<(), TunError> {
    let kind = action.kind();
    let socket = net::socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .map_err(|_| TunError::Io)?;
    net::bind(&socket, &SocketAddrNetlink::new(0, 0)).map_err(|_| TunError::Io)?;
    let address = net::getsockname(&socket).map_err(|_| TunError::Io)?;
    let port = SocketAddrNetlink::try_from(address)
        .map_err(|_| TunError::Io)?
        .pid();
    let mut random = [0; 4];
    getrandom::getrandom(&mut random).map_err(|_| TunError::Io)?;
    let sequence = u32::from_ne_bytes(random).max(1);
    let request = action.request(sequence, port)?;
    #[allow(deprecated)]
    let socket = AsyncFd::new(socket).map_err(|_| TunError::Io)?;
    // После отправки ACK ожидается даже при Drop вызывающего future: новая
    // операция не обгоняет уже переданный kernel запрос внутри процесса.
    tokio::spawn(async move {
        let _lease = lease;
        tokio::time::timeout(
            super::DEADLINE,
            exchange(socket, request, sequence, port, kind),
        )
        .await
        .map_err(|_| TunError::Timeout)?
    })
    .await
    .map_err(|_| TunError::Io)?
}
async fn exchange(
    socket: AsyncFd<OwnedFd>,
    request: Vec<u8>,
    sequence: u32,
    port: u32,
    kind: u16,
) -> Result<(), TunError> {
    loop {
        let mut ready = socket.writable().await.map_err(|_| TunError::Io)?;
        match ready.try_io(|fd| {
            net::sendto(
                fd.get_ref(),
                &request,
                SendFlags::empty(),
                &SocketAddrNetlink::new(0, 0),
            )
            .map_err(io::Error::from)
        }) {
            Err(_) => continue,
            Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
            Ok(Err(_)) => return Err(TunError::Io),
            Ok(Ok(length)) if length == request.len() => break,
            Ok(Ok(_)) => return Err(TunError::Evidence),
        }
    }
    let mut bytes = vec![0; MAX_BYTES];
    let mut used = 0_usize;
    loop {
        let mut ready = socket.readable().await.map_err(|_| TunError::Io)?;
        let (length, address) = match ready.try_io(|fd| {
            net::recvfrom(fd.get_ref(), &mut bytes, RecvFlags::TRUNC)
                .map(|(_, length, address)| (length, address))
                .map_err(io::Error::from)
        }) {
            Err(_) => continue,
            Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
            Ok(Err(_)) => return Err(TunError::Io),
            Ok(Ok(value)) => value,
        };
        used = used.checked_add(length).ok_or(TunError::OutputLimit)?;
        if length == 0 || length > bytes.len() || used > MAX_BYTES {
            return Err(TunError::OutputLimit);
        }
        let sender = address
            .and_then(|v| SocketAddrNetlink::try_from(v).ok())
            .ok_or(TunError::Evidence)?;
        if sender.pid() != 0 {
            continue;
        }
        if sender.groups() != 0 {
            return Err(TunError::Evidence);
        }
        if accept(&bytes[..length], sequence, port, kind, request.len())? {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ack() -> Vec<u8> {
        let request = Action::Delete { index: 7 }.request(3, 5).unwrap();
        let mut bytes = vec![0; 36];
        bytes[..4].copy_from_slice(&36_u32.to_ne_bytes());
        bytes[4..6].copy_from_slice(&2_u16.to_ne_bytes());
        bytes[8..12].copy_from_slice(&3_u32.to_ne_bytes());
        bytes[12..16].copy_from_slice(&5_u32.to_ne_bytes());
        bytes[20..36].copy_from_slice(&request[..16]);
        bytes
    }
    #[test]
    fn request_is_closed_and_targets_index_without_name() {
        let request = Action::Delete { index: 7 }.request(3, 5).unwrap();
        assert_eq!(request.len(), 32);
        assert_eq!(&request[20..24], &7_u32.to_ne_bytes());
        assert!(Action::Delete { index: 0 }.request(3, 5).is_err());
        assert!(
            Action::SetAlias {
                index: 7,
                alias: b"bad\0value".to_vec()
            }
            .request(3, 5)
            .is_err()
        );
        let request = Action::SetAlias {
            index: 7,
            alias: b"owner".to_vec(),
        }
        .request(3, 5)
        .unwrap();
        assert_eq!(&request[32..36], &[10, 0, 20, 0]);
        assert_eq!(&request[36..42], b"owner\0");
    }
    #[test]
    fn ack_requires_matching_outer_and_original_identity() {
        let original = ack();
        assert_eq!(accept(&original, 3, 5, 17, 32), Ok(true));
        assert_eq!(accept(&original, 4, 5, 17, 32), Ok(false));
        for offset in [4, 6, 12, 20, 24, 26, 28, 32] {
            let mut value = original.clone();
            value[offset] ^= 1;
            assert!(accept(&value, 3, 5, 17, 32).is_err(), "offset {offset}");
        }
        for length in [0, 1, 15, 16, 35] {
            assert!(accept(&original[..length], 3, 5, 17, 32).is_err() || length == 0);
        }
        let mut rejected = original;
        rejected[16..20].copy_from_slice(&(-1_i32).to_ne_bytes());
        assert_eq!(accept(&rejected, 3, 5, 17, 32), Err(TunError::Rejected));
    }
}
