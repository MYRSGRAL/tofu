//! Read-only INSPECT_PEER DATA, отдельный от runtime lookup binding.
use super::Unavailable;
use super::lookup_codec::{FRAME, u32_at, u64_at};
pub(super) const MAGIC: &[u8; 8] = b"YNKPEER1";
pub(super) const HELLO: u8 = 1;
pub(super) const ACK: u8 = 2;
pub(super) const INSPECT: u8 = 3;
pub(super) const RESULT: u8 = 4;
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Binding {
    pub(super) uid: u32,
    pub(super) caller: u32,
    pub(super) server: u32,
    pub(super) caller_start: u64,
    pub(super) server_start: u64,
    pub(super) invocation: [u8; 16],
    pub(super) nonce: [u8; 32],
    pub(super) inspector: u32,
    pub(super) inspector_start: u64,
}
impl Binding {
    pub(super) fn encode(&self, kind: u8, id: u64) -> Result<[u8; FRAME], Unavailable> {
        if self.uid == 0
            || self.caller == 0
            || self.server == 0
            || self.caller_start == 0
            || self.server_start == 0
            || self.invocation == [0; 16]
            || self.nonce == [0; 32]
            || self.inspector == 0
            || self.inspector_start == 0
            || !(HELLO..=RESULT).contains(&kind)
            || (matches!(kind, INSPECT | RESULT) != (id != 0))
        {
            return Err(Unavailable::Authority);
        }
        let mut bytes = [0; FRAME];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8] = kind;
        for (offset, value) in [
            (12, self.uid),
            (16, self.caller),
            (20, self.server),
            (96, self.inspector),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        for (offset, value) in [
            (24, self.caller_start),
            (32, self.server_start),
            (88, id),
            (104, self.inspector_start),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
        }
        bytes[40..56].copy_from_slice(&self.invocation);
        bytes[56..88].copy_from_slice(&self.nonce);
        Ok(bytes)
    }
    pub(super) fn hello(bytes: &[u8]) -> Result<Self, Unavailable> {
        if bytes.len() != FRAME || &bytes[..8] != MAGIC {
            return Err(Unavailable::Malformed);
        }
        let value = Self {
            uid: u32_at(bytes, 12)?,
            caller: u32_at(bytes, 16)?,
            server: u32_at(bytes, 20)?,
            caller_start: u64_at(bytes, 24)?,
            server_start: u64_at(bytes, 32)?,
            invocation: bytes[40..56]
                .try_into()
                .map_err(|_| Unavailable::Malformed)?,
            nonce: bytes[56..88]
                .try_into()
                .map_err(|_| Unavailable::Malformed)?,
            inspector: u32_at(bytes, 96)?,
            inspector_start: u64_at(bytes, 104)?,
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
        rights: usize,
    ) -> Result<(), Unavailable> {
        if bytes.len() != FRAME {
            return Err(Unavailable::Malformed);
        }
        let expected = self.encode(kind, id)?;
        if bytes[..9] != expected[..9] || bytes[12..] != expected[12..] {
            return Err(Unavailable::Authority);
        }
        let reason = u16::from_be_bytes([bytes[10], bytes[11]]);
        match (kind, bytes[9], reason, rights) {
            (HELLO | INSPECT, 0, 0, 0) | (ACK | RESULT, 0, 0, 4) => Ok(()),
            (RESULT, 1, 1..=7, 0) => Ok(()),
            _ => Err(Unavailable::Malformed),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> Binding {
        Binding {
            uid: 1000,
            caller: 2,
            server: 3,
            caller_start: 4,
            server_start: 5,
            invocation: [6; 16],
            nonce: [7; 32],
            inspector: 8,
            inspector_start: 9,
        }
    }
    #[test]
    fn pure_inspect_codec_roundtrip_has_no_native_epoch_or_run_role() {
        let b = binding();
        assert!(Binding::hello(&b.encode(HELLO, 0).unwrap()).unwrap() == b);
        for (kind, id, rights) in [(HELLO, 0, 0), (ACK, 0, 4), (INSPECT, 1, 0), (RESULT, 1, 4)] {
            assert!(
                b.validate(&b.encode(kind, id).unwrap(), kind, id, rights)
                    .is_ok()
            );
        }
    }
    #[test]
    fn pure_inspect_codec_rejects_wrong_binding_id_fd_count_and_every_reserved_byte() {
        let b = binding();
        let bytes = b.encode(RESULT, 1).unwrap();
        for index in (0..9).chain(12..FRAME) {
            let mut bad = bytes;
            bad[index] ^= 1;
            assert!(b.validate(&bad, RESULT, 1, 4).is_err());
        }
        for count in [0, 1, 2, 3, 5] {
            assert!(b.validate(&bytes, RESULT, 1, count).is_err());
        }
        assert!(b.validate(&bytes, RESULT, 2, 4).is_err());
        assert!(b.validate(&bytes[..FRAME - 1], RESULT, 1, 4).is_err());
    }
}
