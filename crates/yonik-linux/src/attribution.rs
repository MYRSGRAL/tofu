//! Диагностика доступности proc-механизма. Не выдаёт identity отдельного flow.
use std::{fs, io::Read, os::unix::fs::MetadataExt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionFailure {
    ProcUnavailable,
    ForeignUidDenied,
    ProcessChanged,
    ScanLimit,
    NativeMechanismUnverified,
    WorkerIsCaller,
}

fn start_time(pid: u32) -> Result<u64, AttributionFailure> {
    let file = fs::File::open(format!("/proc/{pid}/stat"))
        .map_err(|_| AttributionFailure::ProcUnavailable)?;
    let mut text = String::new();
    file.take(16385)
        .read_to_string(&mut text)
        .map_err(|_| AttributionFailure::ProcUnavailable)?;
    if text.len() > 16384 {
        return Err(AttributionFailure::ProcUnavailable);
    }
    text.rsplit_once(')')
        .and_then(|(_, tail)| tail.split_whitespace().nth(19))
        .and_then(|value| value.parse().ok())
        .ok_or(AttributionFailure::ProcUnavailable)
}

/// Проверка одного PID ограничена 64 FD, с fencing по starttime до/после.
/// Успех подтверждает доступ текущего процесса, а не доступ production worker.
pub fn probe_process(pid: u32) -> Result<(), AttributionFailure> {
    let before = start_time(pid)?;
    let denied = |error: std::io::Error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            AttributionFailure::ForeignUidDenied
        } else {
            AttributionFailure::ProcUnavailable
        }
    };
    fs::read_link(format!("/proc/{pid}/exe")).map_err(denied)?;
    let entries = fs::read_dir(format!("/proc/{pid}/fd")).map_err(denied)?;
    for (index, entry) in entries.enumerate() {
        if index == 64 {
            return Err(AttributionFailure::ScanLimit);
        }
        match fs::read_link(entry.map_err(denied)?.path()) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(denied(error)),
        }
    }
    if start_time(pid)? != before {
        return Err(AttributionFailure::ProcessChanged);
    }
    Ok(())
}

/// Идентичность поступает только от canonical bus daemon после авторизации.
pub(crate) struct PeerProcess {
    pid: u32,
    uid: u32,
    start: u64,
    pidfd: std::os::fd::OwnedFd,
    namespaces: [std::path::PathBuf; 3],
}
impl PeerProcess {
    pub(crate) fn identity_digest(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"yonik-peer-context-v1\0");
        hash.update(self.pid.to_be_bytes());
        hash.update(self.uid.to_be_bytes());
        hash.update(self.start.to_be_bytes());
        for namespace in &self.namespaces {
            let bytes = namespace.as_os_str().as_encoded_bytes();
            hash.update((bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
        }
        hash.finalize().into()
    }
    pub(crate) fn verify_identity(&self) -> Result<(), AttributionFailure> {
        self.verify_pidfd()?;
        if start_time(self.pid)? != self.start
            || fs::metadata(format!("/proc/{}", self.pid))
                .map_err(|_| AttributionFailure::ProcUnavailable)?
                .uid()
                != self.uid
        {
            return Err(AttributionFailure::ProcessChanged);
        }
        for (name, expected) in ["net", "pid", "mnt"].iter().zip(&self.namespaces) {
            if fs::read_link(format!("/proc/{}/ns/{name}", self.pid))
                .map_err(|_| AttributionFailure::ProcUnavailable)?
                != *expected
            {
                return Err(AttributionFailure::ProcessChanged);
            }
        }
        self.verify_pidfd()
    }
    pub(crate) fn verified_uid(&self) -> Result<u32, AttributionFailure> {
        self.verify_identity()?;
        Ok(self.uid)
    }
    fn verify_pidfd(&self) -> Result<(), AttributionFailure> {
        use std::os::fd::AsRawFd;
        let file = fs::File::open(format!("/proc/self/fdinfo/{}", self.pidfd.as_raw_fd()))
            .map_err(|_| AttributionFailure::ProcUnavailable)?;
        let mut text = String::new();
        file.take(4097)
            .read_to_string(&mut text)
            .map_err(|_| AttributionFailure::ProcUnavailable)?;
        if text.len() > 4096
            || text.lines().find_map(|line| {
                line.strip_prefix("Pid:")
                    .and_then(|pid| pid.trim().parse::<u32>().ok())
            }) != Some(self.pid)
        {
            return Err(AttributionFailure::ProcessChanged);
        }
        Ok(())
    }
    pub(crate) fn capture(pid: u32, uid: u32) -> Result<Self, AttributionFailure> {
        let id = i32::try_from(pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .ok_or(AttributionFailure::ProcessChanged)?;
        let pidfd = rustix::process::pidfd_open(id, rustix::process::PidfdFlags::NONBLOCK)
            .map_err(|_| AttributionFailure::ProcUnavailable)?;
        let start = start_time(pid)?;
        if fs::metadata(format!("/proc/{pid}"))
            .map_err(|_| AttributionFailure::ProcUnavailable)?
            .uid()
            != uid
        {
            return Err(AttributionFailure::ProcessChanged);
        }
        let namespaces =
            ["net", "pid", "mnt"].map(|name| fs::read_link(format!("/proc/{pid}/ns/{name}")));
        let [net, pid_ns, mount] = namespaces;
        let denied = |error: std::io::Error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                AttributionFailure::ForeignUidDenied
            } else {
                AttributionFailure::ProcUnavailable
            }
        };
        Ok(Self {
            namespaces: [
                net.map_err(denied)?,
                pid_ns.map_err(denied)?,
                mount.map_err(denied)?,
            ],
            pid,
            uid,
            start,
            pidfd,
        })
    }
}

