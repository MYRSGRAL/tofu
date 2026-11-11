//! Границы нормализованного DNS label без сетевого lookup.
use yonik_import::{Field, IssueCode, import_bytes};

fn check_label(label: &str, ascii_octets: usize) {
    let report = import_bytes(format!("trojan://CANARY@{label}.test:443").into_bytes()).unwrap();
    if ascii_octets <= 63 {
        assert_eq!(report.servers.len(), 1, "label длины {ascii_octets}");
        assert_eq!(
            report.servers[0]
                .endpoint()
                .host()
                .split('.')
                .next()
                .unwrap()
                .len(),
            ascii_octets
        );
        assert!(report.issues.is_empty());
    } else {
        assert!(
            report.servers.is_empty(),
            "label длины {ascii_octets} не должен становиться сервером"
        );
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].code, IssueCode::InvalidHost);
        assert_eq!(report.issues[0].field, Field::Endpoint);
    }
}

#[test]
fn ascii_label_63_64_65_octets() {
    for width in [63, 64, 65] {
        check_label(&"a".repeat(width), width);
    }
}

#[test]
fn idna_label_counts_punycode_octets() {
    // é × 57 кодируется в xn--9c + 57 букв a: 63 ASCII октета.
    for (characters, octets) in [(57, 63), (58, 64), (59, 65)] {
        let punycode = format!("xn--9c{}", "a".repeat(characters));
        assert_eq!(punycode.len(), octets);
        check_label(&punycode, octets);
        check_label(&"é".repeat(characters), octets);
    }
}

#[test]
fn preserves_ip_underscore_idna_and_opaque_native() {
    for host in [
        "127.0.0.1",
        "[2001:db8::1]",
        "_srv.test",
        "bücher.test",
        "fixture.test.",
    ] {
        let report = import_bytes(format!("trojan://CANARY@{host}:443").into_bytes()).unwrap();
        assert_eq!(report.servers.len(), 1, "совместимый endpoint");
        assert!(report.issues.is_empty());
    }
    let opaque = format!(
        r#"{{"outbounds":[{{"type":"unknown","server":"{}.test"}}]}}"#,
        "a".repeat(65)
    );
    let report = import_bytes(opaque.clone().into_bytes()).unwrap();
    assert!(report.servers.is_empty());
    assert_eq!(
        report.issues[0].code,
        yonik_import::IssueCode::UnsupportedProtocol
    );
    assert_eq!(report.native_json.unwrap().expose(), opaque);
}
