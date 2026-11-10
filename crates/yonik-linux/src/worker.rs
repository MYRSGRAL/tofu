//! Приватный протокол helper → worker: только sealed FD и типизированные метаданные.
use crate::{
    BinarySnapshot, Sha256,
    helper::{
        MAX_ARTIFACT_BYTES,
        memory::{ByteLease, config_budget, executable_budget},
    },
};
use rustix::{
    fs::{MemfdFlags, SealFlags, fcntl_add_seals, fcntl_get_seals, memfd_create},
    net::{
        RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, SendAncillaryBuffer,
        SendAncillaryMessage, SendFlags, recvmsg, sendmsg,
    },
};
use sha2::{Digest, Sha256 as Hasher};
use std::{
    fs::File,
    io::{IoSlice, IoSliceMut, Read, Seek, SeekFrom, Write},
    mem::MaybeUninit,
    net::Shutdown,
    os::{
        fd::AsFd,
        unix::{fs::FileExt, net::UnixStream, process::CommandExt},
    },
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use yonik_application::ConfigDigest;
use yonik_engine::{CompiledConfig, Engine};

const TIMEOUT: Duration = Duration::from_secs(5);
const HEADER_LEN: usize = 80;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerMode {
    Check,
    Run,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerStatus {
    Checked,
    Accepted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerAck {
    pub pid: u32,
    pub status: WorkerStatus,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerError {
    Protocol,
    Peer,
    Io,
    Limit,
    Engine,
    Timeout,
    Check,
    Exec,
}
impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Protocol => "Некорректный протокол worker",
            Self::Peer => "Недопустимый отправитель worker",
            Self::Io => "Ошибка канала worker",
            Self::Limit => "Превышен лимит worker",
            Self::Engine => "Несовместимое ядро worker",
            Self::Timeout => "Истёк срок worker",
            Self::Check => "Ядро отклонило проверку worker",
            Self::Exec => "Не удалось запустить worker",
        })
    }
}
impl std::error::Error for WorkerError {}
struct Header {
    mode: WorkerMode,
    engine: Engine,
    length: usize,
    binary_hash: [u8; 32],
    config_hash: [u8; 32],
}
impl Header {
    fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0; HEADER_LEN];
        b[..8].copy_from_slice(b"YNKWRK\0\x01");
        b[8] = match self.mode {
            WorkerMode::Check => 1,
            WorkerMode::Run => 2,
        };
        b[9] = match self.engine {
            Engine::Xray => 1,
            Engine::SingBox => 2,
        };
        b[12..16].copy_from_slice(&(self.length as u32).to_be_bytes());
        b[16..48].copy_from_slice(&self.binary_hash);
        b[48..80].copy_from_slice(&self.config_hash);
        b
    }
    fn decode(b: &[u8]) -> Result<Self, WorkerError> {
        if b.len() != HEADER_LEN || &b[..8] != b"YNKWRK\0\x01" || b[10..12] != [0, 0] {
            return Err(WorkerError::Protocol);
        }
        let mode = match b[8] {
            1 => WorkerMode::Check,
            2 => WorkerMode::Run,
            _ => return Err(WorkerError::Protocol),
        };
        let engine = match b[9] {
            1 => Engine::Xray,
            2 => Engine::SingBox,
            _ => return Err(WorkerError::Protocol),
        };
        let length =
            u32::from_be_bytes(b[12..16].try_into().map_err(|_| WorkerError::Protocol)?) as usize;
        if length == 0 || length > MAX_ARTIFACT_BYTES {
            return Err(WorkerError::Limit);
        }
        Ok(Self {
            mode,
            engine,
            length,
            binary_hash: b[16..48].try_into().map_err(|_| WorkerError::Protocol)?,
            config_hash: b[48..80].try_into().map_err(|_| WorkerError::Protocol)?,
        })
    }
}
const SEALS: SealFlags = SealFlags::WRITE
    .union(SealFlags::GROW)
    .union(SealFlags::SHRINK)
    .union(SealFlags::SEAL);
