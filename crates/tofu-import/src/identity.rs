use crate::{Credentials, ParsedServer, Protocol, SecretText, Security, Transport};
use percent_encoding::percent_decode_str;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::fmt;

/// Секретный материал для keyed HMAC внешнего хранилища. Не сохранять напрямую
/// и не заменять публичным hash: он стал бы проверкой подбора credentials offline.
pub struct SecretIdentity(Vec<u8>);
impl SecretIdentity {
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}
impl fmt::Debug for SecretIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretIdentity(<скрыто>)")
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityError;
impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Не удалось канонизировать профиль")
    }
}
impl std::error::Error for IdentityError {}

fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}

// Value сворачивает повторные ключи; identity обязана сохранить все опции.
enum IdentityNode {
    Object(Vec<(String, IdentityNode)>),
    Array(Vec<IdentityNode>),
    Scalar(Value),
}

impl<'de> Deserialize<'de> for IdentityNode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NodeVisitor;
        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = IdentityNode;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut fields = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    fields.push(entry);
                }
                Ok(IdentityNode::Object(fields))
            }
            fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element()? {
                    values.push(value);
                }
                Ok(IdentityNode::Array(values))
            }
            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(IdentityNode::Scalar(Value::Bool(value)))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(IdentityNode::Scalar(Value::from(value)))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(IdentityNode::Scalar(Value::from(value)))
            }
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|number| IdentityNode::Scalar(Value::Number(number)))
                    .ok_or_else(|| E::custom("Некорректное число JSON"))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(IdentityNode::Scalar(Value::String(value.to_owned())))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(IdentityNode::Scalar(Value::String(value)))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(IdentityNode::Scalar(Value::Null))
            }
        }
        deserializer.deserialize_any(NodeVisitor)
    }
}

fn canonical_json(value: &IdentityNode, out: &mut Vec<u8>) -> Result<(), IdentityError> {
    match value {
        IdentityNode::Object(object) => {
            out.push(b'o');
            let mut fields: Vec<_> = object.iter().collect();
            // Stable sort сохраняет относительный порядок одинаковых ключей:
            // first/last-wins адаптеры не должны получить одну identity при смене порядка.
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            out.extend_from_slice(&(fields.len() as u64).to_be_bytes());
            for (key, value) in fields {
                field(out, key.as_bytes());
                canonical_json(value, out)?;
            }
        }
        IdentityNode::Array(values) => {
            out.push(b'a');
            out.extend_from_slice(&(values.len() as u64).to_be_bytes());
            for value in values {
                canonical_json(value, out)?;
            }
        }
        IdentityNode::Scalar(value) => {
            out.push(b'v');
            field(out, &serde_json::to_vec(value).map_err(|_| IdentityError)?);
        }
    }
    Ok(())
}

