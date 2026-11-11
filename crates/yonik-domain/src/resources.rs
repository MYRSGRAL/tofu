//! Дополнительные immutable selectors; порядок и DNS semantics остаются в Policy.
use crate::{DomainName, RuleOrigin};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceRule {
    User(u64),
    Preset(u64),
}
impl ResourceRule {
    pub fn origin(self) -> RuleOrigin {
        match self {
            Self::User(id) => RuleOrigin::User(id),
            Self::Preset(id) => RuleOrigin::Preset(id),
        }
    }
}
/// Host предоставляет только проверенный snapshot. OR внутри каждой категории,
/// AND между категориями и executable выполняет единственный evaluator Policy.
pub trait ResourceConditions {
    fn has_domains(&self, rule: ResourceRule) -> bool;
    fn has_cidrs(&self, rule: ResourceRule) -> bool;
    fn matches_domain(&self, rule: ResourceRule, name: &DomainName) -> bool;
    fn matches_ip(&self, rule: ResourceRule, address: IpAddr) -> bool;
}
pub(crate) struct EmptyResources;
impl ResourceConditions for EmptyResources {
    fn has_domains(&self, _: ResourceRule) -> bool {
        false
    }
    fn has_cidrs(&self, _: ResourceRule) -> bool {
        false
    }
    fn matches_domain(&self, _: ResourceRule, _: &DomainName) -> bool {
        false
    }
    fn matches_ip(&self, _: ResourceRule, _: IpAddr) -> bool {
        false
    }
}
