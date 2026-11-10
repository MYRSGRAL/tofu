use super::{
    RecoveryFailure, RecoveryHost, ServiceError, endpoint, keep_after_stop, parse_worker_uid,
    recover_ordered, same_link_contract,
};

fn actor_fixture() -> super::super::SystemProtectionHost {
    let monitor = crate::NetworkMonitor::start().unwrap();
    super::super::SystemProtectionHost {
        backend: std::sync::Arc::new(std::sync::Mutex::new(super::Backend {
            path: "/run/yonik/unused-actor-test".into(),
            journal: None,
            watch: monitor.watch(),
            seed: 1,
            epoch: 1,
            bindings: None,
            supervisor: None,
            worker_uncertain: false,
            active: None,
            registry: None,
            exited: std::sync::Arc::new(tokio::sync::Notify::new()),
            isolated: true,
        })),
        _monitor: monitor,
        runtime: tokio::runtime::Handle::current(),
        _safety: None,
    }
}

struct TestEvents(tokio::sync::watch::Receiver<u64>);
impl super::SafetyEvents for TestEvents {
    async fn next_event(&mut self) -> bool {
        self.0.changed().await.is_ok()
    }
}
struct CleanupOnEvent {
    active: bool,
    calls: usize,
    cleanups: usize,
    reports: tokio::sync::watch::Sender<(usize, usize)>,
}
fn cleanup_on_event(state: &mut CleanupOnEvent, _: &tokio::runtime::Handle) {
    state.calls += 1;
    if state.active {
        state.active = false;
        state.cleanups += 1;
    }
    state.reports.send_replace((state.calls, state.cleanups));
}

#[tokio::test]
async fn network_event_cleans_session_without_rpc_and_waits_for_inflight_operation() {
    let (events, receiver) = tokio::sync::watch::channel(0);
    let (reports, mut observed) = tokio::sync::watch::channel((0, 0));
    let backend = std::sync::Arc::new(std::sync::Mutex::new(CleanupOnEvent {
        active: false,
        calls: 0,
        cleanups: 0,
        reports,
    }));
    let mut safety = super::spawn_safety(
        TestEvents(receiver),
        std::sync::Arc::downgrade(&backend),
        tokio::runtime::Handle::current(),
        cleanup_on_event,
    );
    observed.changed().await.unwrap(); // startup sweep
    let (locked, lock_ready) = tokio::sync::oneshot::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let shared = std::sync::Arc::clone(&backend);
    let inflight = tokio::task::spawn_blocking(move || {
        let mut inflight = shared.lock().unwrap();
        inflight.active = true;
        locked.send(()).unwrap();
        wait_release.recv().unwrap();
        assert_eq!(inflight.cleanups, 0);
    });
    lock_ready.await.unwrap();
    events.send_replace(1);
    // Событие приходит внутри Apply. Actor обязан дождаться mutex, а не
    // проигнорировать его через try_lock и ждать нового RPC State.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), observed.changed())
            .await
            .is_err()
    );
    release.send(()).unwrap();
    inflight.await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), observed.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*observed.borrow(), (2, 1));
    drop(events);
    tokio::time::timeout(std::time::Duration::from_secs(1), &mut safety.0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*observed.borrow(), (3, 1)); // закрытие источника тоже проверено
}

#[tokio::test]
async fn safety_startup_sweep_recovers_prior_session_and_does_not_own_backend_cycle() {
    let (events, receiver) = tokio::sync::watch::channel(0);
    let (reports, mut observed) = tokio::sync::watch::channel((0, 0));
    let backend = std::sync::Arc::new(std::sync::Mutex::new(CleanupOnEvent {
        active: true,
        calls: 0,
        cleanups: 0,
        reports,
    }));
    let weak = std::sync::Arc::downgrade(&backend);
    let mut safety = super::spawn_safety(
        TestEvents(receiver),
        weak.clone(),
        tokio::runtime::Handle::current(),
        cleanup_on_event,
    );
    observed.changed().await.unwrap();
    assert_eq!(*observed.borrow(), (1, 1));
    drop(backend);
    drop(events);
    tokio::time::timeout(std::time::Duration::from_secs(1), &mut safety.0)
        .await
        .unwrap()
        .unwrap();
    assert!(weak.upgrade().is_none());
}

