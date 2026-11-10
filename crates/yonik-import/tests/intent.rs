use yonik_import::{ServerIntent, import_bytes};

#[test]
fn закрытый_контракт_и_ограничение_размера() {
    assert!(ServerIntent::parse_json(&" ".repeat(65537)).is_err());
    assert!(ServerIntent::parse_json(r#"{"protocol":"vless","raw_config":{}}"#).is_err());
}

#[test]
fn сохраняет_протоколы_и_секреты_не_попадают_в_debug() {
    let sources = [
        "vless://11111111-1111-1111-1111-111111111111@example.org:443?security=tls&type=ws&path=%2Ftest&host=example.org",
        "trojan://synthetic-password@example.org:443?type=grpc&serviceName=test",
        "ss://aes-128-gcm:synthetic-password@example.org:8388",
        "https://synthetic-user:synthetic-password@example.org:443",
        "socks5://synthetic-user:synthetic-password@example.org:1080",
        "vmess://eyJ2IjoiMiIsImFkZCI6ImV4YW1wbGUub3JnIiwicG9ydCI6NDQzLCJpZCI6IjExMTExMTExLTExMTEtMTExMS0xMTExLTExMTExMTExMTExMSIsIm5ldCI6IndzIiwidGxzIjoidGxzIiwicGF0aCI6Ii90ZXN0In0=",
    ];
    for source in sources {
        let report = import_bytes(source.as_bytes().to_vec()).unwrap();
        let original = &report.servers[0];
        let intent = ServerIntent::from_server(original).unwrap();
        assert!(!format!("{intent:?}").contains("example.org"));
        let encoded = serde_json::to_string(&intent).unwrap();
        let recovered = ServerIntent::parse_json(&encoded)
            .unwrap()
            .into_server()
            .unwrap();
        assert_eq!(original.protocol(), recovered.protocol());
        assert_eq!(original.transport(), recovered.transport());
        assert_eq!(original.security(), recovered.security());
        assert_eq!(original.endpoint().host(), recovered.endpoint().host());
        let encoded_again =
            serde_json::to_string(&ServerIntent::from_server(&recovered).unwrap()).unwrap();
        assert_eq!(encoded, encoded_again);
    }
}

fn образец() -> serde_json::Value {
    let report = import_bytes(
        b"vless://11111111-1111-1111-1111-111111111111@example.org:443?security=tls".to_vec(),
    )
    .unwrap();
    serde_json::to_value(ServerIntent::from_server(&report.servers[0]).unwrap()).unwrap()
}

#[test]
fn отклоняет_неизвестные_повторные_и_избыточные_поля() {
    let valid = serde_json::to_string(&образец()).unwrap();
    assert!(
        ServerIntent::parse_json(&valid.replacen("{", r#"{"host":"attacker.example","#, 1))
            .is_err()
    );
    let mut wire = образец();
    wire["options"]["raw_config"] = serde_json::json!({"outbounds": []});
    assert!(ServerIntent::parse_json(&wire.to_string()).is_err());
    let mut wire = образец();
    wire["credentials"]["password"] = serde_json::json!("unexpected-synthetic");
    assert!(ServerIntent::parse_json(&wire.to_string()).is_err());
    let mut wire = образец();
    wire["options"]["alpn"] = serde_json::json!(vec!["h2"; 17]);
    assert!(ServerIntent::parse_json(&wire.to_string()).is_err());
    let mut wire = образец();
    wire["options"]["path"] = serde_json::json!("/not-for-tcp");
    assert!(ServerIntent::parse_json(&wire.to_string()).is_err());
    let mut wire = образец();
    wire["options"]["path"] = serde_json::json!([[[[[[[["deep"]]]]]]]]);
    assert!(ServerIntent::parse_json(&wire.to_string()).is_err());
}

#[test]
fn проверяет_endpoint_uuid_и_управляющие_символы() {
    for host in [
        "",
        "bad host.example",
        "bad\nhost.example",
        "bad.example/path",
        "bad.example%00",
        "-bad.example",
    ] {
        let mut wire = образец();
        wire["host"] = serde_json::json!(host);
        assert!(
            ServerIntent::parse_json(&wire.to_string()).is_err(),
            "{host:?}"
        );
    }
    let mut wire = образец();
    wire["credentials"]["uuid"] = serde_json::json!("invalid-synthetic");
    assert!(ServerIntent::parse_json(&wire.to_string()).is_err());
    for port in [0, 65536] {
        let mut wire = образец();
        wire["port"] = serde_json::json!(port);
        assert!(ServerIntent::parse_json(&wire.to_string()).is_err());
    }
    let mut wire = образец();
    wire["host"] = serde_json::json!("пример.рф");
    assert_eq!(
        ServerIntent::parse_json(&wire.to_string())
            .unwrap()
            .into_server()
            .unwrap()
            .endpoint()
            .host(),
        "xn--e1afmkfd.xn--p1ai"
    );
}

#[test]
fn reality_xhttp_и_неподдерживаемые_опции() {
    for source in [
        "vless://11111111-1111-1111-1111-111111111111@example.org:443?security=reality&fp=chrome&pbk=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&sid=0123&flow=xtls-rprx-vision&spx=%2Ftest",
        "vless://11111111-1111-1111-1111-111111111111@example.org:443?security=tls&type=xhttp&mode=stream-up&path=%2Ftest&alpn=h2%2Chttp%2F1.1",
        "trojan://synthetic-password@example.org:443?type=grpc&serviceName=test&mode=gun&authority=example.org%3A443",
    ] {
        let report = import_bytes(source.as_bytes().to_vec()).unwrap();
        let first = ServerIntent::from_server(&report.servers[0]).unwrap();
        let encoded = serde_json::to_string(&first).unwrap();
        let server = ServerIntent::parse_json(&encoded)
            .unwrap()
            .into_server()
            .unwrap();
        assert_eq!(
            encoded,
            serde_json::to_string(&ServerIntent::from_server(&server).unwrap()).unwrap()
        );
    }
    for query in [
        "unknown=option",
        "headerType=http",
        "allowInsecure=1",
        "type=quic",
        "security=reality&fp=chrome&pbk=invalid",
        "security=tls&fp=unknown",
        "type=ws&path=%2F%3Fed%3D1",
    ] {
        let report = import_bytes(
            format!("vless://11111111-1111-1111-1111-111111111111@example.org:443?{query}")
                .into_bytes(),
        )
        .unwrap();
        let error = ServerIntent::from_server(&report.servers[0]).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("example.org"));
    }
}

#[test]
fn сохраняет_http_host_с_портом_и_скрывает_ошибку_секрета() {
    for host in ["example.org:443", "[::1]:443", "EXAMPLE.org"] {
        let source = format!(
            "vless://11111111-1111-1111-1111-111111111111@example.org:443?security=tls&type=ws&host={host}"
        );
        let report = import_bytes(source.into_bytes()).unwrap();
        let intent = ServerIntent::from_server(&report.servers[0]).unwrap();
        let encoded = serde_json::to_string(&intent).unwrap();
        let restored = ServerIntent::parse_json(&encoded)
            .unwrap()
            .into_server()
            .unwrap();
        assert_eq!(
            encoded,
            serde_json::to_string(&ServerIntent::from_server(&restored).unwrap()).unwrap()
        );
    }
    let mut wire = образец();
    wire["credentials"]["password"] = serde_json::json!(123456789);
    let error = ServerIntent::parse_json(&wire.to_string()).unwrap_err();
    assert!(!format!("{error:?} {error}").contains("123456789"));
    wire["credentials"]["password"] = serde_json::Value::Null;
    wire["transport"] = serde_json::json!("ws");
    wire["options"]["host"] = serde_json::json!("example.org:443 injected");
    assert!(ServerIntent::parse_json(&wire.to_string()).is_err());
}
