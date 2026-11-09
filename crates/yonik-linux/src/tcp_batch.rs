//! Ограниченный пакет отменяемых TCP-серий.
use std::{
    collections::{HashSet, VecDeque},
    future::Future,
    time::Duration,
};

use futures_util::{StreamExt, stream::FuturesUnordered};
use tokio::sync::mpsc;

use crate::{Cancellation, HostError, TcpProbeSeriesResult, TcpProbeTarget, check_tcp_series};

#[derive(Debug)]
/// ID принадлежит вызывающему коду; нулевой ID допустим.
pub struct TcpBatchTarget {
    pub id: u64,
    pub target: TcpProbeTarget,
}

#[derive(Debug)]
/// Результат одной серии; ошибка серии не прерывает остальные цели.
pub struct TcpBatchResult {
    pub id: u64,
    pub result: Result<TcpProbeSeriesResult, HostError>,
}

/// Выполняет до 10000 серий с параллелизмом 1..32 и timeout каждой серии до 60 с.
///
/// Канал результатов должен иметь ёмкость не больше 32. Доставка соблюдает
/// backpressure; отмена, закрытие приёмника и Drop освобождают активные серии.
/// Результаты приходят в порядке завершения, по одному для каждого ID при успехе.
pub async fn check_tcp_batch(
    targets: Vec<TcpBatchTarget>,
    parallelism: usize,
    timeout: Duration,
    results: mpsc::Sender<TcpBatchResult>,
    cancel: &mut Cancellation,
) -> Result<(), HostError> {
    batch_with(
        targets,
        parallelism,
        timeout,
        results,
        cancel,
        |target, timeout, mut cancel| async move {
            check_tcp_series(target, timeout, &mut cancel).await
        },
    )
    .await
}

