//! Извлечение управляемых серверов; маршрутизация исходного конфига не исполняется.
use crate::{
    FatalError, Field, Issue, IssueCode, MAX_ENTRY_BYTES, MAX_RECORDS, ParsedServer, ReviewFlag,
    model::RecordError, uri,
};
use serde::de::{Deserialize, Deserializer, Error, SeqAccess, Visitor};
use serde_json::{Value, value::RawValue};
use std::{collections::BTreeMap, fmt};

type RawObject<'a> = BTreeMap<String, &'a RawValue>;
type Parameters = Vec<(&'static str, String)>;
struct Records<'a>(Vec<&'a RawValue>);
impl<'de> Deserialize<'de> for Records<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct RecordsVisitor;
        impl<'de> Visitor<'de> for RecordsVisitor {
            type Value = Records<'de>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("Ограниченный массив JSON")
            }
            fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
                let mut records = Vec::new();
                while let Some(raw) = seq.next_element::<&'de RawValue>()? {
                    if records.len() == MAX_RECORDS {
                        return Err(S::Error::custom("Лимит записей"));
                    }
                    records.push(raw);
                }
                Ok(Records(records))
            }
        }
        d.deserialize_seq(RecordsVisitor)
    }
}
fn records(text: &str) -> Result<Records<'_>, FatalError> {
    serde_json::from_str(text).map_err(|_| FatalError::TooManyRecords)
}
fn issue(issues: &mut Vec<Issue>, index: usize, error: RecordError) {
    issues.push(Issue {
        line: index,
        code: error.0,
        field: error.1,
    });
}

pub(crate) fn extract(
    content: &str,
    servers: &mut Vec<ParsedServer>,
    issues: &mut Vec<Issue>,
) -> Result<(), FatalError> {
    let roots = if content.trim_start().starts_with('[') {
        records(content)?.0
    } else {
        vec![serde_json::from_str::<&RawValue>(content).map_err(|_| FatalError::InvalidJson)?]
    };
    let mut count = 0usize;
    for root in roots {
        let Ok(config) = serde_json::from_str::<RawObject<'_>>(root.get()) else {
            continue;
        };
        let Some(outbounds) = config.get("outbounds") else {
            continue;
        };
        if !outbounds.get().trim_start().starts_with('[') {
            continue;
        }
        let name = config
            .get("remarks")
            .or_else(|| config.get("tag"))
            .and_then(|r| serde_json::from_str::<String>(r.get()).ok())
            .unwrap_or_default();
        let name: String = name.chars().filter(|c| !c.is_control()).take(160).collect();
        for raw in records(outbounds.get())?.0 {
            count += 1;
            if count > MAX_RECORDS {
                return Err(FatalError::TooManyRecords);
            }
            if raw.get().len() > MAX_ENTRY_BYTES {
                issue(issues, count, (IssueCode::TooLarge, Field::Payload));
                continue;
            }
            // Каждое дерево ограничено MAX_ENTRY_BYTES; глубина проверена до extract.
            let node: Value =
                serde_json::from_str(raw.get()).map_err(|_| FatalError::InvalidJson)?;
            let Some(protocol) = node.get("protocol") else {
                continue;
            };
            match protocol.as_str() {
                Some("freedom" | "blackhole" | "dns") => continue,
                Some("vless" | "shadowsocks") => {}
                _ => {
                    issue(
                        issues,
                        count,
                        (IssueCode::UnsupportedProtocol, Field::Source),
                    );
                    continue;
                }
            }
            let normalized = if protocol.as_str() == Some("shadowsocks") {
                normalize_shadowsocks(&node, &name, count, servers)
            } else {
                normalize(&node, &name, count, servers)
            };
            match normalized {
                Ok(()) => {}
                Err(error) => issue(issues, count, error),
            }
            if servers.len() > MAX_RECORDS {
                return Err(FatalError::TooManyRecords);
            }
        }
    }
    Ok(())
}
fn invalid() -> RecordError {
    (IssueCode::InvalidJson, Field::Parameters)
}
fn text<'a>(node: &'a Value, key: &str, default: &'a str) -> Result<&'a str, RecordError> {
    node.get(key)
        .map_or(Ok(default), |v| v.as_str().ok_or_else(invalid))
}
fn unknown(node: &Value, allowed: &[&str]) -> bool {
    node.as_object()
        .is_some_and(|object| object.keys().any(|key| !allowed.contains(&key.as_str())))
}
fn normalize(
    node: &Value,
    name: &str,
    index: usize,
    servers: &mut Vec<ParsedServer>,
) -> Result<(), RecordError> {
    let settings = node
        .get("settings")
        .filter(|v| v.is_object())
        .ok_or_else(invalid)?;
    let endpoints = settings
        .get("vnext")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
        .ok_or_else(invalid)?;
    let (parameters, mut unsupported) = stream_parameters(node)?;
    unsupported |= unknown(settings, &["vnext"]);
    let label = if name.is_empty() {
        text(node, "tag", "Xray")?
    } else {
        name
    };
    let label: String = label
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect();
    let mut pending = Vec::new();
    for endpoint in endpoints {
        unsupported |= unknown(endpoint, &["address", "port", "users"]);
        let host = text(endpoint, "address", "")?;
        // Запрещаем URI-инъекцию до передачи канонической проверки endpoint в uri::parse.
        if host.is_empty()
            || host
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '?' | '#' | '@' | '%' | '[' | ']'))
        {
            return Err((IssueCode::InvalidHost, Field::Endpoint));
        }
        let port = endpoint
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|n| u16::try_from(n).ok())
            .filter(|p| *p != 0)
            .ok_or((IssueCode::InvalidPort, Field::Endpoint))?;
        let users = endpoint
            .get("users")
            .and_then(Value::as_array)
            .filter(|v| !v.is_empty())
            .ok_or((IssueCode::InvalidCredentials, Field::Credentials))?;
        for user in users {
            if servers.len() + pending.len() >= MAX_RECORDS {
                return Err((IssueCode::TooLarge, Field::Payload));
            }
            let id = text(user, "id", "")?;
            let mut pairs = parameters.clone();
            for key in ["encryption", "flow"] {
                if user.get(key).is_some() {
                    pairs.push((key, text(user, key, "")?.to_owned()));
                }
            }
            let query = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(pairs)
                .finish();
            let auth =
                percent_encoding::utf8_percent_encode(id, percent_encoding::NON_ALPHANUMERIC);
            let address = if host.contains(':') {
                format!("[{host}]")
            } else {
                host.to_owned()
            };
            let mut source = url::Url::parse(&format!("vless://{auth}@{address}:{port}?{query}"))
                .map_err(|_| (IssueCode::InvalidHost, Field::Endpoint))?;
            source.set_fragment(Some(&format!("{label} [{index}.{}]", pending.len() + 1)));
            let mut server = uri::parse(source.as_str())?;
            if unsupported
                || unknown(user, &["id", "encryption", "flow", "email", "level"])
                || user.get("level").is_some_and(|v| v.as_u64() != Some(0))
            {
                server.review_flags.push(ReviewFlag::UnsupportedOption);
            }
            pending.push(server);
        }
    }
    servers.extend(pending);
    Ok(())
}

