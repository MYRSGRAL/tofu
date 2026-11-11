use std::{process::Stdio, time::Duration};

use rustix::process::{Pid, Signal, kill_process};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Child,
    time::timeout,
};
use yonik_engine::Engine;

use super::{BinarySnapshot, Cancellation, HostError};

pub(crate) const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) async fn terminate(child: &mut Child) -> Result<(), HostError> {
    if child.try_wait().map_err(|_| HostError::Cleanup)?.is_some() {
        return Ok(());
    }
    let pid = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .and_then(Pid::from_raw)
        .ok_or(HostError::Cleanup)?;
    match kill_process(pid, Signal::TERM) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => {}
        Err(_) => return Err(HostError::Cleanup),
    }
    match timeout(Duration::from_secs(3), child.wait()).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) => Err(HostError::Cleanup),
        Err(_) => child.kill().await.map_err(|_| HostError::Cleanup),
    }
}

pub(crate) async fn verify_version(
    binary: &BinarySnapshot,
    cancel: &mut Cancellation,
) -> Result<(), HostError> {
    cancel.check()?;
    let mut child = binary
        .command()
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| HostError::Process)?;
    let output = child.stdout.take().ok_or(HostError::Process)?;
    let operation = async {
        let mut bytes = Vec::new();
        let mut output = output.take(4097);
        let (read, status) = tokio::join!(output.read_to_end(&mut bytes), child.wait());
        if read.is_err() || !status.map_err(|_| HostError::Process)?.success() || bytes.len() > 4096
        {
            return Err(HostError::Version);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| HostError::Version)?;
        let prefix = match binary.engine {
            Engine::Xray => "Xray 26.10.31",
            Engine::SingBox => "sing-box version 1.14.4",
        };
        let line = text.lines().next().ok_or(HostError::Version)?;
        if line != prefix
            && !line
                .strip_prefix(prefix)
                .is_some_and(|tail| tail.starts_with(' '))
        {
            return Err(HostError::Version);
        }
        Ok(())
    };
    let result = tokio::select! {
        _ = cancel.cancelled() => Err(HostError::Cancelled),
        result = timeout(OPERATION_TIMEOUT, operation) => result.unwrap_or(Err(HostError::Timeout)),
    };
    if result.is_err() {
        terminate(&mut child).await?;
    }
    result
}

pub(crate) fn spawn(binary: &BinarySnapshot, checking: bool) -> Result<Child, HostError> {
    let mut command = binary.command();
    match binary.engine {
        Engine::Xray => {
            command.arg("run");
            if checking {
                command.arg("-test");
            }
            command.args(["-c", "stdin:"]);
        }
        Engine::SingBox => {
            command
                .arg(if checking { "check" } else { "run" })
                .args(["-c", "/dev/stdin"]);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| HostError::Process)
}

pub(crate) async fn write_config(
    child: &mut Child,
    bytes: &[u8],
    cancel: &mut Cancellation,
) -> Result<(), HostError> {
    let mut stdin = child.stdin.take().ok_or(HostError::Process)?;
    tokio::select! {
        _ = cancel.cancelled() => Err(HostError::Cancelled),
        result = timeout(OPERATION_TIMEOUT, stdin.write_all(bytes)) => result.map_err(|_| HostError::Timeout)?.map_err(|_| HostError::Process),
    }
}

pub(crate) async fn config_check(
    binary: &BinarySnapshot,
    bytes: &[u8],
    cancel: &mut Cancellation,
) -> Result<(), HostError> {
    cancel.check()?;
    let mut child = spawn(binary, true)?;
    let operation = async {
        write_config(&mut child, bytes, cancel).await?;
        tokio::select! {
            _ = cancel.cancelled() => Err(HostError::Cancelled),
            result = timeout(OPERATION_TIMEOUT, child.wait()) => {
                let status = result.map_err(|_| HostError::Timeout)?.map_err(|_| HostError::Process)?;
                if status.success() { Ok(()) } else { Err(HostError::ConfigRejected) }
            }
        }
    }.await;
    if operation.is_err() {
        terminate(&mut child).await?;
    }
    operation
}

/// Событие Engine берётся из фактического OS exit status после reap, не из текста.
pub(crate) async fn terminate_logged(
    child: &mut Child,
    log: Option<&crate::event_log::EventLog>,
) -> Result<(), HostError> {
    terminate(child).await?;
    if let Some(log) = log {
        if let Ok(Some(status)) = child.try_wait() {
            log.record(
                crate::event_log::EventSource::Engine,
                crate::event_log::EventCode::ProcessExited,
                status.code(),
            );
        } else {
            log.reject();
        }
    }
    Ok(())
}
