//! Владелец двух фиксированных systemd worker, с подтверждением пустой cgroup.
//! Accepted подтверждает передачу FD, но не готовность сети и не Applied.
use crate::{
    Cancellation,
    worker::{WorkerAck, WorkerMode, WorkerPayload, WorkerStatus},
};
use futures_util::StreamExt;
use rustix::fs::{Mode, OFlags, fstatfs, open, openat};
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    os::unix::fs::MetadataExt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{task::JoinHandle, time::Instant};
use zbus::{
    Connection,
    zvariant::{OwnedObjectPath, OwnedValue},
};

const BUS: &str = "unix:path=/run/dbus/system_bus_socket";
const DESTINATION: &str = "org.freedesktop.systemd1";
const MANAGER: &str = "/org/freedesktop/systemd1";
const UNIT_IFACE: &str = "org.freedesktop.systemd1.Unit";
const SERVICE_IFACE: &str = "org.freedesktop.systemd1.Service";
const SOCKET_IFACE: &str = "org.freedesktop.systemd1.Socket";
const DEADLINE: Duration = Duration::from_secs(8);
static OWNED: AtomicBool = AtomicBool::new(false);
type Properties = HashMap<String, OwnedValue>;
type ExecCommands = Vec<(String, Vec<String>, bool, u64, u64, u64, u64, u32, i32, i32)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerSupervisorError {
    Busy,
    Unavailable,
    Untrusted,
    Timeout,
    Cancelled,
    Protocol,
    Stale,
    NotEmpty,
    RecoveryRequired,
}
impl std::fmt::Display for WorkerSupervisorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Busy => "Worker уже имеет владельца",
            Self::Unavailable => "Systemd worker недоступен",
            Self::Untrusted => "Контракт systemd worker не подтверждён",
            Self::Timeout => "Истёк срок операции worker",
            Self::Cancelled => "Операция worker отменена",
            Self::Protocol => "Некорректный ответ worker",
            Self::Stale => "Экземпляр worker изменился",
            Self::NotEmpty => "Завершение worker не подтверждено",
            Self::RecoveryRequired => "Требуется восстановление worker",
        })
    }
}
impl std::error::Error for WorkerSupervisorError {}
type Result<T> = std::result::Result<T, WorkerSupervisorError>;

struct Permit;
impl Permit {
    fn acquire() -> Result<Arc<Self>> {
        OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| WorkerSupervisorError::Busy)?;
        Ok(Arc::new(Self))
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        OWNED.store(false, Ordering::Release);
    }
}

/// Неподделываемая локальная ссылка на конкретную invocation, не только PID.
#[derive(Debug)]
pub struct WorkerTicket {
    pid: u32,
    invocation: [u8; 16],
}
impl WorkerTicket {
    pub fn pid(&self) -> u32 {
        self.pid
    }
    fn matches(&self, state: &WorkerState) -> bool {
        self.pid != 0
            && self.pid == state.pid
            && self.invocation.as_slice() == state.invocation
            && self.invocation != [0; 16]
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerLiveness {
    Running,
    Stopped,
}
struct WorkerState {
    active: String,
    pid: u32,
    invocation: Vec<u8>,
}
trait WorkerFence: Send + Sync {
    fn empty(&self, mode: WorkerMode) -> Result<bool>;
}
struct SystemFence;
impl WorkerFence for SystemFence {
    fn empty(&self, mode: WorkerMode) -> Result<bool> {
        cgroup_empty(mode)
    }
}
impl WorkerState {
    fn stopped(&self) -> bool {
        matches!(self.active.as_str(), "inactive" | "failed") && self.pid == 0
    }
}
impl WorkerMode {
    fn service(self) -> &'static str {
        match self {
            Self::Check => "yonik-worker-check.service",
            Self::Run => "yonik-worker.service",
        }
    }
    fn socket(self) -> &'static str {
        match self {
            Self::Check => "yonik-worker-check.socket",
            Self::Run => "yonik-worker.socket",
        }
    }
    fn cgroup(self) -> &'static str {
        match self {
            Self::Check => "/system.slice/yonik-worker-check.service",
            Self::Run => "/system.slice/yonik-worker.service",
        }
    }
    fn channel(self) -> &'static str {
        match self {
            Self::Check => "/run/yonik-worker/check.sock",
            Self::Run => "/run/yonik-worker/run.sock",
        }
    }
}

