use yonik_domain::*;
use yonik_engine::dns_policy_plan::*;
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
fn bytes(p: &Policy, d: &DnsPolicy, x: &DesiredPlan) -> Vec<u8> {
    compile_plan(p, d, x).unwrap().bytes().to_vec()
}
fn rule(id: u64) -> Rule {
    Rule {
        id,
        enabled: true,
        conditions: Conditions::default(),
        action: Action::Proxy,
    }
}
#[test]
fn semantic_fields_and_order_are_committed() {
    let x = desired();
    let d = dns();
    let mut p = Policy::default();
    let a = bytes(&p, &d, &x);
    p.default_action = Action::Block;
    assert_ne!(a, bytes(&p, &d, &x));
    p.user_rules = vec![rule(17), rule(2)];
    let a = bytes(&p, &d, &x);
    p.user_rules.swap(0, 1);
    assert_ne!(a, bytes(&p, &d, &x));
    let mut previous = bytes(&p, &d, &x);
    macro_rules! changed {
        ($change:expr) => {{
            $change;
            let next = bytes(&p, &d, &x);
            assert_ne!(previous, next);
            previous = next;
        }};
    }
    changed!(p.user_rules[0].enabled = false);
    changed!(p.user_rules[0].id = 5);
    changed!(p.user_rules[0].action = Action::Direct);
    changed!(
        p.user_rules[0]
            .conditions
            .executables
            .push("/bin/app".into())
    );
    changed!(
        p.user_rules[0]
            .conditions
            .domains
            .push(DomainPattern::Suffix("ПРИМЕР.рф.".parse().unwrap()))
    );
    changed!(
        p.user_rules[0]
            .conditions
            .destinations
            .push("203.0.113.0/24".parse().unwrap())
    );
    changed!(
        p.user_rules[0].conditions.transport = TransportConditions::parse(
            Some(&["tcp"]),
            Some(DestinationPorts::parse(&[(443, 443)]).unwrap())
        )
        .unwrap()
    );
    changed!(p.lan.push("192.168.0.0/16".parse().unwrap()));
    changed!(p.preset_rules.push(rule(9)));
    assert_eq!(previous, bytes(&p, &d, &x));
    let v: serde_json::Value = serde_json::from_slice(&previous).unwrap();
    assert_eq!(v["yonik_policy"]["policy"]["user_rules"][0]["id"], 5);
    assert_eq!(
        v["yonik_policy"]["policy"]["user_rules"][0]["destination"]["domains"][0]["name"],
        "xn--e1afmkfd.xn--p1ai"
    );
}
#[test]
fn dns_axes_and_desired_binding_are_data() {
    let p = Policy::default();
    let mut d = dns();
    let mut x = desired();
    let mut prev = bytes(&p, &d, &x);
    macro_rules! change {
        ($e:expr) => {{
            $e;
            let next = bytes(&p, &d, &x);
            assert_ne!(prev, next);
            prev = next;
        }};
    }
    change!(d.ipv6 = false);
    change!({
        d.ipv4 = false;
        d.ipv6 = true;
    });
    change!(d.upstream = "8.8.8.8:53".parse().unwrap());
    change!(d.listen_port = 5354.try_into().unwrap());
    change!(
        d.fake_exclusions
            .push(DomainPattern::Exact("example.org".parse().unwrap()))
    );
    change!(x.network_epoch += 1);
    change!(x.engine_epoch += 1);
    change!(x.resolver_epoch += 1);
    change!(x.policy_transport = DnsTransport::Tcp);
    change!(x.policy_route = ServiceRoute::Direct);
    change!(x.bootstrap_route = ServiceRoute::Proxy);
    change!(x.bootstrap_transport = DnsTransport::Udp);
    change!(x.bootstrap = "8.8.4.4:53".parse().unwrap());
    d.mode = NativeDnsMode::FakeIp;
    d.require_mapping_lease = true;
    x.fake_pools.push("fc00::/18".into());
    assert_ne!(prev, bytes(&p, &d, &x));
}
#[test]
fn fake_pools_reject_known_lan_overlap_in_both_directions() {
    for (lan, pool) in [
        ("198.18.0.0/15", "198.18.0.0/15"),
        ("198.16.0.0/14", "198.18.0.0/15"),
        ("198.18.0.0/16", "198.18.0.0/15"),
        ("198.18.0.0/15", "198.18.0.0/16"),
        ("fc00::/18", "fc00::/18"),
        ("fc00::/7", "fc00::/18"),
        ("fc00::/32", "fc00::/18"),
        ("fc00::/18", "fc00::/32"),
    ] {
        let p = Policy {
            lan: vec![lan.parse().unwrap()],
            ..Policy::default()
        };
        let mut d = dns();
        d.mode = NativeDnsMode::FakeIp;
        d.require_mapping_lease = true;
        d.ipv4 = !pool.contains(':');
        d.ipv6 = !d.ipv4;
        let mut x = desired();
        x.fake_pools.push(pool.into());
        assert_eq!(compile_plan(&p, &d, &x).unwrap_err(), PlanError::Mapping);
    }
    for (lan, pool) in [
        ("198.20.0.0/16", "198.18.0.0/15"),
        ("fc01::/32", "fc00::/18"),
    ] {
        let p = Policy {
            lan: vec![lan.parse().unwrap()],
            ..Policy::default()
        };
        let mut d = dns();
        d.mode = NativeDnsMode::FakeIp;
        d.require_mapping_lease = true;
        d.ipv4 = !pool.contains(':');
        d.ipv6 = !d.ipv4;
        let mut x = desired();
        x.fake_pools.push(pool.into());
        assert!(compile_plan(&p, &d, &x).is_ok());
    }
}

