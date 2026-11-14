//! Только собственные pipes и zbus DATA; нет bus, services или exec.
use super::*;
use std::os::fd::{AsFd, AsRawFd};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn runtime() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
}
fn json() -> String {
    serde_json::to_string(&ApplyOutputJson {
        version: 1,
        handle: [7; 32],
    })
    .expect("фиксированный DTO сериализуем")
}
fn raw_message(json: &str, fds: &[zbus::zvariant::Fd<'_>]) -> zbus::Message {
    zbus::Message::method(OUTPUT_PATH, OUTPUT_METHOD)
        .expect("константы")
        .interface(OUTPUT_INTERFACE)
        .expect("константа")
        .build(&(json, fds))
        .expect("DATA build")
}
fn eof(readers: &OutputReaders) -> io::Result<()> {
    for fd in [&readers.stdout, &readers.stderr] {
        assert_eq!(
            rustix::io::read(fd, &mut [0])?,
            0,
            "все owned writers закрыты"
        );
    }
    Ok(())
}

#[test]
fn fixed_sah_roundtrip_and_sender_copies_close() -> io::Result<()> {
    let rt = runtime()?;
    let _entered = rt.enter();
    let (mut readers, writers) = create()?;
    let message = encode([7; 32], writers.validate()?)?;
    assert_eq!(
        message.body().signature().as_ref().map(|s| s.as_str()),
        Some("sah")
    );
    assert_eq!(message.header().unix_fds(), Some(2));
    let output = decode(message)?;
    assert_eq!(output.handle, [7; 32]);
    rustix::io::write(&output.writers.stdout, b"out")?;
    rustix::io::write(&output.writers.stderr, b"err")?;
    rt.block_on(async {
        let mut out = [0; 3];
        let mut err = [0; 3];
        readers.stdout.read_exact(&mut out).await?;
        readers.stderr.read_exact(&mut err).await?;
        assert_eq!(&out, b"out");
        assert_eq!(&err, b"err");
        io::Result::Ok(())
    })?;
    drop(output);
    eof(&readers)
}

#[test]
fn json_rejects_extra_duplicate_version_and_handle_fail_closed() -> io::Result<()> {
    let rt = runtime()?;
    let _entered = rt.enter();
    let good = json();
    let cases = [
        "[]".to_owned(),
        "{}".to_owned(),
        good.replace("\"version\":1", "\"version\":2"),
        good.replace("\"version\":1", "\"version\":1,\"version\":1"),
        good.replace("\"version\":1", "\"version\":1,\"generation\":1"),
        good.replace("\"version\":1", "\"version\":1,\"stdout\":0"),
        "{\"version\":1,\"handle\":[7]}".into(),
        " ".repeat(MAX_OUTPUT_JSON + 1),
    ];
    for bad in cases {
        let (readers, writers) = create()?;
        let fds: Vec<_> = writers
            .roles
            .iter()
            .map(|fd| zbus::zvariant::Fd::from(fd.as_fd()))
            .collect();
        let message = raw_message(&bad, &fds);
        drop(fds);
        drop(writers);
        assert!(decode(message).is_err());
        eof(&readers)?;
    }
    Ok(())
}

#[test]
fn exact_count_and_aliases_fail_close_every_received_writer() -> io::Result<()> {
    let rt = runtime()?;
    let _entered = rt.enter();
    for indices in [vec![], vec![0], vec![0, 1, 0], vec![0, 0]] {
        let (readers, writers) = create()?;
        let fds: Vec<_> = indices
            .iter()
            .map(|&i| zbus::zvariant::Fd::from(writers.roles[i].as_fd()))
            .collect();
        let message = raw_message(&json(), &fds);
        drop(fds);
        drop(writers);
        assert!(decode(message).is_err());
        eof(&readers)?;
    }
    // Разные FD номера/индексы одного pipe: inode guard, а не только array alias.
    let (readers, mut writers) = create()?;
    writers.roles[1] = writers.roles[0].as_fd().try_clone_to_owned()?;
    assert_ne!(writers.roles[0].as_raw_fd(), writers.roles[1].as_raw_fd());
    let fds: Vec<_> = writers
        .roles
        .iter()
        .map(|fd| zbus::zvariant::Fd::from(fd.as_fd()))
        .collect();
    let message = raw_message(&json(), &fds);
    drop(fds);
    drop(writers);
    assert!(decode(message).is_err());
    eof(&readers)
}

#[test]
fn wrong_signature_path_interface_and_member_fail_closed() -> io::Result<()> {
    let rt = runtime()?;
    let _entered = rt.enter();
    for kind in 0..5 {
        let (readers, writers) = create()?;
        let fds: Vec<_> = writers
            .roles
            .iter()
            .map(|fd| zbus::zvariant::Fd::from(fd.as_fd()))
            .collect();
        let builder = zbus::Message::method(
            if kind == 0 { "/wrong" } else { OUTPUT_PATH },
            if kind == 1 { "Apply" } else { OUTPUT_METHOD },
        )
        .expect("константы")
        .interface(if kind == 2 {
            "wrong.Interface"
        } else {
            OUTPUT_INTERFACE
        })
        .expect("константы");
        let message = if kind == 3 {
            builder.build(&(json(), 1u32, fds.as_slice()))
        } else if kind == 4 {
            builder.build(&(fds.as_slice(), json()))
        } else {
            builder.build(&(json(), fds.as_slice()))
        }
        .expect("DATA build");
        drop(fds);
        drop(writers);
        assert!(decode(message).is_err());
        eof(&readers)?;
    }
    Ok(())
}

#[test]
fn invalid_owned_fd_flags_and_types_fail_closed() -> io::Result<()> {
    let rt = runtime()?;
    let _entered = rt.enter();
    for kind in 0..6 {
        let (readers, mut writers) = create()?;
        match kind {
            0 => writers.roles[0] = std::fs::File::open("/dev/null")?.into(),
            1 => rustix::fs::fcntl_setfl(&writers.roles[0], OFlags::NONBLOCK)?,
            2 => rustix::io::fcntl_setfd(&writers.roles[0], FdFlags::empty())?,
            3 => writers.roles[0] = readers.stdout.as_fd().try_clone_to_owned()?,
            4 => writers.roles[0] = std::os::unix::net::UnixStream::pair()?.0.into(),
            _ => {
                writers.roles[0] = rustix::fs::open(
                    format!("/proc/self/fd/{}", writers.roles[0].as_raw_fd()),
                    OFlags::RDWR | OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )?
            }
        }
        assert!(writers.validate().is_err());
        eof(&readers)?;
    }
    Ok(())
}

#[test]
fn named_fifo_on_disk_is_not_pipefs() -> io::Result<()> {
    let rt = runtime()?;
    let _entered = rt.enter();
    let root = std::env::var_os("FR16_TASK_ROOT")
        .ok_or_else(|| io::Error::other("нужен locked task root"))?;
    let path = std::path::PathBuf::from(root).join("carrier-named-fifo");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &path,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )?;
    let reader = rustix::fs::open(
        &path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    let writer = rustix::fs::open(
        &path,
        OFlags::WRONLY | OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    assert_eq!(fcntl_getfl(&writer)? & OFlags::ACCMODE, OFlags::WRONLY);
    let (readers, mut writers) = create()?;
    writers.roles[0] = writer;
    let result = writers.validate();
    drop(reader);
    std::fs::remove_file(path)?;
    assert!(result.is_err());
    eof(&readers)
}

#[test]
fn two_pairs_have_unique_identity_and_drop_is_eof() -> io::Result<()> {
    let rt = runtime()?;
    let _entered = rt.enter();
    let (a, aw) = create()?;
    let (b, bw) = create()?;
    let mut identities = std::collections::HashSet::new();
    for fd in aw.roles.iter().chain(bw.roles.iter()) {
        let stat = fstat(fd)?;
        assert!(identities.insert((stat.st_dev, stat.st_ino)));
    }
    drop(aw);
    drop(bw);
    eof(&a)?;
    eof(&b)
}

#[test]
fn own_pipe_partial_reads_without_executor_or_bus() -> io::Result<()> {
    let rt = runtime()?;
    let _entered = rt.enter();
    let (mut readers, writers) = create()?;
    let writers = writers.validate()?;
    rt.block_on(async {
        let mut sender = tokio::net::unix::pipe::Sender::from_owned_fd(writers.stdout)?;
        sender.write_all(b"partial").await?;
        drop(sender);
        drop(writers.stderr);
        let mut bytes = Vec::new();
        readers.stdout.read_to_end(&mut bytes).await?;
        assert_eq!(bytes, b"partial");
        io::Result::Ok(())
    })
}
