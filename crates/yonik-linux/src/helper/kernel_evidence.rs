//! Приватные DATA v4. Ни transport custody, ни применение namespace policy не доказаны.
#![allow(dead_code)]
use super::run_authorization::{RunAuthority, RunAuthorization, RunRevocationGuard};
use std::{sync::Mutex, time::Instant};

const FRAME: usize = 512;
const VECTOR: usize = 19;
// Source170: old3, затем interleaved unshare/legacy clone для known8.
const KNOWN8: [u32; 8] = [
    0x02000000, 0x08000000, 0x40000000, 0x00020000, 0x20000000, 0x10000000, 0x04000000, 0x80,
];

#[derive(Debug, PartialEq, Eq)]
enum Error {
    Revoked,
    Expired,
    Transport,
    Protocol,
    Binding,
    Profile,
    EvidenceUnavailable,
    ObservationMismatch,
    Consumed,
}

/// Значения поступают из private pending context, никогда из report.
struct Expected {
    nonce: [u8; 32],
    digest: [u8; 32],
    session: [u8; 16],
    generation: u64,
    invocation: [u8; 16],
    epoch: u64,
    task_pid: u32,
    task_start: u64,
    server_pid: u32,
    server_start: u64,
    profile: [u8; 32],
    observations: [u32; VECTOR],
}
struct ParsedReport {
    receipt_reference: Option<[u8; 32]>,
    baseline_filters: u32,
    current_filters: u32,
    observations: [u32; VECTOR],
}
impl ParsedReport {
    /// Полный supported-profile verifier отсутствует. Digest не превращается в proof.
    fn supported_profile(&self) -> Result<(), Error> {
        Err(Error::EvidenceUnavailable)
    }
}
fn bytes<const N: usize>(frame: &[u8; FRAME], offset: usize) -> [u8; N] {
    let mut value = [0; N];
    value.copy_from_slice(&frame[offset..offset + N]);
    value
}
fn u32_at(frame: &[u8; FRAME], offset: usize) -> u32 {
    u32::from_be_bytes(bytes(frame, offset))
}
fn u64_at(frame: &[u8; FRAME], offset: usize) -> u64 {
    u64::from_be_bytes(bytes(frame, offset))
}
fn decode(frame: &[u8], expected: &Expected) -> Result<ParsedReport, Error> {
    let frame: &[u8; FRAME] = frame.try_into().map_err(|_| Error::Transport)?;
    if &frame[..8] != b"YNKNS004"
        || [(124, 128), (140, 144), (228, 232), (308, 512)]
            .iter()
            .any(|&(a, b)| frame[a..b].iter().any(|&v| v != 0))
        || [88, 112, 128, 144].iter().any(|&o| u64_at(frame, o) == 0)
        || [120, 136].iter().any(|&o| u32_at(frame, o) == 0)
        || u32_at(frame, 216) != 2
    {
        return Err(Error::Protocol);
    }
    let observations = std::array::from_fn(|i| u32_at(frame, 232 + i * 4));
    if observations.iter().any(|v| !(1..=4095).contains(v)) {
        return Err(Error::Protocol);
    }
    if bytes::<32>(frame, 8) != expected.nonce
        || bytes::<32>(frame, 40) != expected.digest
        || bytes::<16>(frame, 72) != expected.session
        || u64_at(frame, 88) != expected.generation
        || bytes::<16>(frame, 96) != expected.invocation
        || u64_at(frame, 112) != expected.epoch
        || u32_at(frame, 120) != expected.task_pid
        || u64_at(frame, 128) != expected.task_start
        || u32_at(frame, 136) != expected.server_pid
        || u64_at(frame, 144) != expected.server_start
    {
        return Err(Error::Binding);
    }
    if bytes::<32>(frame, 152) != expected.profile {
        return Err(Error::Profile);
    }
    if observations != expected.observations {
        return Err(Error::ObservationMismatch);
    }
    let receipt = bytes(frame, 184);
    Ok(ParsedReport {
        receipt_reference: (receipt != [0; 32]).then_some(receipt),
        baseline_filters: u32_at(frame, 220),
        current_filters: u32_at(frame, 224),
        observations,
    })
}

