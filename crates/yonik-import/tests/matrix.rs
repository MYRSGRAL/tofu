use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use yonik_import::{
    FatalError, Field, ImportReport, IssueCode, MAX_ENTRY_BYTES, MAX_INPUT_BYTES, MAX_JSON_DEPTH,
    MAX_RECORDS, ReviewFlag, import_bytes,
};

fn parse(text: &str) -> ImportReport {
    import_bytes(text.as_bytes().to_vec()).unwrap_or_else(|e| panic!("{e:?}"))
}

fn wrapped(text: &str, envelope: bool) -> String {
    if envelope {
        STANDARD.encode(text)
    } else {
        text.to_owned()
    }
}

#[test]
fn граница_размера_uri_в_обычном_и_base64_списке() {
    for envelope in [false, true] {
        for size in [MAX_ENTRY_BYTES - 1, MAX_ENTRY_BYTES, MAX_ENTRY_BYTES + 1] {
            let prefix = "trojan://пароль@host.test:443?opaque=";
            let uri = format!("{prefix}{}", "x".repeat(size - prefix.len()));
            let text = wrapped(&format!("{uri}\nsocks5://127.0.0.1\n"), envelope);
            let report = parse(&text);
            assert_eq!(report.original().expose(), text);
            assert_eq!(report.base64_envelope, envelope);
            if size <= MAX_ENTRY_BYTES {
                assert_eq!(report.servers.len(), 2);
                assert!(report.issues.is_empty());
                assert_eq!(report.servers[0].source().expose(), uri);
            } else {
                assert_eq!(report.servers.len(), 1);
                assert_eq!(report.issues.len(), 1);
                assert_eq!(report.issues[0].line, 1);
                assert_eq!(report.issues[0].code, IssueCode::TooLarge);
                assert_eq!(report.issues[0].field, Field::Source);
            }
        }
    }
}

#[test]
fn граница_входа_измеряется_до_декодирования() {
    for envelope in [false, true] {
        for size in [MAX_INPUT_BYTES - 1, MAX_INPUT_BYTES, MAX_INPUT_BYTES + 1] {
            let mut text = if envelope {
                STANDARD.encode("{}")
            } else {
                "{}".to_owned()
            };
            text.push_str(&" ".repeat(size - text.len()));
            match import_bytes(text.as_bytes().to_vec()) {
                Ok(report) => {
                    assert!(size <= MAX_INPUT_BYTES);
                    assert!(report.native_json.is_some());
                    assert_eq!(report.base64_envelope, envelope);
                    assert_eq!(report.original().expose(), text);
                }
                Err(error) => {
                    assert_eq!(size, MAX_INPUT_BYTES + 1);
                    assert_eq!(error.kind(), FatalError::TooLarge);
                    assert!(error.input_is_complete());
                    assert_eq!(error.input(), text.as_bytes());
                }
            }
        }
    }
}

#[test]
fn непустые_отклонённые_записи_тоже_расходуют_лимит() {
    for envelope in [false, true] {
        for count in [MAX_RECORDS - 1, MAX_RECORDS, MAX_RECORDS + 1] {
            let text = wrapped(
                &format!("\n{}", "unknown://secret\n\n".repeat(count)),
                envelope,
            );
            match import_bytes(text.as_bytes().to_vec()) {
                Ok(report) => {
                    assert!(count <= MAX_RECORDS);
                    assert!(report.servers.is_empty());
                    assert_eq!(report.issues.len(), count);
                    assert_eq!(report.issues[count - 1].line, count * 2);
                    assert!(
                        report
                            .issues
                            .iter()
                            .all(|i| i.code == IssueCode::UnsupportedProtocol)
                    );
                }
                Err(error) => {
                    assert_eq!(count, MAX_RECORDS + 1);
                    assert_eq!(error.kind(), FatalError::TooManyRecords);
                    assert_eq!(error.input(), text.as_bytes());
                }
            }
        }
    }
}

#[test]
fn вложенность_считает_контейнеры_но_не_экранированные_строки() {
    for envelope in [false, true] {
        for depth in [MAX_JSON_DEPTH - 1, MAX_JSON_DEPTH, MAX_JSON_DEPTH + 1] {
            // Чередуем массивы и объекты, внутри строки — скобки, кавычка и backslash.
            let mut json = serde_json::to_string("[{}]\\\"").unwrap_or_else(|e| panic!("{e}"));
            for level in 0..depth {
                json = if level % 2 == 0 {
                    format!("[{json}]")
                } else {
                    format!("{{\"x\":{json}}}")
                };
            }
            let text = wrapped(&json, envelope);
            match import_bytes(text.as_bytes().to_vec()) {
                Ok(report) => {
                    assert!(depth <= MAX_JSON_DEPTH);
                    assert_eq!(
                        report.native_json.as_ref().map(|v| v.expose()),
                        Some(json.as_str())
                    );
                }
                Err(error) => {
                    assert_eq!(depth, MAX_JSON_DEPTH + 1);
                    assert_eq!(error.kind(), FatalError::JsonTooDeep);
                    assert_eq!(error.input(), text.as_bytes());
                }
            }
        }
    }
}

