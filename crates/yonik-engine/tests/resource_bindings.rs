use std::net::IpAddr;
use yonik_domain::{
    Action, Conditions, Destination, DestinationFlow, DomainName, DomainPattern, Evaluation,
    EvaluationContext, Policy, Route, Rule,
};
use yonik_engine::{
    DnsContract, Engine, PolicyCapability, PolicyCompileError, PolicyMode, resource_bundle::*,
    resource_index::RuleSetIndex,
};
struct Catalog {
    refs: Vec<ExternalRef>,
    indices: Vec<RuleSetIndex>,
}
impl VerifiedCatalog for Catalog {
    fn manifest(&self) -> &[ExternalRef] {
        &self.refs
    }
    fn matches_domain(&self, i: usize, n: &DomainName) -> bool {
        self.indices[i].matches_domain(n)
    }
    fn matches_ip(&self, i: usize, n: IpAddr) -> bool {
        self.indices[i].matches_ip(n)
    }
}
fn catalog(engine: Engine) -> Catalog {
    Catalog {
        refs: vec![
            ExternalRef {
                engine,
                digest: [1; 32],
                bytes: 42,
                records: 1,
                kind: ResourceKind::Domains,
                format_version: if engine == Engine::Xray { 1 } else { 3 },
            },
            ExternalRef {
                engine,
                digest: [2; 32],
                bytes: 42,
                records: 1,
                kind: ResourceKind::Cidrs,
                format_version: if engine == Engine::Xray { 1 } else { 3 },
            },
        ],
        indices: vec![
            RuleSetIndex::parse(b"suffix:external.test\n", ResourceKind::Domains, 1).unwrap(),
            RuleSetIndex::parse(b"10.0.0.0/8\n", ResourceKind::Cidrs, 1).unwrap(),
        ],
    }
}
#[test]
fn single_evaluator_preserves_or_groups_and_app_and_priority() {
    let catalog = catalog(Engine::Xray);
    let mut policy = Policy {
        default_action: Action::Direct,
        ..Policy::default()
    };
    policy.user_rules.push(Rule {
        id: 7,
        enabled: true,
        action: Action::Block,
        conditions: Conditions {
            transport: Default::default(),
            platform_sources: vec![],
            executables: vec!["/app".into()],
            domains: vec![DomainPattern::Exact("inline.test".parse().unwrap())],
            destinations: vec!["192.0.2.0/24".parse().unwrap()],
        },
    });
    let identity = policy_identity(&policy);
    let bound = BoundPolicy::bind(
        &policy,
        4,
        &identity,
        &catalog,
        vec![
            ResourceBinding {
                rule: ResourceRule::User(7),
                resource: 0,
            },
            ResourceBinding {
                rule: ResourceRule::User(7),
                resource: 1,
            },
        ],
    )
    .unwrap();
    for (app, name, ip, blocked) in [
        (Some("/app"), "inline.test", "10.1.1.1", true),
        (Some("/app"), "sub.external.test", "192.0.2.1", true),
        (Some("/other"), "external.test", "10.1.1.1", false),
        (Some("/app"), "miss.test", "10.1.1.1", false),
        (Some("/app"), "external.test", "203.0.113.1", false),
    ] {
        let name = name.parse().unwrap();
        let flow = DestinationFlow {
            executable: app,
            destination: Destination::Ip {
                address: ip.parse().unwrap(),
                name: Some(&name),
            },
        };
        let Evaluation::Decided { decision, .. } =
            bound.evaluate(&flow, &EvaluationContext::default())
        else {
            panic!("Ожидалось решение")
        };
        assert_eq!(
            decision.action,
            if blocked { Route::Block } else { Route::Direct }
        );
    }
    assert!(bound.validate_revision(5, &identity).is_err());
    let mut wrong = identity.clone();
    wrong.push(0);
    assert!(bound.validate_revision(4, &wrong).is_err());
}
#[test]
fn native_resources_preserve_unsupported_gates_and_small_config() {
    let server = yonik_import::import_bytes(b"socks://127.0.0.1:18081".to_vec())
        .unwrap()
        .servers
        .remove(0);
    for engine in [Engine::Xray, Engine::SingBox] {
        let mut catalog = catalog(engine);
        catalog.refs.pop();
        catalog.indices.pop();
        let mut policy = Policy::default();
        policy.user_rules.push(Rule {
            id: 1,
            enabled: true,
            action: Action::Block,
            conditions: Conditions::default(),
        });
        let identity = policy_identity(&policy);
        let bound = BoundPolicy::bind(
            &policy,
            1,
            &identity,
            &catalog,
            vec![ResourceBinding {
                rule: ResourceRule::User(1),
                resource: 0,
            }],
        )
        .unwrap();
        let compile = |mode| {
            compile_bound_policy(
                &server,
                engine,
                10808.try_into().unwrap(),
                10809.try_into().unwrap(),
                &bound,
                1,
                &identity,
                mode,
                &DnsContract::default(),
            )
        };
        assert!(matches!(
            compile(PolicyMode::IpOnly),
            Err(PolicyCompileError::Unsupported(
                PolicyCapability::DomainsInIpOnly
            ))
        ));
        let config = compile(PolicyMode::Names).unwrap();
        let mut bytes = Vec::new();
        config.config().write_sensitive(&mut bytes).unwrap();
        assert!(bytes.len() < 65536);
        assert!(!bytes.windows(13).any(|s| s == b"external.test"));
        drop(bound);
        policy.user_rules[0]
            .conditions
            .executables
            .push("/app".into());
        let identity = policy_identity(&policy);
        let bound = BoundPolicy::bind(
            &policy,
            1,
            &identity,
            &catalog,
            vec![ResourceBinding {
                rule: ResourceRule::User(1),
                resource: 0,
            }],
        )
        .unwrap();
        assert!(matches!(
            compile_bound_policy(
                &server,
                engine,
                10808.try_into().unwrap(),
                10809.try_into().unwrap(),
                &bound,
                1,
                &identity,
                PolicyMode::Names,
                &DnsContract::default()
            ),
            Err(PolicyCompileError::Unsupported(
                PolicyCapability::ProcessAttribution
            ))
        ));
    }
}

