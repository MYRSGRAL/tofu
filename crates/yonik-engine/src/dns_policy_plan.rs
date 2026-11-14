//! Неизменяемый DATA-план. Не конфигурация stock core и не сетевое полномочие.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, io::Write, net::SocketAddr};
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

// Нижняя граница JSON: строки учитываются с точным escaping и кавычками,
// каждый элемент контейнера — минимум один байт; часть фиксированной структуры
// оплачивается консервативным минимумом. Остаток проверяет bounded writer.
struct ByteBudget {
    remaining: usize,
}
impl ByteBudget {
    fn charge(&mut self, bytes: usize) -> Result<(), PlanError> {
        self.remaining = self.remaining.checked_sub(bytes).ok_or(PlanError::Bounds)?;
        Ok(())
    }
    fn rows(&mut self, rows: usize) -> Result<(), PlanError> {
        if rows > MAX_ROWS {
            return Err(PlanError::Bounds);
        }
        self.charge(rows)
    }
    fn repeated(&mut self, rows: usize, minimum: usize) -> Result<(), PlanError> {
        self.charge(rows.checked_mul(minimum).ok_or(PlanError::Bounds)?)
    }
    fn string(&mut self, text: &str) -> Result<(), PlanError> {
        // Быстрый отказ без чтения содержимого заведомо огромной строки.
        self.charge(text.len().checked_add(2).ok_or(PlanError::Bounds)?)?;
        for byte in text.bytes() {
            self.charge(match byte {
                b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 8 | 12 => 1,
                0..=31 => 5,
                _ => 0,
            })?;
        }
        Ok(())
    }
}
fn preflight(
    policy: &Policy,
    dns: &DnsPolicy,
    desired: &DesiredPlan,
    limit: usize,
) -> Result<(), PlanError> {
    let mut budget = ByteBudget { remaining: limit };
    let rules = policy
        .user_rules
        .len()
        .checked_add(policy.preset_rules.len())
        .ok_or(PlanError::Bounds)?;
    budget.rows(rules)?;
    // Ключи/структура самого короткого Rule длиннее 100 bytes без payload.
    budget.repeated(rules, 100)?;
    for count in [
        policy.lan.len(),
        policy.private_networks.len(),
        policy.private_domains.len(),
        dns.fake_exclusions.len(),
        desired.vpns.len(),
        desired.fake_pools.len(),
    ] {
        budget.rows(count)?;
    }
    // Вместе с rows: CIDR минимум 6 bytes ("::/0"), pattern >21 плюс имя.
    budget.repeated(policy.lan.len(), 5)?;
    budget.repeated(policy.private_networks.len(), 5)?;
    budget.repeated(policy.private_domains.len(), 20)?;
    budget.repeated(dns.fake_exclusions.len(), 20)?;
    // Закрытые selectors/constraints никогда не сериализуются, в том числе disabled.
    if !desired.reserved_constraints.is_empty() {
        return Err(PlanError::Unsupported);
    }
    for r in policy.user_rules.iter().chain(&policy.preset_rules) {
        let c = &r.conditions;
        if !c.platform_sources.is_empty() {
            return Err(PlanError::Unsupported);
        }
        budget.rows(c.executables.len())?;
        budget.rows(c.domains.len())?;
        budget.rows(c.destinations.len())?;
        budget.repeated(c.destinations.len(), 5)?;
        budget.repeated(c.domains.len(), 20)?;
        for text in &c.executables {
            budget.string(text)?;
        }
        for domain in &c.domains {
            budget.string(domain.name().as_str())?;
        }
        // TransportConditions закрыт: ≤2 protocol strings, ≤16 port ranges.
        if let Some(protocols) = c.transport.protocols() {
            budget.rows(protocols.len())?;
            for protocol in protocols {
                budget.string(protocol)?;
            }
        }
        if let Some(ports) = c.transport.ports() {
            budget.charge(ports.ranges().len())?;
        }
    }
    for d in &policy.private_domains {
        budget.string(d.domain.name().as_str())?;
    }
    for d in &dns.fake_exclusions {
        budget.string(d.name().as_str())?;
    }
    for vpn in &desired.vpns {
        budget.string(&vpn.ifname)?;
    }
    for pool in &desired.fake_pools {
        budget.string(pool)?;
    }
    Ok(())
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes
            .len()
            .checked_add(bytes.len())
            .filter(|next| *next <= self.limit)
            .ok_or_else(|| std::io::Error::other("Превышен бюджет DATA-плана"))?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn compile_plan(
    policy: &Policy,
    dns: &DnsPolicy,
    desired: &DesiredPlan,
) -> Result<CompiledPolicyPlan, PlanError> {
    compile_plan_bounded(policy, dns, desired, MAX_BYTES, || {})
}

// Callback отмечает реальную границу materialization для малых regression fixtures.
fn compile_plan_bounded(
    policy: &Policy,
    dns: &DnsPolicy,
    desired: &DesiredPlan,
    limit: usize,
    before_value: impl FnOnce(),
) -> Result<CompiledPolicyPlan, PlanError> {
    preflight(policy, dns, desired, limit)?;
    if desired.version != 1 || !desired.reserved_constraints.is_empty() {
        return Err(PlanError::Unsupported);
    }
    if !dns.ipv4 && !dns.ipv6 {
        return Err(PlanError::Invalid);
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
    before_value();
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
    let mut writer = BoundedWriter {
        bytes: Vec::with_capacity(limit),
        limit,
    };
    serde_json::to_writer(&mut writer, &value).map_err(|_| PlanError::Bounds)?;
    Ok(CompiledPolicyPlan(writer.bytes))
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use std::cell::Cell;
    use yonik_domain::{Conditions, DestinationPorts, TransportConditions};

    fn desired() -> DesiredPlan {
        DesiredPlan {
            version: 1,
            network_epoch: 1,
            engine_epoch: 2,
            resolver_epoch: 3,
            policy_transport: DnsTransport::Udp,
            policy_route: ServiceRoute::Proxy,
            bootstrap_route: ServiceRoute::Direct,
            bootstrap: "9.9.9.9:53".parse().unwrap(),
            bootstrap_transport: DnsTransport::Tcp,
            vpns: vec![],
            fake_pools: vec![],
            reserved_constraints: vec![],
        }
    }
    fn dns() -> DnsPolicy {
        DnsPolicy {
            mode: NativeDnsMode::Real,
            upstream: "1.1.1.1:53".parse().unwrap(),
            ipv4: true,
            ipv6: true,
            fake_exclusions: vec![],
            require_mapping_lease: false,
            listen_port: 5353.try_into().unwrap(),
        }
    }
    #[test]
    fn small_matrix_rejected_before_materialization() {
        let mut policy = Policy::default();
        for id in 0..4 {
            policy.user_rules.push(Rule {
                id,
                enabled: true,
                action: Action::Direct,
                conditions: Conditions {
                    executables: vec![format!("/{}", "x".repeat(31)); 4],
                    ..Conditions::default()
                },
            });
        }
        let rest = policy.user_rules.split_off(1);
        preflight(&policy, &dns(), &desired(), 512).unwrap();
        policy.user_rules.extend(rest);
        let calls = Cell::new(0);
        assert_eq!(
            compile_plan_bounded(&policy, &dns(), &desired(), 512, || calls
                .set(calls.get() + 1))
            .unwrap_err(),
            PlanError::Bounds
        );
        assert_eq!(calls.get(), 0);
        // Даже отдельная строка не сканируется при недостаточном raw budget.
        let mut budget = ByteBudget { remaining: 1 };
        assert_eq!(budget.string(&"x".repeat(64)), Err(PlanError::Bounds));
        assert_eq!(budget.charge(usize::MAX), Err(PlanError::Bounds));
        assert_eq!(budget.repeated(usize::MAX, 2), Err(PlanError::Bounds));
    }
    #[test]
    fn directly_constructed_desired_is_preflighted() {
        let mut input = desired();
        input.fake_pools = vec!["x".repeat(64)];
        let calls = Cell::new(0);
        assert_eq!(
            compile_plan_bounded(&Policy::default(), &dns(), &input, 32, || calls
                .set(calls.get() + 1))
            .unwrap_err(),
            PlanError::Bounds
        );
        assert_eq!(calls.get(), 0);
        input.fake_pools.clear();
        input.vpns.push(DesiredVpn {
            vpn: 1,
            ifname: "x".repeat(64),
            table: 1,
            resolver: "10.0.0.53:53".parse().unwrap(),
            transport: DnsTransport::Udp,
            excluded: false,
        });
        assert_eq!(
            preflight(&Policy::default(), &dns(), &input, 32),
            Err(PlanError::Bounds)
        );
        input.vpns.clear();
        input.reserved_constraints.push("x".repeat(64));
        assert_eq!(
            preflight(&Policy::default(), &dns(), &input, 32),
            Err(PlanError::Unsupported)
        );
    }
    #[test]
    fn escaping_and_exact_boundary() {
        for text in ["", "plain", "кириллица", "\"\\\n\r\t\u{8}\u{c}\u{1}"] {
            let expected = serde_json::to_vec(text).unwrap();
            let mut budget = ByteBudget {
                remaining: expected.len(),
            };
            budget.string(text).unwrap();
            assert_eq!(budget.remaining, 0);
            let mut budget = ByteBudget {
                remaining: expected.len() - 1,
            };
            assert_eq!(budget.string(text), Err(PlanError::Bounds));
            let mut writer = BoundedWriter {
                bytes: Vec::with_capacity(expected.len()),
                limit: expected.len(),
            };
            serde_json::to_writer(&mut writer, text).unwrap();
            assert_eq!(writer.bytes, expected);
            assert!(writer.write_all(b" ").is_err());
            assert_eq!(writer.bytes, expected);
        }
        let policy = Policy {
            user_rules: vec![Rule {
                id: 1,
                enabled: true,
                action: Action::Proxy,
                conditions: Conditions {
                    executables: vec!["/a\"\\кириллица".into()],
                    transport: TransportConditions::parse(
                        Some(&["tcp", "udp"]),
                        Some(DestinationPorts::parse(&[(1, 65535)]).unwrap()),
                    )
                    .unwrap(),
                    ..Conditions::default()
                },
            }],
            ..Policy::default()
        };
        let expected = compile_plan(&policy, &dns(), &desired()).unwrap();
        let calls = Cell::new(0);
        let exact =
            compile_plan_bounded(&policy, &dns(), &desired(), expected.bytes().len(), || {
                calls.set(calls.get() + 1)
            })
            .unwrap();
        assert_eq!(exact.bytes(), expected.bytes());
        assert_eq!(calls.get(), 1);
        assert_eq!(
            compile_plan_bounded(
                &policy,
                &dns(),
                &desired(),
                expected.bytes().len() - 1,
                || {}
            )
            .unwrap_err(),
            PlanError::Bounds
        );
    }
    #[test]
    fn every_variable_container_consumes_budget() {
        // Минимальный budget=0 даёт отказ для нетекстовых typed containers тоже.
        let mut p = Policy::default();
        let d = dns();
        let x = desired();
        p.lan.push("192.168.0.0/16".parse().unwrap());
        assert_eq!(preflight(&p, &d, &x, 0), Err(PlanError::Bounds));
        p.lan.clear();
        p.private_networks.push(yonik_domain::PrivateNetwork {
            network: "10.0.0.0/8".parse().unwrap(),
            available: false,
            vpn: yonik_domain::VpnId::new(1).unwrap(),
        });
        assert_eq!(preflight(&p, &d, &x, 0), Err(PlanError::Bounds));
        p.private_networks.clear();
        p.private_domains.push(yonik_domain::PrivateDomain {
            domain: DomainPattern::Suffix("private.test".parse().unwrap()),
            available: false,
            vpn: yonik_domain::VpnId::new(1).unwrap(),
        });
        assert_eq!(preflight(&p, &d, &x, 1), Err(PlanError::Bounds));
        p.private_domains.clear();
        let mut d = d;
        d.fake_exclusions
            .push(DomainPattern::Exact("excluded.test".parse().unwrap()));
        assert_eq!(preflight(&p, &d, &x, 1), Err(PlanError::Bounds));
        p.preset_rules.push(Rule {
            id: 1,
            enabled: false,
            action: Action::Block,
            conditions: Conditions::default(),
        });
        p.preset_rules[0].conditions.domains =
            vec![DomainPattern::Exact("one.test".parse().unwrap()); MAX_ROWS + 1];
        assert_eq!(preflight(&p, &dns(), &x, MAX_BYTES), Err(PlanError::Bounds));
    }
}
