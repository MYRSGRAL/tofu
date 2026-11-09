use std::net::IpAddr;
use tofu_domain::{
    Action, AnswerSet, Cidr, Conditions, Decision, Destination, DestinationFlow, DnsEpoch,
    DomainName, DomainPattern, Evaluation, EvaluationContext, FakeMapping, Policy, PolicyFault,
    PrivateDomain, PrivateNetwork, Resolution, ResolverPath, Route, Rule, VpnId,
};

fn name(value: &str) -> DomainName {
    value.parse().unwrap()
}
fn cidr(value: &str) -> Cidr {
    value.parse().unwrap()
}
fn ip(value: &str) -> IpAddr {
    value.parse().unwrap()
}
fn vpn(value: u64) -> VpnId {
    VpnId::new(value).unwrap()
}
fn rule(id: u64, domains: &[DomainPattern], networks: &[&str], action: Action) -> Rule {
    Rule {
        id,
        enabled: true,
        conditions: Conditions {
            domains: domains.to_vec(),
            destinations: networks.iter().map(|n| cidr(n)).collect(),
            ..Default::default()
        },
        action,
    }
}
fn evaluate(policy: &Policy, name: &DomainName, resolution: Resolution<'_>) -> Evaluation {
    policy.evaluate_destination(
        &DestinationFlow {
            executable: None,
            destination: Destination::Name { name, resolution },
        },
        &EvaluationContext::default(),
    )
}
fn answer(addresses: &[IpAddr]) -> Resolution<'_> {
    Resolution::Answer(AnswerSet {
        addresses,
        complete: true,
        expires_at: 1000,
        epoch: DnsEpoch::default(),
        path: ResolverPath::Policy,
    })
}
fn action(result: Evaluation) -> Route {
    match result {
        Evaluation::Decided { decision, .. } => decision.action,
        other => panic!("Не получено решение: {other:?}"),
    }
}

#[test]
fn idna_case_root_dot_and_suffix_label_boundary() {
    assert_eq!(name("ПРИМЕР.РФ.").as_str(), "xn--e1afmkfd.xn--p1ai");
    assert_eq!(name("EXAMPLE.test."), name("example.test"));
    assert_eq!(name("ＥＸＡＭＰＬＥ．ＴＥＳＴ。"), name("example.test"));
    assert_eq!(
        name("r1---sn-vgqsrn7s.googlevideo.com").as_str(),
        "r1---sn-vgqsrn7s.googlevideo.com"
    );
    let suffix = DomainPattern::Suffix(name("example.test"));
    for value in ["example.test", "www.example.test"] {
        assert!(suffix.matches(&name(value)));
    }
    for value in ["notexample.test", "example.test.evil"] {
        assert!(!suffix.matches(&name(value)));
    }
    for value in [
        "",
        ".test",
        "a..test",
        "-a.test",
        "a_.test",
        "127.0.0.1",
        "a.test..",
        "a.test/path",
        "a\ntest",
    ] {
        assert!(value.parse::<DomainName>().is_err());
    }
}

#[test]
fn d01_private_cidr_precedes_user_domain_and_missing_vpn_blocks() {
    let host = name("public.test");
    let mut policy = Policy::default();
    policy.private_networks.push(PrivateNetwork {
        network: cidr("10.0.0.0/8"),
        available: false,
        vpn: vpn(1),
    });
    policy.user_rules.push(rule(
        1,
        &[DomainPattern::Exact(name("public.test"))],
        &[],
        Action::Proxy,
    ));
    assert_eq!(
        evaluate(&policy, &host, Resolution::Missing),
        Evaluation::NeedResolution(ResolverPath::Policy)
    );
    assert_eq!(
        action(evaluate(&policy, &host, answer(&[ip("10.1.2.3")]))),
        Route::Block
    );
    policy.private_networks[0].available = true;
    assert!(
        matches!(evaluate(&policy, &host, answer(&[ip("10.1.2.3")])), Evaluation::Decided { decision: Decision { action: Route::PrivateVpn, .. }, vpn: Some(id) } if id == vpn(1))
    );
}

#[test]
fn d02_domain_wins_without_lookup_over_younger_cidr() {
    let mut policy = Policy::default();
    policy.user_rules.push(rule(
        1,
        &[DomainPattern::Suffix(name("test"))],
        &[],
        Action::Proxy,
    ));
    policy
        .preset_rules
        .push(rule(2, &[], &["0.0.0.0/0"], Action::Direct));
    assert_eq!(
        action(evaluate(&policy, &name("one.test"), Resolution::Missing)),
        Route::Proxy
    );
}

#[test]
fn d03_any_address_uses_first_rule_independent_of_answer_order() {
    let mut policy = Policy::default();
    policy.user_rules.extend([
        rule(1, &[], &["192.0.2.0/24"], Action::Block),
        rule(2, &[], &["2001:db8::/32"], Action::Direct),
    ]);
    for addresses in [
        [ip("192.0.2.3"), ip("2001:db8::1")],
        [ip("2001:db8::1"), ip("192.0.2.3")],
    ] {
        assert_eq!(
            action(evaluate(&policy, &name("one.test"), answer(&addresses))),
            Route::Block
        );
    }
    let result = policy.evaluate_destination(
        &DestinationFlow {
            executable: None,
            destination: Destination::Ip {
                address: ip("2001:db8::1"),
                name: Some(&name("one.test")),
            },
        },
        &EvaluationContext::default(),
    );
    assert_eq!(action(result), Route::Direct);
}

