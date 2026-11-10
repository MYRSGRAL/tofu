//! Отдельный владелец БД и локальной SOCKS-сессии; UI получает только DTO.

#[path = "desktop_schedule.rs"]
mod desktop_schedule;
use crate::preferences::{Preferences, SaveOutcome, Store};
use desktop_schedule::DesktopSchedule;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, DirBuilder, File, OpenOptions},
    net::{Ipv4Addr, TcpListener, UdpSocket},
    num::NonZeroU16,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, watch};
use yonik_engine::Engine;
use yonik_import::{Field, IssueCode, ReviewFlag};
use yonik_linux::{
    BinarySnapshot, Cancellation, CheckedConfig, CredentialError, EncryptedSubscriptionRepository,
    FetchError, FetchRoute, HostError, LocalSession, NetworkMonitor, NetworkSnapshot, NetworkStamp,
    NetworkWatch, PendingRefresh, ProbeResult, ProbeTarget, ProfileQuery, RefreshResult,
    ResolvedProfile, SubscriptionError, SubscriptionIdentity, SubscriptionService,
    SubscriptionSource, TCP_SERIES_SAMPLES, TcpProbeSeriesResult, TcpProbeStatus, TcpProbeTarget,
    check_tcp_series, subscription_master_key,
};
use zeroize::Zeroizing;

/// Явный ephemeral key предназначен для отдельной тестовой БД; fallback отсутствует.
#[derive(Clone)]
pub struct HostConfig {
    data_dir: PathBuf,
    engine_dir: PathBuf,
    ephemeral_key: Option<Zeroizing<[u8; 32]>>,
    listen_port: Option<NonZeroU16>,
    system_subscription_policy: bool,
    schedule_observer: Option<Arc<dyn Fn(yonik_linux::SubscriptionPolicy, bool) + Send + Sync>>,
}
impl HostConfig {
    pub fn new(data_dir: PathBuf, engine_dir: PathBuf) -> Self {
        Self {
            data_dir,
            engine_dir,
            ephemeral_key: None,
            listen_port: None,
            system_subscription_policy: true,
            schedule_observer: None,
        }
    }
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    pub fn engine_dir(&self) -> &Path {
        &self.engine_dir
    }
    pub fn with_ephemeral_key(mut self, key: [u8; 32]) -> Self {
        self.ephemeral_key = Some(Zeroizing::new(key));
        self.system_subscription_policy = false;
        self
    }
    /// Доверенная настройка platform provider; после ephemeral key требуется
    /// явное включение. GUI JSON не имеет доступа к этой настройке.
    pub fn with_system_subscription_policy(mut self, enabled: bool) -> Self {
        self.system_subscription_policy = enabled;
        self
    }
    /// Read-only наблюдение effective policy после загрузки schedule, вне GUI JSON.
    pub fn with_schedule_observer(
        mut self,
        observer: impl Fn(yonik_linux::SubscriptionPolicy, bool) + Send + Sync + 'static,
    ) -> Self {
        self.schedule_observer = Some(Arc::new(observer));
        self
    }
    pub fn with_listen_port(mut self, port: NonZeroU16) -> Self {
        self.listen_port = Some(port);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesktopError(&'static str);
impl DesktopError {
    pub fn key(self) -> &'static str {
        self.0
    }
}
impl fmt::Display for DesktopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for DesktopError {}

type Result<T> = std::result::Result<T, DesktopError>;
type Callback = Arc<dyn Fn(String) + Send + Sync>;
pub(crate) const DEFAULT_PROBE_URL: &str = "https://www.gstatic.com/generate_204";
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    RoutingCatalog {
        processes: bool,
        query: String,
        offset: usize,
    },
    EditRouting {
        expected_revision: i64,
        request_id: u64,
        create: bool,
        rule: crate::editor::RuleInput,
        confirm_broad: bool,
    },
    RoutingOrder {
        expected_revision: i64,
        request_id: u64,
        ids: Vec<u64>,
    },
    RoutingDefault {
        expected_revision: i64,
        request_id: u64,
        action: String,
    },
    RoutingDelete {
        expected_revision: i64,
        request_id: u64,
        id: u64,
    },
    SubscriptionVisibility {
        visible: bool,
    },
    Reload,
    AddSource {
        name: String,
        url: String,
        local_exception: bool,
        route: Route,
        #[serde(default)]
        hwid_enabled: bool,
    },
    ImportSource {
        name: String,
        url: String,
        local_exception: bool,
        route: Route,
        #[serde(default)]
        hwid_enabled: bool,
    },
    RenameSource {
        id: i64,
        name: String,
    },
    DeleteSource {
        id: i64,
    },
    CancelRefresh {
        id: i64,
        #[serde(default)]
        token: String,
    },
    SetSourceHwid {
        enabled: bool,
    },
    SelectSource {
        id: i64,
    },
    SourcePage {
        offset: usize,
    },
    Refresh,
    ApproveRefresh,
    DiscardRefresh,
    ReviewPage {
        offset: usize,
    },
    Query {
        search: String,
        favorites_only: bool,
        offset: usize,
    },
    Favorite {
        id: i64,
        value: bool,
    },
    SelectProfile {
        id: i64,
        revision: i64,
    },
    SetLanguage {
        language: String,
    },
    ResetListenPort,
    SetEngine {
        engine: EngineChoice,
    },
    SetListenPort {
        port: NonZeroU16,
    },
    SetProbeTarget {
        url: String,
        expected_status: u16,
    },
    ResetProbeTarget,
    CheckTcp,
    CancelTcp,
    Connect,
    Stop,
    Shutdown,
}
impl Command {
    fn is_source_operation(&self) -> bool {
        matches!(
            self,
            Self::Reload
                | Self::AddSource { .. }
                | Self::ImportSource { .. }
                | Self::RenameSource { .. }
                | Self::DeleteSource { .. }
                | Self::CancelRefresh { .. }
                | Self::SetSourceHwid { .. }
                | Self::SelectSource { .. }
                | Self::SourcePage { .. }
                | Self::Refresh
                | Self::ApproveRefresh
                | Self::DiscardRefresh
                | Self::ReviewPage { .. }
                | Self::Query { .. }
                | Self::Favorite { .. }
                | Self::SelectProfile { .. }
        )
    }
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Route {
    Direct,
    Proxy,
}
#[derive(Clone, Copy, Deserialize, Serialize)]
enum EngineChoice {
    #[serde(rename = "xray")]
    Xray,
    #[serde(rename = "sing-box")]
    SingBox,
}
impl EngineChoice {
    fn engine(self) -> Engine {
        match self {
            Self::Xray => Engine::Xray,
            Self::SingBox => Engine::SingBox,
        }
    }
}
struct Envelope {
    generation: u64,
    tcp_generation: u64,
    command: Command,
}
#[derive(Clone, Copy, Default)]
struct Priority {
    generation: u64,
    shutdown: bool,
}
struct Control {
    generation: AtomicU64,
    priority: watch::Sender<Priority>,
    tcp_priority: watch::Sender<u64>,
    subscription_policy: watch::Sender<yonik_linux::SubscriptionPolicy>,
    subscription_override: AtomicBool,
    subscription_visible: watch::Sender<bool>,
    closed: (Mutex<bool>, Condvar),
}
impl Control {
    fn cancel_tcp(&self) {
        // Тот же lock watch сериализует cancel и generation для queued TCP requests.
        self.tcp_priority
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }
    fn interrupt(&self, shutdown: bool) {
        // send_modify сериализует Stop/Shutdown, shutdown остаётся sticky.
        self.priority.send_modify(|state| {
            state.generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
            state.shutdown |= shutdown;
        });
    }
    fn closed(&self) {
        let (lock, condition) = &self.closed;
        if let Ok(mut closed) = lock.lock() {
            *closed = true;
            condition.notify_all();
        }
    }
}

pub struct DesktopHost {
    commands: mpsc::Sender<Envelope>,
    control: Arc<Control>,
}
impl DesktopHost {
    pub fn start(
        config: HostConfig,
        callback: impl Fn(String) + Send + Sync + 'static,
    ) -> Result<Self> {
        let (commands, receiver) = mpsc::channel(128);
        let (priority, priority_rx) = watch::channel(Priority::default());
        let (tcp_priority, _) = watch::channel(0);
        let control = Arc::new(Control {
            generation: AtomicU64::new(0),
            priority,
            tcp_priority,
            subscription_policy: watch::channel(Default::default()).0,
            subscription_override: std::sync::atomic::AtomicBool::new(false),
            subscription_visible: watch::channel(false).0,
            closed: (Mutex::new(false), Condvar::new()),
        });
        let worker_control = control.clone();
        let callback: Callback = Arc::new(callback);
        std::thread::Builder::new()
            .name("yonik-backend".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(run(
                        config,
                        receiver,
                        priority_rx,
                        worker_control,
                        callback,
                    )),
                    Err(_) => {
                        let mut snapshot = Snapshot::new(config.ephemeral_key.is_some());
                        snapshot.port = config.listen_port.map_or(0, NonZeroU16::get);
                        snapshot.error = Some("startup");
                        snapshot.connection_error = Some("startup");
                        snapshot.closed = true;
                        snapshot.emit(&callback);
                        worker_control.closed();
                    }
                }
            })
            .map_err(|_| DesktopError("startup"))?;
        Ok(Self { commands, control })
    }
    /// Доверенный platform provider. Unknown/default запрещает автоматические загрузки.
    pub fn set_subscription_policy(&self, policy: yonik_linux::SubscriptionPolicy) -> Result<()> {
        self.control
            .subscription_override
            .store(true, Ordering::Release);
        self.control.subscription_policy.send_replace(policy);
        Ok(())
    }
    pub fn dispatch(&self, request: &str) -> Result<()> {
        if request.len() > 16 * 1024 {
            return Err(DesktopError("invalid_input"));
        }
        let command: Command =
            serde_json::from_str(request).map_err(|_| DesktopError("invalid_input"))?;
        if matches!(
            command,
            Command::Reload
                | Command::Refresh
                | Command::ApproveRefresh
                | Command::DiscardRefresh
                | Command::Connect
                | Command::Stop
                | Command::Shutdown
                | Command::ResetProbeTarget
                | Command::ResetListenPort
                | Command::CheckTcp
                | Command::CancelTcp
        ) {
            let object: serde_json::Value =
                serde_json::from_str(request).map_err(|_| DesktopError("invalid_input"))?;
            if object.as_object().is_none_or(|object| object.len() != 1) {
                return Err(DesktopError("invalid_input"));
            }
        }
        validate(&command)?;
        match command {
            Command::SubscriptionVisibility { visible } => {
                self.control.subscription_visible.send_replace(visible);
                Ok(())
            }
            Command::Shutdown => {
                self.shutdown();
                Ok(())
            }
            Command::Stop => {
                self.control.interrupt(false);
                Ok(())
            }
            Command::CancelTcp => {
                self.control.cancel_tcp();
                Ok(())
            }
            command => self
                .commands
                .try_send(Envelope {
                    generation: self.control.generation.load(Ordering::SeqCst),
                    tcp_generation: *self.control.tcp_priority.borrow(),
                    command,
                })
                .map_err(|_| DesktopError("busy")),
        }
    }
    pub fn shutdown(&self) {
        self.control.interrupt(true);
    }
    /// Только для тестов/не-UI fallback. Qt ожидает snapshot.closed асинхронно.
    pub fn wait_closed(&self, timeout: Duration) -> bool {
        let (lock, condition) = &self.control.closed;
        let Ok(guard) = lock.lock() else {
            return false;
        };
        condition
            .wait_timeout_while(guard, timeout, |closed| !*closed)
            .is_ok_and(|(closed, _)| *closed)
    }
}
impl Drop for DesktopHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn validate(command: &Command) -> Result<()> {
    let text = |s: &str, max| s.len() <= max && !s.chars().any(char::is_control);
    let valid = match command {
        Command::AddSource { name, url, .. } | Command::ImportSource { name, url, .. } => {
            !name.trim().is_empty() && text(name, 512) && text(url, 8192)
        }
        Command::RenameSource { id, name } => *id > 0 && !name.trim().is_empty() && text(name, 512),
        Command::Query { search, offset, .. } => text(search, 512) && *offset <= 40_000,
        Command::SourcePage { offset } => *offset <= 40_000,
        Command::ReviewPage { offset } => *offset <= 160_000,
        Command::SetProbeTarget { url, .. } => text(url, 2048),
        Command::SelectSource { id }
        | Command::Favorite { id, .. }
        | Command::DeleteSource { id }
        | Command::CancelRefresh { id, .. } => *id > 0,
        Command::SelectProfile { id, revision } => *id > 0 && *revision > 0,
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(DesktopError("invalid_input"))
    }
}

