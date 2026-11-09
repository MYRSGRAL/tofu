use std::{collections::HashSet, fmt, num::NonZeroU16};

use serde_json::{Value, json};
use tofu_domain::{Action, DomainPattern, Policy};
use tofu_import::ParsedServer;

use crate::{CompileError, CompiledConfig, Engine, compile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyMode {
    Names,
    IpOnly,
}
impl std::str::FromStr for PolicyMode {
    type Err = PolicyCompileError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "names" => Ok(Self::Names),
            "ip-only" => Ok(Self::IpOnly),
            _ => Err(PolicyCompileError::InvalidPolicy),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyCapability {
    RequiredRealResolution,
    ProcessAttribution,
    PrivateNetworkBinding,
    DomainsInIpOnly,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyCompileError {
    Server(CompileError),
    Unsupported(PolicyCapability),
    InvalidPolicy,
    ProbePort,
    Random,
}
impl fmt::Display for PolicyCompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Server(error) => error.fmt(f),
            Self::Unsupported(PolicyCapability::RequiredRealResolution) => f.write_str(
                "Нативный режим не подтверждает обязательное DNS разрешение CIDR-политики",
            ),
            Self::Unsupported(PolicyCapability::ProcessAttribution) => {
                f.write_str("Атрибуция источника через локальный SOCKS не подтверждена")
            }
            Self::Unsupported(PolicyCapability::PrivateNetworkBinding) => {
                f.write_str("Нет подтверждённой NetworkHost привязки к приватному VPN")
            }
            Self::Unsupported(PolicyCapability::DomainsInIpOnly) => {
                f.write_str("Доменное правило недоступно в режиме только IP")
            }
            Self::InvalidPolicy => f.write_str("Некорректная или слишком большая политика"),
            Self::ProbePort => f.write_str("Нужен отдельный ненулевой порт служебной проверки"),
            Self::Random => {
                f.write_str("Не удалось получить случайную авторизацию служебной проверки")
            }
        }
    }
}
impl std::error::Error for PolicyCompileError {}

/// Случайный секрет только в памяти host/конфига. Не выводить в обычную сводку.
pub struct ProbeCredentials {
    password: String,
}
impl ProbeCredentials {
    fn random() -> Result<Self, PolicyCompileError> {
        let mut random = [0; 32];
        getrandom::getrandom(&mut random).map_err(|_| PolicyCompileError::Random)?;
        const HEX: &[u8] = b"0123456789abcdef";
        let mut password = String::with_capacity(64);
        for byte in random {
            password.push(char::from(HEX[usize::from(byte >> 4)]));
            password.push(char::from(HEX[usize::from(byte & 15)]));
        }
        Ok(Self { password })
    }
    pub fn username(&self) -> &str {
        "tofu-health"
    }
    pub fn password(&self) -> &str {
        &self.password
    }
}
impl fmt::Debug for ProbeCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProbeCredentials(<скрыто>)")
    }
}

/// Служебный probe не является разрешением отправлять известные приватные
/// зоны/адреса публичному серверу. Проверка не делает собственный DNS lookup.
#[derive(Default)]
pub struct ProbeAccess {
    auth: Option<ProbeCredentials>,
    networks: Vec<tofu_domain::Cidr>,
    domains: Vec<DomainPattern>,
}
impl ProbeAccess {
    pub fn auth(&self) -> Option<&ProbeCredentials> {
        self.auth.as_ref()
    }
    pub fn allows_host(&self, host: &str) -> bool {
        if self.networks.is_empty() && self.domains.is_empty() {
            return true;
        }
        if let Ok(address) = host.parse::<std::net::IpAddr>() {
            return !self
                .networks
                .iter()
                .any(|network| network.contains(address.to_canonical()));
        }
        match url::Host::parse(host) {
            Ok(url::Host::Ipv4(address)) => !self
                .networks
                .iter()
                .any(|network| network.contains(address.into())),
            Ok(url::Host::Ipv6(address)) => !self
                .networks
                .iter()
                .any(|network| network.contains(std::net::IpAddr::V6(address).to_canonical())),
            Ok(url::Host::Domain(domain)) => domain
                .parse::<tofu_domain::DomainName>()
                .is_ok_and(|name| !self.domains.iter().any(|pattern| pattern.matches(&name))),
            Err(_) => false,
        }
    }
}
impl fmt::Debug for ProbeAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProbeAccess(<скрыто>)")
    }
}

