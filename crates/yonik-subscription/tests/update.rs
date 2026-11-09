use std::num::NonZeroU64;
use yonik_subscription::{Error, Subscription};

const VALID: &str = "trojan://secret@example.org:443#old";

fn subscription(id: u64) -> Subscription {
    Subscription::new(NonZeroU64::new(id).expect("ID"))
}

fn install(subscription: &mut Subscription, body: &str) {
    let ticket = subscription.begin().expect("begin");
    let candidate = subscription
        .stage(ticket, body.as_bytes().to_vec())
        .expect("stage");
    let approved = candidate.accept_clean().expect("clean");
    subscription.commit(approved).expect("commit");
}

#[test]
fn failed_empty_and_unusable_refresh_preserve_last_snapshot() {
    let mut state = subscription(1);
    install(&mut state, VALID);
    for body in ["", "broken", "unknown://x"] {
        let ticket = state.begin().expect("begin");
        assert!(state.stage(ticket, body.as_bytes().to_vec()).is_err());
        state.finish(ticket).expect("finish");
        assert_eq!(state.revision(), 1);
        assert_eq!(
            state.snapshot().expect("snapshot").original().expose(),
            VALID
        );
    }
}

#[test]
fn explicit_partial_approval_is_bound_to_the_exact_candidate() {
    let mut state = subscription(1);
    install(&mut state, VALID);
    let ticket = state.begin().expect("begin");
    let partial = format!("{VALID}\nunknown://x");
    let candidate = state
        .stage(ticket, partial.as_bytes().to_vec())
        .expect("stage");
    assert_eq!(candidate.summary().accepted, 1);
    assert_eq!(candidate.summary().rejected, 1);
    assert!(candidate.summary().needs_review());
    assert_eq!(
        candidate.accept_clean().expect_err("needs review"),
        Error::ReviewRequired
    );
    let candidate = state
        .stage(ticket, partial.as_bytes().to_vec())
        .expect("stage again");
    let approval = candidate.approve_after_review();
    state.begin().expect("newer update supersedes old");
    assert_eq!(state.commit(approval), Err(Error::Stale));
    assert_eq!(state.revision(), 1);
}

#[test]
fn source_change_and_cancellation_invalidate_inflight_import() {
    let mut state = subscription(1);
    install(&mut state, VALID);
    for change_source in [false, true] {
        let ticket = state.begin().expect("begin");
        let candidate = state
            .stage(ticket, VALID.as_bytes().to_vec())
            .expect("stage");
        if change_source {
            state.source_changed().expect("source change");
        } else {
            state.finish(ticket).expect("cancel");
        }
        assert_eq!(
            state.commit(candidate.accept_clean().expect("clean")),
            Err(Error::Stale)
        );
        assert_eq!(state.revision(), 1);
        assert!(state.snapshot().is_some());
    }
    assert_eq!(state.source_generation(), 1);
}

#[test]
fn foreign_ticket_and_approval_cannot_modify_another_subscription() {
    let mut first = subscription(1);
    let mut second = subscription(2);
    let ticket = first.begin().expect("begin");
    second.begin().expect("begin");
    assert_eq!(
        second
            .stage(ticket, VALID.as_bytes().to_vec())
            .expect_err("foreign"),
        Error::Stale
    );
    let approved = first
        .stage(ticket, VALID.as_bytes().to_vec())
        .expect("stage")
        .accept_clean()
        .expect("clean");
    assert_eq!(second.commit(approved), Err(Error::Stale));
}

#[test]
fn commit_is_single_use_and_native_json_is_explicitly_opaque() {
    let mut state = subscription(1);
    let ticket = state.begin().expect("begin");
    let candidate = state
        .stage(ticket, b"{\"outbounds\":[]}".to_vec())
        .expect("native");
    assert!(candidate.summary().native_json);
    assert!(candidate.summary().needs_review());
    state
        .commit(candidate.approve_after_review())
        .expect("commit opaque");
    assert_eq!(state.revision(), 1);
    assert!(state.snapshot().expect("snapshot").native_json.is_some());
    assert_eq!(state.finish(ticket), Err(Error::Stale));
    let debug = format!("{state:?}");
    assert!(!debug.contains("outbounds"));
}
