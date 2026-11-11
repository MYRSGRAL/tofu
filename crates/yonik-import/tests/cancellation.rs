use std::cell::Cell;
use yonik_import::{CancellableImportError, import_bytes_cancellable};

#[test]
fn cancellation_interrupts_ten_thousand_records() {
    let body = b"trojan://secret@fixture.invalid:443#node\n".repeat(10_000);
    let calls = Cell::new(0);
    let result = import_bytes_cancellable(body, || {
        calls.set(calls.get() + 1);
        calls.get() < 100
    });
    assert!(matches!(result, Err(CancellableImportError::Cancelled)));
    assert_eq!(
        calls.get(),
        100,
        "отмена должна прервать разбор внутри batch"
    );
}

#[test]
fn never_cancelled_matches_compatibility_api() {
    let body = b"trojan://secret@fixture.invalid:443#node\n".repeat(10_000);
    let report = import_bytes_cancellable(body, || true).ok().unwrap();
    assert_eq!(report.servers.len(), 10_000);
}

#[test]
fn cancellation_interrupts_json_validation_batches() {
    let record = r#"{"protocol":"trojan","settings":{"servers":[{"address":"fixture.invalid","port":443,"password":"secret"}]}}"#;
    let body = format!(r#"{{"outbounds":[{}]}}"#, vec![record; 10_000].join(",")).into_bytes();
    let calls = Cell::new(0);
    let result = import_bytes_cancellable(body, || {
        calls.set(calls.get() + 1);
        calls.get() < 500
    });
    assert!(matches!(result, Err(CancellableImportError::Cancelled)));
    assert_eq!(calls.get(), 500);
}

#[test]
fn cancellation_interrupts_base64_decode_batches() {
    use base64::Engine;
    let body = b"trojan://secret@fixture.invalid:443\n".repeat(10_000);
    let encoded = base64::engine::general_purpose::STANDARD.encode(body);
    let limit = encoded.len().div_ceil(4096) + 5;
    let calls = Cell::new(0);
    let result = import_bytes_cancellable(encoded.into_bytes(), || {
        calls.set(calls.get() + 1);
        calls.get() < limit
    });
    assert!(matches!(result, Err(CancellableImportError::Cancelled)));
    assert_eq!(calls.get(), limit);
}