#[test]
fn strict_negative_vectors() {
    let p = Policy::default();
    let mut d = dns();
    let mut x = desired();
    x.version = 2;
    assert_eq!(
        compile_plan(&p, &d, &x).unwrap_err(),
        PlanError::Unsupported
    );
    x.version = 1;
    x.reserved_constraints.push("stale".into());
    assert!(compile_plan(&p, &d, &x).is_err());
    x.reserved_constraints.clear();
    d.ipv4 = false;
    d.ipv6 = false;
    assert!(compile_plan(&p, &d, &x).is_err());
    d.ipv4 = true;
    d.mode = NativeDnsMode::FakeIp;
    assert!(compile_plan(&p, &d, &x).is_err());
    d.require_mapping_lease = true;
    x.fake_pools.push("0.0.0.0/0".into());
    assert!(compile_plan(&p, &d, &x).is_err());
    let mut v = serde_json::to_value(desired()).unwrap();
    v["authority"] = true.into();
    assert!(DesiredPlan::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    assert!(DesiredPlan::parse(&vec![b' '; MAX_BYTES + 1]).is_err());
    let mut p = Policy {
        user_rules: (0..257).map(rule).collect(),
        ..Policy::default()
    };
    assert_eq!(
        compile_plan(&p, &dns(), &desired()).unwrap_err(),
        PlanError::Bounds
    );
    p.user_rules = vec![rule(1), rule(1)];
    assert!(compile_plan(&p, &dns(), &desired()).is_err());
}
#[test]
fn private_mapping_is_total_and_unambiguous() {
    let mut p = Policy::default();
    let mut x = desired();
    let d = dns();
    p.private_networks.push(PrivateNetwork {
        network: "10.0.0.0/8".parse().unwrap(),
        vpn: VpnId::new(1).unwrap(),
        available: false,
    });
    assert_eq!(compile_plan(&p, &d, &x).unwrap_err(), PlanError::Mapping);
    x.vpns.push(DesiredVpn {
        vpn: 1,
        ifname: "vpn0".into(),
        table: 100,
        resolver: "10.0.0.53:53".parse().unwrap(),
        transport: DnsTransport::Tcp,
        excluded: false,
    });
    let a = bytes(&p, &d, &x);
    p.private_networks[0].available = true;
    assert_ne!(a, bytes(&p, &d, &x));
    x.vpns[0].excluded = true;
    assert!(compile_plan(&p, &d, &x).is_err());
    x.vpns[0].excluded = false;
    p.private_networks.push(PrivateNetwork {
        network: "10.1.0.0/16".parse().unwrap(),
        vpn: VpnId::new(2).unwrap(),
        available: true,
    });
    let mut v = x.vpns[0].clone();
    v.vpn = 2;
    x.vpns.push(v);
    assert_eq!(compile_plan(&p, &d, &x).unwrap_err(), PlanError::Ambiguous);
}

#[test]
fn common_d03_oracle_vectors() {
    let mut p = Policy::default();
    let mut first = rule(10);
    first.action = Action::Direct;
    first
        .conditions
        .destinations
        .push("203.0.113.0/24".parse().unwrap());
    p.user_rules.push(first);
    p.user_rules.push(rule(11));
    let plan = bytes(&p, &dns(), &desired());
    let name = "example.org".parse().unwrap();
    let a = "203.0.113.1".parse().unwrap();
    let b = "2001:db8::1".parse().unwrap();
    for addresses in [[a, b], [b, a]] {
        let flow = DestinationFlow {
            executable: None,
            destination: Destination::Name {
                name: &name,
                resolution: Resolution::Answer(AnswerSet {
                    addresses: &addresses,
                    complete: true,
                    expires_at: 10,
                    epoch: DnsEpoch::default(),
                    path: ResolverPath::Policy,
                }),
            },
        };
        assert!(matches!(
            p.evaluate_destination(&flow, &EvaluationContext::default()),
            Evaluation::Decided {
                decision: Decision {
                    action: Route::Direct,
                    origin: RuleOrigin::User(10)
                },
                ..
            }
        ));
    }
    assert_eq!(plan, bytes(&p, &dns(), &desired()));
    let literal = DestinationFlow {
        executable: None,
        destination: Destination::Ip {
            address: b,
            name: Some(&name),
        },
    };
    assert!(matches!(
        p.evaluate_destination(&literal, &EvaluationContext::default()),
        Evaluation::Decided {
            decision: Decision {
                action: Route::Proxy,
                origin: RuleOrigin::User(11)
            },
            ..
        }
    ));
}

#[test]
fn private_descriptor_mutations_and_advanced_rejection() {
    let mut p = Policy::default();
    let mut x = desired();
    p.private_domains.push(PrivateDomain {
        domain: DomainPattern::Suffix("internal.test".parse().unwrap()),
        vpn: VpnId::new(1).unwrap(),
        available: false,
    });
    x.vpns.push(DesiredVpn {
        vpn: 1,
        ifname: "vpn0".into(),
        table: 100,
        resolver: "10.0.0.53:53".parse().unwrap(),
        transport: DnsTransport::Udp,
        excluded: false,
    });
    let d = dns();
    let mut prev = bytes(&p, &d, &x);
    macro_rules! change {
        ($e:expr) => {{
            $e;
            let next = bytes(&p, &d, &x);
            assert_ne!(prev, next);
            prev = next;
        }};
    }
    change!(p.private_domains[0].available = true);
    change!(p.private_domains[0].domain = DomainPattern::Exact("internal.test".parse().unwrap()));
    change!(x.vpns[0].ifname = "vpn1".into());
    change!(x.vpns[0].table = 101);
    change!(x.vpns[0].resolver = "10.0.0.54:53".parse().unwrap());
    change!(x.vpns[0].transport = DnsTransport::Tcp);
    assert_eq!(prev, bytes(&p, &d, &x));
    let mut r = rule(1);
    r.enabled = false;
    r.conditions
        .platform_sources
        .push(PlatformSource::AndroidPackage {
            package: AndroidPackageId::parse("org.example.app").unwrap(),
        });
    p.user_rules.push(r);
    assert_eq!(
        compile_plan(&p, &d, &x).unwrap_err(),
        PlanError::Unsupported
    );
}

#[test]
fn common_v1_exact_golden() {
    const GOLDEN: &str = r#"{"yonik_policy":{"bounds":{"answers":256,"bytes":262144,"lookup_deadline_ms":10000,"parallel_lookups":2,"rows":256},"desired":{"bootstrap":"9.9.9.9:53","bootstrap_route":"direct","bootstrap_transport":"tcp","engine_epoch":2,"fake_pools":[],"network_epoch":1,"policy_route":"proxy","policy_transport":"udp","reserved_constraints":[],"resolver_epoch":3,"version":1,"vpns":[]},"dns":{"fake_exclusions":[],"family_mask":3,"listen_port":5353,"mode":"real","require_mapping_lease":false,"upstream":"1.1.1.1:53"},"kind":"desired_data_only","policy":{"default_action":"proxy","lan":[],"preset_rules":[],"private_domains":[],"private_networks":[],"user_rules":[]},"requirements":{"epoch_nonreuse":true,"established_tcp_ttl_close":false,"fake_address_nonreuse":true,"fake_before_rules":true,"first_any_match":true,"full_family":true,"literal_only":true,"no_local_reresolve":true,"no_public_private_fallback":true,"original_monotonic_deadline":true,"positive_negative_cname_min_ttl":true,"private_ambiguity_terminal":true,"revoke_closes_sockets":true,"service_paths_nonrecursive":true,"trusted_binding_before_every_dial":true,"unknown_expired_fake_reject":true},"version":1}}"#;
    assert_eq!(
        bytes(&Policy::default(), &dns(), &desired()),
        GOLDEN.as_bytes()
    );
}

#[test]
fn aggregate_byte_budget_rejects_small_escaped_matrix() {
    // Около 132 KiB входа; escaping удваивает payload ещё до создания Value.
    // Каждая строка проходит прежний лимит 4096, массив меньше MAX_ROWS.
    let policy = Policy {
        user_rules: vec![Rule {
            id: 1,
            enabled: true,
            action: Action::Direct,
            conditions: Conditions {
                executables: vec![format!("/{}", "\"".repeat(4095)); 33],
                ..Conditions::default()
            },
        }],
        ..Policy::default()
    };
    assert_eq!(
        compile_plan(&policy, &dns(), &desired()).unwrap_err(),
        PlanError::Bounds
    );
}
