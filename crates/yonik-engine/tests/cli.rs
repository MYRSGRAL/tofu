use std::{
    io::Write,
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
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
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
    ] {
        let output = run(&input, &args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(!error.contains("secret.example"));
        assert!(!error.contains("00112233"));
    }
}
