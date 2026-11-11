//! Закрытый metadata-only DTO нового PrepareBundleV1. FD transport проверяет caller отдельно.
use crate::{
    bundle_identity::{CompileRequirements, IntentBundle},
    resource_fd_receiver::CanonicalDescriptor,
};
use serde::{Deserialize, Serialize};
use std::io;
use yonik_engine::resource_bundle::{ExternalRef, ResourceBinding, ResourceKind, ResourceRule};
fn bad() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Некорректный PrepareBundleV1")
}
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Engine {
    Xray,
    #[serde(rename = "sing-box")]
    SingBox,
}
impl Engine {
    fn typed(self) -> yonik_engine::Engine {
        match self {
            Self::Xray => yonik_engine::Engine::Xray,
            Self::SingBox => yonik_engine::Engine::SingBox,
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Domains,
    Cidrs,
}
impl Kind {
    fn typed(self) -> ResourceKind {
        match self {
            Self::Domains => ResourceKind::Domains,
            Self::Cidrs => ResourceKind::Cidrs,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Canonical {
    digest: [u8; 32],
    bytes: u64,
    records: u64,
    kind: Kind,
    format_version: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Native {
    digest: [u8; 32],
    bytes: u64,
    records: u64,
    kind: Kind,
    format_version: u32,
    engine: Engine,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RuleKind {
    User,
    Preset,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    rule_kind: RuleKind,
    rule_id: u64,
    resource: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    IpOnly,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DnsRequirement {
    NoDomainProvenance,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Requirements {
    system_tun: bool,
    kill_switch: bool,
    mode: Mode,
    dns: DnsRequirement,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire<'a> {
    version: u32,
    engine: Engine,
    native_pin: [u8; 32],
    revision: u64,
    intent_digest: [u8; 32],
    requirements: Requirements,
    #[serde(borrow)]
    server: &'a serde_json::value::RawValue,
    #[serde(borrow)]
    policy: &'a serde_json::value::RawValue,
    bindings: Vec<Binding>,
    canonical: Vec<Canonical>,
    expected_native: Vec<Native>,
}
pub struct BundleRequest {
    pub server: yonik_import::ServerIntent,
    pub policy: yonik_engine::policy_wire::SystemPolicy,
    pub engine: yonik_engine::Engine,
    pub native_pin: [u8; 32],
    pub revision: u64,
    pub intent_digest: [u8; 32],
    pub requirements: CompileRequirements,
    pub bindings: Vec<ResourceBinding>,
    pub canonical: Vec<CanonicalDescriptor>,
    pub expected_native: Vec<ExternalRef>,
}
impl BundleRequest {
    /// Граница до чтения/копирования FD. Semantic SHA вычисляется helper сам;
    /// совпадение SHA не заменяет canonical grammar, binder и native rebuild.
    pub fn parse(json: &str, fd_count: usize) -> io::Result<Self> {
        if json.len() > 64 * 1024
            || !yonik_engine::policy_wire::bounded_depth(json, 8)
            || !json.trim_start().starts_with('{')
        {
            return Err(bad());
        }
        let wire: Wire<'_> = serde_json::from_str(json).map_err(|_| bad())?;
        if wire.version != 1
            || !wire.requirements.system_tun
            || fd_count == 0
            || fd_count > 16
            || wire.canonical.len() != fd_count
            || wire.expected_native.len() != fd_count
        {
            return Err(bad());
        }
        let request = Self {
            server: yonik_import::ServerIntent::parse_json(wire.server.get()).map_err(|_| bad())?,
            policy: yonik_engine::policy_wire::SystemPolicy::parse(wire.policy.get())
                .map_err(|_| bad())?,
            engine: wire.engine.typed(),
            native_pin: wire.native_pin,
            revision: wire.revision,
            intent_digest: wire.intent_digest,
            requirements: CompileRequirements {
                system_tun: wire.requirements.system_tun,
                kill_switch: wire.requirements.kill_switch,
            },
            bindings: wire
                .bindings
                .into_iter()
                .map(|b| ResourceBinding {
                    rule: match b.rule_kind {
                        RuleKind::User => ResourceRule::User(b.rule_id),
                        RuleKind::Preset => ResourceRule::Preset(b.rule_id),
                    },
                    resource: b.resource,
                })
                .collect(),
            canonical: wire
                .canonical
                .into_iter()
                .map(|d| CanonicalDescriptor {
                    digest: d.digest,
                    bytes: d.bytes,
                    records: d.records,
                    kind: d.kind.typed(),
                    format_version: d.format_version,
                })
                .collect(),
            expected_native: wire
                .expected_native
                .into_iter()
                .map(|d| ExternalRef {
                    digest: d.digest,
                    bytes: d.bytes,
                    records: d.records,
                    kind: d.kind.typed(),
                    format_version: d.format_version,
                    engine: d.engine.typed(),
                })
                .collect(),
        };
        if request.digest()? != request.intent_digest {
            return Err(bad());
        }
        Ok(request)
    }
    pub fn digest(&self) -> io::Result<[u8; 32]> {
        IntentBundle {
            server: &self.server,
            policy: &self.policy,
            bindings: &self.bindings,
            revision: self.revision,
            engine: self.engine,
            native_pin: self.native_pin,
            requirements: self.requirements,
            canonical: &self.canonical,
            expected_native: &self.expected_native,
        }
        .digest()
    }
}

impl BundleRequest {
    pub fn encode(&self) -> io::Result<String> {
        let kind = |k| match k {
            ResourceKind::Domains => Kind::Domains,
            ResourceKind::Cidrs => Kind::Cidrs,
        };
        let engine = |e| match e {
            yonik_engine::Engine::Xray => Engine::Xray,
            yonik_engine::Engine::SingBox => Engine::SingBox,
        };
        let canonical = self
            .canonical
            .iter()
            .map(|d| Canonical {
                digest: d.digest,
                bytes: d.bytes,
                records: d.records,
                kind: kind(d.kind),
                format_version: d.format_version,
            })
            .collect::<Vec<_>>();
        let native = self
            .expected_native
            .iter()
            .map(|d| Native {
                digest: d.digest,
                bytes: d.bytes,
                records: d.records,
                kind: kind(d.kind),
                format_version: d.format_version,
                engine: engine(d.engine),
            })
            .collect::<Vec<_>>();
        let bindings = self
            .bindings
            .iter()
            .map(|b| {
                let (rule_kind, rule_id) = match b.rule {
                    ResourceRule::User(id) => (RuleKind::User, id),
                    ResourceRule::Preset(id) => (RuleKind::Preset, id),
                };
                Binding {
                    rule_kind,
                    rule_id,
                    resource: b.resource,
                }
            })
            .collect::<Vec<_>>();
        let value = serde_json::json!({"version":1,"server":self.server,"policy":self.policy,"engine":engine(self.engine),"native_pin":self.native_pin,"revision":self.revision,"intent_digest":self.digest()?,"requirements":Requirements { system_tun:self.requirements.system_tun,kill_switch:self.requirements.kill_switch,mode:Mode::IpOnly,dns:DnsRequirement::NoDomainProvenance },"bindings":bindings,"canonical":canonical,"expected_native":native});
        let json = serde_json::to_string(&value).map_err(|_| bad())?;
        if json.len() > 64 * 1024 {
            return Err(bad());
        }
        Ok(json)
    }
}
