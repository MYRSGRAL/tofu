//! Ограниченная нормализация sing-box. Непредставимые опции требуют review.
use crate::{Field, IssueCode, ParsedServer, ReviewFlag, model::RecordError, uri};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

fn invalid() -> RecordError {
    (IssueCode::InvalidJson, Field::Parameters)
}
fn text<'a>(v: &'a Value, key: &str, default: &'a str) -> Result<&'a str, RecordError> {
    v.get(key)
        .map_or(Ok(default), |x| x.as_str().ok_or_else(invalid))
}
fn unknown(v: &Value, allowed: &[&str]) -> bool {
    !v.is_object()
        || v.as_object()
            .is_some_and(|o| o.keys().any(|k| !allowed.contains(&k.as_str())))
}
pub(crate) fn normalize(v: &Value) -> Result<ParsedServer, RecordError> {
    let protocol = text(v, "type", "")?;
    let scheme = match protocol {
        "vless" | "vmess" | "trojan" => protocol,
        "shadowsocks" => "ss",
        "http" => "http",
        "socks" => "socks5",
        _ => return Err((IssueCode::UnsupportedProtocol, Field::Source)),
    };
    let host = text(v, "server", "")?;
    if host.is_empty()
        || host
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "/?#@%[]\\".contains(c))
    {
        return Err((IssueCode::InvalidHost, Field::Endpoint));
    }
    let port = v
        .get("server_port")
        .and_then(Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0)
        .ok_or((IssueCode::InvalidPort, Field::Endpoint))?;
    let mut unsupported = unknown(
        v,
        &[
            "type",
            "tag",
            "server",
            "server_port",
            "uuid",
            "password",
            "username",
            "method",
            "flow",
            "security",
            "alter_id",
            "version",
            "tls",
            "transport",
        ],
    );
    // Поля другого протокола не должны терять смысл при нормализации.
    for (key, allowed) in [
        ("uuid", matches!(protocol, "vless" | "vmess")),
        ("flow", protocol == "vless"),
        ("security", protocol == "vmess"),
        ("alter_id", protocol == "vmess"),
        ("method", protocol == "shadowsocks"),
        ("version", protocol == "socks"),
        ("username", matches!(protocol, "http" | "socks")),
        ("password", !matches!(protocol, "vless" | "vmess")),
    ] {
        unsupported |= !allowed && v.get(key).is_some();
    }
    unsupported |= v.get("version").is_some_and(|x| x.as_str() != Some("5"));
    unsupported |= v.get("alter_id").is_some_and(|x| x.as_u64() != Some(0));
    let mut pairs: Vec<(&str, String)> = vec![];
    let mut tls = false;
    if let Some(t) = v.get("tls") {
        unsupported |= unknown(t, &["enabled", "server_name", "alpn", "insecure"]);
        tls = t
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(invalid)?;
        unsupported |= !tls && t.as_object().is_some_and(|o| o.len() > 1);
        if let Some(insecure) = t.get("insecure") {
            unsupported |= insecure.as_bool() != Some(false);
        }
        if t.get("server_name").is_some() {
            pairs.push(("sni", text(t, "server_name", "")?.into()));
        }
        if let Some(a) = t.get("alpn") {
            let a = a
                .as_array()
                .ok_or_else(invalid)?
                .iter()
                .map(|x| x.as_str().filter(|s| !s.contains(',')).ok_or_else(invalid))
                .collect::<Result<Vec<_>, _>>()?;
            pairs.push(("alpn", a.join(",")));
        }
    }
    pairs.push(("security", if tls { "tls" } else { "none" }.into()));
    let mut network = "tcp";
    if let Some(t) = v.get("transport") {
        network = text(t, "type", "")?;
        match network {
            "ws" => {
                unsupported |= unknown(t, &["type", "path", "headers"]);
                if t.get("path").is_some() {
                    pairs.push(("path", text(t, "path", "")?.into()));
                }
                if let Some(h) = t.get("headers") {
                    unsupported |= unknown(h, &["Host"]);
                    if h.get("Host").is_some() {
                        pairs.push(("host", text(h, "Host", "")?.into()));
                    }
                }
            }
            "grpc" => {
                unsupported |= unknown(t, &["type", "service_name"]);
                if t.get("service_name").is_some() {
                    pairs.push(("serviceName", text(t, "service_name", "")?.into()));
                }
            }
            _ => {
                unsupported = true;
                network = "tcp";
            }
        }
    }
    pairs.push(("type", network.into()));
    if v.get("flow").is_some() {
        pairs.push(("flow", text(v, "flow", "")?.into()));
    }
    let encode = |s: &str| {
        percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
    };
    let auth = match protocol {
        "vless" => encode(text(v, "uuid", "")?),
        "trojan" => encode(text(v, "password", "")?),
        "shadowsocks" => format!(
            "{}:{}",
            encode(text(v, "method", "")?),
            encode(text(v, "password", "")?)
        ),
        "http" | "socks" => {
            let u = text(v, "username", "")?;
            let p = text(v, "password", "")?;
            if u.is_empty() && p.is_empty() {
                String::new()
            } else {
                format!("{}:{}", encode(u), encode(p))
            }
        }
        _ => String::new(),
    };
    let scheme = if protocol == "http" && tls {
        "https"
    } else {
        scheme
    };
    unsupported |= tls && matches!(protocol, "shadowsocks" | "socks");
    unsupported |=
        v.get("transport").is_some() && !matches!(protocol, "vless" | "vmess" | "trojan");
    let mut server = if protocol == "vmess" {
        let mut envelope = json!({"v":"2","aid":"0","add":host,"port":port,"id":text(v,"uuid", "")?,"net":network,"tls":if tls {"tls"} else {""},"scy":text(v,"security", "auto")?});
        for (k, val) in &pairs {
            let key = match *k {
                "type" | "security" => continue,
                "serviceName" => "path",
                other => other,
            };
            envelope[key] = json!(val);
        }
        uri::parse(&format!(
            "vmess://{}",
            STANDARD.encode(envelope.to_string())
        ))?
    } else {
        let address = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.into()
        };
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish();
        uri::parse(&format!(
            "{scheme}://{}{address}:{port}?{query}",
            if auth.is_empty() {
                String::new()
            } else {
                format!("{auth}@")
            }
        ))?
    };
    if unsupported {
        server.review_flags.push(ReviewFlag::UnsupportedOption);
    }
    Ok(server)
}
