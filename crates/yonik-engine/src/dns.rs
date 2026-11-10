//! Граница DNS capability до генерации native-конфигурации.
use crate::{
    CompiledConfig, Engine, PolicyCapability, PolicyCompileError, PolicyMode, compile_policy,
};
use std::num::NonZeroU16;
use yonik_domain::{DomainPattern, Policy};
use yonik_import::ParsedServer;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DnsMode {
    /// DNS принадлежит вызывающему host; native resolver не создаётся.
    #[default]
    External,
    Real,
    FakeIp,
}

/// Запрошенная семантика, а не подтверждение DNS capability ядра.
/// External с обоими семействами не обещает full-family lookup: DNS вне compiler.
#[derive(Debug)]
pub struct DnsContract {
    pub mode: DnsMode,
    pub fake_exclusions: Vec<DomainPattern>,
    pub ipv4: bool,
    pub ipv6: bool,
}
impl Default for DnsContract {
    fn default() -> Self {
        Self {
            mode: DnsMode::External,
            fake_exclusions: Vec::new(),
            ipv4: true,
            ipv6: true,
        }
    }
}
impl DnsContract {
    pub fn validate(&self) -> Result<(), PolicyCompileError> {
        // Ни один pinned adapter пока не переносит full-family lease в dial
        // и не владеет Fake-IP mapping между поколениями. Не теряем запрос.
        if self.mode != DnsMode::External
            || !self.fake_exclusions.is_empty()
            || !self.ipv4
            || !self.ipv6
        {
            return Err(PolicyCompileError::Unsupported(
                PolicyCapability::DnsContract,
            ));
        }
        Ok(())
    }
}

/// Проверяет DNS до выдачи конфигурации и создания probe credentials.
#[allow(clippy::too_many_arguments)]
pub fn compile_policy_with_dns(
    server: &ParsedServer,
    engine: Engine,
    port: NonZeroU16,
    probe_port: NonZeroU16,
    policy: &Policy,
    mode: PolicyMode,
    dns: &DnsContract,
) -> Result<CompiledConfig, PolicyCompileError> {
    dns.validate()?;
    compile_policy(server, engine, port, probe_port, policy, mode)
}
