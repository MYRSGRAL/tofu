use std::num::NonZeroU64;
use tofu_application::{
    ConfigDigest, EnvironmentStamp, Error, Event, Fault, Outcome, Session, State, Token,
};

const ENV: EnvironmentStamp = EnvironmentStamp {
    network: 0,
    applications: 0,
    authorization: 0,
    engine: 0,
    authorized: true,
};
const DIGEST: ConfigDigest = ConfigDigest::new([0; 32]);

fn starting(session: &mut Session, token: Token) -> Token {
    let plan = session
        .prepare(token, ENV, DIGEST)
        .unwrap_or_else(|e| panic!("{e:?}"));
    match session.apply(plan, ENV, DIGEST) {
        Ok(Outcome::Applied(Some(token))) => token,
        other => panic!("Ожидался Starting: {other:?}"),
    }
}

fn advance(session: &mut Session, token: Token, event: Event) -> Token {
    match session.accept(token, event) {
        Ok(Outcome::Applied(Some(next))) => next,
        result => panic!("Ожидался новый token: {result:?}"),
    }
}

fn checking(session: &mut Session) -> Token {
    let token = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    let token = starting(session, token);
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
    let second = starting(&mut session, first);
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
    let repeated = advance(&mut session, limited, Event::EndpointUnavailable);
    assert_eq!(session.state(), State::Limited);
    advance(&mut session, repeated, Event::CheckPassed);
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
    let starting = starting(&mut session, token);
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
    let next = starting(&mut session, token);
    assert_eq!(
        session.prepare(token, ENV, DIGEST).err(),
        Some(Error::StaleOperation)
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
        second.prepare(foreign, ENV, DIGEST).err(),
        Some(Error::StaleOperation)
    );
    starting(&mut second, own);
    assert_eq!(second.state(), State::Starting);
}

#[test]
fn cancel_preparation_and_duplicate_stop_do_not_revive_old_operation() {
    let mut session = Session::new(NonZeroU64::MIN);
    let old = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(session.stop(), Ok(None));
    assert_eq!(session.state(), State::Disconnected);
    assert_eq!(
        session.prepare(old, ENV, DIGEST).err(),
        Some(Error::StaleOperation)
    );
    let current = checking(&mut session);
    let stop = session.stop().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(session.stop(), Ok(stop));
    assert_eq!(
        session.accept(current, Event::CheckPassed),
        Ok(Outcome::Ignored)
    );
}

#[test]
fn invalidated_check_keeps_applied_generation_and_rejects_stale_results() {
    for event in [
        None,
        Some(Event::CheckPassed),
        Some(Event::EndpointUnavailable),
    ] {
        let mut session = Session::new(NonZeroU64::MIN);
        let mut token = checking(&mut session);
        if let Some(event) = event {
            token = advance(&mut session, token, event);
        }
        let generation = session.applied_generation();
        let invalidated = advance(&mut session, token, Event::CheckInvalidated);
        assert_eq!(session.state(), State::Limited);
        assert_eq!(session.applied_generation(), generation);
        assert_eq!(invalidated.generation(), token.generation());
        assert_ne!(invalidated, token);
        assert_eq!(session.fault(), None);
        assert_eq!(session.begin(), Err(Error::Busy));
        assert_eq!(
            session.accept(token, Event::CheckPassed),
            Ok(Outcome::Ignored)
        );
        assert_eq!(session.state(), State::Limited);
        advance(&mut session, invalidated, Event::CheckPassed);
        assert_eq!(session.state(), State::Connected);
    }
}

#[test]
fn check_invalidation_rejects_inactive_phases_without_mutation() {
    let mut session = Session::new(NonZeroU64::MIN);
    let preparing = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        session.accept(preparing, Event::CheckInvalidated),
        Err(Error::InvalidTransition)
    );
    assert_eq!(session.state(), State::Preparing);
    let token = starting(&mut session, preparing);
    assert_eq!(
        session.accept(token, Event::CheckInvalidated),
        Err(Error::InvalidTransition)
    );
    assert_eq!(session.state(), State::Starting);
    let token = advance(&mut session, token, Event::NetworkApplied);
    let stopping = session.stop().unwrap().unwrap();
    assert_eq!(
        session.accept(stopping, Event::CheckInvalidated),
        Err(Error::InvalidTransition)
    );
    assert_eq!(session.state(), State::Stopping);
    assert_eq!(session.applied_generation(), Some(token.generation()));
    let cleanup_error = advance(&mut session, stopping, Event::CleanupFailed);
    assert_eq!(
        session.accept(cleanup_error, Event::CheckInvalidated),
        Err(Error::InvalidTransition)
    );
    assert_eq!(session.state(), State::CleanupError);
    assert_eq!(session.applied_generation(), Some(token.generation()));
    let cleanup = session.stop().unwrap().unwrap();
    assert_eq!(
        session.accept(cleanup, Event::CleanupConfirmed),
        Ok(Outcome::Applied(None))
    );
    assert_eq!(
        session.accept(cleanup, Event::CheckInvalidated),
        Ok(Outcome::Ignored)
    );
    assert_eq!(session.state(), State::Disconnected);
}