fn process_status() -> Vec<u8> {
    b"Name:\ttrusted-core\nUid:\t969\t969\t969\t969\nNoNewPrivs:\t1\nCapInh:\t0000000000001000\nCapPrm:\t0000000000001000\nCapEff:\t0000000000001000\nCapBnd:\t0000000000001000\nCapAmb:\t0000000000001000\n".to_vec()
}
#[test]
fn worker_status_pins_all_uids_no_new_privileges_and_exact_capability_sets() {
    let status = process_status();
    assert!(super::verify_worker_status(&status, 969).is_ok());
    for (from, to) in [
        ("969\t969\t969\t969", "969\t1000\t969\t969"),
        ("NoNewPrivs:\t1", "NoNewPrivs:\t0"),
        ("CapEff:\t0000000000001000", "CapEff:\t0000000000003000"),
        ("CapAmb:\t0000000000001000", "CapAmb:\t0000000000000000"),
        ("CapPrm:\t0000000000001000", "CapPrm:\tinvalid"),
        ("CapInh:\t0000000000001000", "CapInh:\t0000000000000000"),
        ("Uid:\t969\t969\t969\t969", "Uid:\t969\t969\t969"),
        (
            "CapBnd:\t0000000000001000",
            "CapBnd:\t0000000000001000\nCapBnd:\t0000000000001000",
        ),
    ] {
        let changed = std::str::from_utf8(&status).unwrap().replace(from, to);
        assert_eq!(
            super::verify_worker_status(changed.as_bytes(), 969),
            Err(ServiceError::Unavailable)
        );
    }
    assert_eq!(
        super::verify_worker_status(&status, 1000),
        Err(ServiceError::Unavailable)
    );
    let mut oversized = status.clone();
    oversized.resize(16 * 1024 + 1, b' ');
    assert_eq!(
        super::verify_worker_status(&oversized, 969),
        Err(ServiceError::Unavailable)
    );
}

#[tokio::test]
async fn background_tasks_keep_running_after_blocking_job_returns() {
    let host = actor_fixture();
    let (_keep, cancel) = crate::Cancellation::new();
    let (completed, receiver) = tokio::sync::oneshot::channel();
    host.job(super::super::Operation::RuntimeProbe(completed), cancel)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), receiver)
        .await
        .unwrap()
        .unwrap();
    host._monitor.shutdown().await;
}

#[tokio::test]
async fn dropping_caller_does_not_release_backend_before_effect_finishes() {
    let host = std::sync::Arc::new(actor_fixture());
    let (_keep, cancel) = crate::Cancellation::new();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (resume, resume_wait) = tokio::sync::oneshot::channel();
    let (finished, completed) = tokio::sync::oneshot::channel();
    let shared = std::sync::Arc::clone(&host);
    let caller = tokio::spawn(async move {
        shared
            .job(
                super::super::Operation::RetentionProbe {
                    entered,
                    resume: resume_wait,
                    finished,
                },
                cancel,
            )
            .await
    });
    started.await.unwrap();
    caller.abort();
    assert!(matches!(
        host.backend.try_lock(),
        Err(std::sync::TryLockError::WouldBlock)
    ));
    resume.send(()).unwrap();
    completed.await.unwrap();
    // blocking job ещё может завершать Reply, но обе стороны больше не держат
    // зависимых эффектов; runtime остаётся живым до выхода из теста.
}

#[test]
fn worker_uid_is_dedicated_and_unambiguous() {
    assert_eq!(
        parse_worker_uid(
            b"root:x:0:0::/:/bin/sh\nyonik-worker:x:969:969::/nonexistent:/usr/bin/nologin\n"
        ),
        Ok(969)
    );
    for bytes in [
        b"yonik-worker:x:0:0::/:/bin/sh\n".as_slice(),
        b"yonik-worker:x:969:969::/:/bin/sh\nyonik-worker:x:970:970::/:/bin/sh\n",
        b"yonik-worker:x:969\n",
        b"root:x:0:0::/:/bin/sh\n",
    ] {
        assert_eq!(parse_worker_uid(bytes), Err(ServiceError::Unavailable));
    }
}

#[test]
fn hostname_and_unbound_transport_are_denied_before_effects() {
    for value in [
        "vless://11111111-1111-1111-1111-111111111111@192.0.2.5:443",
        "vless://11111111-1111-1111-1111-111111111111@example.org:443",
        "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443",
    ] {
        let mut report = yonik_import::import_bytes(value.as_bytes().to_vec()).unwrap();
        let server = report.servers.remove(0);
        if value.contains("192.0.2.5") {
            assert_eq!(
                endpoint(&server),
                Ok(("192.0.2.5".parse().unwrap(), crate::TransportProtocol::Tcp))
            );
        } else {
            assert_eq!(endpoint(&server), Err(ServiceError::Unsupported));
        }
    }
}

