//! Типизированные факты Varlink: отсутствие nullable-поля не заменяется значением по умолчанию.

use super::ResolverError;
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

const MAX_CONFIGURATIONS: usize = 4096;
const MAX_SERVERS: usize = 4096;
const MAX_DOMAINS: usize = 16384;
const MAX_STRING: usize = 1024;

/// Неизвестные поля доступны вызывающему коду, но не попадают в диагностику.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct UnknownFields(pub BTreeMap<String, Value>);
impl fmt::Debug for UnknownFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnknownFields")
            .field("count", &self.0.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum Supported<T> {
    Known(T),
    Unsupported(String),
}
impl<T: fmt::Debug> fmt::Debug for Supported<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Known(value) => value.fmt(f),
            Self::Unsupported(_) => f.write_str("Unsupported(<скрыто>)"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobalModeKnown {
    Stub,
    Static,
    Uplink,
    Foreign,
    Missing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DnssecModeKnown {
    No,
    Yes,
    AllowDowngrade,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DnsOverTlsModeKnown {
    No,
    Yes,
    Opportunistic,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveSupportKnown {
    No,
    Yes,
    Resolve,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DnsProtocolKnown {
    Dns,
    Mdns,
    Llmnr,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolverScope {
    Global,
    Link(u32),
    Delegate,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ResolverConfiguration {
    pub entries: Vec<DnsConfiguration>,
    /// Есть неизвестные поля/режимы либо delegate с неподдержанной семантикой.
    /// Nullable-поля дополнительно описывают доступность каждого отдельного факта.
    pub incomplete: bool,
    pub unknown_field_count: usize,
    pub unknown_fields: UnknownFields,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DnsConfiguration {
    pub scope: ResolverScope,
    pub ifindex: Option<u32>,
    pub ifname: Option<String>,
    pub delegate: Option<String>,
    pub default_route: Option<bool>,
    pub current_server: Option<DnsServer>,
    pub servers: Option<Vec<DnsServer>>,
    pub fallback_servers: Option<Vec<DnsServer>>,
    pub search_domains: Option<Vec<SearchDomain>>,
    pub negative_trust_anchors: Option<Vec<String>>,
    pub dnssec: Option<Supported<DnssecModeKnown>>,
    pub dnssec_supported: Option<bool>,
    pub dns_over_tls: Option<Supported<DnsOverTlsModeKnown>>,
    pub llmnr: Option<Supported<ResolveSupportKnown>>,
    pub mdns: Option<Supported<ResolveSupportKnown>>,
    pub resolv_conf_mode: Option<Supported<GlobalModeKnown>>,
    pub scopes: Option<Vec<DnsScope>>,
    pub unknown_fields: UnknownFields,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DnsServer {
    pub address: IpAddr,
    /// Подсказка отображения; источником адреса служит только `address`.
    pub address_string: Option<String>,
    pub port: u16,
    pub ifindex: Option<u32>,
    pub name: Option<String>,
    pub accessible: bool,
    pub unknown_fields: UnknownFields,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SearchDomain {
    pub name: String,
    pub route_only: bool,
    pub ifindex: Option<u32>,
    pub unknown_fields: UnknownFields,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DnsScope {
    pub protocol: Supported<DnsProtocolKnown>,
    /// Linux AF_INET (2), AF_INET6 (10); отсутствие означает AF_UNSPEC.
    pub family: Option<u8>,
    pub ifindex: Option<u32>,
    pub ifname: Option<String>,
    pub dnssec: Option<Supported<DnssecModeKnown>>,
    pub dns_over_tls: Option<Supported<DnsOverTlsModeKnown>>,
    pub unknown_fields: UnknownFields,
}

// Эти модели содержат адреса, зоны, SNI и непрозрачные будущие данные.
macro_rules! redacted_debug {
    ($($name:ty),+ $(,)?) => {$(
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), "(<скрыто>)"))
            }
        }
    )+};
}
redacted_debug!(
    ResolverConfiguration,
    DnsConfiguration,
    DnsServer,
    SearchDomain,
    DnsScope
);

#[derive(Default)]
struct Context {
    servers: usize,
    domains: usize,
    scopes: usize,
    unknown: usize,
    incomplete: bool,
}

impl Context {
    fn unknown(&mut self, fields: Map<String, Value>) -> UnknownFields {
        self.unknown += fields.len();
        self.incomplete |= !fields.is_empty();
        UnknownFields(fields.into_iter().collect())
    }

    fn mode<T>(
        &mut self,
        value: Value,
        known: fn(&str) -> Option<T>,
    ) -> Result<Supported<T>, ResolverError> {
        let value = string(value)?;
        Ok(match known(&value) {
            Some(mode) => Supported::Known(mode),
            None => {
                self.incomplete = true;
                Supported::Unsupported(value)
            }
        })
    }

    fn server(&mut self, value: Value, parent: Option<u32>) -> Result<DnsServer, ResolverError> {
        add_count(&mut self.servers, 1, MAX_SERVERS)?;
        let mut fields = object(value)?;
        let family = family(required(&mut fields, "family")?)?;
        let bytes = array(required(&mut fields, "address")?)?;
        let expected_length = if family == 2 { 4 } else { 16 };
        if bytes.len() != expected_length {
            return Err(ResolverError::Malformed);
        }
        let mut address = [0; 16];
        for (octet, value) in address.iter_mut().zip(bytes) {
            *octet = value
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or(ResolverError::Malformed)?;
        }
        let address = match family {
            2 => IpAddr::V4(Ipv4Addr::new(
                address[0], address[1], address[2], address[3],
            )),
            10 => IpAddr::V6(Ipv6Addr::from(address)),
            _ => return Err(ResolverError::Malformed),
        };
        let port = required(&mut fields, "port")?
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .ok_or(ResolverError::Malformed)?;
        let ifindex = optional(&mut fields, "ifindex", index)?;
        check_index(parent, ifindex)?;
        Ok(DnsServer {
            address,
            port,
            ifindex,
            address_string: optional(&mut fields, "addressString", string)?,
            name: optional(&mut fields, "name", string)?,
            accessible: boolean(required(&mut fields, "accessible")?)?,
            unknown_fields: self.unknown(fields),
        })
    }

    fn domain(&mut self, value: Value, parent: Option<u32>) -> Result<SearchDomain, ResolverError> {
        add_count(&mut self.domains, 1, MAX_DOMAINS)?;
        let mut fields = object(value)?;
        let ifindex = optional(&mut fields, "ifindex", index)?;
        check_index(parent, ifindex)?;
        Ok(SearchDomain {
            name: string(required(&mut fields, "name")?)?,
            route_only: boolean(required(&mut fields, "routeOnly")?)?,
            ifindex,
            unknown_fields: self.unknown(fields),
        })
    }

    fn dns_scope(
        &mut self,
        value: Value,
        parent: Option<u32>,
        parent_name: Option<&str>,
    ) -> Result<DnsScope, ResolverError> {
        add_count(&mut self.scopes, 1, MAX_DOMAINS)?;
        let mut fields = object(value)?;
        let ifindex = optional(&mut fields, "ifindex", index)?;
        let ifname = optional(&mut fields, "ifname", string)?;
        check_index(parent, ifindex)?;
        if ifname.is_some() && ifindex.is_none()
            || matches!((parent_name, ifname.as_deref()), (Some(a),Some(b)) if a != b)
        {
            return Err(ResolverError::Malformed);
        }
        Ok(DnsScope {
            protocol: self.mode(required(&mut fields, "protocol")?, protocol)?,
            family: optional(&mut fields, "family", family)?,
            ifindex,
            ifname,
            dnssec: optional(&mut fields, "dnssec", |value| self.mode(value, dnssec))?,
            dns_over_tls: optional(&mut fields, "dnsOverTLS", |value| {
                self.mode(value, dns_over_tls)
            })?,
            unknown_fields: self.unknown(fields),
        })
    }

    fn configuration(&mut self, value: Value) -> Result<DnsConfiguration, ResolverError> {
        let mut fields = object(value)?;
        let ifindex = optional(&mut fields, "ifindex", index)?;
        let ifname = optional(&mut fields, "ifname", string)?;
        let delegate = optional(&mut fields, "delegate", string)?;
        if ifname.is_some() && ifindex.is_none() || delegate.is_some() && ifindex.is_some() {
            return Err(ResolverError::Malformed);
        }
        let scope = match (ifindex, delegate.as_ref()) {
            (_, Some(_)) => {
                self.incomplete = true;
                ResolverScope::Delegate
            }
            (Some(index), None) => ResolverScope::Link(index),
            (None, None) => ResolverScope::Global,
        };
        let fallback_servers = optional(&mut fields, "fallbackServers", |value| {
            if scope != ResolverScope::Global {
                return Err(ResolverError::Malformed);
            }
            array(value)?
                .into_iter()
                .map(|value| self.server(value, None))
                .collect()
        })?;
        let resolv_conf_mode = optional(&mut fields, "resolvConfMode", |value| {
            if scope != ResolverScope::Global {
                return Err(ResolverError::Malformed);
            }
            self.mode(value, global_mode)
        })?;
        let scopes: Option<Vec<DnsScope>> = optional(&mut fields, "scopes", |value| {
            let mut seen = BTreeSet::new();
            array(value)?
                .into_iter()
                .map(|value| {
                    let parsed = self.dns_scope(value, ifindex, ifname.as_deref())?;
                    let protocol = match &parsed.protocol {
                        Supported::Known(DnsProtocolKnown::Dns) => "dns",
                        Supported::Known(DnsProtocolKnown::Mdns) => "mdns",
                        Supported::Known(DnsProtocolKnown::Llmnr) => "llmnr",
                        Supported::Unsupported(raw) => raw,
                    };
                    if !seen.insert((protocol.to_owned(), parsed.family, parsed.ifindex)) {
                        return Err(ResolverError::Malformed);
                    }
                    Ok(parsed)
                })
                .collect()
        })?;
        Ok(DnsConfiguration {
            scope,
            ifindex,
            ifname,
            delegate,
            fallback_servers,
            resolv_conf_mode,
            scopes,
            default_route: optional(&mut fields, "defaultRoute", boolean)?,
            dnssec_supported: optional(&mut fields, "dnssecSupported", boolean)?,
            current_server: optional(&mut fields, "currentServer", |value| {
                self.server(value, ifindex)
            })?,
            servers: optional(&mut fields, "servers", |value| {
                array(value)?
                    .into_iter()
                    .map(|value| self.server(value, ifindex))
                    .collect()
            })?,
            search_domains: optional(&mut fields, "searchDomains", |value| {
                array(value)?
                    .into_iter()
                    .map(|value| self.domain(value, ifindex))
                    .collect()
            })?,
            negative_trust_anchors: optional(&mut fields, "negativeTrustAnchors", |value| {
                let values = array(value)?;
                add_count(&mut self.domains, values.len(), MAX_DOMAINS)?;
                values.into_iter().map(string).collect()
            })?,
            dnssec: optional(&mut fields, "dnssec", |value| self.mode(value, dnssec))?,
            dns_over_tls: optional(&mut fields, "dnsOverTLS", |value| {
                self.mode(value, dns_over_tls)
            })?,
            llmnr: optional(&mut fields, "llmnr", |value| {
                self.mode(value, resolve_support)
            })?,
            mdns: optional(&mut fields, "mDNS", |value| {
                self.mode(value, resolve_support)
            })?,
            unknown_fields: self.unknown(fields),
        })
    }
}

pub(super) fn parse_configuration(
    parameters: Value,
) -> Result<ResolverConfiguration, ResolverError> {
    validate_bounds(&parameters)?;
    let mut fields = object(parameters)?;
    let values = array(required(&mut fields, "configuration")?)?;
    if values.len() > MAX_CONFIGURATIONS {
        return Err(ResolverError::Limit);
    }
    let mut context = Context::default();
    let mut seen = BTreeSet::new();
    let entries = values
        .into_iter()
        .map(|value| {
            let entry = context.configuration(value)?;
            if !seen.insert((entry.ifindex, entry.delegate.clone())) {
                return Err(ResolverError::Malformed);
            }
            Ok(entry)
        })
        .collect::<Result<_, _>>()?;
    let unknown_fields = context.unknown(fields);
    Ok(ResolverConfiguration {
        entries,
        incomplete: context.incomplete,
        unknown_field_count: context.unknown,
        unknown_fields,
    })
}

fn validate_bounds(value: &Value) -> Result<(), ResolverError> {
    // Точный лимит размера JSON проверяет wire до построения Value.
    let mut pending = vec![(value, 0)];
    while let Some((value, depth)) = pending.pop() {
        if depth > 128 {
            return Err(ResolverError::Limit);
        }
        match value {
            Value::String(text) => {
                if text.len() > MAX_STRING {
                    return Err(ResolverError::Limit);
                }
            }
            Value::Array(values) => pending.extend(values.iter().map(|value| (value, depth + 1))),
            Value::Object(fields) => {
                for (key, value) in fields {
                    if key.len() > MAX_STRING {
                        return Err(ResolverError::Limit);
                    }
                    pending.push((value, depth + 1));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn object(value: Value) -> Result<Map<String, Value>, ResolverError> {
    match value {
        Value::Object(fields) => Ok(fields),
        _ => Err(ResolverError::Malformed),
    }
}
fn array(value: Value) -> Result<Vec<Value>, ResolverError> {
    match value {
        Value::Array(values) => Ok(values),
        _ => Err(ResolverError::Malformed),
    }
}
fn string(value: Value) -> Result<String, ResolverError> {
    match value {
        Value::String(text) => Ok(text),
        _ => Err(ResolverError::Malformed),
    }
}
fn boolean(value: Value) -> Result<bool, ResolverError> {
    value.as_bool().ok_or(ResolverError::Malformed)
}
fn index(value: Value) -> Result<u32, ResolverError> {
    value
        .as_u64()
        .filter(|value| *value > 0 && *value <= i32::MAX as u64)
        .map(|value| value as u32)
        .ok_or(ResolverError::Malformed)
}
fn family(value: Value) -> Result<u8, ResolverError> {
    match value.as_u64() {
        Some(2) => Ok(2),
        Some(10) => Ok(10),
        _ => Err(ResolverError::Malformed),
    }
}
fn required(fields: &mut Map<String, Value>, name: &str) -> Result<Value, ResolverError> {
    fields.remove(name).ok_or(ResolverError::Malformed)
}
fn optional<T>(
    fields: &mut Map<String, Value>,
    name: &str,
    parse: impl FnOnce(Value) -> Result<T, ResolverError>,
) -> Result<Option<T>, ResolverError> {
    match fields.remove(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => parse(value).map(Some),
    }
}
fn add_count(count: &mut usize, additional: usize, max: usize) -> Result<(), ResolverError> {
    *count = count
        .checked_add(additional)
        .filter(|count| *count <= max)
        .ok_or(ResolverError::Limit)?;
    Ok(())
}
fn check_index(parent: Option<u32>, child: Option<u32>) -> Result<(), ResolverError> {
    if matches!((parent,child), (Some(a),Some(b)) if a!=b) {
        Err(ResolverError::Malformed)
    } else {
        Ok(())
    }
}
fn global_mode(value: &str) -> Option<GlobalModeKnown> {
    Some(match value {
        "stub" => GlobalModeKnown::Stub,
        "static" => GlobalModeKnown::Static,
        "uplink" => GlobalModeKnown::Uplink,
        "foreign" => GlobalModeKnown::Foreign,
        "missing" => GlobalModeKnown::Missing,
        _ => return None,
    })
}
fn dnssec(value: &str) -> Option<DnssecModeKnown> {
    Some(match value {
        "no" => DnssecModeKnown::No,
        "yes" => DnssecModeKnown::Yes,
        "allow-downgrade" => DnssecModeKnown::AllowDowngrade,
        _ => return None,
    })
}
fn dns_over_tls(value: &str) -> Option<DnsOverTlsModeKnown> {
    Some(match value {
        "no" => DnsOverTlsModeKnown::No,
        "yes" => DnsOverTlsModeKnown::Yes,
        "opportunistic" => DnsOverTlsModeKnown::Opportunistic,
        _ => return None,
    })
}
fn resolve_support(value: &str) -> Option<ResolveSupportKnown> {
    Some(match value {
        "no" => ResolveSupportKnown::No,
        "yes" => ResolveSupportKnown::Yes,
        "resolve" => ResolveSupportKnown::Resolve,
        _ => return None,
    })
}
fn protocol(value: &str) -> Option<DnsProtocolKnown> {
    Some(match value {
        "dns" => DnsProtocolKnown::Dns,
        "mdns" => DnsProtocolKnown::Mdns,
        "llmnr" => DnsProtocolKnown::Llmnr,
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
