use std::io::{self, Read};
use yonik_import::{
    FatalError, MAX_INPUT_BYTES, MAX_RECORDS, Protocol, import_bytes, import_reader,
};

const UUID: &str = "00000000-0000-0000-0000-000000000001";

fn parse(text: &str) -> yonik_import::ImportReport {
    import_bytes(text.as_bytes().to_vec()).unwrap_or_else(|e| panic!("{e:?}"))
}

#[test]
fn vless_normalizes_endpoint_and_keeps_opaque_source() {
    let text = format!(
        "vless://{UUID}@EXAMPLE.test:443?security=reality&type=tcp&pbk=CANARY_SECRET#private-label"
    );
    let report = parse(&text);
    assert_eq!(report.servers.len(), 1);
    let server = &report.servers[0];
    assert_eq!(server.protocol(), Protocol::Vless);
    assert_eq!(server.endpoint().host(), "example.test");
    assert_eq!(server.endpoint().port(), 443);
    assert_eq!(server.source().expose(), text);
    assert!(!format!("{report:?}").contains("CANARY_SECRET"));
    assert!(!format!("{report:?}").contains("private-label"));
}

#[test]
fn partial_import_reports_indices_without_echoing_input() {
    let text = format!(
        "\nvless://{UUID}@example.test:443\nSECRET_CANARY://bad\nvless://bad-id@host.test:0\n"
    );
    let report = parse(&text);
    assert_eq!(report.servers.len(), 1);
    assert_eq!(report.issues.len(), 2);
    assert_eq!(report.issues[0].line, 3);
    assert_eq!(report.issues[1].line, 4);
    assert!(!format!("{report:?}").contains("SECRET_CANARY"));
    assert_eq!(report.original().expose(), text);
}

#[test]
fn standard_base64_one_layer_and_uri_list() {
    let report = parse("c29ja3M1Oi8vMTI3LjAuMC4xOjEwODA=");
    assert!(report.base64_envelope);
    assert_eq!(report.servers[0].protocol(), Protocol::Socks5);
    // Вторая обёртка не угадывается рекурсивно.
    assert!(import_bytes(b"YzI5amEzTTFPaTh2TVRJM0xqQXVNQzR4T2pFd09EQT0=".to_vec()).is_err());
}

#[test]
fn all_initial_protocols_parse_core_credentials() {
    let text = format!(
        "vless://{UUID}@[::1]:443\ntrojan://pass%3Aword@example.test:443\nss://YWVzLTEyOC1nY206cGFzc3dvcmQ=@example.test:8388\nhttp://user:pass@example.test:8080\nsocks5://user:pass@127.0.0.1:1080\nvmess://eyJhZGQiOiJleGFtcGxlLnRlc3QiLCJwb3J0IjoiNDQzIiwiaWQiOiIwMDAwMDAwMC0wMDAwLTAwMDAtMDAwMC0wMDAwMDAwMDAwMDEifQ=="
    );
    let report = parse(&text);
    assert!(report.issues.is_empty(), "{:?}", report.issues);
    let protocols: Vec<_> = report.servers.iter().map(|s| s.protocol()).collect();
    assert_eq!(
        protocols,
        [
            Protocol::Vless,
            Protocol::Trojan,
            Protocol::Shadowsocks,
            Protocol::HttpConnect,
            Protocol::Socks5,
            Protocol::Vmess
        ]
    );
}

#[test]
fn unknown_and_unsafe_parameters_are_retained_and_require_review() {
    let text = format!("vless://{UUID}@host.test:443?CANARY_KEY=CANARY_VALUE&allowInsecure=1");
    let report = parse(&text);
    assert!(
        report.servers[0]
            .review_flags()
            .contains(&yonik_import::ReviewFlag::UnknownParameter)
    );
    assert!(
        report.servers[0]
            .review_flags()
            .contains(&yonik_import::ReviewFlag::UnsafeOption)
    );
    assert!(report.servers[0].source().expose().contains("CANARY_VALUE"));
    assert!(!format!("{report:?}").contains("CANARY"));
    let duplicate = format!("vless://{UUID}@host.test:443?type=tcp&type=ws");
    assert_eq!(parse(&duplicate).issues.len(), 1);
}

#[test]
fn json_is_opaque_and_depth_limited() {
    let report = parse(r#"{"outbounds":[{"password":"CANARY"}]}"#);
    assert!(report.native_json.is_some());
    assert!(report.servers.is_empty());
    assert!(!format!("{report:?}").contains("CANARY"));
    assert!(import_bytes(format!("{}0{}", "[".repeat(65), "]".repeat(65)).into_bytes()).is_err());
    assert!(import_bytes(br#"{"broken":}"#.to_vec()).is_err());
}

#[test]
fn bounded_read_stops_before_consuming_unbounded_source() {
    struct Endless {
        read: usize,
    }
    impl Read for Endless {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.read += buffer.len();
            buffer.fill(b'a');
            Ok(buffer.len())
        }
    }
    let mut reader = Endless { read: 0 };
    assert_eq!(
        import_reader(&mut reader).err().map(|e| e.kind()),
        Some(FatalError::TooLarge)
    );
    assert_eq!(reader.read, MAX_INPUT_BYTES + 1);
    let text = "socks5://127.0.0.1:1080\n".repeat(MAX_RECORDS + 1);
    assert_eq!(
        import_bytes(text.into_bytes()).err().map(|e| e.kind()),
        Some(FatalError::TooManyRecords)
    );
}
