//! Explicit RED harness: exit 42 только при exact Unsupported после validity controls.
//! Не запускает native cores, не создаёт activation/process-owner authority.
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::{self, File},
    io::{self, Read},
    net::IpAddr,
    os::unix::fs::DirBuilderExt,
    path::Path,
    process::ExitCode,
};
use yonik_engine::{
    DnsContract, Engine, PolicyCapability, PolicyCompileError, PolicyMode, resource_bundle::*,
};
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

fn run(path: &Path, negative: Option<&str>) -> Result<()> {
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
        println!(
            "VALID {label} source digest/count, policy digest, complete bindings, membership/miss, sealed bytes+digest+readonly"
        );
        for app in [false, true] {
            let policy = policy(app);
            let identity = policy_identity(&policy);
            let bound = catalog.bind(&policy, 1, Sha256::digest(&identity).into(), edges(2))?;
            for mode in [PolicyMode::Names, PolicyMode::IpOnly] {
                let compile = |revision, probe| {
                    compile_bound_policy(
                        &server,
                        engine,
                        10808.try_into().expect("Фиксированный порт"),
                        probe,
                        &bound,
                        revision,
                        &identity,
                        mode,
                        &DnsContract::default(),
                    )
                };
                require(
                    matches!(
                        compile(2, 10809.try_into()?),
                        Err(PolicyCompileError::InvalidPolicy)
                    ),
                    "Wrong revision дал не InvalidPolicy",
                )?;
                require(
                    matches!(
                        compile(1, 10808.try_into()?),
                        Err(PolicyCompileError::ProbePort)
                    ),
                    "Одинаковые порты дали не ProbePort",
                )?;
                let expected = if app {
                    PolicyCapability::ProcessAttribution
                } else if mode == PolicyMode::Names {
                    PolicyCapability::RequiredRealResolution
                } else {
                    PolicyCapability::DomainsInIpOnly
                };
                // Fault injection проверяет, что другая ошибка не маскируется под RED.
                let revision = if negative == Some("--invalid-revision") {
                    2
                } else {
                    1
                };
                let probe = if negative == Some("--invalid-port") {
                    10808
                } else {
                    10809
                };
                match compile(revision, probe.try_into()?) {
                    Err(PolicyCompileError::Unsupported(actual)) if actual == expected => {
                        println!(
                            "RED {label} app={app} mode={mode:?} error=Unsupported({actual:?})"
                        );
                    }
                    Err(other) => return Err(format!("Неверная причина RED: {other:?}").into()),
                    Ok(_) => {
                        return Err("Mixed compile неожиданно успешен: пересмотреть RED".into());
                    }
                }
            }
        }
        drop(sealed);
        drop(lease);
        drop(catalog);
        drop(store);
        // Только собственные synthetic files; ни worker, ни runtime не запускались.
        for entry in fs::read_dir(&disk)? {
            fs::remove_file(entry?.path())?;
        }
        fs::remove_dir(disk)?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1).map(std::path::PathBuf::from) else {
        eprintln!("Нужен новый собственный fixture directory");
        return ExitCode::FAILURE;
    };
    let negative = std::env::args().nth(2);
    if !matches!(
        negative.as_deref(),
        None | Some("--invalid-revision" | "--invalid-port")
    ) {
        eprintln!("Неизвестный negative control");
        return ExitCode::FAILURE;
    }
    if let Err(error) = fs::DirBuilder::new().mode(0o700).create(&path) {
        eprintln!("Не создан fixture directory: {error}");
        return ExitCode::FAILURE;
    }
    match run(&path, negative.as_deref()).and_then(|()| {
        fs::remove_dir(&path)?;
        Ok(())
    }) {
        Ok(()) => {
            eprintln!(
                "RED_CONFIRMED: exact Unsupported на валидном mixed compile; НЕ positive PASS"
            );
            ExitCode::from(42)
        }
        Err(error) => {
            eprintln!(
                "FIXTURE_INVALID_OR_CHANGED: {error}; сохранено {}",
                path.display()
            );
            ExitCode::FAILURE
        }
    }
}
