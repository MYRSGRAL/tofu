#![cfg(target_os = "linux")]

use std::{path::Path, time::Duration};
use yonik_engine::Engine;
use yonik_linux::{BinarySnapshot, Cancellation, HostError, ProbeResult, ProbeTarget, Sha256};

#[test]
fn probe_success_requires_the_configured_status() {
    let response = |status| ProbeResult::Response {
        status,
        elapsed: Duration::from_millis(1),
    };
    let target = ProbeTarget::new(
        "https://example.invalid/private?secret=token",
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(target.accepts(response(204)));
    assert!(!target.accepts(response(200)));
    assert!(!response(200).passed());
    let target = target.with_expected_status(200).unwrap();
    assert!(target.accepts(response(200)));
    assert!(!target.accepts(response(204)));
    for result in [
        response(302),
        response(503),
        ProbeResult::Timeout,
        ProbeResult::Tls,
        ProbeResult::Transport,
    ] {
        assert!(!target.accepts(result));
    }
    for status in [0, 199, 300, 302, 600, u16::MAX] {
        assert!(
            ProbeTarget::new("https://example.invalid/", Duration::from_secs(5))
                .unwrap()
                .with_expected_status(status)
                .is_err()
        );
    }
    assert!(!format!("{target:?}").contains("secret"));
}

#[test]
fn hash_and_probe_url_validate_without_disclosing_input() {
    assert!("00".parse::<Sha256>().is_err());
    for url in [
        "http://secret.test/",
        "https://user:password@secret.test/",
        "https://secret.test/#token",
        "https:///",
        "https://secret.test/\n",
    ] {
        assert!(ProbeTarget::new(url, Duration::from_secs(2)).is_err());
    }
    let target = ProbeTarget::new(
        "https://secret.test/private?token=secret",
        Duration::from_secs(2),
    )
    .unwrap();
    assert!(!format!("{target:?}").contains("secret"));
    assert!(ProbeTarget::new("https://test/", Duration::ZERO).is_err());
}

#[test]
fn nonregular_and_wrong_hash_binary_cannot_become_snapshot() {
    let hash = "00".repeat(32).parse::<Sha256>().unwrap();
    assert_eq!(
        BinarySnapshot::load(Path::new("/dev/zero"), hash, Engine::Xray).err(),
        Some(HostError::InvalidBinary)
    );
    assert_eq!(
        BinarySnapshot::load(Path::new("/usr/bin/true"), hash, Engine::Xray).err(),
        Some(HostError::BinaryHash)
    );
}

#[tokio::test]
async fn already_cancelled_validation_does_not_spawn_process() {
    let (source, cancel) = Cancellation::new();
    source.send(true).unwrap();
    let result = cancel.check();
    assert_eq!(result, Err(HostError::Cancelled));
}