#[tokio::test]
async fn actual_backend_rejects_hostname_without_opening_journal_registry_or_worker() {
    let monitor = crate::NetworkMonitor::start().unwrap();
    let mut backend = super::Backend {
        path: "/run/yonik/unused-denied-session-test".into(),
        journal: None,
        watch: monitor.watch(),
        seed: 1,
        epoch: 1,
        bindings: None,
        supervisor: None,
        worker_uncertain: false,
        active: None,
        registry: None,
        exited: std::sync::Arc::new(tokio::sync::Notify::new()),
        isolated: true,
    };
    let report = yonik_import::import_bytes(
        b"vless://11111111-1111-1111-1111-111111111111@example.org:443".to_vec(),
    )
    .unwrap();
    let request = crate::helper::service::SystemProfileRequest {
        server: yonik_import::ServerIntent::from_server(&report.servers[0]).unwrap(),
        engine: yonik_engine::Engine::Xray,
        kill_switch: false,
    };
    let (_keep, mut cancel) = crate::Cancellation::new();
    assert_eq!(
        backend
            .prepare_profile(1000, request, &mut cancel)
            .await
            .err(),
        Some(ServiceError::Unsupported)
    );
    assert!(backend.journal.is_none());
    assert!(backend.supervisor.is_none());
    assert!(backend.registry.is_none());
    assert!(!backend.worker_uncertain);
    monitor.shutdown().await;
}

#[test]
fn supported_tcp_underlays_share_endpoint_but_unbound_udp_is_rejected() {
    for transport in ["tcp", "ws", "grpc", "xhttp"] {
        let value =
            format!("vless://11111111-1111-1111-1111-111111111111@192.0.2.5:443?type={transport}");
        let report = yonik_import::import_bytes(value.into_bytes()).unwrap();
        assert!(endpoint(&report.servers[0]).is_ok());
    }
    let report = yonik_import::import_bytes(b"socks5://192.0.2.5:1080".to_vec()).unwrap();
    assert_eq!(endpoint(&report.servers[0]), Err(ServiceError::Unsupported));
}

#[test]
fn link_binding_keeps_devconf_token_and_gso_exact() {
    let mut expected = super::super::tests::link(16, Some(b"tun"), false);
    expected.unknown.push(crate::NetworkAttribute {
        kind: 23,
        value: vec![0; 200],
    });
    let mut actual = expected.clone();
    actual.flags |= 0x10000 | 0x40;
    actual.operational_state = Some(0);
    assert!(same_link_contract(&expected, &actual));
    for kind in [26, 40, 41, 63, 64] {
        let mut changed = actual.clone();
        changed.unknown.push(crate::NetworkAttribute {
            kind,
            value: vec![1, 2, 3, 4],
        });
        assert!(!same_link_contract(&expected, &changed));
    }
    actual.unknown[0].value.fill(99);
    assert!(same_link_contract(&expected, &actual));
}

#[test]
fn link_binding_ignores_only_dynamic_ipv6_fields_and_pins_nested_conf_and_token() {
    fn attr(kind: u16, value: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&u16::try_from(value.len() + 4).unwrap().to_ne_bytes());
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(value);
        bytes.resize((bytes.len() + 3) & !3, 0);
        bytes
    }
    fn af(conf: u8, token: u8, stats: u8) -> Vec<u8> {
        let mut fields = attr(2, &[conf; 4]);
        fields.extend(attr(7, &[token; 16]));
        fields.extend(attr(3, &[1u64.to_ne_bytes(), [stats; 8]].concat()));
        attr(10, &fields)
    }
    let mut expected = super::super::tests::link(16, Some(b"tun"), false);
    expected.unknown.push(crate::NetworkAttribute {
        kind: 26,
        value: af(1, 2, 3),
    });
    let mut actual = expected.clone();
    actual.unknown[0].value = af(1, 2, 4);
    assert!(same_link_contract(&expected, &actual));
    actual.unknown[0].value = af(9, 2, 4);
    assert!(!same_link_contract(&expected, &actual));
    actual.unknown[0].value = af(1, 9, 4);
    assert!(!same_link_contract(&expected, &actual));
}