/// Names не содержит CIDR; IpOnly принудительно отклоняет доменные SOCKS
/// назначения. Недоказанные комбинации не опускаются и не получают fallback.
pub fn compile_policy(
    server: &ParsedServer,
    engine: Engine,
    port: NonZeroU16,
    probe_port: NonZeroU16,
    policy: &Policy,
    mode: PolicyMode,
) -> Result<CompiledConfig, PolicyCompileError> {
    if port == probe_port {
        return Err(PolicyCompileError::ProbePort);
    }
    validate_policy(policy, mode)?;
    crate::validate_listener(server, probe_port).map_err(PolicyCompileError::Server)?;
    let mut config = compile(server, engine, port).map_err(PolicyCompileError::Server)?;
    let auth = ProbeCredentials::random()?;
    let inbound = match engine {
        Engine::Xray => {
            json!({"tag":"probe","listen":"127.0.0.1","port":probe_port.get(),"protocol":"socks",
            "settings":{"auth":"password","accounts":[{"user":auth.username(),"pass":auth.password()}]}})
        }
        Engine::SingBox => {
            json!({"type":"socks","tag":"probe","listen":"127.0.0.1","listen_port":probe_port.get(),
            "users":[{"username":auth.username(),"password":auth.password()}]})
        }
    };
    config.value["inbounds"]
        .as_array_mut()
        .ok_or(PolicyCompileError::InvalidPolicy)?
        .push(inbound);
    let mut rules = vec![match engine {
        Engine::Xray => json!({"type":"field","inboundTag":["probe"],"outboundTag":"proxy"}),
        Engine::SingBox => json!({"inbound":["probe"],"action":"route","outbound":"proxy"}),
    }];
    // Native matchers не выполняют IDNA и удаление root-dot. Не даём
    // некорректному wire имени обойти правило и попасть в default.
    const INVALID_NAME: &str =
        r"[^A-Za-z0-9.\-]|^\.|\.$|\.\.|(^|\.)-|-(\.|$)|(^|\.)[A-Za-z0-9\-]{64,}(\.|$)|.{254,}";
    let guard = if mode == PolicyMode::IpOnly {
        "."
    } else {
        INVALID_NAME
    };
    rules.push(route_rule(
        engine,
        Action::Block,
        Some(vec![format!("regexp:{guard}")]),
        None,
    ));
    for private in &policy.private_networks {
        rules.push(route_rule(
            engine,
            Action::Block,
            None,
            Some(vec![private.network.to_string()]),
        ));
    }
    for zone in &policy.private_domains {
        rules.push(route_rule(
            engine,
            Action::Block,
            Some(vec![xray_domain(&zone.domain)]),
            None,
        ));
    }
    if !policy.lan.is_empty() {
        rules.push(route_rule(
            engine,
            Action::Direct,
            None,
            Some(policy.lan.iter().map(ToString::to_string).collect()),
        ));
    }
    for rule in policy
        .user_rules
        .iter()
        .chain(&policy.preset_rules)
        .filter(|rule| rule.enabled)
    {
        let domains = (!rule.conditions.domains.is_empty())
            .then(|| rule.conditions.domains.iter().map(xray_domain).collect());
        let cidrs = (!rule.conditions.destinations.is_empty()).then(|| {
            rule.conditions
                .destinations
                .iter()
                .map(ToString::to_string)
                .collect()
        });
        rules.push(route_rule(engine, rule.action, domains, cidrs));
    }
    rules.push(route_rule(engine, policy.default_action, None, None));
    let direct = match engine {
        Engine::Xray => json!({"tag":"direct","protocol":"freedom"}),
        Engine::SingBox => json!({"tag":"direct","type":"direct"}),
    };
    let outbounds = config.value["outbounds"]
        .as_array_mut()
        .ok_or(PolicyCompileError::InvalidPolicy)?;
    outbounds.push(direct);
    match engine {
        Engine::Xray => {
            outbounds.push(json!({"tag":"block","protocol":"blackhole"}));
            config.value["routing"] = json!({"domainStrategy":"AsIs","rules":rules});
        }
        Engine::SingBox => config.value["route"] = json!({"final":"proxy","rules":rules}),
    }
    config.probe_port = probe_port;
    config.probe_access = ProbeAccess {
        auth: Some(auth),
        networks: policy
            .private_networks
            .iter()
            .map(|network| network.network)
            .collect(),
        domains: policy
            .private_domains
            .iter()
            .map(|domain| domain.domain.clone())
            .collect(),
    };
    Ok(config)
}