impl ParsedServer {
    /// Версионированная identity включает все опции, включая неизвестные.
    /// Отсутствующие и явно заданные defaults консервативно различаются.
    /// URI path также включён: адаптер может поддержать его позднее.
    pub fn identity_material(&self) -> Result<SecretIdentity, IdentityError> {
        let mut out = b"tofu-profile-identity\0v1\0".to_vec();
        field(&mut out, self.protocol.as_str().as_bytes());
        field(&mut out, self.endpoint.host.as_bytes());
        field(&mut out, &self.endpoint.port.to_be_bytes());
        match &self.credentials {
            Credentials::None => field(&mut out, b"none"),
            Credentials::Uuid(value) => {
                field(&mut out, b"uuid");
                field(&mut out, value.expose().as_bytes());
            }
            Credentials::Password(value) => {
                field(&mut out, b"password");
                field(&mut out, value.expose().as_bytes());
            }
            Credentials::UserPassword { username, password } => {
                field(&mut out, b"user-password");
                field(&mut out, username.expose().as_bytes());
                field(&mut out, password.expose().as_bytes());
            }
            Credentials::Shadowsocks { method, password } => {
                field(&mut out, b"shadowsocks");
                field(&mut out, method.expose().as_bytes());
                field(&mut out, password.expose().as_bytes());
            }
        }
        field(
            &mut out,
            match self.transport {
                Transport::Tcp => b"tcp",
                Transport::WebSocket => b"websocket",
                Transport::Grpc => b"grpc",
                Transport::Xhttp => b"xhttp",
                Transport::Http2 => b"http2",
                Transport::Quic => b"quic",
                Transport::Unsupported => b"unsupported",
            },
        );
        field(
            &mut out,
            match self.security {
                Security::None => b"none",
                Security::Tls => b"tls",
                Security::Reality => b"reality",
                Security::Unsupported => b"unsupported",
            },
        );
        if self.protocol == Protocol::Vmess {
            let mut value: IdentityNode =
                serde_json::from_str(self.parameters.expose()).map_err(|_| IdentityError)?;
            let IdentityNode::Object(object) = &mut value else {
                return Err(IdentityError);
            };
            // Верхний уровень уже проверен импортёром на уникальность ключей.
            object.retain(|(key, _)| !matches!(key.as_str(), "ps" | "add" | "port" | "id"));
            object.push((
                "add".into(),
                IdentityNode::Scalar(Value::String(self.endpoint.host.clone())),
            ));
            object.push((
                "port".into(),
                IdentityNode::Scalar(Value::from(self.endpoint.port)),
            ));
            if let Credentials::Uuid(id) = &self.credentials {
                object.push((
                    "id".into(),
                    IdentityNode::Scalar(Value::String(id.expose().to_owned())),
                ));
            }
            canonical_json(&value, &mut out)?;
        } else {
            let mut pairs: Vec<_> =
                url::form_urlencoded::parse(self.parameters.expose().as_bytes()).collect();
            pairs.sort_unstable();
            out.extend_from_slice(&(pairs.len() as u64).to_be_bytes());
            for (key, value) in pairs {
                field(&mut out, key.as_bytes());
                field(&mut out, value.as_bytes());
            }
            // Legacy SS не содержит URI path; decoded endpoint уже в модели.
            let source = self.source.expose().split('#').next().unwrap_or_default();
            let legacy_ss = self.protocol == Protocol::Shadowsocks
                && source
                    .split_once("://")
                    .is_some_and(|(_, body)| !body.contains('@'));
            let path = if legacy_ss {
                None
            } else {
                url::Url::parse(source).ok()
            }
            .map(|url| {
                if url.path() == "/" {
                    Vec::new()
                } else {
                    // Reserved escape (%2F и др.) не равен разделителю пути.
                    url.path().as_bytes().to_vec()
                }
            })
            .unwrap_or_default();
            field(&mut out, &path);
        }
        Ok(SecretIdentity(out))
    }

    /// Имя предназначено только для показа; исходник остаётся неизменным.
    pub fn display_name(&self) -> SecretText {
        let candidate = if self.protocol == Protocol::Vmess {
            serde_json::from_str::<Value>(self.parameters.expose())
                .ok()
                .and_then(|v| v.get("ps").and_then(Value::as_str).map(str::to_owned))
        } else {
            self.source
                .expose()
                .split_once('#')
                .and_then(|(_, fragment)| {
                    let bytes = fragment.as_bytes();
                    let valid = bytes.iter().enumerate().all(|(i, b)| {
                        *b != b'%'
                            || (i + 2 < bytes.len()
                                && bytes[i + 1].is_ascii_hexdigit()
                                && bytes[i + 2].is_ascii_hexdigit())
                    });
                    valid
                        .then(|| {
                            percent_decode_str(fragment)
                                .decode_utf8()
                                .ok()
                                .map(|s| s.into_owned())
                        })
                        .flatten()
                })
        };
        let name = sanitize(candidate.as_deref().unwrap_or_default());
        if !name.is_empty() {
            return SecretText::new(name);
        }
        let host = &self.endpoint.host;
        let fallback = if host.contains(':') {
            format!("[{host}]:{}", self.endpoint.port)
        } else {
            format!("{host}:{}", self.endpoint.port)
        };
        SecretText::new(sanitize(&fallback))
    }
}

fn sanitize(text: &str) -> String {
    let mut result = String::new();
    for c in text.chars().filter(|c| !c.is_control() && !matches!(*c,
        '\u{ad}' | '\u{600}'..='\u{605}' | '\u{61c}' | '\u{6dd}' | '\u{70f}' |
        '\u{890}'..='\u{891}' | '\u{8e2}' | '\u{180e}' | '\u{200b}'..='\u{200f}' |
        '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' |
        '\u{fff9}'..='\u{fffb}' | '\u{110bd}' | '\u{110cd}' | '\u{13430}'..='\u{1343f}' |
        '\u{1bca0}'..='\u{1bca3}' | '\u{1d173}'..='\u{1d17a}' | '\u{e0001}' | '\u{e0020}'..='\u{e007f}')) {
        if result.len() + c.len_utf8() > 512 { break; }
        result.push(c);
    }
    result.trim().to_owned()
}
