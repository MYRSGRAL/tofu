//! Ограниченная загрузка подписок. Секреты доступны только через явные методы.

use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use reqwest::{Url, header};
use tokio::sync::watch;

pub const MAX_FETCH_BODY: usize = 8 * 1024 * 1024;
const MAX_FIELD: usize = 8 * 1024;
const MAX_REDIRECTS: usize = 5;
const MAX_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct Source {
    url: Url,
    local_exception: bool,
}

impl Source {
    /// Секретный URL: только для защищённого хранилища и явного использования.
    pub fn expose_url(&self) -> &str {
        self.url.as_str()
    }

    pub fn public(value: &str) -> Result<Self, FetchError> {
        Self::parse(value, false)
    }

    /// Явное доверие источнику: разрешает HTTP и непубличные адреса, включая редиректы.
    pub fn local(value: &str) -> Result<Self, FetchError> {
        Self::parse(value, true)
    }

    fn parse(value: &str, local_exception: bool) -> Result<Self, FetchError> {
        if value.len() > MAX_FIELD || value.chars().any(char::is_control) {
            return Err(FetchError::InvalidSource);
        }
        let url = Url::parse(value).map_err(|_| FetchError::InvalidSource)?;
        validate_url(&url, local_exception)?;
        Ok(Self {
            url,
            local_exception,
        })
    }
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Source([скрыто])")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchRoute {
    Direct,
    Proxy,
}

#[derive(Clone)]
pub struct FetchValidators {
    resource: Url,
    etag: Option<header::HeaderValue>,
    last_modified: Option<header::HeaderValue>,
}

impl FetchValidators {
    /// Секретные значения: только для защищённого хранилища.
    pub fn resource_url(&self) -> &str {
        self.resource.as_str()
    }
    pub fn etag(&self) -> Option<&str> {
        self.etag.as_ref().and_then(|value| value.to_str().ok())
    }
    pub fn last_modified(&self) -> Option<&str> {
        self.last_modified
            .as_ref()
            .and_then(|value| value.to_str().ok())
    }

    /// Значения привязаны к точному URL ресурса, а не только к origin.
    pub fn new(
        source: &Source,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<Self, FetchError> {
        Ok(Self {
            resource: source.url.clone(),
            etag: validator_value(etag)?,
            last_modified: validator_value(last_modified)?,
        })
    }
}

impl fmt::Debug for FetchValidators {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FetchValidators([скрыто])")
    }
}

pub struct SecretBody(Vec<u8>);

impl SecretBody {
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl fmt::Debug for SecretBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretBody([скрыто])")
    }
}

