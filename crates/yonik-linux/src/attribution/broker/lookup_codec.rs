//! Private lookup DATA codec. Эти bytes не являются grant/permit или Go ABI.
use super::{Flow, Unavailable};

pub(super) const FRAME: usize = 256;
pub(super) const REPLY: usize = 768;
const MAGIC: &[u8; 8] = b"YNKLKP01";
pub(super) const HELLO: u8 = 1;
pub(super) const HELLO_ACK: u8 = 2;
pub(super) const LOOKUP: u8 = 3;
pub(super) const RESULT: u8 = 4;
pub(super) const REVOKE: u8 = 5;

/// Только DATA для закрытого transport; не opaque canonical-unit authority.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct LookupBinding {
    pub(crate) target_uid: u32,
    pub(crate) epoch: u64,
    pub(crate) nonce: [u8; 32],
    pub(crate) digest: [u8; 32],
    pub(crate) generation: u64,
    pub(crate) invocation: [u8; 16],
    pub(crate) server_pid: u32,
    pub(crate) server_start: u64,
    pub(crate) network: (u64, u64),
}
fn word<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], Unavailable> {
    bytes
        .get(offset..offset + N)
        .and_then(|v| v.try_into().ok())
        .ok_or(Unavailable::Malformed)
}
pub(super) fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, Unavailable> {
    Ok(u32::from_be_bytes(word(bytes, offset)?))
}
pub(super) fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, Unavailable> {
    Ok(u64::from_be_bytes(word(bytes, offset)?))
}
impl LookupBinding {
    pub(super) fn encode(&self, kind: u8, id: u64) -> Result<[u8; FRAME], Unavailable> {
        if self.target_uid == 0
            || self.epoch == 0
            || self.nonce == [0; 32]
            || self.digest == [0; 32]
            || self.generation == 0
            || self.invocation == [0; 16]
            || self.server_pid == 0
            || self.server_start == 0
            || self.network.1 == 0
            || !(HELLO..=REVOKE).contains(&kind)
            || (matches!(kind, LOOKUP | RESULT) != (id != 0))
        {
            return Err(Unavailable::Authority);
        }
        let mut bytes = [0; FRAME];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8] = kind;
        bytes[12..16].copy_from_slice(&self.target_uid.to_be_bytes());
        for (offset, value) in [
            (16, self.epoch),
            (24, id),
            (96, self.generation),
            (124, self.server_start),
            (132, self.network.0),
            (140, self.network.1),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
        }
        bytes[32..64].copy_from_slice(&self.nonce);
        bytes[64..96].copy_from_slice(&self.digest);
        bytes[104..120].copy_from_slice(&self.invocation);
        bytes[120..124].copy_from_slice(&self.server_pid.to_be_bytes());
        Ok(bytes)
    }
    pub(super) fn decode_hello(bytes: &[u8]) -> Result<Self, Unavailable> {
        if bytes.len() != FRAME
            || &bytes[..8] != MAGIC
            || bytes[8] != HELLO
            || bytes[9..12] != [0; 3]
            || bytes[148..].iter().any(|b| *b != 0)
        {
            return Err(Unavailable::Malformed);
        }
        let value = Self {
            target_uid: u32_at(bytes, 12)?,
            epoch: u64_at(bytes, 16)?,
            nonce: word(bytes, 32)?,
            digest: word(bytes, 64)?,
            generation: u64_at(bytes, 96)?,
            invocation: word(bytes, 104)?,
            server_pid: u32_at(bytes, 120)?,
            server_start: u64_at(bytes, 124)?,
            network: (u64_at(bytes, 132)?, u64_at(bytes, 140)?),
        };
        if value.encode(HELLO, 0)?.as_slice() != bytes {
            return Err(Unavailable::Malformed);
        }
        Ok(value)
    }
    pub(super) fn validate(
        &self,
        bytes: &[u8],
        kind: u8,
        id: u64,
        size: usize,
    ) -> Result<(), Unavailable> {
        if bytes.len() != size {
            return Err(Unavailable::Malformed);
        }
        let expected = self.encode(kind, id)?;
        if bytes[..9] != expected[..9] || bytes[12..148] != expected[12..148] {
            return Err(Unavailable::Authority);
        }
        if kind != RESULT && bytes[9..12] != [0; 3] {
            return Err(Unavailable::Malformed);
        }
        if matches!(kind, HELLO_ACK | REVOKE) && bytes[148..].iter().any(|byte| *byte != 0) {
            return Err(Unavailable::Malformed);
        }
        Ok(())
    }
    pub(super) fn request(&self, id: u64, flow: Flow) -> Result<[u8; FRAME], Unavailable> {
        validate_flow(flow)?;
        let mut b = self.encode(LOOKUP, id)?;
        b[148] = flow.family;
        b[149] = flow.protocol;
        b[150] = u8::from(flow.cookie.is_some());
        b[152..154].copy_from_slice(&flow.local_port.to_be_bytes());
        b[154..156].copy_from_slice(&flow.remote_port.to_be_bytes());
        b[156..172].copy_from_slice(&flow.local);
        b[172..188].copy_from_slice(&flow.remote);
        b[188..196].copy_from_slice(&flow.cookie.unwrap_or(0).to_be_bytes());
        Ok(b)
    }
    pub(super) fn flow(&self, b: &[u8], id: u64) -> Result<Flow, Unavailable> {
        self.validate(b, LOOKUP, id, FRAME)?;
        if b[150] > 1 || b[151] != 0 || b[196..].iter().any(|v| *v != 0) {
            return Err(Unavailable::Malformed);
        }
        let flow = Flow {
            family: b[148],
            protocol: b[149],
            local_port: u16::from_be_bytes(word(b, 152)?),
            remote_port: u16::from_be_bytes(word(b, 154)?),
            local: word(b, 156)?,
            remote: word(b, 172)?,
            cookie: if b[150] == 1 {
                Some(u64_at(b, 188)?)
            } else {
                if u64_at(b, 188)? != 0 {
                    return Err(Unavailable::Malformed);
                }
                None
            },
        };
        validate_flow(flow)?;
        Ok(flow)
    }
}
fn validate_flow(flow: Flow) -> Result<(), Unavailable> {
    if ![2, 10].contains(&flow.family)
        || ![6, 17].contains(&flow.protocol)
        || flow.local_port == 0
        || flow.remote_port == 0
        || flow.family == 2
            && (flow.local[4..].iter().any(|v| *v != 0) || flow.remote[4..].iter().any(|v| *v != 0))
    {
        return Err(Unavailable::Malformed);
    }
    Ok(())
}

