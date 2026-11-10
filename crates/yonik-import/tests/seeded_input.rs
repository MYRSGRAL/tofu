use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use yonik_import::{Credentials, ServerIntent, import_bytes};

const SEED: u64 = 0x5040_4652_3037_2026;
const CORPUS: [&str; 3] = [
    include_str!("corpus/p04-seeded/uri.txt"),
    include_str!("corpus/p04-seeded/xray.json"),
    include_str!("corpus/p04-seeded/sing-box.json"),
];

#[test]
fn скрытый_fragment_legacy_ss_не_теряется() {
    for suffix in ["#hidden", "#", "?hidden=1", "/hidden"] {
        let uri = format!(
            "ss://{}",
            STANDARD.encode(format!("aes-128-gcm:pass@example.test:8388{suffix}"))
        );
        let text = format!("{uri}#display\nsocks5://127.0.0.1:1080");
        let report = import_bytes(text.as_bytes().to_vec()).unwrap();
        assert_eq!(report.original().expose(), text);
        assert_eq!(report.servers.len(), 1, "suffix={suffix}");
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].line, 1);
    }
    // Fragment снаружи Base64 — допустимое имя, разделители внутри пароля допустимы.
    let uri = format!(
        "ss://{}#display",
        STANDARD.encode("aes-128-gcm:p#?/@example.test:8388")
    );
    let report = import_bytes(uri.into_bytes()).unwrap();
    assert!(report.issues.is_empty());
    assert_eq!(report.servers.len(), 1);
    match report.servers[0].credentials() {
        Credentials::Shadowsocks { password, .. } => assert_eq!(password.expose(), "p#?/"),
        _ => panic!("Ожидался Shadowsocks"),
    }
}

fn снимок(bytes: Vec<u8>) -> String {
    match import_bytes(bytes.clone()) {
        Ok(report) => {
            assert_eq!(report.original().expose().as_bytes(), bytes);
            let mut result = format!("{}:{:?}", report.base64_envelope, report.issues);
            if let Some(native) = &report.native_json {
                // Native не пересериализуется и не теряет неизвестные поля.
                result.push_str(native.expose());
            }
            for server in &report.servers {
                result.push_str(server.source().expose());
                result.push_str(server.parameters().expose());
                result.push_str(server.endpoint().host());
                result.push_str(&format!(
                    "{}:{:?}:{:?}",
                    server.endpoint().port(),
                    server.protocol(),
                    server.review_flags()
                ));
                result.push_str(&STANDARD.encode(server.identity_material().unwrap().expose()));
            }
            result
        }
        Err(error) => {
            assert_eq!(error.input(), bytes);
            assert!(error.input_is_complete());
            format!("{:?}", error.kind())
        }
    }
}

#[test]
fn seeded_мутации_детерминированы_и_сохраняют_вход() {
    let mut state = SEED;
    for source in CORPUS {
        for iteration in 0..512 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let mut bytes = source.as_bytes().to_vec();
            let index = state as usize % bytes.len();
            match iteration % 4 {
                0 => {
                    bytes[index] = (state >> 32) as u8;
                }
                1 => {
                    bytes.remove(index);
                }
                2 => {
                    bytes.insert(index, b"%\\\"{}[]=@?#\0"[iteration % 12]);
                }
                _ => {
                    bytes.truncate(index);
                }
            }
            assert!(bytes.len() <= source.len() + 1);
            for input in [
                bytes.clone(),
                STANDARD.encode(&bytes).into_bytes(),
                URL_SAFE_NO_PAD.encode(&bytes).into_bytes(),
            ] {
                assert_eq!(
                    снимок(input.clone()),
                    снимок(input),
                    "seed={SEED:x}, iteration={iteration}"
                );
            }
        }
    }
}

#[test]
fn генерируемые_credentials_endpoint_и_ручной_roundtrip() {
    for (host, expected) in [
        ("пример.рф", "xn--e1afmkfd.xn--p1ai"),
        ("192.0.2.1", "192.0.2.1"),
        ("[2001:db8::1]", "2001:db8::1"),
    ] {
        for port in [1, 443, 65535] {
            for password in ["p@ss:?/#%", "пароль", "a+b c", "%00"] {
                let encoded: String = password
                    .as_bytes()
                    .iter()
                    .map(|b| format!("%{b:02X}"))
                    .collect();
                let source = format!("trojan://{encoded}@{host}:{port}");
                for input in [
                    source.clone(),
                    STANDARD.encode(&source),
                    URL_SAFE_NO_PAD.encode(&source),
                ] {
                    let report = import_bytes(input.as_bytes().to_vec()).unwrap();
                    assert_eq!(report.original().expose(), input);
                    assert!(report.issues.is_empty());
                    let server = &report.servers[0];
                    assert_eq!(server.endpoint().host(), expected);
                    assert_eq!(server.endpoint().port(), port);
                    match server.credentials() {
                        Credentials::Password(value) => assert_eq!(value.expose(), password),
                        _ => panic!("Ожидался Trojan"),
                    }
                    let wire =
                        serde_json::to_string(&ServerIntent::from_server(server).unwrap()).unwrap();
                    let restored = ServerIntent::parse_json(&wire)
                        .unwrap()
                        .into_server()
                        .unwrap();
                    assert_eq!(
                        serde_json::to_string(&ServerIntent::from_server(&restored).unwrap())
                            .unwrap(),
                        wire
                    );
                }
            }
        }
    }
}

#[test]
fn native_обоих_cores_сохраняется_при_обёртке_а_дубли_отклоняются() {
    for native in &CORPUS[1..] {
        for input in [
            native.to_string(),
            STANDARD.encode(native),
            URL_SAFE_NO_PAD.encode(native),
        ] {
            let report = import_bytes(input.as_bytes().to_vec()).unwrap();
            assert_eq!(report.original().expose(), input);
            assert_eq!(report.native_json.as_ref().unwrap().expose(), *native);
            if native.contains("protocol") {
                assert_eq!(report.servers.len(), 1);
                assert!(!report.servers[0].review_flags().is_empty());
                assert!(ServerIntent::from_server(&report.servers[0]).is_err());
            } else {
                assert!(report.servers.is_empty());
            }
        }
        // Эквивалентные escaped keys не могут обойти проверку уникальности.
        let duplicate = native.replace("\"nested\":[1,2]", r#""nested":[1,2],"ne\u0073ted":3"#);
        assert_ne!(duplicate, *native);
        for input in [duplicate.clone(), STANDARD.encode(&duplicate)] {
            let error = import_bytes(input.as_bytes().to_vec()).unwrap_err();
            assert_eq!(error.kind(), yonik_import::FatalError::InvalidJson);
            assert_eq!(error.input(), input.as_bytes());
        }
    }
}
