//! Связь FetchHost, отчёта импорта и атомарного Repository. Сессия не меняется.

use crate::{
    EncryptedSubscriptionRepository, FetchError, FetchHost, FetchResponse, FetchRoute,
    FetchValidators, ProfilePage, ProfileQuery, RepositoryError, ResolvedProfile, SnapshotInput,
    Source,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt,
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore, watch};
use yonik_import::ImportReport;
pub use yonik_subscription::scheduler::{
    Policy as SubscriptionPolicy, RetryState as SubscriptionRetryState,
};
use yonik_subscription::{ApprovedUpdate, StagedUpdate, Subscription, Summary};

/// Подменяемая граница транспорта; реализация обязана соблюдать route и отмену.
pub trait SubscriptionFetch: Send + Sync {
    fn fetch<'a>(
        &'a self,
        source: &'a Source,
        route: FetchRoute,
        validators: Option<&'a FetchValidators>,
        cancel: watch::Receiver<bool>,
    ) -> futures_util::future::BoxFuture<'a, Result<FetchResponse, FetchError>>;
}
impl SubscriptionFetch for FetchHost {
    fn fetch<'a>(
        &'a self,
        source: &'a Source,
        route: FetchRoute,
        validators: Option<&'a FetchValidators>,
        cancel: watch::Receiver<bool>,
    ) -> futures_util::future::BoxFuture<'a, Result<FetchResponse, FetchError>> {
        Box::pin(FetchHost::fetch(self, source, route, validators, cancel))
    }
}

static CONTROLLERS: AtomicU64 = AtomicU64::new(1);
pub(crate) static IMPORT_GATE: Semaphore = Semaphore::const_new(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionError {
    InvalidSource,
    CorruptSnapshot,
    NotFound,
    NoSnapshot,
    Exhausted,
    ForeignApproval,
    StaleUpdate,
    Worker,
    Fetch(FetchError),
    Import(yonik_subscription::Error),
    Repository(RepositoryError),
}
impl fmt::Display for SubscriptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidSource => "Некорректный источник подписки",
            Self::CorruptSnapshot => "Сохранённая подписка не прошла проверку",
            Self::NotFound => "Подписка не найдена",
            Self::NoSnapshot => "Подписка ещё не имеет snapshot",
            Self::Exhausted => "Исчерпаны идентификаторы обновлений",
            Self::ForeignApproval => "Согласие относится к другому хранилищу подписок",
            Self::StaleUpdate => "Обновление подписки устарело или отменено",
            Self::Worker => "Не удалось завершить обработку подписки",
            Self::Fetch(_) => "Не удалось загрузить подписку",
            Self::Import(_) => "Обновление не принято импортером",
            Self::Repository(_) => "Не удалось сохранить подписку",
        })
    }
}
impl std::error::Error for SubscriptionError {}
impl From<RepositoryError> for SubscriptionError {
    fn from(error: RepositoryError) -> Self {
        Self::Repository(error)
    }
}

/// Время в целых секундах UNIX; содержимое остаётся внутри encrypted source payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleState {
    pub version: u8,
    pub due_wall: Option<u64>,
    pub recorded_wall: u64,
    pub failures: u32,
    pub paused: bool,
}
impl ScheduleState {
    pub fn validate(self) -> Result<(), SubscriptionError> {
        if self.version != 1
            || self.failures > 5
            || self.paused != self.due_wall.is_none()
            || (self.failures == 5 && !self.paused)
            || self.recorded_wall > i64::MAX as u64
            || self.due_wall.is_some_and(|due| {
                due < self.recorded_wall
                    || due - self.recorded_wall > u32::MAX as u64
                    || due > i64::MAX as u64
            })
        {
            return Err(SubscriptionError::InvalidSource);
        }
        Ok(())
    }
    pub fn deadline(self, wall: u64, monotonic: Duration) -> Duration {
        let remaining = if wall < self.recorded_wall {
            self.due_wall
                .unwrap_or(wall)
                .saturating_sub(self.recorded_wall)
        } else {
            self.due_wall.unwrap_or(wall).saturating_sub(wall)
        }
        .min(u32::MAX as u64);
        monotonic
            .checked_add(Duration::from_secs(remaining))
            .unwrap_or(monotonic)
    }
}

