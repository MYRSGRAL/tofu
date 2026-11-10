use super::*;

#[test]
fn cgroup_events_requires_a_single_zero_populated_field() {
    assert_eq!(empty_events("populated 0\nfrozen 0\n"), Ok(true));
    assert_eq!(empty_events("populated 1\nfrozen 0\n"), Ok(false));
    for malformed in [
        "",
        "populated 0",
        "populated 2\n",
        "populated 0\npopulated 0\n",
        "populated  0\n",
    ] {
        assert_eq!(
            empty_events(malformed),
            Err(WorkerSupervisorError::Untrusted)
        );
    }
}

#[test]
fn identity_never_accepts_a_replacement_or_unbound_pid() {
    let ticket = WorkerTicket {
        pid: 42,
        invocation: [7; 16],
    };
    let mut state = WorkerState {
        active: "active".into(),
        pid: 42,
        invocation: vec![7; 16],
    };
    assert!(ticket.matches(&state));
    state.pid = 43;
    assert!(!ticket.matches(&state));
    state.pid = 42;
    state.invocation[0] = 8;
    assert!(!ticket.matches(&state));
}

#[test]
fn fixed_units_have_no_client_supplied_names() {
    assert_eq!(WorkerMode::Run.service(), "yonik-worker.service");
    assert_eq!(WorkerMode::Check.service(), "yonik-worker-check.service");
    assert_eq!(WorkerMode::Run.socket(), "yonik-worker.socket");
    assert_eq!(
        WorkerMode::Check.cgroup(),
        "/system.slice/yonik-worker-check.service"
    );
}

use std::sync::Mutex;

