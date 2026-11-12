//! Детерминированная кампания public API; seed и индекс воспроизводят вход.
use base64::{Engine, engine::general_purpose::STANDARD};
use std::io::Write;
use yonik_import::{
    FatalError, MAX_INPUT_BYTES, MAX_JSON_DEPTH, MAX_RECORDS, ServerIntent, import_bytes,
};

const CANARY: &str = "SYNTHETIC_SECRET_9214";
const CORPUS: [&str; 3] = [
    include_str!("corpus/p04-seeded/uri.txt"),
    include_str!("corpus/p04-seeded/xray.json"),
    include_str!("corpus/p04-seeded/sing-box.json"),
];

fn закрыто(text: &str) -> bool {
    !text.contains(CANARY)
}

fn проверить(bytes: Vec<u8>) {
    match import_bytes(bytes.clone()) {
        Ok(report) => {
            assert_eq!(report.original().expose().as_bytes(), bytes);
            assert!(закрыто(&format!("{:?}", report.issues)));
            for server in &report.servers {
                if !server.review_flags().is_empty() {
                    assert!(ServerIntent::from_server(server).is_err());
                }
            }
        }
        Err(error) => {
            assert_eq!(error.input(), bytes);
            assert!(error.input_is_complete());
            assert!(закрыто(&format!("{error:?} {error}")));
        }
    }
}

#[test]
fn отрицательный_контроль_редакции() {
    assert!(!закрыто(&format!("error: {CANARY}")));
    assert!(закрыто("InvalidJson"));
}

#[test]
fn границы_и_соседние_значения() {
    for count in [MAX_RECORDS - 1, MAX_RECORDS, MAX_RECORDS + 1] {
        let input = "socks5://127.0.0.1:1080\n".repeat(count);
        let result = import_bytes(input.as_bytes().to_vec());
        if count <= MAX_RECORDS {
            assert_eq!(result.unwrap().servers.len(), count);
        } else {
            assert_eq!(result.unwrap_err().kind(), FatalError::TooManyRecords);
        }
    }
    for depth in [MAX_JSON_DEPTH - 1, MAX_JSON_DEPTH, MAX_JSON_DEPTH + 1] {
        let input = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        let result = import_bytes(input.into_bytes());
        if depth <= MAX_JSON_DEPTH {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err().kind(), FatalError::JsonTooDeep);
        }
    }
    for size in [MAX_INPUT_BYTES - 1, MAX_INPUT_BYTES, MAX_INPUT_BYTES + 1] {
        let input = format!("{}{{}}", " ".repeat(size - 2));
        let result = import_bytes(input.into_bytes());
        if size <= MAX_INPUT_BYTES {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err().kind(), FatalError::TooLarge);
        }
    }
}

#[test]
fn cidr_не_превращается_в_endpoint_и_неизвестное_не_теряется() {
    for host in [
        "192.0.2.1/24",
        "[2001:db8::1]/64",
        "256.1.1.1",
        "[2001:db8:::1]",
    ] {
        let input = format!("trojan://{CANARY}@{host}:443");
        let report = import_bytes(input.as_bytes().to_vec()).unwrap();
        assert_eq!(report.original().expose(), input);
        assert!(report.servers.is_empty());
    }
    for source in &CORPUS[1..] {
        for nested in ["null", "true", "[]", "{\"x\":[{\"y\":1}]}"] {
            let input = source.replace("[1,2]", nested).replace("CANARY", CANARY);
            let report = import_bytes(input.as_bytes().to_vec()).unwrap();
            assert_eq!(report.native_json.as_ref().unwrap().expose(), input);
            assert_eq!(report.servers.len(), 1);
            assert!(ServerIntent::from_server(&report.servers[0]).is_err());
            let duplicate = input.replace("\"nested\":", "\"nested\":0,\"ne\\u0073ted\":");
            assert_eq!(
                import_bytes(duplicate.into_bytes()).unwrap_err().kind(),
                FatalError::InvalidJson
            );
        }
    }
}

#[test]
fn linux_mutation_campaign() {
    let mut corpus =
        std::env::var_os("YONIK_ROBUST_CORPUS").map(|path| std::fs::File::create(path).unwrap());
    let before = std::fs::read_dir("/proc/self/fd").unwrap().count();
    let mut state = 0x9214_2026_1112_u64;
    for source in CORPUS {
        let source = source.replace("CANARY", CANARY);
        for index in 0..2048 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let mut bytes = source.as_bytes().to_vec();
            let at = state as usize % bytes.len();
            match index % 4 {
                0 => bytes.truncate(at),
                1 => bytes[at] = (state >> 32) as u8,
                2 => {
                    bytes.remove(at);
                }
                _ => bytes.insert(at, b"{}[]%\"\\\0="[index / 4 % b"{}[]%\"\\\0=".len()]),
            }
            for input in [bytes.clone(), STANDARD.encode(bytes).into_bytes()] {
                if let Some(file) = &mut corpus {
                    file.write_all(&(input.len() as u32).to_le_bytes()).unwrap();
                    file.write_all(&input).unwrap();
                }
                проверить(input);
            }
        }
    }
    // Запускать isolated test binary с --test-threads=1: посторонние тесты не открывают FD.
    assert!(std::fs::read_dir("/proc/self/fd").unwrap().count() <= before);
}

#[test]
fn ip_idna_и_граница_длины_uri() {
    for (host, canonical) in [
        ("пример.рф", "xn--e1afmkfd.xn--p1ai"),
        ("bücher.example", "xn--bcher-kva.example"),
        ("192.0.2.255", "192.0.2.255"),
        ("[2001:0db8:0:0:0:0:0:1]", "2001:db8::1"),
    ] {
        for port in [1, 1080, 65535] {
            let uri = format!("socks5://{host}:{port}");
            for input in [uri.clone(), STANDARD.encode(&uri)] {
                let report = import_bytes(input.into_bytes()).unwrap();
                assert!(report.issues.is_empty());
                assert_eq!(report.servers.len(), 1);
                assert_eq!(report.servers[0].endpoint().host(), canonical);
                assert_eq!(report.servers[0].endpoint().port(), port);
            }
        }
    }
    let prefix = "socks5://127.0.0.1:1080#";
    for size in [
        yonik_import::MAX_ENTRY_BYTES - 1,
        yonik_import::MAX_ENTRY_BYTES,
        yonik_import::MAX_ENTRY_BYTES + 1,
    ] {
        let input = format!("{prefix}{}", "x".repeat(size - prefix.len()));
        let report = import_bytes(input.into_bytes()).unwrap();
        assert_eq!(
            report.servers.len(),
            usize::from(size <= yonik_import::MAX_ENTRY_BYTES)
        );
    }
    for input in [
        "A".repeat(MAX_INPUT_BYTES + 1),
        format!("{{\"password\":\"{CANARY}\""),
        format!("trojan://{CANARY}@example.test:443?unknown=%00"),
    ] {
        проверить(input.into_bytes());
    }
}
