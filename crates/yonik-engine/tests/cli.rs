use std::{
    io::{ErrorKind, Write},
    process::{Command, Stdio},
};

fn run(input: &str, args: &[&str]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_yonik-compile"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Err(error) = child.stdin.take().unwrap().write_all(input.as_bytes()) {
        // Отказ по аргументам может закрыть stdin до записи родителя.
        assert_eq!(error.kind(), ErrorKind::BrokenPipe);
    }
    child.wait_with_output().unwrap()
}

const URI: &str = "vless://00112233-4455-6677-8899-aabbccddeeff@secret.example:443?security=tls";

#[test]
fn default_output_is_metadata_and_sensitive_export_requires_flag() {
    let output = run(URI, &["xray"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("compiled_not_checked"));
    assert!(!text.contains("secret.example"));
    assert!(!text.contains("00112233"));
    let secret = run(URI, &["sing-box", "--emit-secret-config"]);
    assert!(secret.status.success());
    assert!(
        String::from_utf8(secret.stdout)
            .unwrap()
            .contains("secret.example")
    );
}

#[test]
fn partial_native_and_bad_arguments_are_rejected_without_secrets() {
    for (input, args) in [
        (format!("{URI}\nbad://secret"), vec!["xray"]),
        ("{\"secret\":true}".into(), vec!["xray"]),
        (URI.into(), vec!["xray", "--server", "2"]),
        (URI.into(), vec!["xray", "--listen-port", "0"]),
        (URI.into(), vec!["xray", "--secret.example"]),
        (URI.repeat(4096), vec!["xray", "--secret.example"]),
    ] {
        let output = run(&input, &args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(!error.contains("secret.example"));
        assert!(!error.contains("00112233"));
    }
}

#[test]
fn system_tun_cli_requires_complete_typed_settings_and_keeps_summary_safe() {
    let fields = [
        "--tun-id",
        "1",
        "--tun-mtu",
        "1500",
        "--routing-mark",
        "99",
        "--tun-address",
        "198.18.0.1/30",
        "--tun-address",
        "2001:db8:746f::1/126",
    ];
    for engine in ["xray", "sing-box"] {
        let mut args = vec![engine];
        args.extend(fields);
        let summary = run(URI, &args);
        assert!(summary.status.success());
        let summary: serde_json::Value = serde_json::from_slice(&summary.stdout).unwrap();
        assert_eq!(summary["system_tun"], true);
        assert!(!summary.to_string().contains("secret.example"));
        args.push("--emit-secret-config");
        let emitted = run(URI, &args);
        assert!(emitted.status.success());
        let value: serde_json::Value = serde_json::from_slice(&emitted.stdout).unwrap();
        assert_eq!(value["inbounds"][1]["tag"], "system-tun");
    }
    for fields in [
        vec!["xray", "--tun-id", "1"],
        vec!["xray", "--tun-mtu", "1500"],
        vec!["xray", "--routing-mark", "1"],
        vec!["xray", "--tun-address", "127.0.0.1/8"],
    ] {
        let output = run(URI, &fields);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