/// Не Clone: один владелец в процессе. Отменённая future оставляет pending и
/// blocking transfer внутри владельца; только recover снова разрешает запуск.
pub struct SystemWorkerSupervisor {
    connection: Connection,
    owner: String,
    permit: Arc<Permit>,
    pending: Option<WorkerMode>,
    transfer: Option<JoinHandle<Result<WorkerAck>>>,
    current: Option<WorkerTicket>,
    clean: bool,
    retained_copy: Option<Arc<WorkerPayload>>,
    fence: Box<dyn WorkerFence>,
}
impl Drop for SystemWorkerSupervisor {
    fn drop(&mut self) {
        if !self.clean {
            if let Some(payload) = self.retained_copy.take() {
                // Без empty-cgroup proof нельзя освободить global reserve:
                // он остаётся в карантине до выхода helper, ограничен ByteBudget.
                // BindsTo прекращает worker при завершении helper unit.
                std::mem::forget(payload);
            }
        }
    }
}
impl SystemWorkerSupervisor {
    pub async fn system() -> Result<Self> {
        let permit = Permit::acquire()?;
        tokio::time::timeout(DEADLINE, async {
            verify_installed()?;
            let connection = zbus::connection::Builder::address(BUS)
                .map_err(unavailable)?
                .build()
                .await
                .map_err(unavailable)?;
            let proxy = zbus::fdo::DBusProxy::new(&connection)
                .await
                .map_err(unavailable)?;
            let name = zbus::names::BusName::try_from(DESTINATION).map_err(unavailable)?;
            let owner = proxy
                .get_name_owner(name)
                .await
                .map_err(unavailable)?
                .to_string();
            let unique = zbus::names::BusName::try_from(owner.as_str()).map_err(unavailable)?;
            if proxy
                .get_connection_unix_user(unique)
                .await
                .map_err(unavailable)?
                != 0
            {
                return Err(WorkerSupervisorError::Untrusted);
            }
            let mut supervisor = Self {
                connection,
                owner,
                permit,
                pending: None,
                transfer: None,
                current: None,
                clean: false,
                retained_copy: None,
                fence: Box::new(SystemFence),
            };
            supervisor.validate_contract().await?;
            // Startup лишь проверяет состояние. Старый worker останавливает только recover.
            supervisor.clean = supervisor.all_empty().await?;
            Ok(supervisor)
        })
        .await
        .map_err(|_| WorkerSupervisorError::Timeout)?
    }
    pub async fn check(
        &mut self,
        payload: Arc<WorkerPayload>,
        cancel: &mut Cancellation,
    ) -> Result<()> {
        let deadline = Instant::now() + DEADLINE;
        self.begin(WorkerMode::Check, cancel, deadline).await?;
        let ack = self
            .send(payload, WorkerMode::Check, cancel, deadline)
            .await?;
        if ack.status != WorkerStatus::Checked {
            return Err(WorkerSupervisorError::Protocol);
        }
        self.cleanup(cancel, deadline).await
    }
    pub async fn start(
        &mut self,
        payload: Arc<WorkerPayload>,
        cancel: &mut Cancellation,
    ) -> Result<WorkerTicket> {
        let deadline = Instant::now() + DEADLINE;
        self.begin(WorkerMode::Run, cancel, deadline).await?;
        let ack = self
            .send(payload, WorkerMode::Run, cancel, deadline)
            .await?;
        if ack.status != WorkerStatus::Accepted {
            return Err(WorkerSupervisorError::Protocol);
        }
        let state = bounded(cancel, deadline, self.state(WorkerMode::Run)).await?;
        let invocation: [u8; 16] = state
            .invocation
            .try_into()
            .map_err(|_| WorkerSupervisorError::Protocol)?;
        if ack.pid != state.pid || invocation == [0; 16] || state.active != "active" {
            return Err(WorkerSupervisorError::Protocol);
        }
        let ticket = WorkerTicket {
            pid: ack.pid,
            invocation,
        };
        self.current = Some(WorkerTicket {
            pid: ticket.pid,
            invocation: ticket.invocation,
        });
        self.pending = None;
        Ok(ticket)
    }
    pub async fn observe(&self, ticket: &WorkerTicket) -> Result<WorkerLiveness> {
        tokio::time::timeout(DEADLINE, async {
            let state = self.state(WorkerMode::Run).await?;
            if state.stopped() && self.fence.empty(WorkerMode::Run)? {
                return Ok(WorkerLiveness::Stopped);
            }
            if !ticket.matches(&state) {
                return Err(WorkerSupervisorError::Stale);
            }
            Ok(WorkerLiveness::Running)
        })
        .await
        .map_err(|_| WorkerSupervisorError::Timeout)?
    }
    pub async fn stop(&mut self, ticket: &WorkerTicket, cancel: &mut Cancellation) -> Result<()> {
        let deadline = Instant::now() + DEADLINE;
        let state = bounded(cancel, deadline, self.state(WorkerMode::Run)).await?;
        if !(ticket.matches(&state) || state.stopped() && self.fence.empty(WorkerMode::Run)?) {
            return Err(WorkerSupervisorError::Stale);
        }
        if !self
            .current
            .as_ref()
            .is_some_and(|owned| owned.pid == ticket.pid && owned.invocation == ticket.invocation)
        {
            return Err(WorkerSupervisorError::Stale);
        }
        self.pending = Some(WorkerMode::Run);
        self.cleanup(cancel, deadline).await
    }
    /// Явное восстановление фиксированных owned units, включая прошлую invocation.
    pub async fn recover(&mut self, cancel: &mut Cancellation) -> Result<()> {
        let deadline = Instant::now() + DEADLINE;
        bounded(cancel, deadline, self.validate_contract()).await?;
        self.clean = false;
        self.current = None;
        self.cleanup(cancel, deadline).await
    }
    async fn begin(
        &mut self,
        mode: WorkerMode,
        cancel: &mut Cancellation,
        deadline: Instant,
    ) -> Result<()> {
        if !self.clean
            || self.pending.is_some()
            || self.transfer.is_some()
            || self.current.is_some()
        {
            return Err(WorkerSupervisorError::RecoveryRequired);
        }
        bounded(cancel, deadline, self.validate_contract()).await?;
        if !bounded(cancel, deadline, self.all_empty()).await? {
            self.clean = false;
            return Err(WorkerSupervisorError::RecoveryRequired);
        }
        cancel
            .check()
            .map_err(|_| WorkerSupervisorError::Cancelled)?;
        // Записано до первого эффекта: drop future не теряет необходимость recovery.
        self.pending = Some(mode);
        self.clean = false;
        bounded(cancel, deadline, self.unit_job("StartUnit", mode.socket())).await
    }
    async fn send(
        &mut self,
        payload: Arc<WorkerPayload>,
        mode: WorkerMode,
        cancel: &mut Cancellation,
        deadline: Instant,
    ) -> Result<WorkerAck> {
        if payload.engine() == yonik_engine::Engine::SingBox {
            self.retained_copy = Some(Arc::clone(&payload));
        }
        let stream = bounded(cancel, deadline, async {
            tokio::net::UnixStream::connect(mode.channel())
                .await
                .map_err(unavailable)
        })
        .await?;
        verify_socket(mode)?;
        let stream = stream.into_std().map_err(unavailable)?;
        stream.set_nonblocking(false).map_err(unavailable)?;
        let permit = Arc::clone(&self.permit);
        self.transfer = Some(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            payload
                .send(&stream, mode)
                .map_err(|_| WorkerSupervisorError::Protocol)
        }));
        let handle = self
            .transfer
            .as_mut()
            .ok_or(WorkerSupervisorError::Unavailable)?;
        let result = bounded(cancel, deadline, async {
            handle.await.map_err(unavailable)?
        })
        .await;
        // При timeout/cancel handle остаётся; повторный cleanup обязательно его ждёт.
        if self.transfer.as_ref().is_some_and(JoinHandle::is_finished) {
            self.transfer = None;
        }
        result
    }
    async fn cleanup(&mut self, cancel: &mut Cancellation, deadline: Instant) -> Result<()> {
        self.clean = false;
        // Сначала запрет новой активации, затем завершение уже начатой передачи.
        for mode in [WorkerMode::Check, WorkerMode::Run] {
            bounded(cancel, deadline, self.unit_job("StopUnit", mode.socket())).await?;
        }
        if let Some(handle) = self.transfer.as_mut() {
            bounded(cancel, deadline, async {
                let _result = handle.await.map_err(unavailable)?;
                Ok(())
            })
            .await?;
            self.transfer = None;
        }
        for mode in [WorkerMode::Check, WorkerMode::Run] {
            if mode == WorkerMode::Run {
                if let Some(ticket) = self.current.as_ref() {
                    let state = bounded(cancel, deadline, self.state(mode)).await?;
                    if !(ticket.matches(&state) || state.stopped() && self.fence.empty(mode)?) {
                        return Err(WorkerSupervisorError::Stale);
                    }
                }
            }
            bounded(cancel, deadline, self.unit_job("StopUnit", mode.service())).await?;
        }
        loop {
            if bounded(cancel, deadline, self.all_empty()).await? {
                self.pending = None;
                self.current = None;
                self.clean = true;
                self.retained_copy = None;
                return Ok(());
            }
            bounded(cancel, deadline, async {
                tokio::time::sleep(Duration::from_millis(25)).await;
                Ok(())
            })
            .await?;
        }
    }
    async fn unit_job(&self, method: &str, unit: &'static str) -> Result<()> {
        tokio::time::timeout(DEADLINE, async {
            let proxy = zbus::Proxy::new(
                &self.connection,
                self.owner.as_str(),
                MANAGER,
                "org.freedesktop.systemd1.Manager",
            )
            .await
            .map_err(unavailable)?;
            let subscribed: std::result::Result<(), zbus::Error> =
                proxy.call("Subscribe", &()).await;
            if let Err(error) = subscribed {
                match error {
                    zbus::Error::MethodError(name, _, _)
                        if name.as_str() == "org.freedesktop.systemd1.AlreadySubscribed" => {}
                    _ => return Err(WorkerSupervisorError::Unavailable),
                }
            }
            // Match устанавливается до запроса: быстрый job может завершиться
            // ещё до ответа StartUnit/StopUnit. Ответ содержит только job path.
            let mut events = proxy
                .receive_signal_with_args("JobRemoved", &[(2, unit)])
                .await
                .map_err(unavailable)?;
            let reply = proxy
                .call_method(method, &(unit, "replace"))
                .await
                .map_err(unavailable)?;
            let (job,): (OwnedObjectPath,) = reply.body().deserialize().map_err(unavailable)?;
            while let Some(message) = events.next().await {
                let (_, completed, name, result): (u32, OwnedObjectPath, String, String) =
                    message.body().deserialize().map_err(unavailable)?;
                if completed == job {
                    return if name == unit && result == "done" {
                        Ok(())
                    } else {
                        Err(WorkerSupervisorError::Protocol)
                    };
                }
            }
            Err(WorkerSupervisorError::Unavailable)
        })
        .await
        .map_err(|_| WorkerSupervisorError::Timeout)?
    }
    async fn path(&self, unit: &str) -> Result<OwnedObjectPath> {
        let reply = self
            .connection
            .call_method(
                Some(self.owner.as_str()),
                MANAGER,
                Some("org.freedesktop.systemd1.Manager"),
                "LoadUnit",
                &(unit,),
            )
            .await
            .map_err(unavailable)?;
        let (path,): (OwnedObjectPath,) = reply.body().deserialize().map_err(unavailable)?;
        Ok(path)
    }
    async fn properties(&self, unit: &str, interface: &str) -> Result<Properties> {
        let path = self.path(unit).await?;
        let proxy = zbus::Proxy::new(
            &self.connection,
            self.owner.as_str(),
            path.as_str(),
            "org.freedesktop.DBus.Properties",
        )
        .await
        .map_err(unavailable)?;
        proxy
            .call("GetAll", &(interface,))
            .await
            .map_err(unavailable)
    }
    async fn state(&self, mode: WorkerMode) -> Result<WorkerState> {
        let unit = self.properties(mode.service(), UNIT_IFACE).await?;
        let service = self.properties(mode.service(), SERVICE_IFACE).await?;
        let cgroup = string(&service, "ControlGroup")?;
        if !cgroup.is_empty() && cgroup != mode.cgroup() {
            return Err(WorkerSupervisorError::Untrusted);
        }
        Ok(WorkerState {
            active: string(&unit, "ActiveState")?,
            pid: number32(&service, "MainPID")?,
            invocation: array(&unit, "InvocationID")?,
        })
    }
    async fn all_empty(&self) -> Result<bool> {
        for mode in [WorkerMode::Check, WorkerMode::Run] {
            let state = self.state(mode).await?;
            if !state.stopped() || !self.fence.empty(mode)? {
                return Ok(false);
            }
            let socket = self.properties(mode.socket(), UNIT_IFACE).await?;
            if !matches!(
                string(&socket, "ActiveState")?.as_str(),
                "inactive" | "failed"
            ) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    async fn validate_contract(&self) -> Result<()> {
        for mode in [WorkerMode::Check, WorkerMode::Run] {
            for (name, interface) in [
                (mode.service(), SERVICE_IFACE),
                (mode.socket(), SOCKET_IFACE),
            ] {
                let unit = self.properties(name, UNIT_IFACE).await?;
                if string(&unit, "LoadState")? != "loaded"
                    || string(&unit, "FragmentPath")? != format!("/usr/lib/systemd/system/{name}")
                    || boolean(&unit, "Transient")?
                    || !strings(&unit, "DropInPaths")?.is_empty()
                {
                    return Err(WorkerSupervisorError::Untrusted);
                }
                for dependency in ["BindsTo", "After"] {
                    if !strings(&unit, dependency)?
                        .iter()
                        .any(|name| name == "yonik-helper.service")
                    {
                        return Err(WorkerSupervisorError::Untrusted);
                    }
                }
                let properties = self.properties(name, interface).await?;
                if interface == SERVICE_IFACE {
                    validate_service(&properties, mode)?;
                } else {
                    validate_socket(&properties, mode)?;
                }
            }
        }
        Ok(())
    }
}
fn unavailable<T>(_: T) -> WorkerSupervisorError {
    WorkerSupervisorError::Unavailable
}
fn string(p: &Properties, k: &str) -> Result<String> {
    p.get(k)
        .and_then(|v| <&str>::try_from(v).ok())
        .map(str::to_owned)
        .ok_or(WorkerSupervisorError::Untrusted)
}
fn boolean(p: &Properties, k: &str) -> Result<bool> {
    p.get(k)
        .and_then(|v| bool::try_from(v).ok())
        .ok_or(WorkerSupervisorError::Untrusted)
}
fn number32(p: &Properties, k: &str) -> Result<u32> {
    p.get(k)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or(WorkerSupervisorError::Untrusted)
}
fn integer(p: &Properties, k: &str) -> Result<i32> {
    p.get(k)
        .and_then(|v| i32::try_from(v).ok())
        .ok_or(WorkerSupervisorError::Untrusted)
}
fn number64(p: &Properties, k: &str) -> Result<u64> {
    p.get(k)
        .and_then(|v| u64::try_from(v).ok())
        .ok_or(WorkerSupervisorError::Untrusted)
}
fn strings(p: &Properties, k: &str) -> Result<Vec<String>> {
    p.get(k)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .ok_or(WorkerSupervisorError::Untrusted)
}
fn array(p: &Properties, k: &str) -> Result<Vec<u8>> {
    p.get(k)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<u8>::try_from(v).ok())
        .ok_or(WorkerSupervisorError::Untrusted)
}
fn validate_service(p: &Properties, mode: WorkerMode) -> Result<()> {
    let caps = if mode == WorkerMode::Check {
        0
    } else {
        1 << 12
    };
    for (key, value) in [
        ("User", "yonik-worker"),
        ("Group", "yonik-worker"),
        ("Type", "exec"),
        ("Restart", "no"),
        ("KillMode", "control-group"),
        ("Slice", "system.slice"),
        ("StandardInput", "socket"),
        ("StandardOutput", "null"),
        ("StandardError", "null"),
        ("ProtectSystem", "strict"),
        ("ProtectHome", "yes"),
        ("DevicePolicy", "closed"),
    ] {
        if string(p, key)? != value {
            return Err(WorkerSupervisorError::Untrusted);
        }
    }
    for key in [
        "NoNewPrivileges",
        "SendSIGKILL",
        "PrivateTmp",
        "ProtectKernelTunables",
        "ProtectKernelModules",
        "ProtectKernelLogs",
        "ProtectControlGroups",
        "RestrictRealtime",
        "RestrictSUIDSGID",
        "LockPersonality",
        "MemoryDenyWriteExecute",
    ] {
        if !boolean(p, key)? {
            return Err(WorkerSupervisorError::Untrusted);
        }
    }
    if boolean(p, "PrivateNetwork")? != (mode == WorkerMode::Check)
        || boolean(p, "PrivateDevices")? != (mode == WorkerMode::Check)
        || number64(p, "CapabilityBoundingSet")? != caps
        || number64(p, "AmbientCapabilities")? != caps
        || number64(p, "MemoryMax")? != 256 * 1024 * 1024
        || number64(p, "TasksMax")? != 64
        || number32(p, "UMask")? != 0o77
        || integer(p, "KillSignal")? != 15
        || integer(p, "FinalKillSignal")? != 9
        || number64(p, "TimeoutStopUSec")? != 5_000_000
    {
        return Err(WorkerSupervisorError::Untrusted);
    }
    let exec: ExecCommands = p
        .get("ExecStart")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::try_from(v).ok())
        .ok_or(WorkerSupervisorError::Untrusted)?;
    if exec.len() != 1
        || exec[0].0 != "/usr/lib/yonik/yonik-worker"
        || exec[0].1
            != [
                "/usr/lib/yonik/yonik-worker",
                if mode == WorkerMode::Check {
                    "--check"
                } else {
                    "--run"
                },
            ]
        || exec[0].2
    {
        return Err(WorkerSupervisorError::Untrusted);
    }
    let devices: Vec<(String, String)> = p
        .get("DeviceAllow")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::try_from(v).ok())
        .ok_or(WorkerSupervisorError::Untrusted)?;
    if (mode == WorkerMode::Check && !devices.is_empty())
        || (mode == WorkerMode::Run && devices != [("/dev/net/tun".into(), "rw".into())])
    {
        return Err(WorkerSupervisorError::Untrusted);
    }
    Ok(())
}
fn validate_socket(p: &Properties, mode: WorkerMode) -> Result<()> {
    if boolean(p, "Accept")?
        || !boolean(p, "RemoveOnStop")?
        || number32(p, "SocketMode")? != 0o600
        || number32(p, "DirectoryMode")? != 0o700
        || string(p, "SocketUser")? != "root"
        || string(p, "SocketGroup")? != "root"
    {
        return Err(WorkerSupervisorError::Untrusted);
    }
    let listen: Vec<(String, String)> = p
        .get("Listen")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::try_from(v).ok())
        .ok_or(WorkerSupervisorError::Untrusted)?;
    if listen != [("Stream".into(), mode.channel().into())] {
        return Err(WorkerSupervisorError::Untrusted);
    }
    Ok(())
}
async fn bounded<T>(
    cancel: &mut Cancellation,
    deadline: Instant,
    operation: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {biased; _=cancel.cancelled()=>Err(WorkerSupervisorError::Cancelled),_=tokio::time::sleep_until(deadline)=>Err(WorkerSupervisorError::Timeout),result=operation=>result}
}

