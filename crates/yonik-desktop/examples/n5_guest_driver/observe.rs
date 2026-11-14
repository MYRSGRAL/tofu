//! Actual kernel/process/network observations; вызываются только после sealed verifier.
use super::sealed::{Flow, Result, VerifiedFixture};
use rustix::{
    fs::{Mode, OFlags, fstatfs, open, openat},
    process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal},
};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::{Read, Write},
    net::{TcpStream, UdpSocket},
    os::fd::{AsRawFd, OwnedFd},
    os::unix::fs::MetadataExt,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
const IO: Duration = Duration::from_secs(2);
pub fn bounded(path: &str, limit: usize) -> Result<Vec<u8>> {
    let f = File::open(path).map_err(|_| "OBSERVATION_OPEN")?;
    let mut bytes = Vec::new();
    f.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "OBSERVATION_READ")?;
    if bytes.len() > limit {
        return Err("OBSERVATION_BOUND");
    }
    Ok(bytes)
}
pub fn start_time(text: &str) -> Result<u64> {
    text.rsplit_once(") ")
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .and_then(|n| n.parse().ok())
        .filter(|n| *n > 0)
        .ok_or("PROC_START")
}
fn proc_start(pid: u32) -> Result<u64> {
    start_time(
        std::str::from_utf8(&bounded(&format!("/proc/{pid}/stat"), 4096)?)
            .map_err(|_| "PROC_UTF8")?,
    )
}
fn pidfd(pid: u32) -> Result<OwnedFd> {
    pidfd_open(
        Pid::from_raw(pid as i32).ok_or("PID")?,
        PidfdFlags::NONBLOCK,
    )
    .map_err(|_| "PIDFD")
}
fn worker_dead(fd: &OwnedFd) -> Result<bool> {
    let text = bounded(&format!("/proc/self/fdinfo/{}", fd.as_raw_fd()), 4096)?;
    let text = std::str::from_utf8(&text).map_err(|_| "PIDFD_INFO")?;
    Ok(text.lines().any(|line| line == "Pid:\t-1"))
}
fn cgroup_empty() -> Result<bool> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut dir = File::from(open("/", flags, Mode::empty()).map_err(|_| "CGROUP_ROOT")?);
    for name in ["sys", "fs", "cgroup", "system.slice"] {
        let m = dir.metadata().map_err(|_| "CGROUP_STAT")?;
        if m.uid() != 0 || m.gid() != 0 || m.mode() & 0o022 != 0 {
            return Err("CGROUP_TRUST");
        }
        dir = File::from(openat(&dir, name, flags, Mode::empty()).map_err(|_| "CGROUP_PARENT")?);
        if name == "cgroup" && fstatfs(&dir).map_err(|_| "CGROUP_FS")?.f_type as u64 != 0x63677270 {
            return Err("CGROUP_FS");
        }
    }
    let m = dir.metadata().map_err(|_| "CGROUP_STAT")?;
    if m.uid() != 0 || m.gid() != 0 || m.mode() & 0o022 != 0 {
        return Err("CGROUP_TRUST");
    }
    let group = match openat(&dir, "yonik-worker.service", flags, Mode::empty()) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(true),
        Err(_) => return Err("CGROUP_OPEN"),
    };
    let m = group.metadata().map_err(|_| "CGROUP_STAT")?;
    if m.uid() != 0 || m.gid() != 0 || m.mode() & 0o022 != 0 {
        return Err("CGROUP_TRUST");
    }
    let events = File::from(
        openat(
            &group,
            "cgroup.events",
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| "CGROUP_EVENTS")?,
    );
    let mut contents = String::new();
    events
        .take(4097)
        .read_to_string(&mut contents)
        .map_err(|_| "CGROUP_READ")?;
    empty_events(&contents)
}
pub fn empty_events(contents: &str) -> Result<bool> {
    if contents.len() > 4096 || !contents.ends_with('\n') {
        return Err("CGROUP_PROTOCOL");
    }
    let mut populated = None;
    for line in contents.lines() {
        let (key, value) = line.split_once(' ').ok_or("CGROUP_PROTOCOL")?;
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err("CGROUP_PROTOCOL");
        }
        if key == "populated" {
            if populated.is_some() || !["0", "1"].contains(&value) {
                return Err("CGROUP_PROTOCOL");
            }
            populated = Some(value == "0");
        }
    }
    populated.ok_or("MISSING_POPULATED")
}
pub struct Baseline {
    worker: Vec<OwnedFd>,
}
impl Baseline {
    pub fn capture(_f: &VerifiedFixture) -> Result<Self> {
        Ok(Self { worker: Vec::new() })
    }
    pub fn bind_worker(&mut self, fixture: &VerifiedFixture) -> Result<Value> {
        let members = bounded(
            "/sys/fs/cgroup/system.slice/yonik-worker.service/cgroup.procs",
            16384,
        )?;
        let members = std::str::from_utf8(&members).map_err(|_| "CGROUP_MEMBERS")?;
        let mut receipts = Vec::new();
        for line in members.lines() {
            let pid: u32 = line.parse().map_err(|_| "CGROUP_PID")?;
            let start = proc_start(pid)?;
            let fd = pidfd(pid)?;
            if start != proc_start(pid)?
                || bounded(&format!("/proc/{pid}/cgroup"), 4096)?
                    != b"0::/system.slice/yonik-worker.service\n"
            {
                return Err("WORKER_REUSED");
            }
            let uid = fs::metadata(format!("/proc/{pid}"))
                .map_err(|_| "WORKER_UID")?
                .uid();
            if uid != fixture.worker_uid()? {
                return Err("WORKER_UID");
            }
            self.worker.push(fd);
            receipts.push(json!({"pid":pid,"start":start,"uid":uid,"cgroup":"/system.slice/yonik-worker.service"}));
        }
        if receipts.is_empty() {
            return Err("MISSING_WORKER");
        }
        Ok(
            json!({"canonical_cgroup_members":receipts,"executing_elf_binding":"SOURCE_INVARIANT_FROM_TRUSTED_UNIT_AND_INSTALLED_TUPLE"}),
        )
    }
    pub fn cleanup(&self) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if cgroup_empty()?
                && !self.worker.is_empty()
                && self
                    .worker
                    .iter()
                    .map(worker_dead)
                    .collect::<Result<Vec<_>>>()?
                    .iter()
                    .all(|v| *v)
            {
                break;
            }
            if Instant::now() >= deadline {
                return Err("RECOVERY_REQUIRED_NONEMPTY_OR_LIVE");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(json!({"worker_pidfd_dead":true,"cgroup_populated":0}))
    }
}

