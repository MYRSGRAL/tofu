//! Ограниченная загрузка подписок. Секреты доступны только через явные методы.

use std::{
    collections::BTreeMap,
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use reqwest::{Url, header};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::watch,
};

pub const MAX_FETCH_BODY: usize = 8 * 1024 * 1024;
const MAX_FIELD: usize = 8 * 1024;
const MAX_REDIRECTS: usize = 5;
const MAX_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) fn valid_hwid(value: &str) -> bool {
    (10..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'=' | b'-'))
}

#[derive(Clone)]
pub struct Source {
    url: Url,
    local_exception: bool,
    hwid: Option<header::HeaderValue>,
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

    /// Идентификатор передаётся только этому origin и скрыт в Debug.
    pub fn with_hwid(mut self, value: &str) -> Result<Self, FetchError> {
        if !valid_hwid(value) {
            return Err(FetchError::InvalidHwid);
        }
        let mut value =
            header::HeaderValue::from_str(value).map_err(|_| FetchError::InvalidHwid)?;
        value.set_sensitive(true);
        self.hwid = Some(value);
        Ok(self)
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
            hwid: None,
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
    hwid: Option<header::HeaderValue>,
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
            hwid: source.hwid.clone(),
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
    InvalidHwid,
    HwidRequired,
    HwidLimit,
    HwidRejected,
    HwidRedirect,
    InvalidValidator,
    InvalidTimeout,
    UnsupportedPeerVerification,
    ProxyDnsRequired,
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
    socks_endpoint: Option<SocketAddr>,
    proxy_dns: Option<SocketAddr>,
}

impl FetchHost {
    pub fn new(timeout: Duration) -> Result<Self, FetchError> {
        if timeout.is_zero() || timeout > MAX_TIMEOUT {
            return Err(FetchError::InvalidTimeout);
        }
        Ok(Self {
            timeout,
            socks_endpoint: None,
            proxy_dns: None,
        })
    }

    /// Явно выбранный SOCKS5 listener активной сессии. Без with_proxy_dns
    /// разрешены только literal IP: домены отклоняются до системного DNS.
    /// Смена endpoint отзывает прежнюю DNS-настройку. Конечный peer доверяется
    /// сессии, для HTTPS проверяется сертификат исходного имени.
    pub fn with_socks_endpoint(mut self, endpoint: SocketAddr) -> Result<Self, FetchError> {
        if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
            return Err(FetchError::BlockedAddress);
        }
        self.socks_endpoint = Some(endpoint);
        self.proxy_dns = None;
        Ok(self)
    }

    /// Явный DNS-over-TCP upstream, доступный через выбранную SOCKS-сессию.
    /// Адрес выбирает вызывающий session provider; bootstrap и fallback нет.
    /// Настройка относится к текущему endpoint, а не к UI порту или system DNS.
    pub fn with_proxy_dns(mut self, upstream: SocketAddr) -> Result<Self, FetchError> {
        if self.socks_endpoint.is_none() {
            return Err(FetchError::UnsupportedPeerVerification);
        }
        if upstream.port() == 0
            || upstream.ip().is_unspecified()
            || upstream.ip().is_multicast()
            || upstream.ip() == IpAddr::V4(Ipv4Addr::BROADCAST)
            || matches!(upstream, SocketAddr::V6(address) if address.scope_id() != 0)
        {
            return Err(FetchError::Dns);
        }
        self.proxy_dns = Some(upstream);
        Ok(self)
    }

