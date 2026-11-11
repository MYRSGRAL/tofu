//! Соответствующий запросу ACK при отказе передачи команды GUI в actor.
use serde_json::{Map, Value, json};

pub fn snapshot(current: &str, request: &str, message: &str) -> String {
    let mut snapshot = serde_json::from_str::<Map<String, Value>>(current).unwrap_or_default();
    if let Ok(command) = serde_json::from_str::<Value>(request) {
        if matches!(
            command["command"].as_str(),
            Some("maintenance_start" | "maintenance_cancel")
        ) {
            if let Some(request_id) = command["request_id"].as_u64() {
                snapshot.insert(
                    "maintenance_ack".into(),
                    json!({"request_id": request_id, "phase": "failed", "error": message}),
                );
            }
        }
        if matches!(
            command["command"].as_str(),
            Some("edit_routing" | "routing_order" | "routing_delete" | "routing_default")
        ) {
            snapshot.insert(
                "routing_ack".into(),
                json!({"request_id": command["request_id"], "saved_id": null, "error": message}),
            );
        }
        if let Some(request_id) = command["request_id"].as_str().filter(|id| {
            !id.is_empty()
                && id.len() <= 20
                && id.bytes().all(|byte| byte.is_ascii_digit())
                && id.parse::<u64>().is_ok()
        }) {
            if command["command"] == "logs_read" {
                snapshot.insert(
                    "logs".into(),
                    json!({
                        "request_id": request_id,
                        "status": if message == "busy" { "busy" } else { "error" },
                        "cursor": "0", "evicted": "0", "rejected": "0", "missed": "0",
                        "events": [], "export": null
                    }),
                );
            }
        }
    }
    snapshot.insert("error".into(), message.into());
    Value::Object(snapshot).to_string()
}
