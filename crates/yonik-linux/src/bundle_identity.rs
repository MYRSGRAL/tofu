//! Новые identities не меняют смысл старого CompiledResourceBundle digest.
use crate::resource_fd_receiver::CanonicalDescriptor;
use sha2::{Digest, Sha256};
use std::io;
use yonik_engine::resource_bundle::{ExternalRef, ResourceBinding, ResourceKind, ResourceRule};

fn bad() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "Некорректная bundle identity")
}
fn bytes(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}
fn native(hash: &mut Sha256, resource: &ExternalRef) -> io::Result<()> {
    resource.validate()?;
    hash.update([match resource.engine {
        yonik_engine::Engine::Xray => 1,
        yonik_engine::Engine::SingBox => 2,
    }]);
    hash.update(resource.digest);
    hash.update(resource.bytes.to_be_bytes());
    hash.update(resource.records.to_be_bytes());
    hash.update([match resource.kind {
        ResourceKind::Domains => 1,
        ResourceKind::Cidrs => 2,
    }]);
    hash.update(resource.format_version.to_be_bytes());
    Ok(())
}
/// Закрытые требования текущего SystemTUN slice: IpOnly, без domain provenance.
/// Это ожидаемые requirements, а не доказательства runtime capabilities.
#[derive(Clone, Copy)]
pub struct CompileRequirements {
    pub kill_switch: bool,
    pub system_tun: bool,
}
pub struct IntentBundle<'a> {
    pub server: &'a yonik_import::ServerIntent,
    pub policy: &'a yonik_engine::policy_wire::SystemPolicy,
    pub bindings: &'a [ResourceBinding],
    pub revision: u64,
    pub engine: yonik_engine::Engine,
    pub native_pin: [u8; 32],
    pub requirements: CompileRequirements,
    pub canonical: &'a [CanonicalDescriptor],
    pub expected_native: &'a [ExternalRef],
}
impl IntentBundle<'_> {
    pub fn digest(&self) -> io::Result<[u8; 32]> {
        if self.revision == 0
            || self.canonical.is_empty()
            || self.canonical.len() > 16
            || self.canonical.len() != self.expected_native.len()
            || self.bindings.len() > 128 * 16
        {
            return Err(bad());
        }
        let mut hash = Sha256::new();
        hash.update(b"yonik-intent-bundle-v1\0");
        let server = zeroize::Zeroizing::new(serde_json::to_vec(self.server).map_err(|_| bad())?);
        bytes(&mut hash, &server);
        bytes(
            &mut hash,
            &yonik_engine::resource_bundle::policy_identity(self.policy.policy()),
        );
        hash.update(self.revision.to_be_bytes());
        hash.update([match self.engine {
            yonik_engine::Engine::Xray => 1,
            yonik_engine::Engine::SingBox => 2,
        }]);
        hash.update(self.native_pin);
        hash.update([
            u8::from(self.requirements.kill_switch),
            u8::from(self.requirements.system_tun),
        ]);
        bytes(&mut hash, b"ip-only/no-domain-provenance");
        hash.update((self.bindings.len() as u64).to_be_bytes());
        for b in self.bindings {
            if b.resource >= self.canonical.len() {
                return Err(bad());
            }
            let (kind, id) = match b.rule {
                ResourceRule::User(id) => (1, id),
                ResourceRule::Preset(id) => (2, id),
            };
            hash.update([kind]);
            hash.update(id.to_be_bytes());
            hash.update((b.resource as u64).to_be_bytes());
        }
        hash.update((self.canonical.len() as u64).to_be_bytes());
        for (source, artifact) in self.canonical.iter().zip(self.expected_native) {
            if source.format_version != 1
                || source.bytes == 0
                || source.bytes > 128 * 1024 * 1024
                || source.records == 0
                || source.records > 1_500_000
                || source.records != artifact.records
                || source.kind != artifact.kind
                || artifact.engine != self.engine
            {
                return Err(bad());
            }
            hash.update(source.digest);
            hash.update(source.bytes.to_be_bytes());
            hash.update(source.records.to_be_bytes());
            hash.update([match source.kind {
                ResourceKind::Domains => 1,
                ResourceKind::Cidrs => 2,
            }]);
            hash.update(source.format_version.to_be_bytes());
            native(&mut hash, artifact)?;
        }
        Ok(hash.finalize().into())
    }
}
/// context_digest вычисляет helper из удерживаемых owner/epoch/network-plan и
/// private runtime context. Этот API не принимает caller ports или credentials.
pub fn runtime_bundle_digest(
    intent: [u8; 32],
    config: [u8; 32],
    manifest: &[ExternalRef],
    context_digest: [u8; 32],
) -> io::Result<[u8; 32]> {
    if manifest.is_empty() || manifest.len() > 16 {
        return Err(bad());
    }
    let mut hash = Sha256::new();
    hash.update(b"yonik-runtime-bundle-v1\0");
    hash.update(intent);
    hash.update(config);
    hash.update(context_digest);
    hash.update((manifest.len() as u64).to_be_bytes());
    for artifact in manifest {
        native(&mut hash, artifact)?;
    }
    Ok(hash.finalize().into())
}