pub(super) fn result_header(
    binding: &LookupBinding,
    id: u64,
    status: u8,
    reason: u16,
) -> Result<[u8; REPLY], Unavailable> {
    let header = binding.encode(RESULT, id)?;
    let mut bytes = [0; REPLY];
    bytes[..FRAME].copy_from_slice(&header);
    bytes[9] = status;
    bytes[10..12].copy_from_slice(&reason.to_be_bytes());
    Ok(bytes)
}
pub(super) fn reason_code(value: Unavailable) -> u16 {
    match value {
        Unavailable::Authority => 1,
        Unavailable::Coverage => 2,
        Unavailable::Limit => 3,
        Unavailable::Timeout => 4,
        Unavailable::Malformed => 5,
        Unavailable::Revoked => 6,
        Unavailable::Cancelled => 7,
        Unavailable::ReceiveOverflow => 8,
        Unavailable::AmbiguousSocket => 9,
        Unavailable::WorkerViewUnavailable => 10,
        Unavailable::Disconnected => 11,
    }
}
pub(super) fn reason_value(code: u16) -> Result<Unavailable, Unavailable> {
    Ok(match code {
        1 => Unavailable::Authority,
        2 => Unavailable::Coverage,
        3 => Unavailable::Limit,
        4 => Unavailable::Timeout,
        5 => Unavailable::Malformed,
        6 => Unavailable::Revoked,
        7 => Unavailable::Cancelled,
        8 => Unavailable::ReceiveOverflow,
        9 => Unavailable::AmbiguousSocket,
        10 => Unavailable::WorkerViewUnavailable,
        11 => Unavailable::Disconnected,
        _ => return Err(Unavailable::Malformed),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct KnownData {
    pub(super) pid: u32,
    pub(super) task_id: u32,
    pub(super) start: u64,
    pub(super) task_start: u64,
    pub(super) socket_inode: u64,
    pub(super) socket_fd: u32,
    pub(super) cookie: u64,
    pub(super) executable: (u64, u64),
    pub(super) nanos: u64,
    pub(super) path: String,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ResultData {
    Unknown,
    Unavailable(Unavailable),
    Known(KnownData),
}
pub(super) fn decode_result(
    binding: &LookupBinding,
    bytes: &[u8],
    id: u64,
    rights: usize,
) -> Result<ResultData, Unavailable> {
    binding.validate(bytes, RESULT, id, REPLY)?;
    let reason = u16::from_be_bytes([bytes[10], bytes[11]]);
    if bytes[9] != 1 {
        if rights != 0 || bytes[148..].iter().any(|b| *b != 0) {
            return Err(Unavailable::Malformed);
        }
        return match (bytes[9], reason) {
            (0, 0) => Ok(ResultData::Unknown),
            (2, code) => Ok(ResultData::Unavailable(reason_value(code)?)),
            _ => Err(Unavailable::Malformed),
        };
    }
    if reason != 0 || rights != 2 || bytes[218..256].iter().any(|b| *b != 0) {
        return Err(Unavailable::Malformed);
    }
    let value = KnownData {
        pid: u32_at(bytes, 148)?,
        task_id: u32_at(bytes, 152)?,
        start: u64_at(bytes, 156)?,
        task_start: u64_at(bytes, 164)?,
        socket_inode: u64_at(bytes, 172)?,
        socket_fd: u32_at(bytes, 180)?,
        cookie: u64_at(bytes, 184)?,
        executable: (u64_at(bytes, 192)?, u64_at(bytes, 200)?),
        nanos: u64_at(bytes, 208)?,
        path: String::new(),
    };
    let length = u16::from_be_bytes([bytes[216], bytes[217]]) as usize;
    if value.pid == 0
        || value.task_id == 0
        || value.start == 0
        || value.task_start == 0
        || value.socket_inode == 0
        || value.executable.1 == 0
        || value.nanos == 0
        || value.nanos > super::DEADLINE.as_nanos() as u64
        || length > super::super::native_wire::MAX_PATH
        || bytes[256 + length..].iter().any(|b| *b != 0)
    {
        return Err(Unavailable::Malformed);
    }
    let path = super::super::native_wire::executable_path(&bytes[256..256 + length])
        .map_err(|_| Unavailable::Malformed)?
        .to_owned();
    Ok(ResultData::Known(KnownData { path, ..value }))
}
pub(super) fn known_frame(
    binding: &LookupBinding,
    id: u64,
    value: &KnownData,
) -> Result<[u8; REPLY], Unavailable> {
    super::super::native_wire::executable_path(value.path.as_bytes())
        .map_err(|_| Unavailable::Malformed)?;
    let mut bytes = result_header(binding, id, 1, 0)?;
    for (offset, number) in [
        (148, value.pid),
        (152, value.task_id),
        (180, value.socket_fd),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&number.to_be_bytes());
    }
    for (offset, number) in [
        (156, value.start),
        (164, value.task_start),
        (172, value.socket_inode),
        (184, value.cookie),
        (192, value.executable.0),
        (200, value.executable.1),
        (208, value.nanos),
    ] {
        bytes[offset..offset + 8].copy_from_slice(&number.to_be_bytes());
    }
    bytes[216..218].copy_from_slice(&(value.path.len() as u16).to_be_bytes());
    bytes[256..256 + value.path.len()].copy_from_slice(value.path.as_bytes());
    decode_result(binding, &bytes, id, 2)?;
    Ok(bytes)
}

#[cfg(test)]
mod result_tests {
    use super::*;
    fn binding() -> LookupBinding {
        LookupBinding {
            target_uid: 1000,
            epoch: 1,
            nonce: [1; 32],
            digest: [2; 32],
            generation: 3,
            invocation: [4; 16],
            server_pid: 5,
            server_start: 6,
            network: (7, 8),
        }
    }
    fn known() -> KnownData {
        KnownData {
            pid: 1,
            task_id: 2,
            start: 3,
            task_start: 4,
            socket_inode: 5,
            socket_fd: 0,
            cookie: 6,
            executable: (7, 8),
            nanos: 1,
            path: "/usr/bin/app".into(),
        }
    }
    #[test]
    fn pure_lookup_result_roundtrip_known_unknown_and_every_unavailable_reason() {
        let binding = binding();
        let known = known();
        let bytes = known_frame(&binding, 1, &known).unwrap();
        assert_eq!(
            decode_result(&binding, &bytes, 1, 2).unwrap(),
            ResultData::Known(known)
        );
        assert_eq!(
            decode_result(&binding, &result_header(&binding, 2, 0, 0).unwrap(), 2, 0).unwrap(),
            ResultData::Unknown
        );
        for reason in 1..=11 {
            assert_eq!(
                decode_result(
                    &binding,
                    &result_header(&binding, 3, 2, reason).unwrap(),
                    3,
                    0
                )
                .unwrap(),
                ResultData::Unavailable(reason_value(reason).unwrap())
            );
        }
    }
    #[test]
    fn pure_lookup_result_rejects_fd_count_reserved_tail_binding_and_stale_id() {
        let binding = binding();
        let bytes = known_frame(&binding, 1, &known()).unwrap();
        for count in [0, 1, 3, 4] {
            assert!(decode_result(&binding, &bytes, 1, count).is_err());
        }
        for index in (218..256).chain(256 + known().path.len()..REPLY) {
            let mut bad = bytes;
            bad[index] = 1;
            assert!(decode_result(&binding, &bad, 1, 2).is_err());
        }
        for index in [0, 12, 16, 24, 32, 64, 96, 104, 120, 124, 132, 140] {
            let mut bad = bytes;
            bad[index] ^= 1;
            assert!(decode_result(&binding, &bad, 1, 2).is_err());
        }
        assert!(decode_result(&binding, &bytes, 2, 2).is_err());
        assert!(decode_result(&binding, &bytes[..REPLY - 1], 1, 2).is_err());
        let unknown = result_header(&binding, 1, 0, 0).unwrap();
        assert!(decode_result(&binding, &unknown, 1, 2).is_err());
    }
    #[test]
    fn pure_lookup_result_rejects_ttl_extension_zero_identity_and_unsafe_path() {
        let binding = binding();
        for nanos in [0, super::super::DEADLINE.as_nanos() as u64 + 1, u64::MAX] {
            assert!(known_frame(&binding, 1, &KnownData { nanos, ..known() }).is_err());
        }
        for path in ["", "relative", "/app (deleted)", "/app\0tail"] {
            assert!(
                known_frame(
                    &binding,
                    1,
                    &KnownData {
                        path: path.into(),
                        ..known()
                    }
                )
                .is_err()
            );
        }
        let exact = "/".to_owned() + &"a".repeat(511);
        assert!(
            known_frame(
                &binding,
                1,
                &KnownData {
                    path: exact,
                    ..known()
                }
            )
            .is_ok()
        );
        assert!(
            known_frame(
                &binding,
                1,
                &KnownData {
                    path: "/".to_owned() + &"a".repeat(512),
                    ..known()
                }
            )
            .is_err()
        );
        let bytes = known_frame(&binding, 1, &known()).unwrap();
        for (offset, size) in [(148, 4), (152, 4), (156, 8), (164, 8), (172, 8), (200, 8)] {
            let mut bad = bytes;
            bad[offset..offset + size].fill(0);
            assert!(decode_result(&binding, &bad, 1, 2).is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> LookupBinding {
        LookupBinding {
            target_uid: 1000,
            epoch: 1,
            nonce: [1; 32],
            digest: [2; 32],
            generation: 3,
            invocation: [4; 16],
            server_pid: 5,
            server_start: 6,
            network: (7, 8),
        }
    }
    fn flow() -> Flow {
        Flow {
            family: 2,
            protocol: 6,
            local_port: 1234,
            remote_port: 443,
            local: [0; 16],
            remote: [0; 16],
            cookie: Some(9),
        }
    }
    #[test]
    fn pure_lookup_codec_roundtrip_preserves_tcp_udp_ports_cookie_and_scope() {
        let binding = binding();
        let hello = binding.encode(HELLO, 0).unwrap();
        assert!(LookupBinding::decode_hello(&hello).unwrap() == binding);
        for family in [2, 10] {
            for protocol in [6, 17] {
                let value = Flow {
                    family,
                    protocol,
                    ..flow()
                };
                let bytes = binding.request(1, value).unwrap();
                assert_eq!(binding.flow(&bytes, 1).unwrap(), value);
            }
        }
    }
    #[test]
    fn pure_lookup_codec_rejects_every_binding_byte_reserved_and_wrong_request_id() {
        let binding = binding();
        let request = binding.request(1, flow()).unwrap();
        for index in (0..9).chain(12..148) {
            let mut bad = request;
            bad[index] ^= 1;
            assert!(binding.flow(&bad, 1).is_err(), "{index}");
        }
        for index in [9, 10, 11, 151, 196, 255] {
            let mut bad = request;
            bad[index] = 1;
            assert!(binding.flow(&bad, 1).is_err());
        }
        assert!(binding.flow(&request, 2).is_err());
        assert!(binding.flow(&request[..255], 1).is_err());
    }
    #[test]
    fn pure_lookup_codec_rejects_zero_ports_protocol_and_ipv4_tail() {
        let binding = binding();
        for value in [
            Flow {
                local_port: 0,
                ..flow()
            },
            Flow {
                remote_port: 0,
                ..flow()
            },
            Flow {
                protocol: 1,
                ..flow()
            },
            Flow {
                local: [1; 16],
                ..flow()
            },
        ] {
            assert!(binding.request(1, value).is_err());
        }
    }

    #[test]
    fn pure_lookup_control_rejects_every_reserved_tail_byte() {
        let binding = binding();
        for kind in [HELLO_ACK, REVOKE] {
            let bytes = binding.encode(kind, 0).unwrap();
            assert!(binding.validate(&bytes, kind, 0, FRAME).is_ok());
            for index in 148..FRAME {
                let mut invalid = bytes;
                invalid[index] = 1;
                assert!(matches!(
                    binding.validate(&invalid, kind, 0, FRAME),
                    Err(Unavailable::Malformed)
                ));
            }
        }
    }
}
