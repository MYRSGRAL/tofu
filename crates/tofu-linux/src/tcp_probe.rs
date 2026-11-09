//! Отменяемая проверка TCP без proxy handshake и изменения системной сети.
use std::{
    collections::{HashSet, VecDeque},
    fmt,
    future::{Future, poll_fn},
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::Path,
    pin::Pin,
    task::Poll,
    time::Duration,
};

use serde::Deserialize;
use tofu_import::{ParsedServer, Transport};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UnixStream},
    time::{Instant, sleep_until},
};

use crate::{Cancellation, HostError};

const RESOLVER: &str = "/run/systemd/resolve/io.systemd.Resolve";
const MAX_REPLY: usize = 64 * 1024;
const MAX_ADDRESSES: usize = 64;
const DIAL_DELAY: Duration = Duration::from_millis(200);

/// Неизменяемый снимок endpoint; credentials и исходная URI не копируются.
pub struct TcpProbeTarget {
    host: String,
    port: u16,
    transport: Transport,
}
impl TcpProbeTarget {
    pub fn for_server(server: &ParsedServer) -> Self {
        Self {
            host: server.endpoint().host().to_owned(),
            port: server.endpoint().port(),
            transport: server.transport(),
        }
    }
}
impl fmt::Debug for TcpProbeTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TcpProbeTarget(<скрыто>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpProbeStatus {
    Reachable,
    NotApplicable,
    ResolverUnavailable,
    DnsFailure,
    ConnectFailure,
    DnsTimeout,
    ConnectTimeout,
    Cancelled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpProbeResult {
    pub status: TcpProbeStatus,
    pub dns_elapsed: Option<Duration>,
    pub connect_elapsed: Option<Duration>,
}

pub async fn check_tcp(
    target: TcpProbeTarget,
    timeout: Duration,
    cancel: &mut Cancellation,
) -> Result<TcpProbeResult, HostError> {
    check_with_resolver(target, timeout, cancel, Path::new(RESOLVER), true).await
}

async fn check_with_resolver(
    target: TcpProbeTarget,
    timeout: Duration,
    cancel: &mut Cancellation,
    resolver: &Path,
    production: bool,
) -> Result<TcpProbeResult, HostError> {
    if timeout.is_zero() || timeout > Duration::from_secs(60) {
        return Err(HostError::ProbeTarget);
    }
    let mut result = TcpProbeResult {
        status: TcpProbeStatus::NotApplicable,
        dns_elapsed: None,
        connect_elapsed: None,
    };
    if matches!(target.transport, Transport::Quic | Transport::Unsupported) {
        return Ok(result);
    }
    if cancel.check().is_err() {
        result.status = TcpProbeStatus::Cancelled;
        return Ok(result);
    }
    let deadline = Instant::now() + timeout;
    let addresses = if let Ok(ip) = target.host.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, target.port)]
    } else {
        let start = Instant::now();
        let resolved = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(TcpProbeStatus::Cancelled),
            _ = sleep_until(deadline) => Err(TcpProbeStatus::DnsTimeout),
            resolved = resolve(&target.host, target.port, resolver, production) => resolved,
        };
        result.dns_elapsed = Some(start.elapsed());
        match resolved {
            Ok(addresses) => addresses,
            Err(status) => {
                result.status = status;
                return Ok(result);
            }
        }
    };
    let start = Instant::now();
    result.status = connect_phase(addresses, deadline, cancel).await;
    result.connect_elapsed = Some(start.elapsed());
    Ok(result)
}

async fn connect_phase(
    addresses: Vec<SocketAddr>,
    deadline: Instant,
    cancel: &mut Cancellation,
) -> TcpProbeStatus {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => TcpProbeStatus::Cancelled,
        _ = sleep_until(deadline) => TcpProbeStatus::ConnectTimeout,
        connected = dial(addresses) => if connected {
            TcpProbeStatus::Reachable
        } else {
            TcpProbeStatus::ConnectFailure
        },
    }
}

