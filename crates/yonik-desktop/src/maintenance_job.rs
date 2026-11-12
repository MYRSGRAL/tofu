//! Все ожидания repository gates находятся в job, вне actor dispatch.
use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum Failure {
    Copy(yonik_linux::MaintenanceError),
    Reopen(yonik_linux::RepositoryError),
    Access(yonik_linux::RepositoryError),
}
impl Failure {
    pub fn context(self) -> Cause {
        match self {
            Self::Copy(yonik_linux::MaintenanceError::Cancelled) => Cause::MaintenanceCancelled,
            Self::Copy(yonik_linux::MaintenanceError::Repository(
                yonik_linux::RepositoryError::InvalidInput,
            )) => Cause::MaintenanceCopyInvalidInput,
            Self::Copy(yonik_linux::MaintenanceError::Repository(
                yonik_linux::RepositoryError::UnsafePath,
            )) => Cause::MaintenanceCopyUnsafePath,
            Self::Copy(yonik_linux::MaintenanceError::Repository(
                yonik_linux::RepositoryError::UnsupportedVersion,
            )) => Cause::MaintenanceCopyUnsupportedVersion,
            Self::Copy(yonik_linux::MaintenanceError::Repository(
                yonik_linux::RepositoryError::Corrupt,
            )) => Cause::MaintenanceCopyCorrupt,
            Self::Copy(yonik_linux::MaintenanceError::Repository(
                yonik_linux::RepositoryError::Storage,
            )) => Cause::MaintenanceCopyStorage,
            Self::Copy(yonik_linux::MaintenanceError::Repository(
                yonik_linux::RepositoryError::Conflict,
            )) => Cause::MaintenanceCopyConflict,
            Self::Copy(yonik_linux::MaintenanceError::Repository(
                yonik_linux::RepositoryError::NotFound,
            )) => Cause::MaintenanceCopyNotFound,
            Self::Reopen(yonik_linux::RepositoryError::InvalidInput) => {
                Cause::MaintenanceReopenInvalidInput
            }
            Self::Reopen(yonik_linux::RepositoryError::UnsafePath) => {
                Cause::MaintenanceReopenUnsafePath
            }
            Self::Reopen(yonik_linux::RepositoryError::UnsupportedVersion) => {
                Cause::MaintenanceReopenUnsupportedVersion
            }
            Self::Reopen(yonik_linux::RepositoryError::Corrupt) => Cause::MaintenanceReopenCorrupt,
            Self::Reopen(yonik_linux::RepositoryError::Storage) => Cause::MaintenanceReopenStorage,
            Self::Reopen(yonik_linux::RepositoryError::Conflict) => {
                Cause::MaintenanceReopenConflict
            }
            Self::Reopen(yonik_linux::RepositoryError::NotFound) => {
                Cause::MaintenanceReopenNotFound
            }
            Self::Access(yonik_linux::RepositoryError::InvalidInput) => {
                Cause::MaintenanceAccessInvalidInput
            }
            Self::Access(yonik_linux::RepositoryError::UnsafePath) => {
                Cause::MaintenanceAccessUnsafePath
            }
            Self::Access(yonik_linux::RepositoryError::UnsupportedVersion) => {
                Cause::MaintenanceAccessUnsupportedVersion
            }
            Self::Access(yonik_linux::RepositoryError::Corrupt) => Cause::MaintenanceAccessCorrupt,
            Self::Access(yonik_linux::RepositoryError::Storage) => Cause::MaintenanceAccessStorage,
            Self::Access(yonik_linux::RepositoryError::Conflict) => {
                Cause::MaintenanceAccessConflict
            }
            Self::Access(yonik_linux::RepositoryError::NotFound) => {
                Cause::MaintenanceAccessNotFound
            }
        }
    }
}

pub(super) struct Outcome {
    pub cause: Option<Failure>,
    pub drained: bool,
    pub handles: Option<(
        SubscriptionService,
        EncryptedSubscriptionRepository,
        DesktopSchedule,
    )>,
    pub result: std::result::Result<(), &'static str>,
}

