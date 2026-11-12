//! Ограниченная read-only диагностика. Не заменяет evaluate_destination.
use std::collections::BTreeSet;

use crate::{Action, Cidr, Conditions, DomainPattern, Policy, RuleOrigin};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_rules: usize,
    pub max_comparisons: usize,
    pub max_warnings: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_rules: 512,
            max_comparisons: 32_768,
            max_warnings: 256,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    Shadowed,
    /// Совместимость селекторов; существование DNS-ответа не доказано.
    PotentialOverlap,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Warning {
    pub rule_id: u64,
    pub earlier: RuleOrigin,
    pub kind: WarningKind,
    /// None для приватного назначения: результат зависит от VPN/доступности.
    pub earlier_action: Option<Action>,
    pub action: Action,
}
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub warnings: Vec<Warning>,
    pub comparisons: usize,
    pub work_units: usize,
    pub truncated: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputError {
    TooLarge,
    InvalidId,
    DuplicateId,
    InvalidExecutable,
    UnsupportedSelector(u64),
}

/// Максимумы вызывающего кода только уменьшают жёсткие пределы.
/// Ошибка входа означает отсутствие частичной диагностики.
pub fn diagnose(policy: &Policy, limits: Limits) -> Result<Report, InputError> {
    if let Some(id) = policy.unsupported_selector() {
        return Err(InputError::UnsupportedSelector(id));
    }
    let count = policy
        .user_rules
        .len()
        .saturating_add(policy.preset_rules.len())
        .saturating_add(policy.lan.len())
        .saturating_add(policy.private_networks.len())
        .saturating_add(policy.private_domains.len());
    if count > limits.max_rules.min(512) {
        return Err(InputError::TooLarge);
    }
    let mut ids = BTreeSet::new();
    let mut path_bytes = 0usize;
    for rule in policy.user_rules.iter().chain(&policy.preset_rules) {
        if rule.id == 0 {
            return Err(InputError::InvalidId);
        }
        if !ids.insert(rule.id) {
            return Err(InputError::DuplicateId);
        }
        let c = &rule.conditions;
        if c.executables.len() > 32 || c.destinations.len() > 32 || c.domains.len() > 32 {
            return Err(InputError::TooLarge);
        }
        for path in &c.executables {
            path_bytes = path_bytes.saturating_add(path.len());
            if path_bytes > 1_048_576 {
                return Err(InputError::TooLarge);
            }
            if path.len() > 4096 {
                return Err(InputError::TooLarge);
            }
            if !path.starts_with('/') || path.chars().any(char::is_control) {
                return Err(InputError::InvalidExecutable);
            }
        }
    }
    let mut earlier = Vec::with_capacity(count);
    for network in &policy.private_networks {
        earlier.push((
            RuleOrigin::PrivateNetwork,
            None,
            Conditions {
                destinations: vec![network.network],
                ..Conditions::default()
            },
        ));
    }
    for domain in &policy.private_domains {
        earlier.push((
            RuleOrigin::PrivateDomain,
            None,
            Conditions {
                domains: vec![domain.domain.clone()],
                ..Conditions::default()
            },
        ));
    }
    for network in &policy.lan {
        earlier.push((
            RuleOrigin::Lan,
            Some(Action::Direct),
            Conditions {
                destinations: vec![*network],
                ..Conditions::default()
            },
        ));
    }
    let mut report = Report::default();
    for (rules, origin) in [
        (
            &policy.user_rules,
            RuleOrigin::User as fn(u64) -> RuleOrigin,
        ),
        (&policy.preset_rules, RuleOrigin::Preset),
    ] {
        for rule in rules.iter().filter(|r| r.enabled) {
            for (previous, action, conditions) in &earlier {
                if report.comparisons >= limits.max_comparisons.min(32_768) {
                    report.truncated = true;
                    return Ok(report);
                }
                let c = &rule.conditions;
                // Верхняя оценка двух проходов OR-списков с учётом длины строк.
                let cost = 1 + 2
                    * (conditions.executables.len() * c.executables.len() * 4096
                        + conditions.domains.len() * c.domains.len() * 253
                        + conditions.destinations.len() * c.destinations.len() * 128);
                if report.work_units + cost > 16_777_216 {
                    report.truncated = true;
                    return Ok(report);
                }
                report.work_units += cost;
                report.comparisons += 1;
                if disjoint(&conditions.executables, &c.executables, |a, b| a == b)
                    || disjoint(&conditions.domains, &c.domains, domain_overlap)
                {
                    continue;
                }
                // Разные CIDR могут совпасть на разных адресах одного DNS-ответа.
                let shadowed = action.is_some()
                    && subset(&conditions.executables, &c.executables, |a, b| a == b)
                    && subset(&conditions.domains, &c.domains, domain_covers)
                    && subset(&conditions.destinations, &c.destinations, cidr_covers);
                if report.warnings.len() >= limits.max_warnings.min(256) {
                    report.truncated = true;
                    return Ok(report);
                }
                report.warnings.push(Warning {
                    rule_id: rule.id,
                    earlier: *previous,
                    kind: if shadowed {
                        WarningKind::Shadowed
                    } else {
                        WarningKind::PotentialOverlap
                    },
                    earlier_action: *action,
                    action: rule.action,
                });
            }
            let c = &rule.conditions;
            earlier.push((
                origin(rule.id),
                Some(rule.action),
                Conditions {
                    platform_sources: vec![],
                    executables: c.executables.clone(),
                    destinations: c.destinations.clone(),
                    domains: c.domains.clone(),
                },
            ));
        }
    }
    Ok(report)
}
fn subset<T>(earlier: &[T], later: &[T], covers: impl Fn(&T, &T) -> bool) -> bool {
    earlier.is_empty()
        || !later.is_empty() && later.iter().all(|b| earlier.iter().any(|a| covers(a, b)))
}
fn disjoint<T>(a: &[T], b: &[T], overlaps: impl Fn(&T, &T) -> bool) -> bool {
    !a.is_empty() && !b.is_empty() && !a.iter().any(|x| b.iter().any(|y| overlaps(x, y)))
}
fn domain_covers(a: &DomainPattern, b: &DomainPattern) -> bool {
    match (a, b) {
        (DomainPattern::Exact(a), DomainPattern::Exact(b)) => a == b,
        (DomainPattern::Suffix(_), _) => a.matches(b.name()),
        _ => false,
    }
}
fn domain_overlap(a: &DomainPattern, b: &DomainPattern) -> bool {
    domain_covers(a, b) || domain_covers(b, a)
}
fn cidr_covers(a: &Cidr, b: &Cidr) -> bool {
    // Cidr предоставляет только contains; канонический Display ограничен 43 байтами.
    let text_a = a.to_string();
    let text_b = b.to_string();
    let (Some((_, prefix_a)), Some((address_b, prefix_b))) =
        (text_a.split_once('/'), text_b.split_once('/'))
    else {
        return false;
    };
    match (
        prefix_a.parse::<u8>(),
        prefix_b.parse::<u8>(),
        address_b.parse(),
    ) {
        (Ok(a_prefix), Ok(b_prefix), Ok(address)) => a_prefix <= b_prefix && a.contains(address),
        _ => false,
    }
}
