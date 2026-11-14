//! Чистая модель startup: снимки не являются разрешением на исполнение Connect.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    Xray,
    SingBox,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Local,
    System,
}

/// Только метаданные явно подтверждённого target в том же repository.
/// Совпадение полей не доказывает identity repository и не заменяет resolve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub source_id: i64,
    pub profile_id: i64,
    pub source_generation: i64,
    pub snapshot_generation: i64,
    pub snapshot_revision: i64,
    pub routing_revision: i64,
    pub engine: Engine,
    pub mode: Mode,
}
impl Target {
    pub fn valid(&self) -> bool {
        self.source_id > 0
            && self.profile_id > 0
            && self.source_generation >= 0
            && self.snapshot_generation >= 0
            && self.snapshot_revision >= 0
            && self.routing_revision >= 0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Custody {
    Unknown,
    Ready,
    Ephemeral,
    Locked,
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Unknown,
    Eligible,
    Blocked,
    Recovery,
    ForeignActive,
}

/// Снимки проверок, НЕ permit. Будущий dispatcher обязан заново проверить
/// repository identity, custody owner, policy/N5 admission и generation после await.
pub struct Input {
    pub enabled: bool,
    pub target: Option<Target>,
    pub resolved: Option<Target>,
    pub custody: Custody,
    pub admission: Admission,
    pub generation: u64,
    pub current_generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Generation,
    Binding,
    Custody,
    Admission,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Noop,
    Blocked(Reason),
    /// Рекомендация повторить реальные gates; не команда и не authority.
    Connect(Target),
}
/// Один экземпляр на lifetime host. Нет reset/rearm по network/key/tray events.
#[derive(Default)]
pub struct Planner {
    consumed: bool,
}
impl Planner {
    pub fn stop(&mut self) {
        self.consumed = true;
    }
    /// Вызывать на startup barrier или его deadline. Отказ окончателен для host.
    pub fn decide(&mut self, input: &Input) -> Decision {
        if self.consumed {
            return Decision::Noop;
        }
        self.consumed = true;
        if !input.enabled {
            return Decision::Noop;
        }
        if input.generation != input.current_generation {
            return Decision::Blocked(Reason::Generation);
        }
        if input.custody != Custody::Ready {
            return Decision::Blocked(Reason::Custody);
        }
        if input.admission != Admission::Eligible {
            return Decision::Blocked(Reason::Admission);
        }
        match (&input.target, &input.resolved) {
            (Some(target), Some(resolved)) if target.valid() && target == resolved => {
                Decision::Connect(target.clone())
            }
            _ => Decision::Blocked(Reason::Binding),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Admission, Custody, Decision, Engine, Input, Mode, Planner, Reason, Target};

    fn target() -> Target {
        Target {
            source_id: 1,
            profile_id: 2,
            source_generation: 3,
            snapshot_generation: 4,
            snapshot_revision: 5,
            routing_revision: 6,
            engine: Engine::Xray,
            mode: Mode::Local,
        }
    }
    fn input() -> Input {
        Input {
            enabled: true,
            target: Some(target()),
            resolved: Some(target()),
            custody: Custody::Ready,
            admission: Admission::Eligible,
            generation: 7,
            current_generation: 7,
        }
    }
    #[test]
    fn попытка_одна_stop_на_весь_host() {
        let mut host = Planner::default();
        assert_eq!(host.decide(&input()), Decision::Connect(target()));
        assert_eq!(host.decide(&input()), Decision::Noop);
        let mut host = Planner::default();
        host.stop();
        for _ in 0..3 {
            assert_eq!(host.decide(&input()), Decision::Noop);
        }
        assert_eq!(
            Planner::default().decide(&input()),
            Decision::Connect(target())
        );
    }
    #[test]
    fn off_не_подключает_и_invalid_target_отклоняется() {
        let mut i = input();
        i.enabled = false;
        let mut p = Planner::default();
        assert_eq!(p.decide(&i), Decision::Noop);
        assert_eq!(p.decide(&input()), Decision::Noop);
        i = input();
        i.target.as_mut().expect("target").source_id = 0;
        i.resolved = i.target.clone();
        assert_eq!(
            Planner::default().decide(&i),
            Decision::Blocked(Reason::Binding)
        );
    }
    #[test]
    fn все_отказы_закрыты_и_не_повторяются() {
        for custody in [
            Custody::Unknown,
            Custody::Ephemeral,
            Custody::Locked,
            Custody::Unavailable,
        ] {
            let mut i = input();
            i.custody = custody;
            let mut p = Planner::default();
            assert_eq!(p.decide(&i), Decision::Blocked(Reason::Custody));
            assert_eq!(p.decide(&input()), Decision::Noop);
        }
        for admission in [
            Admission::Unknown,
            Admission::Blocked,
            Admission::Recovery,
            Admission::ForeignActive,
        ] {
            let mut i = input();
            i.admission = admission;
            assert_eq!(
                Planner::default().decide(&i),
                Decision::Blocked(Reason::Admission)
            );
        }
        let mut i = input();
        i.current_generation += 1;
        assert_eq!(
            Planner::default().decide(&i),
            Decision::Blocked(Reason::Generation)
        );
        i = input();
        i.resolved = None;
        assert_eq!(
            Planner::default().decide(&i),
            Decision::Blocked(Reason::Binding)
        );
        for n in 0..8 {
            let mut i = input();
            let t = i.resolved.as_mut().expect("target");
            match n {
                0 => t.source_id += 1,
                1 => t.profile_id += 1,
                2 => t.source_generation += 1,
                3 => t.snapshot_generation += 1,
                4 => t.snapshot_revision += 1,
                5 => t.routing_revision += 1,
                6 => t.engine = Engine::SingBox,
                _ => t.mode = Mode::System,
            }
            assert_eq!(
                Planner::default().decide(&i),
                Decision::Blocked(Reason::Binding)
            );
        }
    }
}