pub(super) async fn run(
    service: SubscriptionService,
    repository: EncryptedSubscriptionRepository,
    schedule: DesktopSchedule,
    refresh: Option<RefreshTask>,
    request: maintenance::Request,
    cancel: yonik_linux::MaintenanceControl,
    now: Duration,
) -> Outcome {
    #[cfg(test)]
    {
        let gate = TEST_GATE
            .lock()
            .expect("test gate")
            .as_ref()
            .filter(|(path, _, _)| *path == request.destination)
            .map(|(_, entered, release)| (entered.clone(), release.clone()));
        if let Some((entered, release)) = gate {
            return run_with_leaf(
                service,
                repository,
                schedule,
                refresh,
                request,
                cancel,
                now,
                move |access, _, _| async move {
                    entered.notify_one();
                    release.notified().await;
                    (access, Err("cancelled"), None)
                },
            )
            .await;
        }
    }
    run_with_leaf(
        service,
        repository,
        schedule,
        refresh,
        request,
        cancel,
        now,
        |access, request, cancel| async move {
            let typed_result = access
                .copy_closed(request.source, request.destination, cancel)
                .await;
            let cause = typed_result.as_ref().err().copied().map(Failure::Copy);
            let result = typed_result.map_err(|error| match error {
                yonik_linux::MaintenanceError::Cancelled => "cancelled",
                yonik_linux::MaintenanceError::Repository(
                    yonik_linux::RepositoryError::InvalidInput
                    | yonik_linux::RepositoryError::UnsafePath,
                ) => "invalid_input",
                yonik_linux::MaintenanceError::Repository(
                    yonik_linux::RepositoryError::Conflict,
                ) => "busy",
                _ => "storage",
            });
            (access, result, cause)
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_with_leaf<F, Fut>(
    service: SubscriptionService,
    repository: EncryptedSubscriptionRepository,
    mut schedule: DesktopSchedule,
    refresh: Option<RefreshTask>,
    request: maintenance::Request,
    cancel: yonik_linux::MaintenanceControl,
    now: Duration,
    leaf: F,
) -> Outcome
where
    F: FnOnce(
        yonik_linux::MaintenanceAccess,
        maintenance::Request,
        yonik_linux::MaintenanceControl,
    ) -> Fut,
    Fut: std::future::Future<
            Output = (
                yonik_linux::MaintenanceAccess,
                std::result::Result<(), &'static str>,
                Option<Failure>,
            ),
        >,
{
    let started = std::time::Instant::now();
    if let Some((_, sender, task, _, _)) = refresh {
        sender.send_replace(true);
        // Join — подтверждение освобождения service clone, включая commit ACK.
        let _result = task.await;
    }
    let schedule_closed = schedule.runtime.shutdown(now).await.is_ok();
    let replacement = schedule.replacement();
    drop(schedule);
    let access = repository.maintenance_access().await;
    let service_closed = service.close().await.is_ok();
    let repository_closed = repository.close().await.is_ok();
    let access = match access {
        Ok(access) => access,
        Err(error) => {
            return Outcome {
                cause: Some(Failure::Access(error)),
                drained: schedule_closed && service_closed && repository_closed,
                handles: None,
                result: Err("storage"),
            };
        }
    };
    if !schedule_closed || !service_closed || !repository_closed {
        return Outcome {
            cause: None,
            drained: false,
            handles: None,
            result: Err("cleanup_failed"),
        };
    }
    let (access, result, cause) = leaf(access, request, cancel).await;
    match access.reopen().await {
        Ok(repository) => {
            let service = SubscriptionService::new(repository.clone());
            match replacement {
                Ok(mut schedule) => {
                    let loaded = schedule
                        .load(&service, now.saturating_add(started.elapsed()))
                        .await;
                    Outcome {
                        cause: if loaded.is_ok() { cause } else { None },
                        drained: true,
                        handles: Some((service, repository, schedule)),
                        result: if loaded.is_ok() {
                            result
                        } else {
                            Err("storage")
                        },
                    }
                }
                Err(_) => {
                    let service_closed = service.close().await.is_ok();
                    let repository_closed = repository.close().await.is_ok();
                    Outcome {
                        cause: None,
                        drained: service_closed && repository_closed,
                        handles: None,
                        result: Err("storage"),
                    }
                }
            }
        }
        Err(error) => Outcome {
            cause: Some(Failure::Reopen(error)),
            drained: true,
            handles: None,
            result: Err("storage"),
        },
    }
}

#[cfg(test)]
pub(super) fn assert_admission_context(state: &serde_json::Value, reason: &str) {
    let raw = state.to_string();
    if let Some(directory) = std::env::var_os("YONIK_MAINTENANCE_EVIDENCE") {
        std::fs::write(
            PathBuf::from(directory).join(format!("{reason}.json")),
            &raw,
        )
        .expect("evidence");
    }

    let exported = crate::diagnostics::export_snapshot_v2(&raw).expect("actual admission V2");
    let metadata: serde_json::Value = serde_json::from_str(&exported).expect("JSON");
    let context = metadata["errors"]
        .as_array()
        .expect("errors")
        .iter()
        .find(|item| item["slot"] == "maintenance_ack")
        .expect("matching ACK context");
    assert_eq!(context["reason"], reason);
    assert_eq!(context["stage"], "maintenance_admission");
    assert_eq!(context["params"], serde_json::json!({}));
    assert_eq!(
        context["request_id"],
        state["maintenance_ack"]["request_id"].to_string()
    );
    let index = state["error_contexts"]
        .as_array()
        .expect("contexts")
        .iter()
        .position(|item| item["slot"] == "maintenance_ack")
        .expect("maintenance context");
    for field in ["request_id", "generation", "slot"] {
        let mut invalid = state.clone();
        invalid["error_contexts"][index][field] = serde_json::json!("999999");
        assert!(crate::diagnostics::export_snapshot_v2(&invalid.to_string()).is_none());
    }
}

#[cfg(test)]
type TestGate = (PathBuf, Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>);
#[cfg(test)]
static TEST_GATE: Mutex<Option<TestGate>> = Mutex::new(None);

#[cfg(test)]
mod tests {
    use super::*;
    static TEST_SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn shutdown_is_responsive_while_leaf_is_gated_and_lock_survives_until_drain() {
        let _serial = TEST_SERIAL.lock().expect("serial fixture");
        let directory =
            std::env::temp_dir().join(format!("yonik-maintenance-shutdown-{}", std::process::id()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .expect("fixture");
        let data = directory.join("data");
        let destination = directory.join("new.sqlite");
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        *TEST_GATE.lock().expect("test gate") =
            Some((destination.clone(), entered.clone(), release.clone()));
        let (sender, receiver) = std::sync::mpsc::channel();
        let host = DesktopHost::start(
            HostConfig::new(data.clone(), directory.join("engines")).with_ephemeral_key([67; 32]),
            move |raw| {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                    let _closed = sender.send(value);
                }
            },
        )
        .expect("host");
        let ready = loop {
            let state = receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("snapshot");
            if state["ready"] == true {
                break state;
            }
        };
        host.dispatch(&serde_json::json!({"command":"maintenance_start","request_id":1,"operation":"backup","destination":destination,"revision":ready["revision"]}).to_string()).expect("command");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime fixture");
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), entered.notified())
                .await
                .expect("leaf entered");
        });
        struct ReleaseOnDrop(Arc<tokio::sync::Notify>);
        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                self.0.notify_one();
            }
        }
        let _release_on_failure = ReleaseOnDrop(release.clone());
        host.dispatch(r#"{"command":"maintenance_cancel","request_id":2}"#)
            .expect("чужой запрос");
        loop {
            let state = receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("busy ACK");
            if state["maintenance_ack"]["request_id"] == 2 {
                assert_admission_context(&state, "maintenance_admission_draining");
                break;
            }
        }
        for command in [
            serde_json::json!({"command":"logs_read","after":"0","source":"all","request_id":"71"}),
            serde_json::json!({"command":"logs_save","page_request_id":"71","destination":directory.join("report.json"),"request_id":"72"}),
        ] {
            host.dispatch(&command.to_string())
                .expect("команда журнала");
            let state = receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("logs ACK");
            assert_eq!(state["logs"]["request_id"], command["request_id"]);
            assert_eq!(state["logs"]["status"], "busy");
            assert!(state["logs"]["export"].is_null());
        }
        host.dispatch(&serde_json::json!({"command":"maintenance_start","request_id":1,"operation":"backup","destination":destination,"revision":ready["revision"]}).to_string()).expect("повтор Start");
        let repeated = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("повторный ACK");
        assert_eq!(repeated["maintenance_ack"]["request_id"], 1);
        assert_eq!(repeated["maintenance_ack"]["phase"], "draining");
        assert!(repeated["maintenance_ack"]["error"].is_null());
        host.dispatch(r#"{"command":"stop"}"#)
            .expect("Stop admission");
        loop {
            let state = receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("Stop ACK while leaf blocked");
            if state["maintenance_ack"]["phase"] == "cancelling" {
                break;
            }
        }
        let start = std::time::Instant::now();
        host.shutdown();
        assert!(start.elapsed() < Duration::from_millis(200));
        assert!(!host.wait_closed(Duration::from_millis(30)));
        let directory_lock = File::open(&data).expect("directory lock");
        assert!(
            rustix::fs::flock(
                &directory_lock,
                rustix::fs::FlockOperation::NonBlockingLockExclusive
            )
            .is_err()
        );
        drop(directory_lock);
        release.notify_one();
        assert!(host.wait_closed(Duration::from_secs(10)));
        drop(host);
        *TEST_GATE.lock().expect("test gate") = None;
        for entry in fs::read_dir(&data).expect("fixture") {
            fs::remove_file(entry.expect("файл").path()).expect("очистка");
        }
        fs::remove_dir(data).expect("очистка data");
        fs::remove_dir(directory).expect("очистка fixture");
    }

    #[test]
    fn reopen_failure_disables_maintenance_and_matches_each_rejected_request() {
        let _serial = TEST_SERIAL.lock().expect("serial fixture");
        let directory =
            std::env::temp_dir().join(format!("yonik-maintenance-reopen-{}", std::process::id()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .expect("fixture");
        let data = directory.join("data");
        let destination = directory.join("new.sqlite");
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        *TEST_GATE.lock().expect("test gate") =
            Some((destination.clone(), entered.clone(), release.clone()));
        let (sender, receiver) = std::sync::mpsc::channel();
        let host = DesktopHost::start(
            HostConfig::new(data.clone(), directory.join("engines")).with_ephemeral_key([68; 32]),
            move |raw| {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                    let _sent = sender.send(value);
                }
            },
        )
        .expect("host");
        struct ReleaseOnDrop(Arc<tokio::sync::Notify>);
        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                self.0.notify_one();
            }
        }
        let _release_on_failure = ReleaseOnDrop(release.clone());
        let ready = loop {
            let state = receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("ready");
            if state["ready"] == true {
                break state;
            }
        };
        host.dispatch(&serde_json::json!({"command":"maintenance_start","request_id":1,"operation":"backup","destination":destination,"revision":ready["revision"]}).to_string()).expect("Start");
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(5), entered.notified())
                    .await
                    .expect("leaf closed DB");
            });
        // Подтверждённо закрытая БД сохранена, а её путь становится недоступным для reopen.
        let database = data.join("subscriptions.sqlite");
        let held = data.join("held.sqlite");
        fs::rename(&database, &held).expect("сохранить fixture DB");
        fs::create_dir(&database).expect("reopen fault");
        release.notify_one();
        loop {
            let state = receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("terminal ACK");
            if state["maintenance_ack"]["phase"] == "failed" {
                assert_eq!(state["maintenance_available"], false);
                assert_eq!(state["ready"], false);
                break;
            }
        }
        for id in [2, 3] {
            let command = if id == 2 {
                serde_json::json!({"command":"maintenance_start","request_id":id,"operation":"backup","destination":destination,"revision":ready["revision"]})
            } else {
                serde_json::json!({"command":"maintenance_cancel","request_id":id})
            };
            host.dispatch(&command.to_string())
                .expect("offline command");
            let state = receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("matching offline ACK");
            assert_eq!(state["maintenance_ack"]["request_id"], id);
            assert_eq!(state["maintenance_ack"]["phase"], "failed");
            assert_eq!(state["maintenance_ack"]["error"], "busy");
            assert_admission_context(&state, "maintenance_admission_reopen_unavailable");
        }
        assert!(!host.wait_closed(Duration::from_millis(20)));
        host.shutdown();
        assert!(host.wait_closed(Duration::from_secs(5)));
        drop(host);
        *TEST_GATE.lock().expect("gate") = None;
        fs::remove_dir(database).expect("очистка fault");
        for entry in fs::read_dir(&data).expect("fixture") {
            fs::remove_file(entry.expect("файл").path()).expect("очистка");
        }
        fs::remove_dir(data).expect("очистка data");
        fs::remove_dir(directory).expect("очистка fixture");
    }

    #[tokio::test]
    async fn fake_leaf_runs_after_real_handles_drain_and_reopens_after_release() {
        let directory =
            std::env::temp_dir().join(format!("yonik-maintenance-leaf-{}", std::process::id()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .expect("fixture");
        let path = directory.join("source.sqlite");
        let repository =
            EncryptedSubscriptionRepository::open(path.clone(), Zeroizing::new([53; 32]))
                .await
                .expect("БД");
        let service = SubscriptionService::new(repository.clone());
        let mut schedule = DesktopSchedule::new().expect("scheduler");
        schedule.interval = Duration::from_secs(172800);
        schedule
            .runtime
            .set_interval(schedule.interval)
            .expect("интервал");
        schedule.background_enabled = true;
        schedule.window_until = Some(Duration::from_secs(60));
        let request = maintenance::Request {
            request_id: 1,
            operation: maintenance::Operation::Backup,
            source: None,
            destination: directory.join("new.sqlite"),
            revision: 0,
        };
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(run_with_leaf(
            service,
            repository,
            schedule,
            None,
            request,
            yonik_linux::MaintenanceControl::default(),
            Duration::ZERO,
            move |access, _, _| async move {
                // Если job закрыл лишь clone, второе открытие не получит SQLite lock.
                let probe = EncryptedSubscriptionRepository::open(
                    access.source().to_path_buf(),
                    Zeroizing::new([53; 32]),
                )
                .await
                .expect("все реальные handles закрыты");
                probe.close().await.expect("закрытие probe");
                entered.send(()).expect("готовность");
                released.await.expect("leaf release");
                (access, Err("cancelled"), None)
            },
        ));
        tokio::time::timeout(Duration::from_secs(5), ready)
            .await
            .expect("job не блокирует runtime")
            .expect("leaf");
        assert!(!task.is_finished());
        release.send(()).expect("release");
        let outcome = task.await.expect("join");
        assert_eq!(outcome.result, Err("cancelled"));
        let (service, repository, mut schedule) = outcome.handles.expect("reopen после drain");
        assert_eq!(schedule.interval, Duration::from_secs(172800));
        assert!(schedule.background_enabled);
        assert_eq!(schedule.window_until, Some(Duration::from_secs(60)));
        schedule
            .runtime
            .shutdown(Duration::ZERO)
            .await
            .expect("scheduler close");
        drop(schedule);
        service.close().await.expect("service close");
        repository.close().await.expect("repository close");
        for entry in fs::read_dir(&directory).expect("fixture") {
            fs::remove_file(entry.expect("файл").path()).expect("очистка");
        }
        fs::remove_dir(directory).expect("очистка каталога");
    }
}
