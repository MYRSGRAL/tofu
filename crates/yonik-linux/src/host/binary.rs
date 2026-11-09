use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{OpenOptionsExt, PermissionsExt},
    },
    path::Path,
    str::FromStr,
};

use rustix::fs::{MemfdFlags, Mode, OFlags, SealFlags, fchmod, fcntl_add_seals, memfd_create};
use sha2::{Digest, Sha256 as Hasher};
use tokio::process::Command;
use yonik_engine::Engine;

use super::HostError;

const MAX_BINARY: u64 = 128 * 1024 * 1024;

#[derive(Clone, Copy)]
pub struct Sha256(pub(crate) [u8; 32]);
impl std::fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sha256(..)")
    }
}
impl FromStr for Sha256 {
    type Err = HostError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(HostError::BinaryHash);
        }
        let mut bytes = [0; 32];
        for (output, pair) in bytes.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
            let hex = |b: u8| {
                if b.is_ascii_digit() {
                    b - b'0'
                } else {
                    b.to_ascii_lowercase() - b'a' + 10
                }
            };
            *output = hex(pair[0]) * 16 + hex(pair[1]);
        }
        Ok(Self(bytes))
    }
}

/// Один sealed inode используется для version/config-check/start. Исходный путь
/// после копирования больше не читается; ни пути, ни содержимое не попадают в Debug.
pub struct BinarySnapshot {
    pub(crate) file: File,
    pub(crate) engine: Engine,
}
impl std::fmt::Debug for BinarySnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BinarySnapshot(<скрыто>)")
    }
}
impl BinarySnapshot {
    pub fn load(path: &Path, expected: Sha256, engine: Engine) -> Result<Self, HostError> {
        let mut source = OpenOptions::new()
            .read(true)
            .custom_flags(OFlags::NONBLOCK.bits() as i32)
            .open(path)
            .map_err(|_| HostError::BinaryIo)?;
        let metadata = source.metadata().map_err(|_| HostError::BinaryIo)?;
        if !metadata.is_file()
            || metadata.len() < 4
            || metadata.len() > MAX_BINARY
            || metadata.permissions().mode() & 0o111 == 0
        {
            return Err(HostError::InvalidBinary);
        }
        let descriptor = memfd_create(
            "yonik-engine",
            MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING | MemfdFlags::EXEC,
        )
        .map_err(|_| HostError::Snapshot)?;
        let mut file = File::from(descriptor);
        let mut hasher = Hasher::new();
        let mut buffer = [0; 64 * 1024];
        let mut size = 0_u64;
        loop {
            let read = source.read(&mut buffer).map_err(|_| HostError::BinaryIo)?;
            if read == 0 {
                break;
            }
            if size == 0 && (read < 4 || &buffer[..4] != b"\x7fELF") {
                return Err(HostError::InvalidBinary);
            }
            size += read as u64;
            if size > MAX_BINARY {
                return Err(HostError::InvalidBinary);
            }
            hasher.update(&buffer[..read]);
            file.write_all(&buffer[..read])
                .map_err(|_| HostError::Snapshot)?;
        }
        let actual: [u8; 32] = hasher.finalize().into();
        if actual != expected.0 {
            return Err(HostError::BinaryHash);
        }
        fchmod(&file, Mode::RUSR | Mode::XUSR).map_err(|_| HostError::Snapshot)?;
        fcntl_add_seals(
            &file,
            SealFlags::WRITE
                | SealFlags::SHRINK
                | SealFlags::GROW
                | SealFlags::EXEC
                | SealFlags::SEAL,
        )
        .map_err(|_| HostError::Snapshot)?;
        Ok(Self { file, engine })
    }
    pub(crate) fn command(&self) -> Command {
        let mut command = Command::new(format!("/proc/self/fd/{}", self.file.as_raw_fd()));
        command
            .env_clear()
            .env("LANG", "C")
            .current_dir("/")
            .process_group(0)
            .kill_on_drop(true);
        for key in ["SSL_CERT_FILE", "SSL_CERT_DIR"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
    }
}