#[test]
fn алфавиты_base64_и_отказ_от_повторной_обёртки() {
    for (suffix, padding) in [("", 0), ("a", 2), ("ab", 1)] {
        let text = format!("trojan://пароль@пример.рф:443?opaque=\u{ffff}{suffix}");
        assert_eq!((3 - text.len() % 3) % 3, padding);
        for (engine, padded) in [
            (STANDARD, true),
            (STANDARD_NO_PAD, false),
            (URL_SAFE, true),
            (URL_SAFE_NO_PAD, false),
        ] {
            let encoded = engine.encode(&text);
            let expected_padding = if padded { padding } else { 0 };
            assert_eq!(
                encoded
                    .bytes()
                    .rev()
                    .take_while(|byte| *byte == b'=')
                    .count(),
                expected_padding
            );
            let report = parse(&encoded);
            assert!(report.base64_envelope);
            assert!(report.issues.is_empty());
            assert_eq!(report.servers.len(), 1);
            assert_eq!(report.servers[0].source().expose(), text);
            let spaced = encoded
                .as_bytes()
                .chunks(7)
                .map(|s| std::str::from_utf8(s).unwrap_or_else(|e| panic!("{e}")))
                .collect::<Vec<_>>()
                .join(" \r\n\t");
            assert_eq!(parse(&spaced).servers.len(), 1);
            let error = import_bytes(engine.encode(&encoded).into_bytes())
                .err()
                .unwrap_or_else(|| panic!("Ожидался отказ"));
            assert_eq!(error.kind(), FatalError::UnsupportedFormat);
        }
    }
    for (text, kind) in [
        ("A", FatalError::InvalidEncoding),
        ("Zg===", FatalError::InvalidEncoding),
        ("Zh==", FatalError::InvalidEncoding),
        ("/w==", FatalError::InvalidUtf8),
    ] {
        let error = import_bytes(text.as_bytes().to_vec())
            .err()
            .unwrap_or_else(|| panic!("Ожидался отказ"));
        assert_eq!(error.kind(), kind);
        assert_eq!(error.input(), text.as_bytes());
    }
}

#[test]
fn idna_и_ip_нормализуются_одинаково_в_uri_и_vmess() {
    for (address, expected) in [
        ("ПРИМЕР.РФ", "xn--e1afmkfd.xn--p1ai"),
        ("xn--e1afmkfd.xn--p1ai", "xn--e1afmkfd.xn--p1ai"),
        ("192.0.2.1", "192.0.2.1"),
        ("2001:0db8:0:0:0:0:0:1", "2001:db8::1"),
        ("::ffff:192.0.2.1", "::ffff:192.0.2.1"),
    ] {
        let uri_host = if address.contains(':') {
            format!("[{address}]")
        } else {
            address.to_owned()
        };
        let json = serde_json::json!({"add": address, "port": 443, "id": "00000000-0000-0000-0000-000000000001"}).to_string();
        for text in [
            format!("trojan://pass@{uri_host}:443"),
            format!("vmess://{}", STANDARD.encode(json)),
        ] {
            let report = parse(&text);
            assert!(report.issues.is_empty());
            assert_eq!(report.servers.len(), 1);
            assert_eq!(report.servers[0].endpoint().host(), expected);
            assert_eq!(report.servers[0].endpoint().port(), 443);
        }
    }
}

#[test]
fn опасные_uri_отклоняются_без_потери_соседней_записи() {
    for bad in [
        "trojan://pass@[fe80::1%25eth0]:443",
        "trojan://pass@[2001:db8:::1]:443",
        "trojan://pass@xn--:443",
        "trojan://pass@host.test:65536",
        "trojan://pass%0Aword@host.test:443",
        "trojan://pass@host.test:443?x=%ff",
        "trojan://pass@host.test:443?x=%",
        "trojan://pass@host.test:443?type=tcp&%74ype=ws",
        "trojan://pass@host.test:443?opaque=1&opaque=2",
    ] {
        for envelope in [false, true] {
            let text = wrapped(&format!("\nsocks5://127.0.0.1\n{bad}\n"), envelope);
            let report = parse(&text);
            assert_eq!(report.servers.len(), 1);
            assert_eq!(report.issues.len(), 1);
            assert_eq!(report.issues[0].line, 3);
            assert_eq!(report.original().expose(), text);
        }
    }
}

#[test]
fn неизвестные_опции_сохраняются_и_требуют_review() {
    for scheme in [
        "vless://00000000-0000-0000-0000-000000000001",
        "trojan://pass",
        "ss://aes-128-gcm:pass",
        "http://user:pass",
        "socks5://user:pass",
    ] {
        let query = "CANARY=%26%3D%2B&%D0%BA%D0%BB%D1%8E%D1%87=value&allowInsecure=1";
        let text = format!("{scheme}@example.test:443?{query}");
        let report = parse(&text);
        assert!(report.issues.is_empty());
        assert_eq!(report.servers.len(), 1);
        let server = &report.servers[0];
        assert_eq!(server.parameters().expose(), query);
        assert_eq!(server.source().expose(), text);
        assert!(
            server
                .review_flags()
                .contains(&ReviewFlag::UnknownParameter)
        );
        assert!(server.review_flags().contains(&ReviewFlag::UnsafeOption));
        assert!(!format!("{report:?}").contains("CANARY"));
    }
    let json = serde_json::json!({"add":"example.test", "port":443, "id":"00000000-0000-0000-0000-000000000001", "CANARY":{"nested":[1,2]}}).to_string();
    let report = parse(&format!("vmess://{}", STANDARD.encode(&json)));
    assert!(report.issues.is_empty());
    assert_eq!(report.servers[0].parameters().expose(), json);
    assert!(
        report.servers[0]
            .review_flags()
            .contains(&ReviewFlag::UnknownParameter)
    );
}
