//! Потоковый preflight canonical FD. Не выдаёт права Prepare/Apply или release.
use rustix::fs::{OFlags, SealFlags, fcntl_get_seals, fcntl_getfl};
use sha2::{Digest, Sha256};
use std::{fs::File, io, os::unix::fs::FileExt};
use yonik_engine::{resource_bundle::ResourceKind, resource_index::RuleSetIndex};

const PHYSICAL_LIMIT: u64 = 64 * 1024 * 1024;
const MAX_RECORDS: u64 = 1_500_000;
const MAX_LINE: usize = 300;

#[derive(Clone, Debug)]
pub struct CanonicalDescriptor {
    pub digest: [u8; 32],
    pub bytes: u64,
    pub records: u64,
    pub kind: ResourceKind,
    pub format_version: u32,
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Некорректный canonical resource FD",
    )
}
fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Проверка ресурсов отменена")
}
/// Проверяет весь ordered batch до чтения первого FD. claimed_work учитывает
/// дальнейшие native buffers/indices; это preflight, не process-wide reservation.
/// Три physical copies — входной memfd, owned snapshot и buffer компилятора.
/// Caller обязан отдельно удерживать admission reservation до worker empty.
pub fn verify(
    manifest: &[CanonicalDescriptor],
    files: &[File],
    claimed_work: u64,
    cancel: &impl Fn() -> bool,
) -> io::Result<()> {
    if manifest.is_empty() || manifest.len() > 16 || manifest.len() != files.len() {
        return Err(invalid());
    }
    let mut physical = claimed_work.checked_add(8192).ok_or_else(invalid)?;
    let mut records = 0u64;
    let mut inodes = std::collections::HashSet::new();
    for (i, (d, file)) in manifest.iter().zip(files).enumerate() {
        if cancel() {
            return Err(cancelled());
        }
        records = records.checked_add(d.records).ok_or_else(invalid)?;
        physical = physical
            .checked_add(d.bytes.checked_mul(3).ok_or_else(invalid)?)
            .ok_or_else(invalid)?;
        if d.format_version != 1
            || d.bytes == 0
            || d.records == 0
            || records > MAX_RECORDS
            || physical > PHYSICAL_LIMIT
            || manifest[..i].iter().any(|other| other.digest == d.digest)
        {
            return Err(invalid());
        }
        let metadata = file.metadata()?;
        use std::os::unix::fs::MetadataExt;
        if !inodes.insert((metadata.dev(), metadata.ino())) {
            return Err(invalid());
        }
        let seals = SealFlags::WRITE | SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL;
        if !metadata.is_file()
            || metadata.len() != d.bytes
            || fcntl_getfl(file)? & OFlags::ACCMODE != OFlags::RDONLY
            || !fcntl_get_seals(file)?.contains(seals)
        {
            return Err(invalid());
        }
        // F_GET_SEALS + regular file отвергают disk files, sockets и pipes.
        // На Linux seals также доступны shmem O_TMPFILE: принимается только memfd.
        use std::os::fd::AsRawFd;
        let target = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
        if !target
            .as_os_str()
            .as_encoded_bytes()
            .starts_with(b"/memfd:")
        {
            return Err(invalid());
        }
    }
    for (d, file) in manifest.iter().zip(files) {
        let mut hash = Sha256::new();
        let mut block = [0u8; 8192];
        let mut line = [0u8; MAX_LINE];
        let mut used = 0;
        let mut count = 0u64;
        let mut offset = 0;
        while offset < d.bytes {
            if cancel() {
                return Err(cancelled());
            }
            let length = (d.bytes - offset).min(block.len() as u64) as usize;
            let n = file.read_at(&mut block[..length], offset)?;
            if n == 0 {
                return Err(invalid());
            }
            hash.update(&block[..n]);
            for &byte in &block[..n] {
                if used == MAX_LINE {
                    return Err(invalid());
                }
                line[used] = byte;
                used += 1;
                if byte == b'\n' {
                    count += 1;
                    if count > d.records {
                        return Err(invalid());
                    }
                    // Закрытый grammar из trusted evaluator: ни regex, ни JSON,
                    // ни protobuf unknown fields/compression не интерпретируются.
                    RuleSetIndex::parse(&line[..used], d.kind, 1)?;
                    used = 0;
                }
            }
            offset += n as u64;
        }
        if used != 0 || count != d.records || hash.finalize().as_slice() != d.digest {
            return Err(invalid());
        }
    }
    if cancel() {
        return Err(cancelled());
    }
    Ok(())
}

