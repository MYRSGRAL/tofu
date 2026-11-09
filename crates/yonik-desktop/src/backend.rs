//! Отдельный владелец БД и локальной SOCKS-сессии; UI получает только DTO.

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
        atomic::{AtomicU64, Ordering},
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
    ResolvedProfile, SubscriptionError, SubscriptionService, SubscriptionSource,
    TCP_SERIES_SAMPLES, TcpProbeSeriesResult, TcpProbeStatus, TcpProbeTarget, check_tcp_series,
    subscription_master_key,
};
use zeroize::Zeroizing;

/// Явный ephemeral key предназначен для отдельной тестовой БД; fallback отсутствует.
#[derive(Clone)]
pub struct HostConfig {
    data_dir: PathBuf,
    engine_dir: PathBuf,
    ephemeral_key: Option<Zeroizing<[u8; 32]>>,
    listen_port: Option<NonZeroU16>,
}
impl HostConfig {
    pub fn new(data_dir: PathBuf, engine_dir: PathBuf) -> Self {
        Self {
            data_dir,
            engine_dir,
            ephemeral_key: None,
            listen_port: None,
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
const DEFAULT_PROBE_URL: &str = "https://www.gstatic.com/generate_204";
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
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
struct Snapshot {
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

async fn open(config: &mut HostConfig) -> Result<(SubscriptionService, File)> {
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
    Ok((SubscriptionService::new(repo), lock))
}

struct Actor {
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
    fn refresh(&mut self, generation: u64) -> Result<RefreshTask> {
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
        let service = self.service.clone();
        let (cancel, receiver) = watch::channel(false);
        let task =
            tokio::spawn(
                async move { service.refresh(id, Duration::from_secs(30), receiver).await },
            );
        self.state.busy = true;
        self.state.refresh_source_id = Some(id);
        self.state.refresh_token = Some(token.clone());
        Ok((generation, cancel, task, id, token))
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
                self.state.pending = None;
                self.service
                    .commit(pending.approve_after_review())
                    .await
                    .map_err(map_subscription)?;
                self.clear_selection();
                self.page().await
            }
            Command::DiscardRefresh => {
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
            Command::SetEngine { engine } => {
                if self.state.can_stop {
                    return Err(DesktopError("busy"));
                }
                self.state.engine = engine;
                Ok(())
            }
            Command::SetListenPort { port } => {
                if self.state.can_stop {
                    return Err(DesktopError("busy"));
                }
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
                self.probe_url = url;
                self.state.probe_custom = true;
                self.state.probe_expected_status = expected_status;
                self.state.probe_result = None;
                Ok(())
            }
            Command::ResetProbeTarget => {
                if self.state.can_stop {
                    return Err(DesktopError("busy"));
                }
                self.probe_url = Zeroizing::new(DEFAULT_PROBE_URL.into());
                self.state.probe_custom = false;
                self.state.probe_expected_status = 204;
                self.state.probe_result = None;
                Ok(())
            }
            _ => Err(DesktopError("invalid_input")),
        }
    }
    async fn refreshed(
        &mut self,
        result: std::result::Result<RefreshResult, SubscriptionError>,
    ) -> Result<()> {
        match result.map_err(map_subscription)? {
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
        }
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
    use super::{Actor, DEFAULT_PROBE_URL, HostConfig, Snapshot, open};
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
        let (service, _lock) = open(&mut config).await.expect("тестовая БД");
        let mut actor = Actor {
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
type RefreshTask = (
    u64,
    watch::Sender<bool>,
    tokio::task::JoinHandle<std::result::Result<RefreshResult, SubscriptionError>>,
    i64,
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
    let (service, _lock) = match opened {
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
    state.ready = true;
    let network_monitor = NetworkMonitor::start().ok();
    let mut network = network_monitor.as_ref().map(NetworkMonitor::watch);
    synchronize_network(&mut state, network.as_ref().map(NetworkWatch::current));
    let mut actor = Actor {
        service,
        state,
        callback,
        selected: None,
        pending: None,
        automatic_port: config.listen_port.is_none(),
        probe_url: Zeroizing::new(DEFAULT_PROBE_URL.into()),
        network: network.clone(),
    };
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
    loop {
        tokio::select! {
            biased;
            changed = priority.changed() => {
                if changed.is_err() { quitting = true; }
                let action = *priority.borrow_and_update();
                quitting |= action.shutdown;
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
                let (generation, cancel, _, _, _) = refreshing.take().expect("guard");
                actor.state.busy = false;
                actor.state.refresh_source_id = None;
                actor.state.refresh_token = None;
                if generation == control.generation.load(Ordering::SeqCst) && !quitting { actor.state.source_error = None; }
                if generation == control.generation.load(Ordering::SeqCst) && !quitting && !*cancel.borrow() {
                    let outcome = match result { Ok(result) => actor.refreshed(result).await, Err(_) => Err(DesktopError("fetch")) };
                    if let Err(error) = outcome { actor.state.command_error(true, error.key()); }
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
                actor.state.begin_command(&envelope.command);
                let outcome = match envelope.command {
                    Command::CancelRefresh { id, token } => {
                        match refreshing.as_ref() {
                            Some((_, cancel, _, source, current_token)) if *source == id && token == *current_token && !*cancel.borrow() => {
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
                                    match actor.refresh(envelope.generation) {
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
                                Ok(()) => match actor.refresh(envelope.generation) {
                                    Ok(task) => { refreshing = Some(task); Ok(()) }
                                    Err(error) => Err(error),
                                },
                                Err(error) => Err(error),
                            }
                        }
                    }
                    Command::Refresh => {
                        if refreshing.is_some() { Err(DesktopError("busy")) }
                        else { match actor.refresh(envelope.generation) {
                            Ok(task) => { refreshing = Some(task); Ok(()) }
                            Err(error) => Err(error),
                        } }
                    }
                    Command::Connect => {
                        if actor.state.can_stop || running.is_some() || refreshing.is_some() { Err(DesktopError("busy")) }
                        else if actor.selected.is_none() { Err(DesktopError("profile_required")) }
                        else if actor.pending.is_some() { Err(DesktopError("review_required")) }
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
