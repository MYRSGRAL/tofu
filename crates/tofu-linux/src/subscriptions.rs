//! Связь FetchHost, отчёта импорта и атомарного Repository. Сессия не меняется.

use crate::{
    EncryptedSubscriptionRepository, FetchError, FetchHost, FetchResponse, FetchRoute,
    FetchValidators, RepositoryError, SnapshotInput, Source,
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
use tofu_import::{ImportReport, import_bytes};
use tofu_subscription::{ApprovedUpdate, StagedUpdate, Subscription, Summary};
use tokio::sync::{Mutex, watch};

static CONTROLLERS: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionError {
    InvalidSource,
    CorruptSnapshot,
    NotFound,
    NoSnapshot,
    Exhausted,
    ForeignApproval,
    StaleUpdate,
    Fetch(FetchError),
    Import(tofu_subscription::Error),
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceData {
    version: u8,
    name: String,
    url: String,
    local_exception: bool,
    proxy: bool,
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
        });
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<Source, SubscriptionError> {
        if self.0.version != 1
            || self.0.name.trim().is_empty()
            || self.0.name.len() > 512
            || self.0.name.chars().any(char::is_control)
        {
            return Err(SubscriptionError::InvalidSource);
        }
        source_url(&self.0.url, self.0.local_exception)
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
fn decode_validators(bytes: &[u8], local: bool) -> Result<FetchValidators, SubscriptionError> {
    let value: ValidatorData =
        serde_json::from_slice(bytes).map_err(|_| SubscriptionError::CorruptSnapshot)?;
    FetchValidators::new(
        &source_url(&value.resource, local)?,
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
}
struct Request {
    serial: u64,
    cancellation: watch::Sender<bool>,
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
    pub async fn change_source(
        &self,
        id: i64,
        generation: i64,
        source: SubscriptionSource,
    ) -> Result<i64, SubscriptionError> {
        let mut requests = self.requests.lock().await;
        let generation = self
            .repository
            .change_source(id, generation, source.encode()?)
            .await?
            .generation;
        if let Some(request) = requests.get_mut(&id) {
            request.cancellation.send_replace(true);
        }
        Ok(generation)
    }

    pub async fn cancel(&self, id: i64) -> bool {
        let requests = self.requests.lock().await;
        if let Some(request) = requests.get(&id) {
            return !request.cancellation.send_replace(true);
        }
        false
    }

    /// Возвращает исходный утверждённый snapshot без сети. Capability-check
    /// конкретного ядра обязателен отдельно перед активацией.
    pub async fn offline(&self, id: i64) -> Result<OfflineSnapshot, SubscriptionError> {
        let record = self
            .repository
            .load(id)
            .await?
            .ok_or(SubscriptionError::NotFound)?;
        let snapshot = record.snapshot.ok_or(SubscriptionError::NoSnapshot)?;
        let report =
            import_bytes(snapshot.body.to_vec()).map_err(|_| SubscriptionError::CorruptSnapshot)?;
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
        let mut requests = self.requests.lock().await;
        let record = self
            .repository
            .load(id)
            .await?
            .ok_or(SubscriptionError::NotFound)?;
        let serial = requests.get(&id).map_or(Ok(1), |request| {
            request
                .serial
                .checked_add(1)
                .ok_or(SubscriptionError::Exhausted)
        })?;
        let (sender, mut superseded) = watch::channel(false);
        if let Some(previous) = requests.insert(
            id,
            Request {
                serial,
                cancellation: sender,
            },
        ) {
            previous.cancellation.send_replace(true);
        }
        drop(requests);
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
            .map(|bytes| decode_validators(bytes, spec.0.local_exception))
            .transpose()?;
        let host = FetchHost::new(timeout).map_err(SubscriptionError::Fetch)?;
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
                let snapshot = record.snapshot.ok_or(SubscriptionError::CorruptSnapshot)?;
                if snapshot.source_generation != record.source.generation {
                    return Err(SubscriptionError::CorruptSnapshot);
                }
                self.repository
                    .commit_snapshot(
                        id,
                        record.source.generation,
                        revision,
                        SnapshotInput {
                            body: snapshot.body.to_vec(),
                            metadata: snapshot.metadata.to_vec(),
                            validators: Some(encode_validators(&validators)?),
                        },
                    )
                    .await?;
                requests
                    .get(&id)
                    .ok_or(SubscriptionError::StaleUpdate)?
                    .cancellation
                    .send_replace(true);
                return Ok(RefreshResult::NotModified);
            }
            FetchResponse::Modified { body, validators } => {
                (body.into_bytes(), encode_validators(&validators)?)
            }
        };
        let next = CONTROLLERS
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| SubscriptionError::Exhausted)?;
        let mut controller =
            Subscription::new(NonZeroU64::new(next).ok_or(SubscriptionError::Exhausted)?);
        let ticket = controller.begin().map_err(SubscriptionError::Import)?;
        let staged = controller
            .stage(ticket, body)
            .map_err(SubscriptionError::Import)?;
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
        let requests = self.requests.lock().await;
        check_request(&requests, pending.id, pending.request)?;
        let report = approved.report();
        let metadata = serde_json::to_vec(&serde_json::json!({ "version": 1,
            "accepted": report.servers.len(), "rejected": report.issues.len(),
            "native_json": report.native_json.is_some() }))
        .map_err(|_| SubscriptionError::CorruptSnapshot)?;
        let revision = self
            .repository
            .commit_snapshot(
                pending.id,
                pending.generation,
                pending.revision,
                SnapshotInput {
                    body: report.original().expose().as_bytes().to_vec(),
                    metadata,
                    validators: Some(pending.validators),
                },
            )
            .await?;
        requests
            .get(&pending.id)
            .ok_or(SubscriptionError::StaleUpdate)?
            .cancellation
            .send_replace(true);
        pending
            .controller
            .commit(approved)
            .map_err(SubscriptionError::Import)?;
        Ok(revision)
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
