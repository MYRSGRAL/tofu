//! Граница DNS capability до генерации native-конфигурации.
use crate::{
    CompiledConfig, Engine, PolicyCapability, PolicyCompileError, PolicyMode, compile_policy,
};
use std::num::NonZeroU16;
use yonik_domain::{DomainPattern, Policy};
use yonik_import::ParsedServer;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DnsMode {
    /// DNS принадлежит вызывающему host; native resolver не создаётся.
    #[default]
    External,
    Real,
    FakeIp,
}

/// Совместимый вход #63: не содержит upstream/listener для DnsPolicy.
/// Real/FakeIp здесь остаются отказом; исполняемый профиль задаётся DnsPolicy.
/// External с обоими семействами не обещает full-family lookup: DNS вне compiler.
#[derive(Debug)]
pub struct DnsContract {
    pub mode: DnsMode,
    pub fake_exclusions: Vec<DomainPattern>,
    pub ipv4: bool,
    pub ipv6: bool,
}
impl Default for DnsContract {
    fn default() -> Self {
        Self {
            mode: DnsMode::External,
            fake_exclusions: Vec::new(),
            ipv4: true,
            ipv6: true,
        }
    }
}
impl DnsContract {
    pub fn validate(&self) -> Result<(), PolicyCompileError> {
        // Ни один pinned adapter пока не переносит full-family lease в dial
        // и не владеет Fake-IP mapping между поколениями. Не теряем запрос.
        if self.mode != DnsMode::External
            || !self.fake_exclusions.is_empty()
            || !self.ipv4
            || !self.ipv6
        {
            return Err(PolicyCompileError::Unsupported(
                PolicyCapability::DnsContract,
            ));
        }
        Ok(())
    }
}

/// Проверяет DNS до выдачи конфигурации и создания probe credentials.
#[allow(clippy::too_many_arguments)]
pub fn compile_policy_with_dns(
    server: &ParsedServer,
    engine: Engine,
    port: NonZeroU16,
    probe_port: NonZeroU16,
    policy: &Policy,
    mode: PolicyMode,
    dns: &DnsContract,
) -> Result<CompiledConfig, PolicyCompileError> {
    dns.validate()?;
    compile_policy(server, engine, port, probe_port, policy, mode)
}