#[test]
fn index_suffix_boundaries_and_ipv6() {
    let index = RuleSetIndex::parse(
        b"suffix:example.org\nexact:only.test\n",
        ResourceKind::Domains,
        2,
    )
    .unwrap();
    for (name, expected) in [
        ("example.org", true),
        ("sub.example.org", true),
        ("notexample.org", false),
        ("only.test", true),
        ("sub.only.test", false),
    ] {
        assert_eq!(index.matches_domain(&name.parse().unwrap()), expected);
    }
    let index =
        RuleSetIndex::parse(b"2001:db8::/32\n192.0.2.0/24\n", ResourceKind::Cidrs, 2).unwrap();
    for (ip, expected) in [
        ("2001:db8::", true),
        ("2001:db8:ffff:ffff:ffff:ffff:ffff:ffff", true),
        ("2001:db9::", false),
        ("192.0.2.255", true),
        ("192.0.3.0", false),
    ] {
        assert_eq!(index.matches_ip(ip.parse().unwrap()), expected);
    }
    assert!(RuleSetIndex::parse(b"192.0.2.1/24\n", ResourceKind::Cidrs, 1).is_err());
}

#[test]
fn metadata_format_and_manifest_bounds_refuse() {
    let mut r = catalog(Engine::Xray).refs.remove(0);
    r.format_version = 99;
    assert!(r.validate().is_err());
    r.format_version = 1;
    r.bytes = 128 * 1024 * 1024 + 1;
    assert!(r.validate().is_err());
    r.bytes = 42;
    r.records = 1_500_001;
    assert!(r.validate().is_err());
    r.records = 1;
    r.engine = Engine::SingBox;
    assert!(r.validate().is_err());
    let policy = Policy::default();
    let identity = policy_identity(&policy);
    let mut c = catalog(Engine::Xray);
    c.refs = (0..17)
        .map(|i| ExternalRef {
            engine: Engine::Xray,
            digest: [i; 32],
            bytes: 42,
            records: 1,
            kind: ResourceKind::Domains,
            format_version: 1,
        })
        .collect();
    assert!(BoundPolicy::bind(&policy, 1, &identity, &c, Vec::new()).is_err());
}

