use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use tofu_import::{Credentials, Protocol, Security, import_bytes};

#[test]
fn ipv6_idna_and_default_ports_are_normalized() {
    let report = import_bytes(
        "trojan://pass@пример.рф:443\nsocks5://[::1]\nhttps://example.test"
            .as_bytes()
            .to_vec(),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(report.issues.is_empty());
    assert_eq!(report.servers[0].endpoint().host(), "xn--e1afmkfd.xn--p1ai");
    assert_eq!(report.servers[1].endpoint().host(), "::1");
    assert_eq!(report.servers[1].endpoint().port(), 1080);
    assert_eq!(report.servers[2].endpoint().port(), 443);
    assert_eq!(report.servers[2].security(), Security::Tls);
}

#[test]
fn shadowsocks_legacy_and_escaped_userinfo_keep_password_exact() {
    let legacy = STANDARD.encode("aes-128-gcm:pass@word:?@example.test:8388");
    let report = import_bytes(
        format!("ss://{legacy}\nss://aes-128-gcm:pass%40word%3A%3F@example.test:8388").into_bytes(),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(report.issues.is_empty());
    assert_eq!(report.servers.len(), 2);
    for server in report.servers {
        match server.credentials() {
            Credentials::Shadowsocks { method, password } => {
                assert_eq!(method.expose(), "aes-128-gcm");
                assert_eq!(password.expose(), "pass@word:?");
            }
            other => panic!("Неожиданный вид credentials: {other:?}"),
        }
    }
}

#[test]
fn urlsafe_unpadded_envelope_and_bom_are_supported() {
    let text = "\u{feff}\nsocks5://127.0.0.1:1080\n";
    let encoded = URL_SAFE_NO_PAD.encode(text);
    let report =
        import_bytes(format!(" \n{encoded}\n").into_bytes()).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(report.base64_envelope);
    assert_eq!(report.servers[0].protocol(), Protocol::Socks5);
}

#[test]
fn json_brackets_in_strings_do_not_consume_nesting_budget() {
    let text = serde_json::json!({"label": "[".repeat(100)}).to_string();
    let report = import_bytes(text.into_bytes()).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(report.native_json.is_some());
}
