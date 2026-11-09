//! Переносимая логика управления сессией; I/O выполняет платформенный host.

use std::{fmt, num::NonZeroU64};

mod plan;
pub use plan::{ConfigDigest, EnvironmentStamp, PreparedPlan};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Disconnected,
    Preparing,
    Starting,
    Checking,
    Connected,
    Limited,
    Stopping,
    Error,
    CleanupError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    Validation,
    Engine,
    Network,
    Permission,
    StalePlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    NetworkApplied,
    CheckPassed,
    CheckInvalidated,
    EndpointUnavailable,
    Failed(Fault),
    CleanupConfirmed,
    CleanupFailed,
}

/// Correlation token, не средство авторизации. Поля создаёт только controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    controller: NonZeroU64,
    request: u64,
    generation: u64,
}

impl Token {
    pub fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ignored,
    Applied(Option<Token>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Busy,
    InvalidTransition,
    Exhausted,
    StaleOperation,
    StalePlan,
    PermissionDenied,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "Сессия занята",
            Self::InvalidTransition => "Недопустимый переход сессии",
            Self::Exhausted => "Исчерпаны идентификаторы операций",
            Self::StaleOperation => "Операция устарела или отменена",
            Self::StalePlan => "Проверенный план устарел",
            Self::PermissionDenied => "Недостаточно прав для применения плана",
        })
    }
}
impl std::error::Error for Error {}

/// Один host сериализует команды и владеет reducer всю жизнь controller.
/// При пересоздании controller host обязан выбрать новый уникальный ID,
/// если callbacks предыдущего экземпляра ещё могут прийти.
#[derive(Debug)]
pub struct Session {
    controller: NonZeroU64,
    state: State,
    request: u64,
    generation: u64,
    active: Option<Token>,
    applied: Option<u64>,
    fault: Option<Fault>,
}

impl Session {
    pub const fn new(controller: NonZeroU64) -> Self {
        Self {
            controller,
            state: State::Disconnected,
            request: 0,
            generation: 0,
            active: None,
            applied: None,
            fault: None,
        }
    }
    pub fn state(&self) -> State {
        self.state
    }
    pub fn applied_generation(&self) -> Option<u64> {
        self.applied
    }
    pub fn fault(&self) -> Option<Fault> {
        self.fault
    }

    fn next_token(&self, generation: u64) -> Result<Token, Error> {
        Ok(Token {
            controller: self.controller,
            request: self.request.checked_add(1).ok_or(Error::Exhausted)?,
            generation,
        })
    }

    pub fn begin(&mut self) -> Result<Token, Error> {
        if !matches!(self.state, State::Disconnected | State::Error) {
            return Err(Error::Busy);
        }
        let generation = self.generation.checked_add(1).ok_or(Error::Exhausted)?;
        let token = self.next_token(generation)?;
        self.request = token.request;
        self.generation = generation;
        self.active = Some(token);
        self.state = State::Preparing;
        self.fault = None;
        Ok(token)
    }

    /// Возвращает cleanup token; None означает отсутствие сетевых ресурсов сессии.
    /// Подготовительные I/O-задачи и их отмена остаются ответственностью host.
    /// Повторный Stop в Stopping сохраняет token: host-release идемпотентен.
    pub fn stop(&mut self) -> Result<Option<Token>, Error> {
        match self.state {
            State::Disconnected | State::Error => return Ok(None),
            State::Preparing => {
                self.active = None;
                self.state = State::Disconnected;
                return Ok(None);
            }
            State::Stopping => return Ok(self.active),
            _ => {}
        }
        let token = self.next_token(self.generation)?;
        self.request = token.request;
        self.active = Some(token);
        self.state = State::Stopping;
        Ok(Some(token))
    }

    pub fn accept(&mut self, token: Token, event: Event) -> Result<Outcome, Error> {
        if self.active != Some(token) {
            return Ok(Outcome::Ignored);
        }
        let (state, fault) = match (self.state, event) {
            (State::Starting, Event::NetworkApplied) => (State::Checking, None),
            (State::Checking | State::Limited, Event::CheckPassed) => (State::Connected, None),
            (State::Checking | State::Connected | State::Limited, Event::CheckInvalidated) => {
                (State::Limited, None)
            }
            (State::Checking | State::Limited, Event::EndpointUnavailable) => {
                (State::Limited, None)
            }
            (State::Preparing, Event::Failed(fault)) => (State::Error, Some(fault)),
            (
                State::Starting | State::Checking | State::Connected | State::Limited,
                Event::Failed(fault),
            ) => (State::Stopping, Some(fault)),
            (State::Stopping, Event::CleanupConfirmed) => (
                if self.fault.is_some() {
                    State::Error
                } else {
                    State::Disconnected
                },
                self.fault,
            ),
            (State::Stopping, Event::CleanupFailed) => (State::CleanupError, self.fault),
            _ => return Err(Error::InvalidTransition),
        };
        let next = if matches!(state, State::Disconnected | State::Error) {
            None
        } else {
            Some(self.next_token(token.generation)?)
        };
        // Все проверки завершены до мутации: ошибочная операция не меняет snapshot.
        if let Some(next) = next {
            self.request = next.request;
        }
        if event == Event::NetworkApplied {
            self.applied = Some(token.generation);
        }
        if event == Event::CleanupConfirmed {
            self.applied = None;
        }
        self.active = next;
        self.state = state;
        self.fault = fault;
        Ok(Outcome::Applied(next))
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, Session, State};
    use std::num::NonZeroU64;

    #[test]
    fn exhausted_counters_leave_session_unchanged() {
        for (request, generation) in [(u64::MAX, 0), (0, u64::MAX)] {
            let mut session = Session::new(NonZeroU64::MIN);
            session.request = request;
            session.generation = generation;
            assert_eq!(session.begin(), Err(Error::Exhausted));
            assert_eq!(session.state(), State::Disconnected);
            assert_eq!(session.request, request);
            assert_eq!(session.generation, generation);
            assert_eq!(session.active, None);
        }
    }

    #[test]
    fn exhausted_request_cannot_partially_apply_or_cancel_running_operation() {
        let mut session = Session::new(NonZeroU64::MIN);
        let first = session.begin().unwrap_or_else(|e| panic!("{e:?}"));
        let stamp = super::EnvironmentStamp {
            network: 0,
            applications: 0,
            authorization: 0,
            engine: 0,
            authorized: true,
        };
        let digest = super::ConfigDigest::new([0; 32]);
        let plan = session
            .prepare(first, stamp, digest)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let token = match session.apply(plan, stamp, digest) {
            Ok(super::Outcome::Applied(Some(token))) => token,
            other => panic!("Ожидался Starting: {other:?}"),
        };
        session.request = u64::MAX;
        assert_eq!(
            session.accept(token, super::Event::NetworkApplied),
            Err(Error::Exhausted)
        );
        assert_eq!(session.stop(), Err(Error::Exhausted));
        assert_eq!(session.state(), State::Starting);
        assert_eq!(session.applied_generation(), None);
        assert_eq!(session.active, Some(token));
        assert_eq!(session.request, u64::MAX);
    }
}