const FILE_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK);
const DIR_FLAGS: OFlags = FILE_FLAGS.union(OFlags::DIRECTORY);
fn trusted(file: &File, directory: bool) -> Result<()> {
    let m = file.metadata().map_err(unavailable)?;
    if m.uid() != 0
        || m.mode() & 0o022 != 0
        || (directory && !m.is_dir())
        || (!directory && !m.is_file())
    {
        return Err(WorkerSupervisorError::Untrusted);
    }
    Ok(())
}
fn child(parent: &File, name: &str) -> Result<File> {
    let file = File::from(openat(parent, name, DIR_FLAGS, Mode::empty()).map_err(unavailable)?);
    trusted(&file, true)?;
    Ok(file)
}
fn root() -> Result<File> {
    let file = File::from(open("/", DIR_FLAGS, Mode::empty()).map_err(unavailable)?);
    trusted(&file, true)?;
    Ok(file)
}
fn read_bounded(file: File) -> Result<String> {
    let mut contents = String::new();
    file.take(16 * 1024 + 1)
        .read_to_string(&mut contents)
        .map_err(unavailable)?;
    if contents.len() > 16 * 1024 {
        return Err(WorkerSupervisorError::Untrusted);
    }
    Ok(contents)
}
fn verify_installed() -> Result<()> {
    let mut parent = root()?;
    for component in ["usr", "lib", "systemd", "system"] {
        parent = child(&parent, component)?;
    }
    for (name, expected) in [
        (
            "yonik-worker.service",
            include_str!("../../../../packaging/systemd/yonik-worker.service"),
        ),
        (
            "yonik-worker.socket",
            include_str!("../../../../packaging/systemd/yonik-worker.socket"),
        ),
        (
            "yonik-worker-check.service",
            include_str!("../../../../packaging/systemd/yonik-worker-check.service"),
        ),
        (
            "yonik-worker-check.socket",
            include_str!("../../../../packaging/systemd/yonik-worker-check.socket"),
        ),
    ] {
        let file =
            File::from(openat(&parent, name, FILE_FLAGS, Mode::empty()).map_err(unavailable)?);
        trusted(&file, false)?;
        if read_bounded(file)? != expected {
            return Err(WorkerSupervisorError::Untrusted);
        }
    }
    let launcher_directory = child(&child(&child(&root()?, "usr")?, "lib")?, "yonik")?;
    let launcher = File::from(
        openat(
            &launcher_directory,
            "yonik-worker",
            FILE_FLAGS,
            Mode::empty(),
        )
        .map_err(unavailable)?,
    );
    trusted(&launcher, false)?;
    if launcher.metadata().map_err(unavailable)?.mode() & 0o111 == 0 {
        return Err(WorkerSupervisorError::Untrusted);
    }
    Ok(())
}
fn verify_socket(mode: WorkerMode) -> Result<()> {
    let run = child(&root()?, "run")?;
    let directory = child(&run, "yonik-worker")?;
    if directory.metadata().map_err(unavailable)?.mode() & 0o777 != 0o700 {
        return Err(WorkerSupervisorError::Untrusted);
    }
    let stat = rustix::fs::statat(
        &directory,
        if mode == WorkerMode::Check {
            "check.sock"
        } else {
            "run.sock"
        },
        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
    )
    .map_err(unavailable)?;
    if stat.st_uid != 0
        || stat.st_gid != 0
        || stat.st_mode & 0o777 != 0o600
        || rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::Socket
    {
        return Err(WorkerSupervisorError::Untrusted);
    }
    // Каталог root не позволяет UID worker подменить сокет после stat.
    Ok(())
}
fn cgroup_empty(mode: WorkerMode) -> Result<bool> {
    let mut directory = root()?;
    for component in ["sys", "fs", "cgroup"] {
        directory = child(&directory, component)?;
    }
    if fstatfs(&directory).map_err(unavailable)?.f_type as u64 != 0x6367_7270 {
        return Err(WorkerSupervisorError::Untrusted);
    }
    let parent = child(&directory, "system.slice")?;
    let Some(group) = open_worker_group(&parent, mode)? else {
        return Ok(true);
    };
    trusted(&group, true)?;
    let events = File::from(
        openat(&group, "cgroup.events", FILE_FLAGS, Mode::empty()).map_err(unavailable)?,
    );
    empty_events(&read_bounded(events)?)
}
// Вызывающий проверяет root-владельца parent и cgroup2fs до интерпретации ENOENT.
fn open_worker_group(parent: &File, mode: WorkerMode) -> Result<Option<File>> {
    match openat(parent, mode.service(), DIR_FLAGS, Mode::empty()) {
        Ok(fd) => Ok(Some(File::from(fd))),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(_) => Err(WorkerSupervisorError::Unavailable),
    }
}
fn empty_events(contents: &str) -> Result<bool> {
    if !contents.ends_with('\n') {
        return Err(WorkerSupervisorError::Untrusted);
    }
    let mut populated = None;
    for line in contents.lines() {
        let (name, value) = line
            .split_once(' ')
            .ok_or(WorkerSupervisorError::Untrusted)?;
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(WorkerSupervisorError::Untrusted);
        }
        if name == "populated" {
            if populated.is_some() {
                return Err(WorkerSupervisorError::Untrusted);
            }
            populated = Some(match value {
                "0" => true,
                "1" => false,
                _ => return Err(WorkerSupervisorError::Untrusted),
            });
        }
    }
    populated.ok_or(WorkerSupervisorError::Untrusted)
}
#[cfg(test)]
#[path = "systemd_worker_tests.rs"]
mod tests;
