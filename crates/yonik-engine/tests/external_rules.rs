use yonik_engine::external_rules::{Format, Limits, compile_records};

#[test]
fn bounded_native_artifacts() {
    let source = b"exact:example.org\nsuffix:example.net\n";
    let mut dat = Vec::new();
    compile_records(
        std::io::Cursor::new(source),
        &mut dat,
        Format::XrayDomains,
        2,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(dat, b"\x0a\x29\x0a\x05YONIK\x12\x0f\x08\x03\x12\x0bexample.org\x12\x0f\x08\x02\x12\x0bexample.net");
    assert!(
        compile_records(
            std::io::Cursor::new(source),
            Vec::new(),
            Format::XrayDomains,
            1,
            Limits::default()
        )
        .is_err()
    );
    assert!(
        compile_records(
            std::io::Cursor::new(b"10.0.0.1/24\n"),
            Vec::new(),
            Format::XrayCidrs,
            1,
            Limits::default()
        )
        .is_err()
    );
}

#[test]
fn mandatory_150k_stream() {
    let (domains, cidrs) = synthetic();
    for (source, count, formats) in [
        (
            &domains,
            100_000,
            [Format::XrayDomains, Format::SingDomains],
        ),
        (&cidrs, 50_000, [Format::XrayCidrs, Format::SingCidrs]),
    ] {
        for format in formats {
            let mut out = Vec::new();
            compile_records(
                std::io::Cursor::new(source.as_bytes()),
                &mut out,
                format,
                count,
                Limits::default(),
            )
            .unwrap();
            assert!(out.len() < 32 * 1024 * 1024);
            assert!(!out.is_empty());
        }
    }
}

/// Локальная проверка закреплённых tools; синтетические имена не являются private inputs.
#[test]
#[ignore = "Требует закреплённые локальные ядра"]
fn pinned_native_150k() {
    use std::{fs, process::Command};
    let dir = std::env::temp_dir().join(format!("yonik-native-rules-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let (domains, cidrs) = synthetic();
    for (name, source, count, format) in [
        ("domains.dat", &domains, 100_000, Format::XrayDomains),
        ("cidrs.dat", &cidrs, 50_000, Format::XrayCidrs),
        ("domains.json", &domains, 100_000, Format::SingDomains),
        ("cidrs.json", &cidrs, 50_000, Format::SingCidrs),
    ] {
        let file = fs::File::create(dir.join(name)).unwrap();
        let bytes = compile_records(
            std::io::Cursor::new(source.as_bytes()),
            file,
            format,
            count,
            Limits::default(),
        )
        .unwrap();
        eprintln!("{name}: records={count} bytes={bytes}");
    }
    let xray = "/home/myrs/projects/yonik/local/engines/xray-26.10.31/xray";
    let sing = "/home/myrs/projects/yonik/local/engines/sing-box-1.14.4/sing-box";
    let config = serde_json::json!({"log":{"loglevel":"none"},"outbounds":[{"tag":"direct","protocol":"freedom"}],"routing":{"rules":[{"type":"field","domain":["ext:domains.dat:yonik"],"outboundTag":"direct"},{"type":"field","ip":["ext:cidrs.dat:yonik"],"outboundTag":"direct"}]}});
    fs::write(dir.join("xray.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    let check = Command::new(xray)
        .args(["run", "-test", "-config"])
        .arg(dir.join("xray.json"))
        .env("XRAY_LOCATION_ASSET", &dir)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    for name in ["domains", "cidrs"] {
        let compile = Command::new(sing)
            .args(["rule-set", "compile", "--output"])
            .arg(dir.join(format!("{name}.srs")))
            .arg(dir.join(format!("{name}.json")))
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
    }
    let config = serde_json::json!({"outbounds":[{"type":"direct","tag":"direct"}],"route":{"rule_set":[{"type":"local","tag":"domains","format":"binary","path":dir.join("domains.srs")},{"type":"local","tag":"cidrs","format":"binary","path":dir.join("cidrs.srs")}],"rules":[{"rule_set":["domains","cidrs"],"outbound":"direct"}]}});
    fs::write(dir.join("sing.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    let check = Command::new(sing)
        .args(["check", "-c"])
        .arg(dir.join("sing.json"))
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    // Повреждённые ресурсы обязаны отвергаться обоими native loaders.
    fs::write(dir.join("domains.dat"), [0xff, 0xff]).unwrap();
    let rejected = Command::new(xray)
        .args(["run", "-test", "-config"])
        .arg(dir.join("xray.json"))
        .env("XRAY_LOCATION_ASSET", &dir)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    fs::write(dir.join("domains.srs"), [0xff, 0xff]).unwrap();
    let rejected = Command::new(sing)
        .args(["check", "-c"])
        .arg(dir.join("sing.json"))
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    for entry in fs::read_dir(&dir).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    fs::remove_dir(dir).unwrap();
}

#[test]
fn refusal_preserves_destination_before_any_write() {
    let mut previous = b"previous".to_vec();
    for source in [
        "exact:UPPER.example\n",
        "suffix:bad..name\n",
        "exact:example.org",
        "regex:example.org\n",
    ] {
        assert!(
            compile_records(
                std::io::Cursor::new(source),
                &mut previous,
                Format::XrayDomains,
                1,
                Limits::default()
            )
            .is_err()
        );
        assert_eq!(previous, b"previous");
    }
    assert!(
        compile_records(
            std::io::Cursor::new("exact:example.org\n"),
            &mut previous,
            Format::SingDomains,
            1,
            Limits {
                artifact_bytes: 1,
                ..Limits::default()
            }
        )
        .is_err()
    );
    assert_eq!(previous, b"previous");
}

fn synthetic() -> (String, String) {
    use std::fmt::Write;
    let mut domains = String::new();
    let mut cidrs = String::new();
    for i in 0..100_000 {
        writeln!(&mut domains, "exact:d{i}.example.org").unwrap();
    }
    for i in 0..50_000 {
        writeln!(&mut cidrs, "10.0.{}.{}/32", i / 256, i % 256).unwrap();
    }
    (domains, cidrs)
}