/// Mutex сериализует только DATA consume. Нет Clone, публичного raw-конструктора или permit.
struct PendingReport<'a> {
    owner: &'a RunAuthority,
    authorization: RunAuthorization,
    expected: Expected,
    deadline: Instant,
    slot: Mutex<Option<RunRevocationGuard>>,
}
struct ConsumedRecord {
    data: ParsedReport,
    _guard: RunRevocationGuard,
}
impl PendingReport<'_> {
    fn consume(&self, frame: &[u8]) -> Result<ConsumedRecord, Error> {
        let mut slot = self.slot.lock().map_err(|_| Error::Revoked)?;
        if slot.is_none() {
            return Err(Error::Consumed);
        }
        let result = (|| {
            self.authorization
                .validate_owner(self.owner)
                .map_err(|_| Error::Revoked)?;
            if Instant::now() >= self.deadline {
                return Err(Error::Expired);
            }
            let (nonce, session, generation) = self
                .authorization
                .challenge_context()
                .map_err(|_| Error::Revoked)?;
            if (nonce, session, generation)
                != (
                    self.expected.nonce,
                    self.expected.session,
                    self.expected.generation,
                )
            {
                return Err(Error::Binding);
            }
            let data = decode(frame, &self.expected)?;
            self.authorization
                .validate_owner(self.owner)
                .map_err(|_| Error::Revoked)?;
            if Instant::now() >= self.deadline {
                return Err(Error::Expired);
            }
            Ok(data)
        })();
        let guard = slot.take().ok_or(Error::Consumed)?;
        result.map(|data| ConsumedRecord {
            data,
            _guard: guard,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn expected() -> Expected {
        Expected {
            nonce: [2; 32],
            digest: [3; 32],
            session: [1; 16],
            generation: 1,
            invocation: [4; 16],
            epoch: 9,
            task_pid: 10,
            task_start: 11,
            server_pid: 12,
            server_start: 13,
            profile: [5; 32],
            observations: std::array::from_fn(|i| i as u32 + 1),
        }
    }
    fn frame(e: &Expected) -> [u8; FRAME] {
        let mut f = [0; FRAME];
        f[..8].copy_from_slice(b"YNKNS004");
        for (o, v) in [
            (8, e.nonce.as_slice()),
            (40, &e.digest),
            (72, &e.session),
            (96, &e.invocation),
            (152, &e.profile),
        ] {
            f[o..o + v.len()].copy_from_slice(v);
        }
        for (o, v) in [
            (88, e.generation),
            (112, e.epoch),
            (128, e.task_start),
            (144, e.server_start),
        ] {
            f[o..o + 8].copy_from_slice(&v.to_be_bytes());
        }
        for (o, v) in [(120, e.task_pid), (136, e.server_pid), (216, 2)] {
            f[o..o + 4].copy_from_slice(&v.to_be_bytes());
        }
        for (i, v) in e.observations.iter().enumerate() {
            f[232 + i * 4..236 + i * 4].copy_from_slice(&v.to_be_bytes());
        }
        f
    }
    fn pending(owner: &RunAuthority, deadline: Instant) -> PendingReport<'_> {
        let authorization = owner.authorize([2; 32]).unwrap();
        let guard = authorization.guard();
        PendingReport {
            owner,
            authorization,
            expected: expected(),
            deadline,
            slot: Mutex::new(Some(guard)),
        }
    }
    fn error(f: &[u8], e: &Expected) -> Error {
        decode(f, e).err().expect("отказ DATA")
    }
    #[test]
    fn framing_bounds_reserved_and_arbitrary() {
        let e = expected();
        let f = frame(&e);
        for len in 0..FRAME {
            assert_eq!(error(&f[..len], &e), Error::Transport);
        }
        assert_eq!(error(&[0; 513], &e), Error::Transport);
        for i in (124..128)
            .chain(140..144)
            .chain(228..232)
            .chain(308..512)
            .chain(0..8)
        {
            let mut bad = f;
            bad[i] ^= 1;
            assert_eq!(error(&bad, &e), Error::Protocol);
        }
        for offset in [88, 112, 120, 128, 136, 144] {
            let mut bad = f;
            let n = if offset == 120 || offset == 136 { 4 } else { 8 };
            bad[offset..offset + n].fill(0);
            assert_eq!(error(&bad, &e), Error::Protocol);
        }
        for i in 0..VECTOR {
            for v in [0u32, 4096, u32::MAX] {
                let mut bad = f;
                bad[232 + i * 4..236 + i * 4].copy_from_slice(&v.to_be_bytes());
                assert_eq!(error(&bad, &e), Error::Protocol);
            }
        }
        for seed in 0..=255u8 {
            assert_eq!(error(&[seed; 512], &e), Error::Protocol);
        }
    }
    #[test]
    fn every_binding_and_endian() {
        let e = expected();
        let f = frame(&e);
        for i in (8..124).chain(128..140).chain(144..152) {
            let mut bad = f;
            bad[i] ^= 0x40;
            assert_eq!(error(&bad, &e), Error::Binding, "offset {i}");
        }
        for i in 152..184 {
            let mut bad = f;
            bad[i] ^= 1;
            assert_eq!(error(&bad, &e), Error::Profile);
        }
        for i in 3..18 {
            let mut bad = f;
            bad[232 + i * 4..236 + i * 4].copy_from_slice(&e.observations[i + 1].to_be_bytes());
            assert_eq!(error(&bad, &e), Error::ObservationMismatch);
        }
        let mut upper = expected();
        upper.observations = [4095; VECTOR];
        assert!(decode(&frame(&upper), &upper).is_ok());
        let mut bad = f;
        bad[88..96].copy_from_slice(&1u64.to_le_bytes());
        assert_eq!(error(&bad, &e), Error::Binding);
        let mut bad = f;
        bad[232..236].copy_from_slice(&2u32.to_be_bytes());
        assert_eq!(error(&bad, &e), Error::ObservationMismatch);
    }
    #[test]
    fn optional_reference_never_admits() {
        let e = expected();
        let mut f = frame(&e);
        for receipt in [[0; 32], [7; 32]] {
            f[184..216].copy_from_slice(&receipt);
            let data = decode(&f, &e).unwrap();
            assert_eq!(
                data.receipt_reference,
                (receipt != [0; 32]).then_some(receipt)
            );
            assert_eq!(data.supported_profile(), Err(Error::EvidenceUnavailable));
        }
    }
    #[test]
    fn transfer_drop_replay_and_concurrent_winner() {
        let owner = RunAuthority::new([1; 16], 1, 9).unwrap();
        let p = pending(&owner, Instant::now() + Duration::from_secs(1));
        let f = frame(&p.expected);
        let record = std::thread::scope(|s| {
            let a = s.spawn(|| p.consume(&f));
            let b = s.spawn(|| p.consume(&f));
            let results = [a.join().unwrap(), b.join().unwrap()];
            assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
            assert!(results.iter().any(|r| matches!(r, Err(Error::Consumed))));
            results.into_iter().find_map(Result::ok).unwrap()
        });
        drop(p);
        assert!(!owner.is_revoked());
        assert_eq!(
            record.data.supported_profile(),
            Err(Error::EvidenceUnavailable)
        );
        drop(record);
        assert!(owner.is_revoked());
    }
    #[test]
    fn late_wrong_binding_and_retained_revocation() {
        let owner = RunAuthority::new([1; 16], 1, 9).unwrap();
        let p = pending(&owner, Instant::now() - Duration::from_secs(1));
        let mut f = frame(&p.expected);
        f[8] ^= 1;
        assert!(matches!(p.consume(&f), Err(Error::Expired)));
        assert!(matches!(p.consume(&f), Err(Error::Consumed)));
        let owner = RunAuthority::new([1; 16], 1, 9).unwrap();
        let p = pending(&owner, Instant::now() + Duration::from_secs(1));
        let record = p.consume(&frame(&p.expected)).unwrap();
        assert!(matches!(p.consume(&[]), Err(Error::Consumed)));
        owner.revoke();
        assert!(owner.is_revoked());
        drop(record);
    }
    #[test]
    fn trusted_revocation_expiry_and_cancel_precede_framing() {
        for revoke in [false, true] {
            let owner = RunAuthority::new([1; 16], 1, 9).unwrap();
            let p = pending(&owner, Instant::now());
            if revoke {
                owner.revoke();
            }
            assert!(
                matches!(p.consume(&[]),Err(e) if e==if revoke {Error::Revoked}else{Error::Expired})
            );
            assert!(owner.is_revoked());
        }
        let owner = RunAuthority::new([1; 16], 1, 9).unwrap();
        let p = pending(&owner, Instant::now() + Duration::from_secs(1));
        let cancel = p.authorization.guard();
        drop(cancel);
        assert!(matches!(p.consume(&[]), Err(Error::Revoked)));
        let owner = RunAuthority::new([1; 16], 1, 9).unwrap();
        drop(pending(&owner, Instant::now() + Duration::from_secs(1)));
        assert!(owner.is_revoked());
    }
}
