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
    let source =
        Source::local(&format!("http://localhost:{}/secret", target.port())).expect("source");
    let mut resolved: Vec<_> = tokio::net::lookup_host(("localhost", target.port()))
        .await
        .expect("DNS")
        .collect();
    resolved.sort_unstable();
    let pinned = resolved[0];
    let fixture = tokio::spawn(async move {
        socks(
            proxy,
            pinned,
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
