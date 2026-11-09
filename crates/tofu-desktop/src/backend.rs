//! Отдельный владелец БД и локальной SOCKS-сессии; UI получает только DTO.

use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, DirBuilder, File, OpenOptions},
    num::NonZeroU16,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tofu_engine::Engine;
use tofu_linux::{
    BinarySnapshot, Cancellation, CheckedConfig, CredentialError, EncryptedSubscriptionRepository,
    FetchError, FetchRoute, HostError, LocalSession, PendingRefresh, ProbeTarget, ProfileQuery,
    RefreshResult, ResolvedProfile, SubscriptionError, SubscriptionService, SubscriptionSource,
    subscription_master_key,
};
use tokio::sync::{mpsc, watch};
use zeroize::Zeroizing;

/// Явный ephemeral key предназначен для отдельной тестовой БД; fallback отсутствует.
#[derive(Clone)]
pub struct HostConfig {
    data_dir: PathBuf,
    engine_dir: PathBuf,
    ephemeral_key: Option<Zeroizing<[u8; 32]>>,
    listen_port: u16,
}
impl HostConfig {
    pub fn new(data_dir: PathBuf, engine_dir: PathBuf) -> Self {
        Self {
            data_dir,
            engine_dir,
            ephemeral_key: None,
            listen_port: 10808,
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
        self.listen_port = port.get();
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

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Reload,
    AddSource {
        name: String,
        url: String,
        local_exception: bool,
        route: Route,
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
    Connect,
    Stop,
    Shutdown,
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
    closed: (Mutex<bool>, Condvar),
}
impl Control {
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
        let control = Arc::new(Control {
            generation: AtomicU64::new(0),
            priority,
            closed: (Mutex::new(false), Condvar::new()),
        });
        let worker_control = control.clone();
        let callback: Callback = Arc::new(callback);
        std::thread::Builder::new()
            .name("tofu-backend".into())
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
                        snapshot.port = config.listen_port;
                        snapshot.error = Some("startup");
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
            command => self
                .commands
                .try_send(Envelope {
                    generation: self.control.generation.load(Ordering::SeqCst),
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
        Command::AddSource { name, url, .. } => {
            !name.trim().is_empty() && text(name, 512) && text(url, 8192)
        }
        Command::Query { search, offset, .. } => text(search, 512) && *offset <= 40_000,
        Command::SourcePage { offset } => *offset <= 40_000,
        Command::SelectSource { id } | Command::Favorite { id, .. } => *id > 0,
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
}
#[derive(Serialize)]
struct Snapshot {
    ready: bool,
    busy: bool,
    error: Option<&'static str>,
    pending: Option<PendingDto>,
    sources: Vec<SourceDto>,
    sources_total: usize,
    sources_offset: usize,
    source_id: Option<i64>,
    profiles: Vec<ProfileDto>,
    total: usize,
    offset: usize,
    revision: i64,
    status: &'static str,
    selected_profile: Option<i64>,
    selected_name: Option<String>,
    engine: EngineChoice,
    port: u16,
    can_connect: bool,
    closed: bool,
    ephemeral: bool,
    can_stop: bool,
    active_name: Option<String>,
    active_engine: Option<EngineChoice>,
}
impl Snapshot {
    fn new(ephemeral: bool) -> Self {
        Self {
            ready: false,
            busy: false,
            error: None,
            pending: None,
            sources: Vec::new(),
            sources_total: 0,
            sources_offset: 0,
            source_id: None,
            profiles: Vec::new(),
            total: 0,
            offset: 0,
            revision: 0,
            status: "Disconnected",
            selected_profile: None,
            selected_name: None,
            engine: EngineChoice::Xray,
            port: 10808,
            can_connect: false,
            closed: false,
            ephemeral,
            can_stop: false,
            active_name: None,
            active_engine: None,
        }
    }
    fn emit(&self, callback: &Callback) {
        if let Ok(json) = serde_json::to_string(self) {
            callback(json);
        }
    }
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
    search: String,
    favorites_only: bool,
}
impl Actor {
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
            })
            .collect();
        Ok(())
    }
    async fn page(&mut self) -> Result<()> {
        self.state.profiles.clear();
        self.state.total = 0;
        self.state.revision = 0;
        let Some(id) = self.state.source_id else {
            return Ok(());
        };
        let page = self
            .service
            .profiles(
                id,
                ProfileQuery {
                    search: self.search.clone(),
                    favorites_only: self.favorites_only,
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
            } => {
                if matches!(route, Route::Proxy) {
                    return Err(DesktopError("unsupported_proxy"));
                }
                let source =
                    SubscriptionSource::new(name, url, local_exception, FetchRoute::Direct)
                        .map_err(map_subscription)?;
                let id = self.service.add(source).await.map_err(map_subscription)?;
                self.state.source_id = Some(id);
                self.state.offset = 0;
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
            Command::Query {
                search,
                favorites_only,
                offset,
            } => {
                self.search = search;
                self.favorites_only = favorites_only;
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
                    self.state.pending = Some(PendingDto {
                        accepted: summary.accepted,
                        rejected: summary.rejected,
                        flagged: summary.flagged,
                        native_json: summary.native_json,
                    });
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
        SubscriptionError::Fetch(FetchError::UnsupportedPeerVerification) => "unsupported_proxy",
        SubscriptionError::Fetch(_) => "fetch",
        SubscriptionError::Import(_) => "import",
        _ => "storage",
    })
}

struct SessionUpdate {
    status: &'static str,
    error: Option<&'static str>,
}
type RefreshTask = (
    u64,
    watch::Sender<bool>,
    tokio::task::JoinHandle<std::result::Result<RefreshResult, SubscriptionError>>,
);

struct Running {
    cancel: watch::Sender<bool>,
    task: tokio::task::JoinHandle<std::result::Result<(), LocalSession>>,
}

async fn session(
    config: tofu_engine::CompiledConfig,
    engine_dir: PathBuf,
    engine: Engine,
    mut cancel: Cancellation,
    updates: mpsc::Sender<SessionUpdate>,
) -> std::result::Result<(), LocalSession> {
    let Ok(mut session) = LocalSession::new() else {
        let _ = updates
            .send(SessionUpdate {
                status: "Error",
                error: Some("engine_failed"),
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
        let _ = updates
            .send(SessionUpdate {
                status: "Checking",
                error: None,
            })
            .await;
        let target = ProbeTarget::new(
            "https://www.gstatic.com/generate_204",
            Duration::from_secs(10),
        )
        .map_err(|_| "check_failed")?;
        let passed = session
            .check(&target, &mut cancel)
            .await
            .map_err(|e| {
                if e == HostError::Cancelled {
                    "cancelled"
                } else {
                    "check_failed"
                }
            })?
            .passed();
        let _ = updates
            .send(SessionUpdate {
                status: if passed { "Connected" } else { "Limited" },
                error: if passed { None } else { Some("check_failed") },
            })
            .await;
        session.wait_exit(&mut cancel).await.map_err(|e| {
            if e == HostError::Cancelled {
                "cancelled"
            } else {
                "engine_failed"
            }
        })
    }
    .await;
    if session.stop().await.is_err() {
        let _ = updates
            .send(SessionUpdate {
                status: "CleanupError",
                error: Some("cleanup_failed"),
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
    state.port = config.listen_port;
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
    let opened = open(&mut config).await;
    state.busy = false;
    let (service, _lock) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            state.error = Some(error.key());
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
    let mut actor = Actor {
        service,
        state,
        callback,
        selected: None,
        pending: None,
        search: String::new(),
        favorites_only: false,
    };
    if let Err(error) = actor.reload().await {
        actor.state.error = Some(error.key());
    }
    actor.emit();
    let (updates, mut events) = mpsc::channel::<SessionUpdate>(16);
    let mut running: Option<Running> = None;
    let mut failed_session: Option<LocalSession> = None;
    let mut refreshing: Option<RefreshTask> = None;
    let mut quitting = false;
    loop {
        tokio::select! {
            biased;
            changed = priority.changed() => {
                if changed.is_err() { quitting = true; }
                let action = *priority.borrow_and_update();
                quitting |= action.shutdown;
                actor.pending = None; actor.state.pending = None;
                if let Some((_, cancel, _)) = refreshing.as_ref() { cancel.send_replace(true); }
                if let Some(session) = running.as_ref() { session.cancel.send_replace(true); }
                if let Some(mut session) = failed_session.take() {
                    if session.stop().await.is_err() { failed_session = Some(session); actor.state.error = Some("cleanup_failed"); }
                }
                if running.is_none() && failed_session.is_none() {
                    actor.state.can_stop = false; actor.state.active_name = None; actor.state.active_engine = None;
                }
                actor.state.status = if running.is_some() { "Stopping" } else if failed_session.is_some() { "CleanupError" } else { "Disconnected" };
                actor.emit();
            }
            result = async { (&mut running.as_mut().expect("guard").task).await }, if running.is_some() => {
                running = None;
                if let Ok(Err(session)) = result { failed_session = Some(session); actor.state.error = Some("cleanup_failed"); }
                if failed_session.is_none() { actor.state.can_stop = false; actor.state.active_name = None; actor.state.active_engine = None; }
                // Финальный event может ожидать в канале; сначала применяем его.
                while let Ok(update) = events.try_recv() { actor.state.status = update.status; actor.state.error = update.error; }
                actor.emit();
            }
            result = async { (&mut refreshing.as_mut().expect("guard").2).await }, if refreshing.is_some() => {
                let (generation, _, _) = refreshing.take().expect("guard");
                actor.state.busy = false;
                if generation == control.generation.load(Ordering::SeqCst) && !quitting {
                    let outcome = match result { Ok(result) => actor.refreshed(result).await, Err(_) => Err(DesktopError("fetch")) };
                    if let Err(error) = outcome { actor.state.error = Some(error.key()); }
                }
                actor.emit();
            }
            Some(update) = events.recv() => {
                actor.state.status = update.status; actor.state.error = update.error; actor.emit();
            }
            command = commands.recv(), if !quitting => {
                let Some(envelope) = command else { control.interrupt(true); continue; };
                if envelope.generation != control.generation.load(Ordering::SeqCst) { continue; }
                actor.state.error = None;
                let outcome = match envelope.command {
                    Command::Refresh => {
                        if refreshing.is_some() { Err(DesktopError("busy")) }
                        else if let Some(id) = actor.state.source_id {
                            actor.pending = None; actor.state.pending = None;
                            let service = actor.service.clone();
                            let (cancel, receiver) = watch::channel(false);
                            let task = tokio::spawn(async move { service.refresh(id, Duration::from_secs(30), receiver).await });
                            refreshing = Some((envelope.generation, cancel, task)); actor.state.busy = true; Ok(())
                        } else { Err(DesktopError("source_required")) }
                    }
                    Command::Connect => {
                        if actor.state.can_stop || running.is_some() || refreshing.is_some() { Err(DesktopError("busy")) }
                        else if let Some(profile) = actor.selected.as_ref() {
                            if actor.pending.is_some() { Err(DesktopError("review_required")) }
                            else {
                                match NonZeroU16::new(actor.state.port).and_then(|port| tofu_engine::compile(&profile.server, actor.state.engine.engine(), port).ok()) {
                                    Some(compiled) => {
                                        let (cancel, receiver) = Cancellation::new();
                                        let task = tokio::spawn(session(compiled, config.engine_dir.clone(), actor.state.engine.engine(), receiver, updates.clone()));
                                        running = Some(Running { cancel, task }); actor.state.can_stop = true;
                                        actor.state.active_name = Some(profile.profile.name.clone()); actor.state.active_engine = Some(actor.state.engine);
                                        actor.state.status = "Preparing"; Ok(())
                                    }
                                    None => Err(DesktopError("engine_config")),
                                }
                            }
                        } else { Err(DesktopError("profile_required")) }
                    }
                    command => {
                        if refreshing.is_some() && matches!(command, Command::SelectSource { .. } | Command::AddSource { .. } | Command::ApproveRefresh) {
                            Err(DesktopError("busy"))
                        } else { actor.command(command).await }
                    }
                };
                if let Err(error) = outcome { actor.state.error = Some(error.key()); }
                actor.emit();
            }
        }
        if quitting && running.is_none() && refreshing.is_none() {
            if failed_session.is_some() {
                quitting = false;
                actor.state.error = Some("cleanup_failed");
                actor.emit();
                continue;
            }
            break;
        }
    }
    actor.pending = None;
    actor.selected = None;
    let closed = actor.service.close().await.is_ok();
    drop(_lock);
    actor.state.ready = false;
    actor.state.busy = false;
    actor.state.can_connect = false;
    actor.state.closed = closed;
    actor.state.error = if closed { None } else { Some("cleanup_failed") };
    actor.state.emit(&actor.callback);
    if closed {
        control.closed();
    }
    if let Some(task) = signals {
        task.abort();
    }
}
