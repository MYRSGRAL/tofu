use yonik_domain::{
    Action, AndroidPackageId, Conditions, Destination, DestinationFlow, Evaluation,
    EvaluationContext, PlatformSource, Policy, PolicyFault, Rule,
};

#[test]
fn package_grammar_preserves_case() {
    for value in ["org.Example_app", "a.b", "org.app123"] {
        assert_eq!(AndroidPackageId::parse(value).unwrap().as_str(), value);
    }
    for value in [
        "a",
        ".app",
        "org.",
        "org..app",
        "org.1app",
        "org.ap-p",
        "org/abc",
        "орг.app",
        "org.app\n",
    ] {
        assert!(AndroidPackageId::parse(value).is_none(), "{value:?}");
    }
    assert!(AndroidPackageId::parse(&format!("a.{}", "b".repeat(252))).is_none());
}

#[test]
fn unsupported_precedes_matching_user_lan_default_for_every_action() {
    for action in [Action::Direct, Action::Proxy, Action::Block] {
        let mut policy = Policy {
            default_action: action,
            lan: vec!["192.168.0.0/16".parse().unwrap()],
            ..Default::default()
        };
        policy.user_rules.push(Rule {
            id: 41,
            enabled: true,
            action,
            conditions: Conditions {
                executables: vec!["/usr/bin/client".into()],
                platform_sources: vec![PlatformSource::AndroidPackage {
                    package: AndroidPackageId::parse("org.example.app").unwrap(),
                }],
                ..Default::default()
            },
        });
        let flow = DestinationFlow {
            executable: Some("/usr/bin/client"),
            destination: Destination::Ip {
                address: "192.168.1.1".parse().unwrap(),
                name: None,
            },
        };
        assert_eq!(
            policy.evaluate_destination(&flow, &EvaluationContext::default()),
            Evaluation::Rejected(PolicyFault::UnsupportedSelector(41))
        );
        policy.user_rules[0].enabled = false;
        assert!(matches!(
            policy.evaluate_destination(&flow, &EvaluationContext::default()),
            Evaluation::Decided { .. }
        ));
        assert_eq!(policy.user_rules[0].conditions.platform_sources.len(), 1);
    }
}
