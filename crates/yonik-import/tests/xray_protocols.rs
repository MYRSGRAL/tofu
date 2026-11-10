use serde_json::{Value, json};
use yonik_import::{Protocol, ServerIntent, import_bytes};

fn узел(protocol: &str) -> Value {
    let endpoint = json!({"address":"пример.рф","port":443});
    let mut node = json!({"protocol":protocol,"settings":{},"streamSettings":{"network":"ws","security":"tls","tlsSettings":{"serverName":"example.test"},"wsSettings":{"path":"/test","headers":{"Host":"cdn.test"}}}});
    let mut endpoint = endpoint;
    if protocol == "vmess" {
        endpoint["users"] =
            json!([{"id":"11111111-1111-4111-8111-111111111111","security":"auto","alterId":0}]);
        node["settings"]["vnext"] = json!([endpoint]);
    } else {
        endpoint["password"] = json!("CANARY@:/?#%секрет");
        node["settings"]["servers"] = json!([endpoint]);
    }
    node
}

#[test]
fn извлекает_оба_протокола_и_сохраняет_intent_и_исходник() {
    for (name, protocol) in [("vmess", Protocol::Vmess), ("trojan", Protocol::Trojan)] {
        let raw = json!({"routing":{"rules":[]},"outbounds":[узел(name)]}).to_string();
        let report = import_bytes(raw.as_bytes().to_vec()).unwrap();
        assert_eq!(report.servers.len(), 1);
        assert_eq!(report.native_json.as_ref().unwrap().expose(), raw);
        let server = &report.servers[0];
        assert_eq!(server.protocol(), protocol);
        if let yonik_import::Credentials::Password(password) = server.credentials() {
            assert_eq!(password.expose(), "CANARY@:/?#%секрет");
        }
        assert_eq!(server.endpoint().host(), "xn--e1afmkfd.xn--p1ai");
        assert!(server.review_flags().is_empty());
        assert_eq!(
            server.identity_material().unwrap().expose(),
            import_bytes(raw.as_bytes().to_vec()).unwrap().servers[0]
                .identity_material()
                .unwrap()
                .expose()
        );
        let intent = ServerIntent::from_server(server).unwrap();
        let encoded = serde_json::to_string(&intent).unwrap();
        let recovered = ServerIntent::parse_json(&encoded)
            .unwrap()
            .into_server()
            .unwrap();
        assert_eq!(
            serde_json::to_string(&ServerIntent::from_server(&recovered).unwrap()).unwrap(),
            encoded
        );
        assert!(!format!("{report:?}").contains("CANARY"));
    }
}

#[test]
fn alpn_с_запятой_не_превращается_в_несколько_протоколов() {
    for protocol in ["vmess", "trojan"] {
        for alpn in [json!(["h2,http/1.1"]), json!(["h2", "custom,token"])] {
            let mut node = узел(protocol);
            node["streamSettings"]["tlsSettings"]["alpn"] = alpn;
            let raw = json!({"outbounds":[node]}).to_string();
            let report = import_bytes(raw.as_bytes().to_vec()).unwrap();
            assert!(
                report.servers.is_empty(),
                "{protocol}: непредставимый ALPN принят"
            );
            assert_eq!(report.issues.len(), 1);
            assert_eq!(report.native_json.unwrap().expose(), raw);
        }
        let mut node = узел(protocol);
        node["streamSettings"]["tlsSettings"]["alpn"] = json!(["h2", "http/1.1"]);
        let report = import_bytes(json!({"outbounds":[node]}).to_string().into_bytes()).unwrap();
        assert_eq!(report.servers.len(), 1);
        assert!(report.issues.is_empty());
        assert!(ServerIntent::from_server(&report.servers[0]).is_ok());
    }
}

#[test]
fn неоднозначные_массивы_отклоняются_целиком() {
    for protocol in ["vmess", "trojan"] {
        let key = if protocol == "vmess" {
            "vnext"
        } else {
            "servers"
        };
        for count in [0, 2] {
            let mut node = узел(protocol);
            let endpoint = node["settings"][key][0].clone();
            node["settings"][key] = json!(vec![endpoint; count]);
            let report =
                import_bytes(json!({"outbounds":[node]}).to_string().into_bytes()).unwrap();
            assert!(report.servers.is_empty());
            assert_eq!(report.issues.len(), 1);
            assert!(report.native_json.is_some());
        }
    }
    let mut node = узел("vmess");
    let user = node["settings"]["vnext"][0]["users"][0].clone();
    node["settings"]["vnext"][0]["users"] = json!([user.clone(), user]);
    let report = import_bytes(json!({"outbounds":[node]}).to_string().into_bytes()).unwrap();
    assert!(report.servers.is_empty());
    assert_eq!(report.issues.len(), 1);
}

