use sha2::{Digest, Sha256};
use std::{
    fmt::Write,
    fs::{self, File},
    io,
    os::unix::fs::DirBuilderExt,
    process::Command,
};
use yonik_engine::{DnsContract, Engine, PolicyMode, resource_bundle::*};
use yonik_linux::{
    resource_catalog::{ResourceCatalog, SourceResource, bundle_digest},
    rule_set_store::EncryptedRuleSetStore,
};
#[test]
#[ignore = "Нужны pinned cores и unprivileged user/net namespace"]
fn actual_150k_sealed_bundle_membership() {
    let path = std::env::temp_dir().join(format!("yonik-bundle-native-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
    let disk = path.join("encrypted");
    fs::DirBuilder::new().mode(0o700).create(&disk).unwrap();
    let mut domains = String::new();
    let mut cidrs = String::new();
    for i in 0..100_000 {
        writeln!(&mut domains, "exact:d{i}.example.org").unwrap();
    }
    for i in 0..50_000 {
        writeln!(&mut cidrs, "10.0.{}.{}/32", i / 256, i % 256).unwrap();
    }
    let server = yonik_import::import_bytes(b"socks://127.0.0.1:18081".to_vec())
        .unwrap()
        .servers
        .remove(0);
    let mut store = EncryptedRuleSetStore::open(File::open(&disk).unwrap(), [27; 32]).unwrap();
    for (engine, label) in [(Engine::Xray, "xray"), (Engine::SingBox, "sing")] {
        let catalog = ResourceCatalog::build(
            engine,
            &[
                SourceResource {
                    source: domains.as_bytes(),
                    digest: Sha256::digest(domains.as_bytes()).into(),
                    count: 100_000,
                    kind: ResourceKind::Domains,
                },
                SourceResource {
                    source: cidrs.as_bytes(),
                    digest: Sha256::digest(cidrs.as_bytes()).into(),
                    count: 50_000,
                    kind: ResourceKind::Cidrs,
                },
            ],
            &mut store,
        )
        .unwrap();
        for (index, kind, mode) in [
            (0, "domains", PolicyMode::Names),
            (1, "cidrs", PolicyMode::IpOnly),
        ] {
            let policy = Policy {
                user_rules: vec![Rule {
                    id: 7,
                    enabled: true,
                    conditions: if index == 0 {
                        Conditions {
                            domains: vec![DomainPattern::Exact(
                                "inline.example.org".parse().unwrap(),
                            )],
                            ..Conditions::default()
                        }
                    } else {
                        Conditions {
                            destinations: vec!["192.0.2.0/24".parse().unwrap()],
                            ..Conditions::default()
                        }
                    },
                    action: Action::Block,
                }],
                ..Policy::default()
            };
            let identity = policy_identity(&policy);
            // Каждый policy manifest содержит только используемый ресурс.
            let selected = Selected {
                catalog: &catalog,
                index,
                refs: vec![catalog.manifest()[index].clone()],
            };
            let bound = BoundPolicy::bind(
                &policy,
                1,
                &identity,
                &selected,
                vec![ResourceBinding {
                    rule: ResourceRule::User(7),
                    resource: 0,
                }],
            )
            .unwrap();
            let bundle = compile_bound_policy(
                &server,
                engine,
                10808.try_into().unwrap(),
                10809.try_into().unwrap(),
                &bound,
                1,
                &identity,
                mode,
                &DnsContract::default(),
            )
            .unwrap();
            let expected_digest = bundle_digest(&bundle).unwrap();
            store
                .commit(
                    store.generation(),
                    std::slice::from_ref(&catalog.references()[index]),
                )
                .unwrap();
            let sealed = store.lease_current().unwrap().seal_bundle(&bundle).unwrap();
            assert_eq!(sealed.digest, expected_digest);
            for resource in &sealed.resources {
                let mut file = File::create(path.join(&resource.name)).unwrap();
                io::copy(&mut &resource.descriptor, &mut file).unwrap();
                assert!(resource.descriptor.set_len(0).is_err());
            }
            bundle
                .config()
                .write_sensitive(File::create(path.join(format!("{label}-{kind}.json"))).unwrap())
                .unwrap();
        }
    }
    let result = Command::new("unshare")
        .args(["--user", "--map-root-user", "--net", "python3"])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/resource_membership.py"
        ))
        .arg(&path)
        .output()
        .unwrap();
    eprintln!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.status.success());
    drop(store);
    for e in fs::read_dir(&disk).unwrap() {
        fs::remove_file(e.unwrap().path()).unwrap();
    }
    fs::remove_dir(disk).unwrap();
    for e in fs::read_dir(&path).unwrap() {
        fs::remove_file(e.unwrap().path()).unwrap();
    }
    fs::remove_dir(path).unwrap();
}
struct Selected<'a> {
    catalog: &'a ResourceCatalog,
    index: usize,
    refs: Vec<ExternalRef>,
}
impl VerifiedCatalog for Selected<'_> {
    fn manifest(&self) -> &[ExternalRef] {
        &self.refs
    }
    fn matches_domain(&self, _: usize, name: &DomainName) -> bool {
        self.catalog.matches_domain(self.index, name)
    }
    fn matches_ip(&self, _: usize, ip: std::net::IpAddr) -> bool {
        self.catalog.matches_ip(self.index, ip)
    }
}