#[derive(Debug)]
pub enum FetchResponse {
    Modified {
        body: SecretBody,
        validators: FetchValidators,
    },
    NotModified {
        validators: FetchValidators,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchError {
    InvalidSource,
    InvalidValidator,
    InvalidTimeout,
    UnsupportedPeerVerification,
    Cancelled,
    Timeout,
    Dns,
    BlockedAddress,
    PeerMismatch,
    Transport,
    Redirect,
    HttpStatus,
    UnexpectedNotModified,
    UnsupportedEncoding,
    BodyTooLarge,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for FetchError {}

pub struct FetchHost {
    timeout: Duration,
}

impl FetchHost {
    pub fn new(timeout: Duration) -> Result<Self, FetchError> {
        if timeout.is_zero() || timeout > MAX_TIMEOUT {
            return Err(FetchError::InvalidTimeout);
        }
        Ok(Self { timeout })
    }

    pub async fn fetch(
        &self,
        source: &Source,
        route: FetchRoute,
        validators: Option<&FetchValidators>,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<FetchResponse, FetchError> {
        if *cancel.borrow() || cancel.has_changed().is_err() {
            return Err(FetchError::Cancelled);
        }
        if route == FetchRoute::Proxy {
            return Err(FetchError::UnsupportedPeerVerification);
        }
        tokio::select! {
            biased;
            () = cancelled(&mut cancel) => Err(FetchError::Cancelled),
            result = tokio::time::timeout(self.timeout, self.fetch_direct(source, validators)) => {
                result.map_err(|_| FetchError::Timeout)?
            }
        }
    }

    async fn fetch_direct(
        &self,
        source: &Source,
        validators: Option<&FetchValidators>,
    ) -> Result<FetchResponse, FetchError> {
        let mut url = source.url.clone();
        for hop in 0..=MAX_REDIRECTS {
            validate_url(&url, source.local_exception)?;
            let host = url.host_str().ok_or(FetchError::InvalidSource)?;
            let port = url
                .port_or_known_default()
                .ok_or(FetchError::InvalidSource)?;
            let addresses = resolve(&url, port).await?;
            if !source.local_exception
                && addresses.iter().any(|address| !is_public_ip(address.ip()))
            {
                return Err(FetchError::BlockedAddress);
            }
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .referer(false)
                .resolve_to_addrs(host, &addresses)
                .timeout(self.timeout)
                .build()
                .map_err(|_| FetchError::Transport)?;
            let mut request = client
                .get(url.clone())
                // Remnawave распознаёт /^Happ\// и выдаёт Xray JSON вместо URI fallback.
                // Собственное имя клиента сохраняем отдельным product token.
                .header(
                    header::USER_AGENT,
                    concat!("Happ/1.0 tofu/", env!("CARGO_PKG_VERSION")),
                )
                .header(header::ACCEPT_ENCODING, "identity");
            let conditional = validators.filter(|value| value.resource == url);
            let sent_conditional = conditional
                .is_some_and(|value| value.etag.is_some() || value.last_modified.is_some());
            if let Some(value) = conditional {
                if let Some(etag) = &value.etag {
                    request = request.header(header::IF_NONE_MATCH, etag.clone());
                }
                if let Some(modified) = &value.last_modified {
                    request = request.header(header::IF_MODIFIED_SINCE, modified.clone());
                }
            }
            let mut response = request.send().await.map_err(transport_error)?;
            let peer = response.remote_addr().ok_or(FetchError::PeerMismatch)?;
            if !addresses.contains(&peer) || (!source.local_exception && !is_public_ip(peer.ip())) {
                return Err(FetchError::PeerMismatch);
            }
            if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                if hop == MAX_REDIRECTS {
                    return Err(FetchError::Redirect);
                }
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .ok_or(FetchError::Redirect)?;
                if location.len() > MAX_FIELD {
                    return Err(FetchError::Redirect);
                }
                let location = location.to_str().map_err(|_| FetchError::Redirect)?;
                url = url.join(location).map_err(|_| FetchError::Redirect)?;
                continue;
            }
            if response
                .headers()
                .get_all(header::CONTENT_ENCODING)
                .iter()
                .any(|value| !value.as_bytes().eq_ignore_ascii_case(b"identity"))
            {
                return Err(FetchError::UnsupportedEncoding);
            }
            let previous = if response.status() == reqwest::StatusCode::NOT_MODIFIED {
                conditional
            } else {
                None
            };
            let current = response_validators(&url, response.headers(), previous)?;
            if response.status() == reqwest::StatusCode::NOT_MODIFIED {
                if !sent_conditional {
                    return Err(FetchError::UnexpectedNotModified);
                }
                return Ok(FetchResponse::NotModified {
                    validators: current,
                });
            }
            if !response.status().is_success() {
                return Err(FetchError::HttpStatus);
            }
            // Range не запрашивался: валидный URI-префикс частичного ответа
            // иначе мог бы выглядеть полным отчётом и заменить весь snapshot.
            if response.status() == reqwest::StatusCode::PARTIAL_CONTENT
                || response.headers().contains_key(header::CONTENT_RANGE)
            {
                return Err(FetchError::HttpStatus);
            }
            if response
                .content_length()
                .is_some_and(|length| length > MAX_FETCH_BODY as u64)
            {
                return Err(FetchError::BodyTooLarge);
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
                if chunk.len() > MAX_FETCH_BODY - bytes.len() {
                    return Err(FetchError::BodyTooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok(FetchResponse::Modified {
                body: SecretBody(bytes),
                validators: current,
            });
        }
        Err(FetchError::Redirect)
    }
}

fn transport_error(error: reqwest::Error) -> FetchError {
    if error.is_timeout() {
        FetchError::Timeout
    } else {
        FetchError::Transport
    }
}

async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            return;
        }
    }
}

fn validate_url(url: &Url, local_exception: bool) -> Result<(), FetchError> {
    if url.as_str().len() > MAX_FIELD
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || url.port_or_known_default() == Some(0)
        || (url.scheme() != "https" && !(local_exception && url.scheme() == "http"))
    {
        return Err(FetchError::InvalidSource);
    }
    Ok(())
}

async fn resolve(url: &Url, port: u16) -> Result<Vec<SocketAddr>, FetchError> {
    let host = url.host_str().ok_or(FetchError::InvalidSource)?;
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    let mut addresses = if let Ok(ip) = literal.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| FetchError::Dns)?
            .take(65)
            .collect::<Vec<_>>()
    };
    if addresses.len() > 64 {
        return Err(FetchError::Dns);
    }
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty() {
        return Err(FetchError::Dns);
    }
    Ok(addresses)
}

fn validator_value(value: Option<&str>) -> Result<Option<header::HeaderValue>, FetchError> {
    value
        .map(|value| {
            if value.len() > MAX_FIELD {
                return Err(FetchError::InvalidValidator);
            }
            let mut header =
                header::HeaderValue::from_str(value).map_err(|_| FetchError::InvalidValidator)?;
            if header.to_str().is_err() {
                return Err(FetchError::InvalidValidator);
            }
            header.set_sensitive(true);
            Ok(header)
        })
        .transpose()
}

fn response_validators(
    url: &Url,
    headers: &header::HeaderMap,
    previous: Option<&FetchValidators>,
) -> Result<FetchValidators, FetchError> {
    let read = |name: header::HeaderName,
                old: Option<&header::HeaderValue>|
     -> Result<Option<header::HeaderValue>, FetchError> {
        match headers.get(name) {
            Some(value) => {
                if value.len() > MAX_FIELD || value.to_str().is_err() {
                    return Err(FetchError::InvalidValidator);
                }
                let mut value = value.clone();
                value.set_sensitive(true);
                Ok(Some(value))
            }
            None => Ok(old.cloned()),
        }
    };
    Ok(FetchValidators {
        resource: url.clone(),
        etag: read(header::ETAG, previous.and_then(|value| value.etag.as_ref()))?,
        last_modified: read(
            header::LAST_MODIFIED,
            previous.and_then(|value| value.last_modified.as_ref()),
        )?,
    })
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 168)
                || (a == 192 && b == 88 && c == 99)
                || (a == 192 && b == 0 && c == 2)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            // Только global unicast; IETF special-purpose и 6to4 могут скрыть IPv4 endpoint.
            (segments[0] & 0xe000) == 0x2000
                && segments[0] != 0x2002
                && segments[0] != 0x3ffe
                && !(segments[0] == 0x2001 && (segments[1] < 0x0200 || segments[1] == 0x0db8))
                && !(segments[0] == 0x3fff && segments[1] < 0x1000)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Требует публичной сети; проверяет реальный HTTPS/DNS/peer Direct"]
    async fn public_https_direct() {
        let source = Source::public("https://www.gstatic.com/generate_204").expect("публичный URL");
        let (_sender, receiver) = watch::channel(false);
        let host = FetchHost::new(Duration::from_secs(15)).expect("timeout");
        match host
            .fetch(&source, FetchRoute::Direct, None, receiver)
            .await
        {
            Ok(FetchResponse::Modified { body, .. }) => assert!(body.expose().is_empty()),
            Ok(FetchResponse::NotModified { .. }) => panic!("Неожиданный 304"),
            Err(error) => panic!("Проверка HTTPS: {error}"),
        }
    }
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn fixture(responses: Vec<String>) -> (Source, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("локальный listener");
        let source = Source::local(&format!(
            "http://{}/secret",
            listener.local_addr().expect("адрес")
        ))
        .expect("источник");
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().await.expect("accept");
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).await.expect("чтение") == 0 {
                        break;
                    }
                    request.push(byte[0]);
                    assert!(request.len() < 32 * 1024);
                }
                requests.push(String::from_utf8(request).expect("HTTP request"));
                // Отказ клиента по лимиту может закрыть сокет до конца ответа.
                let _ = stream.write_all(response.as_bytes()).await;
            }
            requests
        });
        (source, task)
    }

    fn host() -> FetchHost {
        FetchHost::new(Duration::from_secs(3)).expect("timeout")
    }

    #[test]
    fn public_addresses_reject_special_and_transition_ranges() {
        for value in [
            "0.1.2.3",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "172.31.0.1",
            "192.0.0.1",
            "192.0.2.1",
            "192.168.0.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::ffff:8.8.8.8",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "2001::808:808",
            "2002:7f00:1::",
            "3fff::1",
        ] {
            assert!(!is_public_ip(value.parse().expect("IP")), "{value}");
        }
        for value in [
            "8.8.8.8",
            "1.1.1.1",
            "100.128.0.1",
            "172.32.0.1",
            "192.1.1.1",
            "192.0.8.1",
            "2001:4860:4860::8888",
            "2606:4700:4700::1111",
        ] {
            assert!(is_public_ip(value.parse().expect("IP")), "{value}");
        }
    }

    #[test]
    fn source_and_validator_validation_and_redaction() {
        for url in [
            "http://example.com/",
            "https://user:password@example.com/",
            "https://example.com/#token",
            "file:///etc/passwd",
            "https://example.com:0/",
            "https://example.com/\nsecret",
        ] {
            assert!(Source::public(url).is_err());
        }
        assert!(Source::public(&format!("https://example.com/{}", "x".repeat(MAX_FIELD))).is_err());
        let source = Source::public("https://example.com/secret?token=hidden").expect("URL");
        assert!(!format!("{source:?}").contains("hidden"));
        let validators =
            FetchValidators::new(&source, Some("secret-etag"), None).expect("validator");
        assert!(!format!("{validators:?}").contains("secret-etag"));
        assert!(FetchValidators::new(&source, Some("x\r\nInjected: yes"), None).is_err());
        assert!(FetchValidators::new(&source, Some(&"x".repeat(MAX_FIELD + 1)), None).is_err());
        assert!(FetchHost::new(Duration::ZERO).is_err());
        assert!(FetchHost::new(Duration::from_secs(61)).is_err());
        assert_eq!(
            format!("{:?}", SecretBody(b"secret-body".to_vec())),
            "SecretBody([скрыто])"
        );
    }

    #[tokio::test]
    async fn route_and_public_policy_fail_before_connecting() {
        let source = Source::public("https://127.0.0.1:1/secret").expect("URL");
        let (_sender, receiver) = watch::channel(false);
        assert_eq!(
            host()
                .fetch(&source, FetchRoute::Proxy, None, receiver.clone())
                .await
                .unwrap_err(),
            FetchError::UnsupportedPeerVerification
        );
        assert_eq!(
            host()
                .fetch(&source, FetchRoute::Direct, None, receiver)
                .await
                .unwrap_err(),
            FetchError::BlockedAddress
        );
    }

    #[tokio::test]
    async fn subscription_user_agent_selects_happ_json() {
        let (source, task) = fixture(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into(),
        ])
        .await;
        let (_sender, receiver) = watch::channel(false);
        host()
            .fetch(&source, FetchRoute::Direct, None, receiver)
            .await
            .expect("загрузка");
        let requests = task.await.expect("fixture");
        assert!(requests[0].contains("user-agent: Happ/1.0 tofu/"));
        assert!(requests[0].contains("accept-encoding: identity"));
    }

    #[tokio::test]
    async fn local_body_and_exact_resource_conditional_304() {
        let (source, task) = fixture(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nETag: old\r\nConnection: close\r\n\r\nbody"
                .into(),
            "HTTP/1.1 304 Not Modified\r\nETag: new\r\nConnection: close\r\n\r\n".into(),
        ])
        .await;
        let (_sender, receiver) = watch::channel(false);
        let response = host()
            .fetch(&source, FetchRoute::Direct, None, receiver.clone())
            .await
            .expect("200");
        let FetchResponse::Modified { body, validators } = response else {
            panic!("ожидалось тело")
        };
        assert_eq!(body.into_bytes(), b"body");
        let response = host()
            .fetch(&source, FetchRoute::Direct, Some(&validators), receiver)
            .await
            .expect("304");
        let FetchResponse::NotModified { validators } = response else {
            panic!("ожидался 304")
        };
        assert_eq!(validators.etag.as_ref().expect("etag"), "new");
        let requests = task.await.expect("fixture");
        assert!(!requests[0].contains("if-none-match"));
        assert!(requests[1].contains("if-none-match: old"));
    }

    #[tokio::test]
    async fn redirect_does_not_forward_resource_validators() {
        let (source, task) = fixture(vec![
            "HTTP/1.1 302 Found\r\nLocation: /other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        ]).await;
        let validators =
            FetchValidators::new(&source, Some("private-etag"), None).expect("validator");
        let (_sender, receiver) = watch::channel(false);
        let response = host()
            .fetch(&source, FetchRoute::Direct, Some(&validators), receiver)
            .await
            .expect("redirect");
        let FetchResponse::Modified { validators, .. } = response else {
            panic!("ожидался 200")
        };
        assert!(validators.resource_url().ends_with("/other"));
        assert!(validators.etag().is_none());
        let requests = task.await.expect("fixture");
        assert!(requests[0].contains("if-none-match: private-etag"));
        assert!(!requests[1].contains("private-etag"));
    }

    #[tokio::test]
    async fn rejects_bad_status_encoding_redirect_and_oversized_body() {
        for (response, expected) in [
            (
                "HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".into(),
                FetchError::UnexpectedNotModified,
            ),
            (
                "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n".into(),
                FetchError::HttpStatus,
            ),
            (
                "HTTP/1.1 206 Partial Content\r\nContent-Length: 0\r\n\r\n".into(),
                FetchError::HttpStatus,
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Range: bytes 0-9/100\r\nContent-Length: 0\r\n\r\n"
                    .into(),
                FetchError::HttpStatus,
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 0\r\n\r\n".into(),
                FetchError::UnsupportedEncoding,
            ),
            (
                "HTTP/1.1 302 Found\r\nLocation: file:///etc/passwd\r\nContent-Length: 0\r\n\r\n"
                    .into(),
                FetchError::InvalidSource,
            ),
            (
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                    MAX_FETCH_BODY + 1
                ),
                FetchError::BodyTooLarge,
            ),
            (
                format!(
                    "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{}",
                    "x".repeat(MAX_FETCH_BODY + 1)
                ),
                FetchError::BodyTooLarge,
            ),
        ] {
            let (source, task) = fixture(vec![response]).await;
            let (_sender, receiver) = watch::channel(false);
            assert_eq!(
                host()
                    .fetch(&source, FetchRoute::Direct, None, receiver)
                    .await
                    .unwrap_err(),
                expected
            );
            task.await.expect("fixture");
        }
    }

    #[tokio::test]
    async fn redirect_limit() {
        let response = "HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned();
        let (source, task) = fixture(vec![response; MAX_REDIRECTS + 1]).await;
        let (_sender, receiver) = watch::channel(false);
        assert_eq!(
            host()
                .fetch(&source, FetchRoute::Direct, None, receiver)
                .await
                .unwrap_err(),
            FetchError::Redirect
        );
        assert_eq!(task.await.expect("fixture").len(), MAX_REDIRECTS + 1);
    }

    #[tokio::test]
    async fn cancellation_and_timeout_cover_waiting_for_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let source = Source::local(&format!(
            "http://{}/",
            listener.local_addr().expect("адрес")
        ))
        .expect("URL");
        let (sender, receiver) = watch::channel(false);
        sender.send(true).expect("cancel");
        assert_eq!(
            host()
                .fetch(&source, FetchRoute::Direct, None, receiver)
                .await
                .unwrap_err(),
            FetchError::Cancelled
        );
        let (sender, receiver) = watch::channel(false);
        drop(sender);
        assert_eq!(
            host()
                .fetch(&source, FetchRoute::Direct, None, receiver)
                .await
                .unwrap_err(),
            FetchError::Cancelled
        );
        let (sender, receiver) = watch::channel(false);
        let fetch = host();
        let pending = fetch.fetch(&source, FetchRoute::Direct, None, receiver);
        let cancel = async {
            let (_stream, _) = listener.accept().await.expect("accept");
            sender.send(true).expect("cancel");
        };
        let (result, ()) = tokio::join!(pending, cancel);
        assert_eq!(result.unwrap_err(), FetchError::Cancelled);
        let (_sender, receiver) = watch::channel(false);
        let short = FetchHost::new(Duration::from_millis(20)).expect("timeout");
        assert_eq!(
            short
                .fetch(&source, FetchRoute::Direct, None, receiver)
                .await
                .unwrap_err(),
            FetchError::Timeout
        );
    }
}