#[derive(Default)]
struct FakeRecovery {
    events: Vec<&'static str>,
    unknown_worker: bool,
    cancelled_network: bool,
    failed_unlock: bool,
}
impl RecoveryHost for FakeRecovery {
    async fn stop_worker(&mut self) -> Result<(), ServiceError> {
        self.events
            .push("activation-off/services-off/cgroups-empty");
        if self.unknown_worker {
            Err(ServiceError::Unavailable)
        } else {
            Ok(())
        }
    }
    async fn restrict(&mut self) -> Result<(), ServiceError> {
        self.events.push("public-block-without-exceptions");
        Ok(())
    }
    async fn cleanup_network(&mut self) -> Result<(), ServiceError> {
        self.events.push("rules/routes/addresses/tun");
        if self.cancelled_network {
            Err(ServiceError::Cancelled)
        } else {
            Ok(())
        }
    }
    async fn unlock(&mut self) -> Result<(), ServiceError> {
        self.events.push("unlock-public-block");
        if self.failed_unlock {
            Err(ServiceError::Unavailable)
        } else {
            Ok(())
        }
    }
    async fn clear_intent(&mut self) -> Result<(), ServiceError> {
        self.events.push("clear-durable-disposition");
        Ok(())
    }
}

#[tokio::test]
async fn kill_switch_false_unlocks_only_after_worker_and_network_cleanup() {
    let mut host = FakeRecovery::default();
    assert!(recover_ordered(&mut host, true, false).await.is_ok());
    assert_eq!(
        host.events,
        [
            "activation-off/services-off/cgroups-empty",
            "public-block-without-exceptions",
            "rules/routes/addresses/tun",
            "unlock-public-block",
            "clear-durable-disposition"
        ]
    );
    // Тот же recovery protocol применяется при повторном Stop; stale ticket
    // завершённой invocation не участвует в авторизации cleanup.
    host.events.clear();
    assert!(recover_ordered(&mut host, true, false).await.is_ok());
    assert_eq!(host.events.last(), Some(&"clear-durable-disposition"));
}

#[tokio::test]
async fn kill_switch_true_keeps_restrictive_policy_and_unknown_worker_blocks_cleanup() {
    let mut host = FakeRecovery::default();
    assert!(recover_ordered(&mut host, true, true).await.is_ok());
    assert!(!host.events.contains(&"unlock-public-block"));
    assert_eq!(host.events.last(), Some(&"clear-durable-disposition"));
    host.events.clear();
    host.unknown_worker = true;
    assert!(matches!(
        recover_ordered(&mut host, true, false).await,
        Err(RecoveryFailure::WorkerUnknown)
    ));
    assert_eq!(host.events, ["activation-off/services-off/cgroups-empty"]);
}

#[tokio::test]
async fn partial_cleanup_cancellation_and_unlock_failure_retain_disposition() {
    let mut host = FakeRecovery {
        cancelled_network: true,
        ..FakeRecovery::default()
    };
    assert!(matches!(
        recover_ordered(&mut host, true, false).await,
        Err(RecoveryFailure::Network(ServiceError::Cancelled))
    ));
    assert!(!host.events.contains(&"unlock-public-block"));
    assert!(!host.events.contains(&"clear-durable-disposition"));
    host.events.clear();
    host.cancelled_network = false;
    host.failed_unlock = true;
    assert!(matches!(
        recover_ordered(&mut host, true, false).await,
        Err(RecoveryFailure::Network(ServiceError::Unavailable))
    ));
    assert!(!host.events.contains(&"clear-durable-disposition"));
}

#[test]
fn durable_choice_survives_restart_and_missing_legacy_choice_is_restrictive() {
    assert!(keep_after_stop(false, Some(true)));
    assert!(!keep_after_stop(false, Some(false)));
    assert!(keep_after_stop(false, None));
    assert!(!keep_after_stop(true, Some(true)));
}

#[tokio::test]
async fn pidfd_exit_wakes_safety_without_network_event_or_rpc() {
    let (network, receiver) = tokio::sync::watch::channel(0);
    let signal = std::sync::Arc::new(tokio::sync::Notify::new());
    let (reports, mut observed) = tokio::sync::watch::channel((0, 0));
    let backend = std::sync::Arc::new(std::sync::Mutex::new(CleanupOnEvent {
        active: false,
        calls: 0,
        cleanups: 0,
        reports,
    }));
    let _safety = super::spawn_safety(
        super::SessionEvents::new(TestEvents(receiver), signal.clone()),
        std::sync::Arc::downgrade(&backend),
        tokio::runtime::Handle::current(),
        cleanup_on_event,
    );
    observed.changed().await.unwrap();
    backend.lock().unwrap().active = true;
    let mut child = tokio::process::Command::new("/usr/bin/sleep")
        .arg("60")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let _watch = super::watch_worker_exit(child.id().unwrap(), signal).unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), observed.changed())
            .await
            .is_err()
    );
    child.kill().await.unwrap();
    child.wait().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), observed.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*observed.borrow(), (2, 1));
    assert_eq!(*network.borrow(), 0);
}
