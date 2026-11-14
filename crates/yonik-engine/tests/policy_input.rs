use std::io::{self, Read};

use yonik_domain::{Action, DomainPattern};
use yonik_engine::policy_input::{PolicyInputError, read_policy};

fn read(input: &str) -> Result<yonik_domain::Policy, PolicyInputError> {
    read_policy(input.as_bytes())
}

#[test]
fn defaults_and_normalization() {
    let policy = read(r#"{"version":1,"user_rules":[{"id":0,"action":"direct","conditions":{"domain":["ПРИМЕР.РФ."],"domain_suffix":["EXAMPLE.COM."],"ip_cidr":["2001:db8::/32"],"executable_paths":["/usr/bin/browser"]}}],"private_networks":[{"cidr":"10.0.0.0/8","vpn":1,"available":false}],"private_domains":[{"suffix":"INTERNAL.EXAMPLE","vpn":2,"available":true}]}"#).unwrap();
    assert_eq!(policy.default_action, Action::Proxy);
    let rule = &policy.user_rules[0];
    assert!(rule.enabled);
    assert_eq!(rule.action, Action::Direct);
    assert_eq!(
        rule.conditions.domains[0].name().as_str(),
        "xn--e1afmkfd.xn--p1ai"
    );
    assert!(
        matches!(&rule.conditions.domains[1], DomainPattern::Suffix(name) if name.as_str() == "example.com")
    );
    assert_eq!(rule.conditions.destinations[0].to_string(), "2001:db8::/32");
    assert_eq!(rule.conditions.executables, ["/usr/bin/browser"]);
    assert_eq!(policy.private_networks[0].vpn.get(), 1);
    assert!(!policy.private_networks[0].available);
    assert_eq!(
        policy.private_domains[0].domain.name().as_str(),
        "internal.example"
    );
    let catch_all = read(r#"{"version":1,"user_rules":[{"id":1,"action":"block"}]}"#).unwrap();
    assert!(catch_all.user_rules[0].conditions.domains.is_empty());
    assert!(catch_all.user_rules[0].conditions.destinations.is_empty());
    assert!(catch_all.user_rules[0].conditions.executables.is_empty());
}

#[test]
fn rejects_malformed_unknown_duplicate_and_missing_fields() {
    for input in [
        "{",
        "{}",
        r#"{"version":1} {}"#,
        r#"{"version":1,"version":1}"#,
        r#"{"version":1,"secret-key":"secret-value"}"#,
        r#"{"version":1,"default_action":"private_vpn"}"#,
        r#"{"version":1,"user_rules":[{"id":1,"action":"direct","action":"block"}]}"#,
        r#"{"version":1,"user_rules":[{"id":1,"action":"direct","conditions":{"unknown":[]}}]}"#,
        r#"{"version":1,"private_networks":[{"cidr":"10.0.0.0/8","vpn":1,"available":true,"unknown":0}]}"#,
        r#"{"version":1,"private_domains":[{"suffix":"example.com","vpn":1,"available":true,"vpn":2}]}"#,
    ] {
        assert!(matches!(read(input), Err(PolicyInputError::InvalidJson)));
    }
}

#[test]
fn rejects_invalid_semantics() {
    for input in [
        r#"{"version":4}"#,
        r#"{"version":1,"lan":["10.1.2.3/8"]}"#,
        r#"{"version":1,"lan":["::ffff:192.0.2.0/120"]}"#,
        r#"{"version":1,"private_domains":[{"suffix":"*.example.com","vpn":1,"available":true}]}"#,
        r#"{"version":1,"private_networks":[{"cidr":"10.0.0.0/8","vpn":0,"available":true}]}"#,
        r#"{"version":1,"user_rules":[{"id":1,"action":"proxy","conditions":{"executable_paths":["relative"]}}]}"#,
        r#"{"version":1,"user_rules":[{"id":1,"action":"proxy","conditions":{"executable_paths":["/bad\u0000path"]}}]}"#,
        r#"{"version":1,"user_rules":[{"id":1,"action":"proxy","conditions":{"domain":["192.0.2.1"]}}]}"#,
    ] {
        assert!(matches!(read(input), Err(PolicyInputError::InvalidPolicy)));
    }
    let long_path = format!("/{}", "a".repeat(4096));
    let input = serde_json::json!({"version":1,"user_rules":[{"id":1,"action":"proxy","conditions":{"executable_paths":[long_path]}}]});
    assert!(matches!(
        read(&input.to_string()),
        Err(PolicyInputError::InvalidPolicy)
    ));
}

#[test]
fn ids_are_unique_in_each_group_including_disabled_rules() {
    assert!(matches!(
        read(
            r#"{"version":1,"user_rules":[{"id":1,"action":"proxy"},{"id":1,"enabled":false,"action":"direct"}]}"#
        ),
        Err(PolicyInputError::InvalidPolicy)
    ));
    assert!(matches!(
        read(
            r#"{"version":1,"preset_rules":[{"id":1,"action":"proxy"},{"id":1,"action":"direct"}]}"#
        ),
        Err(PolicyInputError::InvalidPolicy)
    ));
    assert!(read(r#"{"version":1,"user_rules":[{"id":1,"action":"proxy"}],"preset_rules":[{"id":1,"action":"direct"}]}"#).is_ok());
}

#[test]
fn aggregate_rule_and_selector_limits() {
    let rules: Vec<_> = (0..10_000)
        .map(|id| serde_json::json!({"id":id,"action":"proxy"}))
        .collect();
    let mut input = serde_json::json!({"version":1,"user_rules":rules});
    assert!(read(&input.to_string()).is_ok());
    input["preset_rules"] = serde_json::json!([{"id":0,"action":"block"}]);
    assert!(matches!(
        read(&input.to_string()),
        Err(PolicyInputError::Limit)
    ));
    let selectors = vec!["a"; 100_000];
    let mut input = serde_json::json!({"version":1,"user_rules":[{"id":1,"action":"proxy","conditions":{"domain":selectors}}]});
    assert!(read(&input.to_string()).is_ok());
    input["lan"] = serde_json::json!(["10.0.0.0/8"]);
    assert!(matches!(
        read(&input.to_string()),
        Err(PolicyInputError::Limit)
    ));
}

#[test]
fn read_is_bounded_and_errors_never_include_input() {
    struct Endless {
        consumed: usize,
    }
    impl Read for Endless {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            buffer.fill(b' ');
            self.consumed += buffer.len();
            Ok(buffer.len())
        }
    }
    let mut reader = Endless { consumed: 0 };
    assert!(matches!(
        read_policy(&mut reader),
        Err(PolicyInputError::TooLarge)
    ));
    assert_eq!(reader.consumed, 2 * 1024 * 1024 + 1);
    let mut exact = br#"{"version":1}"#.to_vec();
    exact.resize(2 * 1024 * 1024, b' ');
    assert!(read_policy(exact.as_slice()).is_ok());
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("secret-value"))
        }
    }
    let error = read_policy(Broken).unwrap_err();
    assert_eq!(error, PolicyInputError::Io);
    for error in [
        error,
        read(r#"{"version":1,"secret-key":"secret-value"}"#).unwrap_err(),
    ] {
        let display = error.to_string();
        let debug = format!("{error:?}");
        assert!(!display.contains("secret"));
        assert!(!debug.contains("secret"));
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn p07_closed_input_retains_sources() {
    let input = r#"{"version":2,"user_rules":[{"id":41,"enabled":false,"action":"direct","conditions":{"platform_sources":[{"kind":"android_package","package":"org.Example_app"}]}}]}"#;
    assert_eq!(
        read(input).unwrap().user_rules[0]
            .conditions
            .platform_sources
            .len(),
        1
    );
    for invalid in [
        input.replace("org.Example_app", "org.1app"),
        input.replace("android_package", "future"),
        input.replace("\"version\":2", "\"version\":1"),
        input.replace("\"package\":", "\"other\":0,\"package\":"),
    ] {
        assert!(read(&invalid).is_err());
    }
}