fn config_file() -> Result<File, WorkerError> {
    Ok(File::from(
        memfd_create(
            "yonik-config",
            MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
        )
        .map_err(|_| WorkerError::Io)?,
    ))
}
#[cfg(test)]
fn sealed_config(bytes: &[u8]) -> Result<File, WorkerError> {
    let mut file = config_file()?;
    file.write_all(bytes).map_err(|_| WorkerError::Io)?;
    file.seek(SeekFrom::Start(0)).map_err(|_| WorkerError::Io)?;
    fcntl_add_seals(&file, SEALS).map_err(|_| WorkerError::Io)?;
    Ok(file)
}
fn validate_file(file: &File, length: usize, expected: [u8; 32]) -> Result<(), WorkerError> {
    if !fcntl_get_seals(file)
        .map_err(|_| WorkerError::Protocol)?
        .contains(SEALS)
        || file.metadata().map_err(|_| WorkerError::Protocol)?.len() != length as u64
    {
        return Err(WorkerError::Protocol);
    }
    let mut hash = Hasher::new();
    let mut buffer = zeroize::Zeroizing::new([0; 65536]);
    let mut offset = 0;
    while offset < length {
        let n = file
            .read_at(&mut buffer[..65536.min(length - offset)], offset as u64)
            .map_err(|_| WorkerError::Protocol)?;
        if n == 0 {
            return Err(WorkerError::Protocol);
        }
        hash.update(&buffer[..n]);
        offset += n;
    }
    if <[u8; 32]>::from(hash.finalize()) != expected {
        return Err(WorkerError::Protocol);
    }
    Ok(())
}
struct BoundedConfig {
    file: File,
    hash: Hasher,
    length: usize,
}
impl BoundedConfig {
    fn new() -> Result<Self, WorkerError> {
        Ok(Self {
            file: config_file()?,
            hash: Hasher::new(),
            length: 0,
        })
    }
    fn finish(mut self) -> Result<(File, usize, [u8; 32]), WorkerError> {
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| WorkerError::Io)?;
        fcntl_add_seals(&self.file, SEALS).map_err(|_| WorkerError::Io)?;
        Ok((self.file, self.length, self.hash.finalize().into()))
    }
}
impl Write for BoundedConfig {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if b.len() > MAX_ARTIFACT_BYTES.saturating_sub(self.length) {
            return Err(std::io::Error::other("Лимит конфигурации"));
        }
        self.file.write_all(b)?;
        self.hash.update(b);
        self.length += b.len();
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
pub struct WorkerPayload {
    binary: BinarySnapshot,
    config: File,
    header: Header,
    _lease: ByteLease,
}
impl WorkerPayload {
    pub fn new(binary: BinarySnapshot, config: CompiledConfig) -> Result<Self, WorkerError> {
        if config.engine() != binary.engine {
            return Err(WorkerError::Engine);
        }
        let mut lease = config_budget()
            .reserve(MAX_ARTIFACT_BYTES)
            .map_err(|_| WorkerError::Limit)?;
        // Сериализация сразу в учтённый memfd: второй plaintext buffer не нужен.
        let mut bytes = BoundedConfig::new()?;
        config
            .write_sensitive(&mut bytes)
            .map_err(|_| WorkerError::Limit)?;
        let (config, length, config_hash) = bytes.finish()?;
        let header = Header {
            mode: WorkerMode::Check,
            engine: binary.engine,
            length,
            binary_hash: binary.trusted_hash(),
            config_hash,
        };
        validate_file(
            &binary.file,
            binary.file.metadata().map_err(|_| WorkerError::Io)?.len() as usize,
            header.binary_hash,
        )?;
        let retained = header
            .length
            .checked_mul(if binary.engine == Engine::SingBox {
                2
            } else {
                1
            })
            .ok_or(WorkerError::Limit)?;
        lease.resize(retained).map_err(|_| WorkerError::Limit)?;
        Ok(Self {
            binary,
            config,
            header,
            _lease: lease,
        })
    }
    pub fn digest(&self) -> ConfigDigest {
        ConfigDigest::new(self.header.config_hash)
    }
    pub fn retained_bytes(&self) -> usize {
        self._lease.bytes()
    }
    pub fn engine(&self) -> Engine {
        self.header.engine
    }
    pub fn send(&self, stream: &UnixStream, mode: WorkerMode) -> Result<WorkerAck, WorkerError> {
        let started = Instant::now();
        stream
            .set_read_timeout(Some(TIMEOUT))
            .map_err(|_| WorkerError::Io)?;
        stream
            .set_write_timeout(Some(TIMEOUT))
            .map_err(|_| WorkerError::Io)?;
        let mut header = self.header.encode();
        header[8] = match mode {
            WorkerMode::Check => 1,
            WorkerMode::Run => 2,
        };
        let fds = [self.binary.file.as_fd(), self.config.as_fd()];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        if !ancillary.push(SendAncillaryMessage::ScmRights(&fds)) {
            return Err(WorkerError::Protocol);
        }
        let n = sendmsg(
            stream,
            &[IoSlice::new(&header)],
            &mut ancillary,
            SendFlags::NOSIGNAL,
        )
        .map_err(|_| WorkerError::Io)?;
        let mut stream = stream;
        let mut remaining = &header[n..];
        while !remaining.is_empty() {
            stream
                .set_write_timeout(Some(
                    TIMEOUT
                        .checked_sub(started.elapsed())
                        .ok_or(WorkerError::Timeout)?,
                ))
                .map_err(|_| WorkerError::Io)?;
            let written = stream.write(remaining).map_err(|_| WorkerError::Io)?;
            if written == 0 {
                return Err(WorkerError::Io);
            }
            remaining = &remaining[written..];
        }
        stream
            .shutdown(Shutdown::Write)
            .map_err(|_| WorkerError::Io)?;
        stream
            .set_read_timeout(Some(
                TIMEOUT
                    .checked_sub(started.elapsed())
                    .ok_or(WorkerError::Timeout)?,
            ))
            .map_err(|_| WorkerError::Io)?;
        let mut ack = [0; 9];
        let mut offset = 0;
        while offset < ack.len() {
            stream
                .set_read_timeout(Some(
                    TIMEOUT
                        .checked_sub(started.elapsed())
                        .ok_or(WorkerError::Timeout)?,
                ))
                .map_err(|_| WorkerError::Io)?;
            let read = stream.read(&mut ack[offset..]).map_err(|error| {
                #[cfg(test)]
                eprintln!(
                    "Ожидание worker Ack: kind={:?}, errno={:?}, elapsed={:?}",
                    error.kind(),
                    error.raw_os_error(),
                    started.elapsed()
                );
                #[cfg(not(test))]
                let _ = error;
                WorkerError::Io
            })?;
            if read == 0 {
                return Err(WorkerError::Protocol);
            }
            offset += read;
        }
        if &ack[..4] != b"YNKA" {
            return Err(WorkerError::Protocol);
        }
        let pid = u32::from_be_bytes(ack[4..8].try_into().map_err(|_| WorkerError::Protocol)?);
        let status = match (ack[8], mode) {
            (1, WorkerMode::Check) => WorkerStatus::Checked,
            (2, WorkerMode::Run) => WorkerStatus::Accepted,
            _ => return Err(WorkerError::Protocol),
        };
        if pid == 0 {
            return Err(WorkerError::Protocol);
        }
        Ok(WorkerAck { pid, status })
    }
}
fn receive(stream: &UnixStream, mode: WorkerMode) -> Result<(BinarySnapshot, File), WorkerError> {
    if !rustix::net::sockopt::socket_peercred(stream)
        .map_err(|_| WorkerError::Peer)?
        .uid
        .is_root()
    {
        return Err(WorkerError::Peer);
    }
    stream
        .set_read_timeout(Some(TIMEOUT))
        .map_err(|_| WorkerError::Io)?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(|_| WorkerError::Io)?;
    let start = Instant::now();
    let mut bytes = Vec::with_capacity(HEADER_LEN);
    let mut fds = Vec::new();
    loop {
        stream
            .set_read_timeout(Some(
                TIMEOUT
                    .checked_sub(start.elapsed())
                    .ok_or(WorkerError::Timeout)?,
            ))
            .map_err(|_| WorkerError::Io)?;
        let mut part = [0; HEADER_LEN + 1];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(3))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut space);
        let msg = recvmsg(
            stream,
            &mut [IoSliceMut::new(&mut part)],
            &mut ancillary,
            RecvFlags::CMSG_CLOEXEC,
        )
        .map_err(|_| WorkerError::Io)?;
        if msg
            .flags
            .intersects(ReturnFlags::CTRUNC | ReturnFlags::TRUNC)
        {
            return Err(WorkerError::Protocol);
        }
        for message in ancillary.drain() {
            match message {
                RecvAncillaryMessage::ScmRights(rights) => fds.extend(rights),
                _ => return Err(WorkerError::Protocol),
            }
        }
        if fds.len() > 2 || bytes.len() + msg.bytes > HEADER_LEN {
            return Err(WorkerError::Protocol);
        }
        if msg.bytes == 0 {
            break;
        }
        bytes.extend_from_slice(&part[..msg.bytes]);
    }
    let header = Header::decode(&bytes)?;
    if header.mode != mode || fds.len() != 2 {
        return Err(WorkerError::Protocol);
    }
    let config = File::from(fds.pop().ok_or(WorkerError::Protocol)?);
    let mut source = File::from(fds.pop().ok_or(WorkerError::Protocol)?);
    let length = source.metadata().map_err(|_| WorkerError::Protocol)?.len();
    if length > BinarySnapshot::MAX_SIZE {
        return Err(WorkerError::Limit);
    }
    if !fcntl_get_seals(&source)
        .map_err(|_| WorkerError::Protocol)?
        .contains(SEALS | SealFlags::EXEC)
    {
        return Err(WorkerError::Protocol);
    }
    // load_budgeted одновременно копирует sealed ELF и проверяет закреплённый SHA-256.
    validate_file(&config, header.length, header.config_hash)?;
    source
        .seek(SeekFrom::Start(0))
        .map_err(|_| WorkerError::Io)?;
    let lease = executable_budget()
        .reserve(length as usize)
        .map_err(|_| WorkerError::Limit)?;
    let binary =
        BinarySnapshot::load_budgeted(source, Sha256(header.binary_hash), header.engine, lease)
            .map_err(|_| WorkerError::Protocol)?;
    Ok((binary, config))
}
struct WorkerConfig {
    file: File,
    _lease: Option<ByteLease>,
}
fn worker_owned_config(
    source: &File,
    budget: &crate::helper::memory::ByteBudget,
) -> Result<WorkerConfig, WorkerError> {
    let metadata = source.metadata().map_err(|_| WorkerError::Protocol)?;
    let length = usize::try_from(metadata.len()).map_err(|_| WorkerError::Limit)?;
    if !metadata.is_file()
        || length == 0
        || length > MAX_ARTIFACT_BYTES
        || !fcntl_get_seals(source)
            .map_err(|_| WorkerError::Protocol)?
            .contains(SEALS)
    {
        return Err(WorkerError::Protocol);
    }
    let lease = budget.reserve(length).map_err(|_| WorkerError::Limit)?;
    let mut target = BoundedConfig::new()?;
    rustix::fs::fchmod(
        &target.file,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .map_err(|_| WorkerError::Io)?;
    let mut buffer = zeroize::Zeroizing::new([0u8; 65536]);
    let mut offset = 0;
    while offset < length {
        let n = source
            .read_at(&mut buffer[..65536.min(length - offset)], offset as u64)
            .map_err(|_| WorkerError::Io)?;
        if n == 0 {
            return Err(WorkerError::Protocol);
        }
        target
            .write_all(&buffer[..n])
            .map_err(|_| WorkerError::Io)?;
        offset += n;
    }
    let (file, copied, hash) = target.finish()?;
    if copied != length {
        return Err(WorkerError::Protocol);
    }
    validate_file(source, length, hash)?;
    validate_file(&file, length, hash)?;
    Ok(WorkerConfig {
        file,
        _lease: Some(lease),
    })
}
fn command(binary: &BinarySnapshot) -> Command {
    use std::os::fd::AsRawFd;
    let mut command = Command::new(format!("/proc/self/fd/{}", binary.file.as_raw_fd()));
    command
        .env_clear()
        .env("LANG", "C")
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}
fn configured(binary: &BinarySnapshot, config: &File, check: bool) -> Result<Command, WorkerError> {
    let mut command = command(binary);
    match binary.engine {
        Engine::Xray => {
            command.arg("run");
            if check {
                command.arg("-test");
            }
            command.args(["-c", "stdin:"]);
        }
        Engine::SingBox => {
            command
                .arg(if check { "check" } else { "run" })
                .args(["-c", "/dev/stdin"]);
        }
    }
    let mut input = config.try_clone().map_err(|_| WorkerError::Io)?;
    input
        .seek(SeekFrom::Start(0))
        .map_err(|_| WorkerError::Io)?;
    command.stdin(Stdio::from(input));
    Ok(command)
}
async fn check(binary: &BinarySnapshot, config: &File) -> Result<(), WorkerError> {
    use tokio::io::AsyncReadExt;
    let operation = async {
        let mut version = command(binary);
        version.arg("version").stdout(Stdio::piped());
        let mut version = tokio::process::Command::from(version);
        version.kill_on_drop(true);
        let mut child = version.spawn().map_err(|_| WorkerError::Check)?;
        let output = child.stdout.take().ok_or(WorkerError::Check)?;
        let mut bytes = Vec::new();
        let mut output = output.take(4097);
        let (read, status) = tokio::join!(output.read_to_end(&mut bytes), child.wait());
        read.map_err(|_| WorkerError::Check)?;
        if !status.map_err(|_| WorkerError::Check)?.success() || bytes.len() > 4096 {
            return Err(WorkerError::Check);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| WorkerError::Check)?;
        let prefix = match binary.engine {
            Engine::Xray => "Xray 26.10.31",
            Engine::SingBox => "sing-box version 1.14.4",
        };
        let line = text.lines().next().ok_or(WorkerError::Check)?;
        if line != prefix
            && !line
                .strip_prefix(prefix)
                .is_some_and(|tail| tail.starts_with(' '))
        {
            return Err(WorkerError::Check);
        }
        let mut check = tokio::process::Command::from(configured(binary, config, true)?);
        check.kill_on_drop(true);
        if !check
            .status()
            .await
            .map_err(|_| WorkerError::Check)?
            .success()
        {
            return Err(WorkerError::Check);
        }
        Ok(())
    };
    tokio::time::timeout(TIMEOUT, operation)
        .await
        .map_err(|_| WorkerError::Timeout)?
}
/// stdin должен быть listening socket, установленным StandardInput=socket.
pub async fn serve(mode: WorkerMode) -> Result<(), WorkerError> {
    let fd = rustix::net::accept_with(std::io::stdin().as_fd(), rustix::net::SocketFlags::CLOEXEC)
        .map_err(|_| WorkerError::Io)?;
    let stream = UnixStream::from(fd);
    serve_stream(stream, mode).await
}
async fn serve_stream(mut stream: UnixStream, mode: WorkerMode) -> Result<(), WorkerError> {
    let (binary, config) = receive(&stream, mode)?;
    // Исходный FD уже проверен receive: type/seals/declared size/SHA-256.
    // Helper удерживает отдельный global reserve до подтверждённого пустого cgroup.
    let config = if binary.engine == Engine::SingBox {
        worker_owned_config(&config, config_budget())?
    } else {
        WorkerConfig {
            file: config,
            _lease: None,
        }
    };
    if mode == WorkerMode::Check {
        check(&binary, &config.file).await?;
    }
    let mut ack = [0; 9];
    ack[..4].copy_from_slice(b"YNKA");
    ack[4..8].copy_from_slice(&std::process::id().to_be_bytes());
    ack[8] = match mode {
        WorkerMode::Check => 1,
        WorkerMode::Run => 2,
    };
    stream.write_all(&ack).map_err(|_| WorkerError::Io)?;
    if mode == WorkerMode::Run {
        drop(stream);
        let error = configured(&binary, &config.file, false)?.exec();
        let _ = error;
        return Err(WorkerError::Exec);
    }
    Ok(())
}
#[cfg(test)]
mod tests;
