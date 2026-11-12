//! Offline P-04: настоящий импорт и компиляция, без запуска engine или TUN.
use std::{
    io::{self, Read},
    num::NonZeroU16,
    time::Instant,
};
use yonik_domain::{Action, Conditions, Policy, Rule};
use yonik_engine::{Engine, SystemTun, compile, compile_system_policy};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        return Err("Ожидаются count rules engine".into());
    }
    let count: usize = args[1].parse()?;
    let rules: usize = args[2].parse()?;
    if ![1000, 10000].contains(&count) || ![10, 100, 1000].contains(&rules) {
        return Err("Нагрузка вне границ".into());
    }
    let engine = match args[3].as_str() {
        "xray" => Engine::Xray,
        "sing-box" => Engine::SingBox,
        _ => return Err("Неизвестное ядро".into()),
    };
    let mut input = Vec::new();
    io::stdin()
        .take((yonik_import::MAX_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut input)?;
    let started = Instant::now();
    let report = yonik_import::import_bytes(input)?;
    if report.servers.len() != count || !report.issues.is_empty() {
        return Err("Неполный импорт".into());
    }
    let import_ns = started.elapsed().as_nanos();
    let start = Instant::now();
    let port = NonZeroU16::new(10808).ok_or("port")?;
    let mut bytes = Vec::new();
    let mut total = 0usize;
    for server in &report.servers {
        bytes.clear();
        compile(server, engine, port)?.write_sensitive(&mut bytes)?;
        total += bytes.len();
    }
    let compile_ns = start.elapsed().as_nanos();
    let policy = Policy {
        user_rules: (0..rules)
            .map(|i| Rule {
                id: i as u64,
                enabled: true,
                conditions: Conditions {
                    platform_sources: vec![],
                    executables: vec![format!("/opt/fixture/app-{i}")],
                    destinations: vec!["203.0.113.0/24".parse().expect("Фиксированный CIDR")],
                    domains: vec![],
                },
                action: Action::Block,
            })
            .collect(),
        ..Default::default()
    };
    let tun = SystemTun::new(1, 1500, 99, vec!["198.18.0.1/30".parse()?])?;
    let start = Instant::now();
    let config = compile_system_policy(
        &report.servers[0],
        engine,
        port,
        NonZeroU16::new(10809).ok_or("port")?,
        &policy,
        &tun,
    )?;
    bytes.clear();
    config.write_sensitive(&mut bytes)?;
    let policy_ns = start.elapsed().as_nanos();
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let (native, key) = match engine {
        Engine::Xray => (&value["routing"]["rules"], "process"),
        Engine::SingBox => (&value["route"]["rules"], "process_path"),
    };
    let native = native.as_array().ok_or("Нет rules")?;
    let app: Vec<_> = native.iter().filter(|r| r.get(key).is_some()).collect();
    if app.len() != rules {
        return Err("Потеря app-rules".into());
    }
    for (i, rule) in app.iter().enumerate() {
        if rule[key] != serde_json::json!([format!("/opt/fixture/app-{i}")]) {
            return Err("Изменён executable".into());
        }
        let valid = match engine {
            Engine::Xray => {
                rule["ip"] == serde_json::json!(["203.0.113.0/24"])
                    && rule["outboundTag"] == "block"
            }
            Engine::SingBox => {
                rule["ip_cidr"] == serde_json::json!(["203.0.113.0/24"])
                    && rule["action"] == "reject"
            }
        };
        if !valid {
            return Err("Изменена семантика AND/action".into());
        }
    }
    // Доменный AND для системного IP-входа обязан отказать, а не исчезнуть.
    let mut mixed = Policy::default();
    mixed.user_rules.push(Rule {
        id: 1,
        enabled: true,
        conditions: Conditions {
            platform_sources: vec![],
            executables: vec!["/opt/fixture/app-0".into()],
            domains: vec![yonik_domain::DomainPattern::Exact(
                "example.invalid".parse()?,
            )],
            destinations: vec!["203.0.113.0/24".parse()?],
        },
        action: Action::Block,
    });
    let mixed_refused = compile_system_policy(
        &report.servers[0],
        engine,
        port,
        NonZeroU16::new(10809).ok_or("port")?,
        &mixed,
        &tun,
    )
    .is_err();
    if !mixed_refused {
        return Err("Доменное условие не отклонено".into());
    }
    println!(
        "{}",
        serde_json::json!({"imported":count,"compiled":count,"import_ns":import_ns,"compile_ns":compile_ns,"config_total_bytes":total,"policy_ns":policy_ns,"policy_bytes":bytes.len(),"app_rules":app.len(),"native_rules":native.len(),"mixed_domain_refused":mixed_refused})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_rule_set_is_explicitly_refused() -> Result<(), Box<dyn std::error::Error>> {
        let report =
            yonik_import::import_bytes(b"trojan://fixture@server.example.invalid:443".to_vec())?;
        let policy = Policy {
            user_rules: vec![Rule {
                id: 1,
                enabled: true,
                conditions: Conditions {
                    platform_sources: vec![],
                    executables: vec![],
                    domains: vec![
                        yonik_domain::DomainPattern::Exact("example.invalid".parse()?);
                        100_000
                    ],
                    destinations: vec!["203.0.113.0/24".parse()?; 50_000],
                },
                action: Action::Block,
            }],
            ..Default::default()
        };
        for engine in [Engine::Xray, Engine::SingBox] {
            let result = yonik_engine::compile_policy(
                &report.servers[0],
                engine,
                NonZeroU16::new(10808).ok_or("port")?,
                NonZeroU16::new(10809).ok_or("port")?,
                &policy,
                yonik_engine::PolicyMode::IpOnly,
            );
            assert!(matches!(
                result,
                Err(yonik_engine::PolicyCompileError::InvalidPolicy)
            ));
        }
        Ok(())
    }
}