/// Неизменная идентичность ресурса, захваченная до запуска операции.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscriptionIdentity {
    pub id: i64,
    pub generation: i64,
}

/// CAS token без публичного доступа к URL/credentials.
pub struct ScheduleMetadata {
    pub id: i64,
    pub generation: i64,
    pub state: Option<ScheduleState>,
    payload: zeroize::Zeroizing<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceData {
    version: u8,
    name: String,
    url: String,
    local_exception: bool,
    proxy: bool,
    #[serde(default)]
    hwid_enabled: bool,
    #[serde(default)]
    hwid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    schedule: Option<ScheduleState>,
}

/// Debug намеренно не реализован: имя и URL могут содержать секреты.
pub struct SubscriptionSource(SourceData);
impl SubscriptionSource {
    pub fn new(
        name: String,
        url: String,
        local_exception: bool,
        route: FetchRoute,
    ) -> Result<Self, SubscriptionError> {
        let value = Self(SourceData {
            version: 1,
            name,
            url,
            local_exception,
            proxy: route == FetchRoute::Proxy,
            hwid_enabled: false,
            hwid: None,
            schedule: None,
        });
        value.validate()?;
        Ok(value)
    }

    pub fn with_hwid(mut self, enabled: bool) -> Result<Self, SubscriptionError> {
        if enabled && self.0.hwid.is_none() {
            let mut random = [0_u8; 16];
            getrandom::getrandom(&mut random).map_err(|_| SubscriptionError::Worker)?;
            let mut id = String::with_capacity(32);
            for byte in random {
                use std::fmt::Write;
                write!(&mut id, "{byte:02x}").map_err(|_| SubscriptionError::Worker)?;
            }
            self.0.hwid = Some(id);
        }
        self.0.hwid_enabled = enabled;
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<Source, SubscriptionError> {
        if let Some(state) = self.0.schedule {
            state.validate()?;
        }
        if self.0.version != 1
            || self.0.name.trim().is_empty()
            || self.0.name.len() > 512
            || self.0.name.chars().any(char::is_control)
            || self
                .0
                .hwid
                .as_deref()
                .is_some_and(|value| !crate::fetch::valid_hwid(value))
        {
            return Err(SubscriptionError::InvalidSource);
        }
        let source = source_url(&self.0.url, self.0.local_exception)?;
        if self.0.hwid_enabled {
            source
                .with_hwid(
                    self.0
                        .hwid
                        .as_deref()
                        .ok_or(SubscriptionError::InvalidSource)?,
                )
                .map_err(|_| SubscriptionError::InvalidSource)
        } else {
            Ok(source)
        }
    }

    fn encode(&self) -> Result<Vec<u8>, SubscriptionError> {
        self.validate()?;
        serde_json::to_vec(&self.0).map_err(|_| SubscriptionError::InvalidSource)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidatorData {
    resource: String,
    etag: Option<String>,
    last_modified: Option<String>,
}

fn source_url(url: &str, local: bool) -> Result<Source, SubscriptionError> {
    (if local {
        Source::local(url)
    } else {
        Source::public(url)
    })
    .map_err(|_| SubscriptionError::InvalidSource)
}
fn encode_validators(value: &FetchValidators) -> Result<Vec<u8>, SubscriptionError> {
    serde_json::to_vec(&ValidatorData {
        resource: value.resource_url().to_owned(),
        etag: value.etag().map(str::to_owned),
        last_modified: value.last_modified().map(str::to_owned),
    })
    .map_err(|_| SubscriptionError::CorruptSnapshot)
}
fn decode_validators(
    bytes: &[u8],
    spec: &SubscriptionSource,
) -> Result<FetchValidators, SubscriptionError> {
    let value: ValidatorData =
        serde_json::from_slice(bytes).map_err(|_| SubscriptionError::CorruptSnapshot)?;
    let mut source = source_url(&value.resource, spec.0.local_exception)?;
    if spec.0.hwid_enabled {
        source = source
            .with_hwid(
                spec.0
                    .hwid
                    .as_deref()
                    .ok_or(SubscriptionError::InvalidSource)?,
            )
            .map_err(|_| SubscriptionError::InvalidSource)?;
    }
    FetchValidators::new(
        &source,
        value.etag.as_deref(),
        value.last_modified.as_deref(),
    )
    .map_err(|_| SubscriptionError::CorruptSnapshot)
}

#[derive(Clone)]
pub struct SubscriptionService {
    repository: EncryptedSubscriptionRepository,
    identity: Arc<()>,
    requests: Arc<Mutex<HashMap<i64, Request>>>,
    source_updates: Arc<Mutex<()>>,
}
struct Request {
    serial: u64,
    cancellation: watch::Sender<bool>,
    ack: Arc<Semaphore>,
}
pub enum RefreshResult {
    NotModified,
    Pending(Box<PendingRefresh>),
}
pub struct OfflineSnapshot {
    pub report: ImportReport,
    pub revision: i64,
    pub source_generation: i64,
    pub current_source_generation: i64,
}
pub struct SubscriptionSummary {
    pub id: i64,
    pub name: String,
    pub hwid_enabled: bool,
}
pub struct SubscriptionSourcesPage {
    pub sources: Vec<SubscriptionSummary>,
    pub total: usize,
    pub offset: usize,
}
pub struct PendingRefresh {
    service: Arc<()>,
    request: u64,
    id: i64,
    generation: i64,
    revision: i64,
    validators: Vec<u8>,
    controller: Subscription,
    staged: StagedUpdate,
}
pub struct ApprovedRefresh {
    pending: PendingData,
    approved: ApprovedUpdate,
}
struct PendingData {
    service: Arc<()>,
    request: u64,
    id: i64,
    generation: i64,
    revision: i64,
    validators: Vec<u8>,
    controller: Subscription,
}

impl PendingRefresh {
    pub fn identity(&self) -> SubscriptionIdentity {
        SubscriptionIdentity {
            id: self.id,
            generation: self.generation,
        }
    }
    pub fn summary(&self) -> Summary {
        self.staged.summary()
    }
    pub fn report(&self) -> &ImportReport {
        self.staged.report()
    }
    pub fn accept_clean(self) -> Result<ApprovedRefresh, SubscriptionError> {
        let (pending, staged) = self.parts();
        let approved = staged.accept_clean().map_err(SubscriptionError::Import)?;
        Ok(ApprovedRefresh { pending, approved })
    }
    /// UI вызывает только после согласия на показанный неизменяемый отчёт.
    pub fn approve_after_review(self) -> ApprovedRefresh {
        let (pending, staged) = self.parts();
        ApprovedRefresh {
            pending,
            approved: staged.approve_after_review(),
        }
    }
    fn parts(self) -> (PendingData, StagedUpdate) {
        (
            PendingData {
                service: self.service,
                request: self.request,
                id: self.id,
                generation: self.generation,
                revision: self.revision,
                validators: self.validators,
                controller: self.controller,
            },
            self.staged,
        )
    }
}

impl SubscriptionService {
    pub fn new(repository: EncryptedSubscriptionRepository) -> Self {
        Self {
            repository,
            identity: Arc::new(()),
            requests: Arc::new(Mutex::new(HashMap::new())),
            source_updates: Arc::new(Mutex::new(())),
        }
    }

    /// Последний handle сервиса в GUI освобождается вне UI-потока.
    pub async fn close(self) -> Result<(), SubscriptionError> {
        self.repository
            .close()
            .await
            .map_err(SubscriptionError::Repository)
    }

    pub async fn add(&self, source: SubscriptionSource) -> Result<i64, SubscriptionError> {
        Ok(self.repository.create_source(source.encode()?).await?.id)
    }

    pub async fn schedule_metadata(
        &self,
        id: i64,
    ) -> Result<Option<ScheduleMetadata>, SubscriptionError> {
        self.repository
            .source_record_by_id(id)
            .await?
            .map(Self::schedule_record)
            .transpose()
    }
    fn schedule_record(record: crate::SourceRecord) -> Result<ScheduleMetadata, SubscriptionError> {
        let data: SourceData = serde_json::from_slice(&record.payload)
            .map_err(|_| SubscriptionError::InvalidSource)?;
        if let Some(state) = data.schedule {
            state.validate()?;
        }
        Ok(ScheduleMetadata {
            id: record.id,
            generation: record.generation,
            state: data.schedule,
            payload: record.payload,
        })
    }
    pub async fn schedule_metadata_page(
        &self,
        offset: usize,
    ) -> Result<Vec<ScheduleMetadata>, SubscriptionError> {
        self.repository
            .source_records(offset, 64)
            .await?
            .records
            .into_iter()
            .map(Self::schedule_record)
            .collect()
    }
    pub async fn persist_schedule(
        &self,
        token: ScheduleMetadata,
        state: ScheduleState,
    ) -> Result<(), SubscriptionError> {
        state.validate()?;
        let mut data: SourceData =
            serde_json::from_slice(&token.payload).map_err(|_| SubscriptionError::InvalidSource)?;
        data.schedule = Some(state);
        let payload = serde_json::to_vec(&data).map_err(|_| SubscriptionError::InvalidSource)?;
        self.repository
            .update_source_metadata(token.id, token.generation, token.payload, payload)
            .await?;
        Ok(())
    }

    pub async fn reserve_manual_schedule(
        &self,
        identity: SubscriptionIdentity,
        wall: u64,
    ) -> Result<(), SubscriptionError> {
        let token = self
            .schedule_metadata(identity.id)
            .await?
            .ok_or(SubscriptionError::NotFound)?;
        if token.generation != identity.generation {
            return Err(SubscriptionError::StaleUpdate);
        }
        let failures = token
            .state
            .map_or(0, |s| if s.paused { 0 } else { s.failures });
        self.persist_schedule(
            token,
            ScheduleState {
                version: 1,
                due_wall: wall.checked_add(86400),
                recorded_wall: wall,
                failures,
                paused: false,
            },
        )
        .await
    }

    /// Завершение ручной операции использует тот же retry-контракт.
    pub async fn record_schedule_completion(
        &self,
        identity: SubscriptionIdentity,
        success: bool,
        wall: u64,
    ) -> Result<(), SubscriptionError> {
        let token = self
            .schedule_metadata(identity.id)
            .await?
            .ok_or(SubscriptionError::NotFound)?;
        if token.generation != identity.generation {
            return Err(SubscriptionError::StaleUpdate);
        }
        let state = yonik_subscription::scheduler::Scheduler::completion_state(
            token.state.map_or(0, |s| s.failures),
            Duration::ZERO,
            success,
        );
        let due_wall = state
            .due
            .and_then(|delay| wall.checked_add(delay.as_secs()));
        self.persist_schedule(
            token,
            ScheduleState {
                version: 1,
                due_wall,
                recorded_wall: wall,
                failures: state.failures,
                paused: due_wall.is_none(),
            },
        )
        .await
    }

    /// Текущее поколение без чтения snapshot; None после удаления.
    pub async fn schedule_generation(&self, id: i64) -> Result<Option<i64>, SubscriptionError> {
        Ok(self
            .repository
            .source_record_by_id(id)
            .await?
            .map(|record| record.generation))
    }

    /// Ограниченная страница идентификаторов и поколений без snapshot body и секретов.
    pub async fn schedule_sources(
        &self,
        offset: usize,
    ) -> Result<Vec<(i64, i64)>, SubscriptionError> {
        Ok(self
            .repository
            .source_records(offset, 64)
            .await?
            .records
            .into_iter()
            .map(|record| (record.id, record.generation))
            .collect())
    }

    pub async fn sources_page(
        &self,
        offset: usize,
    ) -> Result<SubscriptionSourcesPage, SubscriptionError> {
        let page = self.repository.source_records(offset, 64).await?;
        let mut sources = Vec::with_capacity(page.records.len());
        for record in page.records {
            let data: SourceData = serde_json::from_slice(&record.payload)
                .map_err(|_| SubscriptionError::InvalidSource)?;
            let source = SubscriptionSource(data);
            source.validate()?;
            sources.push(SubscriptionSummary {
                id: record.id,
                name: source.0.name,
                hwid_enabled: source.0.hwid_enabled,
            });
        }
        Ok(SubscriptionSourcesPage {
            sources,
            total: page.total,
            offset,
        })
    }

    pub async fn profiles(
        &self,
        source: i64,
        query: ProfileQuery,
    ) -> Result<ProfilePage, SubscriptionError> {
        self.repository
            .profiles(source, query)
            .await
            .map_err(SubscriptionError::Repository)
    }

    pub async fn set_favorite(
        &self,
        profile: i64,
        favorite: bool,
    ) -> Result<(), SubscriptionError> {
        self.repository
            .set_favorite(profile, favorite)
            .await
            .map_err(SubscriptionError::Repository)
    }

    /// Выбор относится к конкретному snapshot списка. Repository возвращает
    /// сам parsed server из того же snapshot; повторное чтение по ordinal
    /// после resolve создало бы гонку с обновлением подписки.
    pub async fn select_profile(
        &self,
        profile: i64,
        snapshot_revision: i64,
    ) -> Result<ResolvedProfile, SubscriptionError> {
        self.repository
            .resolve_profile(profile, snapshot_revision)
            .await
            .map_err(SubscriptionError::Repository)
    }
    pub async fn change_source(
        &self,
        id: i64,
        generation: i64,
        source: SubscriptionSource,
    ) -> Result<i64, SubscriptionError> {
        let mut source = source;
        source.0.schedule = None;
        let _source_update = self.source_updates.lock().await;
        let generation = self
            .repository
            .change_source(id, generation, source.encode()?)
            .await?
            .generation;
        let mut requests = self.requests.lock().await;
        if let Some(request) = requests.get_mut(&id) {
            request.cancellation.send_replace(true);
        }
        Ok(generation)
    }

    pub async fn rename_source(&self, id: i64, name: String) -> Result<(), SubscriptionError> {
        let _source_update = self.source_updates.lock().await;
        let record = self
            .repository
            .source_record_by_id(id)
            .await?
            .ok_or(SubscriptionError::NotFound)?;
        let data = serde_json::from_slice(&record.payload)
            .map_err(|_| SubscriptionError::InvalidSource)?;
        let mut source = SubscriptionSource(data);
        source.0.name = name;
        self.repository
            .update_source_metadata(id, record.generation, record.payload, source.encode()?)
            .await?;
        Ok(())
    }

    pub async fn delete_source(&self, id: i64) -> Result<(), SubscriptionError> {
        let _source_update = self.source_updates.lock().await;
        self.repository.delete_source(id).await?;
        let mut requests = self.requests.lock().await;
        if let Some(request) = requests.remove(&id) {
            request.cancellation.send_replace(true);
        }
        Ok(())
    }

    pub async fn source_summary(&self, id: i64) -> Result<SubscriptionSummary, SubscriptionError> {
        let record = self
            .repository
            .source_record_by_id(id)
            .await?
            .ok_or(SubscriptionError::NotFound)?;
        let data = serde_json::from_slice(&record.payload)
            .map_err(|_| SubscriptionError::InvalidSource)?;
        let source = SubscriptionSource(data);
        source.validate()?;
        Ok(SubscriptionSummary {
            id,
            name: source.0.name,
            hwid_enabled: source.0.hwid_enabled,
        })
    }

    pub async fn source_hwid_enabled(&self, id: i64) -> Result<bool, SubscriptionError> {
        let record = self
            .repository
            .load(id)
            .await?
            .ok_or(SubscriptionError::NotFound)?;
        let data = serde_json::from_slice(&record.source.payload)
            .map_err(|_| SubscriptionError::InvalidSource)?;
        let source = SubscriptionSource(data);
        source.validate()?;
        Ok(source.0.hwid_enabled)
    }

    pub async fn set_source_hwid(&self, id: i64, enabled: bool) -> Result<(), SubscriptionError> {
        let _source_update = self.source_updates.lock().await;
        let record = self
            .repository
            .load(id)
            .await?
            .ok_or(SubscriptionError::NotFound)?;
        let data = serde_json::from_slice(&record.source.payload)
            .map_err(|_| SubscriptionError::InvalidSource)?;
        let source = SubscriptionSource(data);
        source.validate()?;
        if source.0.hwid_enabled == enabled {
            return Ok(());
        }
        let mut source = source.with_hwid(enabled)?;
        source.0.schedule = None;
        self.repository
            .change_source(id, record.source.generation, source.encode()?)
            .await?;
        let mut requests = self.requests.lock().await;
        if let Some(request) = requests.get_mut(&id) {
            request.cancellation.send_replace(true);
        }
        Ok(())
    }

    pub async fn cancel(&self, id: i64) -> bool {
        let (requested, ack) = {
            let requests = self.requests.lock().await;
            let Some(request) = requests.get(&id) else {
                return false;
            };
            (
                !request.cancellation.send_replace(true),
                request.ack.clone(),
            )
        };
        // Worker владеет permit до завершения parse/rollback/COMMIT.
        let _ack = ack.acquire().await;
        requested
    }

    /// Возвращает исходный утверждённый snapshot без сети. Capability-check
    /// конкретного ядра обязателен отдельно перед активацией.
    pub async fn offline(&self, id: i64) -> Result<OfflineSnapshot, SubscriptionError> {
        let (_sender, cancel) = crate::Cancellation::new();
        self.offline_cancellable(id, cancel).await
    }

    pub async fn offline_cancellable(
        &self,
        id: i64,
        mut cancel: crate::Cancellation,
    ) -> Result<OfflineSnapshot, SubscriptionError> {
        cancel.check().map_err(|_| cancelled())?;
        let record = self
            .repository
            .load(id)
            .await?
            .ok_or(SubscriptionError::NotFound)?;
        let snapshot = record.snapshot.ok_or(SubscriptionError::NoSnapshot)?;
        let permit = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(cancelled()),
            result = IMPORT_GATE.acquire() => result.map_err(|_| SubscriptionError::Worker)?,
        };
        let report = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            yonik_import::import_bytes_cancellable(snapshot.body.to_vec(), || {
                cancel.check().is_ok()
            })
            .map_err(|error| match error {
                yonik_import::CancellableImportError::Cancelled => cancelled(),
                yonik_import::CancellableImportError::Import(_) => {
                    SubscriptionError::CorruptSnapshot
                }
            })
        })
        .await
        .map_err(|_| SubscriptionError::Worker)??;
        Ok(OfflineSnapshot {
            report,
            revision: snapshot.revision,
            source_generation: snapshot.source_generation,
            current_source_generation: record.source.generation,
        })
    }

