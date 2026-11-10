//! Экспортирует только закрытый набор метаданных, никогда не исходный snapshot.
use serde_json::{Value, json};

/// Отказ не содержит входных данных. Свободные поля snapshot не копируются.
pub fn export_snapshot(raw: &str) -> Option<String> {
    if raw.len() > 1_048_576 {
        return None;
    }
    let snapshot: Value = serde_json::from_str(raw).ok()?;
    let object = snapshot.as_object()?;
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
        let code = match message_key {
            "cancelled" => "CANCELLED",
            "cleanup_failed" => "CLEANUP_FAILED",
            _ => return None,
        };
        let error = json!({"code":code,"message_key":message_key,"stage":"operation","retryable":false,"params":{}});
        if !errors.contains(&error) {
            errors.push(error);
        }
    }
    Some(json!({"schema_version":1,"platform":"linux","errors":errors}).to_string())
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
        assert!(export_snapshot("[]").is_none());
        assert!(export_snapshot(&" ".repeat(1_048_577)).is_none());
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
        for key in ["cancelled", "cleanup_failed"] {
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
