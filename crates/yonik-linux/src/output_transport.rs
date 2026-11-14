//! Закрытый output transport legacy Apply: две роли без дополнительной authority.
//! FIFO импорт проверяет PIPEFS, flags и identity до owned duplication.
use rustix::{
    fs::{FileType, OFlags, fcntl_getfl, fstat, fstatfs},
    io::{FdFlags, fcntl_getfd},
};
use std::process::Command;
use std::{io, os::fd::OwnedFd};
use tokio::net::unix::pipe::{Receiver, pipe};

pub(crate) struct OutputReaders {
    pub(crate) stdout: Receiver,
    pub(crate) stderr: Receiver,
}

pub(crate) struct OutputWriters {
    pub(crate) roles: Vec<OwnedFd>,
}

pub struct ValidatedWriters {
    pub(crate) stdout: OwnedFd,
    pub(crate) stderr: OwnedFd,
}

pub(crate) fn create() -> io::Result<(OutputReaders, OutputWriters)> {
    let (stdout, stdout_reader) = pipe()?;
    let (stderr, stderr_reader) = pipe()?;
    Ok((
        OutputReaders {
            stdout: stdout_reader,
            stderr: stderr_reader,
        },
        OutputWriters {
            roles: vec![stdout.into_blocking_fd()?, stderr.into_blocking_fd()?],
        },
    ))
}

impl OutputWriters {
    pub(crate) fn validate(self) -> io::Result<ValidatedWriters> {
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "ожидались два разных blocking CLOEXEC FIFO writer",
            )
        };
        let [stdout, stderr]: [OwnedFd; 2] = self.roles.try_into().map_err(|_| invalid())?;
        use std::os::fd::AsFd;
        validate_pair([stdout.as_fd(), stderr.as_fd()])?;
        Ok(ValidatedWriters { stdout, stderr })
    }
}

// Проверяет исходные borrowed FD до любой owned duplication (не чинит CLOEXEC).
fn validate_pair(fds: [std::os::fd::BorrowedFd<'_>; 2]) -> io::Result<()> {
    let identity = |fd| -> io::Result<_> {
        let stat = fstat(fd)?;
        let flags = fcntl_getfl(fd)?;
        if fstatfs(fd)?.f_type as u64 != 0x5049_5045
            || FileType::from_raw_mode(stat.st_mode) != FileType::Fifo
            || flags & OFlags::ACCMODE != OFlags::WRONLY
            || flags.contains(OFlags::NONBLOCK)
            || !fcntl_getfd(fd)?.contains(FdFlags::CLOEXEC)
        {
            return Err(invalid_message());
        }
        Ok((stat.st_dev, stat.st_ino))
    };
    if identity(fds[0])? == identity(fds[1])? {
        return Err(invalid_message());
    }
    Ok(())
}

impl ValidatedWriters {
    // Назначает только две фиксированные роли; stdin/config остаются у worker.
    pub(crate) fn attach(self, command: &mut Command) {
        command.stdout(self.stdout).stderr(self.stderr);
    }
}

const OUTPUT_METHOD: &str = "ApplyOutputV1";
const OUTPUT_PATH: &str = "/cloud/highpriority/yonik/Network";
const OUTPUT_INTERFACE: &str = "cloud.highpriority.yonik.Network1";
const MAX_OUTPUT_JSON: usize = 4096;
const MAX_OUTPUT_BODY: usize = MAX_OUTPUT_JSON + 64;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyOutputJson {
    version: u32,
    handle: [u8; 32],
}

fn invalid_message() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "неверное фиксированное output сообщение",
    )
}

/// Только owned carrier, не claim/authorization и не capability запуска.
pub(crate) struct DecodedOutput {
    pub(crate) handle: [u8; 32],
    pub(crate) writers: ValidatedWriters,
}

/// Сериализация данных без Connection/send. Исходная пара закрывается при возврате.
#[cfg(test)]
pub(crate) fn encode(handle: [u8; 32], writers: ValidatedWriters) -> io::Result<zbus::Message> {
    use std::os::fd::AsFd;
    let json = serde_json::to_string(&ApplyOutputJson { version: 1, handle })
        .map_err(|_| invalid_message())?;
    let fds = [
        zbus::zvariant::Fd::from(writers.stdout.as_fd()),
        zbus::zvariant::Fd::from(writers.stderr.as_fd()),
    ];
    zbus::Message::method(OUTPUT_PATH, OUTPUT_METHOD)
        .and_then(|builder| builder.interface(OUTPUT_INTERFACE))
        .and_then(|builder| builder.build(&(json, fds.as_slice())))
        .map_err(|_| invalid_message())
}

/// Владеет message до завершения проверки: ошибки закрывают весь принятый набор.
/// Копии Message вне этого вызова не являются собственностью decoder.
pub(crate) fn decode(message: zbus::Message) -> io::Result<DecodedOutput> {
    use std::os::fd::{AsFd, AsRawFd};
    let header = message.header();
    let body = message.body();
    if message.message_type() != zbus::message::Type::MethodCall
        || header.path().map(|v| v.as_str()) != Some(OUTPUT_PATH)
        || header.interface().map(|v| v.as_str()) != Some(OUTPUT_INTERFACE)
        || header.member().map(|v| v.as_str()) != Some(OUTPUT_METHOD)
        || body.len() > MAX_OUTPUT_BODY
        || body.signature().as_ref().map(|s| s.as_str()) != Some("sah")
        || header.unix_fds() != Some(2)
        || message.data().fds().len() != 2
    {
        return Err(invalid_message());
    }
    let ((json, fds), consumed): ((&str, Vec<zbus::zvariant::Fd<'_>>), usize) = body
        .data()
        .deserialize_for_dynamic_signature("sah")
        .map_err(|_| invalid_message())?;
    if consumed != body.len() {
        return Err(invalid_message());
    }
    if fds.len() != 2
        || fds[0].as_raw_fd() == fds[1].as_raw_fd()
        || json.len() > MAX_OUTPUT_JSON
        || !json.trim_start().starts_with('{')
    {
        return Err(invalid_message());
    }
    let request: ApplyOutputJson = serde_json::from_str(json).map_err(|_| invalid_message())?;
    if request.version != 1 {
        return Err(invalid_message());
    }
    validate_pair([fds[0].as_fd(), fds[1].as_fd()])?;
    let roles = fds
        .iter()
        .map(|fd| fd.as_fd().try_clone_to_owned())
        .collect::<io::Result<Vec<_>>>()?;
    let writers = OutputWriters { roles }.validate()?;
    Ok(DecodedOutput {
        handle: request.handle,
        writers,
    })
}

#[cfg(test)]
mod carrier_tests;
