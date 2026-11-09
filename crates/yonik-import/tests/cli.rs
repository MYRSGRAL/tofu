use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn stdin_summary_and_errors_do_not_reveal_credentials() {
    let mut process = Command::new(env!("CARGO_BIN_EXE_yonik-import"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("{e:?}"));
    let mut input = process.stdin.take().unwrap_or_else(|| panic!("Нет stdin"));
    input
        .write_all(b"trojan://CANARY_SECRET@example.test:443\nSECRET_SCHEME://bad\n")
        .unwrap_or_else(|e| panic!("{e:?}"));
    drop(input);
    let output = process
        .wait_with_output()
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap_or_else(|e| panic!("{e:?}"));
    let report: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(report["parsed"], 1);
    assert_eq!(report["rejected"], 1);
    assert!(!text.contains("CANARY"));
    assert!(!text.contains("SECRET_SCHEME"));
    assert!(output.stderr.is_empty());
}

#[test]
fn argument_error_does_not_echo_secret_argument() {
    let output = Command::new(env!("CARGO_BIN_EXE_yonik-import"))
        .arg("CANARY_PRIVATE_URL")
        .output()
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("CANARY"));
}
