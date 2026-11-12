#[path = "error_context.rs"]
pub mod context;

// Экспортирует только закрытый набор метаданных, никогда не исходный snapshot.
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

/// V2 использует контекст producer; отсутствие контекста не заменяется догадкой.
pub fn export_snapshot_v2(raw: &str) -> Option<String> {
    use context::{Cause, ErrorContext, Slot};
    if raw.len() > 1_048_576 || !bounded(raw) {
        return None;
    }
    let snapshot = serde_json::from_str::<Unique>(raw).ok()?.0;
    let object = snapshot.as_object()?;
    let contexts = object.get("error_contexts")?.as_array()?;
    if contexts.len() > 16 {
        return None;
    }
    let mut active = Vec::new();
    for (slot, field) in [
        (Slot::Error, "error"),
        (Slot::Connection, "connection_error"),
        (Slot::Source, "source_error"),
    ] {
        match object.get(field) {
            None | Some(Value::Null) => (),
            Some(Value::String(key)) if key.is_empty() => (),
            Some(Value::String(key)) => active.push((slot, key.as_str(), None)),
            _ => return None,
        }
    }
    for (slot, field) in [
        (Slot::RoutingAck, "routing_ack"),
        (Slot::MaintenanceAck, "maintenance_ack"),
    ] {
        match object.get(field) {
            None | Some(Value::Null) => (),
            Some(Value::Object(ack)) => match ack.get("error") {
                None | Some(Value::Null) => (),
                Some(Value::String(key)) => {
                    active.push((slot, key.as_str(), Some(ack.get("request_id")?.as_u64()?)))
                }
                _ => return None,
            },
            _ => return None,
        }
    }
    match snapshot.get("logs") {
        None | Some(Value::Null) => (),
        Some(Value::Object(logs)) => {
            let key = match logs.get("status").and_then(Value::as_str) {
                Some("" | "ready" | "empty" | "saving" | "saved") => None,
                Some("busy") => Some("busy"),
                Some("refused") => Some("logs_save_failed"),
                Some("saved_durability_unknown") => Some("logs_save_durability_unknown"),
                Some("cancelled") => Some("cancelled"),
                _ => return None,
            };
            if let Some(key) = key {
                let slot: Slot =
                    serde_json::from_value(snapshot.get("logs_channel")?.clone()).ok()?;
                if !matches!(slot, Slot::LogsRead | Slot::LogsSave) {
                    return None;
                }
                let request_id = parse_decimal_id(logs.get("request_id")?)??;
                active.push((slot, key, Some(request_id)));
            }
        }
        _ => return None,
    }
    if active.len() != contexts.len() || active.len() > 16 {
        return None;
    }
    let mut seen = Vec::new();
    for value in contexts {
        let slot: Slot = serde_json::from_value(value.get("slot")?.clone()).ok()?;
        if seen.contains(&slot) {
            return None;
        }
        seen.push(slot);
        let reason: Cause = serde_json::from_value(value.get("reason")?.clone()).ok()?;
        let request_id = parse_decimal_id(value.get("request_id")?)?;
        let generation = parse_decimal_id(value.get("generation")?)?;
        if !reason.allows(slot) || (reason.needs_generation() && generation.is_none()) {
            return None;
        }
        let mut expected = ErrorContext::new(slot, reason, request_id);
        expected.generation = generation;
        if *value != serde_json::to_value(expected).ok()? {
            return None;
        }
        if !active.contains(&(slot, expected.message_key, request_id)) {
            return None;
        }
        if let Some(generation) = generation {
            let field = if matches!(
                expected.stage,
                context::Stage::CatalogRead | context::Stage::StorageWrite
            ) {
                "command_generation"
            } else if matches!(
                expected.stage,
                context::Stage::LogsRead | context::Stage::LogsSave
            ) {
                "logs_generation"
            } else if matches!(
                expected.stage,
                context::Stage::MaintenanceAdmission
                    | context::Stage::MaintenanceCopy
                    | context::Stage::MaintenanceDrain
                    | context::Stage::MaintenanceReopen
            ) {
                "maintenance_generation"
            } else if expected.stage == context::Stage::RoutingSave {
                "routing_generation"
            } else if matches!(
                expected.stage,
                context::Stage::SchedulerAdmission
                    | context::Stage::SchedulerService
                    | context::Stage::SchedulerValidation
            ) {
                "scheduler_generation"
            } else if expected.stage == context::Stage::HelperRpc {
                "helper_generation"
            } else {
                "error_generation"
            };
            if object.get(field)?.as_u64()? != generation {
                return None;
            }
        }
    }
    let output = json!({"schema_version":2,"platform":"linux","errors":contexts}).to_string();
    (output.len() <= 65_536).then_some(output)
}

fn parse_decimal_id(value: &Value) -> Option<Option<u64>> {
    if value.is_null() {
        return Some(None);
    }
    let text = value.as_str()?;
    let number = text.parse::<u64>().ok()?;
    (text.len() <= 20 && number.to_string() == text).then_some(Some(number))
}

