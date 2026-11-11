//! Сериализуемый host для системного IPC; профиль не сохраняется в снимках.
use tokio::time::Instant;
use yonik_engine::Engine;
use yonik_import::ServerIntent;
use yonik_linux::{
    Cancellation,
    system_client::{ClientError, Prepared, State, SystemClient},
};

/// Ревизии выдаёт GUI host при каждом изменении выбора или источника.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileRevision {
    pub source: u64,
    pub profile: u64,
    pub generation: u64,
}

pub struct SystemSession {
    client: SystemClient,
    revision: ProfileRevision,
    prepared: Option<Prepared>,
    state: State,
}
impl SystemSession {
    pub fn new(client: SystemClient, revision: ProfileRevision) -> Self {
        Self {
            client,
            revision,
            prepared: None,
            state: State::Unknown,
        }
    }
    pub fn owner_lost(&self) -> bool {
        self.client.owner_lost()
    }
    pub fn snapshot(&self) -> State {
        if self.owner_lost() {
            State::Unknown
        } else {
            self.state
        }
    }
    /// Вызывается владельцем при изменении источника, профиля или его поколения.
    pub fn revise(&mut self, revision: ProfileRevision) {
        self.revision = revision;
        self.prepared = None;
    }
    fn fence(&self, revision: ProfileRevision) -> Result<(), ClientError> {
        if revision == self.revision {
            Ok(())
        } else {
            Err(ClientError::Stale)
        }
    }
    #[allow(clippy::too_many_arguments)] // Явные границы IPC и fencing без копии профиля.
    pub async fn prepare(
        &mut self,
        revision: ProfileRevision,
        server: &ServerIntent,
        engine: Engine,
        kill_switch: bool,
        deadline: Instant,
        cancel: Cancellation,
    ) -> Result<(), ClientError> {
        self.fence(revision)?;
        self.prepared = None;
        // Публичная блокировка не означает активную сессию: admission проверяет helper.
        if !self
            .client
            .capabilities(deadline, cancel.clone())
            .await?
            .system_tun
        {
            return Err(ClientError::Unsupported);
        }
        self.prepared = Some(
            self.client
                .prepare_profile(server, engine, kill_switch, deadline, cancel)
                .await?,
        );
        Ok(())
    }
    pub async fn apply(
        &mut self,
        revision: ProfileRevision,
        deadline: Instant,
        cancel: Cancellation,
    ) -> Result<State, ClientError> {
        self.fence(revision)?;
        let prepared = self.prepared.take().ok_or(ClientError::InvalidHandle)?;
        // До отправки эффекта публикуем неизвестный исход: drop future безопасен.
        self.state = State::Unknown;
        let result = self.client.apply(prepared, deadline, cancel).await;
        self.record(result)
    }
    fn record(&mut self, result: Result<State, ClientError>) -> Result<State, ClientError> {
        self.state = match result {
            Ok(state) => state,
            Err(_) => State::Unknown,
        };
        result
    }
    pub async fn state(
        &mut self,
        deadline: Instant,
        cancel: Cancellation,
    ) -> Result<State, ClientError> {
        self.state = State::Unknown;
        let result = self.client.state(deadline, cancel).await;
        self.record(result)
    }
    pub async fn stop(
        &mut self,
        deadline: Instant,
        cancel: Cancellation,
    ) -> Result<State, ClientError> {
        self.prepared = None;
        self.state = State::Unknown;
        let result = self.client.stop(deadline, cancel).await;
        self.record(result)
    }
    pub async fn unlock(
        &mut self,
        deadline: Instant,
        cancel: Cancellation,
    ) -> Result<State, ClientError> {
        self.prepared = None;
        self.state = State::Unknown;
        let result = self.client.unlock(deadline, cancel).await;
        self.record(result)
    }
}
