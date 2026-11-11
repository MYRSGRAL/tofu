//! Ограниченная текущая серия; история не сохраняется.
use super::{Actor, DesktopError, Result, TcpResultDto};
use serde::Serialize;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use yonik_linux::{Cancellation, NetworkSnapshot, NetworkStamp, NetworkWatch, TCP_SERIES_SAMPLES};

#[derive(Default, Serialize)]
pub(super) struct BatchState {
    pub busy: bool,
    pub cancelled: bool,
    pub total: usize,
    pub completed: usize,
    pub offset: usize,
    pub invalidated: bool,
    pub source_id: Option<i64>,
    pub revision: i64,
    pub rows: Vec<TcpResultDto>,
    #[serde(skip)]
    results: Vec<TcpResultDto>,
}
impl BatchState {
    pub fn page(&mut self, offset: usize) {
        self.offset = offset;
        self.rows = self.results.iter().skip(offset).take(32).cloned().collect();
    }
    pub fn accept(&mut self, row: TcpResultDto) {
        self.results.push(row);
        self.completed = self.results.len();
        self.page(self.offset);
    }
    pub fn invalidate(&mut self) {
        self.busy = false;
        self.invalidated = true;
        self.results.clear();
        self.rows.clear();
        self.completed = 0;
        self.total = 0;
        self.offset = 0;
    }
}
// Канал ограничен 16 элементами: Box добавит 10k аллокаций ради нескольких KiB.
#[allow(clippy::large_enum_variant)]
pub(super) enum BatchEvent {
    Prepared { total: usize },
    Result(TcpResultDto),
}
pub(super) struct BatchRunning {
    pub cancel: watch::Sender<bool>,
    pub receiver: mpsc::Receiver<BatchEvent>,
    pub task: tokio::task::JoinHandle<Result<()>>,
    pub stamp: Option<NetworkStamp>,
    pub generation: u64,
    pub tcp_generation: u64,
    pub output_closed: bool,
}
pub(super) enum BatchPoll {
    Event(Option<BatchEvent>),
    Finished(std::result::Result<Result<()>, tokio::task::JoinError>),
}
impl BatchRunning {
    pub async fn next(&mut self) -> BatchPoll {
        tokio::select! {
            biased;
            event = self.receiver.recv(), if !self.output_closed => BatchPoll::Event(event),
            result = &mut self.task => BatchPoll::Finished(result),
        }
    }
    pub fn stop(&mut self) {
        self.cancel.send_replace(true);
        self.receiver.close();
        while self.receiver.try_recv().is_ok() {}
        self.output_closed = true;
    }
}
async fn send_event(
    sender: &mpsc::Sender<BatchEvent>,
    event: BatchEvent,
    cancel: &mut Cancellation,
) -> Result<()> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(DesktopError("cancelled")),
        result = sender.send(event) => result.map_err(|_|DesktopError("cancelled")),
    }
}
impl Actor {
    pub fn start_batch(&mut self, generation: u64, tcp_generation: u64) -> Result<BatchRunning> {
        let source = self
            .state
            .source_id
            .ok_or(DesktopError("source_required"))?;
        let revision = self.state.revision;
        let current = self.network.as_ref().map(NetworkWatch::current);
        self.state.batch = BatchState {
            busy: true,
            source_id: Some(source),
            revision,
            ..BatchState::default()
        };
        let repo = self.routing_repository().clone();
        Ok(spawn_batch(
            repo,
            source,
            revision,
            current,
            generation,
            tcp_generation,
        ))
    }
}
fn spawn_batch(
    repo: yonik_linux::EncryptedSubscriptionRepository,
    source: i64,
    revision: i64,
    current: Option<NetworkSnapshot>,
    generation: u64,
    tcp_generation: u64,
) -> BatchRunning {
    let stamp = current.map(|n| n.stamp);
    let (cancel, mut cancellation) = Cancellation::new();
    let (sender, receiver) = mpsc::channel(16);
    let task = tokio::spawn(async move {
        let record = repo
            .load(source)
            .await
            .map_err(|_| DesktopError("storage"))?
            .ok_or(DesktopError("source_required"))?;
        let source_generation = record.source.generation;
        drop(record);
        let prepared = repo
            .batch_targets(source, source_generation, revision, cancellation.clone())
            .await;
        // batch_targets может вернуть отмену до завершения spawn_blocking.
        // Этот проход того же gate/Mutex подтверждает ACK CPU-работы;
        // producer и его JoinHandle всё это время принадлежат actor.
        let current_source = repo
            .load(source)
            .await
            .map_err(|_| DesktopError("storage"))?;
        if cancellation.check().is_err() {
            return Err(DesktopError("cancelled"));
        }
        if current_source.as_ref().map(|r| r.source.generation) != Some(source_generation)
            || current_source
                .as_ref()
                .and_then(|r| r.snapshot.as_ref())
                .map(|snapshot| snapshot.revision)
                != Some(revision)
        {
            return Err(DesktopError("refresh_required"));
        }
        drop(current_source);
        let prepared = prepared
            .map_err(|_| DesktopError("refresh_required"))?
            .ok_or(DesktopError("cancelled"))?;
        let mut targets = Vec::with_capacity(prepared.targets.len());
        let mut rows = std::collections::HashMap::with_capacity(prepared.targets.len());
        for projection in prepared.targets {
            cancellation
                .check()
                .map_err(|_| DesktopError("cancelled"))?;
            let profile = projection.profile;
            let id = profile.id as u64;
            targets.push(yonik_linux::TcpBatchTarget {
                id,
                target: projection.target,
            });
            rows.insert(
                id,
                TcpResultDto {
                    source_id: source,
                    profile_id: profile.id,
                    revision,
                    name: profile.name,
                    host: profile.host,
                    port: profile.port,
                    status: "cancelled",
                    dns_ms: None,
                    connect_ms: None,
                    samples_ms: Vec::new(),
                    samples_requested: TCP_SERIES_SAMPLES,
                    selected_address: None,
                    checked_at_ms: 0,
                    network_epoch: stamp.map(|s| s.to_string()),
                    network_observed: current.is_some_and(NetworkSnapshot::fully_observed),
                },
            );
        }
        send_event(
            &sender,
            BatchEvent::Prepared {
                total: targets.len(),
            },
            &mut cancellation,
        )
        .await?;
        let (results, mut received) = mpsc::channel(16);
        let mut core_cancel = cancellation.clone();
        let core = yonik_linux::check_tcp_batch(
            targets,
            8,
            Duration::from_secs(5),
            results,
            &mut core_cancel,
        );
        tokio::pin!(core);
        let mut finished = false;
        let mut pending = None;
        loop {
            if finished && rows.is_empty() && pending.is_none() {
                return Ok(());
            }
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(DesktopError("cancelled")),
                result = &mut core, if !finished => {
                    result.map_err(|_|DesktopError("tcp_failed"))?;
                    finished = true;
                }
                permit = sender.reserve(), if pending.is_some() => {
                    permit.map_err(|_|DesktopError("cancelled"))?.send(pending.take().expect("guard"));
                }
                result = received.recv(), if pending.is_none() => {
                    match result {
                        Some(result) => if let Some(mut row) = rows.remove(&result.id) {
                            match result.result { Ok(result) => row.finish(result), Err(_) => row.status = "tcp_failed" }
                            pending = Some(BatchEvent::Result(row));
                        },
                        None => return Ok(()),
                    }
                }
            }
        }
    });
    BatchRunning {
        cancel,
        receiver,
        task,
        stamp,
        generation,
        tcp_generation,
        output_closed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{BatchState, TcpResultDto};
    #[tokio::test]
    async fn full_output_channel_cancellation_keeps_producer_until_ack() {
        use std::os::unix::fs::DirBuilderExt;
        let path = std::env::temp_dir().join(format!("yonik-batch-output-{}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("свой каталог");
        let repo = yonik_linux::EncryptedSubscriptionRepository::open(
            path.join("db"),
            zeroize::Zeroizing::new([93; 32]),
        )
        .await
        .expect("БД");
        let source = repo
            .create_source(b"fixture".to_vec())
            .await
            .expect("источник");
        let endpoint = std::net::TcpListener::bind("127.0.0.1:0").expect("свой endpoint");
        let port = endpoint.local_addr().expect("адрес").port();
        drop(endpoint);
        use std::fmt::Write as _;
        let body = (0..128).fold(String::new(), |mut text, id| {
            writeln!(&mut text, "trojan://fixture-{id}@127.0.0.1:{port}#Цель{id}")
                .expect("Строка fixture");
            text
        });
        repo.commit_snapshot(
            source.id,
            1,
            0,
            yonik_linux::SnapshotInput {
                body: body.into_bytes(),
                metadata: vec![],
                validators: None,
            },
        )
        .await
        .expect("snapshot");
        repo.profiles(source.id, yonik_linux::ProfileQuery::default())
            .await
            .expect("каталог");
        let mut batch = super::spawn_batch(repo.clone(), source.id, 1, None, 0, 0);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while batch.receiver.len() < 16 {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("выходной канал заполнен");
        assert!(!batch.task.is_finished());
        batch.stop();
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), &mut batch.task)
            .await
            .expect("ACK без ожидания UI")
            .expect("producer");
        assert_eq!(result.expect_err("отменено").key(), "cancelled");
        assert_eq!(batch.receiver.len(), 0);
        assert!(batch.output_closed);
        drop(batch);
        repo.close().await.expect("закрытие БД");
        std::fs::remove_dir_all(path).expect("очистка своего каталога");
    }
    #[test]
    fn ten_thousand_results_have_bounded_public_pages() {
        let mut state = BatchState::default();
        for id in 0..10_000 {
            state.accept(TcpResultDto {
                source_id: 1,
                profile_id: id,
                revision: 2,
                name: format!("Цель{id}"),
                host: "127.0.0.1".into(),
                port: 1,
                status: "connect_failed",
                dns_ms: None,
                connect_ms: None,
                samples_ms: Vec::new(),
                samples_requested: 3,
                selected_address: None,
                checked_at_ms: 0,
                network_epoch: None,
                network_observed: false,
            });
            assert!(state.rows.len() <= 32);
        }
        state.page(9984);
        assert_eq!(state.rows.len(), 16);
        assert_eq!(state.completed, 10_000);
        assert_eq!(state.rows[0].profile_id, 9984);
        state.invalidate();
        assert!(state.results.is_empty());
    }
    #[test]
    fn invalidation_removes_public_results() {
        let mut state = BatchState {
            busy: true,
            ..BatchState::default()
        };
        state.invalidate();
        assert!(!state.busy);
        assert!(state.invalidated);
        assert!(state.rows.is_empty());
    }
}
