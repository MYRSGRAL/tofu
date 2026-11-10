//! Закрытый переносимый контракт сервера. Сырой URI и настройки ядра не принимаются.
use crate::{Credentials, Endpoint, ParsedServer, Protocol, SecretText, Security, Transport};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as _, MapAccess, Visitor},
    ser::SerializeStruct,
};
use std::fmt;

const MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentError {
    TooLarge,
    InvalidJson,
    InvalidValue,
    Unsupported,
}
impl fmt::Display for IntentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "Превышен размер намерения",
            Self::InvalidJson => "Некорректный контракт JSON",
            Self::InvalidValue => "Некорректное значение намерения",
            Self::Unsupported => "Неподдерживаемые настройки сервера",
        })
    }
}
impl std::error::Error for IntentError {}

// Ручной эквивалент derive + deny_unknown_fields: crate не включает serde/derive.
// Все поля обязательны, включая nullable: отсутствующее поле не подменяет значение.
macro_rules! closed_dto {
    ($(#[$meta:meta])* $name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        $(#[$meta])*
        struct $name { $($field: $ty),* }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut object = serializer.serialize_struct(stringify!($name), 0 $(+ { let _ = stringify!($field); 1 })*)?;
                $(object.serialize_field(stringify!($field), &self.$field)?;)*
                object.end()
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct Object;
                impl<'de> Visitor<'de> for Object {
                    type Value = $name;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("Закрытый объект намерения") }
                    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                        $(let mut $field: Option<$ty> = None;)*
                        while let Some(key) = map.next_key::<String>()? {
                            match key.as_str() {
                                $(stringify!($field) => { if $field.is_some() { return Err(M::Error::custom("Повтор поля")); } $field = Some(map.next_value()?); })*
                                _ => return Err(M::Error::custom("Неизвестное поле")),
                            }
                        }
                        Ok($name { $($field: $field.ok_or_else(|| M::Error::custom("Отсутствует поле"))?),* })
                    }
                }
                deserializer.deserialize_map(Object)
            }
        }
    }
}
closed_dto!(Wire {
    protocol: String,
    host: String,
    port: u16,
    credentials: Auth,
    transport: String,
    security: String,
    options: Options
});
closed_dto!(Auth { kind: String, uuid: Option<SecretString>, username: Option<SecretString>, password: Option<SecretString>, method: Option<SecretString> });
closed_dto!(#[derive(Default)] Options { sni: Option<String>, fp: Option<String>, alpn: Option<Vec<String>>, pbk: Option<String>, sid: Option<String>, flow: Option<String>, host: Option<String>, path: Option<String>, mode: Option<String>, service_name: Option<String>, authority: Option<String>, spx: Option<String>, cipher: Option<String> });
/// Строки учётных данных скрываются и при диагностике внутренних DTO.
struct SecretString(String);
impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}
impl std::ops::Deref for SecretString {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<скрыто>")
    }
}
impl Serialize for SecretString {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}
impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Self)
    }
}