    async fn resolve_proxy(&self, url: &Url, port: u16) -> Result<Vec<SocketAddr>, FetchError> {
        let host = url.host_str().ok_or(FetchError::InvalidSource)?;
        if let Ok(ip) = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
        {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }
        let upstream = self.proxy_dns.ok_or(FetchError::ProxyDnsRequired)?;
        let endpoint = self
            .socks_endpoint
            .ok_or(FetchError::UnsupportedPeerVerification)?;
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let mut addresses = Vec::new();
        // Отказ любого семейства не превращается в успешный неполный набор.
        for kind in [1, 28] {
            let mut nonce = [0; 2];
            getrandom::getrandom(&mut nonce).map_err(|_| FetchError::Dns)?;
            let id = u16::from_be_bytes(nonce);
            let query = dns_query(&host, id, kind)?;
            let mut stream = dns_tunnel(endpoint, upstream).await?;
            stream
                .write_u16(query.len() as u16)
                .await
                .map_err(|_| FetchError::Dns)?;
            stream
                .write_all(&query)
                .await
                .map_err(|_| FetchError::Dns)?;
            let size = stream.read_u16().await.map_err(|_| FetchError::Dns)? as usize;
            if !(12..=MAX_DNS_FRAME).contains(&size) {
                return Err(FetchError::Dns);
            }
            let mut frame = vec![0; size];
            stream
                .read_exact(&mut frame)
                .await
                .map_err(|_| FetchError::Dns)?;
            addresses.extend(
                dns_addresses(&frame, &host, id, kind)?
                    .into_iter()
                    .map(|ip| SocketAddr::new(ip, port)),
            );
            if addresses.len() > 64 {
                return Err(FetchError::Dns);
            }
        }
        addresses.sort_unstable();
        addresses.dedup();
        if addresses.is_empty() {
            return Err(FetchError::Dns);
        }
        Ok(addresses)
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
        if route == FetchRoute::Proxy && self.socks_endpoint.is_none() {
            return Err(FetchError::UnsupportedPeerVerification);
        }
        tokio::select! {
            biased;
            () = cancelled(&mut cancel) => Err(FetchError::Cancelled),
            result = tokio::time::timeout(self.timeout, self.fetch_route(source, route, validators)) => {
                result.map_err(|_| FetchError::Timeout)?
            }
        }
    }

