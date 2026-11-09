use serde_json::{Value, json};
use tofu_import::{FatalError, IssueCode, ReviewFlag, Security, Transport, import_bytes};

fn outbound() -> Value {
    json!({"tag":"proxy", "protocol":"vless", "settings":{"vnext":[{"address":"example.test","port":443,"users":[{"id":"11111111-1111-4111-8111-111111111111","encryption":"none","flow":"xtls-rprx-vision"}]}]},"streamSettings":{"network":"tcp","security":"reality","realitySettings":{"serverName":"sni.test","fingerprint":"chrome","publicKey":"FAKE_PUBLIC_KEY","shortId":"abcd","spiderX":"/a?b=c"}}})
}
fn parse(value: Value) -> tofu_import::ImportReport {
    import_bytes(value.to_string().into_bytes()).unwrap()
}
#[test]
fn remna_array_extracts_only_servers_and_keeps_native_json() {
    let report = parse(
        json!([{"remarks":"Имя # &", "outbounds":[outbound(),{"protocol":"freedom"},{"protocol":"blackhole"},{"protocol":"dns"}]}]),
    );
    assert_eq!(report.servers.len(), 1);
    assert!(report.issues.is_empty());
    assert!(report.native_json.is_some());
    let server = &report.servers[0];
    assert_eq!(server.endpoint().host(), "example.test");
    assert_eq!(server.security(), Security::Reality);
    assert!(server.review_flags().is_empty());
    let uri = url::Url::parse(server.source().expose()).unwrap();
    assert!(uri.fragment().is_some());
    assert_eq!(
        uri.query_pairs().find(|(k, _)| k == "spx").unwrap().1,
        "/a?b=c"
    );
    assert!(!format!("{report:?}").contains("FAKE_PUBLIC_KEY"));
}
#[test]
fn maps_ws_grpc_xhttp_without_losing_parameters() {
    for (network, field, options, expected, key, value) in [
        (
            "ws",
            "wsSettings",
            json!({"path":"/one?two=3", "headers":{"Host":"cdn.test"}}),
            Transport::WebSocket,
            "host",
            "cdn.test",
        ),
        (
            "grpc",
            "grpcSettings",
            json!({"serviceName":"service/a","authority":"grpc.test"}),
            Transport::Grpc,
            "serviceName",
            "service/a",
        ),
        (
            "xhttp",
            "xhttpSettings",
            json!({"path":"/x", "host":"x.test","mode":"auto"}),
            Transport::Xhttp,
            "mode",
            "auto",
        ),
    ] {
        let mut node = outbound();
        node["streamSettings"]["network"] = json!(network);
        node["streamSettings"][field] = options;
        let report = parse(json!({"outbounds":[node]}));
        assert_eq!(report.servers.len(), 1);
        let server = &report.servers[0];
        assert_eq!(server.transport(), expected);
        let uri = url::Url::parse(server.source().expose()).unwrap();
        assert_eq!(uri.query_pairs().find(|(k, _)| k == key).unwrap().1, value);
    }
}
#[test]
fn unknown_protocol_and_malformed_credentials_are_reported() {
    let mut invalid = outbound();
    invalid["settings"]["vnext"][0]["users"][0]["id"] = json!("CANARY_SECRET");
    let report = parse(json!({"outbounds":[invalid,{"protocol":"unknown"}]}));
    assert!(report.servers.is_empty());
    assert_eq!(report.issues.len(), 2);
    assert_eq!(report.issues[1].code, IssueCode::UnsupportedProtocol);
    assert!(!format!("{report:?}").contains("CANARY_SECRET"));
}
#[test]
fn unsupported_semantics_are_blocking_review_flags() {
    for (field, value) in [
        ("proxySettings", json!({"tag":"other"})),
        ("mux", json!({"enabled":true})),
    ] {
        let mut node = outbound();
        node[field] = value;
        let report = parse(json!({"outbounds":[node]}));
        assert!(
            report.servers[0]
                .review_flags()
                .contains(&ReviewFlag::UnsupportedOption)
        );
    }
    let mut node = outbound();
    node["streamSettings"]["sockopt"] = json!({"dialerProxy":"other"});
    let report = parse(json!({"outbounds":[node]}));
    assert!(
        report.servers[0]
            .review_flags()
            .contains(&ReviewFlag::UnsupportedOption)
    );
}
#[test]
fn rejects_duplicate_nested_auth_fields() {
    let text = json!({"outbounds":[outbound()]}).to_string().replace(
        "\"encryption\":\"none\"",
        "\"encryption\":\"none\",\"encryption\":\"unsafe\"",
    );
    assert_eq!(
        import_bytes(text.into_bytes()).unwrap_err().kind(),
        FatalError::InvalidJson
    );
}
#[test]
fn unknown_native_is_opaque_and_oversized_outbound_is_rejected() {
    assert!(
        parse(json!({"outbounds":[{"type":"vless","server":"example.test"}]}))
            .servers
            .is_empty()
    );
    let mut node = outbound();
    node["tag"] = json!("x".repeat(tofu_import::MAX_ENTRY_BYTES));
    let report = parse(json!({"outbounds":[node]}));
    assert!(report.servers.is_empty());
    assert_eq!(report.issues[0].code, IssueCode::TooLarge);
}

