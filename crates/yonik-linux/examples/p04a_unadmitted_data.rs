//! Offline GREEN data artifact: не activation и не runtime proof.
//! Не запускает native cores, не создаёт activation/process-owner authority.
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    net::IpAddr,
    os::unix::fs::DirBuilderExt,
    path::Path,
};
use yonik_engine::{DnsContract, Engine, PolicyMode, resource_bundle::*};
use yonik_linux::{
    resource_catalog::{ResourceCatalog, SourceResource, bundle_digest},
    rule_set_store::EncryptedRuleSetStore,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn require(ok: bool, message: &str) -> Result<()> {
    if !ok {
        return Err(io::Error::other(message).into());
    }
    Ok(())
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
    fn matches_ip(&self, _: usize, ip: IpAddr) -> bool {
        self.catalog.matches_ip(self.index, ip)
    }
}

fn policy(app: bool) -> Policy {
    Policy {
        default_action: Action::Direct,
        user_rules: vec![Rule {
            id: 7,
            enabled: true,
            action: Action::Block,
            conditions: Conditions {
                executables: if app {
                    vec!["/opt/fixture/app-0".into()]
                } else {
                    vec![]
                },
                ..Conditions::default()
            },
        }],
        ..Policy::default()
    }
}

fn edges(count: usize) -> Vec<ResourceBinding> {
    (0..count)
        .map(|resource| ResourceBinding {
            rule: ResourceRule::User(7),
            resource,
        })
        .collect()
}

fn run(path: &Path) -> Result<()> {
    let domains = b"exact:a.example.invalid\n";
    let cidrs = b"203.0.113.0/24\n";
    let server = yonik_import::import_bytes(b"socks://127.0.0.1:18081".to_vec())?
        .servers
        .remove(0);
    for (engine, label) in [(Engine::Xray, "xray"), (Engine::SingBox, "sing-box")] {
        let disk = path.join(label);
        fs::DirBuilder::new().mode(0o700).create(&disk)?;
        // Ключ относится только к собственной синтетической offline БД.
        let mut store = EncryptedRuleSetStore::open(File::open(&disk)?, [27; 32])?;
        for (digest, count) in [([0; 32], 1), (Sha256::digest(domains).into(), 2)] {
            require(
                ResourceCatalog::build(
                    engine,
                    &[SourceResource {
                        source: domains,
                        digest,
                        count,
                        kind: ResourceKind::Domains,
                    }],
                    &mut store,
                )
                .is_err(),
                "Неверный source digest/count принят",
            )?;
        }
        require(
            fs::read_dir(&disk)?.count() == 0,
            "Публикация invalid source",
        )?;
        let catalog = ResourceCatalog::build(
            engine,
            &[
                SourceResource {
                    source: domains,
                    digest: Sha256::digest(domains).into(),
                    count: 1,
                    kind: ResourceKind::Domains,
                },
                SourceResource {
                    source: cidrs,
                    digest: Sha256::digest(cidrs).into(),
                    count: 1,
                    kind: ResourceKind::Cidrs,
                },
            ],
            &mut store,
        )?;
        require(
            catalog.matches_domain(0, &"a.example.invalid".parse()?)
                && !catalog.matches_domain(0, &"miss.example.invalid".parse()?)
                && catalog.matches_ip(1, "203.0.113.7".parse()?)
                && !catalog.matches_ip(1, "203.0.114.7".parse()?),
            "Невалидный индекс fixture",
        )?;
        let plain = policy(false);
        let identity = policy_identity(&plain);
        require(
            catalog.bind(&plain, 1, [0; 32], edges(2)).is_err(),
            "Неверный policy digest принят",
        )?;
        require(
            catalog
                .bind(&plain, 1, Sha256::digest(&identity).into(), edges(1))
                .is_err(),
            "Неполные bindings приняты",
        )?;
        // Реальные успешные compile + seal_bundle каждого компонента отдельно.
        for (index, mode) in [(0, PolicyMode::Names), (1, PolicyMode::IpOnly)] {
            let selected = Selected {
                catalog: &catalog,
                index,
                refs: vec![catalog.manifest()[index].clone()],
            };
            let bound = BoundPolicy::bind(&plain, 1, &identity, &selected, edges(1))?;
            let bundle = compile_bound_policy(
                &server,
                engine,
                10808.try_into()?,
                10809.try_into()?,
                &bound,
                1,
                &identity,
                mode,
                &DnsContract::default(),
            )?;
            store.commit(
                store.generation(),
                std::slice::from_ref(&catalog.references()[index]),
            )?;
            let sealed = store.lease_current()?.seal_bundle(&bundle)?;
            require(
                sealed.digest == bundle_digest(&bundle)?,
                "Неверный sealed bundle digest",
            )?;
            require(sealed.resources.len() == 1, "Потерян sealed component")?;
            println!("VALID {label} component={index} compile_bound_policy+seal_bundle");
        }
        store.commit(store.generation(), catalog.references())?;
        let lease = store.lease_current()?;
        let sealed = lease.sealed_resources()?;
        require(sealed.len() == 2, "Потеря mixed sealed resources")?;
        for (resource, reference) in sealed.iter().zip(catalog.manifest()) {
            let mut bytes = Vec::new();
            (&resource.descriptor).read_to_end(&mut bytes)?;
            require(
                bytes.len() as u64 == reference.bytes
                    && Sha256::digest(&bytes).as_slice() == reference.digest
                    && resource.descriptor.set_len(0).is_err(),
                "Невалидный sealed artifact",
            )?;
        }
        // Независимый контроль порядка: прежний ordered manifest не принимает
        // те же реальные sealed payloads в обратном порядке.
        let mut reordered_matches = true;
        for (resource, reference) in sealed.iter().rev().zip(catalog.manifest()) {
            (&resource.descriptor).seek(SeekFrom::Start(0))?;
            let mut bytes = Vec::new();
            (&resource.descriptor).read_to_end(&mut bytes)?;
            reordered_matches &= Sha256::digest(&bytes).as_slice() == reference.digest;
        }
        require(!reordered_matches, "Reordered resources не различимы")?;
        println!(
            "VALID {label} reordered sealed payloads rejected by original ordered digest sequence"
        );
        println!(
            "VALID {label} source digest/count, policy digest, complete bindings, membership/miss, sealed bytes+digest+readonly"
        );
        let mixed = policy(true);
        let identity = policy_identity(&mixed);
        let bound = catalog.bind(&mixed, 1, Sha256::digest(&identity).into(), edges(2))?;
        let data = compile_bound_data_cancellable(
            &server,
            engine,
            10808.try_into()?,
            10809.try_into()?,
            &bound,
            1,
            &identity,
            || true,
        )?;
        require(
            data.manifest() == catalog.manifest(),
            "Порядок manifest потерян",
        )?;
        let output = path.join(format!("{label}-data"));
        fs::DirBuilder::new().mode(0o700).create(&output)?;
        data.write_sensitive(File::create(output.join("config.json"))?)?;
        let manifest: Vec<_> = data
            .manifest()
            .iter()
            .map(|r| {
                serde_json::json!({
                    "tag":r.tag(),"bytes":r.bytes,"records":r.records,"kind":format!("{:?}",r.kind)
                })
            })
            .collect();
        fs::write(
            output.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        for resource in &sealed {
            (&resource.descriptor).seek(SeekFrom::Start(0))?;
            io::copy(
                &mut &resource.descriptor,
                &mut File::create(output.join(&resource.name))?,
            )?;
        }
        println!("GREEN {label} mixed compile-only artifact; activation не выполнялась");
    }
    Ok(())
}

fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .ok_or("Нужен новый собственный output directory")?;
    fs::DirBuilder::new().mode(0o700).create(&path)?;
    run(&path)
}
