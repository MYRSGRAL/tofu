use base64::{Engine, engine::general_purpose::STANDARD};
use yonik_import::{ParsedServer, import_bytes};

fn server(uri: &str) -> ParsedServer {
    let mut report = import_bytes(uri.as_bytes().to_vec()).unwrap();
    assert!(report.issues.is_empty(), "{:?}", report.issues);
    report.servers.remove(0)
}
fn material(uri: &str) -> Vec<u8> {
    server(uri).identity_material().unwrap().expose().to_vec()
}
fn vmess(json: &str) -> String {
    format!("vmess://{}", STANDARD.encode(json))
}

#[test]
fn names_do_not_change_identity_for_all_protocols() {
    let vm = vmess(
        r#"{"add":"example.org","port":443,"id":"AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA","ps":"Имя"}"#,
    );
    for uri in [
        "vless://aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa@example.org:443".to_owned(),
        vm,
        "trojan://secret@example.org:443".to_owned(),
        "ss://aes-128-gcm:secret@example.org:443".to_owned(),
        "http://user:secret@example.org:443".to_owned(),
        "socks5://user:secret@example.org:443".to_owned(),
    ] {
        assert_eq!(
            material(&format!("{uri}#one")),
            material(&format!("{uri}#two"))
        );
        assert_eq!(material(&uri), material(&uri));
        if !uri.starts_with("vmess://") {
            assert_ne!(
                material(&uri),
                material(&uri.replace("example.org", "other.org"))
            );
        }
    }
}

#[test]
fn query_encoding_order_and_unknown_options() {
    let a = "trojan://secret@example.org:443?future=a%20b&type=ws#one";
    let b = "trojan://secret@example.org:443?type=ws&future=a+b#two";
    assert_eq!(material(a), material(b));
    assert_ne!(material(a), material(&a.replace("a%20b", "different")));
    assert_ne!(material(a), material(&a.replace("secret", "changed")));
    assert_ne!(material(a), material(&a.replace("type=ws", "type=grpc")));
}

#[test]
fn vmess_recursive_canonicalization() {
    let a = vmess(
        r#"{"add":"example.org","port":"443","id":"AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA","ps":"Первое","future":{"b":2,"a":1}}"#,
    );
    let b = vmess(
        r#"{"future":{"a":1,"b":2},"ps":"Второе","id":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","port":443,"add":"example.org"}"#,
    );
    assert_eq!(material(&a), material(&b));
    assert_ne!(
        material(&a),
        material(&vmess(
            r#"{"add":"example.org","port":443,"id":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","future":{"a":2,"b":2}}"#
        ))
    );
    assert_eq!(server(&a).display_name().expose(), "Первое");
}

#[test]
fn display_names_are_safe_bounded_and_source_is_unchanged() {
    let uri = "trojan://secret@[2001:db8::1]:443#%00%0A%E2%80%AEИмя";
    let parsed = server(uri);
    assert_eq!(parsed.display_name().expose(), "Имя");
    assert_eq!(parsed.source().expose(), uri);
    assert_eq!(
        server("trojan://secret@[2001:db8::1]:443#%ZZ")
            .display_name()
            .expose(),
        "[2001:db8::1]:443"
    );
    let long = server(&format!(
        "trojan://secret@example.org:443#{}",
        "Я".repeat(400)
    ));
    assert_eq!(long.display_name().expose().len(), 512);
    let identity = parsed.identity_material().unwrap();
    assert!(!format!("{identity:?}").contains("secret"));
}

#[test]
fn every_protocol_tracks_credentials_and_options() {
    for uri in [
        "vless://aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa@example.org:443?future=1",
        "trojan://secret@example.org:443?future=1",
        "ss://aes-128-gcm:secret@example.org:443?future=1",
        "http://user:secret@example.org:443?future=1",
        "socks5://user:secret@example.org:443?future=1",
    ] {
        let changed = if uri.starts_with("vless") {
            uri.replace(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
            )
        } else {
            uri.replace("secret", "changed")
        };
        assert_ne!(material(uri), material(&changed));
        assert_ne!(
            material(uri),
            material(&uri.replace("future=1", "future=2"))
        );
    }
    let a = r#"{"add":"example.org","port":443,"id":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","future":1}"#;
    for b in [
        a.replace("example.org", "other.org"),
        a.replace(
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
        ),
        a.replace("\"future\":1", "\"future\":2"),
    ] {
        assert_ne!(material(&vmess(a)), material(&vmess(&b)));
    }
}

#[test]
fn legacy_shadowsocks_uses_decoded_credentials() {
    let old = format!(
        "ss://{}#first",
        STANDARD.encode("aes-128-gcm:secret@example.org:443")
    );
    assert_eq!(
        material(&old),
        material("ss://aes-128-gcm:secret@example.org:443#second")
    );
}

#[test]
fn reserved_path_escape_does_not_collapse_different_options() {
    assert_ne!(
        material("trojan://secret@example.org:443/route%2Finner"),
        material("trojan://secret@example.org:443/route/inner"),
    );
}

#[test]
fn unknown_json_numbers_never_round_to_the_same_identity() {
    for future in [
        "18446744073709551616",
        "{\"array\":[18446744073709551616]}",
        "1.0000000000000000000000000000000001",
    ] {
        let first = vmess(&format!(
            "{{\"add\":\"example.org\",\"port\":443,\"id\":\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\",\"future\":{future}}}"
        ));
        let changed = future
            .replace("18446744073709551616", "18446744073709551617")
            .replace("0001", "0002");
        let second = vmess(&format!(
            "{{\"add\":\"example.org\",\"port\":443,\"id\":\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\",\"future\":{changed}}}"
        ));
        assert_ne!(material(&first), material(&second));
    }
}

#[test]
fn vmess_keeps_nested_duplicate_keys_and_their_relative_order() {
    let profile = |future: &str, name: &str| {
        vmess(&format!(
            r#"{{"add":"example.org","port":443,"id":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","ps":"{name}","future":{future}}}"#
        ))
    };
    let duplicate = profile(r#"{"a":1,"b":3,"a":2}"#, "Первое");
    assert_ne!(
        material(&duplicate),
        material(&profile(r#"{"a":2,"b":3}"#, "Первое"))
    );
    assert_ne!(
        material(&duplicate),
        material(&profile(r#"{"a":2,"b":3,"a":1}"#, "Первое"))
    );
    assert_eq!(
        material(&duplicate),
        material(&profile(r#"{"b":3,"a":1,"a":2}"#, "Второе"))
    );
    let nested = profile(r#"[{"a":1,"a":2,"b":[true,null,"x"]}]"#, "Первое");
    assert_eq!(
        material(&nested),
        material(&profile(r#"[{"b":[true,null,"x"],"a":1,"a":2}]"#, "Второе"))
    );
    assert_ne!(
        material(&nested),
        material(&profile(r#"[{"a":2,"b":[true,null,"x"]}]"#, "Первое"))
    );
}
