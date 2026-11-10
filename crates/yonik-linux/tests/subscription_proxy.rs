//! Проверка явного SOCKS пути без обращений к исходному listener.
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::watch,
};
use yonik_linux::{FetchError, FetchHost, FetchResponse, FetchRoute, Source};

async fn socks(listener: TcpListener, target: SocketAddr, response: &str) -> String {
    let (mut stream, _) = listener.accept().await.expect("SOCKS accept");
    let mut greeting = [0; 3];
    stream.read_exact(&mut greeting).await.expect("greeting");
    assert_eq!(greeting, [5, 1, 0]);
    stream.write_all(&[5, 0]).await.expect("no auth");
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix).await.expect("CONNECT");
    let expected_ip = match target.ip() {
        std::net::IpAddr::V4(ip) => {
            assert_eq!(prefix, [5, 1, 0, 1]);
            ip.octets().to_vec()
        }
        std::net::IpAddr::V6(ip) => {
            assert_eq!(prefix, [5, 1, 0, 4]);
            ip.octets().to_vec()
        }
    };
    let mut address = vec![0; expected_ip.len()];
    stream.read_exact(&mut address).await.expect("IP");
    assert_eq!(address, expected_ip);
    assert_eq!(stream.read_u16().await.expect("port"), target.port());
    stream
        .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
        .await
        .expect("connected");
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        request.push(stream.read_u8().await.expect("HTTP"));
        assert!(request.len() < 32768);
    }
    stream
        .write_all(response.as_bytes())
        .await
        .expect("response");
    String::from_utf8(request).expect("HTTP ASCII")
}

#[tokio::test]
async fn explicit_proxy_routes_http_without_direct_connection() {
    let direct = TcpListener::bind("127.0.0.1:0").await.expect("direct");
    let target = direct.local_addr().expect("target");
    let proxy = TcpListener::bind("127.0.0.1:0").await.expect("proxy");
    let endpoint = proxy.local_addr().expect("endpoint");
    let source = Source::local(&format!("http://{target}/secret")).expect("source");
    let fixture = tokio::spawn(async move {
        socks(
            proxy,
            target,
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody",
        )
        .await
    });
    let (_sender, cancel) = watch::channel(false);
    let host = FetchHost::new(Duration::from_secs(2))
        .expect("host")
        .with_socks_endpoint(endpoint)
        .expect("endpoint");
    let response = host
        .fetch(&source, FetchRoute::Proxy, None, cancel)
        .await
        .expect("proxy fetch");
    let FetchResponse::Modified { body, .. } = response else {
        panic!("200")
    };
    assert_eq!(body.expose(), b"body");
    assert!(
        fixture
            .await
            .expect("fixture")
            .starts_with("GET /secret HTTP/1.1")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), direct.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn proxy_failures_never_fall_back() {
    let direct = TcpListener::bind("127.0.0.1:0").await.expect("direct");
    let source =
        Source::local(&format!("http://{}/", direct.local_addr().expect("addr"))).expect("source");
    for mode in 0..4 {
        let proxy = TcpListener::bind("127.0.0.1:0").await.expect("proxy");
        let endpoint = proxy.local_addr().expect("endpoint");
        let (sender, cancel) = watch::channel(false);
        let signal = sender.clone();
        let task = if mode == 0 {
            drop(proxy);
            None
        } else {
            Some(tokio::spawn(async move {
                let (mut stream, _) = proxy.accept().await.expect("accept");
                if mode == 1 {
                    let mut greeting = [0; 3];
                    stream.read_exact(&mut greeting).await.expect("greeting");
                    stream.write_all(&[5, 2]).await.expect("auth required");
                } else if mode == 3 {
                    signal.send(true).expect("cancel");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                } else {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }))
        };
        let host = FetchHost::new(Duration::from_millis(80))
            .expect("host")
            .with_socks_endpoint(endpoint)
            .expect("endpoint");
        let error = host
            .fetch(&source, FetchRoute::Proxy, None, cancel)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            match mode {
                2 => FetchError::Timeout,
                3 => FetchError::Cancelled,
                _ => FetchError::Transport,
            }
        );
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), direct.accept())
            .await
            .is_err()
    );
}

