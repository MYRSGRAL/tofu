#[path = "../src/dispatch_error.rs"]
mod dispatch_error;

use serde_json::{Value, json};

#[test]
fn rejected_logs_request_receives_matching_ack_and_cannot_export_stale_page() {
    let previous = json!({"ready": true, "logs": {"request_id": "6", "status": "ready", "export": "old", "events": [{"id": "1"}]}});
    let rejected = dispatch_error::snapshot(
        &previous.to_string(),
        r#"{"command":"logs_read","request_id":"7","after":"0","source":"all"}"#,
        "busy",
    );
    let ack: Value = serde_json::from_str(&rejected).expect("снимок");
    assert_eq!(ack["logs"]["request_id"], "7");
    assert_eq!(ack["logs"]["status"], "busy");
    assert!(ack["logs"]["export"].is_null());
    assert_eq!(ack["logs"]["events"], json!([]));
    assert_eq!(ack["ready"], true);
    assert_eq!(ack["error"], "busy");
}

#[test]
fn routing_rejection_keeps_matching_ack_and_unrelated_logs() {
    let value = dispatch_error::snapshot(
        r#"{"logs":{"request_id":"9"}}"#,
        r#"{"command":"routing_order","request_id":"11"}"#,
        "busy",
    );
    let ack: Value = serde_json::from_str(&value).expect("снимок");
    assert_eq!(ack["routing_ack"]["request_id"], "11");
    assert_eq!(ack["routing_ack"]["error"], "busy");
    assert_eq!(ack["logs"]["request_id"], "9");
}

#[test]
fn malformed_request_cannot_fabricate_logs_ack() {
    for request in [
        "broken",
        r#"{"command":"logs_read"}"#,
        r#"{"command":"logs_read","request_id":{"secret":"CANARY"}}"#,
    ] {
        let value = dispatch_error::snapshot("{}", request, "invalid_command");
        let ack: Value = serde_json::from_str(&value).expect("снимок");
        assert!(ack.get("logs").is_none());
        assert!(!value.contains("CANARY"));
    }
}

#[test]
fn unavailable_host_completes_logs_request_without_reusing_old_export() {
    let value = dispatch_error::snapshot(
        r#"{"logs":{"export":"old"}}"#,
        r#"{"command":"logs_read","request_id":"8"}"#,
        "startup",
    );
    let ack: Value = serde_json::from_str(&value).expect("снимок");
    assert_eq!(ack["logs"]["request_id"], "8");
    assert_eq!(ack["logs"]["status"], "error");
    assert!(ack["logs"]["export"].is_null());
}

#[test]
fn routing_apply_получает_точный_ack_при_queue_full_и_startup() {
    for error in ["busy", "startup"] {
        let value = dispatch_error::snapshot(
            r#"{"ready":true,"routing_ack":{"request_id":7}}"#,
            r#"{"command":"routing_apply","request_id":81}"#,
            error,
        );
        let ack: Value = serde_json::from_str(&value).expect("DTO");
        assert_eq!(ack["routing_ack"]["request_id"], 81);
        assert_eq!(ack["routing_ack"]["error"], error);
    }
}
