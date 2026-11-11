#![cfg(all(target_os = "linux", feature = "test-support"))]
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
use yonik_desktop::HostConfig;

#[test]
fn cleanup_fault_requires_explicit_ephemeral_key() {
    let config = || {
        HostConfig::new(
            PathBuf::from("unused-data"),
            PathBuf::from("unused-engines"),
        )
    };
    let fault = Arc::new(AtomicBool::new(true));
    assert!(config().with_cleanup_fault(Arc::clone(&fault)).is_err());
    assert!(
        config()
            .with_ephemeral_key([91; 32])
            .with_cleanup_fault(fault)
            .is_ok()
    );
}
