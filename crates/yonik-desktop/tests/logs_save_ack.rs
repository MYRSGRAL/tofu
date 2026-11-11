#[path = "../src/dispatch_error.rs"]
mod dispatch_error;

#[test]
fn save_rejection_matches_new_request_without_export() {
    let value = dispatch_error::snapshot(
        r#"{"logs":{"request_id":"1","export":"old"}}"#,
        r#"{"command":"logs_save","request_id":"2"}"#,
        "busy",
    );
    let ack: serde_json::Value = serde_json::from_str(&value).expect("ACK");
    assert_eq!(ack["logs"]["request_id"], "2");
    assert!(ack["logs"]["export"].is_null());
}
