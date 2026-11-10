//! Экспортирует только закрытый набор метаданных, никогда не исходный snapshot.
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Value, json};
use std::fmt;

struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Strict;
        impl<'de> Visitor<'de> for Strict {
            type Value = Unique;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Unique, M::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom("повторный ключ"));
                    }
                    values.insert(key, map.next_value::<Unique>()?.0);
                }
                Ok(Unique(Value::Object(values)))
            }
            fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Unique, S::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<Unique>()? {
                    values.push(value.0);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Unique, E> {
                Ok(Unique(Value::String(value.into())))
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Unique, E> {
                Ok(Unique(Value::Bool(value)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Unique, E> {
                serde_json::Number::from_f64(value)
                    .map(|n| Unique(Value::Number(n)))
                    .ok_or_else(|| de::Error::custom("число"))
            }
        }
        deserializer.deserialize_any(Strict)
    }
}

fn bounded(raw: &str) -> bool {
    let (mut depth, mut quoted, mut escaped) = (0_u32, false, false);
    for byte in raw.bytes() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else if byte == b'"' {
            quoted = true;
        } else if byte == b'[' || byte == b'{' {
            depth += 1;
            if depth > 64 {
                return false;
            }
        } else if byte == b']' || byte == b'}' {
            let Some(next) = depth.checked_sub(1) else {
                return false;
            };
            depth = next;
        }
    }
    depth == 0 && !quoted
}

/// Отказ не содержит входных данных. Свободные поля snapshot не копируются.
pub fn export_snapshot(raw: &str) -> Option<String> {
    if raw.len() > 1_048_576 || !bounded(raw) {
        return None;
    }
    let snapshot = serde_json::from_str::<Unique>(raw).ok()?.0;
    let object = snapshot.as_object()?;
    let catalog: Value = serde_json::from_str(include_str!(
        "../../../docs/specs/diagnostic-message-catalog.json"
    ))
    .ok()?;
    let mut errors = Vec::new();
    let routing_error = match object.get("routing_ack") {
        None | Some(Value::Null) => None,
        Some(Value::Object(ack)) => ack.get("error"),
        _ => return None,
    };
    for value in [
        object.get("error"),
        object.get("connection_error"),
        object.get("source_error"),
        routing_error,
    ] {
        let Some(value) = value else {
            continue;
        };
        if value.is_null() || value.as_str() == Some("") {
            continue;
        }
        let message_key = value.as_str()?;
        let entry = catalog["messages"]
            .as_array()?
            .iter()
            .find(|entry| entry["message_key"].as_str() == Some(message_key))?;
        let code = entry["code"].as_str()?;
        let error = json!({"code":code,"message_key":message_key,"stage":"operation","retryable":false,"params":{}});
        if !errors.contains(&error) {
            errors.push(error);
        }
    }
    let output = json!({"schema_version":1,"platform":"linux","errors":errors}).to_string();
    (output.len() <= 65_536).then_some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_only_and_unknown_rejected() {
        let safe = export_snapshot(r#"{"error":"cancelled","connection_error":"cancelled","source":"https://secret:password@example.org","native":{"token":"CANARY"}}"#).expect("метаданные");
        let parsed: Value = serde_json::from_str(&safe).expect("JSON");
        assert_eq!(parsed["errors"].as_array().expect("ошибки").len(), 1);
        assert!(!safe.contains("CANARY"));
        assert!(!safe.contains("password"));
        for value in [
            "CANARY",
            "cancelled\u{1b}",
            "cleanup_failed\n",
            "https://secret",
        ] {
            assert!(export_snapshot(&json!({"error":value}).to_string()).is_none());
        }
        assert!(export_snapshot(r#"{"error":{"code":"cancelled"}}"#).is_none());
        for control in (0..=0x1f).chain(0x7f..=0x9f) {
            let value = format!("cancelled{}", char::from_u32(control).expect("символ"));
            assert!(export_snapshot(&json!({"error":value}).to_string()).is_none());
        }
        let at_limit = format!("{{}}{}", " ".repeat(1_048_574));
        assert!(export_snapshot(&at_limit).is_some());
        let nested = format!("{}0{}", "[".repeat(63), "]".repeat(63));
        assert!(export_snapshot(&format!("{{\"native\":{nested}}}")).is_some());
        assert!(export_snapshot("[]").is_none());
        assert!(export_snapshot(&" ".repeat(1_048_577)).is_none());
    }
    #[test]
    fn duplicate_and_depth_bounds() {
        for raw in [
            r#"{"error":"cancelled","error":"cleanup_failed"}"#,
            r#"{"native":{"a":1,"a":2}}"#,
            r#"{"error":true}"#,
        ] {
            assert!(export_snapshot(raw).is_none());
        }
        assert!(export_snapshot(&format!("{}0{}", "[".repeat(65), "]".repeat(65))).is_none());
        assert!(export_snapshot(r#"{"native":"[\"{}]","error":"storage"}"#).is_some());
    }

    #[test]
    fn routing_error_obeys_the_same_allowlist() {
        assert!(export_snapshot(r#"{"routing_ack":{"error":"CANARY"}}"#).is_none());
        let safe = export_snapshot(r#"{"routing_ack":{"error":"cancelled"}}"#).expect("метаданные");
        assert!(safe.contains("CANCELLED"));
    }
    #[test]
    fn validator_parity() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let catalog: Value = serde_json::from_str(include_str!(
            "../../../docs/specs/diagnostic-message-catalog.json"
        ))
        .expect("каталог");
        for entry in catalog["messages"].as_array().expect("сообщения") {
            let key = entry["message_key"].as_str().expect("ключ");
            let safe = export_snapshot(&json!({"error":key}).to_string()).expect("метаданные");
            let mut child = Command::new("python3")
                .arg("../../tools/check_diagnostic_export.py")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()
                .expect("валидатор");
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(safe.as_bytes())
                .expect("запись");
            assert!(child.wait().expect("статус").success());
        }
    }
}
