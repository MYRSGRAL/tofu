use std::num::NonZeroU64;
use tofu_application::{
    ConfigDigest, EnvironmentStamp, Error, Event, Fault, Outcome, Session, State,
};

const ENV: EnvironmentStamp = EnvironmentStamp {
    network: 1,
    applications: 2,
    authorization: 3,
    engine: 4,
    authorized: true,
};
const DIGEST: ConfigDigest = ConfigDigest::new([7; 32]);

#[test]
fn no_authorization_during_prepare_requires_a_new_attempt() {
    let mut session = Session::new(NonZeroU64::MIN);
    let token = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        session
            .prepare(
                token,
                EnvironmentStamp {
                    authorized: false,
                    ..ENV
                },
                DIGEST
            )
            .err(),
        Some(Error::PermissionDenied)
    );
    assert_eq!(session.state(), State::Error);
    assert_eq!(
        session.prepare(token, ENV, DIGEST).err(),
        Some(Error::StaleOperation)
    );
    assert_eq!(session.applied_generation(), None);
}

#[test]
fn apply_requires_matching_validated_plan_and_preserves_unapplied_generation() {
    let mut session = Session::new(NonZeroU64::MIN);
    let preparing = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    let plan = session
        .prepare(preparing, ENV, DIGEST)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(session.state(), State::Preparing);
    assert_eq!(session.applied_generation(), None);
    assert_eq!(
        session.prepare(preparing, ENV, DIGEST).err(),
        Some(Error::StaleOperation)
    );
    let starting = match session.apply(plan, ENV, DIGEST) {
        Ok(Outcome::Applied(Some(token))) => token,
        other => panic!("План не применён: {other:?}"),
    };
    assert_eq!(session.state(), State::Starting);
    assert_eq!(session.applied_generation(), None);
    assert!(matches!(
        session.accept(starting, Event::NetworkApplied),
        Ok(Outcome::Applied(Some(_)))
    ));
    assert_eq!(session.applied_generation(), Some(preparing.generation()));
}

#[test]
fn each_generation_change_rejects_plan_before_network_start() {
    let mut environments = [ENV; 4];
    environments[0].network += 1;
    environments[1].applications += 1;
    environments[2].authorization += 1;
    environments[3].engine += 1;
    for current in environments {
        let mut session = Session::new(NonZeroU64::MIN);
        let token = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
        let plan = session
            .prepare(token, ENV, DIGEST)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(session.apply(plan, current, DIGEST), Err(Error::StalePlan));
        assert_eq!(session.state(), State::Error);
        assert_eq!(session.fault(), Some(Fault::StalePlan));
        assert_eq!(session.applied_generation(), None);
        assert_eq!(
            session.accept(token, Event::NetworkApplied),
            Ok(Outcome::Ignored)
        );
        let retry = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
        assert!(retry.generation() > token.generation());
    }
}

#[test]
fn config_digest_mismatch_is_not_allowed_to_start_engine() {
    let mut session = Session::new(NonZeroU64::MIN);
    let token = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    let plan = session
        .prepare(token, ENV, DIGEST)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        session.apply(plan, ENV, ConfigDigest::new([8; 32])),
        Err(Error::StalePlan)
    );
    assert_eq!(session.state(), State::Error);
    assert_eq!(session.applied_generation(), None);
}

#[test]
fn lost_authorization_rejects_even_unchanged_generations() {
    let mut session = Session::new(NonZeroU64::MIN);
    let token = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    let plan = session
        .prepare(token, ENV, DIGEST)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let denied = EnvironmentStamp {
        authorized: false,
        ..ENV
    };
    assert_eq!(
        session.apply(plan, denied, DIGEST),
        Err(Error::PermissionDenied)
    );
    assert_eq!(session.fault(), Some(Fault::Permission));
    assert_eq!(session.state(), State::Error);
}

#[test]
fn cancelled_plan_cannot_damage_replacement_session_even_if_environment_changed() {
    let mut session = Session::new(NonZeroU64::MIN);
    let token = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    let plan = session
        .prepare(token, ENV, DIGEST)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(session.stop(), Ok(None));
    let replacement = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        session.apply(
            plan,
            EnvironmentStamp {
                authorized: false,
                ..ENV
            },
            DIGEST
        ),
        Ok(Outcome::Ignored)
    );
    assert_eq!(session.state(), State::Preparing);
    assert_eq!(session.fault(), None);
    assert!(session.prepare(replacement, ENV, DIGEST).is_ok());
}

#[test]
fn foreign_plan_cannot_advance_controller_with_identical_local_counters() {
    let mut first = Session::new(NonZeroU64::MIN);
    let mut second = Session::new(NonZeroU64::new(2).unwrap_or_else(|| panic!("Неверный test ID")));
    let a = first.begin().unwrap_or_else(|e| panic!("{e:?}"));
    let b = second.begin().unwrap_or_else(|e| panic!("{e:?}"));
    let plan = first
        .prepare(a, ENV, DIGEST)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let own = second
        .prepare(b, ENV, DIGEST)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(second.apply(plan, ENV, DIGEST), Ok(Outcome::Ignored));
    assert!(matches!(
        second.apply(own, ENV, DIGEST),
        Ok(Outcome::Applied(Some(_)))
    ));
}
