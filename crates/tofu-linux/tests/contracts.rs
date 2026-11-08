#![cfg(target_os = "linux")]

use std::{path::Path, time::Duration};
use tofu_engine::Engine;
use tofu_linux::{BinarySnapshot, Cancellation, HostError, ProbeTarget, Sha256};

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
