//! Разбор согласованного набора. Личные правила не входят в snapshot.
use yonik_domain::{Action, Conditions, DomainName, DomainPattern, Rule};

pub const MAX_LIST_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidPreset;

pub struct Candidate {
    domains: Vec<DomainPattern>,
    networks: Vec<yonik_domain::Cidr>,
}
impl Candidate {
    pub fn parse(
        domain_revision: &str,
        ip_revision: &str,
        domains: &[u8],
        ips: &[u8],
    ) -> Result<Self, InvalidPreset> {
        if !valid_token(domain_revision) || domain_revision != ip_revision {
            return Err(InvalidPreset);
        }
        let domains = lines(domains)?
            .map(|s| {
                s.parse::<DomainName>()
                    .map(DomainPattern::Suffix)
                    .map_err(|_| InvalidPreset)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let networks = lines(ips)?
            .map(|s| s.parse().map_err(|_| InvalidPreset))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { domains, networks })
    }

    /// Два OR-правила: домены и CIDR не становятся AND-условием.
    pub fn into_rules(self, ids: [u64; 2], enabled: bool) -> Vec<Rule> {
        vec![
            Rule {
                id: ids[0],
                enabled,
                conditions: Conditions {
                    domains: self.domains,
                    ..Conditions::default()
                },
                action: Action::Direct,
            },
            Rule {
                id: ids[1],
                enabled,
                conditions: Conditions {
                    destinations: self.networks,
                    ..Conditions::default()
                },
                action: Action::Direct,
            },
        ]
    }
}

pub fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
fn lines(bytes: &[u8]) -> Result<impl Iterator<Item = &str>, InvalidPreset> {
    if bytes.is_empty() || bytes.len() > MAX_LIST_BYTES {
        return Err(InvalidPreset);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| InvalidPreset)?;
    let count = text.lines().count();
    if count == 0
        || count > MAX_ENTRIES
        || text
            .lines()
            .any(|s| s.is_empty() || s.len() > 1024 || s.trim() != s)
    {
        return Err(InvalidPreset);
    }
    Ok(text.lines())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yonik_domain::{Flow, Policy, PrivateNetwork, Route, RuleOrigin, VpnId};
    #[test]
    fn installing_preset_preserves_private_lan_and_user_priority() {
        let mut policy = Policy::default();
        policy.private_networks.push(PrivateNetwork {
            network: "10.0.0.0/8".parse().expect("CIDR"),
            available: false,
            vpn: VpnId::new(1).expect("VPN"),
        });
        policy.lan.push("192.168.0.0/16".parse().expect("LAN"));
        policy.user_rules.push(Rule {
            id: 3,
            enabled: true,
            conditions: Conditions {
                destinations: vec!["1.1.1.0/24".parse().expect("CIDR")],
                ..Conditions::default()
            },
            action: Action::Proxy,
        });
        for version in ["v1", "v2"] {
            policy.preset_rules =
                Candidate::parse(version, version, b"example.org\n", b"0.0.0.0/0\n")
                    .expect("candidate")
                    .into_rules([10, 11], true);
            for (ip, route, origin) in [
                ("10.1.1.1", Route::Block, RuleOrigin::PrivateNetwork),
                ("192.168.1.1", Route::Direct, RuleOrigin::Lan),
                ("1.1.1.1", Route::Proxy, RuleOrigin::User(3)),
                ("8.8.8.8", Route::Direct, RuleOrigin::Preset(11)),
            ] {
                let decision = policy.evaluate(&Flow {
                    executable: None,
                    destination: ip.parse().expect("IP"),
                });
                assert_eq!((decision.action, decision.origin), (route, origin));
            }
        }
    }
}