struct PrivateBus {
    child: std::process::Child,
    address: String,
}
impl PrivateBus {
    fn start() -> Self {
        use std::io::BufRead;
        let mut child = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self {
            child,
            address: address.trim().into(),
        }
    }
    async fn connect(&self) -> Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap()
    }
}
impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[derive(Default)]
struct FakeState {
    jobs: Vec<(String, String)>,
    populated: bool,
    pid: u32,
    invocation: Vec<u8>,
    active: String,
    delay_ms: u64,
    completed: Vec<(String, String)>,
    socket_path: Option<std::path::PathBuf>,
    listener: Option<std::os::unix::net::UnixListener>,
    job_failed: bool,
}
struct FakeFence(Arc<Mutex<FakeState>>);
impl WorkerFence for FakeFence {
    fn empty(&self, _: WorkerMode) -> Result<bool> {
        Ok(!self.0.lock().unwrap().populated)
    }
}
struct FakeManager(Arc<Mutex<FakeState>>);
#[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
impl FakeManager {
    async fn subscribe(&self) {}
    async fn load_unit(&self, name: &str) -> zbus::fdo::Result<OwnedObjectPath> {
        if ![
            WorkerMode::Run.service(),
            WorkerMode::Run.socket(),
            WorkerMode::Check.service(),
            WorkerMode::Check.socket(),
        ]
        .contains(&name)
        {
            return Err(zbus::fdo::Error::InvalidArgs("Неизвестный unit".into()));
        }
        Ok(OwnedObjectPath::try_from(format!("/test/{}", name.replace(['.', '-'], "_"))).unwrap())
    }
    async fn start_unit(
        &self,
        name: &str,
        mode: &str,
        #[zbus(signal_context)] context: zbus::object_server::SignalContext<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        assert_eq!(mode, "replace");
        Ok(queue_job(
            Arc::clone(&self.0),
            "StartUnit",
            name,
            context.to_owned(),
        ))
    }
    async fn stop_unit(
        &self,
        name: &str,
        mode: &str,
        #[zbus(signal_context)] context: zbus::object_server::SignalContext<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        assert_eq!(mode, "replace");
        Ok(queue_job(
            Arc::clone(&self.0),
            "StopUnit",
            name,
            context.to_owned(),
        ))
    }
    #[zbus(signal)]
    async fn job_removed(
        context: &zbus::object_server::SignalContext<'_>,
        id: u32,
        job: &OwnedObjectPath,
        unit: &str,
        result: &str,
    ) -> zbus::Result<()>;
}
fn queue_job(
    state: Arc<Mutex<FakeState>>,
    method: &'static str,
    name: &str,
    context: zbus::object_server::SignalContext<'static>,
) -> OwnedObjectPath {
    let name = name.to_owned();
    let (id, delay) = {
        let mut state = state.lock().unwrap();
        state.jobs.push((method.into(), name.clone()));
        (state.jobs.len() as u32, state.delay_ms)
    };
    let path = OwnedObjectPath::try_from(format!("/job/{id}")).unwrap();
    let result_path = path.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(delay)).await;
        let result = {
            let mut state = state.lock().unwrap();
            if state.job_failed {
                "failed"
            } else {
                if method == "StartUnit" {
                    if let Some(path) = state.socket_path.as_ref() {
                        state.listener =
                            Some(std::os::unix::net::UnixListener::bind(path).unwrap());
                    }
                } else if name == WorkerMode::Run.service() {
                    state.pid = 0;
                    state.active = "inactive".into();
                }
                state.completed.push((method.into(), name.clone()));
                "done"
            }
        };
        let _ = FakeManager::job_removed(&context, id, &result_path, &name, result).await;
    });
    path
}
struct FakeUnit {
    state: Arc<Mutex<FakeState>>,
    run: bool,
}
#[zbus::interface(name = "org.freedesktop.systemd1.Unit")]
impl FakeUnit {
    #[zbus(property)]
    fn active_state(&self) -> String {
        if self.run {
            self.state.lock().unwrap().active.clone()
        } else {
            "inactive".into()
        }
    }
    #[zbus(property, name = "InvocationID")]
    fn invocation_id(&self) -> Vec<u8> {
        self.state.lock().unwrap().invocation.clone()
    }
}
struct FakeService {
    state: Arc<Mutex<FakeState>>,
    run: bool,
}
#[zbus::interface(name = "org.freedesktop.systemd1.Service")]
impl FakeService {
    #[zbus(property, name = "MainPID")]
    fn main_pid(&self) -> u32 {
        if self.run {
            self.state.lock().unwrap().pid
        } else {
            0
        }
    }
    #[zbus(property)]
    fn control_group(&self) -> String {
        if self.run {
            WorkerMode::Run.cgroup()
        } else {
            WorkerMode::Check.cgroup()
        }
        .into()
    }
}
struct Fixture {
    supervisor: SystemWorkerSupervisor,
    state: Arc<Mutex<FakeState>>,
    _server: Connection,
    _bus: PrivateBus,
}
impl Fixture {
    async fn new() -> Self {
        let bus = PrivateBus::start();
        let server = bus.connect().await;
        let state = Arc::new(Mutex::new(FakeState {
            invocation: vec![7; 16],
            active: "inactive".into(),
            ..FakeState::default()
        }));
        server
            .object_server()
            .at(MANAGER, FakeManager(Arc::clone(&state)))
            .await
            .unwrap();
        for mode in [WorkerMode::Run, WorkerMode::Check] {
            for name in [mode.service(), mode.socket()] {
                let path = format!("/test/{}", name.replace(['.', '-'], "_"));
                server
                    .object_server()
                    .at(
                        path.as_str(),
                        FakeUnit {
                            state: Arc::clone(&state),
                            run: name == WorkerMode::Run.service(),
                        },
                    )
                    .await
                    .unwrap();
                if name == mode.service() {
                    server
                        .object_server()
                        .at(
                            path.as_str(),
                            FakeService {
                                state: Arc::clone(&state),
                                run: mode == WorkerMode::Run,
                            },
                        )
                        .await
                        .unwrap();
                }
            }
        }
        let supervisor = SystemWorkerSupervisor {
            connection: bus.connect().await,
            owner: server.unique_name().unwrap().to_string(),
            permit: Arc::new(Permit),
            pending: Some(WorkerMode::Run),
            transfer: None,
            current: None,
            clean: false,
            fence: Box::new(FakeFence(Arc::clone(&state))),
        };
        Self {
            supervisor,
            state,
            _server: server,
            _bus: bus,
        }
    }
}
#[tokio::test]
async fn cleanup_stops_both_sockets_before_services_and_confirms_empty() {
    let mut fixture = Fixture::new().await;
    fixture.state.lock().unwrap().delay_ms = 20;
    let (_sender, mut cancel) = Cancellation::new();
    fixture
        .supervisor
        .cleanup(&mut cancel, Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    assert!(fixture.supervisor.clean);
    let jobs = fixture.state.lock().unwrap().jobs.clone();
    assert_eq!(
        jobs,
        vec![
            ("StopUnit".into(), "yonik-worker-check.socket".into()),
            ("StopUnit".into(), "yonik-worker.socket".into()),
            ("StopUnit".into(), "yonik-worker-check.service".into()),
            ("StopUnit".into(), "yonik-worker.service".into())
        ]
    );
    assert_eq!(fixture.state.lock().unwrap().completed, jobs);
}

#[tokio::test]
async fn activation_waits_for_delayed_job_before_connect() {
    let fixture = Fixture::new().await;
    let directory = std::env::temp_dir().join(format!("yonik-delayed-job-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Area(std::path::PathBuf);
    impl Drop for Area {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(self.0.join("worker.sock"));
            let _ = std::fs::remove_dir(&self.0);
        }
    }
    let area = Area(directory);
    let socket = area.0.join("worker.sock");
    {
        let mut state = fixture.state.lock().unwrap();
        state.delay_ms = 40;
        state.socket_path = Some(socket.clone());
    }
    fixture
        .supervisor
        .unit_job("StartUnit", WorkerMode::Run.socket())
        .await
        .unwrap();
    assert!(std::os::unix::net::UnixStream::connect(socket).is_ok());
}

#[tokio::test]
async fn failed_systemd_job_is_not_readiness() {
    let fixture = Fixture::new().await;
    fixture.state.lock().unwrap().job_failed = true;
    assert_eq!(
        fixture
            .supervisor
            .unit_job("StartUnit", WorkerMode::Run.socket())
            .await,
        Err(WorkerSupervisorError::Protocol)
    );
}
#[tokio::test]
async fn inactive_state_cannot_confirm_cleanup_of_populated_cgroup() {
    let mut fixture = Fixture::new().await;
    fixture.state.lock().unwrap().populated = true;
    let (_sender, mut cancel) = Cancellation::new();
    assert_eq!(
        fixture
            .supervisor
            .cleanup(&mut cancel, Instant::now() + Duration::from_millis(80))
            .await,
        Err(WorkerSupervisorError::Timeout)
    );
    assert!(!fixture.supervisor.clean);
    assert!(fixture.supervisor.pending.is_some());
}
#[tokio::test]
async fn replacement_ticket_never_stops_a_service() {
    let mut fixture = Fixture::new().await;
    {
        let mut state = fixture.state.lock().unwrap();
        state.pid = 42;
        state.invocation = vec![8; 16];
        state.active = "active".into();
    }
    let ticket = WorkerTicket {
        pid: 42,
        invocation: [7; 16],
    };
    let (_sender, mut cancel) = Cancellation::new();
    assert_eq!(
        fixture.supervisor.stop(&ticket, &mut cancel).await,
        Err(WorkerSupervisorError::Stale)
    );
    assert!(fixture.state.lock().unwrap().jobs.is_empty());
    assert_eq!(
        fixture.supervisor.observe(&ticket).await,
        Err(WorkerSupervisorError::Stale)
    );
}
#[tokio::test]
async fn cancelled_cleanup_retains_transfer_until_recovery_waits_for_it() {
    let mut fixture = Fixture::new().await;
    let (release, wait) = std::sync::mpsc::channel();
    fixture.supervisor.transfer = Some(tokio::task::spawn_blocking(move || {
        wait.recv().unwrap();
        Ok(WorkerAck {
            pid: 42,
            status: WorkerStatus::Accepted,
        })
    }));
    let (_sender, mut cancel) = Cancellation::new();
    let result = fixture
        .supervisor
        .cleanup(&mut cancel, Instant::now() + Duration::from_millis(50))
        .await;
    assert_eq!(result, Err(WorkerSupervisorError::Timeout));
    assert!(fixture.supervisor.transfer.is_some());
    assert_eq!(fixture.state.lock().unwrap().jobs.len(), 2);
    release.send(()).unwrap();
    fixture
        .supervisor
        .cleanup(&mut cancel, Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    assert!(fixture.supervisor.transfer.is_none());
    assert!(fixture.supervisor.clean);
}

fn value<'a>(v: impl Into<zbus::zvariant::Value<'a>>) -> OwnedValue {
    OwnedValue::try_from(v.into()).unwrap()
}
fn service_contract(mode: WorkerMode) -> Properties {
    let mut p = Properties::new();
    for (key, v) in [
        ("User", "yonik-worker"),
        ("Group", "yonik-worker"),
        ("Type", "exec"),
        ("Restart", "no"),
        ("KillMode", "control-group"),
        ("Slice", "system.slice"),
        ("StandardInput", "socket"),
        ("StandardOutput", "null"),
        ("StandardError", "null"),
        ("ProtectSystem", "strict"),
        ("ProtectHome", "yes"),
        ("DevicePolicy", "closed"),
    ] {
        p.insert(key.into(), value(v));
    }
    for key in [
        "NoNewPrivileges",
        "SendSIGKILL",
        "PrivateTmp",
        "ProtectKernelTunables",
        "ProtectKernelModules",
        "ProtectKernelLogs",
        "ProtectControlGroups",
        "RestrictRealtime",
        "RestrictSUIDSGID",
        "LockPersonality",
        "MemoryDenyWriteExecute",
    ] {
        p.insert(key.into(), value(true));
    }
    for key in ["PrivateNetwork", "PrivateDevices"] {
        p.insert(key.into(), value(mode == WorkerMode::Check));
    }
    for key in ["CapabilityBoundingSet", "AmbientCapabilities"] {
        p.insert(
            key.into(),
            value(if mode == WorkerMode::Check {
                0u64
            } else {
                1 << 12
            }),
        );
    }
    for (key, v) in [
        ("MemoryMax", 256 * 1024 * 1024u64),
        ("TasksMax", 64),
        ("TimeoutStopUSec", 5_000_000),
    ] {
        p.insert(key.into(), value(v));
    }
    p.insert("UMask".into(), value(0o77u32));
    p.insert("KillSignal".into(), value(15i32));
    p.insert("FinalKillSignal".into(), value(9i32));
    let exec: ExecCommands = vec![(
        "/usr/lib/yonik/yonik-worker".into(),
        vec![
            "/usr/lib/yonik/yonik-worker".into(),
            if mode == WorkerMode::Check {
                "--check"
            } else {
                "--run"
            }
            .into(),
        ],
        false,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    )];
    p.insert("ExecStart".into(), value(exec));
    p.insert(
        "DeviceAllow".into(),
        value(if mode == WorkerMode::Check {
            Vec::<(String, String)>::new()
        } else {
            vec![("/dev/net/tun".into(), "rw".into())]
        }),
    );
    p
}
#[test]
fn loaded_service_contract_rejects_privilege_and_command_changes() {
    for mode in [WorkerMode::Run, WorkerMode::Check] {
        assert_eq!(validate_service(&service_contract(mode), mode), Ok(()));
        for (key, v) in [
            ("User", value("root")),
            ("AmbientCapabilities", value(u64::MAX)),
            ("CapabilityBoundingSet", value(u64::MAX)),
            ("NoNewPrivileges", value(false)),
            ("PrivateNetwork", value(mode != WorkerMode::Check)),
            ("KillMode", value("process")),
            ("MemoryMax", value(u64::MAX)),
            ("KillSignal", value(9i32)),
            ("TimeoutStopUSec", value(u64::MAX)),
        ] {
            let mut p = service_contract(mode);
            p.insert(key.into(), v);
            assert_eq!(
                validate_service(&p, mode),
                Err(WorkerSupervisorError::Untrusted),
                "{key}"
            );
        }
        let mut p = service_contract(mode);
        p.remove("DeviceAllow");
        assert_eq!(
            validate_service(&p, mode),
            Err(WorkerSupervisorError::Untrusted)
        );
    }
}
#[tokio::test]
async fn closed_cancellation_source_preserves_dirty_state() {
    let mut fixture = Fixture::new().await;
    let (sender, mut cancel) = Cancellation::new();
    drop(sender);
    assert_eq!(
        fixture
            .supervisor
            .cleanup(&mut cancel, Instant::now() + Duration::from_secs(1))
            .await,
        Err(WorkerSupervisorError::Cancelled)
    );
    assert!(!fixture.supervisor.clean);
    assert!(fixture.supervisor.pending.is_some());
    assert!(fixture.state.lock().unwrap().jobs.is_empty());
}

#[test]
fn missing_cgroup_is_empty_but_symlink_and_wrong_type_are_unknown() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("yonik-worker-fence-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let parent = File::open(&path).unwrap();
    assert!(
        open_worker_group(&parent, WorkerMode::Run)
            .unwrap()
            .is_none()
    );
    std::fs::write(path.join(WorkerMode::Run.service()), b"").unwrap();
    assert!(open_worker_group(&parent, WorkerMode::Run).is_err());
    std::fs::remove_file(path.join(WorkerMode::Run.service())).unwrap();
    std::os::unix::fs::symlink(&path, path.join(WorkerMode::Run.service())).unwrap();
    assert!(open_worker_group(&parent, WorkerMode::Run).is_err());
    std::fs::remove_file(path.join(WorkerMode::Run.service())).unwrap();
    std::fs::create_dir(path.join(WorkerMode::Run.service())).unwrap();
    assert!(
        open_worker_group(&parent, WorkerMode::Run)
            .unwrap()
            .is_some()
    );
    std::fs::remove_dir(path.join(WorkerMode::Run.service())).unwrap();
    std::fs::remove_dir(path).unwrap();
}
