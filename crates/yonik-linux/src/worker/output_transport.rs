//! Frozen #204 tests используют общий carrier, без production registration.
use crate::output_transport::{OutputReaders, OutputWriters, ValidatedWriters, create};
use rustix::{
    fs::{OFlags, fcntl_getfl},
    io::{FdFlags, fcntl_getfd},
};
use std::io;
use tokio::net::unix::pipe::pipe;

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

mod golden_compat;
mod source_mutants;
