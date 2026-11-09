use super::{
    DnsOverTlsModeKnown, DnsProtocolKnown, DnssecModeKnown, GlobalModeKnown, ResolveSupportKnown,
    ResolverScope, Supported, parse_configuration,
};
use crate::resolver::ResolverError;
use serde_json::json;
use std::net::{IpAddr, Ipv6Addr};

#[test]
fn accepts_global_and_link_configuration_with_nullable_fields() {
    assert!(
        parse_configuration(json!({"configuration": [
            {"resolvConfMode":"foreign", "servers":[{
                "address":[192,0,2,53], "family":2, "port":0,
                "accessible":true, "name":null,
                "addressString":"192.0.2.53:53"
            }]},
            {"ifindex":7,"ifname":"test0","defaultRoute":false,
             "dnsOverTLS":null,"searchDomains":[{"name":".","routeOnly":true,"ifindex":7}],
             "servers":[{"address":[32,1,13,184,0,0,0,0,0,0,0,0,0,0,0,53],
             "family":10,"port":853,"ifindex":7,"accessible":false,"name":"dns.example"}]}
        ]}))
        .is_ok()
    );
}

#[test]
fn retains_false_empty_and_unset_separately() {
    let result = parse_configuration(json!({"configuration":[
        {"servers":[],"defaultRoute":false,"dnssecSupported":false},
        {"ifindex":2,"servers":null}
    ]}))
    .unwrap();
    assert_eq!(result.entries[0].servers, Some(vec![]));
    assert_eq!(result.entries[0].default_route, Some(false));
    assert_eq!(result.entries[0].dnssec_supported, Some(false));
    assert_eq!(result.entries[1].servers, None);
    assert_eq!(result.entries[1].default_route, None);
    assert_eq!(result.entries[1].dns_over_tls, None);
}

#[test]
fn preserves_unknown_fields_modes_and_delegate_as_incomplete() {
    let result = parse_configuration(json!({"future":{"answer":42},"configuration":[
        {"resolvConfMode":"foreign", "dnssec":"future-mode", "extra":[1,2]},
        {"delegate":"worker.example", "servers":[], "scopes":[{"protocol":"future","new":true}]}
    ]}))
    .unwrap();
    assert!(result.incomplete);
    assert_eq!(result.unknown_field_count, 3);
    assert_eq!(result.unknown_fields.0["future"], json!({"answer":42}));
    assert_eq!(result.entries[0].unknown_fields.0["extra"], json!([1, 2]));
    assert_eq!(
        result.entries[0].resolv_conf_mode,
        Some(Supported::Known(GlobalModeKnown::Foreign))
    );
    assert_eq!(
        result.entries[0].dnssec,
        Some(Supported::Unsupported("future-mode".into()))
    );
    assert_eq!(result.entries[1].scope, ResolverScope::Delegate);
    assert_eq!(
        result.entries[1].delegate.as_deref(),
        Some("worker.example")
    );
    assert_eq!(
        parse_configuration(json!({"configuration":[{"resolvConfMode":"future"}]}))
            .unwrap()
            .entries[0]
            .resolv_conf_mode,
        Some(Supported::Unsupported("future".into()))
    );
}

#[test]
fn preserves_dns_names_and_redacts_debug_output() {
    let result = parse_configuration(json!({"configuration":[{"searchDomains":[
        {"name":"_service._tcp.example.","routeOnly":false},
        {"name":"escaped\\032name.example", "routeOnly":true},
        {"name":".", "routeOnly":true}
    ],"negativeTrustAnchors":["private.example"],"servers":[{
        "address":[192,0,2,53],"addressString":"private-address-hint", "family":2,
        "port":65535,"name":"private-sni.example","accessible":true,"unknown":"private-value"
    }]}]}))
    .unwrap();
    assert_eq!(
        result.entries[0].search_domains.as_ref().unwrap()[0].name,
        "_service._tcp.example."
    );
    let debug = format!(
        "{result:?} {:?} {:?} {:?}",
        result.entries[0],
        result.entries[0].servers.as_ref().unwrap()[0],
        result.entries[0].search_domains.as_ref().unwrap()[0]
    );
    for secret in ["192", "private", "example", "escaped", "_service"] {
        assert!(!debug.contains(secret), "Debug раскрыл данные: {secret}");
    }
}

