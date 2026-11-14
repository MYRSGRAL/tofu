//! Настоящий Prepared с синтетическими DATA-полями; не helper-issued authorization.
use super::*;
use crate::event_log::{EventCode, EventContext, EventSource, LogFilter, MAX_PAGE};
use std::os::fd::AsFd;

fn prepared(handle: [u8; 32]) -> Prepared {
    Prepared {
        handle,
        owner: ":1.42".into(),
        peer: ":1.43".into(),
    }
}
fn runtime() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
}

#[test]
fn actual_prepared_handle_owned_with_immutable_log_and_engine() -> io::Result<()> {
    let rt = runtime()?;
    let _entered = rt.enter();
    let root = EventLog::new();
    let first = PreparedOutput::bind(prepared([3; 32]), Engine::Xray, &root, 7)?;
    let second = PreparedOutput::bind(prepared([4; 32]), Engine::SingBox, &root, 8)?;
    first
        .log
        .record(EventSource::Engine, EventCode::ProcessSpawned, None);
    second
        .log
        .record(EventSource::Engine, EventCode::ProcessSpawned, None);
    first
        .log
        .record(EventSource::Engine, EventCode::ProcessExited, None);
    let page = root
        .read_scoped(0, LogFilter::default(), MAX_PAGE)
        .expect("журнал доступен");
    assert_eq!(
        page.contexts,
        vec![
            Some(EventContext {
                session_id: 1,
                generation: 7
            }),
            Some(EventContext {
                session_id: 2,
                generation: 8
            }),
            Some(EventContext {
                session_id: 1,
                generation: 7
            }),
        ]
    );
    assert_eq!(first.engine, Engine::Xray);
    assert_eq!(second.engine, Engine::SingBox);
    assert_eq!(first.prepared.owner, ":1.42");
    assert_eq!(first.prepared.peer, ":1.43");
    let encoded = first.encode()?;
    assert_eq!(encoded.prepared.handle, [3; 32]);
    assert_eq!(encoded.engine, Engine::Xray);
    encoded
        .log
        .record(EventSource::Engine, EventCode::ProcessExited, None);
    let decoded = output_transport::decode(encoded.message)?;
    assert_eq!(decoded.handle, [3; 32]);
    drop(decoded);
    for fd in [&encoded.readers.stdout, &encoded.readers.stderr] {
        assert_eq!(rustix::io::read(fd, &mut [0])?, 0);
    }
    Ok(())
}

#[test]
fn invalid_generation_and_drop_close_bound_transport() -> io::Result<()> {
    let rt = runtime()?;
    let _entered = rt.enter();
    let root = EventLog::new();
    assert!(PreparedOutput::bind(prepared([1; 32]), Engine::Xray, &root, 0).is_err());
    let bound = PreparedOutput::bind(prepared([2; 32]), Engine::Xray, &root, 1)?;
    let stdout = bound.readers.stdout.as_fd().try_clone_to_owned()?;
    let stderr = bound.readers.stderr.as_fd().try_clone_to_owned()?;
    drop(bound);
    assert_eq!(rustix::io::read(&stdout, &mut [0])?, 0);
    assert_eq!(rustix::io::read(&stderr, &mut [0])?, 0);
    Ok(())
}