#[test]
fn неизвестные_и_небезопасные_поля_блокируют_intent() {
    for protocol in ["vmess", "trojan"] {
        for location in [
            "root",
            "settings",
            "endpoint",
            "credential",
            "tls",
            "transport",
            "unsafe",
        ] {
            let mut node = узел(protocol);
            let key = if protocol == "vmess" {
                "vnext"
            } else {
                "servers"
            };
            match location {
                "root" => node["proxySettings"] = json!({"tag":"other"}),
                "settings" => node["settings"]["unknown"] = json!(true),
                "endpoint" => node["settings"][key][0]["unknown"] = json!(true),
                "credential" if protocol == "vmess" => {
                    node["settings"][key][0]["users"][0]["unknown"] = json!(true)
                }
                "credential" => node["settings"][key][0]["flow"] = json!("unknown"),
                "tls" => node["streamSettings"]["tlsSettings"]["unknown"] = json!(true),
                "transport" => node["streamSettings"]["wsSettings"]["unknown"] = json!(true),
                _ => node["streamSettings"]["tlsSettings"]["allowInsecure"] = json!(true),
            }
            let raw = json!({"outbounds":[node]}).to_string();
            let report = import_bytes(raw.as_bytes().to_vec()).unwrap();
            assert_eq!(report.servers.len(), 1, "{protocol}/{location}");
            assert!(
                !report.servers[0].review_flags().is_empty(),
                "{protocol}/{location}"
            );
            assert!(ServerIntent::from_server(&report.servers[0]).is_err());
            assert_eq!(report.native_json.unwrap().expose(), raw);
        }
    }
}

#[test]
fn повреждённые_endpoint_и_credentials_отклоняются() {
    for protocol in ["vmess", "trojan"] {
        let key = if protocol == "vmess" {
            "vnext"
        } else {
            "servers"
        };
        for (field, value) in [
            ("address", json!("bad/path")),
            ("address", json!("bad host")),
            ("port", json!(0)),
            ("port", json!(65536)),
            ("port", json!("443")),
            ("port", Value::Null),
        ] {
            let mut node = узел(protocol);
            node["settings"][key][0][field] = value;
            let report =
                import_bytes(json!({"outbounds":[node]}).to_string().into_bytes()).unwrap();
            assert!(report.servers.is_empty());
            assert_eq!(report.issues.len(), 1);
        }
        let mut node = узел(protocol);
        if protocol == "vmess" {
            node["settings"][key][0]["users"][0]["id"] = json!("CANARY");
        } else {
            node["settings"][key][0]["password"] = json!("");
        }
        let report = import_bytes(json!({"outbounds":[node]}).to_string().into_bytes()).unwrap();
        assert!(report.servers.is_empty());
        assert!(!format!("{report:?}").contains("CANARY"));
    }
}

#[test]
fn дубли_escaped_keys_размер_и_глубина_проверяются_до_экстракции() {
    use yonik_import::{FatalError, IssueCode, MAX_ENTRY_BYTES};
    for protocol in ["vmess", "trojan"] {
        let raw = json!({"outbounds":[узел(protocol)]})
            .to_string()
            .replace("\"port\":443", r#""port":443,"p\u006frt":444"#);
        assert_eq!(
            import_bytes(raw.into_bytes()).unwrap_err().kind(),
            FatalError::InvalidJson
        );
        let mut node = узел(protocol);
        node["unknown"] = json!("x".repeat(MAX_ENTRY_BYTES));
        let report = import_bytes(json!({"outbounds":[node]}).to_string().into_bytes()).unwrap();
        assert!(report.servers.is_empty());
        assert_eq!(report.issues[0].code, IssueCode::TooLarge);
    }
    let deep = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert_eq!(
        import_bytes(deep.into_bytes()).unwrap_err().kind(),
        FatalError::JsonTooDeep
    );
}

#[test]
fn ipv6_grpc_и_непредставимые_reality_опции_не_теряются() {
    for protocol in ["vmess", "trojan"] {
        let mut node = узел(protocol);
        let key = if protocol == "vmess" {
            "vnext"
        } else {
            "servers"
        };
        node["settings"][key][0]["address"] = json!("2001:db8::1");
        node["streamSettings"] = json!({"network":"grpc","security":"tls","grpcSettings":{"serviceName":"service/a"},"tlsSettings":{"serverName":"sni.test"}});
        let report =
            import_bytes(json!({"outbounds":[node.clone()]}).to_string().into_bytes()).unwrap();
        let server = &report.servers[0];
        assert_eq!(server.endpoint().host(), "2001:db8::1");
        assert_eq!(server.transport(), yonik_import::Transport::Grpc);
        assert!(ServerIntent::from_server(server).is_ok());
        node["streamSettings"]
            .as_object_mut()
            .unwrap()
            .remove("tlsSettings");
        node["streamSettings"]["security"] = json!("reality");
        node["streamSettings"]["realitySettings"] = json!({"publicKey":"FAKE_PUBLIC","shortId":"abcd","serverName":"sni.test","fingerprint":"chrome","unknown":true});
        let report = import_bytes(json!({"outbounds":[node]}).to_string().into_bytes()).unwrap();
        assert_eq!(report.servers.len(), 1);
        assert!(ServerIntent::from_server(&report.servers[0]).is_err());
        assert!(!format!("{report:?}").contains("FAKE_PUBLIC"));
    }
}

#[test]
fn legacy_vmess_и_неизвестные_credentials_остаются_на_review() {
    for value in [json!(1), json!(65535)] {
        let mut node = узел("vmess");
        node["settings"]["vnext"][0]["users"][0]["alterId"] = value;
        let report = import_bytes(json!({"outbounds":[node]}).to_string().into_bytes()).unwrap();
        assert!(
            report.servers[0]
                .review_flags()
                .contains(&yonik_import::ReviewFlag::LegacyOption)
        );
        assert!(ServerIntent::from_server(&report.servers[0]).is_err());
    }
    for value in [Value::Null, json!(-1), json!("0"), json!(65536)] {
        let mut node = узел("vmess");
        node["settings"]["vnext"][0]["users"][0]["alterId"] = value;
        let report = import_bytes(json!({"outbounds":[node]}).to_string().into_bytes()).unwrap();
        assert!(report.servers.is_empty());
        assert_eq!(report.issues.len(), 1);
    }
}