#[cfg(test)]
mod v2_tests {
    use super::*;
    use context::{Cause, ErrorContext, Slot};
    #[test]
    fn typed_context_and_whole_document_rejection() {
        let context = serde_json::to_value(ErrorContext::new(
            Slot::Connection,
            Cause::DataDirectoryPermissions,
            None,
        ))
        .expect("контекст");
        let snapshot = json!({"connection_error":"storage", "error_contexts":[context], "native":"https://secret:CANARY@example.invalid/"});
        let safe = export_snapshot_v2(&snapshot.to_string()).expect("V2");
        assert!(!safe.contains("CANARY"));
        let metadata: Value = serde_json::from_str(&safe).expect("JSON");
        assert_eq!(
            metadata["errors"][0]["reason"],
            "data_directory_permissions"
        );
        for (field, value) in [
            ("reason", json!("storage")),
            ("params", json!({"url":"CANARY"})),
            ("code", json!("CANARY")),
            ("request_id", json!("01")),
            ("request_id", json!(1)),
            ("generation", json!("18446744073709551616")),
            ("extra", json!(true)),
        ] {
            let mut bad = snapshot.clone();
            bad["error_contexts"][0][field] = value;
            assert!(export_snapshot_v2(&bad.to_string()).is_none(), "{field}");
        }
        let mut stale = snapshot.clone();
        stale["connection_error"] = json!("busy");
        assert!(export_snapshot_v2(&stale.to_string()).is_none());
        stale["connection_error"] = Value::Null;
        assert!(export_snapshot_v2(&stale.to_string()).is_none());
        assert!(export_snapshot_v2(r#"{"error":"storage","error_contexts":[]}"#).is_none());
    }
    #[test]
    fn canonical_u64_ack_binding_and_generation() {
        let mut context = ErrorContext::new(
            Slot::RoutingAck,
            Cause::RoutingRepositoryFailed,
            Some(u64::MAX),
        );
        context.generation = Some(7);
        let mut snapshot = json!({"routing_ack":{"request_id":u64::MAX,"error":"routing_storage"},"routing_generation":7,"error_contexts":[context]});
        let safe = export_snapshot_v2(&snapshot.to_string()).expect("ACK");
        assert!(safe.contains("\"18446744073709551615\""));
        snapshot["routing_ack"]["request_id"] = json!(1);
        assert!(export_snapshot_v2(&snapshot.to_string()).is_none());
        snapshot["routing_ack"]["request_id"] = json!(u64::MAX);
        snapshot["routing_generation"] = json!(8);
        assert!(export_snapshot_v2(&snapshot.to_string()).is_none());
    }
}

/// Legacy replacement не может переносить причину предыдущего отказа.
pub fn invalidate_contexts(snapshot: &mut Value, slot: Option<context::Slot>) {
    let Some(object) = snapshot.as_object_mut() else {
        return;
    };
    let Some(slot) = slot else {
        object.remove("error_contexts");
        return;
    };
    let expected = serde_json::to_value(slot).ok();
    match object.get_mut("error_contexts") {
        Some(Value::Array(contexts)) => {
            contexts.retain(|value| Some(&value["slot"]) != expected.as_ref())
        }
        _ => {
            object.remove("error_contexts");
        }
    }
}

#[cfg(test)]
mod legacy_context_tests {
    use super::*;
    #[test]
    fn same_key_new_failure_cannot_reuse_reason() {
        let context = context::ErrorContext::new(
            context::Slot::Error,
            context::Cause::DataDirectoryPermissions,
            None,
        );
        let mut snapshot = json!({"error":"storage","error_contexts":[context]});
        assert!(export_snapshot_v2(&snapshot.to_string()).is_some());
        invalidate_contexts(&mut snapshot, Some(context::Slot::Error));
        snapshot["error"] = json!("storage");
        assert!(export_snapshot_v2(&snapshot.to_string()).is_none());
        let mut replacement =
            json!({"error":"storage","error_contexts":[context],"unknown":{"url":"CANARY"}});
        invalidate_contexts(&mut replacement, None);
        assert!(export_snapshot_v2(&replacement.to_string()).is_none());
        assert!(replacement.get("error_contexts").is_none());
    }
}

#[cfg(test)]
mod context_manifest_tests {
    use super::*;
    #[test]
    fn typed_reason_matrix_matches_versioned_validator() {
        let matrix: Value = serde_json::from_str(include_str!(
            "../../../docs/specs/diagnostic-context-v2.json"
        ))
        .expect("матрица");
        for (reason, identity) in matrix.as_object().expect("объект") {
            let cause: context::Cause =
                serde_json::from_value(json!(reason)).expect("закрытая причина");
            let typed = serde_json::to_value(context::ErrorContext::new(
                context::Slot::Error,
                cause,
                None,
            ))
            .expect("контекст");
            for field in ["code", "message_key", "stage"] {
                assert_eq!(typed[field], identity[field], "{reason}/{field}");
            }
            for slot in [
                context::Slot::Error,
                context::Slot::Connection,
                context::Slot::Source,
                context::Slot::RoutingAck,
                context::Slot::MaintenanceAck,
                context::Slot::LogsRead,
                context::Slot::LogsSave,
            ] {
                let name = serde_json::to_value(slot).expect("slot");
                assert_eq!(
                    cause.allows(slot),
                    identity["slots"].as_array().expect("slots").contains(&name),
                    "{reason}/slot"
                );
            }
            assert_eq!(
                cause.needs_generation(),
                identity["generation_required"]
                    .as_bool()
                    .expect("generation")
            );
        }
    }
}

#[cfg(test)]
mod logs_identity_tests {
    use super::*;
    #[test]
    fn legacy_string_ids_are_not_silently_normalized_into_sibling_ids() {
        let mut context = context::ErrorContext::new(
            context::Slot::LogsSave,
            context::Cause::LogsIoAlreadyExists,
            Some(1),
        );
        context.generation = Some(0);
        let mut snapshot = json!({"logs":{"request_id":"1","status":"refused"},"logs_channel":"logs_save","logs_generation":0,"error_contexts":[context]});
        assert!(export_snapshot_v2(&snapshot.to_string()).is_some());
        for id in ["01", "+1", "2"] {
            snapshot["logs"]["request_id"] = json!(id);
            assert!(export_snapshot_v2(&snapshot.to_string()).is_none());
        }
    }
}