/// Prepare classification, не actual Apply/Session/Ready authority.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PreflightStage {
    NotRequired,
    StaticInputsVerified,
}
fn static_stage(
    required: bool,
    verify: impl FnOnce() -> Result<(), crate::helper::WorkerSupervisorError>,
) -> Result<PreflightStage, AttributionFailure> {
    if !required {
        return Ok(PreflightStage::NotRequired);
    }
    verify()
        .map(|()| PreflightStage::StaticInputsVerified)
        .map_err(|_| AttributionFailure::NativeMechanismUnverified)
}

pub(crate) async fn preflight(
    required: bool,
    peer: Option<&PeerProcess>,
    worker_uid: u32,
) -> Result<PreflightStage, AttributionFailure> {
    if !required {
        return Ok(PreflightStage::NotRequired);
    }
    let peer = peer.ok_or(AttributionFailure::ProcUnavailable)?;
    peer.verify_pidfd()?;
    if start_time(peer.pid)? != peer.start
        || fs::metadata(format!("/proc/{}", peer.pid))
            .map_err(|_| AttributionFailure::ProcUnavailable)?
            .uid()
            != peer.uid
    {
        return Err(AttributionFailure::ProcessChanged);
    }
    for (name, expected) in ["net", "pid", "mnt"].iter().zip(&peer.namespaces) {
        if fs::read_link(format!("/proc/{}/ns/{name}", peer.pid))
            .map_err(|_| AttributionFailure::ProcUnavailable)?
            != *expected
        {
            return Err(AttributionFailure::ProcessChanged);
        }
    }
    probe_process(peer.pid)?;
    peer.verify_pidfd()?;
    // Helper с CAP_NET_ADMIN не имеет CAP_SETUID/CAP_SETGID для запуска
    // диагностического child от worker UID. Его собственный успешный lookup
    // не подтверждает worker proc-view, namespaces и native owner lookup.
    if worker_uid == peer.uid {
        return Err(AttributionFailure::WorkerIsCaller);
    }
    // Production callers уже выполняются внутри tracked blocking actor job
    // (protection_host::execute_job), с FIFO/operation lease до завершения.
    // Никакого nested detached spawn и никакого permit из static результата.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let stage = static_stage(true, || {
        crate::helper::native_bootstrap::verify_preflight(deadline)
    })?;
    peer.verify_identity()?;
    Ok(stage)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_uid_read_access_and_missing_pid() {
        assert_eq!(probe_process(std::process::id()), Ok(()));
        assert_eq!(
            probe_process(u32::MAX),
            Err(AttributionFailure::ProcUnavailable)
        );
    }
    #[tokio::test]
    async fn no_app_policy_preserves_admission() {
        assert_eq!(
            preflight(false, None, 1002).await,
            Ok(PreflightStage::NotRequired)
        );
        assert!(preflight(true, None, 1002).await.is_err());
        let mut peer =
            PeerProcess::capture(std::process::id(), rustix::process::getuid().as_raw()).unwrap();
        assert_eq!(
            preflight(true, Some(&peer), 1002).await,
            Err(AttributionFailure::NativeMechanismUnverified)
        );
        peer.start += 1;
        assert_eq!(
            preflight(true, Some(&peer), 1002).await,
            Err(AttributionFailure::ProcessChanged)
        );
    }
}

/// Независимая библиотека private owner receiver.
pub mod broker;

/// Production NativeBroker wire; не fixture YOB1 ABI.
pub mod native_wire;

/// Закрытая сессия frozen native ABI.
pub mod native_session;

#[cfg(test)]
mod static_preflight_tests {
    use super::*;
    #[test]
    fn pure_static_preflight_not_required_skips_all_installed_input_io() {
        assert_eq!(
            static_stage(false, || panic!("не читать profile")),
            Ok(PreflightStage::NotRequired)
        );
    }
    #[test]
    fn pure_static_preflight_missing_profile_defaults_unverified() {
        assert_eq!(
            static_stage(true, || Err(
                crate::helper::WorkerSupervisorError::Unavailable
            )),
            Err(AttributionFailure::NativeMechanismUnverified)
        );
    }
    #[test]
    fn pure_static_preflight_valid_inputs_only_classify_prepare_not_ready_or_session() {
        assert_eq!(
            static_stage(true, || Ok(())),
            Ok(PreflightStage::StaticInputsVerified)
        );
        for error in [
            crate::helper::WorkerSupervisorError::Untrusted,
            crate::helper::WorkerSupervisorError::Stale,
            crate::helper::WorkerSupervisorError::Timeout,
        ] {
            assert_eq!(
                static_stage(true, || Err(error)),
                Err(AttributionFailure::NativeMechanismUnverified)
            );
        }
    }
}