#[derive(Serialize)]
struct SourceDto {
    id: i64,
    name: String,
    hwid_enabled: bool,
}
#[derive(Serialize)]
struct ProfileDto {
    id: i64,
    name: String,
    host: String,
    port: u16,
    protocol: String,
    favorite: bool,
    available: bool,
}
#[derive(Serialize)]
struct PendingDto {
    accepted: usize,
    rejected: usize,
    flagged: usize,
    native_json: bool,
    rows: Vec<ReviewRow>,
    rows_total: usize,
    rows_offset: usize,
}
#[derive(Serialize)]
struct ReviewRow {
    kind: &'static str,
    position: usize,
    code: &'static str,
    field: Option<&'static str>,
}

fn pending_dto(pending: &PendingRefresh, offset: usize) -> Result<PendingDto> {
    let report = pending.report();
    let total = report.issues.len()
        + report
            .servers
            .iter()
            .map(|server| server.review_flags().len())
            .sum::<usize>();
    if offset > total {
        return Err(DesktopError("invalid_input"));
    }
    // Только закрытые enum-коды и позиции: ни URI, ни пользовательские параметры.
    let issues = report.issues.iter().map(|issue| ReviewRow {
        kind: "rejected",
        position: issue.line,
        code: match issue.code {
            IssueCode::MalformedUri => "malformed_uri",
            IssueCode::UnsupportedProtocol => "unsupported_protocol",
            IssueCode::InvalidPort => "invalid_port",
            IssueCode::InvalidHost => "invalid_host",
            IssueCode::InvalidCredentials => "invalid_credentials",
            IssueCode::InvalidEncoding => "invalid_encoding",
            IssueCode::InvalidJson => "invalid_json",
            IssueCode::TooLarge => "too_large",
            IssueCode::DuplicateParameter => "duplicate_parameter",
            IssueCode::ConflictingParameter => "conflicting_parameter",
        },
        field: Some(match issue.field {
            Field::Source => "source",
            Field::Endpoint => "endpoint",
            Field::Credentials => "credentials",
            Field::Parameters => "parameters",
            Field::Payload => "payload",
        }),
    });
    let flags = report
        .servers
        .iter()
        .enumerate()
        .flat_map(|(index, server)| {
            server.review_flags().iter().map(move |flag| ReviewRow {
                kind: "flagged",
                // Это номер принятого профиля в отчёте, а не строка исходного URI.
                position: index + 1,
                code: match flag {
                    ReviewFlag::UnknownParameter => "unknown_parameter",
                    ReviewFlag::UnsupportedTransport => "unsupported_transport",
                    ReviewFlag::UnsupportedSecurity => "unsupported_security",
                    ReviewFlag::UnsupportedCipher => "unsupported_cipher",
                    ReviewFlag::UnsupportedOption => "unsupported_option",
                    ReviewFlag::UnsafeOption => "unsafe_option",
                    ReviewFlag::LegacyOption => "legacy_option",
                },
                field: None,
            })
        });
    let summary = pending.summary();
    Ok(PendingDto {
        accepted: summary.accepted,
        rejected: summary.rejected,
        flagged: summary.flagged,
        native_json: summary.native_json,
        rows: issues.chain(flags).skip(offset).take(100).collect(),
        rows_total: total,
        rows_offset: offset,
    })
}
#[derive(Serialize)]
struct RoutingAck {
    request_id: u64,
    saved_id: Option<u64>,
    error: Option<&'static str>,
}
enum RoutingMutation {
    Put {
        rule: crate::editor::RuleInput,
        confirm_broad: bool,
        create: bool,
    },
    Order(Vec<u64>),
    Delete(u64),
    Default(String),
}
#[derive(Serialize)]
struct Snapshot {
    routing_revision: Option<i64>,
    routing_ack: Option<RoutingAck>,
    routing_catalog: serde_json::Value,
    routing: crate::editor::Draft,
    language: String,
    preferences_warning: bool,
    preferences_durability_warning: bool,
    build_id: &'static str,
    source_hwid_enabled: bool,
    ready: bool,
    busy: bool,
    error: Option<&'static str>,
    source_error: Option<&'static str>,
    connection_error: Option<&'static str>,
    pending: Option<PendingDto>,
    sources: Vec<SourceDto>,
    sources_total: usize,
    sources_offset: usize,
    source_id: Option<i64>,
    source_name: Option<String>,
    refresh_source_id: Option<i64>,
    refresh_token: Option<String>,
    profiles: Vec<ProfileDto>,
    total: usize,
    offset: usize,
    revision: i64,
    status: &'static str,
    selected_profile: Option<i64>,
    selected_name: Option<String>,
    engine: EngineChoice,
    port: u16,
    search: String,
    favorites_only: bool,
    can_connect: bool,
    closed: bool,
    ephemeral: bool,
    can_stop: bool,
    active_name: Option<String>,
    active_engine: Option<EngineChoice>,
    probe_custom: bool,
    probe_persistent: bool,
    probe_expected_status: u16,
    probe_result: Option<HealthResultDto>,
    tcp_busy: bool,
    tcp_name: Option<String>,
    tcp_result: Option<TcpResultDto>,
    network_epoch: Option<String>,
    network_observed: bool,
    network_routes: bool,
    network_resolver: bool,
    network_manager: bool,
    tcp_invalidated: bool,
}
impl Snapshot {
    fn new(ephemeral: bool) -> Self {
        Self {
            routing: crate::editor::Draft::default(),
            routing_revision: Some(0),
            routing_ack: None,
            routing_catalog: serde_json::json!({"items":[],"total":0,"offset":0,"incomplete":false}),
            language: "ru".into(),
            preferences_warning: false,
            preferences_durability_warning: false,
            build_id: env!("YONIK_BUILD_ID"),
            source_hwid_enabled: false,
            ready: false,
            busy: false,
            error: None,
            source_error: None,
            connection_error: None,
            pending: None,
            sources: Vec::new(),
            sources_total: 0,
            sources_offset: 0,
            source_id: None,
            source_name: None,
            refresh_source_id: None,
            refresh_token: None,
            profiles: Vec::new(),
            total: 0,
            offset: 0,
            revision: 0,
            status: "Disconnected",
            selected_profile: None,
            selected_name: None,
            engine: EngineChoice::Xray,
            port: 0,
            search: String::new(),
            favorites_only: false,
            can_connect: false,
            closed: false,
            ephemeral,
            can_stop: false,
            active_name: None,
            active_engine: None,
            probe_custom: false,
            probe_persistent: true,
            probe_expected_status: 204,
            probe_result: None,
            tcp_busy: false,
            tcp_name: None,
            tcp_result: None,
            network_epoch: None,
            network_observed: false,
            network_routes: false,
            network_resolver: false,
            network_manager: false,
            tcp_invalidated: false,
        }
    }
    fn begin_command(&mut self, command: &Command) {
        self.error = None;
        if command.is_source_operation() {
            if matches!(
                command,
                Command::AddSource { .. }
                    | Command::ImportSource { .. }
                    | Command::SetSourceHwid { .. }
                    | Command::SelectSource { .. }
                    | Command::DeleteSource { .. }
                    | Command::Refresh
                    | Command::ApproveRefresh
                    | Command::DiscardRefresh
            ) {
                self.source_error = None;
            }
        } else {
            self.connection_error = None;
        }
    }
    fn command_error(&mut self, source: bool, error: &'static str) {
        self.error = Some(error);
        if source {
            self.source_error = Some(error);
        } else {
            self.connection_error = Some(error);
        }
    }
    fn emit(&self, callback: &Callback) {
        #[derive(Serialize)]
        struct View<'a> {
            #[serde(flatten)]
            snapshot: &'a Snapshot,
            listen_address: String,
        }
        let view = View {
            snapshot: self,
            listen_address: format!("127.0.0.1:{}", self.port),
        };
        if let Ok(json) = serde_json::to_string(&view) {
            callback(json);
        }
    }
}

// Ядро не принимает готовые FD: проверка сужает окно гонки, но не резервирует порт.
// Проверяем также UDP, поскольку SOCKS inbound Xray включает udp:true.
fn available_port(requested: u16) -> Result<NonZeroU16> {
    for _ in 0..16 {
        let tcp = TcpListener::bind((Ipv4Addr::LOCALHOST, requested))
            .map_err(|_| DesktopError("listen_port_busy"))?;
        let port = tcp
            .local_addr()
            .map_err(|_| DesktopError("listen_port_busy"))?
            .port();
        if UdpSocket::bind((Ipv4Addr::LOCALHOST, port)).is_ok() {
            return NonZeroU16::new(port).ok_or(DesktopError("listen_port_busy"));
        }
        if requested != 0 {
            break;
        }
    }
    Err(DesktopError("listen_port_busy"))
}

fn private_directory(path: &Path) -> Result<(PathBuf, File)> {
    if !path.is_absolute() {
        return Err(DesktopError("storage"));
    }
    match DirBuilder::new().recursive(true).mode(0o700).create(path) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err(DesktopError("storage")),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| DesktopError("storage"))?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(DesktopError("storage"));
    }
    let canonical = path.canonicalize().map_err(|_| DesktopError("storage"))?;
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::DIRECTORY).bits() as i32)
        .open(&canonical)
        .map_err(|_| DesktopError("storage"))?;
    rustix::fs::flock(
        &directory,
        rustix::fs::FlockOperation::NonBlockingLockExclusive,
    )
    .map_err(|_| DesktopError("busy"))?;
    Ok((canonical, directory))
}

async fn open(
    config: &mut HostConfig,
) -> Result<(SubscriptionService, File, EncryptedSubscriptionRepository)> {
    let path = config.data_dir.clone();
    let (directory, lock) = tokio::task::spawn_blocking(move || private_directory(&path))
        .await
        .map_err(|_| DesktopError("storage"))??;
    let path = directory.join("subscriptions.sqlite");
    let key = match config.ephemeral_key.take() {
        Some(key) => key,
        None => subscription_master_key(!path.exists())
            .await
            .map_err(|error| {
                DesktopError(match error {
                    CredentialError::Missing => "keyring_missing",
                    _ => "keyring_locked",
                })
            })?,
    };
    let repo = EncryptedSubscriptionRepository::open(path, key)
        .await
        .map_err(|_| DesktopError("storage"))?;
    Ok((SubscriptionService::new(repo.clone()), lock, repo))
}

