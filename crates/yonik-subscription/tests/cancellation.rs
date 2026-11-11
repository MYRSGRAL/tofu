use std::{cell::Cell, num::NonZeroU64};
use yonik_subscription::{CancellableStageError, Subscription};

#[test]
fn cancelled_stage_keeps_previous_revision() {
    let mut subscription = Subscription::new(NonZeroU64::MIN);
    let ticket = subscription.begin().unwrap();
    let first = subscription
        .stage(ticket, b"trojan://old@fixture.invalid:443".to_vec())
        .unwrap();
    subscription.commit(first.accept_clean().unwrap()).unwrap();
    let ticket = subscription.begin().unwrap();
    let calls = Cell::new(0);
    let result = subscription.stage_cancellable(
        ticket,
        b"trojan://new@fixture.invalid:443\n".repeat(10_000),
        || {
            calls.set(calls.get() + 1);
            calls.get() < 100
        },
    );
    assert!(matches!(result, Err(CancellableStageError::Cancelled)));
    assert_eq!(calls.get(), 100);
    assert_eq!(subscription.revision(), 1);
    assert_eq!(subscription.snapshot().unwrap().servers.len(), 1);
    subscription.finish(ticket).unwrap();
    assert!(subscription.begin().is_ok());
}