// Два concrete executable: этот ELF и fixed Python. Никакого shell/общего executor.
struct OwnedChild {
    child: Child,
    fd: OwnedFd,
    reaped: bool,
}
impl OwnedChild {
    fn wait(&mut self) -> Result<()> {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().map_err(|_| "CHILD_WAIT")? {
                self.reaped = true;
                return if status.success() {
                    Ok(())
                } else {
                    Err("FLOW_CHILD_FAILED")
                };
            }
            if Instant::now() >= end {
                return Err("FLOW_CHILD_DEADLINE");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = pidfd_send_signal(&self.fd, Signal::KILL);
            let _ = self.child.wait();
        }
    }
}
async fn read_tcp_frame<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    deadline: tokio::time::Instant,
    limit: usize,
) -> Result<Vec<u8>> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    let mut frame = Vec::new();
    let mut bounded = BufReader::new(reader.take(limit as u64 + 2));
    tokio::time::timeout_at(deadline, bounded.read_until(b'\n', &mut frame))
        .await
        .map_err(|_| "TCP_READ_TIMEOUT")?
        .map_err(|_| "TCP_READ")?;
    if frame.len() > limit + 1 || frame.last() != Some(&b'\n') {
        return Err("TCP_FRAME");
    }
    frame.pop();
    Ok(frame)
}
fn read_observer_frame(mut read: impl FnMut(&mut [u8]) -> Result<usize>) -> Result<Vec<u8>> {
    let mut frame = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        let count = read(&mut chunk)?;
        if count == 0 {
            return Err("FLOW_CAPTURE_TRUNCATED");
        }
        frame.extend_from_slice(&chunk[..count]);
        if frame.len() > 4096 {
            return Err("FLOW_CAPTURE_BOUND");
        }
        if let Some(index) = frame.iter().position(|byte| *byte == b'\n') {
            if index != frame.len() - 1 {
                return Err("FLOW_CAPTURE_TRAILING");
            }
            frame.pop();
            return Ok(frame);
        }
    }
}
pub fn flow_child(flow: &Flow, nonce: &str) -> Result<Value> {
    let payload = format!("{}:{nonce}", flow.token);
    let mut buffer = [0u8; 1024];
    let (local, result) = if flow.transport == "tcp" {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "FLOW_RUNTIME")?;
        return runtime.block_on(async {
            use tokio::io::AsyncWriteExt;
            let deadline = tokio::time::Instant::now() + IO;
            let socket = if flow.endpoint.is_ipv4() {
                tokio::net::TcpSocket::new_v4()
            } else {
                tokio::net::TcpSocket::new_v6()
            }.map_err(|_| "TCP_SOCKET")?;
            socket.bind(std::net::SocketAddr::new(flow.source_ip, 0)).map_err(|_| "TCP_BIND")?;
            let local = socket.local_addr().map_err(|_| "TCP_LOCAL")?;
            let mut socket = match tokio::time::timeout_at(deadline, socket.connect(flow.endpoint)).await {
                Ok(Ok(socket)) => socket,
                Err(_) => return Ok(json!({"local":local.to_string(),"remote":flow.endpoint.to_string(),"transport":"tcp","received":false,"reply":""})),
                _ => return Err("TCP_CONNECT"),
            };
            let framed = format!("{payload}\n");
            tokio::time::timeout_at(deadline, socket.write_all(framed.as_bytes())).await.map_err(|_| "TCP_WRITE_TIMEOUT")?.map_err(|_| "TCP_WRITE")?;
            let (received, reply) = match read_tcp_frame(&mut socket, deadline, 1024).await {
                Ok(bytes) => (true, String::from_utf8(bytes).map_err(|_| "FLOW_UTF8")?),
                Err("TCP_READ_TIMEOUT") => (false, String::new()),
                Err(error) => return Err(error),
            };
            Ok(json!({"local":local.to_string(),"remote":flow.endpoint.to_string(),"transport":"tcp","received":received,"reply":reply}))
        });
    } else {
        let socket = UdpSocket::bind(std::net::SocketAddr::new(flow.source_ip, 0))
            .map_err(|_| "UDP_BIND")?;
        socket.connect(flow.endpoint).map_err(|_| "UDP_CONNECT")?;
        socket
            .set_read_timeout(Some(IO))
            .map_err(|_| "SOCKET_TIMEOUT")?;
        socket.send(payload.as_bytes()).map_err(|_| "FLOW_SEND")?;
        (
            socket.local_addr().map_err(|_| "SOCKET_LOCAL")?,
            socket.recv(&mut buffer),
        )
    };
    let (received, reply) = match result {
        Ok(n) => (
            true,
            String::from_utf8(buffer[..n].to_vec()).map_err(|_| "FLOW_UTF8")?,
        ),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            (false, String::new())
        }
        Err(_) => return Err("FLOW_RECEIVE"),
    };
    Ok(
        json!({"local":local.to_string(),"remote":flow.endpoint.to_string(),"transport":flow.transport,"received":received,"reply":reply}),
    )
}
pub fn run_flows(f: &VerifiedFixture) -> Result<Value> {
    let mut results = Vec::new();
    for flow in &f.controls().flows {
        let mut nonce = [0u8; 16];
        getrandom::getrandom(&mut nonce).map_err(|_| "FLOW_NONCE")?;
        let nonce = super::hash::hex(&nonce);
        let mut command = if flow.application == "rust" {
            let mut c = Command::new(super::sealed::DRIVER);
            c.arg("--flow-child=n5-v1");
            c
        } else {
            let mut c = Command::new("/usr/lib/yonik-acceptance/n5-python");
            c.args(["-I", "/usr/lib/yonik-acceptance/n5-flow-client.py"]);
            c
        };
        command
            .env_clear()
            .env("DBUS_SESSION_BUS_ADDRESS", &f.controls().private_bus)
            .env(
                "XDG_RUNTIME_DIR",
                format!("/run/user/{}", f.controls().caller_uid),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|_| "FLOW_SPAWN")?;
        let pid = child.id();
        let fd = match pidfd(pid) {
            Ok(fd) => fd,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let mut owned = OwnedChild {
            child,
            fd,
            reaped: false,
        };
        let start = proc_start(pid)?;
        let executable = fs::read_link(format!("/proc/{pid}/exe")).map_err(|_| "CHILD_EXE")?;
        let expected = if flow.application == "rust" {
            super::sealed::DRIVER
        } else {
            "/usr/lib/yonik-acceptance/n5-python"
        };
        // Python distro может canonicalize versioned executable; compare identity.
        let actual = fs::metadata(format!("/proc/{pid}/exe")).map_err(|_| "CHILD_EXE")?;
        let pinned = fs::metadata(expected).map_err(|_| "CHILD_EXE")?;
        if actual.dev() != pinned.dev()
            || actual.ino() != pinned.ino()
            || fs::metadata(format!("/proc/{pid}"))
                .map_err(|_| "CHILD_UID")?
                .uid()
                != f.controls().caller_uid
        {
            return Err("CHILD_IDENTITY");
        }
        let input = json!({"flow":flow,"nonce":nonce});
        let mut stdin = owned.child.stdin.take().ok_or("CHILD_STDIN")?;
        writeln!(stdin, "{input}").map_err(|_| "CHILD_WRITE")?;
        drop(stdin);
        // Child output bounded by both implementations; wait before read cannot grow pipe.
        owned.wait()?;
        let mut output = Vec::new();
        owned
            .child
            .stdout
            .take()
            .ok_or("CHILD_STDOUT")?
            .take(4097)
            .read_to_end(&mut output)
            .map_err(|_| "CHILD_READ")?;
        if output.len() > 4096 {
            return Err("CHILD_OUTPUT_BOUND");
        }
        let client = super::protocol::strict_json(&output)?;
        let observer_deadline = Instant::now() + IO;
        let mut observer = TcpStream::connect_timeout(&f.controls().flow_observer, IO)
            .map_err(|_| "FLOW_OBSERVER")?;
        observer
            .set_read_timeout(Some(IO))
            .map_err(|_| "FLOW_OBSERVER")?;
        observer
            .set_write_timeout(Some(
                observer_deadline
                    .checked_duration_since(Instant::now())
                    .ok_or("FLOW_OBSERVER_TIMEOUT")?,
            ))
            .map_err(|_| "FLOW_OBSERVER")?;
        writeln!(observer,"{}",json!({"token":format!("{}:{nonce}",flow.token),"transport":flow.transport,"endpoint":flow.endpoint.to_string()})).map_err(|_|"FLOW_OBSERVER")?;
        let capture = read_observer_frame(|buffer| {
            let remaining = observer_deadline
                .checked_duration_since(Instant::now())
                .ok_or("FLOW_OBSERVER_TIMEOUT")?;
            observer
                .set_read_timeout(Some(remaining))
                .map_err(|_| "FLOW_OBSERVER")?;
            observer.read(buffer).map_err(|_| "FLOW_OBSERVER")
        })?;
        let capture = super::protocol::strict_json(&capture)?;
        validate_flow(flow, &nonce, &client, &capture)?;
        results.push(json!({"class":"ACTUAL","flow":flow,"pid":pid,"start":start,"uid":f.controls().caller_uid,"executable":executable,"client":client,"endpoint_capture":capture,"child_reaped":true}));
    }
    Ok(json!(results))
}
pub fn validate_flow(flow: &Flow, nonce: &str, client: &Value, capture: &Value) -> Result<()> {
    if capture["token"] != format!("{}:{nonce}", flow.token)
        || capture["transport"] != flow.transport
        || capture["endpoint"] != flow.endpoint.to_string()
        || client["remote"] != flow.endpoint.to_string()
        || client["transport"] != flow.transport
        || client["local"]
            .as_str()
            .and_then(|v| v.parse::<std::net::SocketAddr>().ok())
            .is_none()
    {
        return Err("FLOW_BINDING");
    }
    let seen = capture["seen"].as_u64().ok_or("FLOW_COUNTER")?;
    if flow.action == "block" {
        if seen != 0 || client["received"] != false {
            return Err("BLOCK_REACHED_ENDPOINT");
        }
    } else if seen != 1
        || client["received"] != true
        || client["reply"] != format!("{}:{nonce}:{}", flow.token, flow.response)
        || capture["server"] != flow.response
        || capture["peer_ip"] != flow.expected_peer_ip.to_string()
    {
        return Err("FLOW_PATH_OR_RESPONSE");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cgroup_zero_is_exact_not_inactive() {
        assert_eq!(empty_events("populated 0\nfrozen 0\n"), Ok(true));
        assert_eq!(empty_events("populated 1\n"), Ok(false));
        assert!(empty_events("populated 0\npopulated 0\n").is_err());
        assert!(empty_events("inactive\n").is_err());
    }
    #[test]
    fn proc_comm_parentheses_do_not_shift_start() {
        let fields = format!("S {} 123", vec!["0"; 18].join(" "));
        assert_eq!(start_time(&format!("7 (a ) b) {fields}")), Ok(123));
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    fn flow(action: &str) -> Flow {
        serde_json::from_value(json!({"endpoint":"10.0.0.4:100","source_ip":"10.0.0.2","expected_peer_ip":"10.0.0.3","transport":"tcp","application":"rust","action":action,"token":"token","response":"proxy-endpoint","control":false})).unwrap()
    }
    #[test]
    fn endpoint_echo_does_not_replace_route_observation() {
        let f = flow("proxy");
        let client = json!({"local":"10.0.0.2:200","remote":"10.0.0.4:100","transport":"tcp","received":true,"reply":"token:n:proxy-endpoint"});
        let mut capture = json!({"token":"token:n","transport":"tcp","endpoint":"10.0.0.4:100","seen":1,"server":"proxy-endpoint","peer_ip":"10.0.0.3"});
        assert_eq!(validate_flow(&f, "n", &client, &capture), Ok(()));
        capture["peer_ip"] = json!("10.0.0.2");
        assert!(validate_flow(&f, "n", &client, &capture).is_err());
    }
    #[test]
    fn client_timeout_alone_cannot_prove_block() {
        let f = flow("block");
        let client = json!({"local":"10.0.0.2:200","remote":"10.0.0.4:100","transport":"tcp","received":false});
        assert!(validate_flow(&f, "n", &client, &Value::Null).is_err());
        let capture =
            json!({"token":"token:n","transport":"tcp","endpoint":"10.0.0.4:100","seen":1});
        assert!(validate_flow(&f, "n", &client, &capture).is_err());
    }
}

#[cfg(test)]
mod framing_tests {
    use super::*;
    #[test]
    fn observer_accepts_split_frame_and_rejects_truncated_frame() {
        let mut chunks = [b"{\"seen\":".as_slice(), b"0}\n".as_slice()].into_iter();
        let result = read_observer_frame(|buffer| {
            let part = chunks.next().unwrap_or_default();
            buffer[..part.len()].copy_from_slice(part);
            Ok(part.len())
        });
        assert_eq!(result, Ok(b"{\"seen\":0}".to_vec()));
        assert!(read_observer_frame(|_| Ok(0)).is_err());
        assert!(
            read_observer_frame(|buffer| {
                buffer.fill(b'x');
                Ok(buffer.len())
            })
            .is_err()
        );
    }
    #[tokio::test]
    async fn tcp_response_frame_supports_fragmented_memory_stream() {
        use tokio::io::AsyncWriteExt;
        let (mut sender, mut receiver) = tokio::io::duplex(64);
        let writer = tokio::spawn(async move {
            sender.write_all(b"token:n:").await.unwrap();
            tokio::task::yield_now().await;
            sender.write_all(b"server\n").await.unwrap();
        });
        let frame = read_tcp_frame(&mut receiver, tokio::time::Instant::now() + IO, 1024).await;
        assert_eq!(frame, Ok(b"token:n:server".to_vec()));
        writer.await.unwrap();
        let mut partial = b"partial".as_slice();
        assert!(
            read_tcp_frame(&mut partial, tokio::time::Instant::now() + IO, 1024)
                .await
                .is_err()
        );
        let mut oversized = b"oversized\n".as_slice();
        assert!(
            read_tcp_frame(&mut oversized, tokio::time::Instant::now() + IO, 2)
                .await
                .is_err()
        );
    }
}