#[test]
fn unadmitted_mixed_data_and_cancellation_do_not_relax_admission() {
    use std::cell::Cell;
    use yonik_engine::resource_bundle::{BoundCompileError, compile_bound_data_cancellable};
    let server = yonik_import::import_bytes(b"socks://127.0.0.1:18081".to_vec())
        .unwrap()
        .servers
        .remove(0);
    for engine in [Engine::Xray, Engine::SingBox] {
        let catalog = catalog(engine);
        let policy = Policy {
            user_rules: vec![Rule {
                id: 7,
                enabled: true,
                action: Action::Block,
                conditions: Conditions {
                    executables: vec!["/opt/fixture/app-0".into()],
                    ..Conditions::default()
                },
            }],
            ..Policy::default()
        };
        let identity = policy_identity(&policy);
        let bound = BoundPolicy::bind(
            &policy,
            1,
            &identity,
            &catalog,
            vec![
                ResourceBinding {
                    rule: ResourceRule::User(7),
                    resource: 0,
                },
                ResourceBinding {
                    rule: ResourceRule::User(7),
                    resource: 1,
                },
            ],
        )
        .unwrap();
        let compile = |revision, proceed: &dyn Fn() -> bool| {
            compile_bound_data_cancellable(
                &server,
                engine,
                10808.try_into().unwrap(),
                10809.try_into().unwrap(),
                &bound,
                revision,
                &identity,
                proceed,
            )
        };
        let calls = Cell::new(0usize);
        let count = || {
            calls.set(calls.get() + 1);
            true
        };
        let artifact = compile(1, &count as &dyn Fn() -> bool).unwrap();
        let total = calls.get();
        assert_eq!(artifact.manifest(), catalog.manifest());
        assert_eq!(artifact.revision(), 1);
        assert_eq!(artifact.policy_identity(), identity);
        let mut bytes = Vec::new();
        artifact.write_sensitive(&mut bytes).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        if engine == Engine::SingBox {
            let rule = &value["route"]["rules"][2];
            assert_eq!(rule["type"], "logical");
            assert_eq!(rule["mode"], "and");
            assert_eq!(rule["rules"][0]["process_path"][0], "/opt/fixture/app-0");
            assert_eq!(rule["rules"][1]["mode"], "and");
            assert!(rule.get("process_path").is_none());
        }
        for cancel_at in [1, 2, 5, 20, total / 2, total - 1, total] {
            let calls = Cell::new(0usize);
            // После единственного false callback снова true: отмена обязана быть sticky.
            let proceed = || {
                calls.set(calls.get() + 1);
                calls.get() != cancel_at
            };
            assert!(matches!(
                compile(1, &proceed as &dyn Fn() -> bool),
                Err(BoundCompileError::Cancelled)
            ));
            assert_eq!(calls.get(), cancel_at);
            assert!(compile(1, &(|| true) as &dyn Fn() -> bool).is_ok());
        }
        assert!(matches!(
            compile(2, &(|| true) as &dyn Fn() -> bool),
            Err(BoundCompileError::Policy(PolicyCompileError::InvalidPolicy))
        ));
        for mode in [PolicyMode::Names, PolicyMode::IpOnly] {
            assert!(matches!(
                compile_bound_policy(
                    &server,
                    engine,
                    10808.try_into().unwrap(),
                    10809.try_into().unwrap(),
                    &bound,
                    1,
                    &identity,
                    mode,
                    &DnsContract::default()
                ),
                Err(PolicyCompileError::Unsupported(
                    PolicyCapability::ProcessAttribution
                ))
            ));
        }
    }
}

#[test]
fn unadmitted_data_rejects_invalid_path_and_actual_four_mib_output() {
    use yonik_engine::resource_bundle::{BoundCompileError, compile_bound_data_cancellable};
    let server = yonik_import::import_bytes(b"socks://127.0.0.1:18081".to_vec())
        .unwrap()
        .servers
        .remove(0);
    for engine in [Engine::Xray, Engine::SingBox] {
        let catalog = catalog(engine);
        for oversized in [false, true] {
            let name = format!(
                "{}.{}.{}.invalid",
                "a".repeat(63),
                "b".repeat(63),
                "c".repeat(63)
            );
            let count = if oversized { 10000 } else { 1 };
            let conditions = if oversized {
                Conditions {
                    domains: vec![DomainPattern::Exact(name.parse().unwrap())],
                    executables: vec!["/opt/fixture/app".into()],
                    ..Conditions::default()
                }
            } else {
                Conditions {
                    executables: vec!["/opt/../invalid".into()],
                    ..Conditions::default()
                }
            };
            let policy = Policy {
                user_rules: (0..count)
                    .map(|id| Rule {
                        id,
                        enabled: true,
                        action: Action::Block,
                        conditions: Conditions {
                            transport: Default::default(),
                            executables: conditions.executables.clone(),
                            domains: conditions.domains.clone(),
                            destinations: conditions.destinations.clone(),
                            platform_sources: vec![],
                        },
                    })
                    .collect(),
                ..Policy::default()
            };
            let identity = policy_identity(&policy);
            let bindings = (0..count)
                .flat_map(|id| {
                    (0..2).map(move |resource| ResourceBinding {
                        rule: ResourceRule::User(id),
                        resource,
                    })
                })
                .collect();
            let bound = BoundPolicy::bind(&policy, 1, &identity, &catalog, bindings).unwrap();
            assert!(matches!(
                compile_bound_data_cancellable(
                    &server,
                    engine,
                    10808.try_into().unwrap(),
                    10809.try_into().unwrap(),
                    &bound,
                    1,
                    &identity,
                    || true
                ),
                Err(BoundCompileError::Policy(PolicyCompileError::InvalidPolicy))
            ));
        }
    }
}
