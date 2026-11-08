use std::time::{Duration, Instant};

use reqwest::{Client, Proxy, Url, redirect::Policy};

use super::HostError;

pub struct ProbeTarget {
    pub(crate) url: Url,
    pub(crate) timeout: Duration,
}
impl std::fmt::Debug for ProbeTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProbeTarget(<скрыто>)")
    }
}
impl ProbeTarget {
    pub fn new(url: &str, timeout: Duration) -> Result<Self, HostError> {
        if url.len() > 2048
            || url.chars().any(char::is_control)
            || timeout.is_zero()
            || timeout > Duration::from_secs(60)
        {
            return Err(HostError::ProbeTarget);
        }
        let url = Url::parse(url).map_err(|_| HostError::ProbeTarget)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(HostError::ProbeTarget);
        }
        Ok(Self { url, timeout })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeResult {
    Response { status: u16, elapsed: Duration },
    Timeout,
    Tls,
    Transport,
}
impl ProbeResult {
    pub fn passed(self) -> bool {
        matches!(
            self,
            Self::Response {
                status: 200..=299,
                ..
            }
        )
    }
}

pub(crate) async fn probe(port: u16, target: &ProbeTarget) -> ProbeResult {
    let proxy = match Proxy::all(format!("socks5h://127.0.0.1:{port}")) {
        Ok(proxy) => proxy,
        Err(_) => return ProbeResult::Transport,
    };
    let client = match Client::builder()
        .no_proxy()
        .proxy(proxy)
        .redirect(Policy::none())
        .timeout(target.timeout)
        .pool_max_idle_per_host(0)
        .build()
    {
        Ok(client) => client,
        Err(_) => return ProbeResult::Transport,
    };
    let start = Instant::now();
    match client.get(target.url.clone()).send().await {
        Ok(response) => ProbeResult::Response {
            status: response.status().as_u16(),
            elapsed: start.elapsed(),
        },
        Err(error) if error.is_timeout() => ProbeResult::Timeout,
        Err(error) => classify_error(&error),
    }
}

fn classify_error(error: &(dyn std::error::Error + 'static)) -> ProbeResult {
    let mut current = Some(error);
    while let Some(error) = current {
        if error.is::<rustls::Error>()
            || error
                .downcast_ref::<std::io::Error>()
                .and_then(std::io::Error::get_ref)
                .is_some_and(|source| source.is::<rustls::Error>())
        {
            return ProbeResult::Tls;
        }
        current = error.source();
    }
    // SOCKS не сообщает отдельно причину отказа удалённого DNS/туннеля.
    ProbeResult::Transport
}

#[cfg(test)]
mod tests {
    use super::{ProbeResult, classify_error};
    #[test]
    fn typed_tls_error_is_classified_without_parsing_sensitive_messages() {
        let error = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::NoCertificatesPresented,
        );
        assert_eq!(classify_error(&error), ProbeResult::Tls);
    }

    #[tokio::test]
    async fn socks5h_forwards_hostname_without_resolving_it_on_host() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let target = super::ProbeTarget::new(
            "https://tofu-probe.invalid/",
            std::time::Duration::from_secs(2),
        )
        .unwrap();
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut greeting = [0; 3];
            socket.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            socket.write_all(&[5, 0]).await.unwrap();
            let mut header = [0; 5];
            socket.read_exact(&mut header).await.unwrap();
            assert_eq!(&header[..4], &[5, 1, 0, 3]);
            let mut destination = vec![0; usize::from(header[4]) + 2];
            socket.read_exact(&mut destination).await.unwrap();
            assert_eq!(&destination[..destination.len() - 2], b"tofu-probe.invalid");
            assert_eq!(&destination[destination.len() - 2..], &[1, 187]);
            socket
                .write_all(&[5, 1, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
        };
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            tokio::join!(super::probe(port, &target), server)
        })
        .await
        .unwrap();
        assert_eq!(result, ProbeResult::Transport);
    }
}
