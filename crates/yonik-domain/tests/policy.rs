use std::net::IpAddr;
use yonik_domain::{
    Action, Cidr, Conditions, Decision, Flow, Policy, PrivateNetwork, Route, Rule, RuleOrigin,
};

fn ip(value: &str) -> IpAddr {
    value
        .parse()
        .unwrap_or_else(|_| panic!("Неверный тестовый IP"))
}

fn cidr(value: &str) -> Cidr {
    value
        .parse()
        .unwrap_or_else(|_| panic!("Неверная тестовая подсеть"))
}

fn flow<'a>(executable: Option<&'a str>, address: &str) -> Flow<'a> {
    Flow {
        executable,
        destination: ip(address),
    }
}

fn rule(id: u64, paths: &[&str], networks: &[&str], action: Action) -> Rule {
    Rule {
        id,
        enabled: true,
        conditions: Conditions {
            executables: paths.iter().map(|p| (*p).to_owned()).collect(),
            destinations: networks.iter().map(|p| cidr(p)).collect(),
            domains: Vec::new(),
        },
        action,
    }
}

#[test]
fn canonical_networks_and_boundaries() {
    for invalid in [
        "10.0.0.1/24",
        "::1/64",
        "10.0.0.0/33",
        "::/129",
        "10.0.0.0",
        "garbage",
    ] {
        assert!(invalid.parse::<Cidr>().is_err(), "{invalid}");
    }
    assert!(cidr("10.0.0.0/24").contains(ip("10.0.0.255")));
    assert!(!cidr("10.0.0.0/24").contains(ip("10.0.1.0")));
    assert!(cidr("2001:db8::/32").contains(ip("2001:db8::ffff")));
    assert!(!cidr("2001:db8::/32").contains(ip("2001:db9::")));
    assert!(cidr("0.0.0.0/0").contains(ip("255.255.255.255")));
    assert!(cidr("::/0").contains(ip("ffff::")));
    assert!(!cidr("::/0").contains(ip("1.1.1.1")));
    assert!(cidr("::1/128").contains(ip("::1")));
    assert!(!cidr("::1/128").contains(ip("::2")));
}

#[test]
fn private_network_cannot_escape_to_lan_user_or_preset() {
    let mut policy = Policy::default();
    policy.private_networks.push(PrivateNetwork {
        network: cidr("10.0.0.0/24"),
        available: false,
        vpn: yonik_domain::VpnId::new(1).unwrap(),
    });
    policy.lan.push(cidr("10.0.0.0/24"));
    policy.user_rules.push(rule(1, &[], &[], Action::Proxy));
    assert_eq!(
        policy.evaluate(&flow(None, "10.0.0.1")),
        Decision {
            action: Route::Block,
            origin: RuleOrigin::PrivateNetwork
        }
    );
    policy.private_networks[0].available = true;
    assert_eq!(
        policy.evaluate(&flow(None, "10.0.0.1")).action,
        Route::PrivateVpn
    );
}

#[test]
fn overlapping_private_networks_block_if_any_required_route_is_missing() {
    let mut policy = Policy::default();
    policy.private_networks.extend([
        PrivateNetwork {
            network: cidr("10.0.0.0/8"),
            available: true,
            vpn: yonik_domain::VpnId::new(1).unwrap(),
        },
        PrivateNetwork {
            network: cidr("10.1.0.0/16"),
            available: false,
            vpn: yonik_domain::VpnId::new(1).unwrap(),
        },
    ]);
    assert_eq!(
        policy.evaluate(&flow(None, "10.1.2.3")).action,
        Route::Block
    );
    policy.private_networks.reverse();
    assert_eq!(
        policy.evaluate(&flow(None, "10.1.2.3")).action,
        Route::Block
    );
    assert_eq!(
        policy.evaluate(&flow(None, "10.2.2.3")).action,
        Route::PrivateVpn
    );
}

#[test]
fn source_and_destination_are_and_values_are_or() {
    let mut policy = Policy::default();
    policy.user_rules.push(rule(
        7,
        &["/usr/bin/a", "/usr/bin/b"],
        &["192.0.2.0/24", "2001:db8::/32"],
        Action::Block,
    ));
    for address in ["192.0.2.3", "2001:db8::1"] {
        assert_eq!(
            policy.evaluate(&flow(Some("/usr/bin/b"), address)).action,
            Route::Block
        );
        for path in [None, Some("b"), Some("/opt/b")] {
            assert_eq!(policy.evaluate(&flow(path, address)).action, Route::Proxy);
        }
    }
    assert_eq!(
        policy
            .evaluate(&flow(Some("/usr/bin/b"), "198.51.100.1"))
            .action,
        Route::Proxy
    );
}

#[test]
fn lan_user_preset_and_default_have_explicit_order() {
    let mut policy = Policy::default();
    policy.lan.push(cidr("192.0.2.0/24"));
    let mut disabled = rule(1, &[], &[], Action::Block);
    disabled.enabled = false;
    policy.user_rules.extend([
        disabled,
        rule(2, &["/a"], &[], Action::Block),
        rule(3, &[], &[], Action::Proxy),
    ]);
    policy.preset_rules.push(rule(4, &[], &[], Action::Direct));
    assert_eq!(
        policy.evaluate(&flow(Some("/a"), "192.0.2.1")).origin,
        RuleOrigin::Lan
    );
    assert_eq!(
        policy.evaluate(&flow(Some("/a"), "198.51.100.1")).origin,
        RuleOrigin::User(2)
    );
    assert_eq!(
        policy.evaluate(&flow(None, "198.51.100.1")).origin,
        RuleOrigin::User(3)
    );
    policy.user_rules.clear();
    assert_eq!(
        policy.evaluate(&flow(None, "198.51.100.1")).origin,
        RuleOrigin::Preset(4)
    );
    policy.preset_rules.clear();
    assert_eq!(
        policy.evaluate(&flow(None, "198.51.100.1")),
        Decision {
            action: Route::Proxy,
            origin: RuleOrigin::Default
        }
    );
}