#[test]
fn endpoint_must_be_loopback_and_nonzero() {
    for endpoint in ["8.8.8.8:1080", "0.0.0.0:1080", "127.0.0.1:0"] {
        assert!(
            FetchHost::new(Duration::from_secs(1))
                .expect("host")
                .with_socks_endpoint(endpoint.parse().expect("addr"))
                .is_err()
        );
    }
}

#[tokio::test]
async fn proxy_redirect_preserves_hwid_origin_boundary() {
    let direct = TcpListener::bind("127.0.0.1:0").await.expect("direct");
    let target = direct.local_addr().expect("target");
    let proxy = TcpListener::bind("127.0.0.1:0").await.expect("proxy");
    let endpoint = proxy.local_addr().expect("endpoint");
    let source = Source::local(&format!("http://{target}/secret"))
        .expect("source")
        .with_hwid("test-hwid-1234")
        .expect("hwid");
    let fixture = tokio::spawn(async move {
        socks(proxy, target, "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await
    });
    let (_sender, cancel) = watch::channel(false);
    let host = FetchHost::new(Duration::from_secs(2))
        .expect("host")
        .with_socks_endpoint(endpoint)
        .expect("endpoint");
    assert_eq!(
        host.fetch(&source, FetchRoute::Proxy, None, cancel)
            .await
            .unwrap_err(),
        FetchError::HwidRedirect
    );
    assert!(
        fixture
            .await
            .expect("fixture")
            .contains("x-hwid: test-hwid-1234")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), direct.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn public_proxy_cannot_reach_private_destination() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.expect("proxy");
    let host = FetchHost::new(Duration::from_secs(1))
        .expect("host")
        .with_socks_endpoint(proxy.local_addr().expect("addr"))
        .expect("endpoint");
    let source = Source::public("https://127.0.0.1/secret").expect("source");
    let (_sender, cancel) = watch::channel(false);
    assert_eq!(
        host.fetch(&source, FetchRoute::Proxy, None, cancel)
            .await
            .unwrap_err(),
        FetchError::BlockedAddress
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), proxy.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn proxy_domain_guard_no_system_dns() {
    let source = Source::local("http://fixture-only.subscription.test/sub").expect("source");
    let host = FetchHost::new(Duration::from_millis(200))
        .expect("host")
        .with_socks_endpoint("127.0.0.1:9".parse().expect("endpoint"))
        .expect("endpoint");
    let (_sender, cancel) = watch::channel(false);
    assert_eq!(
        host.fetch(&source, FetchRoute::Proxy, None, cancel)
            .await
            .unwrap_err(),
        FetchError::ProxyDnsRequired
    );
}

async fn socks_connect(listener: &TcpListener, target: SocketAddr) -> tokio::net::TcpStream {
    let (mut stream, _) = listener.accept().await.expect("SOCKS accept");
    let mut greeting = [0; 3];
    stream.read_exact(&mut greeting).await.expect("greeting");
    assert_eq!(greeting, [5, 1, 0]);
    stream.write_all(&[5, 0]).await.expect("no auth");
    let mut connect = [0; 10];
    stream.read_exact(&mut connect).await.expect("CONNECT");
    assert_eq!(&connect[..4], &[5, 1, 0, 1]);
    let std::net::IpAddr::V4(ip) = target.ip() else {
        panic!("IPv4")
    };
    assert_eq!(&connect[4..8], &ip.octets());
    assert_eq!(&connect[8..], &target.port().to_be_bytes());
    stream
        .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
        .await
        .expect("success");
    stream
}

fn dns_response(query: &[u8], address: Option<[u8; 4]>) -> Vec<u8> {
    assert_eq!(
        &query[12..query.len() - 4],
        b"\x0cfixture-only\x0csubscription\x04test\0"
    );
    let kind = u16::from_be_bytes(
        query[query.len() - 4..query.len() - 2]
            .try_into()
            .expect("qtype"),
    );
    assert!(matches!(kind, 1 | 28));
    let mut response = query.to_vec();
    response[2..4].copy_from_slice(&[0x81, 0x80]);
    if let Some(ip) = address.filter(|_| kind == 1) {
        response[6..8].copy_from_slice(&1_u16.to_be_bytes());
        response.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 30, 0, 4]);
        response.extend_from_slice(&ip);
    }
    response
}