struct Actor {
    routing_repository: EncryptedSubscriptionRepository,
    routing_revision: Option<i64>,
    preferences: Preferences,
    preferences_store: Store,
    service: SubscriptionService,
    state: Snapshot,
    callback: Callback,
    selected: Option<ResolvedProfile>,
    pending: Option<Box<PendingRefresh>>,
    automatic_port: bool,
    probe_url: Zeroizing<String>,
    network: Option<NetworkWatch>,
}
impl Actor {
    async fn save_routing(&mut self, candidate: crate::editor::Draft) -> Result<()> {
        candidate.policy().map_err(DesktopError)?;
        let expected = self
            .routing_revision
            .ok_or(DesktopError("routing_storage"))?;
        let revision = self
            .routing_repository
            .save_routing_draft(expected, candidate.json().into_bytes())
            .await
            .map_err(|error| {
                DesktopError(if error == yonik_linux::RepositoryError::Conflict {
                    "routing_stale"
                } else {
                    "routing_storage"
                })
            })?;
        self.routing_revision = Some(revision);
        self.state.routing_revision = Some(revision);
        self.state.routing = candidate;
        Ok(())
    }

    async fn mutate_routing(
        &mut self,
        expected_revision: i64,
        request_id: u64,
        mutation: RoutingMutation,
    ) -> Result<()> {
        let saved_id = match &mutation {
            RoutingMutation::Put { rule, .. } => Some(rule.id),
            _ => None,
        };
        let result = async {
            if request_id == 0 || request_id > 9_007_199_254_740_991 {
                return Err(DesktopError("invalid_input"));
            }
            let current = self
                .routing_revision
                .ok_or(DesktopError("routing_storage"))?;
            if expected_revision != current {
                return Err(DesktopError("routing_stale"));
            }
            let mut candidate = self.state.routing.clone();
            match mutation {
                RoutingMutation::Put {
                    rule,
                    confirm_broad,
                    create,
                } => {
                    let exists = candidate.rules.iter().any(|r| r.id == rule.id);
                    if exists == create {
                        return Err(DesktopError("id"));
                    }
                    candidate.put(rule, confirm_broad).map_err(DesktopError)?;
                }
                RoutingMutation::Order(ids) => {
                    if ids.len() != candidate.rules.len() {
                        return Err(DesktopError("id"));
                    }
                    let mut available: std::collections::HashMap<_, _> =
                        candidate.rules.into_iter().map(|r| (r.id, r)).collect();
                    candidate.rules = ids
                        .into_iter()
                        .map(|id| available.remove(&id).ok_or(DesktopError("id")))
                        .collect::<Result<Vec<_>>>()?;
                }
                RoutingMutation::Delete(id) => {
                    let index = candidate
                        .rules
                        .iter()
                        .position(|r| r.id == id)
                        .ok_or(DesktopError("id"))?;
                    candidate.rules.remove(index);
                }
                RoutingMutation::Default(action) => candidate.default_action = action,
            }
            self.save_routing(candidate).await
        }
        .await;
        self.state.routing_ack = Some(RoutingAck {
            request_id,
            saved_id: if result.is_ok() { saved_id } else { None },
            error: result.as_ref().err().map(|e| e.key()),
        });
        result
    }

    fn save_preferences(&mut self, preferences: Preferences) -> Result<()> {
        let outcome = self
            .preferences_store
            .save(&preferences)
            .map_err(|_| DesktopError("preferences_write"))?;
        self.state.preferences_durability_warning =
            outcome == SaveOutcome::PublishedDurabilityUnknown;
        self.preferences = preferences;
        Ok(())
    }
    fn prepare_port(&mut self) -> Result<()> {
        match available_port(self.state.port) {
            Ok(_) => Ok(()),
            Err(_) if self.automatic_port => {
                self.state.port = available_port(0)?.get();
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
    async fn refresh(&mut self, generation: u64) -> Result<RefreshTask> {
        let id = self
            .state
            .source_id
            .ok_or(DesktopError("source_required"))?;
        static NEXT_REFRESH: AtomicU64 = AtomicU64::new(1);
        let token = NEXT_REFRESH
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_add(1)
            })
            .map_err(|_| DesktopError("refresh_required"))?
            .to_string();
        self.pending = None;
        self.state.pending = None;
        let identity = SubscriptionIdentity {
            id,
            generation: self
                .service
                .schedule_generation(id)
                .await
                .map_err(map_subscription)?
                .ok_or(DesktopError("source_required"))?,
        };
        let service = self.service.clone();
        let (cancel, receiver) = watch::channel(false);
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        service
            .reserve_manual_schedule(identity, wall)
            .await
            .map_err(map_subscription)?;
        let task = tokio::spawn(async move {
            let host = yonik_linux::FetchHost::new(Duration::from_secs(30))
                .map_err(SubscriptionError::Fetch)?;
            service
                .refresh_with_host(identity.id, Some(identity.generation), &host, receiver)
                .await
        });
        self.state.busy = true;
        self.state.refresh_source_id = Some(id);
        self.state.refresh_token = Some(token.clone());
        Ok((generation, cancel, task, identity, token))
    }
    fn tcp_key(&self) -> Option<TcpKey> {
        let selected = self.selected.as_ref()?;
        Some(TcpKey {
            source: selected.profile.source_id,
            profile: selected.profile.id,
            revision: selected.snapshot_revision,
        })
    }
    fn probe_target(&self) -> Result<ProbeTarget> {
        ProbeTarget::new(&self.probe_url, PROBE_TIMEOUT)
            .and_then(|target| target.with_expected_status(self.state.probe_expected_status))
            .map_err(|_| DesktopError("probe_target_invalid"))
    }
    fn update(&mut self, update: SessionUpdate) {
        apply_session_update(
            &mut self.state,
            update,
            self.network.as_ref().map(NetworkWatch::current),
        );
    }
    fn emit(&mut self) {
        self.state.can_connect = self.selected.is_some()
            && !self.state.can_stop
            && !self.state.busy
            && self.state.pending.is_none();
        self.state.emit(&self.callback);
    }
    fn clear_selection(&mut self) {
        self.selected = None;
        self.state.selected_profile = None;
        self.state.selected_name = None;
        self.state.tcp_result = None;
    }
    async fn reload(&mut self) -> Result<()> {
        self.sources().await?;
        if self.state.source_id.is_none() {
            self.state.source_id = self.state.sources.first().map(|source| source.id);
            self.clear_selection();
        }
        self.page().await
    }
    async fn sources(&mut self) -> Result<()> {
        let page = self
            .service
            .sources_page(self.state.sources_offset)
            .await
            .map_err(map_subscription)?;
        self.state.sources_total = page.total;
        self.state.sources_offset = page.offset;
        self.state.sources = page
            .sources
            .into_iter()
            .map(|source| SourceDto {
                id: source.id,
                name: source.name,
                hwid_enabled: source.hwid_enabled,
            })
            .collect();
        Ok(())
    }
    async fn page(&mut self) -> Result<()> {
        self.state.profiles.clear();
        self.state.total = 0;
        self.state.revision = 0;
        self.state.source_hwid_enabled = false;
        self.state.source_name = None;
        let Some(id) = self.state.source_id else {
            return Ok(());
        };
        let summary = self
            .service
            .source_summary(id)
            .await
            .map_err(map_subscription)?;
        self.state.source_hwid_enabled = summary.hwid_enabled;
        self.state.source_name = Some(summary.name);
        let page = self
            .service
            .profiles(
                id,
                ProfileQuery {
                    search: self.state.search.clone(),
                    favorites_only: self.state.favorites_only,
                    include_unavailable: true,
                    offset: self.state.offset,
                    limit: 200,
                    ..ProfileQuery::default()
                },
            )
            .await
            .map_err(map_subscription)?;
        self.state.total = page.total;
        self.state.revision = page.snapshot_revision;
        self.state.profiles = page
            .profiles
            .into_iter()
            .map(|p| ProfileDto {
                id: p.id,
                name: p.name,
                host: p.host,
                port: p.port,
                protocol: p.protocol,
                favorite: p.favorite,
                available: p.available,
            })
            .collect();
        // Подтверждение нового snapshot всегда требует повторного выбора.
        if self
            .selected
            .as_ref()
            .is_some_and(|p| p.snapshot_revision != self.state.revision)
        {
            self.clear_selection();
        }
        Ok(())
    }
    async fn command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::RoutingCatalog {
                processes,
                query,
                offset,
            } => {
                self.state.routing_catalog = tokio::task::spawn_blocking(move || {
                    crate::editor::catalog(processes, &query, offset)
                })
                .await
                .map_err(|_| DesktopError("catalog"))?
                .map_err(DesktopError)?;
                Ok(())
            }
            Command::EditRouting {
                expected_revision,
                request_id,
                create,
                rule,
                confirm_broad,
            } => {
                self.mutate_routing(
                    expected_revision,
                    request_id,
                    RoutingMutation::Put {
                        rule,
                        confirm_broad,
                        create,
                    },
                )
                .await
            }
            Command::RoutingOrder {
                expected_revision,
                request_id,
                ids,
            } => {
                self.mutate_routing(expected_revision, request_id, RoutingMutation::Order(ids))
                    .await
            }
            Command::RoutingDelete {
                expected_revision,
                request_id,
                id,
            } => {
                self.mutate_routing(expected_revision, request_id, RoutingMutation::Delete(id))
                    .await
            }
            Command::RoutingDefault {
                expected_revision,
                request_id,
                action,
            } => {
                self.mutate_routing(
                    expected_revision,
                    request_id,
                    RoutingMutation::Default(action),
                )
                .await
            }
            Command::Reload => self.reload().await,
            Command::SourcePage { offset } => {
                self.state.sources_offset = offset;
                self.sources().await
            }
            Command::AddSource {
                name,
                url,
                local_exception,
                route,
                hwid_enabled,
            } => {
                if matches!(route, Route::Proxy) {
                    return Err(DesktopError("unsupported_proxy"));
                }
                let source =
                    SubscriptionSource::new(name, url, local_exception, FetchRoute::Direct)
                        .and_then(|source| source.with_hwid(hwid_enabled))
                        .map_err(map_subscription)?;
                let id = self.service.add(source).await.map_err(map_subscription)?;
                self.state.source_id = Some(id);
                self.state.offset = 0;
                self.pending = None;
                self.state.pending = None;
                self.clear_selection();
                self.reload().await
            }
            Command::RenameSource { id, name } => {
                self.service
                    .rename_source(id, name.clone())
                    .await
                    .map_err(map_subscription)?;
                if self.state.source_id == Some(id) {
                    self.state.source_name = Some(name);
                }
                self.sources().await
            }
            Command::DeleteSource { id } => {
                if self.state.can_stop {
                    return Err(DesktopError("source_in_use"));
                }
                if self.pending.is_some() {
                    return Err(DesktopError("busy"));
                }
                self.service
                    .delete_source(id)
                    .await
                    .map_err(map_subscription)?;
                self.state.sources_offset = 0;
                self.sources().await?;
                if self.state.source_id == Some(id) {
                    self.state.source_id = self.state.sources.first().map(|source| source.id);
                    self.state.offset = 0;
                    self.clear_selection();
                    self.page().await?;
                }
                Ok(())
            }
            Command::SetSourceHwid { enabled } => {
                let id = self
                    .state
                    .source_id
                    .ok_or(DesktopError("source_required"))?;
                self.service
                    .set_source_hwid(id, enabled)
                    .await
                    .map_err(map_subscription)?;
                self.pending = None;
                self.state.pending = None;
                self.clear_selection();
                self.reload().await
            }
            Command::SelectSource { id } => {
                if !self.state.sources.iter().any(|s| s.id == id) {
                    return Err(DesktopError("source_required"));
                }
                self.state.source_id = Some(id);
                self.state.offset = 0;
                self.pending = None;
                self.state.pending = None;
                self.clear_selection();
                self.page().await
            }
            Command::ApproveRefresh => {
                let pending = self.pending.take().ok_or(DesktopError("review_required"))?;
                let identity = pending.identity();
                self.state.pending = None;
                self.service
                    .commit(pending.approve_after_review())
                    .await
                    .map_err(map_subscription)?;
                self.persist_refresh_completion(identity, true).await?;
                self.clear_selection();
                self.page().await
            }
            Command::DiscardRefresh => {
                if let Some(pending) = self.pending.as_ref() {
                    self.persist_refresh_completion(pending.identity(), true)
                        .await?;
                }
                self.pending = None;
                self.state.pending = None;
                Ok(())
            }
            Command::ReviewPage { offset } => {
                let pending = self
                    .pending
                    .as_ref()
                    .ok_or(DesktopError("review_required"))?;
                self.state.pending = Some(pending_dto(pending, offset)?);
                Ok(())
            }
            Command::Query {
                search,
                favorites_only,
                offset,
            } => {
                self.state.search = search;
                self.state.favorites_only = favorites_only;
                self.state.offset = offset;
                self.page().await
            }
            Command::Favorite { id, value } => {
                self.service
                    .set_favorite(id, value)
                    .await
                    .map_err(map_subscription)?;
                self.page().await
            }
            Command::SelectProfile { id, revision } => {
                let profile = self
                    .service
                    .select_profile(id, revision)
                    .await
                    .map_err(|_| DesktopError("profile_unavailable"))?;
                if Some(profile.profile.source_id) != self.state.source_id
                    || profile.snapshot_revision != self.state.revision
                {
                    return Err(DesktopError("profile_unavailable"));
                }
                if self.state.selected_profile != Some(id) {
                    self.state.tcp_result = None;
                }
                self.state.selected_profile = Some(id);
                self.state.selected_name = Some(profile.profile.name.clone());
                self.selected = Some(profile);
                Ok(())
            }
            Command::SetLanguage { language } => {
                if !matches!(language.as_str(), "ru" | "en") {
                    return Err(DesktopError("invalid_input"));
                }
                let mut preferences = self.preferences.clone();
                preferences.language = language.clone();
                self.save_preferences(preferences)?;
                self.state.language = language;
                Ok(())
            }
            Command::ResetListenPort => {
                if self.state.can_stop {
                    return Err(DesktopError("busy"));
                }
                let port = available_port(0)?.get();
                let mut preferences = self.preferences.clone();
                preferences.port = None;
                self.save_preferences(preferences)?;
                self.state.port = port;
                self.automatic_port = true;
                Ok(())
            }
            Command::SetEngine { engine } => {
                if self.state.can_stop {
                    return Err(DesktopError("busy"));
                }
                let mut preferences = self.preferences.clone();
                preferences.engine = match engine {
                    EngineChoice::Xray => "xray",
                    EngineChoice::SingBox => "sing-box",
                }
                .into();
                self.save_preferences(preferences)?;
                self.state.engine = engine;
                Ok(())
            }
            Command::SetListenPort { port } => {
                if self.state.can_stop {
                    return Err(DesktopError("busy"));
                }
                let mut preferences = self.preferences.clone();
                preferences.port = Some(port);
                self.save_preferences(preferences)?;
                self.state.port = port.get();
                self.automatic_port = false;
                Ok(())
            }
            Command::SetProbeTarget {
                url,
                expected_status,
            } => {
                if self.state.can_stop {
                    return Err(DesktopError("busy"));
                }
                let url = Zeroizing::new(url);
                ProbeTarget::new(&url, PROBE_TIMEOUT)
                    .and_then(|target| target.with_expected_status(expected_status))
                    .map_err(|_| DesktopError("probe_target_invalid"))?;
                let mut preferences = self.preferences.clone();
                let persistent = !url.contains(['@', '?', '#']);
                preferences.probe_url = if persistent {
                    url.to_string()
                } else {
                    DEFAULT_PROBE_URL.into()
                };
                preferences.probe_status = if persistent { expected_status } else { 204 };
                self.save_preferences(preferences)?;
                self.probe_url = url;
                self.state.probe_custom = true;
                self.state.probe_persistent = persistent;
                self.state.probe_expected_status = expected_status;
                self.state.probe_result = None;
                Ok(())
            }
            Command::ResetProbeTarget => {
                if self.state.can_stop {
                    return Err(DesktopError("busy"));
                }
                let mut preferences = self.preferences.clone();
                preferences.probe_url = DEFAULT_PROBE_URL.into();
                preferences.probe_status = 204;
                self.save_preferences(preferences)?;
                self.probe_url = Zeroizing::new(DEFAULT_PROBE_URL.into());
                self.state.probe_custom = false;
                self.state.probe_persistent = true;
                self.state.probe_expected_status = 204;
                self.state.probe_result = None;
                Ok(())
            }
            _ => Err(DesktopError("invalid_input")),
        }
    }
    async fn persist_refresh_completion(
        &self,
        identity: SubscriptionIdentity,
        success: bool,
    ) -> Result<()> {
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.service
            .record_schedule_completion(identity, success, wall)
            .await
            .map_err(map_subscription)
    }
    async fn refreshed(
        &mut self,
        identity: SubscriptionIdentity,
        result: std::result::Result<RefreshResult, SubscriptionError>,
    ) -> Result<()> {
        let outcome = match result.map_err(map_subscription) {
            Err(error) => Err(error),
            Ok(result) => match result {
                RefreshResult::NotModified => self.page().await,
                RefreshResult::Pending(pending) => {
                    let summary = pending.summary();
                    if summary.needs_review() {
                        self.state.pending = Some(pending_dto(&pending, 0)?);
                        self.pending = Some(pending);
                        Ok(())
                    } else {
                        self.service
                            .commit(pending.accept_clean().map_err(map_subscription)?)
                            .await
                            .map_err(map_subscription)?;
                        self.clear_selection();
                        self.page().await
                    }
                }
            },
        };
        if self.pending.is_none() {
            self.persist_refresh_completion(identity, outcome.is_ok())
                .await?;
        }
        outcome
    }
}

