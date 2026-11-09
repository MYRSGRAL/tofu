//! Ограниченное чтение переносимой политики без раскрытия содержимого в ошибках.

use std::{collections::HashSet, fmt, io::Read};

use serde::Deserialize;
use tofu_domain::{
    Action, Cidr, Conditions, DomainName, DomainPattern, Policy, PrivateDomain, PrivateNetwork,
    Rule, VpnId,
};

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_RULES: usize = 10_000;
const MAX_SELECTORS: usize = 100_000;
const MAX_EXECUTABLE_BYTES: usize = 4096;

/// Ошибки намеренно не содержат исходный JSON и ошибки нижележащего reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyInputError {
    Io,
    TooLarge,
    InvalidJson,
    InvalidPolicy,
    Limit,
}

impl fmt::Display for PolicyInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Io => "Не удалось прочитать политику",
            Self::TooLarge => "Превышен размер входной политики",
            Self::InvalidJson => "Некорректная структура JSON политики",
            Self::InvalidPolicy => "Некорректные значения политики",
            Self::Limit => "Превышен лимит правил или селекторов политики",
        })
    }
}

impl std::error::Error for PolicyInputError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputPolicy {
    version: u64,
    #[serde(default)]
    default_action: InputAction,
    #[serde(default)]
    user_rules: Vec<InputRule>,
    #[serde(default)]
    preset_rules: Vec<InputRule>,
    #[serde(default)]
    lan: Vec<String>,
    #[serde(default)]
    private_networks: Vec<InputNetwork>,
    #[serde(default)]
    private_domains: Vec<InputDomain>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum InputAction {
    #[default]
    Proxy,
    Direct,
    Block,
}

impl From<InputAction> for Action {
    fn from(value: InputAction) -> Self {
        match value {
            InputAction::Proxy => Self::Proxy,
            InputAction::Direct => Self::Direct,
            InputAction::Block => Self::Block,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputRule {
    id: u64,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    action: InputAction,
    #[serde(default)]
    conditions: InputConditions,
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct InputConditions {
    #[serde(default)]
    domain: Vec<String>,
    #[serde(default)]
    domain_suffix: Vec<String>,
    #[serde(default)]
    ip_cidr: Vec<String>,
    #[serde(default)]
    executable_paths: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputNetwork {
    cidr: String,
    vpn: u64,
    available: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputDomain {
    suffix: String,
    vpn: u64,
    available: bool,
}

/// Читает не более 2 МиБ + один байт для определения превышения размера.
pub fn read_policy(reader: impl Read) -> Result<Policy, PolicyInputError> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| PolicyInputError::Io)?;
    if bytes.len() > MAX_BYTES {
        return Err(PolicyInputError::TooLarge);
    }
    let input: InputPolicy =
        serde_json::from_slice(&bytes).map_err(|_| PolicyInputError::InvalidJson)?;
    if input.version != 1 {
        return Err(PolicyInputError::InvalidPolicy);
    }
    check_limits(&input)?;
    Ok(Policy {
        default_action: input.default_action.into(),
        user_rules: convert_rules(input.user_rules)?,
        preset_rules: convert_rules(input.preset_rules)?,
        lan: input
            .lan
            .iter()
            .map(|value| parse_cidr(value))
            .collect::<Result<_, _>>()?,
        private_networks: input
            .private_networks
            .into_iter()
            .map(|value| {
                Ok(PrivateNetwork {
                    network: parse_cidr(&value.cidr)?,
                    vpn: parse_vpn(value.vpn)?,
                    available: value.available,
                })
            })
            .collect::<Result<_, PolicyInputError>>()?,
        private_domains: input
            .private_domains
            .into_iter()
            .map(|value| {
                Ok(PrivateDomain {
                    domain: DomainPattern::Suffix(parse_domain(&value.suffix)?),
                    vpn: parse_vpn(value.vpn)?,
                    available: value.available,
                })
            })
            .collect::<Result<_, PolicyInputError>>()?,
    })
}

fn check_limits(input: &InputPolicy) -> Result<(), PolicyInputError> {
    if input.user_rules.len() + input.preset_rules.len() > MAX_RULES {
        return Err(PolicyInputError::Limit);
    }
    let mut selectors =
        input.lan.len() + input.private_networks.len() + input.private_domains.len();
    for rule in input.user_rules.iter().chain(&input.preset_rules) {
        let conditions = &rule.conditions;
        selectors += conditions.domain.len()
            + conditions.domain_suffix.len()
            + conditions.ip_cidr.len()
            + conditions.executable_paths.len();
        if selectors > MAX_SELECTORS {
            return Err(PolicyInputError::Limit);
        }
    }
    if selectors > MAX_SELECTORS {
        return Err(PolicyInputError::Limit);
    }
    Ok(())
}

fn convert_rules(input: Vec<InputRule>) -> Result<Vec<Rule>, PolicyInputError> {
    let mut ids = HashSet::with_capacity(input.len());
    input
        .into_iter()
        .map(|rule| {
            if !ids.insert(rule.id) {
                return Err(PolicyInputError::InvalidPolicy);
            }
            let conditions = rule.conditions;
            if conditions.executable_paths.iter().any(|path| {
                !path.starts_with('/')
                    || path.len() > MAX_EXECUTABLE_BYTES
                    || path.chars().any(char::is_control)
            }) {
                return Err(PolicyInputError::InvalidPolicy);
            }
            let domains = conditions
                .domain
                .iter()
                .map(|value| parse_domain(value).map(DomainPattern::Exact))
                .chain(
                    conditions
                        .domain_suffix
                        .iter()
                        .map(|value| parse_domain(value).map(DomainPattern::Suffix)),
                )
                .collect::<Result<_, _>>()?;
            Ok(Rule {
                id: rule.id,
                enabled: rule.enabled,
                action: rule.action.into(),
                conditions: Conditions {
                    executables: conditions.executable_paths,
                    destinations: conditions
                        .ip_cidr
                        .iter()
                        .map(|value| parse_cidr(value))
                        .collect::<Result<_, _>>()?,
                    domains,
                },
            })
        })
        .collect()
}

fn parse_cidr(value: &str) -> Result<Cidr, PolicyInputError> {
    value.parse().map_err(|_| PolicyInputError::InvalidPolicy)
}

fn parse_domain(value: &str) -> Result<DomainName, PolicyInputError> {
    value.parse().map_err(|_| PolicyInputError::InvalidPolicy)
}

fn parse_vpn(value: u64) -> Result<VpnId, PolicyInputError> {
    VpnId::new(value).ok_or(PolicyInputError::InvalidPolicy)
}
