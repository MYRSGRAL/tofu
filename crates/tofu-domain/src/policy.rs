use crate::{Cidr, DomainPattern, VpnId};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Action {
    Direct,
    #[default]
    Proxy,
    Block,
}

/// Результат содержит VPN только после проверки приватной сети.
/// Пользовательские правила ограничены Direct/Proxy/Block.
/// ```compile_fail
/// use tofu_domain::Action;
/// let invalid = Action::PrivateVpn;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Direct,
    Proxy,
    Block,
    PrivateVpn,
}

impl From<Action> for Route {
    fn from(action: Action) -> Self {
        match action {
            Action::Direct => Self::Direct,
            Action::Proxy => Self::Proxy,
            Action::Block => Self::Block,
        }
    }
}

/// Только подтверждённый host-адаптером executable, не display name/PID.
pub struct Flow<'a> {
    pub executable: Option<&'a str>,
    pub destination: IpAddr,
}

#[derive(Debug, Default)]
pub struct Conditions {
    pub executables: Vec<String>,
    pub destinations: Vec<Cidr>,
    pub domains: Vec<DomainPattern>,
}

impl Conditions {
    pub(crate) fn matches_source(&self, executable: Option<&str>) -> bool {
        self.executables.is_empty()
            || executable.is_some_and(|path| self.executables.iter().any(|p| p == path))
    }
}

#[derive(Debug)]
pub struct Rule {
    pub id: u64,
    pub enabled: bool,
    pub conditions: Conditions,
    pub action: Action,
}

#[derive(Debug)]
pub struct PrivateNetwork {
    pub network: Cidr,
    pub available: bool,
    pub vpn: VpnId,
}

#[derive(Debug, Default)]
pub struct Policy {
    pub private_networks: Vec<PrivateNetwork>,
    pub private_domains: Vec<crate::PrivateDomain>,
    pub lan: Vec<Cidr>,
    pub user_rules: Vec<Rule>,
    pub preset_rules: Vec<Rule>,
    pub default_action: Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleOrigin {
    PrivateNetwork,
    PrivateDomain,
    Lan,
    User(u64),
    Preset(u64),
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub action: Route,
    pub origin: RuleOrigin,
}

impl Policy {
    /// Совместимый расчёт для literal IP без имени. Для DNS/fake и кода отказа
    /// используйте evaluate_destination; неоднозначность здесь даёт Block.
    pub fn evaluate(&self, flow: &Flow<'_>) -> Decision {
        let request = crate::DestinationFlow {
            executable: flow.executable,
            destination: crate::Destination::Ip {
                address: flow.destination,
                name: None,
            },
        };
        match self.evaluate_destination(&request, &crate::EvaluationContext::default()) {
            crate::Evaluation::Decided { decision, .. } => decision,
            _ => Decision {
                action: Route::Block,
                origin: RuleOrigin::PrivateNetwork,
            },
        }
    }
}