fn stream_parameters(node: &Value) -> Result<(Parameters, bool), RecordError> {
    let stream = node.get("streamSettings");
    if stream.is_some_and(|v| !v.is_object()) {
        return Err(invalid());
    }
    let empty = serde_json::json!({});
    let stream = stream.unwrap_or(&empty);
    let network = text(stream, "network", "tcp")?;
    let security = text(stream, "security", "none")?;
    let mut parameters = vec![
        ("type", network.to_owned()),
        ("security", security.to_owned()),
    ];
    let mut unsupported = unknown(
        node,
        &["tag", "protocol", "settings", "streamSettings", "mux"],
    ) || unknown(
        stream,
        &[
            "network",
            "security",
            "tlsSettings",
            "realitySettings",
            "wsSettings",
            "grpcSettings",
            "xhttpSettings",
            "tcpSettings",
        ],
    );
    if let Some(mux) = node.get("mux") {
        unsupported |= !mux.is_object()
            || unknown(mux, &["enabled"])
            || mux.get("enabled").and_then(Value::as_bool) != Some(false);
    }
    let security_key = match security {
        "tls" => Some("tlsSettings"),
        "reality" => Some("realitySettings"),
        _ => None,
    };
    for key in ["tlsSettings", "realitySettings"] {
        if let Some(options) = stream.get(key) {
            if !options.is_object() {
                return Err(invalid());
            }
            if Some(key) != security_key {
                unsupported = true;
            }
            let allowed: &[&str] = if key == "tlsSettings" {
                &[
                    "serverName",
                    "fingerprint",
                    "alpn",
                    "allowInsecure",
                    "enableSessionResumption",
                ]
            } else {
                &[
                    "serverName",
                    "fingerprint",
                    "publicKey",
                    "shortId",
                    "spiderX",
                ]
            };
            unsupported |= unknown(options, allowed);
            if let Some(resumption) = options.get("enableSessionResumption") {
                unsupported |= resumption.as_bool() != Some(false);
            }
            for (from, to) in [
                ("serverName", "sni"),
                ("fingerprint", "fp"),
                ("publicKey", "pbk"),
                ("shortId", "sid"),
                ("spiderX", "spx"),
            ] {
                if options.get(from).is_some() {
                    parameters.push((to, text(options, from, "")?.to_owned()));
                }
            }
            if let Some(alpn) = options.get("alpn") {
                let parts = alpn
                    .as_array()
                    .ok_or_else(invalid)?
                    .iter()
                    .map(|v| v.as_str().ok_or_else(invalid))
                    .collect::<Result<Vec<_>, _>>()?;
                parameters.push(("alpn", parts.join(",")));
            }
            if let Some(insecure) = options.get("allowInsecure") {
                parameters.push((
                    "allowInsecure",
                    insecure.as_bool().ok_or_else(invalid)?.to_string(),
                ));
            }
        }
    }
    for (key, expected, allowed, mapping) in [
        (
            "wsSettings",
            "ws",
            &["path", "headers"][..],
            &[("path", "path")][..],
        ),
        (
            "grpcSettings",
            "grpc",
            &["serviceName", "authority"][..],
            &[("serviceName", "serviceName"), ("authority", "authority")][..],
        ),
        (
            "xhttpSettings",
            "xhttp",
            &["path", "host", "mode"][..],
            &[("path", "path"), ("host", "host"), ("mode", "mode")][..],
        ),
    ] {
        if let Some(options) = stream.get(key) {
            if !options.is_object() {
                return Err(invalid());
            }
            unsupported |= network != expected || unknown(options, allowed);
            for (from, to) in mapping {
                if options.get(from).is_some() {
                    parameters.push((*to, text(options, from, "")?.to_owned()));
                }
            }
            if key == "wsSettings" {
                if let Some(headers) = options.get("headers") {
                    if !headers.is_object() {
                        return Err(invalid());
                    }
                    unsupported |= unknown(headers, &["Host"]);
                    if headers.get("Host").is_some() {
                        parameters.push(("host", text(headers, "Host", "")?.to_owned()));
                    }
                }
            }
        }
    }
    if let Some(tcp) = stream.get("tcpSettings") {
        if !tcp.is_object() {
            return Err(invalid());
        }
        // Даже header.type=none проверяется явно, HTTP camouflage не теряется.
        unsupported |= unknown(tcp, &["header"]);
        if let Some(header) = tcp.get("header") {
            unsupported |= !header.is_object()
                || unknown(header, &["type"])
                || text(header, "type", "none")? != "none";
        }
    }
    Ok((parameters, unsupported))
}

