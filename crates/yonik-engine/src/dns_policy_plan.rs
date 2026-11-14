//! Неизменяемый DATA-план. Не конфигурация stock core и не сетевое полномочие.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, net::SocketAddr};
use yonik_domain::{Action, Cidr, DnsPolicy, DomainPattern, NativeDnsMode, Policy, Rule};

pub const MAX_BYTES: usize = 262_144;
pub const MAX_ROWS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    Bounds,
    Unsupported,
    Invalid,
    Mapping,
    Ambiguous,
}

/// Только желаемая привязка; ни ifname, ни epoch не доказывают authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesiredVpn {
    pub vpn: u64,
    pub ifname: String,
    pub table: u32,
    pub resolver: SocketAddr,
    pub transport: DnsTransport,
    pub excluded: bool,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsTransport {
    Udp,
    Tcp,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceRoute {
    Direct,
    Proxy,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesiredPlan {
    pub version: u32,
    pub network_epoch: u64,
    pub engine_epoch: u64,
    pub resolver_epoch: u64,
    pub policy_transport: DnsTransport,
    pub policy_route: ServiceRoute,
    pub bootstrap_route: ServiceRoute,
    pub bootstrap: SocketAddr,
    pub bootstrap_transport: DnsTransport,
    pub vpns: Vec<DesiredVpn>,
    pub fake_pools: Vec<String>,
    /// Закрытый резерв: regex, stale, hosts, scripts и расширения не допускаются.
    pub reserved_constraints: Vec<String>,
}
impl DesiredPlan {
    pub fn parse(bytes: &[u8]) -> Result<Self, PlanError> {
        if bytes.len() > MAX_BYTES {
            return Err(PlanError::Bounds);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| PlanError::Invalid)?;
        if !crate::policy_wire::bounded_depth(text, 5) {
            return Err(PlanError::Bounds);
        }
        serde_json::from_slice(bytes).map_err(|_| PlanError::Invalid)
    }
}
/// Байты закрыты после проверки. Вызывающий код хеширует ровно bytes() через LinuxDigest.
#[derive(Debug)]
pub struct CompiledPolicyPlan(Vec<u8>);
impl CompiledPolicyPlan {
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}
fn action(a: Action) -> &'static str {
    match a {
        Action::Direct => "direct",
        Action::Proxy => "proxy",
        Action::Block => "block",
    }
}
fn pattern(p: &DomainPattern) -> Value {
    json!({"kind":match p { DomainPattern::Exact(_) => "exact", DomainPattern::Suffix(_) => "suffix" },"name":p.name().as_str()})
}
fn endpoint(a: SocketAddr) -> Result<(), PlanError> {
    if a.port() == 0
        || a.ip().is_unspecified()
        || a.ip().is_multicast()
        || matches!(a, SocketAddr::V6(v) if v.scope_id()!=0 || v.flowinfo()!=0 || v.ip().to_ipv4_mapped().is_some())
    {
        return Err(PlanError::Invalid);
    }
    Ok(())
}
fn rule(r: &Rule) -> Result<Value, PlanError> {
    let c = &r.conditions;
    if !c.platform_sources.is_empty() {
        return Err(PlanError::Unsupported);
    }
    if [c.domains.len(), c.destinations.len(), c.executables.len()]
        .into_iter()
        .any(|n| n > MAX_ROWS)
        || c.executables
            .iter()
            .any(|s| s.len() > 4096 || !s.starts_with('/') || s.chars().any(char::is_control))
    {
        return Err(PlanError::Bounds);
    }
    Ok(
        json!({"id":r.id,"enabled":r.enabled,"action":action(r.action),
        "source":{"executables":c.executables,"platform_sources":[]},
        "destination":{"domains":c.domains.iter().map(pattern).collect::<Vec<_>>(),
        "cidrs":c.destinations.iter().map(ToString::to_string).collect::<Vec<_>>()},
        "transport":crate::transport::TransportInput::from_typed(&c.transport)}),
    )
}

pub fn compile_plan(
    policy: &Policy,
    dns: &DnsPolicy,
    desired: &DesiredPlan,
) -> Result<CompiledPolicyPlan, PlanError> {
    if desired.version != 1 || !desired.reserved_constraints.is_empty() {
        return Err(PlanError::Unsupported);
    }
    if !dns.ipv4 && !dns.ipv6 {
        return Err(PlanError::Invalid);
    }
    if policy.user_rules.len() + policy.preset_rules.len() > MAX_ROWS
        || [
            policy.lan.len(),
            policy.private_domains.len(),
            policy.private_networks.len(),
            dns.fake_exclusions.len(),
            desired.vpns.len(),
            desired.fake_pools.len(),
        ]
        .into_iter()
        .any(|n| n > MAX_ROWS)
    {
        return Err(PlanError::Bounds);
    }
    endpoint(dns.upstream)?;
    endpoint(desired.bootstrap)?;
    let mut ids = BTreeSet::new();
    for r in policy.user_rules.iter().chain(&policy.preset_rules) {
        if !ids.insert(r.id) {
            return Err(PlanError::Invalid);
        }
    }
    let mut vpns = BTreeSet::new();
    for v in &desired.vpns {
        endpoint(v.resolver)?;
        if v.vpn == 0
            || !vpns.insert(v.vpn)
            || v.excluded
            || v.table == 0
            || v.ifname.is_empty()
            || v.ifname.len() > 15
            || !v
                .ifname
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
        {
            return Err(PlanError::Mapping);
        }
    }
    let referenced: BTreeSet<_> = policy
        .private_networks
        .iter()
        .map(|n| n.vpn.get())
        .chain(policy.private_domains.iter().map(|d| d.vpn.get()))
        .collect();
    if referenced != vpns {
        return Err(PlanError::Mapping);
    }
    // Перекрытия разных VPN не имеют однозначной желаемой привязки.
    for (i, a) in policy.private_networks.iter().enumerate() {
        for b in &policy.private_networks[i + 1..] {
            let ip = |c: Cidr| c.to_string().split('/').next().and_then(|s| s.parse().ok());
            if a.vpn != b.vpn
                && (ip(a.network).is_some_and(|ip| b.network.contains(ip))
                    || ip(b.network).is_some_and(|ip| a.network.contains(ip)))
            {
                return Err(PlanError::Ambiguous);
            }
        }
    }
    for (i, a) in policy.private_domains.iter().enumerate() {
        for b in &policy.private_domains[i + 1..] {
            if a.vpn != b.vpn
                && (a.domain.matches(b.domain.name()) || b.domain.matches(a.domain.name()))
            {
                return Err(PlanError::Ambiguous);
            }
        }
    }
    let pools = desired
        .fake_pools
        .iter()
        .map(|s| s.parse::<Cidr>().map_err(|_| PlanError::Invalid))
        .collect::<Result<Vec<_>, _>>()?;
    match dns.mode {
        NativeDnsMode::Real if !pools.is_empty() || dns.require_mapping_lease => {
            return Err(PlanError::Unsupported);
        }
        NativeDnsMode::FakeIp if pools.is_empty() || !dns.require_mapping_lease => {
            return Err(PlanError::Mapping);
        }
        _ => {}
    }
    for (index, p) in pools.iter().enumerate() {
        if p.to_string() != desired.fake_pools[index] || pools[..index].contains(p) {
            return Err(PlanError::Invalid);
        }
        let prefix: u8 = p
            .to_string()
            .split_once('/')
            .ok_or(PlanError::Invalid)?
            .1
            .parse()
            .map_err(|_| PlanError::Invalid)?;
        let ip = p
            .to_string()
            .split('/')
            .next()
            .and_then(|s| s.parse::<std::net::IpAddr>().ok())
            .ok_or(PlanError::Invalid)?;
        if ip.is_ipv4() && !dns.ipv4 || ip.is_ipv6() && !dns.ipv6 {
            return Err(PlanError::Invalid);
        }
        // V1 фиксирует безопасные пространства; произвольные pools требуют отдельного review.
        let allowed: Cidr = if ip.is_ipv4() {
            "198.18.0.0/15"
        } else {
            "fc00::/18"
        }
        .parse()
        .map_err(|_| PlanError::Invalid)?;
        if !allowed.contains(ip) || prefix < if ip.is_ipv4() { 15 } else { 18 } {
            return Err(PlanError::Unsupported);
        }
        if policy
            .private_networks
            .iter()
            .map(|n| &n.network)
            .chain(&policy.lan)
            .any(|network| {
                network.contains(ip)
                    || network
                        .to_string()
                        .split_once('/')
                        .and_then(|(a, _)| a.parse().ok())
                        .is_some_and(|a| p.contains(a))
            })
        {
            return Err(PlanError::Mapping);
        }
    }
    let value = json!({"yonik_policy":{
        "version":1,"kind":"desired_data_only","desired":desired,
        "bounds":{"bytes":MAX_BYTES,"rows":MAX_ROWS,"answers":256,"parallel_lookups":2,"lookup_deadline_ms":10000},
        "policy":{"default_action":action(policy.default_action),
            "user_rules":policy.user_rules.iter().map(rule).collect::<Result<Vec<_>,_>>()?,
            "preset_rules":policy.preset_rules.iter().map(rule).collect::<Result<Vec<_>,_>>()?,
            "lan":policy.lan.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "private_networks":policy.private_networks.iter().map(|n|json!({"cidr":n.network.to_string(),"vpn":n.vpn.get(),"available":n.available})).collect::<Vec<_>>(),
            "private_domains":policy.private_domains.iter().map(|d|json!({"domain":pattern(&d.domain),"vpn":d.vpn.get(),"available":d.available})).collect::<Vec<_>>()},
        "dns":{"mode":match dns.mode {NativeDnsMode::Real=>"real",NativeDnsMode::FakeIp=>"fake_ip"},
            "upstream":dns.upstream,"family_mask":u8::from(dns.ipv4) | (u8::from(dns.ipv6)<<1),
            "listen_port":dns.listen_port.get(),"fake_exclusions":dns.fake_exclusions.iter().map(pattern).collect::<Vec<_>>(),"require_mapping_lease":dns.require_mapping_lease},
        "requirements":{"full_family":true,"first_any_match":true,"literal_only":true,"private_ambiguity_terminal":true,
            "no_public_private_fallback":true,"no_local_reresolve":true,"original_monotonic_deadline":true,
            "positive_negative_cname_min_ttl":true,"unknown_expired_fake_reject":true,"fake_before_rules":true,
            "epoch_nonreuse":true,"fake_address_nonreuse":true,"trusted_binding_before_every_dial":true,
            "revoke_closes_sockets":true,"service_paths_nonrecursive":true,"established_tcp_ttl_close":false}
    }});
    let bytes = serde_json::to_vec(&value).map_err(|_| PlanError::Invalid)?;
    if bytes.len() > MAX_BYTES {
        return Err(PlanError::Bounds);
    }
    Ok(CompiledPolicyPlan(bytes))
}