#[test]
fn endpoints_users_and_names_are_distinct_and_stable() {
    let mut node = outbound();
    let mut second = node["settings"]["vnext"][0].clone();
    second["address"] = json!("second.test");
    second["users"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"22222222-2222-4222-8222-222222222222","encryption":"none"}));
    node["settings"]["vnext"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let input = json!({"remarks":"Имя\nУзла", "outbounds":[node]});
    let a = parse(input.clone());
    let b = parse(input);
    assert_eq!(a.servers.len(), 3);
    for (first, second) in a.servers.iter().zip(&b.servers) {
        assert_eq!(first.source().expose(), second.source().expose());
        assert!(!first.source().expose().contains('\n'));
    }
    let names: std::collections::HashSet<_> = a
        .servers
        .iter()
        .map(|s| {
            url::Url::parse(s.source().expose())
                .unwrap()
                .fragment()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(names.len(), 3);
}
#[test]
fn tls_unsafe_and_unknown_nested_options_cannot_be_lost() {
    let mut node = outbound();
    node["streamSettings"] = json!({"network":"tcp","security":"tls","tlsSettings":{"serverName":"verify.test","fingerprint":"chrome","alpn":["h2","http/1.1"],"allowInsecure":true}});
    let report = parse(json!({"outbounds":[node.clone()]}));
    assert_eq!(report.servers[0].security(), Security::Tls);
    assert!(
        report.servers[0]
            .review_flags()
            .contains(&ReviewFlag::UnsafeOption)
    );
    node["streamSettings"]["tlsSettings"]["allowInsecure"] = json!(false);
    node["streamSettings"]["tlsSettings"]["publicKey"] = json!("FAKE_SECRET");
    let report = parse(json!({"outbounds":[node]}));
    assert!(
        report.servers[0]
            .review_flags()
            .contains(&ReviewFlag::UnsupportedOption)
    );
}
#[test]
fn native_json_bounds_and_duplicate_security_fields_are_enforced() {
    let raw = json!({"outbounds":[outbound()]}).to_string().replace(
        "\"security\":\"reality\"",
        "\"security\":\"tls\",\"security\":\"reality\"",
    );
    assert_eq!(
        import_bytes(raw.into_bytes()).unwrap_err().kind(),
        FatalError::InvalidJson
    );
    let deep = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert_eq!(
        import_bytes(deep.into_bytes()).unwrap_err().kind(),
        FatalError::JsonTooDeep
    );
    let too_many =
        json!({"outbounds":vec![json!({"protocol":"freedom"}); tofu_import::MAX_RECORDS+1]});
    assert_eq!(
        import_bytes(too_many.to_string().into_bytes())
            .unwrap_err()
            .kind(),
        FatalError::TooManyRecords
    );
}

#[test]
fn escaped_root_keys_are_recognized_and_large_names_are_bounded() {
    let raw = json!({"outbounds":[outbound()]})
        .to_string()
        .replace("\"outbounds\"", "\"out\\u0062ounds\"");
    assert_eq!(import_bytes(raw.into_bytes()).unwrap().servers.len(), 1);
    let report = parse(
        json!({"remarks":"x".repeat(tofu_import::MAX_ENTRY_BYTES), "outbounds":[outbound()]}),
    );
    assert_eq!(report.servers.len(), 1);
    assert!(report.servers[0].source().expose().len() < 2048);
}

#[test]
fn explicitly_disabled_mux_and_plain_tcp_header_are_noops() {
    let mut node = outbound();
    node["mux"] = json!({"enabled":false});
    node["streamSettings"]["tcpSettings"] = json!({"header":{"type":"none"}});
    let report = parse(json!({"outbounds":[node.clone()]}));
    assert!(report.servers[0].review_flags().is_empty());
    node["mux"]["enabled"] = json!(true);
    let report = parse(json!({"outbounds":[node]}));
    assert!(
        report.servers[0]
            .review_flags()
            .contains(&ReviewFlag::UnsupportedOption)
    );
}

#[test]
fn unrelated_native_numbers_keep_their_original_opaque_representation() {
    let raw = br#"{"native":{"large":1e400},"array":[-1e999]}"#;
    let report = import_bytes(raw.to_vec()).unwrap();
    assert!(report.servers.is_empty());
    assert_eq!(report.native_json.unwrap().expose().as_bytes(), raw);
}

fn shadowsocks() -> Value {
    json!({"protocol":"shadowsocks","tag":"SS", "settings":{"servers":[{"address":"ss.test","port":8388,"method":"chacha20-ietf-poly1305","password":"FAKE@PASS:/?#%"}]}})
}
#[test]
fn shadowsocks_json_preserves_credentials_endpoints_and_native_source() {
    let mut node = shadowsocks();
    let mut second = node["settings"]["servers"][0].clone();
    second["address"] = json!("2001:db8::1");
    node["settings"]["servers"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let report = parse(json!([{ "remarks":"SS узел", "outbounds":[node,{"protocol":"hysteria"}]}]));
    assert_eq!(report.servers.len(), 2);
    assert_eq!(report.issues[0].code, IssueCode::UnsupportedProtocol);
    assert!(report.native_json.is_some());
    let first = &report.servers[0];
    assert_eq!(first.protocol(), tofu_import::Protocol::Shadowsocks);
    assert_eq!(report.servers[1].endpoint().host(), "2001:db8::1");
    assert!(first.review_flags().is_empty());
    match first.credentials() {
        tofu_import::Credentials::Shadowsocks { method, password } => {
            assert_eq!(method.expose(), "chacha20-ietf-poly1305");
            assert_eq!(password.expose(), "FAKE@PASS:/?#%");
        }
        _ => panic!("Ожидались Shadowsocks credentials"),
    }
    assert!(!format!("{report:?}").contains("FAKE"));
}
#[test]
fn shadowsocks_unknown_options_and_malformed_credentials_are_not_lost() {
    for (field, value) in [
        ("plugin", json!("unknown")),
        ("proxySettings", json!({"tag":"other"})),
    ] {
        let mut node = shadowsocks();
        node[field] = value;
        let report = parse(json!({"outbounds":[node]}));
        assert!(
            report.servers[0]
                .review_flags()
                .contains(&ReviewFlag::UnsupportedOption)
        );
    }
    let mut node = shadowsocks();
    node["settings"]["servers"][0]["ota"] = json!(true);
    let report = parse(json!({"outbounds":[node.clone()]}));
    assert!(
        report.servers[0]
            .review_flags()
            .contains(&ReviewFlag::UnsupportedOption)
    );
    node["settings"]["servers"][0]["password"] = json!("");
    let report = parse(json!({"outbounds":[node]}));
    assert!(report.servers.is_empty());
    assert_eq!(report.issues[0].code, IssueCode::InvalidCredentials);
}
#[test]
fn disabled_tls_session_resumption_is_noop_but_enabled_or_malformed_is_blocked() {
    for (value, blocked) in [
        (json!(false), false),
        (json!(true), true),
        (json!("false"), true),
    ] {
        let mut node = outbound();
        node["streamSettings"] = json!({"network":"tcp","security":"tls","tlsSettings":{"serverName":"tls.test","enableSessionResumption":value}});
        let report = parse(json!({"outbounds":[node]}));
        assert_eq!(
            report.servers[0]
                .review_flags()
                .contains(&ReviewFlag::UnsupportedOption),
            blocked
        );
    }
}
