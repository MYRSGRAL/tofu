use std::{
    collections::HashSet,
    fs,
    io::Read,
    num::{NonZeroU16, NonZeroU64},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use sha2::{Digest, Sha256};
use tokio::{
    process::Child,
    time::{sleep, timeout},
};
use yonik_application::{
    ConfigDigest, EnvironmentStamp, Event, Fault, Outcome, Session, State, Token,
};
use yonik_engine::{CompiledConfig, ProbeAccess};

use super::{BinarySnapshot, Cancellation, HostError, ProbeResult, ProbeTarget, probe, process};

static CONTROLLER: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

// Локальный режим не владеет системной сетью/правами. Snapshot ядра immutable;
// эти значения не выдаются за поколения будущих NetworkHost/ApplicationCatalog.
const LOCAL_ENV: EnvironmentStamp = EnvironmentStamp {
    network: 0,
    applications: 0,
    authorization: 0,
    engine: 1,
    authorized: true,
};

pub struct CheckedConfig {
    binary: BinarySnapshot,
    bytes: Vec<u8>,
    digest: ConfigDigest,
    port: NonZeroU16,
    probe_port: NonZeroU16,
    probe_access: ProbeAccess,
}
impl std::fmt::Debug for CheckedConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CheckedConfig(<скрыто>)")
    }
}
impl CheckedConfig {
    pub async fn validate(
        binary: BinarySnapshot,
        config: CompiledConfig,
        cancel: &mut Cancellation,
    ) -> Result<Self, HostError> {
        if binary.engine != config.engine() {
            return Err(HostError::ConfigRejected);
        }
        let port = config.listen_port();
        let probe_port = config.probe_port();
        let mut bytes = Vec::new();
        config
            .write_sensitive(&mut bytes)
            .map_err(|_| HostError::ConfigRejected)?;
        let probe_access = config.into_probe_access();
        process::verify_version(&binary, cancel).await?;
        process::config_check(&binary, &bytes, cancel).await?;
        let digest = ConfigDigest::new(Sha256::digest(&bytes).into());
        Ok(Self {
            binary,
            bytes,
            digest,
            port,
            probe_port,
            probe_access,
        })
    }
}