#[test]
fn d04_private_public_or_different_vpn_answers_are_ambiguous() {
    let mut policy = Policy::default();
    policy.private_networks.extend([
        PrivateNetwork {
            network: cidr("10.0.0.0/8"),
            available: true,
            vpn: vpn(1),
        },
        PrivateNetwork {
            network: cidr("192.168.0.0/16"),
            available: true,
            vpn: vpn(2),
        },
    ]);
    for addresses in [
        [ip("10.1.1.1"), ip("192.0.2.1")],
        [ip("10.1.1.1"), ip("192.168.1.1")],
    ] {
        assert_eq!(
            evaluate(&policy, &name("one.test"), answer(&addresses)),
            Evaluation::Rejected(PolicyFault::DnsPolicyAmbiguous)
        );
    }
}

#[test]
fn d05_failed_or_partial_required_lookup_does_not_reach_younger_domain_rule() {
    let mut policy = Policy::default();
    policy.user_rules.extend([
        rule(1, &[], &["192.0.2.0/24"], Action::Block),
        rule(
            2,
            &[DomainPattern::Suffix(name("test"))],
            &[],
            Action::Proxy,
        ),
    ]);
    assert_eq!(
        evaluate(&policy, &name("one.test"), Resolution::Failed),
        Evaluation::Rejected(PolicyFault::DnsPolicyUnresolved)
    );
    let addresses = [ip("2001:db8::1")];
    let Resolution::Answer(mut set) = answer(&addresses) else {
        unreachable!()
    };
    set.complete = false;
    assert_eq!(
        evaluate(&policy, &name("one.test"), Resolution::Answer(set)),
        Evaluation::Rejected(PolicyFault::DnsPolicyUnresolved)
    );
    set.complete = true; // NODATA для IPv4 при успешном AAAA — полный ответ.
    assert_eq!(
        action(evaluate(
            &policy,
            &name("one.test"),
            Resolution::Answer(set)
        )),
        Route::Proxy
    );
}

#[test]
fn d06_private_zone_never_requests_public_resolver() {
    let mut policy = Policy::default();
    policy.private_domains.push(PrivateDomain {
        domain: DomainPattern::Suffix(name("corp.test")),
        vpn: vpn(1),
        available: false,
    });
    assert_eq!(
        action(evaluate(
            &policy,
            &name("one.corp.test"),
            Resolution::Missing
        )),
        Route::Block
    );
    policy.private_domains[0].available = true;
    assert_eq!(
        evaluate(&policy, &name("one.corp.test"), Resolution::Missing),
        Evaluation::NeedResolution(ResolverPath::Vpn(vpn(1)))
    );
    assert_eq!(
        evaluate(&policy, &name("one.corp.test"), answer(&[ip("10.0.0.1")])),
        Evaluation::Rejected(PolicyFault::WrongResolver)
    );
}

#[test]
fn d07_stale_dns_requests_refresh_and_unknown_fake_mapping_is_rejected() {
    let mut policy = Policy::default();
    policy
        .user_rules
        .push(rule(1, &[], &["192.0.2.0/24"], Action::Block));
    let addresses = [ip("192.0.2.1")];
    let Resolution::Answer(mut set) = answer(&addresses) else {
        unreachable!()
    };
    set.expires_at = 0;
    assert_eq!(
        evaluate(&policy, &name("one.test"), Resolution::Answer(set)),
        Evaluation::NeedResolution(ResolverPath::Policy)
    );
    set.expires_at = 1000;
    set.epoch.network = 2;
    assert_eq!(
        evaluate(&policy, &name("one.test"), Resolution::Answer(set)),
        Evaluation::NeedResolution(ResolverPath::Policy)
    );
    let pools = [cidr("198.18.0.0/15")];
    let context = EvaluationContext {
        fake_pools: &pools,
        ..Default::default()
    };
    let flow = DestinationFlow {
        executable: None,
        destination: Destination::Fake {
            address: ip("198.18.0.2"),
            mapping: None,
            resolution: Resolution::Missing,
        },
    };
    assert_eq!(
        policy.evaluate_destination(&flow, &context),
        Evaluation::Rejected(PolicyFault::UnknownFakeMapping)
    );
}

