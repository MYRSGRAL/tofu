//! Ограниченная текущая серия; история не сохраняется.
use super::{Actor, Control, DesktopError, Result, TcpResultDto, map_subscription};
use serde::Serialize;
use std::{sync::atomic::Ordering, time::Duration};
use tokio::sync::{mpsc, watch};
use yonik_linux::{
    Cancellation, HostError, NetworkSnapshot, NetworkStamp, NetworkWatch, ProfileQuery,
    TCP_SERIES_SAMPLES, TcpProbeTarget,
};

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
pub(super) struct BatchRunning {
    pub cancel: watch::Sender<bool>,
    pub receiver: mpsc::Receiver<yonik_linux::TcpBatchResult>,
    pub task: tokio::task::JoinHandle<std::result::Result<(), HostError>>,
    pub rows: std::collections::HashMap<u64, TcpResultDto>,
    pub stamp: Option<NetworkStamp>,
    pub generation: u64,
    pub tcp_generation: u64,
}
impl Actor {
    pub async fn start_batch(
        &mut self,
        generation: u64,
        tcp_generation: u64,
        control: &Control,
    ) -> Result<BatchRunning> {
        let source = self
            .state
            .source_id
            .ok_or(DesktopError("source_required"))?;
        let revision = self.state.revision;
        let current = self.network.as_ref().map(NetworkWatch::current);
        let stamp = current.map(|n| n.stamp);
        let mut targets = Vec::new();
        let mut rows = std::collections::HashMap::new();
        let mut offset = 0;
        loop {
            let page = self
                .service
                .profiles(
                    source,
                    ProfileQuery {
                        offset,
                        limit: 200,
                        include_unavailable: true,
                        ..ProfileQuery::default()
                    },
                )
                .await
                .map_err(map_subscription)?;
            if page.total > 10_000 {
                return Err(DesktopError("batch_limit"));
            }
            if page.snapshot_revision != revision {
                return Err(DesktopError("refresh_required"));
            }
            let count = page.profiles.len();
            for profile in page.profiles {
                if generation != control.generation.load(Ordering::SeqCst)
                    || tcp_generation != *control.tcp_priority.borrow()
                {
                    return Err(DesktopError("cancelled"));
                }
                let resolved = self
                    .service
                    .select_profile(profile.id, revision)
                    .await
                    .map_err(map_subscription)?;
                let id = profile.id as u64;
                targets.push(yonik_linux::TcpBatchTarget {
                    id,
                    target: TcpProbeTarget::for_server(&resolved.server),
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
            offset += count;
            if count == 0 || offset >= page.total {
                break;
            }
        }
        self.state.batch = BatchState {
            busy: true,
            total: targets.len(),
            source_id: Some(source),
            revision,
            ..BatchState::default()
        };
        let (cancel, mut cancellation) = Cancellation::new();
        let (sender, receiver) = mpsc::channel(32);
        let task = tokio::spawn(async move {
            yonik_linux::check_tcp_batch(
                targets,
                8,
                Duration::from_secs(5),
                sender,
                &mut cancellation,
            )
            .await
        });
        Ok(BatchRunning {
            cancel,
            receiver,
            task,
            rows,
            stamp,
            generation,
            tcp_generation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{BatchState, TcpResultDto};
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