/// Не связано с config/executable pool: физический бюджет только ресурсов.
static RESERVED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub(crate) struct Reservation(u64);
impl Reservation {
    pub(crate) fn acquire(bytes: u64) -> io::Result<Self> {
        use std::sync::atomic::Ordering;
        RESERVED
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|next| *next <= PHYSICAL_LIMIT)
            })
            .map_err(|_| invalid())?;
        Ok(Self(bytes))
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        RESERVED.fetch_sub(self.0, std::sync::atomic::Ordering::AcqRel);
    }
}
/// Владение только pre-admission: этот тип нельзя считать runtime lease.
/// Для передачи worker необходим отдельный custody owner с quarantine semantics.
pub struct RebuiltBatch {
    pub artifacts: Vec<File>,
    _sources: Vec<File>,
    indices: Vec<RuleSetIndex>,
    manifest: Vec<yonik_engine::resource_bundle::ExternalRef>,
    _reservation: Reservation,
}
struct Positioned<'a, C> {
    cancel: &'a C,
    file: &'a File,
    position: u64,
}
impl<C: Fn() -> bool> io::Read for Positioned<'_, C> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if (self.cancel)() {
            return Err(cancelled());
        }
        let length = bytes.len().min(8192);
        let n = self.file.read_at(&mut bytes[..length], self.position)?;
        self.position += n as u64;
        Ok(n)
    }
}
impl<C> io::Seek for Positioned<'_, C> {
    fn seek(&mut self, from: io::SeekFrom) -> io::Result<u64> {
        let position = match from {
            io::SeekFrom::Start(n) => Some(n),
            io::SeekFrom::Current(n) => self.position.checked_add_signed(n),
            io::SeekFrom::End(n) => self.file.metadata()?.len().checked_add_signed(n),
        }
        .ok_or_else(invalid)?;
        self.position = position;
        Ok(position)
    }
}
struct BoundedWriter<'a, C> {
    file: &'a mut File,
    hash: Sha256,
    remaining: u64,
    cancel: &'a C,
}
impl<C: Fn() -> bool> io::Write for BoundedWriter<'_, C> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if (self.cancel)() {
            return Err(cancelled());
        }
        if bytes.len() as u64 > self.remaining {
            return Err(invalid());
        }
        let n = std::io::Write::write(self.file, bytes)?;
        self.hash.update(&bytes[..n]);
        self.remaining -= n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        std::io::Write::flush(self.file)
    }
}
/// Перестраивает native bytes только из проверенного canonical input. Expected
/// native digest не заменяет grammar validation. Ни caller paths, ни JSON rules.
pub fn rebuild(
    sources: Vec<File>,
    descriptors: &[CanonicalDescriptor],
    expected: &[yonik_engine::resource_bundle::ExternalRef],
    cancel: &impl Fn() -> bool,
) -> io::Result<RebuiltBatch> {
    use rustix::fs::{MemfdFlags, fcntl_add_seals, memfd_create};
    use yonik_engine::{
        Engine,
        external_rules::{Format, Limits, compile_records},
    };
    if descriptors.len() != expected.len() {
        return Err(invalid());
    }
    let mut work = 8192u64;
    let mut total = 0u64;
    for (d, e) in descriptors.iter().zip(expected) {
        e.validate()?;
        if d.kind != e.kind || d.records != e.records {
            return Err(invalid());
        }
        work = work
            .checked_add(
                e.bytes
                    .checked_mul(if e.engine == yonik_engine::Engine::Xray {
                        3
                    } else {
                        2
                    })
                    .ok_or_else(invalid)?,
            )
            .and_then(|n| n.checked_add(d.records.checked_mul(128)?))
            .ok_or_else(invalid)?;
        total = total
            .checked_add(d.bytes.checked_mul(3).ok_or_else(invalid)?)
            .ok_or_else(invalid)?;
    }
    let reservation = Reservation::acquire(total.checked_add(work).ok_or_else(invalid)?)?;
    verify(descriptors, &sources, work, cancel)?;
    let mut artifacts = Vec::with_capacity(expected.len());
    let mut indices = Vec::with_capacity(expected.len());
    for ((source, d), e) in sources.iter().zip(descriptors).zip(expected) {
        if cancel() {
            return Err(cancelled());
        }
        let mut plaintext = zeroize::Zeroizing::new(vec![0; d.bytes as usize]);
        for (i, block) in plaintext.chunks_mut(8192).enumerate() {
            if cancel() {
                return Err(cancelled());
            }
            source.read_exact_at(block, (i * 8192) as u64)?;
        }
        indices.push(RuleSetIndex::parse_cancellable(
            &plaintext, d.kind, d.records, cancel,
        )?);
        drop(plaintext);
        let format = match (e.engine, d.kind) {
            (Engine::Xray, ResourceKind::Domains) => Format::XrayDomains,
            (Engine::Xray, ResourceKind::Cidrs) => Format::XrayCidrs,
            (Engine::SingBox, ResourceKind::Domains) => Format::SingDomains,
            (Engine::SingBox, ResourceKind::Cidrs) => Format::SingCidrs,
        };
        let mut file = File::from(memfd_create(
            format!("yonik-{}", e.tag()),
            MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
        )?);
        let mut writer = BoundedWriter {
            file: &mut file,
            hash: Sha256::new(),
            remaining: e.bytes,
            cancel,
        };
        compile_records(
            Positioned {
                file: source,
                position: 0,
                cancel,
            },
            &mut writer,
            format,
            d.records,
            Limits {
                records: d.records,
                source_bytes: d.bytes,
                artifact_bytes: e.bytes,
            },
        )?;
        if writer.remaining != 0 || writer.hash.finalize().as_slice() != e.digest {
            return Err(invalid());
        }
        fcntl_add_seals(
            &file,
            SealFlags::WRITE | SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL,
        )?;
        use std::os::fd::AsRawFd;
        artifacts.push(File::open(format!("/proc/self/fd/{}", file.as_raw_fd()))?);
    }
    if cancel() {
        return Err(cancelled());
    }
    Ok(RebuiltBatch {
        artifacts,
        _sources: sources,
        indices,
        manifest: expected.to_vec(),
        _reservation: reservation,
    })
}

impl yonik_engine::resource_bundle::VerifiedCatalog for RebuiltBatch {
    fn manifest(&self) -> &[yonik_engine::resource_bundle::ExternalRef] {
        &self.manifest
    }
    fn matches_domain(
        &self,
        index: usize,
        name: &yonik_engine::resource_bundle::DomainName,
    ) -> bool {
        self.indices
            .get(index)
            .is_some_and(|resource| resource.matches_domain(name))
    }
    fn matches_ip(&self, index: usize, ip: std::net::IpAddr) -> bool {
        self.indices
            .get(index)
            .is_some_and(|resource| resource.matches_ip(ip))
    }
}

#[cfg(test)]
pub(crate) fn physical_reserved_bytes() -> u64 {
    RESERVED.load(std::sync::atomic::Ordering::Acquire)
}
