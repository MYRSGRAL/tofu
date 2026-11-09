//! Переносимая модель yonik: без GUI, сети и обращения к ОС.

mod cidr;
mod domain;
mod evaluation;
mod policy;

pub use cidr::{Cidr, InvalidCidr};
pub use domain::{DomainName, DomainPattern, InvalidDomain};
pub use evaluation::{
    AnswerSet, Destination, DestinationFlow, DnsEpoch, Evaluation, EvaluationContext, FakeMapping,
    PolicyFault, PrivateDomain, Resolution, ResolverPath, VpnId,
};
pub use policy::{
    Action, Conditions, Decision, Flow, Policy, PrivateNetwork, Route, Rule, RuleOrigin,
};
