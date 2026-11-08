use std::num::NonZeroU64;
use tofu_application::{Error, Event, Fault, Outcome, Session, State, Token};

fn advance(session: &mut Session, token: Token, event: Event) -> Token {
    match session.accept(token, event) {
        Ok(Outcome::Applied(Some(next))) => next,
        result => panic!("Ожидался новый token: {result:?}"),
    }
}

fn checking(session: &mut Session) -> Token {
    let token = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    let token = advance(session, token, Event::Validated);
    advance(session, token, Event::NetworkApplied)
}

#[test]
fn engine_start_is_not_a_verified_connection() {
    let mut session = Session::new(NonZeroU64::MIN);
    let first = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(session.state(), State::Preparing);
    assert_eq!(
        session.accept(first, Event::CheckPassed),
        Err(Error::InvalidTransition)
    );
    let second = advance(&mut session, first, Event::Validated);
    assert_eq!(session.state(), State::Starting);
    assert_eq!(session.applied_generation(), None);
    let third = advance(&mut session, second, Event::NetworkApplied);
    assert_eq!(session.state(), State::Checking);
    assert_eq!(session.applied_generation(), Some(first.generation()));
    advance(&mut session, third, Event::CheckPassed);
    assert_eq!(session.state(), State::Connected);
}

#[test]
fn endpoint_failure_keeps_limited_session_and_can_be_rechecked() {
    let mut session = Session::new(NonZeroU64::MIN);
    let token = checking(&mut session);
    let limited = advance(&mut session, token, Event::EndpointUnavailable);
    assert_eq!(session.state(), State::Limited);
    assert!(session.applied_generation().is_some());
    advance(&mut session, limited, Event::CheckPassed);
    assert_eq!(session.state(), State::Connected);
}

#[test]
fn stop_invalidates_callbacks_and_requires_confirmed_cleanup() {
    let mut session = Session::new(NonZeroU64::MIN);
    let old = checking(&mut session);
    let stop = session
        .stop()
        .unwrap_or_else(|e| panic!("{e:?}"))
        .unwrap_or_else(|| panic!("Нет cleanup token"));
    assert_eq!(session.state(), State::Stopping);
    assert_eq!(
        session.accept(old, Event::CheckPassed),
        Ok(Outcome::Ignored)
    );
    let failed = advance(&mut session, stop, Event::CleanupFailed);
    assert_eq!(session.state(), State::CleanupError);
    assert!(session.applied_generation().is_some());
    assert_eq!(session.begin(), Err(Error::Busy));
    assert_eq!(
        session.accept(failed, Event::CleanupConfirmed),
        Err(Error::InvalidTransition)
    );
    let retry = session
        .stop()
        .unwrap_or_else(|e| panic!("{e:?}"))
        .unwrap_or_else(|| panic!("Нет retry token"));
    assert_eq!(
        session.accept(stop, Event::CleanupConfirmed),
        Ok(Outcome::Ignored)
    );
    assert_eq!(
        session.accept(retry, Event::CleanupConfirmed),
        Ok(Outcome::Applied(None))
    );
    assert_eq!(session.state(), State::Disconnected);
    assert_eq!(session.applied_generation(), None);
    assert_eq!(session.stop(), Ok(None));
}

#[test]
fn failure_after_launch_keeps_reason_until_cleanup_and_restart() {
    let mut session = Session::new(NonZeroU64::MIN);
    let token = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    let starting = advance(&mut session, token, Event::Validated);
    let cleanup = advance(&mut session, starting, Event::Failed(Fault::Engine));
    assert_eq!(session.state(), State::Stopping);
    assert_eq!(session.fault(), Some(Fault::Engine));
    assert_eq!(
        session.accept(cleanup, Event::CleanupConfirmed),
        Ok(Outcome::Applied(None))
    );
    assert_eq!(session.state(), State::Error);
    assert_eq!(session.fault(), Some(Fault::Engine));
    let next = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    assert!(next.generation() > token.generation());
    assert_eq!(session.fault(), None);
    assert_eq!(
        session.accept(starting, Event::NetworkApplied),
        Ok(Outcome::Ignored)
    );
}

#[test]
fn validation_failure_needs_no_network_cleanup_and_duplicate_events_are_stale() {
    let mut session = Session::new(NonZeroU64::MIN);
    let token = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(session.begin(), Err(Error::Busy));
    assert_eq!(
        session.accept(token, Event::Failed(Fault::Validation)),
        Ok(Outcome::Applied(None))
    );
    assert_eq!(session.state(), State::Error);
    let token = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    let next = advance(&mut session, token, Event::Validated);
    assert_eq!(
        session.accept(token, Event::Validated),
        Ok(Outcome::Ignored)
    );
    assert_eq!(session.state(), State::Starting);
    assert_eq!(
        session.accept(next, Event::CleanupConfirmed),
        Err(Error::InvalidTransition)
    );
}

#[test]
fn foreign_controller_token_cannot_advance_same_generation() {
    let mut first = Session::new(NonZeroU64::MIN);
    let mut second = Session::new(NonZeroU64::new(2).unwrap_or_else(|| panic!("Неверный test ID")));
    let foreign = first.begin().unwrap_or_else(|e| panic!("{e:?}"));
    let own = second.begin().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(foreign.generation(), own.generation());
    assert_eq!(
        second.accept(foreign, Event::Validated),
        Ok(Outcome::Ignored)
    );
    advance(&mut second, own, Event::Validated);
    assert_eq!(second.state(), State::Starting);
}

#[test]
fn cancel_preparation_and_duplicate_stop_do_not_revive_old_operation() {
    let mut session = Session::new(NonZeroU64::MIN);
    let old = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(session.stop(), Ok(None));
    assert_eq!(session.state(), State::Disconnected);
    assert_eq!(session.accept(old, Event::Validated), Ok(Outcome::Ignored));
    let current = checking(&mut session);
    let stop = session.stop().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(session.stop(), Ok(stop));
    assert_eq!(
        session.accept(current, Event::CheckPassed),
        Ok(Outcome::Ignored)
    );
}