#[test]
fn fake_ip_is_never_compared_with_real_cidr_and_mapping_is_session_bound() {
    let mut policy = Policy::default();
    policy.user_rules.extend([
        rule(1, &[], &["198.18.0.0/15"], Action::Block),
        rule(
            2,
            &[DomainPattern::Exact(name("one.test"))],
            &[],
            Action::Direct,
        ),
    ]);
    let pools = [cidr("198.18.0.0/15")];
    let context = EvaluationContext {
        fake_pools: &pools,
        ..Default::default()
    };
    let mut mapping = FakeMapping {
        address: ip("198.18.0.2"),
        name: name("one.test"),
        epoch: DnsEpoch::default(),
        expires_at: 1000,
    };
    let addresses = [ip("192.0.2.3")];
    let evaluate = |mapping: &FakeMapping| {
        policy.evaluate_destination(
            &DestinationFlow {
                executable: None,
                destination: Destination::Fake {
                    address: ip("198.18.0.2"),
                    mapping: Some(mapping),
                    resolution: answer(&addresses),
                },
            },
            &context,
        )
    };
    assert_eq!(action(evaluate(&mapping)), Route::Direct);
    mapping.epoch.engine = 1;
    assert_eq!(
        evaluate(&mapping),
        Evaluation::Rejected(PolicyFault::UnknownFakeMapping)
    );
}

#[test]
fn source_short_circuit_and_destination_or_do_not_trigger_needless_dns() {
    let mut policy = Policy::default();
    let mut first = rule(1, &[], &["192.0.2.0/24"], Action::Block);
    first.conditions.executables.push("/usr/bin/other".into());
    policy.user_rules.extend([
        first,
        rule(
            2,
            &[DomainPattern::Exact(name("one.test"))],
            &["192.0.2.0/24"],
            Action::Direct,
        ),
    ]);
    assert_eq!(
        action(evaluate(&policy, &name("one.test"), Resolution::Missing)),
        Route::Direct
    );
}

#[test]
fn overlapping_vpn_owners_and_private_resolver_route_are_explicit() {
    let mut policy = Policy::default();
    policy.private_domains.push(PrivateDomain {
        domain: DomainPattern::Suffix(name("corp.test")),
        vpn: vpn(1),
        available: true,
    });
    let addresses = [ip("10.1.2.3")];
    let Resolution::Answer(mut set) = answer(&addresses) else {
        unreachable!()
    };
    set.path = ResolverPath::Vpn(vpn(1));
    assert!(
        matches!(evaluate(&policy, &name("a.corp.test"), Resolution::Answer(set)), Evaluation::Decided { decision: Decision { action: Route::PrivateVpn, .. }, vpn: Some(id) } if id == vpn(1))
    );
    policy.private_networks.extend([
        PrivateNetwork {
            network: cidr("10.0.0.0/8"),
            vpn: vpn(1),
            available: true,
        },
        PrivateNetwork {
            network: cidr("10.1.0.0/16"),
            vpn: vpn(2),
            available: true,
        },
    ]);
    assert_eq!(
        evaluate(&policy, &name("a.corp.test"), Resolution::Answer(set)),
        Evaluation::Rejected(PolicyFault::DnsPolicyAmbiguous)
    );
}

#[test]
fn fake_address_cannot_enter_as_literal_or_real_dns_answer() {
    let pools = [cidr("198.18.0.0/15")];
    let context = EvaluationContext {
        fake_pools: &pools,
        ..Default::default()
    };
    let mut policy = Policy::default();
    policy
        .user_rules
        .push(rule(1, &[], &["0.0.0.0/0"], Action::Direct));
    for address in [ip("198.18.0.2"), ip("::ffff:198.18.0.2")] {
        let flow = DestinationFlow {
            executable: None,
            destination: Destination::Ip {
                address,
                name: None,
            },
        };
        assert_eq!(
            policy.evaluate_destination(&flow, &context),
            Evaluation::Rejected(PolicyFault::UnknownFakeMapping)
        );
    }
    let host = name("one.test");
    let addresses = [ip("198.18.0.2")];
    let flow = DestinationFlow {
        executable: None,
        destination: Destination::Name {
            name: &host,
            resolution: answer(&addresses),
        },
    };
    assert_eq!(
        policy.evaluate_destination(&flow, &context),
        Evaluation::Rejected(PolicyFault::DnsPolicyUnresolved)
    );
}

#[test]
fn mapped_private_address_and_every_dns_epoch_are_checked() {
    let mut policy = Policy::default();
    policy.private_networks.push(PrivateNetwork {
        network: cidr("10.0.0.0/8"),
        available: false,
        vpn: vpn(1),
    });
    let flow = DestinationFlow {
        executable: None,
        destination: Destination::Ip {
            address: ip("::ffff:10.0.0.1"),
            name: None,
        },
    };
    assert_eq!(
        action(policy.evaluate_destination(&flow, &EvaluationContext::default())),
        Route::Block
    );
    let addresses = [ip("192.0.2.1")];
    for epoch in [
        DnsEpoch {
            network: 1,
            ..Default::default()
        },
        DnsEpoch {
            engine: 1,
            ..Default::default()
        },
        DnsEpoch {
            resolver: 1,
            ..Default::default()
        },
    ] {
        let Resolution::Answer(mut set) = answer(&addresses) else {
            unreachable!()
        };
        set.epoch = epoch;
        assert_eq!(
            evaluate(&policy, &name("one.test"), Resolution::Answer(set)),
            Evaluation::NeedResolution(ResolverPath::Policy)
        );
    }
}