/// Сериализация содержит секреты: передавать только по защищённому каналу.
/// Вход разрешён только через bounded `parse_json`, не через общий Deserialize.
///
/// ```compile_fail
/// let _: yonik_import::ServerIntent = serde_json::from_str("{}").unwrap();
/// ```
pub struct ServerIntent(Wire);
impl fmt::Debug for ServerIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServerIntent(<скрыто>)")
    }
}
impl Serialize for ServerIntent {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}
impl ServerIntent {
    pub fn parse_json(text: &str) -> Result<Self, IntentError> {
        if text.len() > MAX_BYTES {
            return Err(IntentError::TooLarge);
        }
        // У закрытого DTO максимальная глубина — пять контейнеров.
        let mut depth = 0u8;
        let mut quoted = false;
        let mut escaped = false;
        for byte in text.bytes() {
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
            } else {
                match byte {
                    b'"' => quoted = true,
                    b'{' | b'[' => {
                        depth += 1;
                        if depth > 5 {
                            return Err(IntentError::InvalidJson);
                        }
                    }
                    b'}' | b']' => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
        }
        let intent = Self(serde_json::from_str(text).map_err(|_| IntentError::InvalidJson)?);
        intent.validate()?;
        Ok(intent)
    }

    pub fn from_server(server: &ParsedServer) -> Result<Self, IntentError> {
        if !server.review_flags.is_empty() {
            return Err(IntentError::Unsupported);
        }
        if server.parameters.expose().len() > MAX_BYTES {
            return Err(IntentError::TooLarge);
        }
        let mut options = Options::default();
        if server.protocol == Protocol::Vmess {
            let vmess = read_vmess(server.parameters.expose())?;
            options = vmess;
        } else {
            let mut seen = std::collections::HashSet::new();
            for (key, value) in url::form_urlencoded::parse(server.parameters.expose().as_bytes()) {
                if !seen.insert(key.clone()) {
                    return Err(IntentError::Unsupported);
                }
                match key.as_ref() {
                    "type" | "security" => {}
                    "encryption" if value == "none" && server.protocol == Protocol::Vless => {}
                    "allowInsecure" if value == "0" || value == "false" => {}
                    "headerType" if value.is_empty() || value == "none" => {}
                    _ => options.insert(&key, value.into_owned())?,
                }
            }
        }
        let mut auth = Auth {
            kind: "none".into(),
            uuid: None,
            username: None,
            password: None,
            method: None,
        };
        match &server.credentials {
            Credentials::None => {}
            Credentials::Uuid(value) => {
                auth.kind = "uuid".into();
                auth.uuid = Some(value.expose().into());
            }
            Credentials::Password(value) => {
                auth.kind = "password".into();
                auth.password = Some(value.expose().into());
            }
            Credentials::UserPassword { username, password } => {
                auth.kind = "user_password".into();
                auth.username = Some(username.expose().into());
                auth.password = Some(password.expose().into());
            }
            Credentials::Shadowsocks { method, password } => {
                auth.kind = "shadowsocks".into();
                auth.method = Some(method.expose().into());
                auth.password = Some(password.expose().into());
            }
        }
        let transport = match server.transport {
            Transport::Tcp => "tcp",
            Transport::WebSocket => "ws",
            Transport::Grpc => "grpc",
            Transport::Xhttp => "xhttp",
            _ => return Err(IntentError::Unsupported),
        };
        let security = match server.security {
            Security::None => "none",
            Security::Tls => "tls",
            Security::Reality => "reality",
            _ => return Err(IntentError::Unsupported),
        };
        let result = Self(Wire {
            protocol: server.protocol.as_str().into(),
            host: server.endpoint.host.clone(),
            port: server.endpoint.port,
            credentials: auth,
            transport: transport.into(),
            security: security.into(),
            options,
        });
        result.validate()?;
        if serde_json::to_vec(&result)
            .map_err(|_| IntentError::InvalidValue)?
            .len()
            > MAX_BYTES
        {
            return Err(IntentError::TooLarge);
        }
        Ok(result)
    }

    pub fn into_server(self) -> Result<ParsedServer, IntentError> {
        self.validate()?;
        let w = self.0;
        let protocol = protocol(&w.protocol)?;
        let transport = match w.transport.as_str() {
            "tcp" => Transport::Tcp,
            "ws" => Transport::WebSocket,
            "grpc" => Transport::Grpc,
            "xhttp" => Transport::Xhttp,
            _ => return Err(IntentError::Unsupported),
        };
        let security = match w.security.as_str() {
            "none" => Security::None,
            "tls" => Security::Tls,
            "reality" => Security::Reality,
            _ => return Err(IntentError::Unsupported),
        };
        let parameters = if protocol == Protocol::Vmess {
            vmess_parameters(&w)?
        } else {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query
                .append_pair("type", &w.transport)
                .append_pair("security", &w.security);
            for (key, value) in w.options.pairs() {
                query.append_pair(key, &value);
            }
            query.finish()
        };
        let auth = w.credentials;
        let secret = |value: Option<SecretString>| {
            SecretText::new(value.map(|secret| secret.0).unwrap_or_default())
        };
        let credentials = match auth.kind.as_str() {
            "none" => Credentials::None,
            "uuid" => Credentials::Uuid(secret(auth.uuid)),
            "password" => Credentials::Password(secret(auth.password)),
            "user_password" => Credentials::UserPassword {
                username: secret(auth.username),
                password: secret(auth.password),
            },
            "shadowsocks" => Credentials::Shadowsocks {
                method: secret(auth.method),
                password: secret(auth.password),
            },
            _ => return Err(IntentError::InvalidValue),
        };
        Ok(ParsedServer {
            protocol,
            endpoint: Endpoint {
                host: normalize_host(&w.host)?,
                port: w.port,
            },
            credentials,
            transport,
            security,
            parameters: SecretText::new(parameters),
            source: SecretText::new(String::new()),
            review_flags: Vec::new(),
        })
    }

    fn validate(&self) -> Result<(), IntentError> {
        let w = &self.0;
        let p = protocol(&w.protocol)?;
        normalize_host(&w.host)?;
        if w.port == 0
            || !matches!(w.transport.as_str(), "tcp" | "ws" | "grpc" | "xhttp")
            || !matches!(w.security.as_str(), "none" | "tls" | "reality")
        {
            return Err(IntentError::InvalidValue);
        }
        if !matches!(p, Protocol::Vless | Protocol::Vmess | Protocol::Trojan)
            && w.transport != "tcp"
            || matches!(p, Protocol::Shadowsocks | Protocol::Socks5) && w.security != "none"
            || p == Protocol::Trojan && w.security != "tls"
            || w.security == "reality" && (p != Protocol::Vless || w.transport != "tcp")
        {
            return Err(IntentError::Unsupported);
        }
        let a = &w.credentials;
        for value in [&a.uuid, &a.username, &a.password, &a.method]
            .into_iter()
            .flatten()
        {
            text(value, 4096)?;
        }
        let present = (
            a.uuid.is_some(),
            a.username.is_some(),
            a.password.is_some(),
            a.method.is_some(),
        );
        match (p, a.kind.as_str(), present) {
            (Protocol::Vless | Protocol::Vmess, "uuid", (true, false, false, false)) => {
                let id = a.uuid.as_deref().ok_or(IntentError::InvalidValue)?;
                if id.len() != 36
                    || !id.bytes().enumerate().all(|(i, b)| {
                        if [8, 13, 18, 23].contains(&i) {
                            b == b'-'
                        } else {
                            b.is_ascii_hexdigit()
                        }
                    })
                {
                    return Err(IntentError::InvalidValue);
                }
            }
            (Protocol::Trojan, "password", (false, false, true, false))
                if a.password.as_deref().is_some_and(|s| !s.is_empty()) => {}
            (Protocol::HttpConnect | Protocol::Socks5, "none", (false, false, false, false)) => {}
            (
                Protocol::HttpConnect | Protocol::Socks5,
                "user_password",
                (false, true, true, false),
            ) => {}
            (Protocol::Shadowsocks, "shadowsocks", (false, false, true, true))
                if a.password.as_deref().is_some_and(|s| !s.is_empty())
                    && matches!(
                        a.method.as_deref(),
                        Some(
                            "aes-128-gcm"
                                | "aes-256-gcm"
                                | "chacha20-ietf-poly1305"
                                | "2022-blake3-aes-128-gcm"
                                | "2022-blake3-aes-256-gcm"
                                | "2022-blake3-chacha20-poly1305"
                        )
                    ) => {}
            _ => return Err(IntentError::InvalidValue),
        }
        w.options.validate(p, &w.transport, &w.security)
    }
}
fn protocol(text: &str) -> Result<Protocol, IntentError> {
    match text {
        "vless" => Ok(Protocol::Vless),
        "vmess" => Ok(Protocol::Vmess),
        "trojan" => Ok(Protocol::Trojan),
        "shadowsocks" => Ok(Protocol::Shadowsocks),
        "http_connect" => Ok(Protocol::HttpConnect),
        "socks5" => Ok(Protocol::Socks5),
        _ => Err(IntentError::Unsupported),
    }
}
fn text(value: &str, max: usize) -> Result<(), IntentError> {
    if value.len() > max || value.chars().any(char::is_control) {
        Err(IntentError::InvalidValue)
    } else {
        Ok(())
    }
}
fn normalize_host(value: &str) -> Result<String, IntentError> {
    text(value, 253)?;
    if value.is_empty()
        || value.chars().any(char::is_whitespace)
        || value.contains(['/', '\\', '@', '#', '?', '%'])
    {
        return Err(IntentError::InvalidValue);
    }
    if let Ok(ip) = value.parse::<std::net::IpAddr>() {
        return Ok(ip.to_string());
    }
    let host = url::Host::parse(value).map_err(|_| IntentError::InvalidValue)?;
    match host {
        url::Host::Domain(domain)
            if domain.len() <= 253
                && domain.trim_end_matches('.').split('.').all(|part| {
                    !part.is_empty()
                        && part.len() <= 63
                        && !part.starts_with('-')
                        && !part.ends_with('-')
                        && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                }) =>
        {
            Ok(domain)
        }
        url::Host::Ipv4(ip) => Ok(ip.to_string()),
        _ => Err(IntentError::InvalidValue),
    }
}
impl Options {
    fn insert(&mut self, key: &str, value: String) -> Result<(), IntentError> {
        let slot = match key {
            "sni" => &mut self.sni,
            "fp" => &mut self.fp,
            "pbk" => &mut self.pbk,
            "sid" => &mut self.sid,
            "flow" => &mut self.flow,
            "host" => &mut self.host,
            "path" => &mut self.path,
            "mode" => &mut self.mode,
            "serviceName" => &mut self.service_name,
            "authority" => &mut self.authority,
            "spx" => &mut self.spx,
            "scy" => &mut self.cipher,
            "alpn" => {
                if self.alpn.is_some() {
                    return Err(IntentError::Unsupported);
                }
                self.alpn = Some(value.split(',').map(str::to_owned).collect());
                return Ok(());
            }
            _ => return Err(IntentError::Unsupported),
        };
        if slot.replace(value).is_some() {
            return Err(IntentError::Unsupported);
        }
        Ok(())
    }
    fn pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = Vec::new();
        for (key, value) in [
            ("sni", &self.sni),
            ("fp", &self.fp),
            ("pbk", &self.pbk),
            ("sid", &self.sid),
            ("flow", &self.flow),
            ("host", &self.host),
            ("path", &self.path),
            ("mode", &self.mode),
            ("serviceName", &self.service_name),
            ("authority", &self.authority),
            ("spx", &self.spx),
            ("scy", &self.cipher),
        ] {
            if let Some(value) = value {
                pairs.push((key, value.clone()));
            }
        }
        if let Some(alpn) = &self.alpn {
            pairs.push(("alpn", alpn.join(",")));
        }
        pairs
    }
    fn validate(
        &self,
        protocol: Protocol,
        transport: &str,
        security: &str,
    ) -> Result<(), IntentError> {
        for (_, value) in self.pairs() {
            text(&value, 4096)?;
        }
        if security == "none"
            && [&self.sni, &self.fp, &self.pbk, &self.sid, &self.spx]
                .into_iter()
                .any(Option::is_some)
            || security != "tls" && self.alpn.is_some()
            || security != "reality"
                && [&self.pbk, &self.sid, &self.spx]
                    .into_iter()
                    .any(Option::is_some)
            || !matches!(transport, "ws" | "xhttp")
                && [&self.host, &self.path].into_iter().any(Option::is_some)
            || transport != "grpc"
                && [&self.service_name, &self.authority]
                    .into_iter()
                    .any(Option::is_some)
            || !matches!(transport, "grpc" | "xhttp") && self.mode.is_some()
            || protocol != Protocol::Vmess && self.cipher.is_some()
        {
            return Err(IntentError::Unsupported);
        }
        if let Some(sni) = &self.sni {
            if !sni.is_empty() {
                normalize_host(sni)?;
            }
        }
        if let Some(fp) = &self.fp {
            if !matches!(
                fp.as_str(),
                "" | "chrome" | "firefox" | "safari" | "ios" | "android" | "edge"
            ) {
                return Err(IntentError::Unsupported);
            }
        }
        if let Some(alpn) = &self.alpn {
            if alpn.len() > 16
                || alpn.iter().any(|s| {
                    s.is_empty()
                        || s.len() > 255
                        || !s.bytes().all(|b| b.is_ascii_graphic())
                        || s.contains(',')
                })
            {
                return Err(IntentError::InvalidValue);
            }
        }
        if security == "reality" {
            if self
                .pbk
                .as_ref()
                .and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())
                .is_none_or(|bytes| bytes.len() != 32)
                || self.fp.as_ref().is_none_or(String::is_empty)
            {
                return Err(IntentError::InvalidValue);
            }
            if let Some(sid) = &self.sid {
                if sid.len() > 16
                    || sid.len() % 2 != 0
                    || !sid.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(IntentError::InvalidValue);
                }
            }
        }
        if let Some(flow) = &self.flow {
            if !flow.is_empty()
                && (flow != "xtls-rprx-vision"
                    || protocol != Protocol::Vless
                    || transport != "tcp"
                    || security == "none")
            {
                return Err(IntentError::Unsupported);
            }
        }
        if self
            .host
            .as_ref()
            .is_some_and(|host| host.chars().any(char::is_whitespace))
        {
            // HTTP Host является authority: порт и IPv6-скобки допустимы.
            // Проверка control chars и длины выполнена для всех options выше.
            return Err(IntentError::InvalidValue);
        }
        if let Some(path) = &self.path {
            if !path.is_empty() && !path.starts_with('/') {
                return Err(IntentError::InvalidValue);
            }
            if transport == "ws" && path.contains('?') {
                return Err(IntentError::Unsupported);
            }
        }
        if let Some(spx) = &self.spx {
            if !spx.is_empty()
                && (!spx.starts_with('/')
                    || spx.starts_with("//")
                    || spx.contains(['\\', '#'])
                    || spx.chars().any(char::is_whitespace))
            {
                return Err(IntentError::InvalidValue);
            }
        }
        if let Some(mode) = &self.mode {
            if !(transport == "grpc" && mode == "gun"
                || transport == "xhttp"
                    && matches!(
                        mode.as_str(),
                        "" | "auto" | "packet-up" | "stream-up" | "stream-one"
                    ))
            {
                return Err(IntentError::Unsupported);
            }
        }
        if let Some(authority) = &self.authority {
            if !authority.is_empty() {
                if authority.contains(['/', '\\', '@', '#', '?', '%'])
                    || !authority.bytes().all(|b| b.is_ascii_graphic())
                {
                    return Err(IntentError::InvalidValue);
                }
                let url = url::Url::parse(&format!("https://{authority}/"))
                    .map_err(|_| IntentError::InvalidValue)?;
                normalize_host(
                    url.host_str()
                        .ok_or(IntentError::InvalidValue)?
                        .trim_matches(['[', ']']),
                )?;
            }
        }
        if let Some(cipher) = &self.cipher {
            if !matches!(
                cipher.as_str(),
                "" | "auto" | "aes-128-gcm" | "chacha20-poly1305"
            ) {
                return Err(IntentError::Unsupported);
            }
        }
        Ok(())
    }
}