fn normalize_shadowsocks(
    node: &Value,
    name: &str,
    index: usize,
    servers: &mut Vec<ParsedServer>,
) -> Result<(), RecordError> {
    let settings = node
        .get("settings")
        .filter(|v| v.is_object())
        .ok_or_else(invalid)?;
    let endpoints = settings
        .get("servers")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
        .ok_or_else(invalid)?;
    let (parameters, unsupported) = stream_parameters(node)?;
    let unsupported = unsupported || unknown(settings, &["servers"]);
    let label = if name.is_empty() {
        text(node, "tag", "Shadowsocks")?
    } else {
        name
    };
    let label: String = label
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(parameters)
        .finish();
    let mut pending = Vec::new();
    for endpoint in endpoints {
        if servers.len() + pending.len() >= MAX_RECORDS {
            return Err((IssueCode::TooLarge, Field::Payload));
        }
        let host = text(endpoint, "address", "")?;
        if host.is_empty()
            || host
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '?' | '#' | '@' | '%' | '[' | ']'))
        {
            return Err((IssueCode::InvalidHost, Field::Endpoint));
        }
        let port = endpoint
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|n| u16::try_from(n).ok())
            .filter(|p| *p != 0)
            .ok_or((IssueCode::InvalidPort, Field::Endpoint))?;
        let method = text(endpoint, "method", "")?;
        let password = text(endpoint, "password", "")?;
        if method.is_empty() || password.is_empty() {
            return Err((IssueCode::InvalidCredentials, Field::Credentials));
        }
        let method =
            percent_encoding::utf8_percent_encode(method, percent_encoding::NON_ALPHANUMERIC);
        let password =
            percent_encoding::utf8_percent_encode(password, percent_encoding::NON_ALPHANUMERIC);
        let address = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        let mut source = url::Url::parse(&format!(
            "ss://{method}:{password}@{address}:{port}?{query}"
        ))
        .map_err(|_| (IssueCode::InvalidHost, Field::Endpoint))?;
        source.set_fragment(Some(&format!("{label} [{index}.{}]", pending.len() + 1)));
        let mut server = uri::parse(source.as_str())?;
        if unsupported
            || unknown(
                endpoint,
                &["address", "port", "method", "password", "email", "level"],
            )
            || endpoint.get("level").is_some_and(|v| v.as_u64() != Some(0))
        {
            server.review_flags.push(ReviewFlag::UnsupportedOption);
        }
        pending.push(server);
    }
    servers.extend(pending);
    Ok(())
}
