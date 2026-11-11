//! Отдельная authenticated роль исходных canonical records без смены native schema.
use super::{Descriptor, EncryptedRuleSetStore, Inner, RuntimeLease, bad, open, read_resource};
use crate::resource_fd_receiver::CanonicalDescriptor;
use crate::rule_set_custody::{RuleSetFormat, RuleSetRef};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rustix::fs;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read, Write},
    os::fd::AsRawFd,
};
use yonik_engine::resource_bundle::ResourceKind;
use zeroize::Zeroizing;
const HEADER: usize = 50;
const SOURCE_CAP: u64 = 16 * 1024 * 1024;
pub(super) fn source_name(d: &Descriptor) -> String {
    d.name().replacen("r-", "c-", 1)
}
fn header(source: &[u8], kind: ResourceKind, records: u64) -> Vec<u8> {
    let mut h = vec![1, if kind == ResourceKind::Domains { 1 } else { 2 }];
    h.extend_from_slice(&Sha256::digest(source));
    h.extend_from_slice(&(source.len() as u64).to_be_bytes());
    h.extend_from_slice(&records.to_be_bytes());
    h
}
fn aad(d: &Descriptor, header: &[u8]) -> io::Result<Vec<u8>> {
    let mut a = b"yonik-canonical-source-v1\0".to_vec();
    a.extend_from_slice(&serde_json::to_vec(d).map_err(|_| bad())?);
    a.extend_from_slice(header);
    Ok(a)
}
fn parse(h: &[u8]) -> io::Result<CanonicalDescriptor> {
    if h.len() != HEADER || h[0] != 1 {
        return Err(bad());
    }
    let d = CanonicalDescriptor {
        format_version: 1,
        kind: match h[1] {
            1 => ResourceKind::Domains,
            2 => ResourceKind::Cidrs,
            _ => return Err(bad()),
        },
        digest: h[2..34].try_into().map_err(|_| bad())?,
        bytes: u64::from_be_bytes(h[34..42].try_into().map_err(|_| bad())?),
        records: u64::from_be_bytes(h[42..50].try_into().map_err(|_| bad())?),
    };
    if d.bytes == 0 || d.bytes > SOURCE_CAP || d.records == 0 || d.records > 1_500_000 {
        return Err(bad());
    }
    Ok(d)
}
fn read_source(
    inner: &Inner,
    native: &Descriptor,
) -> io::Result<(CanonicalDescriptor, Zeroizing<Vec<u8>>)> {
    let mut file = match open(&inner.root, &source_name(native)) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Legacy artifact не содержит canonical proof; требуется rebuild из исходного набора",
            ));
        }
        other => other?,
    };
    let mut h = [0u8; HEADER];
    file.read_exact(&mut h)?;
    let d = parse(&h)?;
    if native.records != d.records || file.metadata()?.len() != d.bytes + HEADER as u64 + 40 {
        return Err(bad());
    }
    let _reservation =
        crate::resource_fd_receiver::Reservation::acquire(d.bytes.checked_mul(3).ok_or_else(bad)?)?;
    let mut nonce = [0u8; 24];
    file.read_exact(&mut nonce)?;
    let mut ciphertext = Zeroizing::new(vec![0; d.bytes as usize + 16]);
    file.read_exact(&mut ciphertext)?;
    let cipher = XChaCha20Poly1305::new((&*inner.key).into());
    let body = Zeroizing::new(
        cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &aad(native, &h)?,
                },
            )
            .map_err(|_| bad())?,
    );
    if Sha256::digest(&*body).as_slice() != d.digest {
        return Err(bad());
    }
    Ok((d, body))
}
impl EncryptedRuleSetStore {
    /// Сохраняет точные original records, включая порядок и дубликаты.
    /// Native counterpart обязан уже существовать. Atomic current/previous не меняет.
    pub fn publish_canonical(
        &mut self,
        reference: &RuleSetRef,
        source: &[u8],
        kind: ResourceKind,
    ) -> io::Result<()> {
        self.publish_canonical_reserved(reference, source, kind, false)
    }
    pub(crate) fn publish_canonical_reserved(
        &mut self,
        reference: &RuleSetRef,
        source: &[u8],
        kind: ResourceKind,
        pre_reserved: bool,
    ) -> io::Result<()> {
        if self.poisoned || source.is_empty() || source.len() as u64 > SOURCE_CAP {
            return Err(bad());
        }
        let d = Descriptor::from_ref(reference);
        let _reservation = if pre_reserved {
            None
        } else {
            Some(crate::resource_fd_receiver::Reservation::acquire(
                (source.len() as u64).checked_mul(3).ok_or_else(bad)?,
            )?)
        };
        let mut count = 0u64;
        if source.last() != Some(&b'\n') {
            return Err(bad());
        }
        for line in source.split_inclusive(|b| *b == b'\n') {
            yonik_engine::resource_index::RuleSetIndex::parse(line, kind, 1)?;
            count += 1;
        }
        if count != reference.records() {
            return Err(bad());
        }
        use yonik_engine::external_rules::{Format, Limits, compile_records};
        let format = match (reference.format(), kind) {
            (RuleSetFormat::XrayDomainsV1, ResourceKind::Domains) => Format::XrayDomains,
            (RuleSetFormat::XrayCidrsV1, ResourceKind::Cidrs) => Format::XrayCidrs,
            (RuleSetFormat::SingSourceV3, ResourceKind::Domains) => Format::SingDomains,
            (RuleSetFormat::SingSourceV3, ResourceKind::Cidrs) => Format::SingCidrs,
            _ => return Err(bad()),
        };
        let mut digest = DigestWriter {
            hash: Sha256::new(),
            bytes: 0,
        };
        compile_records(
            io::Cursor::new(source),
            &mut digest,
            format,
            reference.records(),
            Limits::default(),
        )?;
        if digest.bytes != reference.bytes()
            || digest.hash.finalize().as_slice() != reference.digest()
        {
            return Err(bad());
        }
        // Проверяет существующий native ciphertext до создания association.
        if reference.bytes() > SOURCE_CAP {
            return Err(bad());
        }
        let _native_reservation = if pre_reserved {
            None
        } else {
            Some(crate::resource_fd_receiver::Reservation::acquire(
                reference.bytes().checked_mul(2).ok_or_else(bad)?,
            )?)
        };
        read_resource(&self.inner, &d)?;
        let h = header(source, kind, reference.records());
        match open(&self.inner.root, &source_name(&d)) {
            Ok(_) => {
                let (old, body) = read_source(&self.inner, &d)?;
                if old.kind != kind || *body != source {
                    return Err(bad());
                }
                self.reserve(0, 0)?;
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        self.reserve(source.len() as u64 + HEADER as u64 + 40, 1)?;
        let mut nonce = [0u8; 24];
        getrandom::getrandom(&mut nonce).map_err(|_| bad())?;
        let cipher = XChaCha20Poly1305::new((&*self.inner.key).into());
        let encrypted = Zeroizing::new(
            cipher
                .encrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: source,
                        aad: &aad(&d, &h)?,
                    },
                )
                .map_err(|_| bad())?,
        );
        let mut envelope = Zeroizing::new(Vec::with_capacity(HEADER + 24 + encrypted.len()));
        envelope.extend_from_slice(&h);
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&encrypted);
        self.atomic_write(&source_name(&d), &envelope, false)
    }
}
struct DigestWriter {
    hash: Sha256,
    bytes: u64,
}
impl Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.hash.update(bytes);
        self.bytes += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub struct CanonicalSources {
    pub resources: Vec<(CanonicalDescriptor, File)>,
    _reservation: crate::resource_fd_receiver::Reservation,
}
impl RuntimeLease {
    /// Отсутствующий legacy proof — Unsupported, не выдуманный source из index.
    pub fn canonical_sources(&self) -> io::Result<CanonicalSources> {
        let mut total = 0u64;
        for d in &self.resources {
            let mut f = open(&self.inner.root, &source_name(d)).map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    io::Error::new(
                        io::ErrorKind::Unsupported,
                        "Требуется rebuild canonical source",
                    )
                } else {
                    e
                }
            })?;
            let mut h = [0u8; HEADER];
            f.read_exact(&mut h)?;
            total = total.checked_add(parse(&h)?.bytes).ok_or_else(bad)?;
        }
        let reservation = crate::resource_fd_receiver::Reservation::acquire(
            total.checked_mul(3).ok_or_else(bad)?,
        )?;
        let mut out = Vec::new();
        for d in &self.resources {
            let (descriptor, body) = read_source(&self.inner, d)?;
            let mut fd = File::from(fs::memfd_create(
                "yonik-canonical-source",
                fs::MemfdFlags::CLOEXEC | fs::MemfdFlags::ALLOW_SEALING,
            )?);
            fd.write_all(&body)?;
            fs::fcntl_add_seals(
                &fd,
                fs::SealFlags::SEAL
                    | fs::SealFlags::WRITE
                    | fs::SealFlags::GROW
                    | fs::SealFlags::SHRINK,
            )?;
            let readonly = File::open(format!("/proc/self/fd/{}", fd.as_raw_fd()))?;
            out.push((descriptor, readonly));
        }
        Ok(CanonicalSources {
            resources: out,
            _reservation: reservation,
        })
    }
}
