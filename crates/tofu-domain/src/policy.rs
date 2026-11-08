use crate::Cidr;
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
}

impl Conditions {
    fn matches(&self, flow: &Flow<'_>) -> bool {
        (self.executables.is_empty()
            || flow
                .executable
                .is_some_and(|path| self.executables.iter().any(|p| p == path)))
            && (self.destinations.is_empty()
                || self
                    .destinations
                    .iter()
                    .any(|cidr| cidr.contains(flow.destination)))
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
}

#[derive(Debug, Default)]
pub struct Policy {
    pub private_networks: Vec<PrivateNetwork>,
    pub lan: Vec<Cidr>,
    pub user_rules: Vec<Rule>,
    pub preset_rules: Vec<Rule>,
    pub default_action: Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleOrigin {
    PrivateNetwork,
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
    /// Расчёт политики, не доказательство фактического пути пакета.
    /// Служебные исключения и подтверждение приватных маршрутов выполняет host.
    pub fn evaluate(&self, flow: &Flow<'_>) -> Decision {
        let mut private_match = false;
        for private in &self.private_networks {
            if private.network.contains(flow.destination) {
                private_match = true;
                // Пересечение с недоступной защищённой сетью не разрешает обход.
                if !private.available {
                    return Decision {
                        action: Route::Block,
                        origin: RuleOrigin::PrivateNetwork,
                    };
                }
            }
        }
        if private_match {
            return Decision {
                action: Route::PrivateVpn,
                origin: RuleOrigin::PrivateNetwork,
            };
        }
        if self.lan.iter().any(|cidr| cidr.contains(flow.destination)) {
            return Decision {
                action: Route::Direct,
                origin: RuleOrigin::Lan,
            };
        }
        for (rules, origin) in [
            (&self.user_rules, RuleOrigin::User as fn(u64) -> RuleOrigin),
            (&self.preset_rules, RuleOrigin::Preset),
        ] {
            if let Some(rule) = rules
                .iter()
                .find(|rule| rule.enabled && rule.conditions.matches(flow))
            {
                return Decision {
                    action: rule.action.into(),
                    origin: origin(rule.id),
                };
            }
        }
        Decision {
            action: self.default_action.into(),
            origin: RuleOrigin::Default,
        }
    }
}
