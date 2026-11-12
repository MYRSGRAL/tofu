use std::num::NonZeroU16;
use yonik_domain::{Action, Conditions, Policy, Rule};
use yonik_engine::{Engine, PolicyMode, SystemTun, compile_policy, compile_system_policy};

#[test]
fn executable_conditions_are_scoped_to_tun_and_keep_destination_and_or_values() {
    let report = yonik_import::import_bytes(
        b"vless://00112233-4455-6677-8899-aabbccddeeff@127.0.0.1:24443".to_vec(),
    )
    .unwrap();
    let tun = SystemTun::new(1, 1500, 99, vec!["198.18.0.1/30".parse().unwrap()]).unwrap();
    let mut policy = Policy::default();
    policy.user_rules.push(Rule {
        id: 1,
        enabled: true,
        conditions: Conditions {
            executables: vec!["/usr/bin/a".into(), "/usr/bin/b".into()],
            destinations: vec!["198.18.0.2/32".parse().unwrap()],
            ..Conditions::default()
        },
        action: Action::Block,
    });
    for engine in [Engine::Xray, Engine::SingBox] {
        let port = NonZeroU16::new(10808).unwrap();
        let probe = NonZeroU16::new(10809).unwrap();
        assert!(
            compile_policy(
                &report.servers[0],
                engine,
                port,
                probe,
                &policy,
                PolicyMode::IpOnly
            )
            .is_err()
        );
        let compiled =
            compile_system_policy(&report.servers[0], engine, port, probe, &policy, &tun).unwrap();
        let mut bytes = Vec::new();
        compiled.write_sensitive(&mut bytes).unwrap();
        let native: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let (rules, source, destination, inbound) = match engine {
            Engine::Xray => (&native["routing"]["rules"], "process", "ip", "inboundTag"),
            Engine::SingBox => (
                &native["route"]["rules"],
                "process_path",
                "ip_cidr",
                "inbound",
            ),
        };
        let rule = rules
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r.get(source).is_some())
            .unwrap();
        assert_eq!(
            rule[source],
            serde_json::json!(["/usr/bin/a", "/usr/bin/b"])
        );
        assert_eq!(rule[destination], serde_json::json!(["198.18.0.2/32"]));
        assert_eq!(rule[inbound], serde_json::json!(["system-tun"]));
        for invalid in ["relative", "/usr/bin/", "/usr/../bin/a", "/usr//a", "/a\n"] {
            policy.user_rules[0].conditions.executables = vec![invalid.into()];
            assert!(
                compile_system_policy(&report.servers[0], engine, port, probe, &policy, &tun)
                    .is_err()
            );
        }
        policy.user_rules[0].conditions.executables =
            vec!["/usr/bin/a".into(), "/usr/bin/b".into()];
    }
}

#[test]
fn p07_system_compilers_refuse_unsupported_even_with_matching_executable() {
    use yonik_domain::{AndroidPackageId, PlatformSource};
    let report = yonik_import::import_bytes(
        b"vless://00112233-4455-6677-8899-aabbccddeeff@127.0.0.1:24443".to_vec(),
    )
    .unwrap();
    let tun = SystemTun::new(1, 1500, 99, vec!["198.18.0.1/30".parse().unwrap()]).unwrap();
    let mut policy = Policy::default();
    policy.user_rules.push(Rule {
        id: 41,
        enabled: true,
        action: Action::Direct,
        conditions: Conditions {
            executables: vec!["/usr/bin/client".into()],
            platform_sources: vec![PlatformSource::AndroidPackage {
                package: AndroidPackageId::parse("org.example.app").unwrap(),
            }],
            ..Default::default()
        },
    });
    for engine in [Engine::Xray, Engine::SingBox] {
        assert!(matches!(
            compile_system_policy(
                &report.servers[0],
                engine,
                NonZeroU16::new(10808).unwrap(),
                NonZeroU16::new(10809).unwrap(),
                &policy,
                &tun
            ),
            Err(yonik_engine::PolicyCompileError::UnsupportedSelector(41))
        ));
    }
    policy.user_rules[0].enabled = false;
    for engine in [Engine::Xray, Engine::SingBox] {
        assert!(
            compile_system_policy(
                &report.servers[0],
                engine,
                NonZeroU16::new(10808).unwrap(),
                NonZeroU16::new(10809).unwrap(),
                &policy,
                &tun
            )
            .is_ok()
        );
    }
}
