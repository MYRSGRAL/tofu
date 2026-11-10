use std::num::NonZeroU16;
use yonik_domain::{DomainPattern, Policy};
use yonik_engine::{
    DnsContract, DnsMode, Engine, PolicyCapability, PolicyCompileError, PolicyMode,
    compile_policy_with_dns,
};
use yonik_import::import_bytes;

#[test]
fn unsupported_dns_semantics_never_produce_a_config() {
    let source = import_bytes(b"socks5://192.0.2.1:1080".to_vec()).unwrap();
    for engine in [Engine::Xray, Engine::SingBox] {
        let mut cases = vec![
            DnsContract {
                mode: DnsMode::Real,
                ..Default::default()
            },
            DnsContract {
                mode: DnsMode::FakeIp,
                ..Default::default()
            },
            DnsContract {
                ipv4: false,
                ..Default::default()
            },
            DnsContract {
                ipv6: false,
                ..Default::default()
            },
            DnsContract {
                ipv4: false,
                ipv6: false,
                ..Default::default()
            },
        ];
        cases.push(DnsContract {
            fake_exclusions: vec![DomainPattern::Suffix("local".parse().unwrap())],
            ..Default::default()
        });
        for dns in cases {
            for mode in [PolicyMode::Names, PolicyMode::IpOnly] {
                let result = compile_policy_with_dns(
                    &source.servers[0],
                    engine,
                    NonZeroU16::new(10808).unwrap(),
                    NonZeroU16::new(10809).unwrap(),
                    &Policy::default(),
                    mode,
                    &dns,
                );
                assert_eq!(
                    result.unwrap_err(),
                    PolicyCompileError::Unsupported(PolicyCapability::DnsContract)
                );
            }
        }
    }
}

#[test]
fn external_dns_is_explicit_and_does_not_enable_native_resolver() {
    let source = import_bytes(b"socks5://192.0.2.1:1080".to_vec()).unwrap();
    for engine in [Engine::Xray, Engine::SingBox] {
        let config = compile_policy_with_dns(
            &source.servers[0],
            engine,
            NonZeroU16::new(10808).unwrap(),
            NonZeroU16::new(10809).unwrap(),
            &Policy::default(),
            PolicyMode::IpOnly,
            &DnsContract::default(),
        )
        .unwrap();
        let mut bytes = Vec::new();
        config.write_sensitive(&mut bytes).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json.get("dns").is_none());
        assert!(json.get("fakedns").is_none());
    }
}

#[test]
fn private_zones_and_full_family_cidrs_are_not_dropped() {
    use yonik_domain::{Action, Conditions, PrivateDomain, Rule, VpnId};
    let source = import_bytes(b"socks5://192.0.2.1:1080".to_vec()).unwrap();
    for engine in [Engine::Xray, Engine::SingBox] {
        for available in [false, true] {
            let policy = Policy {
                private_domains: vec![PrivateDomain {
                    domain: DomainPattern::Suffix("internal.test".parse().unwrap()),
                    vpn: VpnId::new(1).unwrap(),
                    available,
                }],
                ..Default::default()
            };
            let result = compile_policy_with_dns(
                &source.servers[0],
                engine,
                NonZeroU16::new(10808).unwrap(),
                NonZeroU16::new(10809).unwrap(),
                &policy,
                PolicyMode::IpOnly,
                &DnsContract::default(),
            );
            assert_eq!(
                result.unwrap_err(),
                PolicyCompileError::Unsupported(if available {
                    PolicyCapability::PrivateNetworkBinding
                } else {
                    PolicyCapability::DomainsInIpOnly
                })
            );
        }
        let policy = Policy {
            user_rules: vec![Rule {
                id: 1,
                enabled: true,
                action: Action::Block,
                conditions: Conditions {
                    destinations: vec![
                        "192.0.2.0/24".parse().unwrap(),
                        "2001:db8::/32".parse().unwrap(),
                    ],
                    ..Default::default()
                },
            }],
            ..Default::default()
        };
        let config = compile_policy_with_dns(
            &source.servers[0],
            engine,
            NonZeroU16::new(10808).unwrap(),
            NonZeroU16::new(10809).unwrap(),
            &policy,
            PolicyMode::IpOnly,
            &DnsContract::default(),
        )
        .unwrap();
        let mut bytes = Vec::new();
        config.write_sensitive(&mut bytes).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let cidrs = match engine {
            Engine::Xray => &json["routing"]["rules"][2]["ip"],
            Engine::SingBox => &json["route"]["rules"][2]["ip_cidr"],
        };
        assert_eq!(cidrs, &serde_json::json!(["192.0.2.0/24", "2001:db8::/32"]));
    }
}
