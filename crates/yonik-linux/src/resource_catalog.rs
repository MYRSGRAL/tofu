//! Trusted bridge: source hash → canonical index → native hash → encrypted store.
use crate::{
    rule_set_custody::{RuleSetFormat, RuleSetRef},
    rule_set_store::EncryptedRuleSetStore,
};
use sha2::{Digest, Sha256};
use std::{io, net::IpAddr};
use yonik_engine::{
    Engine,
    external_rules::{Format, Limits, compile_records},
    resource_bundle::{
        BoundPolicy, DomainName, ExternalRef, Policy, ResourceBinding, ResourceKind,
        VerifiedCatalog, policy_identity,
    },
    resource_index::RuleSetIndex,
};
use zeroize::Zeroizing;
pub struct ResourceCatalog {
    manifest: Vec<ExternalRef>,
    indices: Vec<RuleSetIndex>,
    references: Vec<RuleSetRef>,
    _reservation: crate::resource_fd_receiver::Reservation,
}
pub struct SourceResource<'a> {
    pub source: &'a [u8],
    pub digest: [u8; 32],
    pub count: u64,
    pub kind: ResourceKind,
}
fn bad() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Несовпадение source/policy digest",
    )
}
impl ResourceCatalog {
    pub fn build(
        engine: Engine,
        sources: &[SourceResource<'_>],
        store: &mut EncryptedRuleSetStore,
    ) -> io::Result<Self> {
        if sources.is_empty() || sources.len() > 16 {
            return Err(bad());
        }
        let mut manifest = Vec::new();
        let mut indices = Vec::new();
        let mut references = Vec::new();
        let mut bytes = 0u64;
        let mut records = 0u64;
        // Проверить все заявленные metadata/hash до первой публикации.
        for s in sources {
            bytes = bytes.checked_add(s.source.len() as u64).ok_or_else(bad)?;
            records = records.checked_add(s.count).ok_or_else(bad)?;
            if s.source.len() > 128 * 1024 * 1024
                || s.count == 0
                || s.count > 1_500_000
                || bytes > 512 * 1024 * 1024
                || records > 1_500_000
                || Sha256::digest(s.source).as_slice() != s.digest
            {
                return Err(bad());
            }
        }
        // Отдельный physical pool до индексов и whole-buffer artifacts. Dry encoder
        // считает фактический native размер без накопления native bytes.
        let mut physical = 0u64;
        for s in sources {
            let format = match (engine, s.kind) {
                (Engine::Xray, ResourceKind::Domains) => Format::XrayDomains,
                (Engine::Xray, ResourceKind::Cidrs) => Format::XrayCidrs,
                (_, ResourceKind::Domains) => Format::SingDomains,
                (_, ResourceKind::Cidrs) => Format::SingCidrs,
            };
            let mut size = CountWriter(0);
            compile_records(
                std::io::Cursor::new(s.source),
                &mut size,
                format,
                s.count,
                Limits::default(),
            )?;
            let allocation = (s.source.len() as u64)
                .checked_mul(3)
                .and_then(|n| n.checked_add(size.0.checked_mul(6)?))
                .and_then(|n| n.checked_add(s.count.checked_mul(128)?))
                .ok_or_else(bad)?;
            physical = physical.checked_add(allocation).ok_or_else(bad)?;
        }
        let reservation = crate::resource_fd_receiver::Reservation::acquire(physical)?;
        for s in sources {
            let index = RuleSetIndex::parse(s.source, s.kind, s.count)?;
            let (format, stored) = match (engine, s.kind) {
                (Engine::Xray, ResourceKind::Domains) => {
                    (Format::XrayDomains, RuleSetFormat::XrayDomainsV1)
                }
                (Engine::Xray, ResourceKind::Cidrs) => {
                    (Format::XrayCidrs, RuleSetFormat::XrayCidrsV1)
                }
                (_, ResourceKind::Domains) => (Format::SingDomains, RuleSetFormat::SingSourceV3),
                (_, ResourceKind::Cidrs) => (Format::SingCidrs, RuleSetFormat::SingSourceV3),
            };
            let mut artifact = Zeroizing::new(Vec::new());
            compile_records(
                std::io::Cursor::new(s.source),
                &mut *artifact,
                format,
                s.count,
                Limits::default(),
            )?;
            let digest = Sha256::digest(&*artifact).into();
            let reference = RuleSetRef::new(digest, artifact.len() as u64, s.count, stored)?;
            store.publish(&reference, &artifact)?;
            store.publish_canonical_reserved(&reference, s.source, s.kind, true)?;
            manifest.push(ExternalRef {
                engine,
                digest,
                bytes: reference.bytes(),
                records: s.count,
                kind: s.kind,
                format_version: if engine == Engine::Xray { 1 } else { 3 },
            });
            indices.push(index);
            references.push(reference);
        }
        Ok(Self {
            manifest,
            indices,
            references,
            _reservation: reservation,
        })
    }
    pub fn references(&self) -> &[RuleSetRef] {
        &self.references
    }
    pub fn bind<'a>(
        &'a self,
        policy: &'a Policy,
        revision: u64,
        expected_digest: [u8; 32],
        bindings: Vec<ResourceBinding>,
    ) -> io::Result<BoundPolicy<'a, Self>> {
        yonik_engine::resource_bundle::validate_policy_bounds(policy)?;
        let identity = policy_identity(policy);
        if Sha256::digest(&identity).as_slice() != expected_digest {
            return Err(bad());
        }
        BoundPolicy::bind(policy, revision, &identity, self, bindings)
    }
}
impl VerifiedCatalog for ResourceCatalog {
    fn manifest(&self) -> &[ExternalRef] {
        &self.manifest
    }
    fn matches_domain(&self, index: usize, name: &DomainName) -> bool {
        self.indices
            .get(index)
            .is_some_and(|i| i.matches_domain(name))
    }
    fn matches_ip(&self, index: usize, ip: IpAddr) -> bool {
        self.indices.get(index).is_some_and(|i| i.matches_ip(ip))
    }
}
/// Связывает config digest с порядком и полной metadata manifest, не раскрывая JSON.
pub fn bundle_digest(
    bundle: &yonik_engine::resource_bundle::CompiledResourceBundle,
) -> io::Result<[u8; 32]> {
    struct Writer {
        hash: Sha256,
        bytes: u64,
    }
    impl io::Write for Writer {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.bytes += b.len() as u64;
            if self.bytes > 4 * 1024 * 1024 {
                return Err(bad());
            }
            self.hash.update(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer {
        hash: Sha256::new(),
        bytes: 0,
    };
    bundle
        .config()
        .write_sensitive(&mut writer)
        .map_err(|_| bad())?;
    let config = writer.hash.finalize();
    let mut hash = Sha256::new();
    hash.update(b"yonik-bundle-v1");
    hash.update(config);
    hash.update(Sha256::digest(bundle.policy_identity()));
    hash.update(bundle.revision().to_be_bytes());
    for r in bundle.manifest() {
        r.validate()?;
        hash.update([if r.engine == Engine::Xray { 1 } else { 2 }]);
        hash.update(r.digest);
        hash.update(r.bytes.to_be_bytes());
        hash.update(r.records.to_be_bytes());
        hash.update([match r.kind {
            ResourceKind::Domains => 1,
            ResourceKind::Cidrs => 2,
        }]);
        hash.update(r.format_version.to_be_bytes());
    }
    Ok(hash.finalize().into())
}

struct CountWriter(u64);
impl std::io::Write for CountWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.checked_add(bytes.len() as u64).ok_or_else(bad)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::{self, File},
        os::unix::fs::DirBuilderExt,
    };
    #[test]
    fn source_hash_count_and_policy_digest_refuse_before_commit() {
        let path = std::env::temp_dir().join(format!("yonik-catalog-{}", std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        let mut store = EncryptedRuleSetStore::open(File::open(&path).unwrap(), [5; 32]).unwrap();
        let source = b"exact:private.example\n";
        assert!(
            ResourceCatalog::build(
                Engine::Xray,
                &[SourceResource {
                    source,
                    digest: [0; 32],
                    count: 1,
                    kind: ResourceKind::Domains
                }],
                &mut store
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        assert!(
            ResourceCatalog::build(
                Engine::Xray,
                &[SourceResource {
                    source,
                    digest: Sha256::digest(source).into(),
                    count: 2,
                    kind: ResourceKind::Domains
                }],
                &mut store
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        let catalog = ResourceCatalog::build(
            Engine::Xray,
            &[SourceResource {
                source,
                digest: Sha256::digest(source).into(),
                count: 1,
                kind: ResourceKind::Domains,
            }],
            &mut store,
        )
        .unwrap();
        let policy = Policy {
            user_rules: vec![yonik_engine::resource_bundle::Rule {
                id: 1,
                enabled: true,
                conditions: Default::default(),
                action: yonik_engine::resource_bundle::Action::Block,
            }],
            ..Policy::default()
        };
        let edge = || {
            vec![ResourceBinding {
                rule: yonik_engine::resource_bundle::ResourceRule::User(1),
                resource: 0,
            }]
        };
        assert!(catalog.bind(&policy, 1, [0; 32], edge()).is_err());
        assert!(
            catalog
                .bind(
                    &policy,
                    1,
                    Sha256::digest(policy_identity(&policy)).into(),
                    edge()
                )
                .is_ok()
        );
        assert_eq!(store.generation(), 0);
        drop(store);
        for e in fs::read_dir(&path).unwrap() {
            fs::remove_file(e.unwrap().path()).unwrap();
        }
        fs::remove_dir(path).unwrap();
    }
}
