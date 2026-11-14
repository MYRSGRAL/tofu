//! Targeted mixed compile/cancel/recovery, без повторного import/snapshot baseline.
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    error::Error,
    fmt::Write as _,
    fs::{self, File},
    os::unix::fs::DirBuilderExt,
    path::Path,
    time::Instant,
};
use yonik_engine::{Engine, PolicyCompileError, resource_bundle::*};
use yonik_linux::{
    resource_catalog::{ResourceCatalog, SourceResource},
    rule_set_store::EncryptedRuleSetStore,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
fn rss() -> Result<u64> {
    let status = fs::read_to_string("/proc/self/status")?;
    Ok(status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .ok_or("Нет VmRSS")?
        .split_whitespace()
        .next()
        .ok_or("Нет RSS")?
        .parse()?)
}
fn make_policy(count: usize, round: usize) -> Policy {
    Policy {
        default_action: Action::Direct,
        user_rules: (0..count)
            .map(|i| Rule {
                id: i as u64,
                enabled: true,
                action: Action::Block,
                conditions: Conditions {
                    executables: vec![format!("/opt/fixture/app-{i}-r{round}")],
                    ..Conditions::default()
                },
            })
            .collect(),
        ..Policy::default()
    }
}
fn edges(count: usize) -> Vec<ResourceBinding> {
    (0..count)
        .flat_map(|i| {
            (0..2).map(move |resource| ResourceBinding {
                rule: ResourceRule::User(i as u64),
                resource,
            })
        })
        .collect()
}
fn rule_nodes(rule: &serde_json::Value) -> usize {
    1 + rule
        .get("rules")
        .and_then(|r| r.as_array())
        .map_or(0, |rules| rules.iter().map(rule_nodes).sum())
}
fn run(path: &Path, count: usize, engine: Engine) -> Result<serde_json::Value> {
    let mut domains = String::new();
    let mut cidrs = String::new();
    for i in 0..100_000 {
        writeln!(&mut domains, "exact:d{i}.example.invalid")?;
    }
    for i in 0..50_000 {
        writeln!(&mut cidrs, "10.0.{}.{}/32", i / 256, i % 256)?;
    }
    let server = yonik_import::import_bytes(b"socks://127.0.0.1:18081".to_vec())?
        .servers
        .remove(0);
    let mut store = EncryptedRuleSetStore::open(File::open(path)?, [27; 32])?;
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
    )?;
    let mut rounds = Vec::new();
    for round in 0..4 {
        let policy = make_policy(count, round);
        let identity = policy_identity(&policy);
        let bound = catalog.bind(&policy, 1, Sha256::digest(&identity).into(), edges(count))?;
        let calls = Cell::new(0usize);
        let started = Instant::now();
        let result = compile_bound_data_cancellable(
            &server,
            engine,
            10808.try_into()?,
            10809.try_into()?,
            &bound,
            1,
            &identity,
            || {
                calls.set(calls.get() + 1);
                true
            },
        );
        let elapsed = started.elapsed().as_nanos();
        let (status, bytes, native_rules, native_nodes) = match result {
            Ok(data) => {
                // Oracle после compile timing; его bounded allocations входят в child peak RSS.
                let mut bytes = Vec::new();
                data.write_sensitive(&mut bytes)?;
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                let rules = if engine == Engine::Xray {
                    &value["routing"]["rules"]
                } else {
                    &value["route"]["rules"]
                };
                let rules = rules.as_array().ok_or("Нет native rules")?;
                let nodes: usize = rules.iter().map(rule_nodes).sum();
                if rules.len() != count + 3 || nodes > count * 5 + 16 {
                    return Err("Неконтролируемое разрастание native rules".into());
                }
                ("COMPILED", bytes.len(), rules.len(), nodes)
            }
            Err(BoundCompileError::Policy(PolicyCompileError::InvalidPolicy))
                if count == 10_000 =>
            {
                ("REFUSED", 0, 0, 0)
            }
            Err(error) => return Err(error.into()),
        };
        let total = calls.get();
        let after_compile = rss()?;
        let mut cancels = Vec::new();
        for cancel_at in [1, 100, if status == "COMPILED" { total } else { 1000 }] {
            let calls = Cell::new(0usize);
            let requested = Cell::new(None);
            let started = Instant::now();
            let cancelled = compile_bound_data_cancellable(
                &server,
                engine,
                10808.try_into()?,
                10809.try_into()?,
                &bound,
                1,
                &identity,
                || {
                    calls.set(calls.get() + 1);
                    if calls.get() == cancel_at {
                        requested.set(Some(Instant::now()));
                        false
                    } else {
                        true
                    }
                },
            );
            if !matches!(cancelled, Err(BoundCompileError::Cancelled)) || calls.get() != cancel_at {
                return Err("Отмена не доказана".into());
            }
            cancels.push(
                serde_json::json!({"checkpoint":cancel_at,"ns":started.elapsed().as_nanos(),
                    "request_to_return_ns":requested.get().ok_or("Нет cancel request")?.elapsed().as_nanos()}),
            );
        }
        drop(bound);
        drop(policy);
        drop(identity);
        let recovery = make_policy(10, round);
        let identity = policy_identity(&recovery);
        let bound = catalog.bind(&recovery, 1, Sha256::digest(&identity).into(), edges(10))?;
        let recovered = compile_bound_data_cancellable(
            &server,
            engine,
            10808.try_into()?,
            10809.try_into()?,
            &bound,
            1,
            &identity,
            || true,
        )?;
        drop(recovered);
        drop(bound);
        drop(recovery);
        drop(identity);
        rounds.push(serde_json::json!({"round":round,"status":status,"compile_ns":elapsed,"bytes":bytes,"native_rules":native_rules,"native_nodes":native_nodes,"callback_count":total,"rss_after_compile_kib":after_compile,"rss_after_recovery_kib":rss()?,"cancel":cancels}));
    }
    let native_bytes: u64 = catalog.manifest().iter().map(|r| r.bytes).sum();
    drop(catalog);
    drop(store);
    for entry in fs::read_dir(path)? {
        fs::remove_file(entry?.path())?;
    }
    Ok(
        serde_json::json!({"rules":count,"domains":100000,"cidrs":50000,"native_resource_bytes":native_bytes,"rounds":rounds}),
    )
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("Нужны engine rules new-fixture-dir".into());
    }
    let engine = match args[1].as_str() {
        "xray" => Engine::Xray,
        "sing-box" => Engine::SingBox,
        _ => return Err("Неверный engine".into()),
    };
    let count: usize = args[2].parse()?;
    if ![10, 100, 1000, 10000].contains(&count) {
        return Err("Неверная нагрузка".into());
    }
    let path = Path::new(&args[3]);
    fs::DirBuilder::new().mode(0o700).create(path)?;
    let result = run(path, count, engine)?;
    fs::remove_dir(path)?;
    println!("{result}");
    Ok(())
}
