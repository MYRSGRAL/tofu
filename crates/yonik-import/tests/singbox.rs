use serde_json::{Value, json};
use yonik_import::{ServerIntent, import_bytes};

fn узел(protocol: &str) -> Value {
    let mut v = json!({"type":protocol,"server":"example.test","server_port":443});
    match protocol {
        "vless" | "vmess" => v["uuid"] = json!("00112233-4455-6677-8899-aabbccddeeff"),
        "trojan" => {
            v["password"] = json!("test-password");
            v["tls"] = json!({"enabled":true});
        }
        "shadowsocks" => {
            v["method"] = json!("aes-128-gcm");
            v["password"] = json!("test-password");
        }
        _ => {
            v["username"] = json!("test-user");
            v["password"] = json!("test-password");
        }
    }
    v
}
#[test]
fn шесть_протоколов_сохраняют_оригинал_и_intent() {
    for p in ["vless", "vmess", "trojan", "shadowsocks", "http", "socks"] {
        let raw = json!({"outbounds":[узел(p)]}).to_string();
        let r = import_bytes(raw.as_bytes().to_vec()).unwrap();
        assert!(r.issues.is_empty(), "{p}: {:?}", r.issues);
        assert_eq!(r.servers.len(), 1);
        assert_eq!(r.original().expose(), raw);
        assert_eq!(r.native_json.as_ref().unwrap().expose(), raw);
        let i = ServerIntent::from_server(&r.servers[0]).unwrap();
        assert!(
            ServerIntent::parse_json(&serde_json::to_string(&i).unwrap())
                .unwrap()
                .into_server()
                .is_ok()
        );
    }
}
#[test]
fn неизвестные_опции_не_активируются() {
    for (key, value) in [
        ("detour", json!("other")),
        ("multiplex", json!({"enabled":true})),
        ("transport", json!({"type":"http"})),
        ("tls", json!({"enabled":true,"insecure":true})),
        ("tls", json!({"enabled":true,"utls":{"enabled":true}})),
    ] {
        let mut n = узел("vless");
        n[key] = value;
        let r = import_bytes(json!({"outbounds":[n]}).to_string().into_bytes()).unwrap();
        assert_eq!(r.servers.len(), 1);
        assert!(ServerIntent::from_server(&r.servers[0]).is_err());
    }
}
#[test]
fn повреждения_и_лимиты_отклоняются() {
    for (key, value) in [
        ("uuid", json!("bad")),
        ("server", json!("bad/path")),
        ("server_port", json!(65536)),
        ("tls", json!({"enabled":"true"})),
        ("tls", json!({"enabled":true,"alpn":["h2,http/1.1"]})),
    ] {
        let mut n = узел("vless");
        n[key] = value;
        let r = import_bytes(json!({"outbounds":[n]}).to_string().into_bytes()).unwrap();
        assert!(r.servers.is_empty());
        assert!(!r.issues.is_empty());
    }
    let mut n = узел("vless");
    n["tag"] = json!("x".repeat(yonik_import::MAX_ENTRY_BYTES));
    let r = import_bytes(json!({"outbounds":[n]}).to_string().into_bytes()).unwrap();
    assert!(r.servers.is_empty());
    let raw = json!({"outbounds":vec![узел("socks");yonik_import::MAX_RECORDS+1]}).to_string();
    assert!(import_bytes(raw.into_bytes()).is_err());
}
#[test]
fn tls_ws_и_grpc_представимы() {
    for p in ["vless", "vmess", "trojan"] {
        for transport in [
            json!({"type":"ws","path":"/test","headers":{"Host":"edge.test"}}),
            json!({"type":"grpc","service_name":"test"}),
        ] {
            let mut n = узел(p);
            n["tls"] = json!({"enabled":true,"server_name":"example.test","alpn":["h2"]});
            n["transport"] = transport;
            let r = import_bytes(json!({"outbounds":[n]}).to_string().into_bytes()).unwrap();
            assert!(
                ServerIntent::from_server(&r.servers[0]).is_ok(),
                "{p}: {:?}",
                r.servers[0].review_flags()
            );
        }
    }
}

#[test]
fn fixture_и_контекст_конфига() {
    let raw = include_str!("corpus/singbox/six-protocols.json");
    let r = import_bytes(raw.as_bytes().to_vec()).unwrap();
    assert_eq!(r.servers.len(), 6);
    for s in &r.servers {
        assert!(ServerIntent::from_server(s).is_ok());
    }
    let mut v: Value = serde_json::from_str(raw).unwrap();
    v["route"] = json!({"final":"other"});
    let r = import_bytes(v.to_string().into_bytes()).unwrap();
    for s in &r.servers {
        assert!(ServerIntent::from_server(s).is_err());
    }
    assert!(import_bytes(vec![b' '; yonik_import::MAX_INPUT_BYTES + 1]).is_err());
    let deep = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert!(import_bytes(deep.into_bytes()).is_err());
}

#[test]
fn mixed_dispatch_и_неизвестные_ws_headers_не_дают_intent() {
    for change in [
        json!({"protocol":"vless"}),
        json!({"protocol":"trojan"}),
        json!({"type":123}),
        json!({"transport":{"type":"grpc","service_name":"exact-service","multi_mode":true}}),
        json!({"type":"unknown"}),
        json!({"transport":{"type":"ws","headers":{"Host":"edge.test","Authorization":"synthetic-secret-header"}}}),
        json!({"transport":{"type":"ws","headers":{"Host":42}}}),
        json!({"transport":{"type":"grpc","service_name":42}}),
    ] {
        let mut node = узел("vless");
        for (key, value) in change.as_object().unwrap() {
            node[key] = value.clone();
        }
        let report = import_bytes(json!({"outbounds":[node]}).to_string().into_bytes()).unwrap();
        assert!(
            report
                .servers
                .iter()
                .all(|server| ServerIntent::from_server(server).is_err())
        );
        assert!(!report.servers.is_empty() || !report.issues.is_empty());
    }
}

#[test]
fn native_credentials_скрыты_в_диагностике() {
    for protocol in ["vless", "vmess", "trojan", "shadowsocks", "http", "socks"] {
        let mut node = узел(protocol);
        node["transport"] =
            json!({"type":"ws","headers":{"Authorization":"synthetic-secret-header"}});
        let report = import_bytes(json!({"outbounds":[node]}).to_string().into_bytes()).unwrap();
        let mut diagnostic = format!("{report:?}");
        for server in &report.servers {
            diagnostic.push_str(&format!("{server:?}"));
            let error = ServerIntent::from_server(server).unwrap_err();
            diagnostic.push_str(&format!("{error:?} {error}"));
        }
        for secret in [
            "test-password",
            "test-user",
            "00112233-4455-6677-8899-aabbccddeeff",
            "synthetic-secret-header",
        ] {
            assert!(!diagnostic.contains(secret), "{protocol}");
        }
    }
}
