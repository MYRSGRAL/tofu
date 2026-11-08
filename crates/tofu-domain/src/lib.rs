//! Переносимая модель tofu: без GUI, сети и обращения к ОС.

mod cidr;
mod policy;

pub use cidr::{Cidr, InvalidCidr};
pub use policy::{Action, Conditions, Decision, Flow, Policy, PrivateNetwork, Rule, RuleOrigin};