#[tokio::test]
async fn proxy_dns_queries_literal_upstream_and_pins_http_without_system_dns() {
    let direct = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("direct sentinel");
    let target = direct.local_addr().expect("target");
    let dns = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("DNS upstream");
    let upstream = dns.local_addr().expect("upstream");
    let dns_task = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = dns.accept().await.expect("DNS accept through proxy");
            let size = stream.read_u16().await.expect("DNS frame length");
            let mut query = vec![0; size as usize];
            stream.read_exact(&mut query).await.expect("DNS query");
            let response = dns_response(&query, Some([127, 0, 0, 1]));
            stream
                .write_u16(response.len() as u16)
                .await
                .expect("length");
            stream.write_all(&response).await.expect("DNS response");
        }
    });
    let proxy = TcpListener::bind("127.0.0.1:0").await.expect("proxy");
    let endpoint = proxy.local_addr().expect("endpoint");
    let proxy_task = tokio::spawn(async move {
        for _ in 0..2 {
            let mut stream = socks_connect(&proxy, upstream).await;
            let mut remote = tokio::net::TcpStream::connect(upstream)
                .await
                .expect("fixture relay");
            tokio::io::copy_bidirectional(&mut stream, &mut remote)
                .await
                .expect("DNS relay");
        }
        // Третий CONNECT обязан содержать только IP из DNS, без повторного имени.
        let mut stream = socks_connect(&proxy, target).await;
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(stream.read_u8().await.expect("HTTP request"));
            assert!(request.len() < 32768);
        }
        assert!(
            String::from_utf8(request)
                .expect("HTTP")
                .contains("host: fixture-only.subscription.test:")
        );
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody")
            .await
            .expect("HTTP response");
    });
    let source = Source::local(&format!(
        "http://fixture-only.subscription.test:{}/secret",
        target.port()
    ))
    .expect("source");
    let host = FetchHost::new(Duration::from_secs(2))
        .expect("host")
        .with_socks_endpoint(endpoint)
        .expect("endpoint")
        .with_proxy_dns(upstream)
        .expect("explicit upstream");
    let (_sender, cancel) = watch::channel(false);
    let FetchResponse::Modified { body, .. } = host
        .fetch(&source, FetchRoute::Proxy, None, cancel)
        .await
        .expect("proxy DNS fetch")
    else {
        panic!("body")
    };
    assert_eq!(body.expose(), b"body");
    dns_task.await.expect("DNS fixture");
    proxy_task.await.expect("SOCKS fixture");
    assert!(
        tokio::time::timeout(Duration::from_millis(20), direct.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn proxy_dns_failures_and_cancellation_never_use_system_dns() {
    let source = Source::local("http://fixture-only.subscription.test/sub").expect("source");
    for mode in 0..8 {
        let proxy = TcpListener::bind("127.0.0.1:0").await.expect("proxy");
        let endpoint = proxy.local_addr().expect("endpoint");
        let upstream: SocketAddr = "127.0.0.1:5353".parse().expect("explicit upstream");
        let (sender, cancel) = watch::channel(false);
        let signal = sender.clone();
        let fixture = if mode == 0 {
            drop(proxy);
            None
        } else {
            Some(tokio::spawn(async move {
                let mut stream = socks_connect(&proxy, upstream).await;
                let size = stream.read_u16().await.expect("length");
                let mut query = vec![0; size as usize];
                stream.read_exact(&mut query).await.expect("query");
                if mode == 1 || mode == 2 {
                    if mode == 2 {
                        signal.send_replace(true);
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                } else {
                    let mut response = dns_response(&query, None);
                    match mode {
                        3 => response[3] = 0x83,  // NXDOMAIN.
                        4 => response[0] ^= 1,    // Чужой transaction ID.
                        5 => response[2] |= 2,    // Усечённый ответ не становится NODATA.
                        6 => response[12] = 0xff, // Повреждённый вопрос.
                        7 => {
                            stream.write_u16(65535).await.expect("oversized frame");
                            return;
                        }
                        _ => unreachable!(),
                    }
                    stream
                        .write_u16(response.len() as u16)
                        .await
                        .expect("length");
                    stream.write_all(&response).await.expect("response");
                }
            }))
        };
        let host = FetchHost::new(Duration::from_millis(80))
            .expect("host")
            .with_socks_endpoint(endpoint)
            .expect("endpoint")
            .with_proxy_dns(upstream)
            .expect("DNS path");
        let error = host
            .fetch(&source, FetchRoute::Proxy, None, cancel)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            match mode {
                1 => FetchError::Timeout,
                2 => FetchError::Cancelled,
                _ => FetchError::Dns,
            }
        );
        if let Some(fixture) = fixture {
            fixture.abort();
            let _ = fixture.await;
        }
    }
}

#[tokio::test]
async fn literal_redirect_to_domain_requires_explicit_proxy_dns() {
    let direct = TcpListener::bind("127.0.0.1:0").await.expect("direct");
    let target = direct.local_addr().expect("target");
    let proxy = TcpListener::bind("127.0.0.1:0").await.expect("proxy");
    let endpoint = proxy.local_addr().expect("endpoint");
    let source = Source::local(&format!("http://{target}/sub")).expect("source");
    let fixture = tokio::spawn(async move {
        socks(proxy, target, "HTTP/1.1 302 Found\r\nLocation: http://fixture-only.subscription.test/other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await
    });
    let host = FetchHost::new(Duration::from_secs(1))
        .expect("host")
        .with_socks_endpoint(endpoint)
        .expect("endpoint");
    let (_sender, cancel) = watch::channel(false);
    assert_eq!(
        host.fetch(&source, FetchRoute::Proxy, None, cancel)
            .await
            .unwrap_err(),
        FetchError::ProxyDnsRequired
    );
    fixture.await.expect("fixture");
    assert!(
        tokio::time::timeout(Duration::from_millis(20), direct.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn proxy_dns_private_answer_is_blocked_before_destination_connect() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.expect("proxy");
    let endpoint = proxy.local_addr().expect("endpoint");
    let upstream = "127.0.0.1:5353".parse().expect("upstream");
    let fixture = tokio::spawn(async move {
        for _ in 0..2 {
            let mut stream = socks_connect(&proxy, upstream).await;
            let size = stream.read_u16().await.expect("length");
            let mut query = vec![0; size as usize];
            stream.read_exact(&mut query).await.expect("query");
            let response = dns_response(&query, Some([127, 0, 0, 1]));
            stream
                .write_u16(response.len() as u16)
                .await
                .expect("length");
            stream.write_all(&response).await.expect("response");
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(30), proxy.accept())
                .await
                .is_err()
        );
    });
    let host = FetchHost::new(Duration::from_secs(1))
        .expect("host")
        .with_socks_endpoint(endpoint)
        .expect("endpoint")
        .with_proxy_dns(upstream)
        .expect("DNS path");
    let source = Source::public("https://fixture-only.subscription.test/sub").expect("source");
    let (_sender, cancel) = watch::channel(false);
    assert_eq!(
        host.fetch(&source, FetchRoute::Proxy, None, cancel)
            .await
            .unwrap_err(),
        FetchError::BlockedAddress
    );
    fixture.await.expect("fixture");
}

#[tokio::test]
async fn replacing_proxy_endpoint_clears_dns_path() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.expect("proxy");
    let endpoint = proxy.local_addr().expect("endpoint");
    let host = FetchHost::new(Duration::from_secs(1))
        .expect("host")
        .with_socks_endpoint(endpoint)
        .expect("endpoint")
        .with_proxy_dns("127.0.0.1:5353".parse().expect("DNS upstream"))
        .expect("DNS path")
        .with_socks_endpoint(endpoint)
        .expect("new session path");
    let source = Source::local("http://fixture-only.subscription.test/sub").expect("source");
    let (_sender, cancel) = watch::channel(false);
    assert_eq!(
        host.fetch(&source, FetchRoute::Proxy, None, cancel)
            .await
            .unwrap_err(),
        FetchError::ProxyDnsRequired
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), proxy.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn literal_ipv6_proxy_uses_no_system_resolver() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.expect("proxy");
    let endpoint = proxy.local_addr().expect("endpoint");
    let target: SocketAddr = "[::1]:18888".parse().expect("IPv6 target");
    let fixture = tokio::spawn(async move {
        socks(
            proxy,
            target,
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await
    });
    let source = Source::local("http://[::1]:18888/sub").expect("source");
    let host = FetchHost::new(Duration::from_secs(1))
        .expect("host")
        .with_socks_endpoint(endpoint)
        .expect("endpoint");
    let (_sender, cancel) = watch::channel(false);
    host.fetch(&source, FetchRoute::Proxy, None, cancel)
        .await
        .expect("literal IPv6");
    fixture.await.expect("SOCKS fixture");
}
