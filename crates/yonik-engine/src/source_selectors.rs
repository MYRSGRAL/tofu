//! Только существующее точное executable-условие, до публикации конфигурации.
use crate::{Engine, PolicyCompileError};
use serde_json::{Value, json};
use yonik_domain::Policy;

pub(crate) fn validate(policy: &Policy) -> Result<(), PolicyCompileError> {
    if let Some(id) = policy.unsupported_selector() {
        return Err(PolicyCompileError::UnsupportedSelector(id));
    }
    let mut bytes = 0usize;
    for rule in policy.user_rules.iter().chain(&policy.preset_rules) {
        if rule.conditions.executables.len() > 32 {
            return Err(PolicyCompileError::InvalidPolicy);
        }
        for path in &rule.conditions.executables {
            bytes = bytes.saturating_add(path.len());
            // Xray трактует завершающий slash как каталог и xray/ как special token.
            if path.len() > 4096
                || !path.starts_with('/')
                || path.ends_with('/')
                || path.chars().any(char::is_control)
                || path
                    .split('/')
                    .skip(1)
                    .any(|part| part.is_empty() || part == "." || part == "..")
                || bytes > 1_048_576
            {
                return Err(PolicyCompileError::InvalidPolicy);
            }
        }
    }
    Ok(())
}

pub(crate) fn apply(engine: Engine, rule: &mut Value, paths: &[String]) {
    if !paths.is_empty() {
        let key = match engine {
            Engine::Xray => "process",
            Engine::SingBox => "process_path",
        };
        rule[key] = json!(paths);
        // Локальный SOCKS не должен получить process semantics системного TUN.
        match engine {
            Engine::Xray => rule["inboundTag"] = json!(["system-tun"]),
            Engine::SingBox => rule["inbound"] = json!(["system-tun"]),
        }
    }
}