/// Компилирует ограниченный локальный DNS-профиль; не включает системный TUN.
#[allow(clippy::too_many_arguments)]
pub fn compile_native_dns(
    server: &ParsedServer,
    engine: Engine,
    port: NonZeroU16,
    probe_port: NonZeroU16,
    policy: &Policy,
    mode: PolicyMode,
    dns: &yonik_domain::DnsPolicy,
) -> Result<CompiledConfig, PolicyCompileError> {
    use serde_json::json;
    use yonik_domain::NativeDnsMode;
    // Пока не доказаны required-real, private resolver и epochs, разрешаем
    // только именную публичную политику без исключений/состояния host broker.
    if server
        .endpoint()
        .host()
        .parse::<std::net::IpAddr>()
        .is_err()
        || dns.ipv4 == dns.ipv6
        || !dns.fake_exclusions.is_empty()
        || dns.require_mapping_lease
        || mode != PolicyMode::Names
        || policy.default_action != yonik_domain::Action::Proxy
        || !policy.user_rules.is_empty()
        || !policy.preset_rules.is_empty()
        || !policy.private_domains.is_empty()
        || !policy.private_networks.is_empty()
        || !policy.lan.is_empty()
        || dns.listen_port == port
        || dns.listen_port == probe_port
        || (dns.upstream.ip().is_loopback()
            && [port, probe_port, dns.listen_port]
                .iter()
                .any(|port| port.get() == dns.upstream.port()))
        || matches!(dns.upstream, std::net::SocketAddr::V6(address)
            if address.scope_id() != 0 || address.flowinfo() != 0 || address.ip().to_ipv4_mapped().is_some())
        || dns.upstream.port() == 0
        || dns.upstream.ip().is_unspecified()
        || dns.upstream.ip().is_multicast()
    {
        return Err(PolicyCompileError::Unsupported(
            PolicyCapability::DnsContract,
        ));
    }
    let mut config = compile_policy(server, engine, port, probe_port, policy, mode)?;
    let strategy = if dns.ipv6 { "ipv6_only" } else { "ipv4_only" };
    let fake_pool = if dns.ipv6 {
        "fc00::/18"
    } else {
        "198.18.0.0/15"
    };
    let fake = dns.mode == NativeDnsMode::FakeIp;
    match engine {
        Engine::Xray => {
            config.value["dns"] = json!({"servers": if fake {json!(["fakedns"])} else {json!([{ "address": dns.upstream.ip().to_string(), "port": dns.upstream.port()}])}, "tag":"dns-upstream", "queryStrategy":if dns.ipv6 {"UseIPv6"} else {"UseIPv4"}, "disableFallback":true, "disableCache":true});
            config.value["routing"]["rules"]
                .as_array_mut()
                .ok_or(PolicyCompileError::InvalidPolicy)?
                .insert(
                    0,
                    json!({"type":"field","inboundTag":["dns-upstream"],"outboundTag":"direct"}),
                );
            if fake {
                config.value["fakedns"] = json!([{ "ipPool":fake_pool, "poolSize":65535}]);
                // Успешный sniff заменяет fake IP именем; оставшийся literal
                // не имеет mapping и не должен уходить в публичный proxy.
                config.value["routing"]["rules"]
                    .as_array_mut()
                    .ok_or(PolicyCompileError::InvalidPolicy)?
                    .insert(
                        0,
                        json!({"type":"field","ip":[fake_pool],"outboundTag":"block"}),
                    );
            }
            config.value["inbounds"].as_array_mut().ok_or(PolicyCompileError::InvalidPolicy)?.push(json!({"tag":"dns-in","listen":"127.0.0.1","port":dns.listen_port.get(),"protocol":"dokodemo-door","settings":{"address":"1.1.1.1","port":53,"network":"udp"}}));
            if fake {
                config.value["inbounds"][0]["sniffing"] =
                    json!({"enabled":true,"destOverride":["fakedns"],"metadataOnly":true});
            }
            config.value["outbounds"]
                .as_array_mut()
                .ok_or(PolicyCompileError::InvalidPolicy)?
                .push(json!({"tag":"dns-out","protocol":"dns"}));
            config.value["routing"]["rules"]
                .as_array_mut()
                .ok_or(PolicyCompileError::InvalidPolicy)?
                .insert(
                    0,
                    json!({"type":"field","inboundTag":["dns-in"],"outboundTag":"dns-out"}),
                );
        }
        Engine::SingBox => {
            config.value["route"]["default_domain_resolver"] =
                json!({"server":"real", "strategy":strategy});
            let mut servers = vec![
                json!({"type":"udp","tag":"real","server":dns.upstream.ip().to_string(),"server_port":dns.upstream.port()}),
            ];
            if fake {
                let range = if dns.ipv6 {
                    "inet6_range"
                } else {
                    "inet4_range"
                };
                servers.push(json!({"type":"fakeip","tag":"fake",range:fake_pool}));
            }
            config.value["dns"] = json!({"servers":servers,"final":"real", "rules":if fake {json!([{ "query_type":[if dns.ipv6 {"AAAA"} else {"A"}],"action":"route","server":"fake"}])} else {json!([])},"disable_cache":true,"strategy":strategy});
            config.value["inbounds"].as_array_mut().ok_or(PolicyCompileError::InvalidPolicy)?.push(json!({"type":"direct","tag":"dns-in","listen":"127.0.0.1","listen_port":dns.listen_port.get(),"network":"udp"}));
            config.value["route"]["rules"]
                .as_array_mut()
                .ok_or(PolicyCompileError::InvalidPolicy)?
                .insert(0, json!({"inbound":["dns-in"],"action":"hijack-dns"}));
        }
    }
    Ok(config)
}