fn map_subscription(error: SubscriptionError) -> DesktopError {
    DesktopError(match error {
        SubscriptionError::InvalidSource => "invalid_input",
        SubscriptionError::Fetch(FetchError::HwidRequired) => "hwid_required",
        SubscriptionError::Fetch(FetchError::HwidLimit) => "hwid_limit",
        SubscriptionError::Fetch(FetchError::HwidRejected) => "hwid_rejected",
        SubscriptionError::Fetch(FetchError::HwidRedirect) => "hwid_redirect",
        SubscriptionError::Fetch(FetchError::UnsupportedPeerVerification) => "unsupported_proxy",
        SubscriptionError::Fetch(_) => "fetch",
        SubscriptionError::Import(_) => "import",
        _ => "storage",
    })
}

struct SessionUpdate {
    status: &'static str,
    error: Option<&'static str>,
    probe_result: Option<HealthResultDto>,
    network_stamp: Option<NetworkStamp>,
}

fn network_matches(network: Option<NetworkSnapshot>, stamp: Option<NetworkStamp>) -> bool {
    match (network, stamp) {
        (Some(network), Some(stamp)) => network.is_current(stamp),
        (None, None) => true,
        _ => false,
    }
}

fn apply_session_update(
    state: &mut Snapshot,
    update: SessionUpdate,
    network: Option<NetworkSnapshot>,
) {
    if update.network_stamp.is_some() && !network_matches(network, update.network_stamp) {
        state.status = "Limited";
        state.error = Some("network_changed");
        state.connection_error = Some("network_changed");
        state.probe_result = None;
        return;
    }
    state.status = update.status;
    state.error = update.error;
    state.connection_error = update.error;
    if let Some(result) = update.probe_result {
        state.probe_result = Some(result);
    }
}

fn synchronize_network(state: &mut Snapshot, network: Option<NetworkSnapshot>) {
    let epoch = network.map(|network| network.stamp.to_string());
    let changed = state.network_epoch != epoch || network.is_some_and(|network| network.exhausted);
    state.network_epoch = epoch;
    state.network_observed = network.is_some_and(NetworkSnapshot::fully_observed);
    let coverage = network.map(|network| network.coverage).unwrap_or_default();
    state.network_routes = coverage.routes;
    state.network_resolver = coverage.resolver;
    state.network_manager = coverage.manager;
    if changed {
        state.tcp_invalidated |= state.tcp_result.is_some() || state.tcp_busy;
        state.tcp_result = None;
        state.probe_result = None;
        if matches!(state.status, "Checking" | "Connected" | "Limited") {
            state.status = "Limited";
            state.error = Some("network_changed");
            state.connection_error = Some("network_changed");
        }
    }
}

fn apply_network_event(state: &mut Snapshot, network: Option<NetworkSnapshot>, control: &Control) {
    let changed = state.network_epoch != network.map(|network| network.stamp.to_string())
        || network.is_some_and(|network| network.exhausted);
    if changed {
        // Отменяем также CheckTcp, уже принятые UI, но ещё ожидающие в очереди.
        control.cancel_tcp();
    }
    synchronize_network(state, network);
}

async fn network_changed(network: &mut Option<NetworkWatch>) {
    match network {
        Some(watch) => {
            if watch.changed().await.is_err() {
                *network = None;
            }
        }
        None => std::future::pending().await,
    }
}

async fn network_invalidated(network: &mut Option<NetworkWatch>, stamp: Option<NetworkStamp>) {
    loop {
        if !network_matches(network.as_ref().map(NetworkWatch::current), stamp) {
            return;
        }
        network_changed(network).await;
    }
}