async fn batch_with<F, J>(
    targets: Vec<TcpBatchTarget>,
    parallelism: usize,
    timeout: Duration,
    results: mpsc::Sender<TcpBatchResult>,
    cancel: &mut Cancellation,
    mut job: J,
) -> Result<(), HostError>
where
    J: FnMut(TcpProbeTarget, Duration, Cancellation) -> F,
    F: Future<Output = Result<TcpProbeSeriesResult, HostError>>,
{
    if targets.len() > 10_000
        || !(1..=32).contains(&parallelism)
        || timeout.is_zero()
        || timeout > Duration::from_secs(60)
        || results.max_capacity() > 32
    {
        return Err(HostError::ProbeTarget);
    }
    let mut ids = HashSet::with_capacity(targets.len());
    if targets.iter().any(|target| !ids.insert(target.id)) {
        return Err(HostError::ProbeTarget);
    }
    drop(ids);
    let mut waiting = targets.into_iter();
    let mut active = FuturesUnordered::new();
    let mut completed = VecDeque::with_capacity(parallelism);
    loop {
        if active.is_empty() && completed.is_empty() && waiting.len() == 0 {
            return Ok(());
        }
        cancel.check()?;
        if results.is_closed() {
            return Err(HostError::Cancelled);
        }
        // Завершённые серии занимают слот до доставки: backpressure ограничивает
        // память, но не останавливает polling уже начатых соединений и их timers.
        while active.len() + completed.len() < parallelism {
            let Some(target) = waiting.next() else {
                break;
            };
            let future = job(target.target, timeout, cancel.clone());
            active.push(async move {
                TcpBatchResult {
                    id: target.id,
                    result: future.await,
                }
            });
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(HostError::Cancelled),
            _ = results.closed() => return Err(HostError::Cancelled),
            permit = results.reserve(), if !completed.is_empty() => {
                let permit = permit.map_err(|_| HostError::Cancelled)?;
                if let Some(result) = completed.pop_front() {
                    permit.send(result);
                }
            }
            result = active.next(), if !active.is_empty() => {
                if let Some(result) = result {
                    completed.push_back(result);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Cancellation, Duration, HostError, TcpBatchTarget, TcpProbeTarget, batch_with,
        check_tcp_batch,
    };
    use crate::{TcpProbeSeriesResult, TcpProbeStatus};
    use futures_util::poll;
    use std::{cell::Cell, rc::Rc, sync::Arc, task::Poll};
    use tokio::sync::mpsc;

    fn target(id: u64) -> TcpBatchTarget {
        let report = yonik_import::import_bytes(b"trojan://secret@127.0.0.1:443".to_vec()).unwrap();
        TcpBatchTarget {
            id,
            target: TcpProbeTarget::for_server(&report.servers[0]),
        }
    }

    #[tokio::test]
    async fn invalid_batch_never_starts_jobs() {
        for (ids, parallelism, timeout, capacity) in [
            (vec![0], 0, Duration::from_secs(1), 1),
            (vec![0], 33, Duration::from_secs(1), 1),
            (vec![0], 1, Duration::ZERO, 1),
            (
                vec![0],
                1,
                Duration::from_secs(60) + Duration::from_nanos(1),
                1,
            ),
            (vec![0, 0], 1, Duration::from_secs(1), 1),
            ((0..10_001).collect(), 1, Duration::from_secs(1), 1),
            (vec![0], 1, Duration::from_secs(1), 33),
        ] {
            let (results, _receiver) = mpsc::channel(capacity);
            let (_source, mut cancel) = Cancellation::new();
            assert_eq!(
                batch_with(
                    ids.into_iter().map(target).collect(),
                    parallelism,
                    timeout,
                    results,
                    &mut cancel,
                    |_, _, _| async {
                        panic!("Некорректный пакет начал проверку")
                    }
                )
                .await,
                Err(HostError::ProbeTarget)
            );
        }
    }

    fn success() -> TcpProbeSeriesResult {
        TcpProbeSeriesResult {
            status: TcpProbeStatus::Reachable,
            dns_elapsed: None,
            selected_address: None,
            samples: vec![Duration::from_millis(1); 3],
            median: Some(Duration::from_millis(1)),
        }
    }

    #[tokio::test]
    async fn ids_zero_and_max_are_delivered_once_and_job_error_does_not_abort() {
        let (results, mut receiver) = mpsc::channel(1);
        let (_source, mut cancel) = Cancellation::new();
        let mut index = 0;
        let batch = batch_with(
            vec![target(0), target(u64::MAX), target(12)],
            2,
            Duration::from_secs(1),
            results,
            &mut cancel,
            |_, _, _| {
                index += 1;
                let result = if index == 2 {
                    Err(HostError::Timeout)
                } else {
                    Ok(success())
                };
                async move { result }
            },
        );
        let consumer = async {
            let mut received = Vec::new();
            while let Some(result) = receiver.recv().await {
                received.push((result.id, result.result));
            }
            received.sort_by_key(|result| result.0);
            assert_eq!(received.len(), 3);
            assert_eq!(received[0].0, 0);
            assert_eq!(received[1].0, 12);
            assert_eq!(received[2], (u64::MAX, Err(HostError::Timeout)));
        };
        let (batch, ()) = tokio::join!(batch, consumer);
        assert_eq!(batch, Ok(()));
    }

    #[derive(Default)]
    struct Counts {
        live: Cell<usize>,
        peak: Cell<usize>,
        started: Cell<usize>,
    }
    struct Guard(Rc<Counts>);
    impl Guard {
        fn new(counts: &Rc<Counts>) -> Self {
            counts.started.set(counts.started.get() + 1);
            counts.live.set(counts.live.get() + 1);
            counts.peak.set(counts.peak.get().max(counts.live.get()));
            Self(Rc::clone(counts))
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.live.set(self.0.live.get() - 1);
        }
    }

    #[tokio::test]
    async fn pending_jobs_obey_parallelism_and_batch_drop_destroys_them() {
        let counts = Rc::new(Counts::default());
        let (results, _receiver) = mpsc::channel(1);
        let (_source, mut cancel) = Cancellation::new();
        let mut batch = Box::pin(batch_with(
            (0..100).map(target).collect(),
            4,
            Duration::from_secs(1),
            results,
            &mut cancel,
            |_, _, _| {
                let guard = Guard::new(&counts);
                async move {
                    let _guard = guard;
                    std::future::pending().await
                }
            },
        ));
        assert!(poll!(&mut batch).is_pending());
        assert_eq!(counts.started.get(), 4);
        assert_eq!(counts.live.get(), 4);
        drop(batch);
        assert_eq!(counts.live.get(), 0);
        assert_eq!(counts.peak.get(), 4);
    }

    #[tokio::test]
    async fn full_output_stops_scheduling_and_cancellation_drops_jobs() {
        let counts = Rc::new(Counts::default());
        let (results, _receiver) = mpsc::channel(1);
        let (source, mut cancel) = Cancellation::new();
        let mut batch = Box::pin(batch_with(
            (0..100).map(target).collect(),
            4,
            Duration::from_secs(1),
            results,
            &mut cancel,
            |_, _, _| {
                let guard = Guard::new(&counts);
                async move {
                    drop(guard);
                    Ok(success())
                }
            },
        ));
        assert!(poll!(&mut batch).is_pending());
        assert_eq!(counts.started.get(), 5);
        assert!(counts.peak.get() <= 4);
        source.send_replace(true);
        assert_eq!(batch.await, Err(HostError::Cancelled));
        assert_eq!(counts.live.get(), 0);
    }

    #[tokio::test]
    async fn full_output_keeps_polling_active_jobs_before_consumer_drains() {
        let (results, receiver) = mpsc::channel(1);
        let (_source, mut cancel) = Cancellation::new();
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let finished = Rc::new(Cell::new(0));
        let mut started = 0;
        let mut batch = Box::pin(batch_with(
            (0..5).map(target).collect(),
            4,
            Duration::from_secs(1),
            results,
            &mut cancel,
            |_, _, _| {
                started += 1;
                let waits = started > 2;
                let gate = Arc::clone(&gate);
                let finished = Rc::clone(&finished);
                async move {
                    if waits {
                        let _permit = gate.acquire().await.unwrap();
                        finished.set(finished.get() + 1);
                    }
                    Ok(success())
                }
            },
        ));
        assert!(poll!(&mut batch).is_pending());
        assert_eq!(receiver.len(), 1);
        gate.add_permits(3);
        assert!(poll!(&mut batch).is_pending());
        assert_eq!(
            finished.get(),
            3,
            "Backpressure не должен замораживать начатые серии"
        );
        assert_eq!(receiver.len(), 1);
    }

    #[tokio::test]
    async fn cancellation_and_receiver_close_stop_pending_io() {
        for close_receiver in [false, true] {
            let counts = Rc::new(Counts::default());
            let (results, receiver) = mpsc::channel(1);
            let (source, mut cancel) = Cancellation::new();
            let mut batch = Box::pin(batch_with(
                (0..10).map(target).collect(),
                3,
                Duration::from_secs(1),
                results,
                &mut cancel,
                |_, _, _| {
                    let guard = Guard::new(&counts);
                    async move {
                        let _guard = guard;
                        std::future::pending().await
                    }
                },
            ));
            assert!(poll!(&mut batch).is_pending());
            if close_receiver {
                drop(receiver);
            } else {
                source.send_replace(true);
            }
            assert_eq!(poll!(&mut batch), Poll::Ready(Err(HostError::Cancelled)));
            assert_eq!(counts.live.get(), 0);
            assert_eq!(counts.started.get(), 3);
        }
    }

    #[tokio::test]
    async fn cancellation_has_priority_over_ready_jobs_and_full_output() {
        let (results, mut receiver) = mpsc::channel(1);
        let (source, mut cancel) = Cancellation::new();
        source.send_replace(true);
        assert_eq!(
            batch_with(
                vec![target(1)],
                1,
                Duration::from_secs(1),
                results,
                &mut cancel,
                |_, _, _| async { Ok(success()) }
            )
            .await,
            Err(HostError::Cancelled)
        );
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn empty_valid_batch_finishes_without_jobs() {
        let (results, _receiver) = mpsc::channel(32);
        let (_source, mut cancel) = Cancellation::new();
        assert_eq!(
            batch_with(
                Vec::new(),
                32,
                Duration::from_secs(60),
                results,
                &mut cancel,
                |_, _, _| async { panic!("Пустой пакет начал проверку") }
            )
            .await,
            Ok(())
        );
    }

    async fn loopback_batch(count: u64) {
        use tokio::{io::AsyncReadExt, net::TcpListener};
        let mut listeners = Vec::new();
        for _ in 0..16 {
            listeners.push(TcpListener::bind("127.0.0.1:0").await.unwrap());
        }
        let targets = (0..count)
            .map(|id| {
                let address = listeners[id as usize % 16].local_addr().unwrap();
                let report =
                    yonik_import::import_bytes(format!("trojan://secret@{address}").into_bytes())
                        .unwrap();
                TcpBatchTarget {
                    id,
                    target: TcpProbeTarget::for_server(&report.servers[0]),
                }
            })
            .collect();
        let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let servers = futures_util::future::join_all(listeners.into_iter().enumerate().map(
            |(index, listener)| {
                let accepted = Arc::clone(&accepted);
                async move {
                    let series = (0..count).filter(|id| *id as usize % 16 == index).count();
                    for _ in 0..series * 3 {
                        let (mut socket, _) = listener.accept().await.unwrap();
                        assert_eq!(socket.read(&mut [0; 1]).await.unwrap(), 0);
                        accepted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            },
        ));
        let (results, mut receiver) = mpsc::channel(32);
        let (_source, mut cancel) = Cancellation::new();
        let batch = check_tcp_batch(targets, 32, Duration::from_secs(5), results, &mut cancel);
        let consumer = async {
            let mut seen = std::collections::HashSet::new();
            while let Some(result) = receiver.recv().await {
                assert!(seen.insert(result.id));
                let result = result.result.unwrap();
                assert_eq!(result.status, TcpProbeStatus::Reachable);
                assert_eq!(result.samples.len(), 3);
                assert!(!format!("{result:?}").contains("127.0.0.1"));
            }
            assert_eq!(seen.len(), count as usize);
        };
        let (result, (), _) = tokio::time::timeout(Duration::from_secs(120), async {
            tokio::join!(batch, consumer, servers)
        })
        .await
        .unwrap();
        assert_eq!(result, Ok(()));
        assert_eq!(
            accepted.load(std::sync::atomic::Ordering::Relaxed),
            count as usize * 3
        );
    }

    #[tokio::test]
    async fn real_loopback_series_deliver_all_ids_and_eof_without_payload() {
        loopback_batch(64).await;
    }

    #[tokio::test]
    #[ignore = "Явная нагрузочная проверка: 10000 серий, 30000 loopback-соединений"]
    async fn ten_thousand_loopback_series() {
        loopback_batch(10_000).await;
    }
}
