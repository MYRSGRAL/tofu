#![cfg(target_os = "linux")]
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn partial_selection_requires_acknowledgement_and_explicit_index() {
    let source = b"socks5://user:private-password@127.0.0.1:24443\nunknown://private-token";
    for extra in [
        &[][..],
        &["--accept-partial"][..],
        &["--accept-partial", "--server", "0"][..],
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_yonik-local"))
            .args([
                "xray",
                "--binary",
                "/not-a-yonik-engine",
                "--sha256",
                &"00".repeat(32),
            ])
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(source).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("private"));
        assert!(
            !String::from_utf8(output.stderr)
                .unwrap()
                .contains("private")
        );
        let acknowledged = text
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .any(|event| event["event"] == "partial_import" && event["rejected"] == 1);
        assert_eq!(acknowledged, extra.len() == 3);
    }
}