    async fn fetch_route(
        &self,
        source: &Source,
        route: FetchRoute,
        validators: Option<&FetchValidators>,
    ) -> Result<FetchResponse, FetchError> {
        let mut url = source.url.clone();
        for hop in 0..=MAX_REDIRECTS {
            if source.hwid.is_some() && url.origin() != source.url.origin() {
                return Err(FetchError::HwidRedirect);
            }
            validate_url(&url, source.local_exception)?;
            let host = url.host_str().ok_or(FetchError::InvalidSource)?;
            let port = url
                .port_or_known_default()
                .ok_or(FetchError::InvalidSource)?;
            let addresses = match route {
                FetchRoute::Direct => resolve(&url, port).await?,
                FetchRoute::Proxy => self.resolve_proxy(&url, port).await?,
            };
            if !source.local_exception
                && addresses.iter().any(|address| !is_public_ip(address.ip()))
            {
                return Err(FetchError::BlockedAddress);
            }
            let proxy_endpoint = if route == FetchRoute::Proxy {
                self.socks_endpoint
            } else {
                None
            };
            let mut builder = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .referer(false)
                .dns_resolver(Arc::new(NoSystemDns))
                .resolve_to_addrs(host, &addresses)
                .timeout(self.timeout);
            if let Some(endpoint) = proxy_endpoint {
                let proxy = reqwest::Proxy::all(format!("socks5://{endpoint}"))
                    .map_err(|_| FetchError::Transport)?;
                builder = builder.proxy(proxy);
            }
            let client = builder.build().map_err(|_| FetchError::Transport)?;
            let mut request = client
                .get(url.clone())
                // Remnawave распознаёт /^Happ\// и выдаёт Xray JSON вместо URI fallback.
                // Собственное имя клиента сохраняем отдельным product token.
                .header(
                    header::USER_AGENT,
                    concat!("Happ/1.0 yonik/", env!("CARGO_PKG_VERSION")),
                )
                .header(header::ACCEPT_ENCODING, "identity");
            if let Some(hwid) = &source.hwid {
                request = request.header("x-hwid", hwid.clone());
            }
            let conditional =
                validators.filter(|value| value.resource == url && value.hwid == source.hwid);
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
            let valid_peer = match proxy_endpoint {
                Some(endpoint) => peer == endpoint,
                None => {
                    addresses.contains(&peer) && (source.local_exception || is_public_ip(peer.ip()))
                }
            };
            if !valid_peer {
                return Err(FetchError::PeerMismatch);
            }
            let flag = |name: &str| {
                response
                    .headers()
                    .get_all(name)
                    .iter()
                    .any(|value| value.as_bytes().eq_ignore_ascii_case(b"true"))
            };
            if flag("x-hwid-not-supported") {
                return Err(FetchError::HwidRequired);
            }
            if flag("x-hwid-max-devices-reached") {
                return Err(FetchError::HwidLimit);
            }
            if flag("x-hwid-limit") {
                return Err(FetchError::HwidRejected);
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
            let current =
                response_validators(&url, source.hwid.as_ref(), response.headers(), previous)?;
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

/// Overrides содержат полный проверенный набор; ошибка ключа не вызывает GAI.
struct NoSystemDns;
impl reqwest::dns::Resolve for NoSystemDns {
    fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async { Err(std::io::Error::other("DNS pin отсутствует").into()) })
    }
}

const MAX_DNS_FRAME: usize = 16 * 1024;
const MAX_DNS_RECORDS: usize = 128;

async fn dns_tunnel(endpoint: SocketAddr, upstream: SocketAddr) -> Result<TcpStream, FetchError> {
    let mut stream = TcpStream::connect(endpoint)
        .await
        .map_err(|_| FetchError::Dns)?;
    if stream.peer_addr().map_err(|_| FetchError::Dns)? != endpoint {
        return Err(FetchError::Dns);
    }
    stream
        .write_all(&[5, 1, 0])
        .await
        .map_err(|_| FetchError::Dns)?;
    let mut method = [0; 2];
    stream
        .read_exact(&mut method)
        .await
        .map_err(|_| FetchError::Dns)?;
    if method != [5, 0] {
        return Err(FetchError::Dns);
    }
    let mut connect = vec![5, 1, 0];
    match upstream.ip() {
        IpAddr::V4(ip) => {
            connect.push(1);
            connect.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            connect.push(4);
            connect.extend_from_slice(&ip.octets());
        }
    }
    connect.extend_from_slice(&upstream.port().to_be_bytes());
    stream
        .write_all(&connect)
        .await
        .map_err(|_| FetchError::Dns)?;
    let mut reply = [0; 4];
    stream
        .read_exact(&mut reply)
        .await
        .map_err(|_| FetchError::Dns)?;
    if reply[..3] != [5, 0, 0] {
        return Err(FetchError::Dns);
    }
    let size = match reply[3] {
        1 => 4,
        4 => 16,
        3 => stream.read_u8().await.map_err(|_| FetchError::Dns)? as usize,
        _ => return Err(FetchError::Dns),
    };
    if size == 0 {
        return Err(FetchError::Dns);
    }
    let mut bound = [0; 257];
    stream
        .read_exact(&mut bound[..size + 2])
        .await
        .map_err(|_| FetchError::Dns)?;
    Ok(stream)
}

fn dns_query(host: &str, id: u16, kind: u16) -> Result<Vec<u8>, FetchError> {
    if host.is_empty() || host.len() > 253 {
        return Err(FetchError::Dns);
    }
    let mut query = Vec::with_capacity(host.len() + 18);
    query.extend_from_slice(&id.to_be_bytes());
    query.extend_from_slice(&[1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 || !label.bytes().all(dns_label_byte) {
            return Err(FetchError::Dns);
        }
        query.push(label.len() as u8);
        query.extend_from_slice(label.as_bytes());
    }
    query.push(0);
    query.extend_from_slice(&kind.to_be_bytes());
    query.extend_from_slice(&1_u16.to_be_bytes());
    Ok(query)
}

fn dns_label_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn dns_take<'a>(
    frame: &'a [u8],
    position: &mut usize,
    length: usize,
) -> Result<&'a [u8], FetchError> {
    let end = position.checked_add(length).ok_or(FetchError::Dns)?;
    let bytes = frame.get(*position..end).ok_or(FetchError::Dns)?;
    *position = end;
    Ok(bytes)
}
fn dns_u16(frame: &[u8], position: &mut usize) -> Result<u16, FetchError> {
    let bytes = dns_take(frame, position, 2)?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}
fn dns_name(frame: &[u8], position: &mut usize) -> Result<String, FetchError> {
    let mut cursor = *position;
    let mut end = None;
    let mut name = String::new();
    for _ in 0..128 {
        let length = *frame.get(cursor).ok_or(FetchError::Dns)?;
        if length & 0xc0 == 0xc0 {
            let low = *frame.get(cursor + 1).ok_or(FetchError::Dns)?;
            let pointer = (((length & 0x3f) as usize) << 8) | low as usize;
            // RFC 1035 compression ссылается на предыдущее имя; forward/loops запрещены.
            if pointer >= cursor {
                return Err(FetchError::Dns);
            }
            end.get_or_insert(cursor + 2);
            cursor = pointer;
        } else {
            if length & 0xc0 != 0 {
                return Err(FetchError::Dns);
            }
            cursor += 1;
            if length == 0 {
                *position = end.unwrap_or(cursor);
                return Ok(name);
            }
            let label = dns_take(frame, &mut cursor, length as usize)?;
            if !label.iter().copied().all(dns_label_byte) {
                return Err(FetchError::Dns);
            }
            if !name.is_empty() {
                name.push('.');
            }
            if name.len() + label.len() > 253 {
                return Err(FetchError::Dns);
            }
            for byte in label {
                name.push(byte.to_ascii_lowercase() as char);
            }
        }
    }
    Err(FetchError::Dns)
}

fn dns_addresses(frame: &[u8], host: &str, id: u16, kind: u16) -> Result<Vec<IpAddr>, FetchError> {
    let mut position = 0;
    if dns_u16(frame, &mut position)? != id {
        return Err(FetchError::Dns);
    }
    let flags = dns_u16(frame, &mut position)?;
    // QR=response, стандартный opcode, TC=0, reserved Z=0, RCODE=NOERROR.
    if flags & 0xfa4f != 0x8000 || dns_u16(frame, &mut position)? != 1 {
        return Err(FetchError::Dns);
    }
    let answers = dns_u16(frame, &mut position)? as usize;
    let authority = dns_u16(frame, &mut position)? as usize;
    let additional = dns_u16(frame, &mut position)? as usize;
    let total = answers + authority + additional;
    if total > MAX_DNS_RECORDS {
        return Err(FetchError::Dns);
    }
    if dns_name(frame, &mut position)? != host
        || dns_u16(frame, &mut position)? != kind
        || dns_u16(frame, &mut position)? != 1
    {
        return Err(FetchError::Dns);
    }
    let mut aliases = BTreeMap::new();
    let mut addresses = Vec::new();
    for index in 0..total {
        let owner = dns_name(frame, &mut position)?;
        let record_kind = dns_u16(frame, &mut position)?;
        let class = dns_u16(frame, &mut position)?;
        dns_take(frame, &mut position, 4)?; // TTL: кэширования между операциями нет.
        let length = dns_u16(frame, &mut position)? as usize;
        let start = position;
        let bytes = dns_take(frame, &mut position, length)?;
        if index >= answers || class != 1 {
            continue;
        }
        match record_kind {
            1 if length == 4 => addresses.push((
                owner,
                IpAddr::V4(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])),
            )),
            28 if length == 16 => {
                let octets: [u8; 16] = bytes.try_into().map_err(|_| FetchError::Dns)?;
                addresses.push((owner, IpAddr::V6(Ipv6Addr::from(octets))));
            }
            1 | 28 => return Err(FetchError::Dns),
            5 => {
                let mut offset = start;
                let alias = dns_name(frame, &mut offset)?;
                if alias.is_empty() || offset != position {
                    return Err(FetchError::Dns);
                }
                if aliases.insert(owner, alias).is_some() {
                    return Err(FetchError::Dns);
                }
            }
            _ => {}
        }
    }
    if position != frame.len() {
        return Err(FetchError::Dns);
    }
    let mut canonical = host;
    for depth in 0..=8 {
        let Some(alias) = aliases.get(canonical) else {
            break;
        };
        if depth == 8 || addresses.iter().any(|(owner, _)| owner == canonical) {
            return Err(FetchError::Dns);
        }
        canonical = alias;
    }
    let result: Vec<_> = addresses
        .into_iter()
        .filter_map(|(owner, ip)| {
            (owner == canonical && matches!((kind, ip), (1, IpAddr::V4(_)) | (28, IpAddr::V6(_))))
                .then_some(ip)
        })
        .collect();
    if result.len() > 64 {
        return Err(FetchError::Dns);
    }
    Ok(result)
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
    hwid: Option<&header::HeaderValue>,
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
        hwid: hwid.cloned(),
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
        assert!(requests[0].contains("user-agent: Happ/1.0 yonik/"));
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

