//! Отдельный opt-in gate: один контролируемый процесс в собственном /proc.
use super::{
    DEADLINE, Flow, Instant, MAX_FDS_PER_TASK, Ordering, TrustedContext, Unavailable, WITNESSES,
    capture, fs,
};
use std::time::Duration;

#[test]
#[ignore = "требует изолированных PID/mount/network namespaces и поднятого lo"]
fn isolated_cold_wire_known() {
    use std::net::{TcpListener, TcpStream, UdpSocket};
    use std::os::fd::OwnedFd;
    assert_eq!(std::process::id(), 1, "fixture обязан стать PID 1");
    let pids: Vec<_> = fs::read_dir("/proc")
        .unwrap()
        .filter_map(|e| e.unwrap().file_name().to_str()?.parse::<u32>().ok())
        .collect();
    assert_eq!(pids, [1], "посторонние процессы недопустимы");
    for address in ["127.0.0.1:0", "[::1]:0"] {
        for protocol in [6, 17] {
            let (client, _peer, local, remote): (OwnedFd, OwnedFd, _, _) = if protocol == 6 {
                let listener = TcpListener::bind(address).unwrap();
                let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
                let (peer, _) = listener.accept().unwrap();
                let local = client.local_addr().unwrap();
                let remote = client.peer_addr().unwrap();
                (client.into(), peer.into(), local, remote)
            } else {
                let client = UdpSocket::bind(address).unwrap();
                let peer = UdpSocket::bind(address).unwrap();
                client.connect(peer.local_addr().unwrap()).unwrap();
                let local = client.local_addr().unwrap();
                let remote = peer.local_addr().unwrap();
                (client.into(), peer.into(), local, remote)
            };
            let mut f = Flow {
                family: 2,
                protocol,
                local_port: local.port(),
                remote_port: remote.port(),
                local: [0; 16],
                remote: [0; 16],
                cookie: None,
            };
            match (local.ip(), remote.ip()) {
                (std::net::IpAddr::V4(a), std::net::IpAddr::V4(b)) => {
                    f.local[..4].copy_from_slice(&a.octets());
                    f.remote[..4].copy_from_slice(&b.octets());
                }
                (std::net::IpAddr::V6(a), std::net::IpAddr::V6(b)) => {
                    f.family = 10;
                    f.local = a.octets();
                    f.remote = b.octets();
                }
                _ => unreachable!(),
            }
            let client = std::sync::Arc::new(client);
            let retained = std::sync::Arc::clone(&client);
            let uid = rustix::process::getuid().as_raw();
            let broker = super::super::Broker::new(
                TrustedContext::from_host(1, uid, uid, 1, [3; 32], [2; 32]).unwrap(),
            )
            .unwrap();
            crate::attribution::native_session::cold_roundtrip(
                broker,
                move |broker| {
                    assert!(broker.cache.is_empty());
                    assert!(broker.last_cold.is_none());
                    WITNESSES.with(|n| n.set(0));
                    // Никакой подстановки snapshot: serve вызывает настоящий cold capture.
                    let _ = &client;
                    Ok(())
                },
                f,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
            drop(retained);
            eprintln!(
                "cold wire Known family={} protocol={protocol} cookie=None",
                f.family
            );
        }
    }
}

#[test]
#[ignore = "требует изолированных PID/mount/network namespaces и поднятого lo"]
fn isolated_cold_limit_cancel() {
    assert_eq!(std::process::id(), 1);
    let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    client.connect(peer.local_addr().unwrap()).unwrap();
    let f = Flow {
        family: 2,
        protocol: 17,
        local_port: client.local_addr().unwrap().port(),
        remote_port: peer.local_addr().unwrap().port(),
        local: [127, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        remote: [127, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        cookie: None,
    };
    let uid = rustix::process::getuid().as_raw();
    let context = TrustedContext::from_host(1, uid, uid, 1, [3; 32], [2; 32]).unwrap();
    let held: Vec<_> = (0..=MAX_FDS_PER_TASK)
        .map(|_| fs::File::open("/dev/null").unwrap())
        .collect();
    WITNESSES.with(|n| n.set(0));
    let begun = Instant::now();
    assert!(matches!(capture(&context, f), Err(Unavailable::Limit)));
    assert_eq!(WITNESSES.with(|n| n.get()), 1);
    assert!(begun.elapsed() < DEADLINE);
    eprintln!(
        "negative FD Limit witnesses=1 elapsed={:?} held={}",
        begun.elapsed(),
        held.len()
    );
    drop(held);
    context.cancelled.store(true, Ordering::Release);
    WITNESSES.with(|n| n.set(0));
    assert!(matches!(capture(&context, f), Err(Unavailable::Cancelled)));
    assert_eq!(WITNESSES.with(|n| n.get()), 0);
}
