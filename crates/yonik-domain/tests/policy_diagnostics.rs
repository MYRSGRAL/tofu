use yonik_domain::policy_diagnostics::{Limits, WarningKind, diagnose};
use yonik_domain::{Action, Conditions, Policy, Rule};

fn rule(id: u64, paths: &[&str], nets: &[&str], domains: &[&str]) -> Rule {
    Rule {
        id,
        enabled: true,
        action: Action::Block,
        conditions: Conditions {
            executables: paths.iter().map(|v| v.to_string()).collect(),
            destinations: nets.iter().map(|v| v.parse().unwrap()).collect(),
            domains: domains
                .iter()
                .map(|v| yonik_domain::DomainPattern::Suffix(v.parse().unwrap()))
                .collect(),
        },
    }
}
#[test]
fn subset_and_order() {
    let mut p = Policy {
        user_rules: vec![
            rule(7, &["/a", "/b"], &["10.0.0.0/8"], &["example.org"]),
            rule(9, &["/b"], &["10.1.0.0/16"], &["x.example.org"]),
        ],
        ..Policy::default()
    };
    let r = diagnose(&p, Limits::default()).unwrap();
    assert_eq!(r.warnings.len(), 1);
    assert_eq!(r.warnings[0].kind, WarningKind::Shadowed);
    assert_eq!(r.warnings[0].rule_id, 9);
    p.user_rules.reverse();
    assert_eq!(
        diagnose(&p, Limits::default()).unwrap().warnings[0].kind,
        WarningKind::PotentialOverlap
    );
}
#[test]
fn source_and_domains_are_and_disabled_is_silent() {
    let mut p = Policy {
        user_rules: vec![
            rule(1, &["/a"], &[], &["a.org"]),
            rule(2, &["/b"], &[], &["a.org"]),
            rule(3, &["/a"], &[], &["b.org"]),
        ],
        ..Policy::default()
    };
    assert!(diagnose(&p, Limits::default()).unwrap().warnings.is_empty());
    p.user_rules[1].enabled = false;
    p.user_rules[2].enabled = false;
    assert!(diagnose(&p, Limits::default()).unwrap().warnings.is_empty());
}
#[test]
fn budgets_and_duplicate_ids_are_explicit() {
    let mut p = Policy {
        user_rules: vec![
            rule(1, &[], &[], &[]),
            rule(2, &[], &[], &[]),
            rule(3, &[], &[], &[]),
        ],
        ..Policy::default()
    };
    let r = diagnose(
        &p,
        Limits {
            max_comparisons: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(r.truncated);
    assert_eq!(r.comparisons, 1);
    p.user_rules[2].id = 1;
    assert!(diagnose(&p, Limits::default()).is_err());
    p.user_rules[2].id = 0;
    assert!(diagnose(&p, Limits::default()).is_err());
}

#[test]
fn dns_uncertainty_is_not_proven_intersection() {
    let p = Policy {
        user_rules: vec![
            rule(1, &[], &["10.0.0.0/8"], &[]),
            rule(2, &[], &["192.0.2.0/24"], &["example.org"]),
        ],
        ..Policy::default()
    };
    assert_eq!(
        diagnose(&p, Limits::default()).unwrap().warnings[0].kind,
        WarningKind::PotentialOverlap
    );
}
#[test]
fn exact_domains_and_ipv6_or_lists() {
    let mut p = Policy {
        user_rules: vec![
            rule(1, &[], &["::/0", "0.0.0.0/0"], &["example.org"]),
            rule(2, &[], &["2001:db8::/32", "192.0.2.0/24"], &["example.org"]),
        ],
        ..Policy::default()
    };
    p.user_rules[0].conditions.domains[0] =
        yonik_domain::DomainPattern::Exact("example.org".parse().unwrap());
    assert_eq!(
        diagnose(&p, Limits::default()).unwrap().warnings[0].kind,
        WarningKind::PotentialOverlap
    );
    p.user_rules[1].conditions.domains[0] =
        yonik_domain::DomainPattern::Exact("example.org".parse().unwrap());
    assert_eq!(
        diagnose(&p, Limits::default()).unwrap().warnings[0].kind,
        WarningKind::Shadowed
    );
}
#[test]
fn priority_and_actions_are_preserved_without_mutation() {
    let mut p = Policy::default();
    p.private_networks.push(yonik_domain::PrivateNetwork {
        network: "10.0.0.0/8".parse().unwrap(),
        available: false,
        vpn: yonik_domain::VpnId::new(1).unwrap(),
    });
    p.lan.push("10.0.0.0/8".parse().unwrap());
    p.user_rules.push(rule(8, &[], &["10.1.0.0/16"], &[]));
    p.preset_rules.push(rule(9, &[], &["10.1.0.0/16"], &[]));
    p.preset_rules[0].action = Action::Direct;
    let flow = yonik_domain::Flow {
        executable: None,
        destination: "10.1.0.1".parse().unwrap(),
    };
    let before = p.evaluate(&flow);
    let r = diagnose(&p, Limits::default()).unwrap();
    assert_eq!(p.evaluate(&flow), before);
    assert_eq!(
        r.warnings[0].earlier,
        yonik_domain::RuleOrigin::PrivateNetwork
    );
    assert_eq!(r.warnings[0].kind, WarningKind::PotentialOverlap);
    assert_eq!(r.warnings[1].earlier, yonik_domain::RuleOrigin::Lan);
    assert_eq!(r.warnings[1].kind, WarningKind::Shadowed);
    assert_eq!(
        r.warnings.last().unwrap().earlier,
        yonik_domain::RuleOrigin::User(8)
    );
    assert_ne!(
        r.warnings.last().unwrap().earlier_action,
        Some(r.warnings.last().unwrap().action)
    );
}
#[test]
fn input_and_output_limits() {
    let mut p = Policy {
        user_rules: vec![
            rule(1, &[], &[], &[]),
            rule(2, &[], &[], &[]),
            rule(3, &[], &[], &[]),
        ],
        ..Policy::default()
    };
    let r = diagnose(
        &p,
        Limits {
            max_warnings: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(r.warnings.len(), 1);
    assert!(r.truncated);
    assert!(
        diagnose(
            &p,
            Limits {
                max_rules: 2,
                ..Limits::default()
            }
        )
        .is_err()
    );
    p.user_rules[0]
        .conditions
        .executables
        .push("relative".into());
    assert!(diagnose(&p, Limits::default()).is_err());
    p.user_rules[0].conditions.executables = vec!["/a".into(); 33];
    assert!(diagnose(&p, Limits::default()).is_err());
}

#[test]
fn costly_lists_stop_at_work_budget() {
    let paths = ["/a"; 32];
    let p = Policy {
        user_rules: vec![
            rule(1, &paths, &[], &[]),
            rule(2, &paths, &[], &[]),
            rule(3, &paths, &[], &[]),
        ],
        ..Policy::default()
    };
    let r = diagnose(&p, Limits::default()).unwrap();
    assert!(r.truncated);
    assert!(r.work_units <= 16_777_216);
    assert_eq!(r.comparisons, 1);
}