#[cfg(test)]
mod proxy_dns_tests {
    use super::*;

    fn answer() -> Vec<u8> {
        let mut frame = dns_query("a.test", 7, 1).expect("query");
        frame[2..4].copy_from_slice(&[0x81, 0x80]);
        frame[6..8].copy_from_slice(&1_u16.to_be_bytes());
        frame.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 1, 0, 4, 8, 8, 4, 4]);
        frame
    }

    #[test]
    fn response_is_bound_to_id_question_and_complete_frame() {
        let frame = answer();
        assert_eq!(
            dns_addresses(&frame, "a.test", 7, 1).expect("answer"),
            vec!["8.8.4.4".parse::<IpAddr>().expect("IP")]
        );
        assert_eq!(dns_addresses(&frame, "b.test", 7, 1), Err(FetchError::Dns));
        assert_eq!(dns_addresses(&frame, "a.test", 8, 1), Err(FetchError::Dns));
        assert_eq!(dns_addresses(&frame, "a.test", 7, 28), Err(FetchError::Dns));
        for end in 0..frame.len() {
            assert_eq!(
                dns_addresses(&frame[..end], "a.test", 7, 1),
                Err(FetchError::Dns),
                "truncation {end}"
            );
        }
        let mut extra = frame.clone();
        extra.push(0);
        assert_eq!(dns_addresses(&extra, "a.test", 7, 1), Err(FetchError::Dns));
    }

    #[test]
    fn cname_is_followed_only_within_bounded_answer_chain() {
        let mut frame = dns_query("a.test", 7, 1).expect("query");
        frame[2..4].copy_from_slice(&[0x81, 0x80]);
        frame[6..8].copy_from_slice(&2_u16.to_be_bytes());
        frame.extend_from_slice(&[0xc0, 0x0c, 0, 5, 0, 1, 0, 0, 0, 1, 0, 8]);
        let alias = frame.len();
        frame.extend_from_slice(b"\x01b\x04test\0");
        frame.extend_from_slice(&[
            0xc0,
            alias as u8,
            0,
            1,
            0,
            1,
            0,
            0,
            0,
            1,
            0,
            4,
            127,
            0,
            0,
            1,
        ]);
        assert_eq!(
            dns_addresses(&frame, "a.test", 7, 1).expect("CNAME"),
            vec!["127.0.0.1".parse::<IpAddr>().expect("IP")]
        );
        // Тот же адрес с посторонним owner не становится ответом на исходное имя.
        frame[alias + 1] = b'c';
        // Переопределяем owner A на a.test: CNAME + A у одного owner неоднозначны.
        frame[alias + 9] = 0x0c;
        assert_eq!(dns_addresses(&frame, "a.test", 7, 1), Err(FetchError::Dns));
    }

    #[test]
    fn compression_loops_and_record_flood_are_rejected() {
        let mut frame = answer();
        frame[12..14].copy_from_slice(&[0xc0, 0x0c]);
        assert_eq!(dns_addresses(&frame, "a.test", 7, 1), Err(FetchError::Dns));
        let mut frame = answer();
        frame[6..8].copy_from_slice(&129_u16.to_be_bytes());
        assert_eq!(dns_addresses(&frame, "a.test", 7, 1), Err(FetchError::Dns));
    }

    #[test]
    fn explicit_upstream_requires_endpoint_and_valid_literal_address() {
        assert!(
            FetchHost::new(Duration::from_secs(1))
                .expect("host")
                .with_proxy_dns("127.0.0.1:53".parse().expect("address"))
                .is_err()
        );
        for value in [
            "0.0.0.0:53",
            "[::]:53",
            "224.0.0.1:53",
            "255.255.255.255:53",
            "127.0.0.1:0",
        ] {
            assert!(
                FetchHost::new(Duration::from_secs(1))
                    .expect("host")
                    .with_socks_endpoint("127.0.0.1:1080".parse().expect("endpoint"))
                    .expect("endpoint")
                    .with_proxy_dns(value.parse().expect("address"))
                    .is_err()
            );
        }
    }
}
