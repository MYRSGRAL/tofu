//! Только одноразовая root VM с настоящими systemd и cgroup2.
//! Проверяет production SystemWorkerSupervisor, но TUN и VLESS peer находятся в разных netns.
//! Клиент запускается только установленным production systemd worker.
#![cfg(target_os = "linux")]

use sha2::{Digest, Sha256 as Hasher};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, Read, Seek, SeekFrom, Write},
    num::NonZeroU16,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use yonik_engine::{Engine, SystemTun, SystemTunAddress, compile_system};
use yonik_linux::{
    BinarySnapshot, Cancellation, Sha256,
    helper::{SystemWorkerSupervisor, WorkerLiveness},
    managed_network::{
        Journal, Ownership,
        tun::{TunAddress, TunExecutor, TunRoute, TunRule},
    },
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
    capture: bool,
    management: bool,
    veth: bool,
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
    let current_ns = fs::metadata("/proc/thread-self/ns/net")?;
    let init_ns = fs::metadata("/proc/1/ns/net")?;
    ensure(
        current_ns.dev() == init_ns.dev() && current_ns.ino() == init_ns.ino(),
        "Gate должен исполняться в initial guest netns systemd",
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
        self.run("/usr/bin/systemctl", args)
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        // Повторная read-only проверка непосредственно перед каждым subprocess effect.
        vm_guard(std::env::var_os("YONIK_SYSTEMD_VM_GATE").as_deref())?;
        // Regular memfd не ждёт EOF от унаследованного write-end, в отличие
        // от pipe/wait_with_output. Чтение ограничено независимо от потомков.
        let mut stdout = capture_file()?;
        let mut stderr = capture_file()?;
        let mut child = Command::new(program)
            .args(args)
            .env_clear()
            .env("LANG", "C")
            .env("SYSTEMD_PAGER", "cat")
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout.try_clone()?))
            .stderr(Stdio::from(stderr.try_clone()?))
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(20);
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
                return Err(io::Error::other("Истёк срок fixture subprocess").into());
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
        if self.capture || self.veth || self.management {
            // В Drop нет async ownership reconciliation. Ни priority, ни имя
            // интерфейса не разрешают DEL; неизвестное сохраняет VM для destroy.
            eprintln!("Сетевой cleanup fixture не подтверждён; требуется уничтожить VM");
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

struct Process(Child);
impl Process {
    fn finish(&mut self) -> Result {
        drop(self.0.stdin.take());
        let deadline = Instant::now() + LIMIT;
        loop {
            if let Some(status) = self.0.try_wait()? {
                return ensure(status.success(), "Peer завершился с ошибкой");
            }
            ensure(Instant::now() < deadline, "Peer не завершился после EOF")?;
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!("Уборка peer: {error}");
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
fn ip(gate: &VmGate, args: &[&str]) -> Result<serde_json::Value> {
    let mut arguments = vec!["-j"];
    arguments.extend(args);
    let output = gate.run("/usr/bin/ip", &arguments)?;
    if output.stdout.is_empty() {
        Ok(serde_json::Value::Null)
    } else {
        Ok(serde_json::from_slice(&output.stdout)?)
    }
}
fn peer_config(engine: Engine) -> serde_json::Value {
    use serde_json::json;
    match engine {
        Engine::Xray => {
            json!({"log":{"loglevel":"none"},"inbounds":[{"listen":"127.0.0.1","port":24443,"protocol":"vless","settings":{"decryption":"none","clients":[{"id":"00112233-4455-6677-8899-aabbccddeeff"}]}}],"outbounds":[{"tag":"echo4","protocol":"freedom","settings":{"redirect":"127.0.0.1:24081","finalRules":[{"action":"allow","ip":["127.0.0.1/32"],"port":"24081"}]}},{"tag":"echo6","protocol":"freedom","settings":{"redirect":"[::1]:24081","finalRules":[{"action":"allow","ip":["::1/128"],"port":"24081"}]}}],"routing":{"rules":[{"type":"field","ip":["198.18.0.0/30"],"outboundTag":"echo4"},{"type":"field","ip":["2001:db8:746f::/126"],"outboundTag":"echo6"}]}})
        }
        Engine::SingBox => {
            json!({"log":{"disabled":true},"inbounds":[{"type":"vless","listen":"127.0.0.1","listen_port":24443,"users":[{"uuid":"00112233-4455-6677-8899-aabbccddeeff"}]}],"outbounds":[{"type":"direct","tag":"echo"}],"route":{"rules":[{"ip_cidr":["198.18.0.0/30"],"action":"route","outbound":"echo","override_address":"127.0.0.1","override_port":24081},{"ip_cidr":["2001:db8:746f::/126"],"action":"route","outbound":"echo","override_address":"::1","override_port":24081}]}})
        }
    }
}
async fn peer_signal(peer: &mut Process, expected: &str) -> Result {
    let output = peer
        .0
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Нет peer stdout"))?;
    let read = tokio::task::spawn_blocking(move || {
        let mut output = std::io::BufReader::new(output);
        let mut line = String::new();
        output.read_line(&mut line)?;
        Ok::<_, io::Error>((output.into_inner(), line))
    });
    let (output, line) = tokio::time::timeout(LIMIT, read).await???;
    peer.0.stdout = Some(output);
    ensure(line.trim() == expected, "Неожиданный ответ peer")
}

async fn remote_peer(
    gate: &mut VmGate,
    binary: &Path,
    config: &Path,
    tools: &Path,
) -> Result<Process> {
    // VLESS peer находится за veth: local rule не может скрыть отсутствие mark.
    let script = format!(
        r#"import sys, subprocess, socket, time
sys.path.insert(0, {tools:?})
from traffic_echo import echo_servers
print('namespace', flush=True)
assert sys.stdin.buffer.readline() == b'go\n'
def ip(*args):
 subprocess.run(['/usr/bin/ip', *args], check=True, timeout=5, stdout=subprocess.DEVNULL)
ip('link', 'set', 'lo', 'up')
ip('address', 'add', '10.203.0.2/30', 'dev', 'yonik-inner')
ip('-6', 'address', 'add', '2001:db8:7470::2/126', 'dev', 'yonik-inner', 'nodad')
ip('link', 'set', 'yonik-inner', 'up')
with echo_servers():
 with subprocess.Popen([{binary:?}, 'run', '-c', {config:?}], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) as core:
  try:
   deadline = time.monotonic() + 5
   while True:
    if core.poll() is not None: raise RuntimeError('VLESS peer stopped')
    try:
     with socket.create_connection(('127.0.0.1', 24443), timeout=0.1): break
    except OSError:
     if time.monotonic() >= deadline: raise TimeoutError('VLESS peer readiness')
     time.sleep(0.02)
   print('ready', flush=True)
   sys.stdin.buffer.read()
  finally:
   core.kill()
   core.wait(timeout=5)
"#
    );
    let mut peer = Process(
        Command::new("/usr/bin/unshare")
            .args(["--net", "/usr/bin/python3", "-u", "-c", &script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    peer_signal(&mut peer, "namespace").await?;
    ip(
        gate,
        &[
            "link",
            "add",
            "yonik-outer",
            "type",
            "veth",
            "peer",
            "name",
            "yonik-inner",
        ],
    )?;
    gate.veth = true;
    ip(
        gate,
        &[
            "link",
            "set",
            "yonik-inner",
            "netns",
            &peer.0.id().to_string(),
        ],
    )?;
    ip(
        gate,
        &["address", "add", "10.203.0.1/30", "dev", "yonik-outer"],
    )?;
    ip(
        gate,
        &[
            "-6",
            "address",
            "add",
            "2001:db8:7470::1/126",
            "dev",
            "yonik-outer",
            "nodad",
        ],
    )?;
    ip(gate, &["link", "set", "yonik-outer", "up"])?;
    peer.0
        .stdin
        .as_mut()
        .ok_or_else(|| io::Error::other("Нет peer stdin"))?
        .write_all(b"go\n")?;
    peer_signal(&mut peer, "ready").await?;
    Ok(peer)
}

fn artifact(variable: &str, expected: &str) -> Result<PathBuf> {
    let path = PathBuf::from(
        std::env::var_os(variable).ok_or_else(|| io::Error::other(variable.to_owned()))?,
    );
    ensure(
        path == Path::new(expected),
        "Artifact должен быть staged в /opt/yonik-gate",
    )?;
    let metadata = fs::symlink_metadata(&path)?;
    ensure(
        metadata.is_file() && metadata.uid() == 0 && metadata.mode() & 0o022 == 0,
        "Небезопасный staged artifact",
    )?;
    Ok(path)
}
fn payload(binary: &Path, engine: Engine) -> Result<Arc<WorkerPayload>> {
    let mut source = File::open(binary)?;
    let mut hash = Hasher::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let length = source.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    let digest: Sha256 = format!("{:x}", hash.finalize()).parse()?;
    let binary = BinarySnapshot::load(binary, digest, engine)?;
    let report = yonik_import::import_bytes(
        b"vless://00112233-4455-6677-8899-aabbccddeeff@10.203.0.2:24443".to_vec(),
    )?;
    let tun = SystemTun::new(
        1,
        1500,
        99,
        vec![
            "198.18.0.1/30".parse::<SystemTunAddress>()?,
            "2001:db8:746f::1/126".parse()?,
        ],
    )?;
    let config = compile_system(
        &report.servers[0],
        engine,
        NonZeroU16::new(10808).ok_or_else(|| io::Error::other("Неверный порт"))?,
        &tun,
    )?;
    Ok(Arc::new(WorkerPayload::new(binary, config)?))
}
fn global_addresses(gate: &VmGate) -> Result<Vec<serde_json::Value>> {
    let data = ip(gate, &["address", "show", "dev", "tofu00000001"])?;
    let addresses = data[0]["addr_info"]
        .as_array()
        .ok_or_else(|| io::Error::other("Нет адресов TUN"))?;
    Ok(addresses
        .iter()
        .filter(|address| address["scope"] == "global")
        .cloned()
        .collect())
}
fn probe(gate: &VmGate, tools: &Path, negative: bool) -> Result {
    let action = if negative {
        "  try: probe(family,address,kind)\n  except OSError: continue\n  raise RuntimeError('Proxy bypass')\n"
    } else {
        "  before=counters()\n  probe(family,address,kind)\n  after=counters()\n  assert all(end > start for start,end in zip(before,after)), 'Нет пакетов через TUN'\n"
    };
    let script = format!(
        "import sys,socket,subprocess,json\nsys.path.insert(0,{tools:?})\nfrom engine_traffic import probe\ndef counters():\n data=json.loads(subprocess.check_output(['/usr/bin/ip','-j','-s','link','show','dev','tofu00000001'],timeout=5))[0]['stats64']\n return data['rx']['packets'],data['tx']['packets']\nfor family,address in [(socket.AF_INET,'198.18.0.2'),(socket.AF_INET6,'2001:db8:746f::2')]:\n for kind in (socket.SOCK_STREAM,socket.SOCK_DGRAM):\n{action}"
    );
    gate.run("/usr/bin/python3", &["-c", &script])?;
    Ok(())
}
async fn cleanup(executor: &mut TunExecutor, journal: &mut Journal) -> Result {
    // Порядок важен: сначала освобождаем route capture, затем дочерние ресурсы TUN.
    for id in [7, 6, 5, 4, 3, 2, 1] {
        if !journal.records().iter().any(|record| record.id() == id) {
            continue;
        }
        match id {
            6 | 7 => executor.remove_rule(journal, 1, id).await?,
            4 | 5 => executor.remove_route(journal, 1, id).await?,
            2 | 3 => executor.remove_address(journal, 1, id).await?,
            _ => executor.remove_tun(journal, 1).await?,
        }
    }
    ensure(journal.records().is_empty(), "Журнал не пуст после cleanup")
}
async fn session(
    gate: &mut VmGate,
    binary: &Path,
    engine: Engine,
    tools: &Path,
    area: &Path,
) -> Result {
    let ns = File::open("/proc/thread-self/ns/net")?.metadata()?;
    let owner = Ownership::new(0, 7, 9, ns.dev(), ns.ino())?;
    let mut executor = TunExecutor::open(owner)?;
    let mut journal = Journal::open(&area.join("network"), owner)?;
    let mut supervisor = SystemWorkerSupervisor::system().await?;
    let (_sender, mut cancel) = Cancellation::new();
    supervisor.recover(&mut cancel).await?;
    let result: Result = async {
        executor.ensure_tun(&mut journal, 1, 1).await?;
        executor.configure_link(&mut journal, 1, 1500).await?;
        for (id, address, prefix) in [(2, "198.18.0.1", 30), (3, "2001:db8:746f::1", 126)] {
            executor
                .ensure_address(
                    &mut journal,
                    1,
                    id,
                    TunAddress::new(address.parse()?, prefix)?,
                )
                .await?;
        }
        for (id, address) in [(4, "0.0.0.0"), (5, "::")] {
            executor
                .ensure_route(
                    &mut journal,
                    1,
                    id,
                    TunRoute::new(address.parse()?, 0, 1001, 100)?,
                )
                .await?;
        }
        let before_addresses = global_addresses(gate)?;
        let peer_path = area.join("peer.json");
        let mut value = peer_config(engine);
        value["inbounds"][0]["listen"] = serde_json::json!("0.0.0.0");
        fs::write(&peer_path, serde_json::to_vec(&value)?)?;
        let mut peer = remote_peer(gate, binary, &peer_path, tools).await?;
        let payload = payload(binary, engine)?;
        supervisor.check(Arc::clone(&payload), &mut cancel).await?;
        assert_gone(&[]).await?;
        gate.capture = true;
        for (id, ipv6) in [(6, false), (7, true)] {
            executor
                .ensure_rule(&mut journal, 1, id, TunRule::new(10000, 1001, 99, ipv6)?)
                .await?;
        }
        for (family, target) in [("-4", "10.203.0.2"), ("-6", "2001:db8:7470::2")] {
            ensure(
                ip(gate, &[family, "route", "get", target])?[0]["dev"] == "tofu00000001",
                "Нет capture маршрута в TUN",
            )?;
            ensure(
                ip(gate, &[family, "route", "get", target, "mark", "99"])?[0]["dev"]
                    == "yonik-outer",
                "Mark не обходит TUN",
            )?;
        }
        let ticket = supervisor.start(payload, &mut cancel).await?;
        ready(ticket.pid()).await?;
        assert_fence(gate, ticket.pid())?;
        ensure(
            fs::metadata(format!("/proc/{}/ns/net", ticket.pid()))?.ino() == ns.ino(),
            "Worker в другом netns",
        )?;
        ensure(
            supervisor.observe(&ticket).await? == WorkerLiveness::Running,
            "Нет Running",
        )?;
        for (family, target) in [("-4", "198.18.0.2"), ("-6", "2001:db8:746f::2")] {
            ensure(
                ip(gate, &[family, "route", "get", target])?[0]["dev"] == "tofu00000001",
                "Echo обходит TUN",
            )?;
        }
        ensure(
            global_addresses(gate)? == before_addresses,
            "Worker изменил глобальные адреса TUN",
        )?;
        for family in ["-4", "-6"] {
            let routes = ip(
                gate,
                &[
                    family,
                    "route",
                    "show",
                    "table",
                    "main",
                    "dev",
                    "tofu00000001",
                ],
            )?;
            ensure(
                routes
                    .as_array()
                    .is_some_and(|routes| routes.iter().all(|route| route["dst"] == "fe80::/64")),
                "Worker добавил main route",
            )?;
        }
        probe(gate, tools, false)?;
        eprintln!(
            "{engine:?}: TCP/UDP IPv4/IPv6 через installed worker TUN→VLESS→echo подтверждены"
        );
        peer.finish()?;
        probe(gate, tools, true)?;
        let pids = group_pids()?;
        supervisor.stop(&ticket, &mut cancel).await?;
        assert_gone(&pids).await?;
        supervisor.recover(&mut cancel).await?;
        Ok(())
    }
    .await;
    // Даже при ошибке сначала останавливаем системный worker, затем снимаем сеть.
    let recovered = supervisor.recover(&mut cancel).await;
    let stopped = assert_gone(&[]).await;
    if recovered.is_err() || stopped.is_err() {
        return Err(
            io::Error::other("Worker cleanup не подтверждён; VM должна быть уничтожена").into(),
        );
    }
    let cleaned = cleanup(&mut executor, &mut journal).await;
    if cleaned.is_ok() {
        gate.capture = false;
    }
    if gate.veth {
        let links = ip(gate, &["link", "show"])?;
        if links
            .as_array()
            .is_some_and(|links| links.iter().any(|link| link["ifname"] == "yonik-outer"))
        {
            return Err(
                io::Error::other("Peer netns оставила veth; требуется уничтожить VM").into(),
            );
        }
        gate.veth = false;
    }
    cleaned?;
    result
}

#[test]
fn refuses_without_explicit_vm_opt_in_before_effects() {
    assert!(vm_guard(None).is_err());
    assert!(vm_guard(Some(OsStr::new("0"))).is_err());
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "Только disposable root QEMU VM с установленными units и двумя staged ELF"]
async fn installed_real_cores_cross_owned_tun_and_proxy() -> Result {
    vm_guard(std::env::var_os("YONIK_SYSTEMD_VM_GATE").as_deref())?;
    let mut gate = VmGate {
        _lock: vm_lock::acquire()?,
        capture: false,
        management: false,
        veth: false,
    };
    installed_contract()?;
    let xray = artifact("YONIK_GATE_XRAY", "/opt/yonik-gate/xray")?;
    let sing_box = artifact("YONIK_GATE_SING_BOX", "/opt/yonik-gate/sing-box")?;
    let tools = PathBuf::from(
        std::env::var_os("YONIK_GATE_TOOLS")
            .ok_or_else(|| io::Error::other("Требуется YONIK_GATE_TOOLS"))?,
    );
    ensure(
        tools == Path::new("/opt/yonik-gate/tools"),
        "Неверный tools path",
    )?;
    // Read-only evidence до первого сетевого эффекта; фиксируем namespace и конфликты.
    let initial = ip(&gate, &["link", "show"])?;
    ensure(
        !initial
            .as_array()
            .ok_or_else(|| io::Error::other("Нет links"))?
            .iter()
            .any(|link| {
                ["yonik-outer", "yonik-inner", "tofu00000001"]
                    .iter()
                    .any(|name| link["ifname"] == *name)
            }),
        "Fixture интерфейс уже существует",
    )?;
    for family in ["-4", "-6"] {
        let rules = ip(&gate, &[family, "rule", "show"])?;
        ensure(
            !rules
                .as_array()
                .ok_or_else(|| io::Error::other("Нет rules"))?
                .iter()
                .any(|rule| rule["priority"] == 1000 || rule["priority"] == 10000),
            "Fixture rule priority занят",
        )?;
        ensure(
            ip(&gate, &[family, "route", "show", "table", "all"])?
                .as_array()
                .is_some_and(|routes| routes.iter().all(|route| route["table"] != 1001)),
            "Fixture table 1001 занят",
        )?;
    }
    eprintln!("VM guard подтверждён до effects; initial links={initial}");
    let area = Path::new("/run/yonik-installed-tun-gate");
    fs::DirBuilder::new().mode(0o700).create(area)?;
    // Исключение только для SSH management одноразовой QEMU fixture.
    ip(
        &gate,
        &[
            "-4",
            "rule",
            "add",
            "pref",
            "1000",
            "to",
            "10.0.2.0/24",
            "lookup",
            "main",
            "protocol",
            "253",
        ],
    )?;
    gate.management = true;
    gate.command(&["start", "yonik-helper.service"])?;
    let result: Result = async {
        for (engine, binary) in [(Engine::Xray, xray), (Engine::SingBox, sing_box)] {
            session(&mut gate, &binary, engine, &tools, area).await?;
        }
        Ok(())
    }
    .await;
    if result.is_err() {
        gate.diagnostics();
    }
    if result.is_ok() {
        ip(
            &gate,
            &[
                "-4",
                "rule",
                "del",
                "pref",
                "1000",
                "to",
                "10.0.2.0/24",
                "lookup",
                "main",
                "protocol",
                "253",
            ],
        )?;
        gate.management = false;
        eprintln!("Оба engine проверены; worker cgroups пусты, capture снят, journal пуст");
    }
    result
}
