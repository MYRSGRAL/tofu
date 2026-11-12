//! FROZEN production YNKBROK1 framing. Не создаёт unit/owner authority.
pub const MAGIC: &[u8; 8] = b"YNKBROK1";
pub const HEADER: usize = 64;
pub const MAX_FRAME: usize = 768;
pub const MAX_PATH: usize = 512;
pub const OWNER_TTL_NS: u64 = 20_000_000;
pub const BOOTSTRAP_NS: u64 = 1_000_000_000;
pub const OWNED_TUN: u64 = 1;
pub const OWNER_LOOKUP: u64 = 2;
pub const STRICT_APP_REQUIRED: u64 = 4;
pub const FEATURES: u64 = 7;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Kind {
    Hello = 1,
    Features = 2,
    Attach = 3,
    AttachAck = 4,
    Lookup = 16,
    Reply = 17,
    Revoke = 32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub request_id: u64,
    pub epoch: u64,
    pub digest: [u8; 32],
}
fn u16_at(b: &[u8], n: usize) -> u16 {
    u16::from_be_bytes([b[n], b[n + 1]])
}
fn u32_at(b: &[u8], n: usize) -> u32 {
    u32::from_be_bytes([b[n], b[n + 1], b[n + 2], b[n + 3]])
}
fn u64_at(b: &[u8], n: usize) -> u64 {
    u64::from_be_bytes([
        b[n],
        b[n + 1],
        b[n + 2],
        b[n + 3],
        b[n + 4],
        b[n + 5],
        b[n + 6],
        b[n + 7],
    ])
}
fn zero(b: &[u8]) -> bool {
    b.iter().all(|v| *v == 0)
}
impl Kind {
    pub fn fd_count(self) -> usize {
        usize::from(self == Self::Attach)
    }
    pub fn fixed_bytes(self) -> Option<usize> {
        match self {
            Self::Hello => Some(176),
            Self::Features => Some(112),
            Self::Attach => Some(128),
            Self::AttachAck | Self::Revoke => Some(104),
            Self::Lookup => Some(144),
            Self::Reply => None,
        }
    }
}
impl Header {
    /// Framing не является проверкой credentials/unit/anchor authority.
    pub fn decode(frame: &[u8], fd_count: usize) -> Result<Self, Malformed> {
        if frame.len() < HEADER
            || frame.len() > MAX_FRAME
            || &frame[..8] != MAGIC
            || u16_at(frame, 8) != 1
            || u32_at(frame, 12) as usize != frame.len()
        {
            return Err(Malformed);
        }
        let kind = match u16_at(frame, 10) {
            1 => Kind::Hello,
            2 => Kind::Features,
            3 => Kind::Attach,
            4 => Kind::AttachAck,
            16 => Kind::Lookup,
            17 => Kind::Reply,
            32 => Kind::Revoke,
            _ => return Err(Malformed),
        };
        if fd_count != kind.fd_count()
            || kind.fixed_bytes().is_some_and(|n| n != frame.len())
            || (kind == Kind::Reply && !(160..=160 + MAX_PATH).contains(&frame.len()))
        {
            return Err(Malformed);
        }
        let request_id = u64_at(frame, 16);
        let epoch = u64_at(frame, 24);
        if matches!(kind, Kind::Lookup | Kind::Reply) != (request_id != 0)
            || epoch == 0
            || zero(&frame[32..64])
            || zero(&frame[64..96])
        {
            return Err(Malformed);
        }
        validate_body(kind, frame)?;
        Ok(Self {
            kind,
            request_id,
            epoch,
            digest: frame[32..64].try_into().map_err(|_| Malformed)?,
        })
    }
    pub fn write(self, frame: &mut [u8]) -> Result<(), Malformed> {
        if frame.len() < HEADER || frame.len() > MAX_FRAME {
            return Err(Malformed);
        }
        let length = u32::try_from(frame.len()).map_err(|_| Malformed)?;
        frame[..8].copy_from_slice(MAGIC);
        frame[8..10].copy_from_slice(&1u16.to_be_bytes());
        frame[10..12].copy_from_slice(&(self.kind as u16).to_be_bytes());
        frame[12..16].copy_from_slice(&length.to_be_bytes());
        frame[16..24].copy_from_slice(&self.request_id.to_be_bytes());
        frame[24..32].copy_from_slice(&self.epoch.to_be_bytes());
        frame[32..64].copy_from_slice(&self.digest);
        Ok(())
    }
}
fn validate_body(kind: Kind, b: &[u8]) -> Result<(), Malformed> {
    let valid = match kind {
        Kind::Hello => {
            !zero(&b[96..112])
                && u64_at(b, 112) != 0
                && !zero(&b[120..136])
                && u32_at(b, 136) == MAX_FRAME as u32
                && u16_at(b, 140) == 1
                && zero(&b[142..144])
                && (1..=OWNER_TTL_NS).contains(&u64_at(b, 144))
                && (1..=BOOTSTRAP_NS).contains(&u64_at(b, 152))
                && u64_at(b, 160) & !FEATURES == 0
                && u64_at(b, 160) & OWNED_TUN != 0
                && u64_at(b, 168) & !1 == 0
                && (u64_at(b, 168) == 0 || u64_at(b, 160) & FEATURES == FEATURES)
        }
        Kind::Features => {
            u64_at(b, 96) & !FEATURES == 0
                && u64_at(b, 104) & !u64_at(b, 96) == 0
                && u64_at(b, 104) & OWNED_TUN != 0
        }
        Kind::Attach => {
            u32_at(b, 96) == 1
                && u32_at(b, 100) != 0
                && u32_at(b, 104) == 0x1001
                && (68..=65535).contains(&u32_at(b, 108))
                && interface_name(&b[112..128]).is_ok()
        }
        Kind::AttachAck => u32_at(b, 96) == 1 && zero(&b[100..104]),
        Kind::Lookup => {
            [4, 6].contains(&b[96])
                && [6, 17].contains(&b[97])
                && zero(&b[98..100])
                && zero(&b[136..144])
                && u16_at(b, 100) != 0
                && u16_at(b, 102) != 0
                && (b[96] != 4 || zero(&b[108..120]) && zero(&b[124..136]))
        }
        Kind::Reply => {
            let n = usize::from(u16_at(b, 100));
            if b.len() != 160 + n
                || n > MAX_PATH
                || !zero(&b[98..100])
                || !zero(&b[102..104])
                || !zero(&b[124..128])
                || !zero(&b[152..160])
            {
                return Err(Malformed);
            }
            match (b[96], b[97]) {
                (1, 0) => {
                    (1..=OWNER_TTL_NS).contains(&u64_at(b, 104))
                        && u32_at(b, 120) != 0
                        && u64_at(b, 128) != 0
                        && n != 0
                        && executable_path(&b[160..]).is_ok()
                }
                (2, 0) | (3, 1..=7) => n == 0 && zero(&b[104..152]),
                _ => false,
            }
        }
        Kind::Revoke => (1..=7).contains(&u32_at(b, 96)) && zero(&b[100..104]),
    };
    if valid { Ok(()) } else { Err(Malformed) }
}
fn interface_name(b: &[u8]) -> Result<&str, Malformed> {
    let n = b.iter().position(|v| *v == 0).ok_or(Malformed)?;
    if n == 0
        || !zero(&b[n..])
        || !b[..n]
            .iter()
            .all(|v| v.is_ascii_alphanumeric() || b"_.-".contains(v))
    {
        return Err(Malformed);
    }
    std::str::from_utf8(&b[..n]).map_err(|_| Malformed)
}
pub fn executable_path(b: &[u8]) -> Result<&str, Malformed> {
    if b.is_empty()
        || b.len() > MAX_PATH
        || b[0] != b'/'
        || b.contains(&0)
        || b.ends_with(b" (deleted)")
    {
        return Err(Malformed);
    }
    std::str::from_utf8(b).map_err(|_| Malformed)
}
/// app_required берётся из helper session policy, не FEATURES caller.
pub fn selected_features(frame: &[u8], app_required: bool) -> Result<u64, Malformed> {
    if Header::decode(frame, 0)?.kind != Kind::Features {
        return Err(Malformed);
    }
    let selected = u64_at(frame, 104);
    let required = if app_required { FEATURES } else { OWNED_TUN };
    if selected & required != required {
        return Err(Malformed);
    }
    Ok(selected)
}
pub fn lookup_flow(frame: &[u8]) -> Result<super::broker::Flow, Malformed> {
    if Header::decode(frame, 0)?.kind != Kind::Lookup {
        return Err(Malformed);
    }
    Ok(super::broker::Flow {
        family: if frame[96] == 4 { 2 } else { 10 },
        protocol: frame[97],
        local_port: u16_at(frame, 100),
        remote_port: u16_at(frame, 102),
        local: frame[104..120].try_into().map_err(|_| Malformed)?,
        remote: frame[120..136].try_into().map_err(|_| Malformed)?,
        cookie: None,
    })
}
/// Один outstanding request; timeout означает retire всего endpoint.
pub struct Sequence {
    last: u64,
    pending: Option<u64>,
    retired: bool,
}
impl Default for Sequence {
    fn default() -> Self {
        Self::new()
    }
}
impl Sequence {
    pub fn new() -> Self {
        Self {
            last: 0,
            pending: None,
            retired: false,
        }
    }
    pub fn begin(&mut self, id: u64) -> Result<(), Malformed> {
        if self.retired || self.pending.is_some() || id == 0 || id <= self.last {
            self.retire();
            return Err(Malformed);
        }
        self.last = id;
        self.pending = Some(id);
        Ok(())
    }
    pub fn complete(&mut self, id: u64) -> Result<(), Malformed> {
        if self.retired || self.pending != Some(id) {
            self.retire();
            return Err(Malformed);
        }
        self.pending = None;
        Ok(())
    }
    pub fn retire(&mut self) {
        self.pending = None;
        self.retired = true;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn lookup() -> Vec<u8> {
        let mut b = vec![0; 144];
        b[..8].copy_from_slice(MAGIC);
        b[8..10].copy_from_slice(&1u16.to_be_bytes());
        b[10..12].copy_from_slice(&(Kind::Lookup as u16).to_be_bytes());
        b[12..16].copy_from_slice(&144u32.to_be_bytes());
        b[16..24].copy_from_slice(&1u64.to_be_bytes());
        b[24..32].copy_from_slice(&7u64.to_be_bytes());
        b[32..64].fill(2);
        b[64..96].fill(3);
        b[96] = 4;
        b[97] = 6;
        b[100..102].copy_from_slice(&1234u16.to_be_bytes());
        b[102..104].copy_from_slice(&443u16.to_be_bytes());
        b[104..108].copy_from_slice(&[127, 0, 0, 1]);
        b[120..124].copy_from_slice(&[1, 2, 3, 4]);
        b
    }
    #[test]
    fn frozen_lookup_header_and_cookie_source_rejection() {
        let b = lookup();
        assert_eq!(Header::decode(&b, 0).unwrap().kind, Kind::Lookup);
        for offset in [98, 99, 136, 143] {
            let mut bad = b.clone();
            bad[offset] = 1;
            assert_eq!(Header::decode(&bad, 0), Err(Malformed));
        }
    }
    #[test]
    fn fixtures_extra_fds_lengths_and_versions_do_not_negotiate_production() {
        let b = lookup();
        assert_eq!(Header::decode(&b, 1), Err(Malformed));
        for n in [0, 63, 135, 143] {
            assert_eq!(Header::decode(&b[..n], 0), Err(Malformed));
        }
        for offset in [0, 8, 9, 10, 11, 12, 15] {
            let mut bad = b.clone();
            bad[offset] ^= 1;
            assert_eq!(Header::decode(&bad, 0), Err(Malformed));
        }
        let mut old = vec![0; 136];
        old[..4].copy_from_slice(b"YOB1");
        assert_eq!(Header::decode(&old, 0), Err(Malformed));
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    fn frame(kind: Kind, n: usize, id: u64) -> Vec<u8> {
        let mut b = vec![0; n];
        Header {
            kind,
            request_id: id,
            epoch: 7,
            digest: [2; 32],
        }
        .write(&mut b)
        .unwrap();
        b[64..96].fill(3);
        b
    }
    #[test]
    fn mandatory_tun_and_app_features_cannot_be_downgraded() {
        let mut b = frame(Kind::Features, 112, 0);
        b[96..104].copy_from_slice(&FEATURES.to_be_bytes());
        for selected in [0u64, 2, 4, 6, 8] {
            b[104..112].copy_from_slice(&selected.to_be_bytes());
            assert!(selected_features(&b, true).is_err());
        }
        b[104..112].copy_from_slice(&1u64.to_be_bytes());
        assert_eq!(selected_features(&b, false), Ok(1));
        assert!(selected_features(&b, true).is_err());
        b[104..112].copy_from_slice(&FEATURES.to_be_bytes());
        assert_eq!(selected_features(&b, true), Ok(FEATURES));
    }
    #[test]
    fn attach_has_exactly_one_fd_and_fixed_bound_tun_role() {
        let mut b = frame(Kind::Attach, 128, 0);
        b[96..100].copy_from_slice(&1u32.to_be_bytes());
        b[100..104].copy_from_slice(&9u32.to_be_bytes());
        b[104..108].copy_from_slice(&0x1001u32.to_be_bytes());
        b[108..112].copy_from_slice(&1500u32.to_be_bytes());
        b[112..121].copy_from_slice(b"yonik-tun");
        assert!(Header::decode(&b, 1).is_ok());
        for n in [0, 2, 3] {
            assert!(Header::decode(&b, n).is_err());
        }
        b[99] = 2;
        assert!(Header::decode(&b, 1).is_err());
    }
    #[test]
    fn ids_reuse_outstanding_and_late_replies_retire_endpoint() {
        let mut s = Sequence::new();
        assert!(s.begin(1).is_ok());
        assert!(s.begin(2).is_err());
        assert!(s.complete(1).is_err());
        let mut s = Sequence::new();
        s.begin(1).unwrap();
        s.complete(1).unwrap();
        assert!(s.begin(1).is_err());
        let mut s = Sequence::new();
        s.begin(u64::MAX).unwrap();
        s.complete(u64::MAX).unwrap();
        assert!(s.begin(1).is_err());
    }
    #[test]
    fn known_reply_projection_ttl_path_and_unknown_fields_are_bounded() {
        let path = b"/usr/bin/test";
        let mut b = frame(Kind::Reply, 160 + path.len(), 1);
        b[96] = 1;
        b[100..102].copy_from_slice(&(path.len() as u16).to_be_bytes());
        b[104..112].copy_from_slice(&OWNER_TTL_NS.to_be_bytes());
        b[120..124].copy_from_slice(&123u32.to_be_bytes());
        b[128..136].copy_from_slice(&12u64.to_be_bytes());
        b[160..].copy_from_slice(path);
        assert!(Header::decode(&b, 0).is_ok());
        b[104..112].copy_from_slice(&(OWNER_TTL_NS + 1).to_be_bytes());
        assert!(Header::decode(&b, 0).is_err());
        let mut b = frame(Kind::Reply, 160, 1);
        b[96] = 2;
        assert!(Header::decode(&b, 0).is_ok());
        b[120] = 1;
        assert!(Header::decode(&b, 0).is_err());
    }
}

#[cfg(test)]
mod golden_tests {
    use super::*;
    #[test]
    fn frozen_cross_language_vectors() {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/native-broker-v1.json"
        ))
        .unwrap();
        for row in data["vectors"].as_array().unwrap() {
            let hex = row["hex"].as_str().unwrap();
            let bytes: Vec<u8> = hex
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect();
            assert_eq!(
                Header::decode(&bytes, row["fd_count"].as_u64().unwrap() as usize).is_ok(),
                row["header_ok"].as_bool().unwrap(),
                "{}",
                row["name"]
            );
            if let Some(app_required) = row["app_required"].as_bool() {
                assert_eq!(
                    selected_features(&bytes, app_required).is_ok(),
                    row["features_ok"].as_bool().unwrap(),
                    "{}",
                    row["name"]
                );
            }
        }
    }
}