/// Один владелец сериализует start/check/stop. На Drop — аварийный kill_on_drop;
/// подтверждение cleanup выдаётся только после явного stop().await.
pub struct LocalSession {
    session: Session,
    active: Option<Token>,
    child: Option<Child>,
    binary: Option<BinarySnapshot>,
    port: Option<NonZeroU16>,
    probe_port: Option<NonZeroU16>,
    probe_access: ProbeAccess,
}
impl std::fmt::Debug for LocalSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalSession")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}
impl LocalSession {
    pub fn new() -> Result<Self, HostError> {
        let id = CONTROLLER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| HostError::State)?;
        let id = NonZeroU64::new(id).ok_or(HostError::State)?;
        Ok(Self {
            session: Session::new(id),
            active: None,
            child: None,
            binary: None,
            port: None,
            probe_port: None,
            probe_access: ProbeAccess::default(),
        })
    }
    pub fn state(&self) -> State {
        self.session.state()
    }
    pub fn engine_pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
    }

    /// Инвалидирует результат проверки, сохраняя запущенное ядро и его ресурсы.
    pub fn invalidate_check(&mut self) -> Result<(), HostError> {
        self.accept(Event::CheckInvalidated)
    }

    fn accept(&mut self, event: Event) -> Result<(), HostError> {
        let token = self.active.ok_or(HostError::State)?;
        match self
            .session
            .accept(token, event)
            .map_err(|_| HostError::State)?
        {
            Outcome::Applied(next) => {
                self.active = next;
                Ok(())
            }
            Outcome::Ignored => Err(HostError::State),
        }
    }

    pub async fn start(
        &mut self,
        checked: CheckedConfig,
        cancel: &mut Cancellation,
    ) -> Result<(), HostError> {
        cancel.check()?;
        let token = self.session.begin().map_err(|_| HostError::State)?;
        let plan = self
            .session
            .prepare(token, LOCAL_ENV, checked.digest)
            .map_err(|_| HostError::State)?;
        let current = ConfigDigest::new(Sha256::digest(&checked.bytes).into());
        self.active = match self
            .session
            .apply(plan, LOCAL_ENV, current)
            .map_err(|_| HostError::State)?
        {
            Outcome::Applied(Some(token)) => Some(token),
            _ => return Err(HostError::State),
        };
        self.port = Some(checked.port);
        self.probe_port = Some(checked.probe_port);
        self.probe_access = checked.probe_access;
        self.binary = Some(checked.binary);
        let result = async {
            let binary = self.binary.as_ref().ok_or(HostError::State)?;
            self.child = Some(process::spawn(binary, false)?);
            let child = self.child.as_mut().ok_or(HostError::State)?;
            process::write_config(child, &checked.bytes, cancel).await?;
            let readiness = async {
                loop {
                    if child.try_wait().map_err(|_| HostError::Process)?.is_some() { return Err(HostError::EngineExited); }
                    if listeners_owned(child.id().ok_or(HostError::EngineExited)?, checked.port.get(), checked.probe_port.get())? { return Ok(()); }
                    sleep(Duration::from_millis(50)).await;
                }
            };
            tokio::select! {
                _ = cancel.cancelled() => Err(HostError::Cancelled),
                result = timeout(process::OPERATION_TIMEOUT, readiness) => result.unwrap_or(Err(HostError::Timeout)),
            }
        }.await;
        if let Err(error) = result {
            if error == HostError::Cancelled {
                self.stop().await?;
            } else {
                self.fail().await?;
            }
            return Err(error);
        }
        self.accept(Event::NetworkApplied)
    }

    pub async fn check(
        &mut self,
        target: &ProbeTarget,
        cancel: &mut Cancellation,
    ) -> Result<ProbeResult, HostError> {
        if !matches!(self.state(), State::Checking | State::Limited) {
            return Err(HostError::State);
        }
        cancel.check()?;
        if !self
            .probe_access
            .allows_host(target.url.host_str().ok_or(HostError::ProbeTarget)?)
        {
            return Err(HostError::ProbeTarget);
        }
        let port = self.port.ok_or(HostError::State)?.get();
        let probe_port = self.probe_port.ok_or(HostError::State)?.get();
        let child = self.child.as_mut().ok_or(HostError::State)?;
        if let Err(error) = verify_listener(child, port, probe_port) {
            self.fail().await?;
            return Err(error);
        }
        let result = tokio::select! {
            biased;
            status = child.wait() => { let _ = status.map_err(|_| HostError::Process)?; Err(HostError::EngineExited) },
            _ = cancel.cancelled() => Err(HostError::Cancelled),
            result = probe::probe(probe_port, target, self.probe_access.auth()) => Ok(result),
        };
        match result {
            Ok(result) => {
                if let Err(error) = verify_listener(child, port, probe_port) {
                    self.fail().await?;
                    return Err(error);
                }
                self.accept(if target.accepts(result) {
                    Event::CheckPassed
                } else {
                    Event::EndpointUnavailable
                })?;
                Ok(result)
            }
            Err(HostError::Cancelled) => Err(HostError::Cancelled),
            Err(error) => {
                self.fail().await?;
                Err(error)
            }
        }
    }

    pub async fn wait_exit(&mut self, cancel: &mut Cancellation) -> Result<(), HostError> {
        let child = self.child.as_mut().ok_or(HostError::State)?;
        tokio::select! {
            _ = cancel.cancelled() => return Err(HostError::Cancelled),
            status = child.wait() => { status.map_err(|_| HostError::Process)?; }
        }
        self.fail().await?;
        Err(HostError::EngineExited)
    }

    async fn fail(&mut self) -> Result<(), HostError> {
        self.accept(Event::Failed(Fault::Engine))?;
        self.release().await
    }
    async fn release(&mut self) -> Result<(), HostError> {
        if let Some(child) = self.child.as_mut() {
            if process::terminate(child).await.is_err() {
                self.accept(Event::CleanupFailed)?;
                return Err(HostError::Cleanup);
            }
        }
        self.child = None;
        self.binary = None;
        self.port = None;
        self.probe_port = None;
        self.probe_access = ProbeAccess::default();
        self.accept(Event::CleanupConfirmed)
    }
    pub async fn stop(&mut self) -> Result<(), HostError> {
        self.active = self.session.stop().map_err(|_| HostError::State)?;
        if self.active.is_some() {
            self.release().await
        } else {
            Ok(())
        }
    }
}

fn verify_listener(child: &mut Child, port: u16, probe_port: u16) -> Result<(), HostError> {
    if child.try_wait().map_err(|_| HostError::Process)?.is_some() {
        return Err(HostError::EngineExited);
    }
    if listeners_owned(child.id().ok_or(HostError::EngineExited)?, port, probe_port)? {
        Ok(())
    } else {
        Err(HostError::Listener)
    }
}

fn listeners_owned(pid: u32, port: u16, probe_port: u16) -> Result<bool, HostError> {
    Ok(listener_owned(pid, port)? && (port == probe_port || listener_owned(pid, probe_port)?))
}

pub(super) fn listener_owned(pid: u32, port: u16) -> Result<bool, HostError> {
    let mut table = String::new();
    fs::File::open(format!("/proc/{pid}/net/tcp"))
        .map_err(|_| HostError::Listener)?
        .take(8 * 1024 * 1024 + 1)
        .read_to_string(&mut table)
        .map_err(|_| HostError::Listener)?;
    if table.len() > 8 * 1024 * 1024 {
        return Err(HostError::Listener);
    }
    let endpoint = format!("0100007F:{port:04X}");
    let inodes: HashSet<&str> = table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.get(1) == Some(&endpoint.as_str()) && fields.get(3) == Some(&"0A") {
                fields.get(9).copied()
            } else {
                None
            }
        })
        .collect();
    if inodes.is_empty() {
        return Ok(false);
    }
    for fd in fs::read_dir(format!("/proc/{pid}/fd")).map_err(|_| HostError::Listener)? {
        let fd = fd.map_err(|_| HostError::Listener)?;
        match fs::read_link(fd.path()) {
            Ok(path) => {
                if let Some(inode) = path
                    .to_str()
                    .and_then(|text| text.strip_prefix("socket:["))
                    .and_then(|text| text.strip_suffix(']'))
                {
                    if inodes.contains(inode) {
                        return Ok(true);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(HostError::Listener),
        }
    }
    Ok(false)
}