#[cfg(test)]
mod settings_tests {
    use super::{Actor, DEFAULT_PROBE_URL, HostConfig, Snapshot, Store, open};
    use std::{
        fs,
        os::unix::fs::DirBuilderExt,
        path::PathBuf,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };
    use zeroize::Zeroizing;

    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("очистка своего тестового каталога");
        }
    }

    #[test]
    fn hwid_fetch_failures_have_specific_public_codes() {
        for (error, key) in [
            (super::FetchError::HwidRequired, "hwid_required"),
            (super::FetchError::HwidLimit, "hwid_limit"),
            (super::FetchError::HwidRejected, "hwid_rejected"),
            (super::FetchError::HwidRedirect, "hwid_redirect"),
        ] {
            assert_eq!(
                super::map_subscription(super::SubscriptionError::Fetch(error)).key(),
                key
            );
        }
    }

    #[test]
    fn udp_collision_is_detected_without_touching_occupied_socket() {
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").expect("UDP fixture");
        let port = udp.local_addr().expect("адрес").port();
        assert_eq!(
            super::available_port(port).expect_err("UDP занят").key(),
            "listen_port_busy"
        );
        let automatic = super::available_port(0).expect("автопорт");
        assert_ne!(automatic.get(), port);
        assert!(udp.local_addr().is_ok());
    }

    #[tokio::test]
    async fn listen_port_is_busy_during_active_session_and_failed_cleanup() {
        let path = std::env::temp_dir().join(format!(
            "yonik-desktop-settings-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("часы")
                .as_nanos()
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("создание собственного тестового каталога");
        let directory = Directory(path);
        let mut config = HostConfig::new(directory.0.clone(), directory.0.join("engines"))
            .with_ephemeral_key([77; 32]);
        let (service, _lock, routing_repository) = open(&mut config).await.expect("тестовая БД");
        let (preferences_store, preferences, _) = Store::open(&directory.0);
        let mut actor = Actor {
            routing_repository,
            routing_revision: Some(0),
            preferences_store,
            preferences,
            service,
            state: Snapshot::new(true),
            callback: Arc::new(|_| {}),
            selected: None,
            pending: None,
            automatic_port: true,
            probe_url: Zeroizing::new(DEFAULT_PROBE_URL.into()),
            network: None,
        };
        actor.state.port = 18888;
        actor.state.can_stop = true;
        for status in ["Preparing", "Connected", "CleanupError"] {
            actor.state.status = status;
            let command = serde_json::from_str(r#"{"command":"set_listen_port","port":18889}"#)
                .expect("команда настройки");
            assert_eq!(
                actor
                    .command(command)
                    .await
                    .expect_err("сессия удерживается")
                    .key(),
                "busy"
            );
            assert_eq!(actor.state.port, 18888);
            for raw in [
                r#"{"command":"set_engine","engine":"sing-box"}"#,
                r#"{"command":"set_probe_target","url":"https://example.com/","expected_status":200}"#,
                r#"{"command":"reset_probe_target"}"#,
                r#"{"command":"reset_listen_port"}"#,
            ] {
                let command = serde_json::from_str(raw).expect("настройка");
                assert_eq!(
                    actor
                        .command(command)
                        .await
                        .expect_err("снимок сессии неизменен")
                        .key(),
                    "busy"
                );
            }
            let command =
                serde_json::from_str(r#"{"command":"delete_source","id":1}"#).expect("удаление");
            assert_eq!(
                actor
                    .command(command)
                    .await
                    .expect_err("активное ядро")
                    .key(),
                "source_in_use"
            );
            assert!(actor.state.can_stop);
            assert_eq!(actor.state.status, status);
        }
        // После подтверждённого освобождения сессии настройка снова применима.
        actor.state.can_stop = false;
        let command = serde_json::from_str(r#"{"command":"set_listen_port","port":18889}"#)
            .expect("команда настройки");
        actor.command(command).await.expect("сессия освобождена");
        assert_eq!(actor.state.port, 18889);
        actor.preferences_store.sync_directory =
            |_| Err(std::io::Error::other("инъекция dir fsync"));
        actor
            .command(
                serde_json::from_str(r#"{"command":"set_language","language":"en"}"#)
                    .expect("язык"),
            )
            .await
            .expect("опубликованное изменение применено");
        assert_eq!(actor.state.language, "en");
        assert_eq!(actor.preferences.language, "en");
        assert!(actor.state.preferences_durability_warning);
        assert_eq!(Store::open(&directory.0).1.language, "en");
        actor.service.close().await.expect("закрытие БД");
    }
}

#[cfg(test)]
mod error_scope_tests {
    use super::{Command, SessionUpdate, Snapshot, apply_session_update};

    #[test]
    fn metadata_query_does_not_clear_download_error() {
        let mut state = Snapshot::new(true);
        state.command_error(true, "hwid_required");
        let command: Command = serde_json::from_str(
            r#"{"command":"query","search":"","favorites_only":false,"offset":0}"#,
        )
        .expect("query");
        state.begin_command(&command);
        assert_eq!(state.source_error, Some("hwid_required"));
    }

    #[test]
    fn session_updates_and_source_commands_keep_independent_errors() {
        let mut state = Snapshot::new(true);
        state.command_error(true, "hwid_required");
        apply_session_update(
            &mut state,
            SessionUpdate {
                status: "Connected",
                error: None,
                probe_result: None,
                network_stamp: None,
            },
            None,
        );
        assert_eq!(state.source_error, Some("hwid_required"));
        assert_eq!(state.connection_error, None);
        assert_eq!(state.status, "Connected");
        state.command_error(false, "engine_failed");
        state.begin_command(&Command::Refresh);
        assert_eq!(state.source_error, None);
        assert_eq!(state.connection_error, Some("engine_failed"));
        assert_eq!(state.status, "Connected");
        state.command_error(true, "fetch");
        state.begin_command(&Command::Connect);
        assert_eq!(state.source_error, Some("fetch"));
        assert_eq!(state.connection_error, None);
        apply_session_update(
            &mut state,
            SessionUpdate {
                status: "Disconnected",
                error: Some("engine_missing"),
                probe_result: None,
                network_stamp: None,
            },
            None,
        );
        assert_eq!(state.source_error, Some("fetch"));
        assert_eq!(state.connection_error, Some("engine_missing"));
        assert_eq!(state.error, Some("engine_missing"));
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    #[tokio::test]
    async fn network_change_invalidates_tcp_commands_already_in_queue() {
        let first = NetworkMonitor::start().expect("первый наблюдатель");
        let second = NetworkMonitor::start().expect("второй наблюдатель");
        let (priority, _) = watch::channel(Priority::default());
        let (tcp_priority, _) = watch::channel(0);
        let control = Control {
            generation: AtomicU64::new(0),
            priority,
            tcp_priority,
            subscription_policy: watch::channel(Default::default()).0,
            subscription_override: std::sync::atomic::AtomicBool::new(false),
            subscription_visible: watch::channel(false).0,
            closed: (Mutex::new(false), Condvar::new()),
        };
        let (sender, mut receiver) = mpsc::channel(1);
        sender
            .try_send(Envelope {
                generation: 0,
                tcp_generation: *control.tcp_priority.borrow(),
                command: Command::CheckTcp,
            })
            .expect("проверка уже в очереди");
        let mut state = Snapshot::new(true);
        synchronize_network(&mut state, Some(first.watch().current()));
        apply_network_event(&mut state, Some(second.watch().current()), &control);
        let queued = receiver.recv().await.expect("ожидавшая команда");
        assert_ne!(queued.tcp_generation, *control.tcp_priority.borrow());
        let generation = *control.tcp_priority.borrow();
        apply_network_event(&mut state, Some(second.watch().current()), &control);
        assert_eq!(
            generation,
            *control.tcp_priority.borrow(),
            "повтор snapshot не отменяет новую ручную команду"
        );
        first.shutdown().await;
        second.shutdown().await;
    }

    #[tokio::test]
    async fn late_success_from_previous_network_cannot_restore_connected() {
        let first = NetworkMonitor::start().expect("первый наблюдатель");
        let second = NetworkMonitor::start().expect("второй наблюдатель");
        let mut state = Snapshot::new(true);
        state.status = "Limited";
        apply_session_update(
            &mut state,
            SessionUpdate {
                status: "Connected",
                error: None,
                probe_result: Some(ProbeResult::Timeout.into()),
                network_stamp: Some(first.watch().current().stamp),
            },
            Some(second.watch().current()),
        );
        assert_eq!(state.status, "Limited");
        assert_eq!(state.error, Some("network_changed"));
        assert!(state.probe_result.is_none());
        first.shutdown().await;
        second.shutdown().await;
    }

    #[tokio::test]
    async fn new_network_removes_old_proof_without_stopping_core() {
        let first = NetworkMonitor::start().expect("первый наблюдатель");
        let second = NetworkMonitor::start().expect("второй наблюдатель");
        let mut state = Snapshot::new(true);
        state.network_epoch = Some(first.watch().current().stamp.to_string());
        state.status = "Connected";
        state.can_stop = true;
        state.probe_result = Some(ProbeResult::Timeout.into());
        state.tcp_busy = true;
        synchronize_network(&mut state, Some(second.watch().current()));
        assert!(state.probe_result.is_none());
        assert!(state.tcp_invalidated);
        assert_eq!(state.status, "Limited");
        assert!(state.can_stop);
        first.shutdown().await;
        second.shutdown().await;
    }
}
#[cfg(test)]
mod tcp_series_tests {
    use super::{TCP_SERIES_SAMPLES, TcpProbeSeriesResult, TcpProbeStatus, TcpResultDto};
    use std::time::Duration;

    #[test]
    fn cancelled_or_incomplete_series_cannot_publish_a_median() {
        let mut dto = TcpResultDto {
            source_id: 1,
            profile_id: 2,
            revision: 3,
            name: "Проверка".into(),
            host: "127.0.0.1".into(),
            port: 443,
            status: "cancelled",
            dns_ms: None,
            connect_ms: None,
            samples_ms: Vec::new(),
            samples_requested: TCP_SERIES_SAMPLES,
            selected_address: None,
            checked_at_ms: 0,
            network_epoch: None,
            network_observed: false,
        };
        for (status, count, expected) in [
            (TcpProbeStatus::Reachable, 3, Some(10)),
            (TcpProbeStatus::Cancelled, 3, None),
            (TcpProbeStatus::ConnectFailure, 1, None),
            (TcpProbeStatus::Reachable, 2, None),
        ] {
            // Поздняя отмена меняет status уже завершённой серии. Сохранённая
            // внутри неё median не должна обойти проверку при публикации DTO.
            dto.finish(TcpProbeSeriesResult {
                status,
                dns_elapsed: None,
                selected_address: Some("127.0.0.1:443".parse().expect("адрес")),
                samples: vec![Duration::from_millis(10); count],
                median: Some(Duration::from_millis(10)),
            });
            assert_eq!(dto.connect_ms, expected);
            assert_eq!(dto.samples_ms.len(), count);
        }
    }
}

#[derive(Serialize)]
struct HealthResultDto {
    kind: &'static str,
    http_status: Option<u16>,
    elapsed_ms: Option<u64>,
    network_epoch: Option<String>,
    network_observed: bool,
}
impl From<ProbeResult> for HealthResultDto {
    fn from(result: ProbeResult) -> Self {
        match result {
            ProbeResult::Response { status, elapsed } => Self {
                kind: "http",
                http_status: Some(status),
                elapsed_ms: Some(elapsed.as_millis().min(u64::MAX as u128) as u64),
                network_epoch: None,
                network_observed: false,
            },
            other => Self {
                kind: match other {
                    ProbeResult::Timeout => "timeout",
                    ProbeResult::Tls => "tls",
                    _ => "transport",
                },
                http_status: None,
                elapsed_ms: None,
                network_epoch: None,
                network_observed: false,
            },
        }
    }
}
/// Completion старого ресурса не отменяет очередь/review нового поколения.
async fn reconcile_manual_schedule(
    scheduled: &mut DesktopSchedule,
    service: &SubscriptionService,
    identity: SubscriptionIdentity,
    clock: impl Fn() -> Duration,
    storage_failed: bool,
) -> Result<()> {
    if service
        .schedule_generation(identity.id)
        .await
        .map_err(map_subscription)?
        != Some(identity.generation)
    {
        return Ok(());
    }
    scheduled.cancel(identity.id, &clock).await?;
    if !storage_failed {
        scheduled.reload_id(service, identity.id, clock()).await?;
    }
    Ok(())
}

type RefreshTask = (
    u64,
    watch::Sender<bool>,
    tokio::task::JoinHandle<std::result::Result<RefreshResult, SubscriptionError>>,
    SubscriptionIdentity,
    String,
);

struct Running {
    cancel: watch::Sender<bool>,
    task: tokio::task::JoinHandle<std::result::Result<(), LocalSession>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TcpKey {
    source: i64,
    profile: i64,
    revision: i64,
}
#[derive(Serialize)]
struct TcpResultDto {
    source_id: i64,
    profile_id: i64,
    revision: i64,
    name: String,
    host: String,
    port: u16,
    status: &'static str,
    dns_ms: Option<u64>,
    connect_ms: Option<u64>,
    samples_ms: Vec<u64>,
    samples_requested: usize,
    selected_address: Option<String>,
    checked_at_ms: u64,
    network_epoch: Option<String>,
    network_observed: bool,
}
impl TcpResultDto {
    fn key(&self) -> TcpKey {
        TcpKey {
            source: self.source_id,
            profile: self.profile_id,
            revision: self.revision,
        }
    }
    fn finish(&mut self, result: TcpProbeSeriesResult) {
        self.status = match result.status {
            TcpProbeStatus::Reachable => "reachable",
            TcpProbeStatus::NotApplicable => "not_applicable",
            TcpProbeStatus::ResolverUnavailable => "resolver_unavailable",
            TcpProbeStatus::DnsFailure => "dns_failed",
            TcpProbeStatus::ConnectFailure => "connect_failed",
            TcpProbeStatus::DnsTimeout => "dns_timeout",
            TcpProbeStatus::ConnectTimeout => "connect_timeout",
            TcpProbeStatus::Cancelled => "cancelled",
        };
        let millis = |duration: Duration| duration.as_millis().min(u64::MAX as u128) as u64;
        self.dns_ms = result.dns_elapsed.map(millis);
        self.connect_ms = if result.status == TcpProbeStatus::Reachable
            && result.samples.len() == TCP_SERIES_SAMPLES
        {
            result.median.map(millis)
        } else {
            None
        };
        self.samples_ms = result.samples.into_iter().map(millis).collect();
        self.selected_address = result.selected_address.map(|address| address.to_string());
        self.checked_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(millis)
            .unwrap_or_default();
    }
}
struct TcpRunning {
    generation: u64,
    tcp_generation: u64,
    network_stamp: Option<NetworkStamp>,
    result: TcpResultDto,
    cancel: watch::Sender<bool>,
    task: tokio::task::JoinHandle<std::result::Result<TcpProbeSeriesResult, HostError>>,
}

async fn session(
    config: yonik_engine::CompiledConfig,
    engine_dir: PathBuf,
    engine: Engine,
    target: ProbeTarget,
    mut cancel: Cancellation,
    updates: mpsc::Sender<SessionUpdate>,
    mut network: Option<NetworkWatch>,
) -> std::result::Result<(), LocalSession> {
    let Ok(mut session) = LocalSession::new() else {
        let _ = updates
            .send(SessionUpdate {
                status: "Error",
                error: Some("engine_failed"),
                probe_result: None,
                network_stamp: None,
            })
            .await;
        return Ok(());
    };
    let operation = async {
        let (relative, digest) = match engine {
            Engine::Xray => (
                "xray-26.10.31/xray",
                "cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1",
            ),
            Engine::SingBox => (
                "sing-box-1.14.4/sing-box",
                "1264361b5f8868c8768f7555d3e77e20118bf503c41ac08bb290a0d99056aa3a",
            ),
        };
        cancel.check().map_err(|_| "cancelled")?;
        let path = engine_dir.join(relative);
        let binary = tokio::task::spawn_blocking(move || {
            BinarySnapshot::load(
                &path,
                digest.parse().map_err(|_| HostError::BinaryHash)?,
                engine,
            )
        })
        .await
        .map_err(|_| "engine_missing")?
        .map_err(|_| "engine_missing")?;
        cancel.check().map_err(|_| "cancelled")?;
        let checked = CheckedConfig::validate(binary, config, &mut cancel)
            .await
            .map_err(|e| {
                if e == HostError::Cancelled {
                    "cancelled"
                } else {
                    "engine_config"
                }
            })?;
        session.start(checked, &mut cancel).await.map_err(|e| {
            if e == HostError::Cancelled {
                "cancelled"
            } else {
                "engine_failed"
            }
        })?;
        let checked_network = network.as_ref().map(NetworkWatch::current);
        let mut network_stamp = checked_network.map(|network| network.stamp);
        let _ = updates
            .send(SessionUpdate {
                status: "Checking",
                error: None,
                probe_result: None,
                network_stamp,
            })
            .await;
        let result = tokio::select! {
            biased;
            _ = network_invalidated(&mut network, network_stamp) => None,
            result = session.check(&target, &mut cancel) => Some(result),
        };
        let result = result.transpose().map_err(|e| {
            if e == HostError::Cancelled {
                "cancelled"
            } else {
                "check_failed"
            }
        })?;
        if let Some(result) = result.filter(|_| {
            network_matches(network.as_ref().map(NetworkWatch::current), network_stamp)
        }) {
            let passed = target.accepts(result);
            let error = if passed {
                None
            } else {
                Some(match result {
                    ProbeResult::Response { .. } => "check_http_status",
                    ProbeResult::Timeout => "check_timeout",
                    ProbeResult::Tls => "check_tls",
                    ProbeResult::Transport => "check_transport",
                })
            };
            let mut proof: HealthResultDto = result.into();
            proof.network_epoch = network_stamp.map(|stamp| stamp.to_string());
            proof.network_observed = checked_network.is_some_and(NetworkSnapshot::fully_observed);
            let _ = updates.send(SessionUpdate {
                status: if passed { "Connected" } else { "Limited" },
                error,
                probe_result: Some(proof),
                network_stamp,
            }).await;
        } else {
            session.invalidate_check().map_err(|_| "check_failed")?;
            let _ = updates.send(SessionUpdate {
                status: "Limited",
                error: Some("network_changed"),
                probe_result: None,
                network_stamp: None,
            }).await;
            network_stamp = network.as_ref().map(|watch| watch.current().stamp);
        }
        loop {
            tokio::select! {
                biased;
                result = session.wait_exit(&mut cancel) => return result.map_err(|e| {
                    if e == HostError::Cancelled { "cancelled" } else { "engine_failed" }
                }),
                _ = network_changed(&mut network) => {
                    if !network_matches(network.as_ref().map(NetworkWatch::current), network_stamp) {
                        session.invalidate_check().map_err(|_| "check_failed")?;
                        network_stamp = network.as_ref().map(|watch| watch.current().stamp);
                        let _ = updates.send(SessionUpdate {
                            status: "Limited",
                            error: Some("network_changed"),
                            probe_result: None,
                            network_stamp: None,
                        }).await;
                    }
                }
            }
        }
    }
    .await;
    if session.stop().await.is_err() {
        let _ = updates
            .send(SessionUpdate {
                status: "CleanupError",
                error: Some("cleanup_failed"),
                probe_result: None,
                network_stamp: None,
            })
            .await;
        return Err(session);
    }
    let error = operation.err().filter(|key| *key != "cancelled");
    let _ = updates
        .send(SessionUpdate {
            status: if error.is_some() {
                "Error"
            } else {
                "Disconnected"
            },
            error,
            probe_result: None,
            network_stamp: None,
        })
        .await;
    Ok(())
}

async fn run(
    mut config: HostConfig,
    mut commands: mpsc::Receiver<Envelope>,
    mut priority: watch::Receiver<Priority>,
    control: Arc<Control>,
    callback: Callback,
) {
    let mut state = Snapshot::new(config.ephemeral_key.is_some());
    state.port = config.listen_port.map_or(0, NonZeroU16::get);
    state.busy = true;
    state.emit(&callback);
    // Обработчики регистрируются до любого spawn ядра.
    let signals = match (
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()),
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()),
    ) {
        (Ok(mut interrupt), Ok(mut terminate)) => {
            let control = control.clone();
            Some(tokio::spawn(async move {
                tokio::select! { _ = interrupt.recv() => (), _ = terminate.recv() => () };
                control.interrupt(true);
            }))
        }
        _ => {
            // Без обработчиков завершения не разрешаем запуск дочернего ядра.
            state.busy = false;
            state.error = Some("startup");
            state.connection_error = Some("startup");
            state.emit(&callback);
            while !priority.borrow_and_update().shutdown {
                if priority.changed().await.is_err() {
                    break;
                }
            }
            state.closed = true;
            state.emit(&callback);
            control.closed();
            return;
        }
    };
    let opened = match config.listen_port.map_or_else(|| available_port(0), Ok) {
        Ok(port) => {
            state.port = port.get();
            open(&mut config).await
        }
        Err(error) => Err(error),
    };
    state.busy = false;
    let (service, _lock, routing_repository) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            state.command_error(false, error.key());
            state.emit(&callback);
            loop {
                if priority.borrow_and_update().shutdown {
                    break;
                }
                if priority.changed().await.is_err() {
                    break;
                }
            }
            state.closed = true;
            state.emit(&callback);
            control.closed();
            if let Some(task) = signals {
                task.abort();
            }
            return;
        }
    };
    let (preferences_store, mut preferences, warning) = Store::open(&config.data_dir);
    if let Some(port) = config.listen_port {
        preferences.port = Some(port);
    }
    state.engine = if preferences.engine == "sing-box" {
        EngineChoice::SingBox
    } else {
        EngineChoice::Xray
    };
    if let Some(port) = preferences.port {
        state.port = port.get();
    }
    state.language = preferences.language.clone();
    state.preferences_warning = warning;
    state.probe_custom = preferences.probe_url != DEFAULT_PROBE_URL;
    state.probe_expected_status = preferences.probe_status;
    state.ready = true;
    let network_monitor = NetworkMonitor::start().ok();
    let mut network = network_monitor.as_ref().map(NetworkMonitor::watch);
    synchronize_network(&mut state, network.as_ref().map(NetworkWatch::current));
    let automatic_port = preferences.port.is_none();
    let probe_url = Zeroizing::new(preferences.probe_url.clone());
    let mut actor = Actor {
        routing_repository,
        routing_revision: Some(0),
        preferences_store,
        preferences,
        service,
        state,
        callback,
        selected: None,
        pending: None,
        automatic_port,
        probe_url,
        network: network.clone(),
    };
    match actor.routing_repository.routing_draft().await {
        Ok(None) => {}
        Ok(Some((revision, raw))) => match std::str::from_utf8(&raw)
            .ok()
            .and_then(|s| crate::editor::Draft::parse(s).ok())
        {
            Some(draft) => {
                actor.state.routing = draft;
                actor.routing_revision = Some(revision);
            }
            None => {
                actor.routing_revision = None;
                actor.state.error = Some("routing_storage");
            }
        },
        Err(_) => {
            actor.routing_revision = None;
            actor.state.error = Some("routing_storage");
        }
    }
    actor.state.routing_revision = actor.routing_revision;
    if let Err(error) = actor.reload().await {
        actor.state.command_error(true, error.key());
    }
    actor.emit();
    let (updates, mut events) = mpsc::channel::<SessionUpdate>(16);
    let mut running: Option<Running> = None;
    let mut failed_session: Option<LocalSession> = None;
    let mut refreshing: Option<RefreshTask> = None;
    let mut tcp_running: Option<TcpRunning> = None;
    let mut tcp_priority = control.tcp_priority.subscribe();
    let mut quitting = false;
    let (system_policy_sender, mut system_policy) = watch::channel(Default::default());
    let provider_enabled = config.system_subscription_policy;
    let provider = async move {
        if provider_enabled {
            yonik_linux::system_subscription_policy(system_policy_sender).await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::pin!(provider);
    let mut subscription_policy = control.subscription_policy.subscribe();
    let mut subscription_visible = control.subscription_visible.subscribe();
    let origin = tokio::time::Instant::now();
    let mut scheduled = match DesktopSchedule::new() {
        Ok(owner) => owner,
        Err(error) => {
            actor.state.command_error(true, error.key());
            actor.state.ready = false;
            actor.state.closed = actor.service.close().await.is_ok();
            actor.state.emit(&actor.callback);
            control.closed();
            return;
        }
    };
    scheduled.policy = *subscription_policy.borrow_and_update();
    scheduled.visible = *subscription_visible.borrow_and_update();
    let mut schedule_loaded = false;
    if let Err(error) = scheduled.load(&actor.service, origin.elapsed()).await {
        actor.state.command_error(true, error.key());
        actor.emit();
    } else {
        schedule_loaded = true;
    }
    let fetch_host = yonik_linux::FetchHost::new(Duration::from_secs(30))
        .ok()
        .map(Arc::new);
    loop {
        if let Some(observer) = &config.schedule_observer {
            observer(scheduled.policy(), schedule_loaded);
        }
        let allowed = fetch_host.is_some()
            && !quitting
            && refreshing.is_none()
            && actor.pending.is_none()
            && scheduled.review.is_none();
        let deadline = if allowed { scheduled.deadline() } else { None };

        tokio::select! {
            biased;
            changed = priority.changed() => {
                if changed.is_err() { quitting = true; }
                let action = *priority.borrow_and_update();
                quitting |= action.shutdown;
                // Сначала все сигналы отмены, затем любые await: spawned TCP не должен
                // получить polling сети во время ожидания scheduler acknowledgement.
                if let Some((_, cancel, _, _, _)) = refreshing.as_ref() { cancel.send_replace(true); }
                if let Some(session) = running.as_ref() { session.cancel.send_replace(true); }
                if let Some(tcp) = tcp_running.as_ref() { tcp.cancel.send_replace(true); }
                if scheduled.runtime.shutdown(origin.elapsed()).await.is_err() { actor.state.command_error(true, "storage"); }
                scheduled.fetching = false; scheduled.review = None; scheduled.deferred = None;
                if !quitting {
                    let policy = scheduled.policy;
                    let visible = scheduled.visible;
                    if let Ok(mut replacement) = DesktopSchedule::new() {
                        replacement.policy = policy; replacement.visible = visible;
                        if let Err(error) = replacement.load(&actor.service, origin.elapsed()).await { actor.state.command_error(true, error.key()); }
                        scheduled = replacement;
                    }
                }
                actor.state.connection_error = None;
                actor.pending = None; actor.state.pending = None;
                if let Some((_, cancel, _, _, _)) = refreshing.as_ref() { cancel.send_replace(true); }
                if let Some(session) = running.as_ref() { session.cancel.send_replace(true); }
                if let Some(tcp) = tcp_running.as_ref() { tcp.cancel.send_replace(true); }
                if let Some(mut session) = failed_session.take() {
                    if session.stop().await.is_err() { failed_session = Some(session); actor.state.command_error(false, "cleanup_failed"); }
                }
                if running.is_none() && failed_session.is_none() {
                    actor.state.can_stop = false; actor.state.active_name = None; actor.state.active_engine = None;
                }
                actor.state.status = if running.is_some() { "Stopping" } else if failed_session.is_some() { "CleanupError" } else { "Disconnected" };
                actor.emit();
            }
            _ = &mut provider => {},
            changed = system_policy.changed(), if provider_enabled => {
                if changed.is_ok() && !control.subscription_override.load(Ordering::Acquire) {
                    scheduled.policy = *system_policy.borrow_and_update();
                }
            }
            _ = subscription_policy.changed() => {
                scheduled.policy = *subscription_policy.borrow_and_update();
            }
            _ = subscription_visible.changed() => {
                scheduled.visible = *subscription_visible.borrow_and_update();
            }
            event = async {
                if let Some(event) = scheduled.deferred.take() { return Ok(Some(event)); }
                scheduled.runtime.next_event(|| origin.elapsed()).await
            }, if scheduled.fetching || scheduled.deferred.is_some() => {
                scheduled.fetching = false;
                match event {
                    Ok(Some(yonik_linux::SubscriptionScheduleEvent::Review { id })) => {
                        scheduled.review = Some(id);
                        actor.state.source_id = Some(id);
                        actor.state.offset = 0;
                        actor.clear_selection();
                        if let Err(error) = actor.reload().await { actor.state.command_error(true, error.key()); }
                        if let Some(pending) = scheduled.runtime.review(id) {
                            match pending_dto(pending, 0) {
                                Ok(dto) => actor.state.pending = Some(dto),
                                Err(error) => actor.state.command_error(true, error.key()),
                            }
                        }
                    }
                    Ok(Some(yonik_linux::SubscriptionScheduleEvent::Failed { error, .. })) => actor.state.command_error(true, map_subscription(error).key()),
                    Ok(_) => {},
                    Err(_) => actor.state.command_error(true, "storage"),
                }
                actor.emit();
            }
            _ = async {
                match deadline {
                    Some(due) => tokio::time::sleep_until(origin + due).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                if let Some(host) = fetch_host.as_ref() {
                    let policy = scheduled.policy();
                    match scheduled.runtime.dispatch(origin.elapsed(), policy, &actor.service, host.clone()) {
                        Ok(()) => scheduled.fetching = true,
                        Err(_) => actor.state.command_error(true, "storage"),
                    }
                }
            }
            changed = tcp_priority.changed() => {
                let _ = changed;
                tcp_priority.borrow_and_update();
                if let Some(tcp) = tcp_running.as_ref() { tcp.cancel.send_replace(true); }
            }
            _ = network_changed(&mut network) => {
                let current = network.as_ref().map(NetworkWatch::current);
                actor.network = network.clone();
                apply_network_event(&mut actor.state, current, &control);
                if let Some(tcp) = tcp_running.as_ref() {
                    if !network_matches(current, tcp.network_stamp) { tcp.cancel.send_replace(true); }
                }
                actor.emit();
            }
            result = async { (&mut tcp_running.as_mut().expect("guard").task).await }, if tcp_running.is_some() => {
                let mut tcp = tcp_running.take().expect("guard");
                actor.state.tcp_busy = false;
                actor.state.tcp_name = None;
                if actor.tcp_key() == Some(tcp.result.key()) && !quitting
                    && network_matches(network.as_ref().map(NetworkWatch::current), tcp.network_stamp) {
                    match result {
                        Ok(Ok(mut result)) => {
                            if tcp.generation != control.generation.load(Ordering::SeqCst)
                                || tcp.tcp_generation != *control.tcp_priority.borrow() {
                                result.status = TcpProbeStatus::Cancelled;
                                result.samples.clear();
                                result.median = None;
                                result.selected_address = None;
                            }
                            tcp.result.finish(result); actor.state.tcp_result = Some(tcp.result);
                        }
                        _ => { actor.state.tcp_result = None; actor.state.command_error(false, "tcp_failed"); }
                    }
                }
                actor.emit();
            }
            result = async { (&mut running.as_mut().expect("guard").task).await }, if running.is_some() => {
                running = None;
                if let Ok(Err(session)) = result { failed_session = Some(session); actor.state.command_error(false, "cleanup_failed"); }
                if failed_session.is_none() { actor.state.can_stop = false; actor.state.active_name = None; actor.state.active_engine = None; }
                // Финальный event может ожидать в канале; сначала применяем его.
                while let Ok(update) = events.try_recv() { actor.update(update); }
                actor.emit();
            }
            result = async { (&mut refreshing.as_mut().expect("guard").2).await }, if refreshing.is_some() => {
                let (generation, cancel, _, identity, _) = refreshing.take().expect("guard");
                actor.state.busy = false;
                actor.state.refresh_source_id = None;
                actor.state.refresh_token = None;
                if generation == control.generation.load(Ordering::SeqCst) && !quitting { actor.state.source_error = None; }
                if generation == control.generation.load(Ordering::SeqCst) && !quitting && !*cancel.borrow() {
                    let outcome = match result { Ok(result) => actor.refreshed(identity, result).await, Err(_) => Err(DesktopError("fetch")) };
                    let storage_failed = matches!(&outcome, Err(error) if error.key() == "storage");
                    if let Err(error) = outcome { actor.state.command_error(true, error.key()); }
                    if let Err(error) = reconcile_manual_schedule(&mut scheduled, &actor.service, identity, || origin.elapsed(), storage_failed).await { actor.state.command_error(true, error.key()); }
                }
                actor.emit();
            }
            Some(update) = events.recv() => {
                actor.update(update); actor.emit();
            }
            command = commands.recv(), if !quitting => {
                let Some(envelope) = command else { control.interrupt(true); continue; };
                if envelope.generation != control.generation.load(Ordering::SeqCst) { continue; }
                let source_operation = envelope.command.is_source_operation();
                let reload_schedule = matches!(&envelope.command, Command::AddSource { .. } | Command::ImportSource { .. } | Command::SetSourceHwid { .. });
                if (scheduled.fetching || scheduled.deferred.is_some()) && matches!(&envelope.command, Command::ImportSource { .. } | Command::ApproveRefresh | Command::DiscardRefresh) {
                    actor.state.command_error(true, "busy"); actor.emit(); continue;
                }
                if matches!(&envelope.command, Command::Refresh | Command::SetSourceHwid { .. } | Command::ImportSource { .. }) && (scheduled.review.is_some() || actor.pending.is_some()) {
                    actor.state.command_error(true, "review_required"); actor.emit(); continue;
                }
                if matches!(&envelope.command, Command::Refresh) && (scheduled.fetching || scheduled.deferred.is_some()) {
                    actor.state.command_error(true, "busy"); actor.emit(); continue;
                }
                let cancel_id = match &envelope.command {
                    Command::DeleteSource { id } => Some(*id),
                    Command::SetSourceHwid { .. } | Command::Refresh => actor.state.source_id,
                    _ => None,
                };
                if let Some(id) = cancel_id {
                    let cancelled_review = scheduled.review == Some(id);
                    if let Err(error) = scheduled.cancel(id, || origin.elapsed()).await {
                        actor.state.command_error(true, error.key()); actor.emit(); continue;
                    }
                    if (scheduled.deferred.is_some() || scheduled.fetching) && matches!(&envelope.command, Command::Refresh | Command::SetSourceHwid { .. }) {
                        if let Err(error) = scheduled.reload_id(&actor.service, id, origin.elapsed()).await {
                            actor.state.command_error(true, error.key());
                        } else { actor.state.command_error(true, "busy"); }
                        actor.emit(); continue;
                    }
                    if cancelled_review { actor.state.pending = None; }
                }
                actor.state.begin_command(&envelope.command);
                let manual_review_completion = if scheduled.review.is_none() && matches!(envelope.command, Command::ApproveRefresh | Command::DiscardRefresh) { actor.pending.as_ref().map(|pending| pending.identity()) } else { None };
                let outcome = match envelope.command {
                    Command::SubscriptionVisibility { visible } => { scheduled.visible = visible; Ok(()) }
                    Command::ApproveRefresh | Command::DiscardRefresh if scheduled.review.is_some() => {
                        let approve = matches!(envelope.command, Command::ApproveRefresh);
                        let id = scheduled.review.take().expect("review guard");
                        actor.state.pending = None;
                        match scheduled.runtime.finish_review(&actor.service, id, approve, || origin.elapsed()).await {
                            Ok(_) => { actor.clear_selection(); actor.page().await }
                            Err(error) => Err(map_subscription(error)),
                        }
                    }
                    Command::ReviewPage { offset } if scheduled.review.is_some() => {
                        match scheduled.review.and_then(|id| scheduled.runtime.review(id)) {
                            Some(pending) => pending_dto(pending, offset).map(|dto| actor.state.pending = Some(dto)),
                            None => Err(DesktopError("review_required")),
                        }
                    }
                    Command::SelectSource { .. } | Command::ImportSource { .. } | Command::AddSource { .. } if scheduled.review.is_some() => Err(DesktopError("review_required")),

                    Command::CancelRefresh { id, token } => {
                        match refreshing.as_ref() {
                            Some((_, cancel, _, source, current_token)) if source.id == id && token == *current_token && !*cancel.borrow() => {
                                cancel.send_replace(true);
                                Ok(())
                            }
                            _ => Err(DesktopError("refresh_required")),
                        }
                    }
                    Command::CheckTcp => {
                        if envelope.tcp_generation != *control.tcp_priority.borrow() {
                            Err(DesktopError("cancelled"))
                        } else if tcp_running.is_some() || actor.state.busy {
                            Err(DesktopError("busy"))
                        } else if let Some(profile) = actor.selected.as_ref() {
                            let target = TcpProbeTarget::for_server(&profile.server);
                            let current = network.as_ref().map(NetworkWatch::current);
                            let network_stamp = current.map(|network| network.stamp);
                            let result = TcpResultDto {
                                source_id: profile.profile.source_id, profile_id: profile.profile.id,
                                revision: profile.snapshot_revision, name: profile.profile.name.clone(),
                                host: profile.server.endpoint().host().into(), port: profile.server.endpoint().port(),
                                status: "cancelled", dns_ms: None, connect_ms: None, checked_at_ms: 0,
                                samples_ms: Vec::new(), samples_requested: TCP_SERIES_SAMPLES,
                                selected_address: None,
                                network_epoch: network_stamp.map(|stamp| stamp.to_string()),
                                network_observed: current.is_some_and(NetworkSnapshot::fully_observed),
                            };
                            let (cancel, mut receiver) = Cancellation::new();
                            let task = tokio::spawn(async move { check_tcp_series(target, Duration::from_secs(5), &mut receiver).await });
                            actor.state.tcp_busy = true; actor.state.tcp_name = Some(result.name.clone());
                            actor.state.tcp_result = None;
                            actor.state.tcp_invalidated = false;
                            tcp_running = Some(TcpRunning { generation: envelope.generation,
                                tcp_generation: envelope.tcp_generation, network_stamp, result, cancel, task });
                            Ok(())
                        } else { Err(DesktopError("profile_required")) }
                    }
                    Command::ImportSource { name, url, local_exception, route, hwid_enabled } => {
                        if refreshing.is_some() { Err(DesktopError("busy")) }
                        else {
                            match actor.command(Command::AddSource { name, url, local_exception, route, hwid_enabled }).await {
                                Ok(()) => {
                                    actor.state.search.clear(); actor.state.favorites_only = false;
                                    match actor.refresh(envelope.generation).await {
                                        Ok(task) => { refreshing = Some(task); Ok(()) }
                                        Err(error) => Err(error),
                                    }
                                }
                                Err(error) => Err(error),
                            }
                        }
                    }
                    Command::SetSourceHwid { enabled } => {
                        if refreshing.is_some() { Err(DesktopError("busy")) }
                        else {
                            match actor.command(Command::SetSourceHwid { enabled }).await {
                                Ok(()) => match actor.refresh(envelope.generation).await {
                                    Ok(task) => { refreshing = Some(task); Ok(()) }
                                    Err(error) => Err(error),
                                },
                                Err(error) => Err(error),
                            }
                        }
                    }
                    Command::Refresh => {
                        if refreshing.is_some() { Err(DesktopError("busy")) }
                        else { match actor.refresh(envelope.generation).await {
                            Ok(task) => { refreshing = Some(task); Ok(()) }
                            Err(error) => Err(error),
                        } }
                    }
                    Command::Connect => {
                        if actor.state.can_stop || running.is_some() || refreshing.is_some() { Err(DesktopError("busy")) }
                        else if actor.selected.is_none() { Err(DesktopError("profile_required")) }
                        else if actor.pending.is_some() || scheduled.review.is_some() { Err(DesktopError("review_required")) }
                        else if let Err(error) = actor.prepare_port() { Err(error) }
                        else if let Some(profile) = actor.selected.as_ref() {
                            if actor.pending.is_some() { Err(DesktopError("review_required")) }
                            else {
                                match (NonZeroU16::new(actor.state.port).and_then(|port| yonik_engine::compile(&profile.server, actor.state.engine.engine(), port).ok()), actor.probe_target()) {
                                    (Some(compiled), Ok(target)) => {
                                        let (cancel, receiver) = Cancellation::new();
                                        let task = tokio::spawn(session(compiled, config.engine_dir.clone(), actor.state.engine.engine(), target, receiver, updates.clone(), network.clone()));
                                        running = Some(Running { cancel, task }); actor.state.can_stop = true;
                                        actor.state.active_name = Some(profile.profile.name.clone()); actor.state.active_engine = Some(actor.state.engine);
                                        actor.state.probe_result = None; actor.state.status = "Preparing"; Ok(())
                                    }
                                    (_, Err(error)) => Err(error),
                                    _ => Err(DesktopError("engine_config")),
                                }
                            }
                        } else { Err(DesktopError("profile_required")) }
                    }
                    command => {
                        if refreshing.is_some() && matches!(command, Command::SelectSource { .. } | Command::AddSource { .. } | Command::ApproveRefresh | Command::RenameSource { .. } | Command::DeleteSource { .. }) {
                            Err(DesktopError("busy"))
                        } else { actor.command(command).await }
                    }
                };
                if let Some(identity) = manual_review_completion {
                    let storage_failed = matches!(&outcome, Err(error) if error.key() == "storage");
                    if let Err(error) = reconcile_manual_schedule(&mut scheduled, &actor.service, identity, || origin.elapsed(), storage_failed).await { actor.state.command_error(true, error.key()); }
                }
                if reload_schedule || cancel_id.is_some() {
                    if let Some(id) = cancel_id.or(actor.state.source_id) {
                        if let Err(error) = scheduled.reload_id(&actor.service, id, origin.elapsed()).await {
                            actor.state.command_error(true, error.key());
                        }
                    }
                }
                if let Err(error) = outcome { actor.state.command_error(source_operation, error.key()); }
                actor.emit();
            }
        }
        if let Some(tcp) = tcp_running.as_ref() {
            if actor.tcp_key() != Some(tcp.result.key()) {
                tcp.cancel.send_replace(true);
            }
        }
        if actor
            .state
            .tcp_result
            .as_ref()
            .is_some_and(|result| actor.tcp_key() != Some(result.key()))
        {
            actor.state.tcp_result = None;
            actor.emit();
        }
        if quitting && running.is_none() && refreshing.is_none() && tcp_running.is_none() {
            if failed_session.is_some() {
                quitting = false;
                actor.state.command_error(false, "cleanup_failed");
                actor.emit();
                continue;
            }
            break;
        }
    }
    if scheduled.runtime.shutdown(origin.elapsed()).await.is_err() {
        actor.state.command_error(true, "storage");
    }
    actor.pending = None;
    if let Some(monitor) = network_monitor {
        monitor.shutdown().await;
    }
    actor.selected = None;
    let closed = actor.service.close().await.is_ok();
    drop(_lock);
    actor.state.ready = false;
    actor.state.busy = false;
    actor.state.can_connect = false;
    actor.state.closed = closed;
    actor.state.error = if closed { None } else { Some("cleanup_failed") };
    actor.state.connection_error = actor.state.error;
    actor.state.emit(&actor.callback);
    if closed {
        control.closed();
    }
    if let Some(task) = signals {
        task.abort();
    }
}

#[cfg(test)]
mod routing_regressions {
    use super::*;
    use std::os::unix::fs::DirBuilderExt;

    async fn fixture() -> (Actor, File, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "yonik-routing-p2-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("часы")
                .as_nanos()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("fixture");
        let mut config =
            HostConfig::new(path.clone(), path.join("engines")).with_ephemeral_key([71; 32]);
        let (service, lock, routing_repository) = open(&mut config).await.expect("БД");
        routing_repository
            .routing_draft()
            .await
            .expect("инициализация");
        let (preferences_store, preferences, _) = Store::open(&path);
        (
            Actor {
                routing_repository,
                routing_revision: Some(0),
                preferences_store,
                preferences,
                service,
                state: Snapshot::new(true),
                callback: Arc::new(|_| {}),
                selected: None,
                pending: None,
                automatic_port: true,
                probe_url: Zeroizing::new(DEFAULT_PROBE_URL.into()),
                network: None,
            },
            lock,
            path,
        )
    }
    async fn send(actor: &mut Actor, value: serde_json::Value) -> Result<()> {
        actor
            .command(serde_json::from_value(value).expect("контракт команды редактора"))
            .await
    }
    fn put(id: u64, revision: i64, request: u64, create: bool) -> serde_json::Value {
        serde_json::json!({"command":"edit_routing","expected_revision":revision,"request_id":request,"create":create,"confirm_broad":false,"rule":{"id":id,"enabled":true,"executables":[],"domains":["example.org"],"destinations":[],"action":"proxy"}})
    }
    #[tokio::test]
    async fn stale_order_and_partial_order_cannot_delete_queued_add() {
        let (mut actor, lock, path) = fixture().await;
        send(&mut actor, put(1, 0, 1, true)).await.expect("первое");
        send(&mut actor, put(2, 1, 2, true))
            .await
            .expect("queued add");
        assert_eq!(send(&mut actor, serde_json::json!({"command":"routing_order","expected_revision":1,"request_id":3,"ids":[1]})).await.expect_err("stale").key(), "routing_stale");
        assert_eq!(send(&mut actor, serde_json::json!({"command":"routing_order","expected_revision":2,"request_id":4,"ids":[1]})).await.expect_err("неполная перестановка").key(), "id");
        assert_eq!(
            actor
                .state
                .routing
                .rules
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        send(&mut actor, serde_json::json!({"command":"routing_order","expected_revision":2,"request_id":5,"ids":[2,1]})).await.expect("перестановка");
        send(&mut actor, serde_json::json!({"command":"routing_delete","expected_revision":3,"request_id":6,"id":1})).await.expect("явное удаление");
        assert_eq!(
            actor
                .state
                .routing
                .rules
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>(),
            vec![2]
        );
        let saved = actor
            .routing_repository
            .routing_draft()
            .await
            .expect("БД")
            .expect("запись");
        assert_eq!(saved.0, 4);
        drop(actor);
        drop(lock);
        std::fs::remove_dir_all(path).expect("очистка");
    }
    #[tokio::test]
    async fn repository_conflict_returns_matching_error_ack_without_accepting_form() {
        let (mut actor, lock, path) = fixture().await;
        actor
            .routing_repository
            .save_routing_draft(0, actor.state.routing.json().into_bytes())
            .await
            .expect("внешняя запись");
        assert_eq!(
            send(&mut actor, put(1, 0, 9, true))
                .await
                .expect_err("CAS conflict")
                .key(),
            "routing_stale"
        );
        assert!(actor.state.routing.rules.is_empty());
        let snapshot = serde_json::to_value(&actor.state).expect("ACK");
        assert_eq!(snapshot["routing_ack"]["request_id"], 9);
        assert_eq!(snapshot["routing_ack"]["error"], "routing_stale");
        assert!(snapshot["routing_ack"]["saved_id"].is_null());
        drop(actor);
        drop(lock);
        std::fs::remove_dir_all(path).expect("очистка");
    }
    #[tokio::test]
    async fn every_mutation_rejects_stale_snapshot_and_create_is_explicit() {
        let (mut actor, lock, path) = fixture().await;
        send(&mut actor, put(1, 0, 1, true))
            .await
            .expect("добавление");
        for command in [
            put(2, 0, 2, true),
            put(1, 0, 3, false),
            serde_json::json!({"command":"routing_default","expected_revision":0,"request_id":4,"action":"direct"}),
            serde_json::json!({"command":"routing_delete","expected_revision":0,"request_id":5,"id":1}),
        ] {
            assert_eq!(
                send(&mut actor, command)
                    .await
                    .expect_err("старая ревизия")
                    .key(),
                "routing_stale"
            );
        }
        assert_eq!(
            send(&mut actor, put(1, 1, 6, true))
                .await
                .expect_err("ID уже есть")
                .key(),
            "id"
        );
        assert_eq!(
            send(&mut actor, put(2, 1, 7, false))
                .await
                .expect_err("нет ID для update")
                .key(),
            "id"
        );
        assert_eq!(actor.state.routing.rules.len(), 1);
        assert_eq!(actor.routing_revision, Some(1));
        let snapshot = serde_json::to_value(&actor.state).expect("DTO");
        assert_eq!(snapshot["routing_revision"], 1);
        assert_eq!(snapshot["routing_ack"]["request_id"], 7);
        assert_eq!(snapshot["routing_ack"]["error"], "id");
        send(&mut actor, put(1, 1, 8, false)).await.expect("update");
        let snapshot = serde_json::to_value(&actor.state).expect("ACK");
        assert_eq!(snapshot["routing_ack"]["saved_id"], 1);
        assert!(snapshot["routing_ack"]["error"].is_null());
        drop(actor);
        drop(lock);
        std::fs::remove_dir_all(path).expect("очистка");
    }
}
