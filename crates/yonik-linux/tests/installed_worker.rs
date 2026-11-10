//! Только одноразовая root VM с настоящими systemd и cgroup2.
//! Проверяет production SystemWorkerSupervisor, но helper — sleep fixture,
//! а искусственное ядро явно передано root и не проходит production registry.
#![cfg(target_os = "linux")]

use sha2::{Digest, Sha256 as Hasher};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    num::NonZeroU16,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use yonik_engine::Engine;
use yonik_linux::{
    BinarySnapshot, Cancellation, Sha256,
    helper::{SystemWorkerSupervisor, WorkerLiveness, WorkerSupervisorError},
    worker::WorkerPayload,
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const UNITS: [&str; 4] = [
    "yonik-worker.service",
    "yonik-worker.socket",
    "yonik-worker-check.service",
    "yonik-worker-check.socket",
];
const GROUP: &str = "/sys/fs/cgroup/system.slice/yonik-worker.service";
const LIMIT: Duration = Duration::from_secs(10);
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[path = "common/vm_lock.rs"]
mod vm_lock;

fn ensure(ok: bool, message: &str) -> Result {
    if ok {
        Ok(())
    } else {
        Err(io::Error::other(message).into())
    }
}

fn bounded_read(path: impl AsRef<Path>, limit: u64) -> io::Result<String> {
    let mut value = String::new();
    File::open(path)?
        .take(limit + 1)
        .read_to_string(&mut value)?;
    if value.len() as u64 > limit {
        return Err(io::Error::other("Превышен предел диагностического файла"));
    }
    Ok(value)
}

// Наличие token означает, что cleanup разрешён только в выделенной VM.
struct VmGate {
    _lock: File,
}
impl VmGate {
    fn acquire() -> Result<Self> {
        vm_guard(std::env::var_os("YONIK_SYSTEMD_VM_GATE").as_deref())?;
        Ok(Self {
            _lock: vm_lock::acquire()?,
        })
    }
}
fn capture_file() -> io::Result<File> {
    rustix::fs::memfd_create("yonik-gate-output", rustix::fs::MemfdFlags::CLOEXEC)
        .map(File::from)
        .map_err(io::Error::from)
}
fn captured(file: &mut File) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(io::Error::other("Превышен предел вывода systemctl"));
    }
    Ok(bytes)
}
fn vm_guard(opt_in: Option<&OsStr>) -> Result {
    ensure(
        opt_in == Some(OsStr::new("1")),
        "Требуется YONIK_SYSTEMD_VM_GATE=1",
    )?;
    ensure(
        rustix::process::geteuid().is_root(),
        "Gate требует root внутри VM",
    )?;
    let marker = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open("/run/yonik-vm-gate")?;
    let metadata = marker.metadata()?;
    ensure(
        metadata.is_file() && metadata.uid() == 0 && metadata.mode() & 0o7777 == 0o600,
        "Небезопасный VM marker",
    )?;
    let mut contents = String::new();
    marker.take(128).read_to_string(&mut contents)?;
    ensure(
        contents == "yonik-disposable-systemd-gate-v1\n",
        "Неизвестный VM marker",
    )?;
    let vendor = bounded_read("/sys/class/dmi/id/sys_vendor", 256)?;
    let product = bounded_read("/sys/class/dmi/id/product_name", 256)?;
    ensure(
        vendor.trim() == "QEMU" && product.trim().starts_with("Standard PC"),
        "Gate разрешён только для QEMU Standard PC",
    )?;
    ensure(
        bounded_read("/proc/1/comm", 128)?.trim() == "systemd",
        "PID 1 должен быть systemd",
    )?;
    let cgroup = rustix::fs::open(
        "/sys/fs/cgroup",
        rustix::fs::OFlags::RDONLY,
        rustix::fs::Mode::empty(),
    )?;
    ensure(
        rustix::fs::fstatfs(cgroup)?.f_type as u64 == 0x6367_7270,
        "Требуется cgroup2",
    )?;
    Ok(())
}