// VMess-импорт содержит фиксированный envelope. Читаем только скаляры без Value/map.
fn read_vmess(text: &str) -> Result<Options, IntentError> {
    struct Envelope(Options);
    impl<'de> Deserialize<'de> for Envelope {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct Object;
            impl<'de> Visitor<'de> for Object {
                type Value = Envelope;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("VMess envelope")
                }
                fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Envelope, M::Error> {
                    let mut seen = std::collections::HashSet::new();
                    let mut options = Options::default();
                    let mut grpc = false;
                    let mut path = None;
                    while let Some(key) = map.next_key::<String>()? {
                        if !seen.insert(key.clone()) {
                            return Err(M::Error::custom("Повтор поля"));
                        }
                        if matches!(key.as_str(), "port" | "v" | "aid") {
                            let scalar = map.next_value::<Scalar>()?.0;
                            if key == "v" && scalar != "2" || key == "aid" && scalar != "0" {
                                return Err(M::Error::custom("Неподдерживаемый envelope"));
                            }
                        } else {
                            let value = map.next_value::<String>()?;
                            if value.is_empty() {
                                continue;
                            }
                            match key.as_str() {
                                "ps" | "add" | "id" | "tls" => {}
                                "net" => grpc = value == "grpc",
                                "type" if value.is_empty() || value == "none" => {}
                                "path" => path = Some(value),
                                _ => options
                                    .insert(&key, value)
                                    .map_err(|_| M::Error::custom("Неподдерживаемое поле"))?,
                            }
                        }
                    }
                    if grpc {
                        options.service_name = path;
                    } else {
                        options.path = path;
                    }
                    Ok(Envelope(options))
                }
            }
            d.deserialize_map(Object)
        }
    }
    serde_json::from_str::<Envelope>(text)
        .map(|e| e.0)
        .map_err(|_| IntentError::Unsupported)
}
struct Scalar(String);
impl<'de> Deserialize<'de> for Scalar {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ScalarVisitor;
        impl Visitor<'_> for ScalarVisitor {
            type Value = Scalar;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("Строка или целое")
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<Scalar, E> {
                Ok(Scalar(s.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, n: u64) -> Result<Scalar, E> {
                Ok(Scalar(n.to_string()))
            }
        }
        d.deserialize_any(ScalarVisitor)
    }
}
fn vmess_parameters(wire: &Wire) -> Result<String, IntentError> {
    struct Envelope<'a>(&'a Wire);
    impl Serialize for Envelope<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;
            let w = self.0;
            let mut map = s.serialize_map(None)?;
            map.serialize_entry("v", "2")?;
            map.serialize_entry("aid", "0")?;
            map.serialize_entry("add", &w.host)?;
            map.serialize_entry("port", &w.port)?;
            map.serialize_entry("id", &w.credentials.uuid)?;
            map.serialize_entry("net", &w.transport)?;
            map.serialize_entry("tls", &w.security)?;
            for (key, value) in w.options.pairs() {
                map.serialize_entry(if key == "serviceName" { "path" } else { key }, &value)?;
            }
            map.end()
        }
    }
    serde_json::to_string(&Envelope(wire)).map_err(|_| IntentError::InvalidValue)
}