    pub async fn refresh(
        &self,
        id: i64,
        timeout: Duration,
        cancel: watch::Receiver<bool>,
    ) -> Result<RefreshResult, SubscriptionError> {
        let host = FetchHost::new(timeout).map_err(SubscriptionError::Fetch)?;
        self.refresh_with_host(id, None, &host, cancel).await
    }

    /// generation защищает запланированную работу от смены источника до запуска.
    pub async fn refresh_with_host(
        &self,
        id: i64,
        generation: Option<i64>,
        host: &dyn SubscriptionFetch,
        cancel: watch::Receiver<bool>,
    ) -> Result<RefreshResult, SubscriptionError> {
        let source_update = self.source_updates.lock().await;
        let record = self
            .repository
            .load(id)
            .await?
            .ok_or(SubscriptionError::NotFound)?;
        if generation.is_some_and(|value| value != record.source.generation) {
            return Err(SubscriptionError::StaleUpdate);
        }
        let serial = CONTROLLERS
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| SubscriptionError::Exhausted)?;
        let (sender, mut superseded) = watch::channel(false);
        let request_sender = sender.clone();
        let ack = Arc::new(Semaphore::new(1));
        let mut requests = self.requests.lock().await;
        if let Some(previous) = requests.insert(
            id,
            Request {
                serial,
                cancellation: sender,
                ack: ack.clone(),
            },
        ) {
            previous.cancellation.send_replace(true);
        }
        drop(requests);
        drop(source_update);
        let result = async {
        let data: SourceData = serde_json::from_slice(&record.source.payload)
            .map_err(|_| SubscriptionError::InvalidSource)?;
        let spec = SubscriptionSource(data);
        let source = spec.validate()?;
        let revision = record
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.revision);
        let validators = record
            .snapshot
            .as_ref()
            .filter(|snapshot| snapshot.source_generation == record.source.generation)
            .and_then(|snapshot| snapshot.validators.as_ref())
            .map(|bytes| decode_validators(bytes, &spec))
            .transpose()?;
        let mut parse_cancel = cancel.clone();
        let response = tokio::select! {
            biased;
            _ = superseded.changed() => return Err(SubscriptionError::Fetch(FetchError::Cancelled)),
            result = host.fetch(
                &source,
                if spec.0.proxy {
                    FetchRoute::Proxy
                } else {
                    FetchRoute::Direct
                },
                validators.as_ref(),
                cancel,
            ) => result.map_err(SubscriptionError::Fetch)?,
        };
        let (body, validators) = match response {
            FetchResponse::NotModified { validators } => {
                let requests = self.requests.lock().await;
                check_request(&requests, id, serial)?;
                drop(requests);
                let permit = ack.clone().acquire_owned().await.map_err(|_| SubscriptionError::Worker)?;
                let snapshot = record.snapshot.ok_or(SubscriptionError::CorruptSnapshot)?;
                if snapshot.source_generation != record.source.generation {
                    return Err(SubscriptionError::CorruptSnapshot);
                }
                self.repository
                    .commit_snapshot_ack(
                        id,
                        record.source.generation,
                        revision,
                        SnapshotInput {
                            body: snapshot.body.to_vec(),
                            metadata: snapshot.metadata.to_vec(),
                            validators: Some(encode_validators(&validators)?),
                        },
                        crate::Cancellation::from_receiver(superseded.clone()).with_other(parse_cancel.clone()),
                        Some(permit),
                    )
                    .await.map_err(commit_error)?;
                request_sender.send_replace(true);
                return Ok(RefreshResult::NotModified);
            }
            FetchResponse::Modified { body, validators } => {
                (body.into_bytes(), encode_validators(&validators)?)
            }
        };
        // Сохраняем operation (включая JoinHandle blocking-import) до acknowledgement.
        // Отмена сигнала запрещает публикацию, но не отсоединяет уже начатый parser.
        let operation = stage_body(body, crate::Cancellation::from_receiver(superseded.clone()).with_other(parse_cancel.clone()), ack.clone());
        tokio::pin!(operation);
        let (controller, staged) = tokio::select! {
            biased;
            _ = watch_cancelled(&mut superseded) => {
                let _ = operation.await;
                return Err(SubscriptionError::Fetch(FetchError::Cancelled));
            },
            _ = watch_cancelled(&mut parse_cancel) => {
                request_sender.send_replace(true);
                let _ = operation.await;
                return Err(SubscriptionError::Fetch(FetchError::Cancelled));
            },
            result = &mut operation => result?,
        };
        check_request(&*self.requests.lock().await, id, serial)?;
        Ok(RefreshResult::Pending(Box::new(PendingRefresh {
            service: self.identity.clone(),
            request: serial,
            id,
            generation: record.source.generation,
            revision,
            validators,
            controller,
            staged,
        })))
        }.await;
        if !matches!(result, Ok(RefreshResult::Pending(_))) {
            let mut requests = self.requests.lock().await;
            if requests
                .get(&id)
                .is_some_and(|request| request.serial == serial)
            {
                requests.remove(&id);
            }
        }
        result
    }

    /// Освободить неутверждённый отчёт, не затрагивая более новый запрос.
    pub async fn discard_refresh(&self, pending: PendingRefresh) {
        let mut requests = self.requests.lock().await;
        if Arc::ptr_eq(&self.identity, &pending.service)
            && requests
                .get(&pending.id)
                .is_some_and(|r| r.serial == pending.request)
        {
            if let Some(request) = requests.remove(&pending.id) {
                request.cancellation.send_replace(true);
            }
        }
    }

    /// CAS внутри Repository — окончательная проверка свежести. Смена списка
    /// не перезапускает активную сессию и не публикует новый engine generation.
    pub async fn commit(&self, refresh: ApprovedRefresh) -> Result<i64, SubscriptionError> {
        let ApprovedRefresh {
            mut pending,
            approved,
        } = refresh;
        if !Arc::ptr_eq(&self.identity, &pending.service) {
            return Err(SubscriptionError::ForeignApproval);
        }
        let (cancel, ack) = {
            let requests = self.requests.lock().await;
            check_request(&requests, pending.id, pending.request)?;
            let request = requests
                .get(&pending.id)
                .ok_or(SubscriptionError::StaleUpdate)?;
            (
                crate::Cancellation::from_receiver(request.cancellation.subscribe()),
                request.ack.clone(),
            )
        };
        let permit = ack
            .acquire_owned()
            .await
            .map_err(|_| SubscriptionError::Worker)?;
        let report = approved.report();
        let metadata = serde_json::to_vec(&serde_json::json!({ "version": 1,
            "accepted": report.servers.len(), "rejected": report.issues.len(),
            "native_json": report.native_json.is_some() }))
        .map_err(|_| SubscriptionError::CorruptSnapshot)?;
        let result = self
            .repository
            .commit_snapshot_ack(
                pending.id,
                pending.generation,
                pending.revision,
                SnapshotInput {
                    body: report.original().expose().as_bytes().to_vec(),
                    metadata,
                    validators: Some(pending.validators),
                },
                cancel,
                Some(permit),
            )
            .await
            .map_err(commit_error);
        let mut requests = self.requests.lock().await;
        if requests
            .get(&pending.id)
            .is_some_and(|r| r.serial == pending.request)
        {
            requests.remove(&pending.id);
        }
        drop(requests);
        let revision = result?;
        pending
            .controller
            .commit(approved)
            .map_err(SubscriptionError::Import)?;
        Ok(revision)
    }
}