impl VmGate {
    fn command(&self, args: &[&str]) -> Result<Output> {
        // Повторная read-only проверка непосредственно перед каждым subprocess effect.
        vm_guard(std::env::var_os("YONIK_SYSTEMD_VM_GATE").as_deref())?;
        // Regular memfd не ждёт EOF от унаследованного write-end, в отличие
        // от pipe/wait_with_output. Чтение ограничено независимо от потомков.
        let mut stdout = capture_file()?;
        let mut stderr = capture_file()?;
        let mut child = Command::new("/usr/bin/systemctl")
            .args(args)
            .env_clear()
            .env("LANG", "C")
            .env("SYSTEMD_PAGER", "cat")
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout.try_clone()?))
            .stderr(Stdio::from(stderr.try_clone()?))
            .spawn()?;
        let deadline = Instant::now() + LIMIT;
        loop {
            if let Some(status) = child.try_wait()? {
                ensure(status.success(), "systemctl завершился с ошибкой")?;
                return Ok(Output {
                    status,
                    stdout: captured(&mut stdout)?,
                    stderr: captured(&mut stderr)?,
                });
            }
            if Instant::now() >= deadline {
                child.kill()?;
                let reap = Instant::now() + Duration::from_secs(1);
                while child.try_wait()?.is_none() && Instant::now() < reap {
                    std::thread::sleep(Duration::from_millis(20));
                }
                return Err(io::Error::other("Истёк срок systemctl").into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn diagnostics(&self) {
        for unit in UNITS {
            if let Ok(output) = self.command(&[
                "show",
                unit,
                "--property=Id,ActiveState,SubState,MainPID,ControlGroup,Result,InvocationID",
            ]) {
                eprintln!("{}", String::from_utf8_lossy(&output.stdout));
            }
        }
    }
}

impl Drop for VmGate {
    fn drop(&mut self) {
        // При panic/error остаётся ограниченная уборка только фиксированных VM units.
        if let Err(error) = self.command(&[
            "stop",
            "yonik-worker.socket",
            "yonik-worker-check.socket",
            "yonik-worker.service",
            "yonik-worker-check.service",
            "yonik-helper.service",
        ]) {
            eprintln!("Уборка VM units не подтверждена: {error}");
        }
    }
}

fn installed_contract() -> Result {
    for (unit, expected) in [
        (
            UNITS[0],
            include_str!("../../../packaging/systemd/yonik-worker.service"),
        ),
        (
            UNITS[1],
            include_str!("../../../packaging/systemd/yonik-worker.socket"),
        ),
        (
            UNITS[2],
            include_str!("../../../packaging/systemd/yonik-worker-check.service"),
        ),
        (
            UNITS[3],
            include_str!("../../../packaging/systemd/yonik-worker-check.socket"),
        ),
    ] {
        ensure(
            bounded_read(format!("/usr/lib/systemd/system/{unit}"), 16 * 1024)? == expected,
            "Установленный unit отличается от packaging",
        )?;
    }
    Ok(())
}

fn payload(variable: &str, engine: Engine) -> Result<Arc<WorkerPayload>> {
    let path = PathBuf::from(
        std::env::var_os(variable)
            .ok_or_else(|| io::Error::other(format!("Требуется {variable}")))?,
    );
    ensure(
        path.is_absolute(),
        "Artifact path должен быть абсолютным VM path",
    )?;
    let mut source = File::open(&path)?;
    let mut hash = Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let length = source.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    let digest: Sha256 = format!("{:x}", hash.finalize()).parse()?;
    let binary = BinarySnapshot::load(&path, digest, engine)?;
    let server = yonik_import::import_bytes(b"socks5://127.0.0.1:10999".to_vec())?
        .servers
        .remove(0);
    let port = NonZeroU16::new(10808).ok_or_else(|| io::Error::other("Некорректный порт"))?;
    let config = yonik_engine::compile(&server, engine, port)?;
    Ok(Arc::new(WorkerPayload::new(binary, config)?))
}

fn group_pids() -> Result<Vec<u32>> {
    match bounded_read(format!("{GROUP}/cgroup.procs"), 65536) {
        Ok(text) => text
            .split_whitespace()
            .map(|pid| Ok(pid.parse()?))
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

fn empty_group(group: &str) -> Result<bool> {
    match bounded_read(format!("{group}/cgroup.events"), 4096) {
        Ok(text) => Ok(text.lines().any(|line| line == "populated 0")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}

async fn assert_gone(pids: &[u32]) -> Result {
    let deadline = tokio::time::Instant::now() + LIMIT;
    loop {
        let gone = pids
            .iter()
            .all(|pid| !Path::new(&format!("/proc/{pid}")).exists());
        if gone
            && group_pids()?.is_empty()
            && empty_group(GROUP)?
            && empty_group("/sys/fs/cgroup/system.slice/yonik-worker-check.service")?
        {
            return Ok(());
        }
        ensure(
            tokio::time::Instant::now() < deadline,
            "PID или populated cgroup остались после stop",
        )?;
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn assert_fence(gate: &VmGate, pid: u32) -> Result {
    let main = gate.command(&[
        "show",
        "yonik-worker.service",
        "--property=MainPID",
        "--value",
    ])?;
    ensure(
        String::from_utf8(main.stdout)?.trim().parse::<u32>()? == pid,
        "MainPID systemd не совпал с ticket",
    )?;
    let group = gate.command(&[
        "show",
        "yonik-worker.service",
        "--property=ControlGroup",
        "--value",
    ])?;
    ensure(
        String::from_utf8(group.stdout)?.trim() == "/system.slice/yonik-worker.service",
        "ControlGroup systemd не совпал с фактической cgroup",
    )?;
    let status = bounded_read(format!("/proc/{pid}/status"), 32768)?;
    let value = |key: &str| -> Result<&str> {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .map(str::trim)
            .ok_or_else(|| io::Error::other("Нет поля proc status").into())
    };
    ensure(
        value("Uid:")?.split_whitespace().all(|uid| uid != "0"),
        "Ядро работает от root",
    )?;
    for name in ["CapEff:", "CapBnd:", "CapAmb:"] {
        ensure(
            u64::from_str_radix(value(name)?, 16)? == 0x1000,
            "Лишние или отсутствующие capabilities",
        )?;
    }
    ensure(value("NoNewPrivs:")? == "1", "Отсутствует NoNewPrivileges")?;
    let executable = fs::read_link(format!("/proc/{pid}/exe"))?;
    ensure(
        executable
            .as_os_str()
            .as_encoded_bytes()
            .starts_with(b"/memfd:"),
        "Ядро не исполняется из memfd",
    )?;
    ensure(group_pids()?.contains(&pid), "Ядро вне ожидаемой cgroup")
}

async fn ready(pid: u32) -> Result {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        ensure(
            Path::new(&format!("/proc/{pid}")).exists(),
            "Ядро завершилось до listen",
        )?;
        if let Ok(Ok(stream)) = tokio::time::timeout(
            Duration::from_millis(200),
            tokio::net::TcpStream::connect("127.0.0.1:10808"),
        )
        .await
        {
            drop(stream);
            return Ok(());
        }
        ensure(
            tokio::time::Instant::now() < deadline,
            "SOCKS listen не появился",
        )?;
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn escaped_child(parent: u32) -> Result<Vec<u32>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let pids = group_pids()?;
        for pid in pids.iter().copied().filter(|pid| *pid != parent) {
            let stat = match bounded_read(format!("/proc/{pid}/stat"), 8192) {
                Ok(stat) => stat,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            // После comm идут state, ppid, pgrp, session; comm может содержать пробелы.
            let fields: Vec<_> = stat
                .rsplit_once(") ")
                .ok_or_else(|| io::Error::other("proc stat"))?
                .1
                .split_whitespace()
                .collect();
            if fields.get(2).is_some_and(|field| *field == pid.to_string())
                && fields.get(3).is_some_and(|field| *field == pid.to_string())
                && bounded_read(format!("/proc/{pid}/comm"), 128)?.trim() == "sleep"
            {
                ensure(
                    bounded_read(format!("/proc/{pid}/cgroup"), 8192)?
                        .lines()
                        .any(|line| line == "0::/system.slice/yonik-worker.service"),
                    "Дочерний процесс покинул cgroup",
                )?;
                return Ok(pids);
            }
        }
        ensure(
            tokio::time::Instant::now() < deadline,
            "Не появился session leader в cgroup",
        )?;
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[test]
fn refuses_without_explicit_vm_opt_in_before_effects() {
    assert!(vm_guard(None).is_err());
    assert!(vm_guard(Some(OsStr::new("0"))).is_err());
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "Только одноразовая root QEMU VM: установленный systemd worker и два canonical ELF"]
async fn installed_real_cores_check_start_stop() -> Result {
    vm_guard(std::env::var_os("YONIK_SYSTEMD_VM_GATE").as_deref())?;
    let _serial = SERIAL.lock().await;
    vm_guard(std::env::var_os("YONIK_SYSTEMD_VM_GATE").as_deref())?;
    let gate = VmGate::acquire()?;
    let result = async {
        installed_contract()?;
        gate.command(&["start", "yonik-helper.service"])?;
        let mut supervisor = SystemWorkerSupervisor::system().await?;
        let (_sender, mut cancel) = Cancellation::new();
        supervisor.recover(&mut cancel).await?;
        for (variable, engine) in [
            ("YONIK_GATE_XRAY", Engine::Xray),
            ("YONIK_GATE_SING_BOX", Engine::SingBox),
        ] {
            eprintln!("Проверка установленного worker: {engine:?}");
            let payload = payload(variable, engine)?;
            supervisor.check(Arc::clone(&payload), &mut cancel).await?;
            assert_gone(&[]).await?;
            let ticket = supervisor.start(payload, &mut cancel).await?;
            ready(ticket.pid()).await?;
            assert_fence(&gate, ticket.pid())?;
            ensure(
                supervisor.observe(&ticket).await? == WorkerLiveness::Running,
                "Нет Running",
            )?;
            let pids = group_pids()?;
            supervisor.stop(&ticket, &mut cancel).await?;
            assert_gone(&pids).await?;
            ensure(
                supervisor.stop(&ticket, &mut cancel).await == Err(WorkerSupervisorError::Stale),
                "Повторный ticket должен быть Stale после успешного stop",
            )?;
            assert_gone(&pids).await?;
            supervisor.recover(&mut cancel).await?;
        }
        Ok(())
    }
    .await;
    if result.is_err() {
        gate.diagnostics();
    }
    result
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "Только одноразовая root QEMU VM: fixture ELF, реальный KillMode и BindsTo"]
async fn installed_fixture_tree_and_helper_binds_to() -> Result {
    vm_guard(std::env::var_os("YONIK_SYSTEMD_VM_GATE").as_deref())?;
    let _serial = SERIAL.lock().await;
    vm_guard(std::env::var_os("YONIK_SYSTEMD_VM_GATE").as_deref())?;
    let gate = VmGate::acquire()?;
    let result = async {
        installed_contract()?;
        gate.command(&["start", "yonik-helper.service"])?;
        let mut supervisor = SystemWorkerSupervisor::system().await?;
        let (_sender, mut cancel) = Cancellation::new();
        supervisor.recover(&mut cancel).await?;
        let payload = payload("YONIK_GATE_FIXTURE", Engine::Xray)?;
        supervisor.check(Arc::clone(&payload), &mut cancel).await?;
        let ticket = supervisor.start(Arc::clone(&payload), &mut cancel).await?;
        let pids = escaped_child(ticket.pid()).await?;
        eprintln!("Fixture: main={}, cgroup PIDs={pids:?}", ticket.pid());
        assert_fence(&gate, ticket.pid())?;
        supervisor.stop(&ticket, &mut cancel).await?;
        assert_gone(&pids).await?;
        supervisor.recover(&mut cancel).await?;

        let ticket = supervisor.start(Arc::clone(&payload), &mut cancel).await?;
        let pids = escaped_child(ticket.pid()).await?;
        eprintln!("Fixture: main={}, cgroup PIDs={pids:?}", ticket.pid());
        gate.command(&[
            "kill",
            "--kill-whom=main",
            "--signal=KILL",
            "yonik-helper.service",
        ])?;
        let deadline = tokio::time::Instant::now() + LIMIT;
        loop {
            match supervisor.observe(&ticket).await {
                Ok(WorkerLiveness::Stopped) => break,
                Ok(WorkerLiveness::Running) | Err(WorkerSupervisorError::Stale) => {}
                Err(error) => return Err(error.into()),
            }
            ensure(
                tokio::time::Instant::now() < deadline,
                "BindsTo не остановил worker",
            )?;
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_gone(&pids).await?;
        for unit in UNITS {
            let output = gate.command(&["show", unit, "--property=ActiveState", "--value"])?;
            ensure(
                matches!(
                    String::from_utf8_lossy(&output.stdout).trim(),
                    "inactive" | "failed"
                ),
                "BindsTo оставил активный worker unit",
            )?;
        }
        supervisor.recover(&mut cancel).await?;
        gate.command(&["start", "yonik-helper.service"])?;
        let ticket = supervisor.start(payload, &mut cancel).await?;
        let pids = escaped_child(ticket.pid()).await?;
        eprintln!("Fixture: main={}, cgroup PIDs={pids:?}", ticket.pid());
        supervisor.stop(&ticket, &mut cancel).await?;
        assert_gone(&pids).await?;
        supervisor.recover(&mut cancel).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        gate.diagnostics();
    }
    result
}
