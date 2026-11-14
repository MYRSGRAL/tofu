//! Только тестовая основа FR16: две фиксированные роли без metadata authority.
//! Factory требует явно введённый Tokio runtime с I/O driver. Wire, EventLog
//! binding и production caller остаются отдельным gate; ABI и протокол не меняются.
use rustix::{
    fs::{FileType, OFlags, fcntl_getfl, fstat, fstatfs},
    io::{FdFlags, fcntl_getfd},
};
use std::{io, os::fd::OwnedFd, process::Command};
use tokio::net::unix::pipe::{Receiver, pipe};

struct OutputReaders {
    stdout: Receiver,
    stderr: Receiver,
}

struct OutputWriters {
    roles: Vec<OwnedFd>,
}

struct ValidatedWriters {
    stdout: OwnedFd,
    stderr: OwnedFd,
}

fn create() -> io::Result<(OutputReaders, OutputWriters)> {
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
    fn validate(self) -> io::Result<ValidatedWriters> {
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "ожидались два разных blocking CLOEXEC FIFO writer",
            )
        };
        let [stdout, stderr]: [OwnedFd; 2] = self.roles.try_into().map_err(|_| invalid())?;
        let identity = |fd: &OwnedFd| -> io::Result<_> {
            let stat = fstat(fd)?;
            let flags = fcntl_getfl(fd)?;
            // PIPEFS_MAGIC фиксирован ABI Linux; обычный named FIFO не является pipe.
            if fstatfs(fd)?.f_type as u64 != 0x5049_5045
                || FileType::from_raw_mode(stat.st_mode) != FileType::Fifo
                || flags & OFlags::ACCMODE != OFlags::WRONLY
                || flags.contains(OFlags::NONBLOCK)
                || !fcntl_getfd(fd)?.contains(FdFlags::CLOEXEC)
            {
                return Err(invalid());
            }
            Ok((stat.st_dev, stat.st_ino))
        };
        if identity(&stdout)? == identity(&stderr)? {
            return Err(invalid());
        }
        Ok(ValidatedWriters { stdout, stderr })
    }
}

impl ValidatedWriters {
    // Метод приватен даже внутри test-only модуля: не capability общего executor.
    fn attach(self, command: &mut Command) {
        command.stdout(self.stdout).stderr(self.stderr);
    }
}

#[cfg(test)]
mod tests {
    use super::{FdFlags, OFlags, OutputWriters, create, fcntl_getfd, fcntl_getfl, io, pipe};
    use std::process::Command;
    use std::{fs::File, os::fd::AsFd};
    use tokio::io::AsyncReadExt;

    fn runtime() -> io::Result<tokio::runtime::Runtime> {
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
    }

    #[test]
    fn fixed_flags_identity_and_stdio_eof() -> io::Result<()> {
        let runtime = runtime()?;
        let _entered = runtime.enter();
        let (mut readers, writers) = create()?;
        for reader in [&readers.stdout, &readers.stderr] {
            assert!(fcntl_getfl(reader)?.contains(OFlags::NONBLOCK));
            assert!(fcntl_getfd(reader)?.contains(FdFlags::CLOEXEC));
        }
        let validated = writers.validate()?;
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf stdout; printf stderr >&2"]);
        validated.attach(&mut command);
        let mut child = command.spawn()?;
        drop(command); // Родитель не удерживает копии writer в Command.
        assert!(child.wait()?.success());
        runtime.block_on(async {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            readers.stdout.read_to_end(&mut stdout).await?;
            readers.stderr.read_to_end(&mut stderr).await?;
            assert_eq!(stdout, b"stdout");
            assert_eq!(stderr, b"stderr");
            Ok(())
        })
    }

    #[test]
    fn rejects_invalid_descriptors_and_role_counts() -> io::Result<()> {
        let runtime = runtime()?;
        let _entered = runtime.enter();
        let reject = |roles| assert!(OutputWriters { roles }.validate().is_err());
        reject(vec![]);
        let (_readers, writers) = create()?;
        let mut roles = writers.roles;
        let duplicate = roles[0].as_fd().try_clone_to_owned()?;
        reject(vec![roles.remove(0), duplicate]);
        reject(roles);
        let (_readers, writers) = create()?;
        let mut roles = writers.roles;
        roles.push(File::open("/dev/null")?.into());
        reject(roles);
        for invalid in [
            File::open("Cargo.toml")?.into(),
            File::open(".")?.into(),
            std::os::unix::net::UnixStream::pair()?.0.into(),
        ] {
            let (_readers, writers) = create()?;
            let mut roles = writers.roles;
            roles[0] = invalid;
            reject(roles);
        }
        let (sender, receiver) = pipe()?;
        let (_readers, writers) = create()?;
        let mut roles = writers.roles;
        roles[0] = receiver.into_blocking_fd()?;
        reject(roles);
        let (_readers, writers) = create()?;
        let mut roles = writers.roles;
        roles[0] = sender.into_blocking_fd()?;
        rustix::fs::fcntl_setfl(&roles[0], OFlags::NONBLOCK)?;
        reject(roles);
        let (_readers, writers) = create()?;
        let mut roles = writers.roles;
        use std::os::fd::AsRawFd;
        roles[0] = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!("/proc/self/fd/{}", roles[0].as_raw_fd()))?
            .into();
        reject(roles);
        let (_readers, writers) = create()?;
        rustix::io::fcntl_setfd(&writers.roles[0], FdFlags::empty())?;
        reject(writers.roles);
        Ok(())
    }

    #[test]
    fn failure_and_drop_close_owned_writers() -> io::Result<()> {
        let runtime = runtime()?;
        let _entered = runtime.enter();
        let (readers, writers) = create()?;
        let mut roles = writers.roles;
        roles.push(File::open("/dev/null")?.into());
        assert!(OutputWriters { roles }.validate().is_err());
        let mut byte = [0];
        assert_eq!(rustix::io::read(&readers.stdout, &mut byte)?, 0);
        assert_eq!(rustix::io::read(&readers.stderr, &mut byte)?, 0);
        let (readers, writers) = create()?;
        drop(writers.validate()?);
        assert_eq!(rustix::io::read(&readers.stdout, &mut byte)?, 0);
        assert_eq!(rustix::io::read(&readers.stderr, &mut byte)?, 0);
        Ok(())
    }
}

mod compatibility;