// Каталоги защищены root-цепочкой; resolved вправе владеть своим каталогом и socket.
fn resolver_metadata(path: &Path, production: bool) -> io::Result<std::fs::Metadata> {
    let socket = std::fs::symlink_metadata(path)?;
    if !socket.file_type().is_socket() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    if production {
        if path != Path::new(RESOLVER) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        for directory in ["/", "/run", "/run/systemd", "/run/systemd/resolve"] {
            let metadata = std::fs::symlink_metadata(directory)?;
            let owner_ok = metadata.uid() == 0
                || (directory == "/run/systemd/resolve" && metadata.uid() == socket.uid());
            if !metadata.file_type().is_dir() || metadata.mode() & 0o022 != 0 || !owner_ok {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
        }
    }
    Ok(socket)
}

async fn resolve(
    host: &str,
    port: u16,
    path: &Path,
    production: bool,
) -> Result<Vec<SocketAddr>, TcpProbeStatus> {
    let unavailable = |_| TcpProbeStatus::ResolverUnavailable;
    let metadata = resolver_metadata(path, production).map_err(unavailable)?;
    let mut socket = UnixStream::connect(path).await.map_err(unavailable)?;
    let current = resolver_metadata(path, production).map_err(unavailable)?;
    if current.dev() != metadata.dev()
        || current.ino() != metadata.ino()
        || socket.peer_cred().map_err(unavailable)?.uid() != metadata.uid()
    {
        return Err(TcpProbeStatus::ResolverUnavailable);
    }
    let mut request = serde_json::to_vec(&serde_json::json!({
        "method": "io.systemd.Resolve.ResolveHostname",
        "parameters": {"name": host}
    }))
    .map_err(|_| TcpProbeStatus::DnsFailure)?;
    request.push(0);
    socket
        .write_all(&request)
        .await
        .map_err(|_| TcpProbeStatus::DnsFailure)?;
    let mut reply = Vec::with_capacity(1024);
    let mut buffer = [0; 1024];
    loop {
        let count = socket
            .read(&mut buffer)
            .await
            .map_err(|_| TcpProbeStatus::DnsFailure)?;
        if count == 0 {
            return Err(TcpProbeStatus::DnsFailure);
        }
        let end = buffer[..count].iter().position(|byte| *byte == 0);
        let payload = end.unwrap_or(count);
        if reply.len() + payload > MAX_REPLY {
            return Err(TcpProbeStatus::DnsFailure);
        }
        reply.extend_from_slice(&buffer[..payload]);
        if let Some(end) = end {
            if end + 1 != count {
                return Err(TcpProbeStatus::DnsFailure);
            }
            return parse_reply(&reply, port);
        }
    }
}

#[derive(Deserialize)]
struct Reply {
    parameters: Option<Answer>,
    error: Option<serde_json::Value>,
    continues: Option<bool>,
}
#[derive(Deserialize)]
struct Answer {
    addresses: Vec<Address>,
}
#[derive(Deserialize)]
struct Address {
    family: u32,
    address: Vec<u8>,
    ifindex: Option<u32>,
}
fn parse_reply(reply: &[u8], port: u16) -> Result<Vec<SocketAddr>, TcpProbeStatus> {
    let failure = || TcpProbeStatus::DnsFailure;
    let reply: Reply = serde_json::from_slice(reply).map_err(|_| failure())?;
    if reply.error.is_some() || reply.continues == Some(true) {
        return Err(failure());
    }
    let addresses = reply.parameters.ok_or_else(failure)?.addresses;
    if addresses.is_empty() || addresses.len() > MAX_ADDRESSES {
        return Err(failure());
    }
    let mut seen = HashSet::with_capacity(addresses.len());
    let mut result = Vec::with_capacity(addresses.len());
    for address in addresses {
        let socket = match address.family {
            2 => {
                let octets: [u8; 4] = address.address.try_into().map_err(|_| failure())?;
                SocketAddr::new(Ipv4Addr::from(octets).into(), port)
            }
            10 => {
                let octets: [u8; 16] = address.address.try_into().map_err(|_| failure())?;
                let ip = Ipv6Addr::from(octets);
                let scope = if ip.is_unicast_link_local() {
                    address
                        .ifindex
                        .filter(|scope| *scope != 0)
                        .ok_or_else(failure)?
                } else {
                    0
                };
                SocketAddr::V6(SocketAddrV6::new(ip, port, 0, scope))
            }
            _ => return Err(failure()),
        };
        if seen.insert(socket) {
            result.push(socket);
        }
    }
    Ok(result)
}

fn interleave(addresses: Vec<SocketAddr>) -> VecDeque<SocketAddr> {
    let first_v6 = addresses.first().is_some_and(SocketAddr::is_ipv6);
    let (mut v6, mut v4): (VecDeque<_>, VecDeque<_>) =
        addresses.into_iter().partition(SocketAddr::is_ipv6);
    let mut result = VecDeque::with_capacity(v6.len() + v4.len());
    while !v6.is_empty() || !v4.is_empty() {
        let pair = if first_v6 {
            [&mut v6, &mut v4]
        } else {
            [&mut v4, &mut v6]
        };
        for queue in pair {
            if let Some(address) = queue.pop_front() {
                result.push_back(address);
            }
        }
    }
    result
}

type Dial = Pin<Box<dyn Future<Output = io::Result<TcpStream>> + Send>>;

async fn dial(addresses: Vec<SocketAddr>) -> bool {
    dial_with(addresses, |address| Box::pin(TcpStream::connect(address))).await
}

async fn dial_with(
    addresses: Vec<SocketAddr>,
    mut connect: impl FnMut(SocketAddr) -> Dial,
) -> bool {
    let mut waiting = interleave(addresses);
    let mut attempts: [Option<Dial>; 2] = [None, None];
    let Some(first) = waiting.pop_front() else {
        return false;
    };
    attempts[0] = Some(connect(first));
    let mut next_start = Instant::now() + DIAL_DELAY;
    loop {
        let can_start = !waiting.is_empty() && attempts.iter().any(Option::is_none);
        let finished = tokio::select! {
            finished = poll_fn(|cx| {
                for (index, attempt) in attempts.iter_mut().enumerate() {
                    if let Some(attempt) = attempt {
                        if let Poll::Ready(result) = attempt.as_mut().poll(cx) {
                            // Успешный TCP socket закрывается здесь без записи данных.
                            return Poll::Ready((index, result.is_ok()));
                        }
                    }
                }
                Poll::Pending
            }) => Some(finished),
            _ = sleep_until(next_start), if can_start => None,
        };
        if let Some((index, success)) = finished {
            attempts[index] = None;
            if success {
                return true;
            }
        }
        // Ошибка сразу освобождает слот; blackhole занимает максимум один из двух.
        if let Some(slot) = attempts.iter_mut().find(|attempt| attempt.is_none()) {
            if let Some(address) = waiting.pop_front() {
                *slot = Some(connect(address));
                next_start = Instant::now() + DIAL_DELAY;
            }
        }
        if attempts.iter().all(Option::is_none) {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Cancellation, Dial, Duration, HostError, Instant, MAX_ADDRESSES, MAX_REPLY, Path,
        SocketAddr, TcpProbeStatus, TcpProbeTarget, TcpStream, Transport, UnixStream, check_tcp,
        check_with_resolver, connect_phase, dial_with, interleave, parse_reply, resolve,
    };
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, UnixListener};

    fn target(host: &str, port: u16) -> TcpProbeTarget {
        TcpProbeTarget {
            host: host.into(),
            port,
            transport: Transport::Tcp,
        }
    }

    struct ResolverFixture {
        directory: PathBuf,
        path: PathBuf,
        listener: UnixListener,
    }
    impl ResolverFixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "tofu-tcp-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            let path = directory.join("resolve");
            let listener = UnixListener::bind(&path).unwrap();
            Self {
                directory,
                path,
                listener,
            }
        }
    }
    impl Drop for ResolverFixture {
        fn drop(&mut self) {
            std::fs::remove_file(&self.path).unwrap();
            std::fs::remove_dir(&self.directory).unwrap();
        }
    }

    async fn read_request(socket: &mut UnixStream) {
        let mut request = Vec::new();
        loop {
            let byte = socket.read_u8().await.unwrap();
            if byte == 0 {
                break;
            }
            request.push(byte);
        }
        let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
        assert_eq!(request["method"], "io.systemd.Resolve.ResolveHostname");
        assert_eq!(request["parameters"]["name"], "private.invalid");
    }

    #[tokio::test]
    async fn literal_reachable_closes_socket_without_payload() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (_source, mut cancel) = Cancellation::new();
        let client = check_with_resolver(
            target("127.0.0.1", port),
            Duration::from_secs(1),
            &mut cancel,
            Path::new("/missing/resolver"),
            true,
        );
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert_eq!(socket.read(&mut [0; 1]).await.unwrap(), 0);
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(client, server)
        })
        .await
        .unwrap();
        let result = result.unwrap();
        assert_eq!(result.status, TcpProbeStatus::Reachable);
        assert!(result.dns_elapsed.is_none());
        assert!(result.connect_elapsed.is_some());
    }

    #[tokio::test]
    async fn refused_and_invalid_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let (_source, mut cancel) = Cancellation::new();
        assert_eq!(
            check_tcp(
                target("127.0.0.1", port),
                Duration::from_secs(1),
                &mut cancel
            )
            .await
            .unwrap()
            .status,
            TcpProbeStatus::ConnectFailure
        );
        for timeout in [Duration::ZERO, Duration::from_secs(61)] {
            assert_eq!(
                check_tcp(target("127.0.0.1", port), timeout, &mut cancel).await,
                Err(HostError::ProbeTarget)
            );
        }
    }

    #[tokio::test]
    async fn udp_and_unsupported_do_no_io() {
        let fixture = ResolverFixture::new();
        let (source, mut cancel) = Cancellation::new();
        source.send(true).unwrap();
        for transport in [Transport::Quic, Transport::Unsupported] {
            let mut target = target("private.invalid", 443);
            target.transport = transport;
            let result = check_with_resolver(
                target,
                Duration::from_secs(1),
                &mut cancel,
                &fixture.path,
                false,
            )
            .await
            .unwrap();
            assert_eq!(result.status, TcpProbeStatus::NotApplicable);
            assert_eq!(result.dns_elapsed, None);
            assert_eq!(result.connect_elapsed, None);
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(10), fixture.listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn cancelled_dns_closes_unix_socket() {
        let fixture = ResolverFixture::new();
        let (source, mut cancel) = Cancellation::new();
        let client = check_with_resolver(
            target("private.invalid", 443),
            Duration::from_secs(1),
            &mut cancel,
            &fixture.path,
            false,
        );
        let server = async {
            let (mut socket, _) = fixture.listener.accept().await.unwrap();
            read_request(&mut socket).await;
            source.send(true).unwrap();
            assert_eq!(socket.read(&mut [0; 1]).await.unwrap(), 0);
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(client, server)
        })
        .await
        .unwrap();
        let result = result.unwrap();
        assert_eq!(result.status, TcpProbeStatus::Cancelled);
        assert!(result.dns_elapsed.is_some());
        assert!(result.connect_elapsed.is_none());
    }

    #[tokio::test]
    async fn dns_timeout_closes_unix_socket() {
        let fixture = ResolverFixture::new();
        let (_source, mut cancel) = Cancellation::new();
        let client = check_with_resolver(
            target("private.invalid", 443),
            Duration::from_millis(30),
            &mut cancel,
            &fixture.path,
            false,
        );
        let server = async {
            let (mut socket, _) = fixture.listener.accept().await.unwrap();
            read_request(&mut socket).await;
            assert_eq!(socket.read(&mut [0; 1]).await.unwrap(), 0);
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(client, server)
        })
        .await
        .unwrap();
        assert_eq!(result.unwrap().status, TcpProbeStatus::DnsTimeout);
    }

    #[tokio::test]
    async fn unavailable_and_untrusted_resolver_are_distinct() {
        let (_source, mut cancel) = Cancellation::new();
        let fixture = ResolverFixture::new();
        for (path, production) in [
            (Path::new("/nonexistent/tofu-resolver"), false),
            (fixture.path.as_path(), true),
        ] {
            assert_eq!(
                check_with_resolver(
                    target("private.invalid", 443),
                    Duration::from_secs(1),
                    &mut cancel,
                    path,
                    production
                )
                .await
                .unwrap()
                .status,
                TcpProbeStatus::ResolverUnavailable
            );
        }
    }

    #[tokio::test]
    async fn fake_dns_to_loopback() {
        let fixture = ResolverFixture::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (_source, mut cancel) = Cancellation::new();
        let client = check_with_resolver(
            target("private.invalid", listener.local_addr().unwrap().port()),
            Duration::from_secs(1),
            &mut cancel,
            &fixture.path,
            false,
        );
        let server = async {
            let (mut socket, _) = fixture.listener.accept().await.unwrap();
            read_request(&mut socket).await;
            socket
                .write_all(
                    b"{\"parameters\":{\"addresses\":[{\"family\":2,\"address\":[127,0,0,1]}]}}\0",
                )
                .await
                .unwrap();
            let (mut tcp, _) = listener.accept().await.unwrap();
            assert_eq!(tcp.read(&mut [0; 1]).await.unwrap(), 0);
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(client, server)
        })
        .await
        .unwrap();
        let result = result.unwrap();
        assert_eq!(result.status, TcpProbeStatus::Reachable);
        assert!(result.dns_elapsed.is_some());
        assert!(result.connect_elapsed.is_some());
    }

    #[tokio::test]
    async fn oversized_and_malformed_reply() {
        for reply in [vec![b'x'; MAX_REPLY + 1], b"invalid\0".to_vec()] {
            let fixture = ResolverFixture::new();
            let client = resolve("private.invalid", 443, &fixture.path, false);
            let server = async {
                let (mut socket, _) = fixture.listener.accept().await.unwrap();
                read_request(&mut socket).await;
                socket.write_all(&reply).await.unwrap();
            };
            let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
                tokio::join!(client, server)
            })
            .await
            .unwrap();
            assert_eq!(result, Err(TcpProbeStatus::DnsFailure));
        }
    }

    #[test]
    fn rejects_empty_malformed_and_too_many_records() {
        for reply in [
            b"{}".as_slice(),
            br#"{"parameters":{"addresses":[]}}"#,
            br#"{"parameters":{"addresses":[{"family":2,"address":[256,0,0,1]}]}}"#,
            br#"{"error":"io.systemd.Resolve.NoSuchResourceRecord"}"#,
        ] {
            assert_eq!(parse_reply(reply, 443), Err(TcpProbeStatus::DnsFailure));
        }
        let addresses =
            vec![serde_json::json!({"family":2,"address":[127,0,0,1]}); MAX_ADDRESSES + 1];
        let reply =
            serde_json::to_vec(&serde_json::json!({"parameters":{"addresses":addresses}})).unwrap();
        assert_eq!(parse_reply(&reply, 443), Err(TcpProbeStatus::DnsFailure));
    }

    #[test]
    fn scoped_ipv6_deduplicates_by_ip_and_scope() {
        let link_local = [254, 128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let reply = serde_json::to_vec(&serde_json::json!({"parameters":{"addresses":[
            {"family":10,"address":link_local,"ifindex":2},
            {"family":10,"address":link_local,"ifindex":2},
            {"family":10,"address":link_local,"ifindex":3},
            {"family":2,"address":[127,0,0,1]},
            {"family":2,"address":[127,0,0,1],"ifindex":2}
        ]}}))
        .unwrap();
        let result = parse_reply(&reply, 443).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "[fe80::1%2]:443".parse().unwrap());
        assert_eq!(result[1], "[fe80::1%3]:443".parse().unwrap());
        let missing_scope = serde_json::to_vec(
            &serde_json::json!({"parameters":{"addresses":[{"family":10,"address":link_local}]}}),
        )
        .unwrap();
        assert_eq!(
            parse_reply(&missing_scope, 443),
            Err(TcpProbeStatus::DnsFailure)
        );
    }

    #[test]
    fn families_interleave_and_debug_is_redacted() {
        let addresses = ["[::1]:443", "[::2]:443", "127.0.0.1:443", "127.0.0.2:443"]
            .map(|address| address.parse().unwrap());
        let result: Vec<_> = interleave(addresses.to_vec()).into_iter().collect();
        assert_eq!(
            result,
            [addresses[0], addresses[2], addresses[1], addresses[3]]
        );
        assert_eq!(
            format!("{:?}", target("secret.invalid", 443)),
            "TcpProbeTarget(<скрыто>)"
        );
    }
    #[tokio::test]
    async fn connect_deadline_and_cancellation_have_separate_statuses() {
        let (source, mut cancel) = Cancellation::new();
        let deadline = Instant::now();
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(
            connect_phase(vec!["127.0.0.1:1".parse().unwrap()], deadline, &mut cancel).await,
            TcpProbeStatus::ConnectTimeout
        );
        source.send(true).unwrap();
        assert_eq!(
            connect_phase(
                vec!["127.0.0.1:1".parse().unwrap()],
                Instant::now() + Duration::from_secs(1),
                &mut cancel
            )
            .await,
            TcpProbeStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn pending_ipv6_does_not_hide_ipv4_and_all_attempts_are_dropped() {
        use std::sync::Arc;
        struct Active(Arc<AtomicU64>);
        impl Drop for Active {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::Relaxed);
            }
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ipv4 = listener.local_addr().unwrap();
        let active = Arc::new(AtomicU64::new(0));
        let counts = active.clone();
        let connect = move |address: SocketAddr| -> Dial {
            let count = counts.fetch_add(1, Ordering::Relaxed) + 1;
            assert!(count <= 2);
            let guard = Active(counts.clone());
            Box::pin(async move {
                let _guard = guard;
                if address.is_ipv6() {
                    std::future::pending::<()>().await;
                }
                TcpStream::connect(address).await
            })
        };
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            dial_with(
                vec!["[::1]:1".parse().unwrap(), "[::2]:1".parse().unwrap(), ipv4],
                connect,
            ),
        )
        .await
        .unwrap();
        assert!(result);
        assert_eq!(active.load(Ordering::Relaxed), 0);
        let (mut socket, _) = listener.accept().await.unwrap();
        assert_eq!(socket.read(&mut [0; 1]).await.unwrap(), 0);
    }
}
