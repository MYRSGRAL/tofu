use base64::{Engine, engine::general_purpose::STANDARD};
use yonik_import::{ReviewFlag, Security, import_bytes};

fn vmess(add: &str, aid: serde_json::Value) -> yonik_import::ImportReport {
    let value = serde_json::json!({"add":add,"port":443,"id":"00000000-0000-0000-0000-000000000001","aid":aid});
    import_bytes(format!("vmess://{}", STANDARD.encode(value.to_string())).into_bytes())
        .unwrap_or_else(|e| panic!("{e:?}"))
}

#[test]
fn vmess_json_accepts_bare_ipv6_and_normalizes_it() {
    let report = vmess("2001:db8::1", serde_json::json!(0));
    assert!(report.issues.is_empty());
    assert_eq!(report.servers[0].endpoint().host(), "2001:db8::1");
}

#[test]
fn http_proxy_security_cannot_conflict_with_scheme() {
    for uri in [
        "https://example.test?security=none",
        "https://example.test?security=reality",
        "http://example.test?security=tls",
    ] {
        let report = import_bytes(uri.as_bytes().to_vec()).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(report.servers.is_empty(), "Конфликт схемы не отклонён");
        assert_eq!(report.issues.len(), 1);
    }
    for (uri, security) in [
        ("https://example.test?security=tls", Security::Tls),
        ("http://example.test?security=none", Security::None),
    ] {
        let report = import_bytes(uri.as_bytes().to_vec()).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(report.servers[0].security(), security);
    }
}

#[test]
fn vmess_alter_id_rejects_malformed_values_and_flags_legacy() {
    for aid in [
        serde_json::json!(-1),
        serde_json::json!({}),
        serde_json::json!(true),
        serde_json::json!("bad"),
        serde_json::json!("-1"),
        serde_json::json!(null),
        serde_json::json!(1.5),
        serde_json::json!(65536),
        serde_json::json!("65536"),
    ] {
        let report = vmess("example.test", aid);
        assert!(report.servers.is_empty());
        assert_eq!(report.issues.len(), 1);
    }
    for aid in [
        serde_json::json!(1),
        serde_json::json!("12"),
        serde_json::json!(65535),
    ] {
        let report = vmess("example.test", aid);
        assert!(
            report.servers[0]
                .review_flags()
                .contains(&ReviewFlag::LegacyOption)
        );
    }
    for aid in [serde_json::json!(0), serde_json::json!("0")] {
        let report = vmess("example.test", aid);
        assert!(
            !report.servers[0]
                .review_flags()
                .contains(&ReviewFlag::LegacyOption)
        );
    }
}
