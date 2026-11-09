use std::io::{self, Read};
use yonik_import::{FatalError, MAX_ENTRY_BYTES, MAX_INPUT_BYTES, import_bytes, import_reader};

#[test]
fn hostile_reader_errors_are_redacted_and_invalid_utf8_is_rejected() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("CANARY_READER_URL"))
        }
    }
    let error = import_reader(Broken)
        .err()
        .unwrap_or_else(|| panic!("Ожидался отказ"));
    assert_eq!(error.kind(), FatalError::Io);
    assert!(!format!("{error:?} {error}").contains("CANARY"));
    assert_eq!(
        import_bytes(vec![0xff]).err().map(|e| e.kind()),
        Some(FatalError::InvalidUtf8)
    );
    assert_eq!(
        import_bytes(vec![b'a'; MAX_INPUT_BYTES + 1])
            .err()
            .map(|e| e.kind()),
        Some(FatalError::TooLarge)
    );
}

#[test]
fn oversized_uri_and_control_credentials_do_not_become_profiles() {
    for text in [
        format!("trojan://{}@host.test:443", "x".repeat(MAX_ENTRY_BYTES)),
        "trojan://pass%00word@host.test:443".to_owned(),
        "socks5://host.test:65536".to_owned(),
        "socks5://host.test:0".to_owned(),
    ] {
        let report = import_bytes(text.into_bytes()).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(report.servers.is_empty());
        assert_eq!(report.issues.len(), 1);
    }
}

#[test]
fn vmess_duplicate_or_mistyped_core_fields_are_rejected() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    for json in [
        r#"{"add":"a.test","add":"b.test","port":443,"id":"00000000-0000-0000-0000-000000000001"}"#,
        r#"{"add":"a.test","port":443,"id":"00000000-0000-0000-0000-000000000001","net":12}"#,
    ] {
        let report = import_bytes(format!("vmess://{}", STANDARD.encode(json)).into_bytes())
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(report.servers.is_empty());
        assert_eq!(report.issues.len(), 1);
    }
}

#[test]
fn aead_2022_userinfo_must_not_use_base64_wrapper() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let body = STANDARD.encode("2022-blake3-aes-256-gcm:c29tZS1rZXk=");
    let report = import_bytes(format!("ss://{body}@example.test:8388").into_bytes())
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(report.servers.is_empty());
    assert_eq!(report.issues.len(), 1);
}

#[test]
fn exact_contract_limits_accept_valid_input() {
    use yonik_import::{MAX_JSON_DEPTH, MAX_RECORDS};
    let report = import_bytes("socks5://127.0.0.1:1080\n".repeat(MAX_RECORDS).into_bytes())
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(report.servers.len(), MAX_RECORDS);
    let json = format!(
        "{}0{}",
        "[".repeat(MAX_JSON_DEPTH),
        "]".repeat(MAX_JSON_DEPTH)
    );
    assert!(
        import_bytes(json.into_bytes())
            .unwrap_or_else(|e| panic!("{e:?}"))
            .native_json
            .is_some()
    );
    let body = format!("{}{{}}", " ".repeat(MAX_INPUT_BYTES - 2));
    assert!(
        import_bytes(body.into_bytes())
            .unwrap_or_else(|e| panic!("{e:?}"))
            .native_json
            .is_some()
    );
}
