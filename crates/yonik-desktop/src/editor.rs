//! DTO редактора преобразуется в общую Policy; сетевых побочных эффектов нет.
use serde::{Deserialize, Serialize};
use yonik_domain::{Action, Conditions, DomainName, DomainPattern, Policy, Rule};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleInput {
    pub id: u64,
    pub enabled: bool,
    pub executables: Vec<String>,
    pub domains: Vec<String>,
    pub destinations: Vec<String>,
    pub action: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub version: u8,
    pub rules: Vec<RuleInput>,
    pub default_action: String,
}
impl Default for Draft {
    fn default() -> Self {
        Self {
            version: 1,
            rules: vec![],
            default_action: "proxy".into(),
        }
    }
}
fn action(value: &str) -> Result<Action, &'static str> {
    match value {
        "proxy" => Ok(Action::Proxy),
        "direct" => Ok(Action::Direct),
        "block" => Ok(Action::Block),
        _ => Err("action"),
    }
}
impl Draft {
    pub fn json(&self) -> String {
        serde_json::to_string(self).expect("DTO без fallible полей")
    }
    pub fn parse(raw: &str) -> Result<Self, &'static str> {
        if raw.len() > 262144 {
            return Err("limit");
        }
        let draft: Self = serde_json::from_str(raw).map_err(|_| "format")?;
        draft.policy()?;
        Ok(draft)
    }
    pub fn policy(&self) -> Result<Policy, &'static str> {
        if self.version != 1 || self.rules.len() > 256 {
            return Err("limit");
        }
        let bytes: usize = self
            .rules
            .iter()
            .flat_map(|r| {
                r.executables
                    .iter()
                    .chain(&r.domains)
                    .chain(&r.destinations)
            })
            .map(String::len)
            .sum();
        if bytes > 200_000 {
            return Err("limit");
        }
        let mut ids = std::collections::HashSet::new();
        let mut rules = Vec::with_capacity(self.rules.len());
        for input in &self.rules {
            if input.id == 0 || input.id > 9_007_199_254_740_991 || !ids.insert(input.id) {
                return Err("id");
            }
            if [&input.executables, &input.domains, &input.destinations]
                .iter()
                .any(|v| v.len() > 64 || v.iter().any(|s| s.len() > 4096))
            {
                return Err("limit");
            }
            if input.executables.iter().any(|s| {
                !s.starts_with('/')
                    || s.contains(['\0', '\n'])
                    || s.split('/').any(|c| c == ".." || c == ".")
            }) {
                return Err("executable");
            }
            let domains = input
                .domains
                .iter()
                .map(|s| {
                    let (suffix, name) = s
                        .strip_prefix("*.")
                        .map_or((false, s.as_str()), |s| (true, s));
                    let name: DomainName = name.parse().map_err(|_| "domain")?;
                    Ok(if suffix {
                        DomainPattern::Suffix(name)
                    } else {
                        DomainPattern::Exact(name)
                    })
                })
                .collect::<Result<Vec<_>, &'static str>>()?;
            let destinations = input
                .destinations
                .iter()
                .map(|s| {
                    let value = match s.parse::<std::net::IpAddr>() {
                        Ok(ip) => format!("{ip}/{}", if ip.is_ipv4() { 32 } else { 128 }),
                        Err(_) => s.clone(),
                    };
                    value.parse().map_err(|_| "cidr")
                })
                .collect::<Result<Vec<_>, _>>()?;
            rules.push(Rule {
                id: input.id,
                enabled: input.enabled,
                action: action(&input.action)?,
                conditions: Conditions {
                    executables: input.executables.clone(),
                    domains,
                    destinations,
                },
            });
        }
        Ok(Policy {
            user_rules: rules,
            default_action: action(&self.default_action)?,
            ..Policy::default()
        })
    }
    pub fn put(&mut self, input: RuleInput, confirm_broad: bool) -> Result<(), &'static str> {
        if input.executables.is_empty()
            && input.domains.is_empty()
            && input.destinations.is_empty()
            && !confirm_broad
        {
            return Err("broad");
        }
        let mut candidate = self.clone();
        if let Some(rule) = candidate.rules.iter_mut().find(|r| r.id == input.id) {
            *rule = input;
        } else {
            candidate.rules.push(input);
        }
        candidate.policy()?;
        *self = candidate;
        Ok(())
    }
}

/// Только запрос пользователя; PID остаётся диагностикой и не входит в RuleInput.
pub fn catalog(
    processes: bool,
    query: &str,
    offset: usize,
) -> Result<serde_json::Value, &'static str> {
    use yonik_linux::applications::{Catalog, Limits};
    if query.len() > 256 || offset > 4096 {
        return Err("limit");
    }
    let mut roots = vec![
        std::path::PathBuf::from("/usr/local/share/applications"),
        std::path::PathBuf::from("/usr/share/applications"),
    ];
    if let Some(home) = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        roots.insert(0, home.join("applications"));
    } else if let Some(home) = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        roots.insert(0, home.join(".local/share/applications"));
    }
    let catalog = Catalog::new(roots, "/proc".into(), "ru".into(), Limits::default())
        .map_err(|_| "catalog")?;
    let (items, incomplete) = if processes {
        let snapshot = catalog.process_snapshot().map_err(|_| "catalog")?;
        (snapshot.items.into_iter().map(|p| serde_json::json!({"name":p.name,"pid":p.pid,"executable":p.exe.and_then(|p| p.to_str().map(str::to_owned)), "unsupported":""})).collect::<Vec<_>>(), snapshot.incomplete)
    } else {
        let snapshot = catalog.desktop_snapshot().map_err(|_| "catalog")?;
        (
            snapshot
                .items
                .into_iter()
                .map(|p| {
                    let (path, unsupported) = match p.executable {
                        Ok(path) => (path.to_str().map(str::to_owned), String::new()),
                        Err(reason) => (None, format!("{reason:?}")),
                    };
                    serde_json::json!({"name":p.name,"executable":path,"unsupported":unsupported})
                })
                .collect::<Vec<_>>(),
            snapshot.incomplete,
        )
    };
    let query = query.to_lowercase();
    let filtered: Vec<_> = items
        .into_iter()
        .filter(|p| {
            p["name"]
                .as_str()
                .is_some_and(|name| name.to_lowercase().contains(&query))
        })
        .collect();
    Ok(
        serde_json::json!({"total":filtered.len(),"offset":offset,"incomplete":incomplete,"items":filtered.into_iter().skip(offset).take(32).collect::<Vec<_>>()}),
    )
}