fn cancelled() -> SubscriptionError {
    SubscriptionError::Fetch(FetchError::Cancelled)
}
fn commit_error(error: crate::SnapshotCommitError) -> SubscriptionError {
    match error {
        crate::SnapshotCommitError::Cancelled => cancelled(),
        crate::SnapshotCommitError::Repository(error) => error.into(),
    }
}

fn check_request(
    requests: &HashMap<i64, Request>,
    id: i64,
    serial: u64,
) -> Result<(), SubscriptionError> {
    match requests.get(&id) {
        Some(request) if request.serial == serial && !*request.cancellation.borrow() => Ok(()),
        _ => Err(SubscriptionError::StaleUpdate),
    }
}

async fn watch_cancelled(receiver: &mut watch::Receiver<bool>) {
    loop {
        if *receiver.borrow_and_update() || receiver.changed().await.is_err() {
            return;
        }
    }
}

async fn stage_body(
    body: Vec<u8>,
    mut cancel: crate::Cancellation,
    ack: Arc<Semaphore>,
) -> Result<(Subscription, StagedUpdate), SubscriptionError> {
    // Permit остаётся внутри blocking-задачи даже при отмене ожидающего future.
    let ack = ack
        .acquire_owned()
        .await
        .map_err(|_| SubscriptionError::Worker)?;
    let permit = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(cancelled()),
        result = IMPORT_GATE.acquire() => result.map_err(|_| SubscriptionError::Worker)?,
    };
    tokio::task::spawn_blocking(move || {
        let _ack = ack;
        let _permit = permit;
        #[cfg(test)]
        scheduler_tests::hold_parser(&body);
        let next = CONTROLLERS
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| SubscriptionError::Exhausted)?;
        let mut controller =
            Subscription::new(NonZeroU64::new(next).ok_or(SubscriptionError::Exhausted)?);
        let ticket = controller.begin().map_err(SubscriptionError::Import)?;
        let staged = controller
            .stage_cancellable(ticket, body, || cancel.check().is_ok())
            .map_err(|error| match error {
                yonik_subscription::CancellableStageError::Cancelled => cancelled(),
                yonik_subscription::CancellableStageError::Stage(error) => {
                    SubscriptionError::Import(error)
                }
            })?;
        Ok((controller, staged))
    })
    .await
    .map_err(|_| SubscriptionError::Worker)?
}

#[cfg(test)]
#[path = "subscription_scheduler_tests.rs"]
mod scheduler_tests;

#[cfg(test)]
#[path = "subscription_cancellation_tests.rs"]
mod cancellation_tests;
