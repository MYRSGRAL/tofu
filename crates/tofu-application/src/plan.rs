use crate::{Error, Fault, Outcome, Session, State, Token};

/// Поколения выдаёт доверенный host без переиспользования значений, пока живы планы.
/// Любое релевантное изменение, включая уход и возврат VPN/прав, меняет поколение.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvironmentStamp {
    pub network: u64,
    pub applications: u64,
    pub authorization: u64,
    pub engine: u64,
    pub authorized: bool,
}

/// SHA-256 проверенного immutable-конфига вычисляет host, не reducer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ConfigDigest([u8; 32]);

impl ConfigDigest {
    pub const fn new(sha256: [u8; 32]) -> Self {
        Self(sha256)
    }
}

impl std::fmt::Debug for ConfigDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConfigDigest(..)")
    }
}

/// Одноразовый план после host config-check. Не является разрешением ОС.
/// ```compile_fail
/// use tofu_application::PreparedPlan;
/// fn duplicate(plan: PreparedPlan) { let another = plan.clone(); }
/// ```
/// ```compile_fail
/// use tofu_application::{ConfigDigest, EnvironmentStamp, PreparedPlan, Session};
/// fn replay(session: &mut Session, plan: PreparedPlan, env: EnvironmentStamp, digest: ConfigDigest) {
///     let _ = session.apply(plan, env, digest);
///     let _ = session.apply(plan, env, digest);
/// }
/// ```
/// Старого перехода, обходящего apply guard, нет:
/// ```compile_fail
/// use tofu_application::Event;
/// let bypass = Event::Validated;
/// ```
#[derive(Debug)]
#[must_use = "План следует применить или отменить через Session::stop"]
pub struct PreparedPlan {
    token: Token,
    environment: EnvironmentStamp,
    digest: ConfigDigest,
}

impl Session {
    /// Host вызывает после успешного config-check, до любых сетевых эффектов.
    pub fn prepare(
        &mut self,
        token: Token,
        environment: EnvironmentStamp,
        digest: ConfigDigest,
    ) -> Result<PreparedPlan, Error> {
        if self.active != Some(token) {
            return Err(Error::StaleOperation);
        }
        if self.state != State::Preparing {
            return Err(Error::InvalidTransition);
        }
        if !environment.authorized {
            self.reject_prepared(Fault::Permission);
            return Err(Error::PermissionDenied);
        }
        let next = self.next_token(token.generation)?;
        self.request = next.request;
        self.active = Some(next);
        Ok(PreparedPlan {
            token: next,
            environment,
            digest,
        })
    }

    /// Snapshot и digest должны относиться к тому же artifact прямо перед Apply.
    /// Host обеспечивает атомарность со своими эффектами и не делает скрытый re-read.
    pub fn apply(
        &mut self,
        plan: PreparedPlan,
        current: EnvironmentStamp,
        digest: ConfigDigest,
    ) -> Result<Outcome, Error> {
        // Сначала correlation: старый план не должен портить новую активную сессию.
        if self.active != Some(plan.token) {
            return Ok(Outcome::Ignored);
        }
        if self.state != State::Preparing {
            return Err(Error::InvalidTransition);
        }
        if !current.authorized {
            self.reject_prepared(Fault::Permission);
            return Err(Error::PermissionDenied);
        }
        if plan.environment != current || plan.digest != digest {
            self.reject_prepared(Fault::StalePlan);
            return Err(Error::StalePlan);
        }
        let next = self.next_token(plan.token.generation)?;
        self.request = next.request;
        self.active = Some(next);
        self.state = State::Starting;
        Ok(Outcome::Applied(Some(next)))
    }

    fn reject_prepared(&mut self, fault: Fault) {
        self.state = State::Error;
        self.active = None;
        self.fault = Some(fault);
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigDigest, EnvironmentStamp};
    use crate::{Error, Session, State};
    use std::num::NonZeroU64;

    const ENV: EnvironmentStamp = EnvironmentStamp {
        network: 1,
        applications: 1,
        authorization: 1,
        engine: 1,
        authorized: true,
    };
    const DIGEST: ConfigDigest = ConfigDigest::new([1; 32]);

    #[test]
    fn exhausted_prepare_and_apply_do_not_start_or_replace_active_operation() {
        let mut session = Session::new(NonZeroU64::MIN);
        let token = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
        session.request = u64::MAX;
        assert_eq!(
            session.prepare(token, ENV, DIGEST).err(),
            Some(Error::Exhausted)
        );
        assert_eq!(session.state(), State::Preparing);
        assert_eq!(session.active, Some(token));

        let mut session = Session::new(NonZeroU64::MIN);
        let token = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
        let plan = session
            .prepare(token, ENV, DIGEST)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let active = plan.token;
        session.request = u64::MAX;
        assert_eq!(session.apply(plan, ENV, DIGEST), Err(Error::Exhausted));
        assert_eq!(session.state(), State::Preparing);
        assert_eq!(session.active, Some(active));
        assert_eq!(session.applied_generation(), None);
        assert_eq!(session.fault(), None);
    }
}