fn xray_domain(domain: &DomainPattern) -> String {
    format!(
        "{}:{}",
        match domain {
            DomainPattern::Exact(_) => "full",
            DomainPattern::Suffix(_) => "domain",
        },
        domain.name().as_str()
    )
}
fn route_rule(
    engine: Engine,
    action: Action,
    domains: Option<Vec<String>>,
    cidrs: Option<Vec<String>>,
) -> Value {
    match engine {
        Engine::Xray => {
            let mut rule = json!({"type":"field","outboundTag":match action { Action::Direct => "direct", Action::Proxy => "proxy", Action::Block => "block" }});
            let all = domains.is_none() && cidrs.is_none();
            if let Some(domains) = domains {
                rule["domain"] = json!(domains);
            }
            if let Some(cidrs) = cidrs {
                rule["ip"] = json!(cidrs);
            }
            if all {
                rule["network"] = json!("tcp,udp");
            }
            rule
        }
        Engine::SingBox => {
            let mut rule = if action == Action::Block {
                json!({"action":"reject"})
            } else {
                json!({"action":"route","outbound":if action == Action::Direct { "direct" } else { "proxy" }})
            };
            if let Some(domains) = domains {
                for (prefix, key) in [
                    ("full:", "domain"),
                    ("domain:", "domain_suffix"),
                    ("regexp:", "domain_regex"),
                ] {
                    let values: Vec<&str> = domains
                        .iter()
                        .filter_map(|domain| domain.strip_prefix(prefix))
                        .collect();
                    if !values.is_empty() {
                        rule[key] = json!(values);
                    }
                }
            }
            if let Some(cidrs) = cidrs {
                rule["ip_cidr"] = json!(cidrs);
            }
            rule
        }
    }
}

fn validate_policy(policy: &Policy, mode: PolicyMode) -> Result<(), PolicyCompileError> {
    let mut selectors =
        policy.lan.len() + policy.private_networks.len() + policy.private_domains.len();
    if policy.user_rules.len() + policy.preset_rules.len() > 10000 {
        return Err(PolicyCompileError::InvalidPolicy);
    }
    for rules in [&policy.user_rules, &policy.preset_rules] {
        let mut ids = HashSet::with_capacity(rules.len());
        for rule in rules {
            if !ids.insert(rule.id) {
                return Err(PolicyCompileError::InvalidPolicy);
            }
            selectors = selectors
                .saturating_add(rule.conditions.executables.len())
                .saturating_add(rule.conditions.domains.len())
                .saturating_add(rule.conditions.destinations.len());
            if rule.enabled && !rule.conditions.executables.is_empty() {
                return Err(PolicyCompileError::Unsupported(
                    PolicyCapability::ProcessAttribution,
                ));
            }
        }
    }
    if selectors > 100000 {
        return Err(PolicyCompileError::InvalidPolicy);
    }
    if policy
        .private_networks
        .iter()
        .any(|network| network.available)
        || policy.private_domains.iter().any(|domain| domain.available)
    {
        return Err(PolicyCompileError::Unsupported(
            PolicyCapability::PrivateNetworkBinding,
        ));
    }
    let rules = || {
        policy
            .user_rules
            .iter()
            .chain(&policy.preset_rules)
            .filter(|rule| rule.enabled)
    };
    match mode {
        PolicyMode::Names
            if !policy.lan.is_empty()
                || !policy.private_networks.is_empty()
                || rules().any(|rule| !rule.conditions.destinations.is_empty()) =>
        {
            Err(PolicyCompileError::Unsupported(
                PolicyCapability::RequiredRealResolution,
            ))
        }
        PolicyMode::IpOnly
            if !policy.private_domains.is_empty()
                || rules().any(|rule| !rule.conditions.domains.is_empty()) =>
        {
            Err(PolicyCompileError::Unsupported(
                PolicyCapability::DomainsInIpOnly,
            ))
        }
        _ => Ok(()),
    }
}