#[test]
fn rejects_malformed_types_addresses_and_conflicting_scopes() {
    for parameters in [
        json!({}),
        json!({"configuration":null}),
        json!({"configuration":{}}),
        json!({"configuration":[{"ifindex":0}]}),
        json!({"configuration":[{"ifindex":-1}]}),
        json!({"configuration":[{"ifname":"orphan"}]}),
        json!({"configuration":[{},{}]}),
        json!({"configuration":[{"ifindex":2},{"ifindex":2}]}),
        json!({"configuration":[{"ifindex":2,"delegate":"worker"}]}),
        json!({"configuration":[{"ifindex":2,"fallbackServers":[]}]}),
        json!({"configuration":[{"ifindex":2,"searchDomains":[{"name":".","routeOnly":true,"ifindex":3}]}]}),
        json!({"configuration":[{"searchDomains":[{"name":"."}]}]}),
        json!({"configuration":[{"scopes":[{"protocol":"dns","family":0}]}]}),
        json!({"configuration":[{"scopes":[{"protocol":"dns"},{"protocol":"dns"}]}]}),
        json!({"configuration":[{"defaultRoute":"false"}]}),
    ] {
        assert_eq!(
            parse_configuration(parameters),
            Err(ResolverError::Malformed)
        );
    }
    let server = json!({"address":[192,0,2,53],"family":2,"port":53,"accessible":true});
    for (field, value) in [
        ("address", json!([192, 0, 2])),
        ("address", json!([192, 0, 2, 256])),
        ("family", json!(10)),
        ("family", json!(0)),
        ("port", json!(-1)),
        ("port", json!(65536)),
        ("accessible", json!(null)),
        ("ifindex", json!(0)),
    ] {
        let mut invalid = server.clone();
        invalid[field] = value;
        assert_eq!(
            parse_configuration(json!({"configuration":[{"servers":[invalid]}]})),
            Err(ResolverError::Malformed)
        );
    }
}

#[test]
fn enforces_snapshot_wide_limits_and_string_bounds() {
    let server = json!({"address":[192,0,2,53],"family":2,"port":53,"accessible":true});
    assert_eq!(
        parse_configuration(json!({"configuration":[
            {"servers":vec![server.clone();2048]}, {"ifindex":2,"servers":vec![server;2049]}
        ]})),
        Err(ResolverError::Limit)
    );
    let domain = json!({"name":".","routeOnly":true});
    assert_eq!(
        parse_configuration(json!({"configuration":[
            {"searchDomains":vec![domain.clone();8192]},
            {"ifindex":2,"searchDomains":vec![domain;8193]}
        ]})),
        Err(ResolverError::Limit)
    );
    assert_eq!(
        parse_configuration(
            json!({"configuration":(1..=4097).map(|n|json!({"ifindex":n})).collect::<Vec<_>>()})
        ),
        Err(ResolverError::Limit)
    );
    assert_eq!(
        parse_configuration(
            json!({"configuration":[{"searchDomains":[{"name":"x".repeat(1025),"routeOnly":false}]}]})
        ),
        Err(ResolverError::Limit)
    );
}

#[test]
fn decodes_current_server_scoped_ipv6_and_known_modes() {
    let parsed = parse_configuration(json!({"configuration":[{
        "currentServer":{"address":[254,128,0,0,0,0,0,0,0,0,0,0,0,0,0,53],
            "family":10,"port":853,"ifindex":9,"name":"dns.example","accessible":true,
            "addressString":"[fe80::35%9]:853"},
        "dnssec":"allow-downgrade", "dnsOverTLS":"opportunistic",
        "llmnr":"resolve", "mDNS":"yes", "resolvConfMode":"stub",
        "scopes":[{"protocol":"dns", "dnssec":"yes", "dnsOverTLS":"no"}]
    }]}))
    .unwrap();
    assert!(!parsed.incomplete);
    let entry = &parsed.entries[0];
    assert_eq!(entry.scope, ResolverScope::Global);
    let current = entry.current_server.as_ref().unwrap();
    assert_eq!(
        current.address,
        IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 53))
    );
    assert_eq!(current.ifindex, Some(9));
    assert_eq!(current.port, 853);
    assert_eq!(current.name.as_deref(), Some("dns.example"));
    assert_eq!(
        entry.dnssec,
        Some(Supported::Known(DnssecModeKnown::AllowDowngrade))
    );
    assert_eq!(
        entry.dns_over_tls,
        Some(Supported::Known(DnsOverTlsModeKnown::Opportunistic))
    );
    assert_eq!(
        entry.llmnr,
        Some(Supported::Known(ResolveSupportKnown::Resolve))
    );
    assert_eq!(entry.mdns, Some(Supported::Known(ResolveSupportKnown::Yes)));
    assert_eq!(
        entry.resolv_conf_mode,
        Some(Supported::Known(GlobalModeKnown::Stub))
    );
    let scope = &entry.scopes.as_ref().unwrap()[0];
    assert_eq!(scope.family, None);
    assert_eq!(scope.protocol, Supported::Known(DnsProtocolKnown::Dns));
    assert_eq!(scope.dnssec, Some(Supported::Known(DnssecModeKnown::Yes)));
    assert_eq!(
        scope.dns_over_tls,
        Some(Supported::Known(DnsOverTlsModeKnown::No))
    );
}

#[test]
fn retains_unknown_values_within_wire_frame_without_estimated_size_rejection() {
    let parsed = parse_configuration(json!({"configuration":[],"future":vec![0;140_000]})).unwrap();
    assert!(parsed.incomplete);
    assert_eq!(
        parsed.unknown_fields.0["future"].as_array().unwrap().len(),
        140_000
    );
}
