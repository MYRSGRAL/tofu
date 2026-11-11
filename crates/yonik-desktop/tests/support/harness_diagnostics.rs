//! Allowlist диагностики тестового Harness: raw DTO/команды не выводятся.
use serde_json::{Value, json};
use std::{
    panic::Location,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::RecvTimeoutError,
    },
    time::Duration,
};

static NEXT: AtomicU64 = AtomicU64::new(1);
pub(super) struct Trace {
    id: u64,
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    command: Value,
    snapshot: Value,
    received: u64,
}
fn project(value: &Value) -> Value {
    let mut result = serde_json::Map::new();
    for key in [
        "ready",
        "closed",
        "busy",
        "port",
        "source_id",
        "refresh_source_id",
        "total",
        "sources_total",
        "revision",
        "selected_profile",
        "source_hwid_enabled",
        "tcp_busy",
        "preferences_warning",
    ] {
        if let Some(v) = value
            .get(key)
            .filter(|v| v.is_boolean() || v.is_number() || v.is_null())
        {
            result.insert(key.into(), v.clone());
        }
    }
    for key in [
        "error",
        "source_error",
        "connection_error",
        "status",
        "engine",
        "language",
    ] {
        if let Some(v) = value.get(key) {
            let allowed = v.is_null()
                || v.as_str().is_some_and(|s| match key {
                    "engine" => ["xray", "sing-box"].contains(&s),
                    "language" => ["ru", "en"].contains(&s),
                    "status" => [
                        "Idle",
                        "Starting",
                        "Connected",
                        "Stopping",
                        "Stopped",
                        "Failed",
                        "Limited",
                        "Disconnected",
                        "Connecting",
                        "Disconnecting",
                    ]
                    .contains(&s),
                    _ => [
                        "storage",
                        "busy",
                        "startup",
                        "listen_port_busy",
                        "preferences_write",
                        "preferences_invalid",
                        "cleanup_failed",
                        "keyring_locked",
                        "keyring_missing",
                        "fetch",
                        "hwid_required",
                        "hwid_limit",
                        "hwid_rejected",
                        "hwid_redirect",
                        "source_required",
                        "profile_required",
                        "invalid_input",
                        "cancelled",
                        "stale",
                        "closed",
                        "unsupported_proxy",
                        "routing_storage",
                    ]
                    .contains(&s),
                });
            result.insert(
                key.into(),
                if allowed {
                    v.clone()
                } else {
                    Value::String("<redacted>".into())
                },
            );
        }
    }
    result.insert(
        "pending_present".into(),
        Value::Bool(value.get("pending").is_some_and(|v| !v.is_null())),
    );
    Value::Object(result)
}
impl Trace {
    pub(super) fn new() -> Self {
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            state: Mutex::new(State::default()),
        }
    }
    pub(super) fn snapshot(&self, value: &Value) {
        let mut state = self.state.lock().unwrap();
        state.snapshot = project(value);
        state.received += 1;
    }
    pub(super) fn command(&self, value: &Value) {
        let command = value.get("command").and_then(Value::as_str).filter(|s| {
            [
                "add_source",
                "import_source",
                "select_source",
                "set_source_hwid",
                "refresh",
                "approve_refresh",
                "discard_refresh",
                "delete_source",
                "rename_source",
                "shutdown",
                "stop",
                "connect",
                "select_profile",
                "set_engine",
                "set_listen_port",
                "reset_listen_port",
                "set_probe_target",
                "set_language",
                "reload",
                "routing_default",
                "check_tcp",
                "check_batch",
            ]
            .contains(s)
        });
        self.state.lock().unwrap().command = json!({"name": command});
    }
    pub(super) fn closed(&self) {
        let state = self.state.lock().unwrap();
        eprintln!(
            "{}",
            json!({"event":"yonik_harness_closed", "pid":std::process::id(), "harness":self.id,
            "received":state.received,"last_command":state.command,"last_snapshot":state.snapshot})
        );
    }
    pub(super) fn fail(
        &self,
        error: RecvTimeoutError,
        caller: &Location<'_>,
        elapsed: Duration,
    ) -> ! {
        let state = self.state.lock().unwrap();
        let diagnostic = json!({"event":"yonik_harness_wait_failed", "pid":std::process::id(), "harness":self.id,
            "wait_file":caller.file(),"wait_line":caller.line(), "wait_column":caller.column(), "elapsed_ms":elapsed.as_millis(),
            "receive_error":format!("{error:?}"), "received":state.received,"last_command":state.command,"last_snapshot":state.snapshot});
        panic!("snapshot до deadline: {diagnostic}");
    }
}

#[test]
fn diagnostic_excludes_private_snapshot_and_command_fields() {
    let trace = Trace::new();
    trace.snapshot(&json!({"ready":false,"error":"storage","port":12345,"source_name":"CANARY_PRIVATE","profiles":[{"password":"CANARY_PRIVATE"}],"refresh_token":"CANARY_PRIVATE","url":"CANARY_PRIVATE", "connection_error":"CANARY_PRIVATE", "status":"CANARY_PRIVATE"}));
    trace.command(&json!({"command":"add_source","url":"CANARY_PRIVATE","name":"CANARY_PRIVATE"}));
    let state = trace.state.lock().unwrap();
    assert_eq!(state.snapshot["error"], "storage");
    assert_eq!(state.snapshot["port"], 12345);
    assert!(!format!("{}{}", state.snapshot, state.command).contains("CANARY_PRIVATE"));
}

#[test]
fn timeout_reports_wait_site_and_safe_last_state() {
    let trace = Trace::new();
    trace.snapshot(
        &json!({"ready":false,"busy":false,"error":"storage","source_name":"CANARY_PRIVATE"}),
    );
    trace.command(&json!({"command":"add_source","url":"CANARY_PRIVATE"}));
    let failure = std::panic::catch_unwind(|| {
        trace.fail(
            RecvTimeoutError::Timeout,
            Location::caller(),
            Duration::from_secs(10),
        );
    })
    .unwrap_err();
    let text = failure.downcast_ref::<String>().unwrap();
    assert!(text.contains("yonik_harness_wait_failed"));
    assert!(text.contains("last_snapshot"));
    assert!(text.contains("storage"));
    assert!(text.contains("wait_line"));
    assert!(text.contains("add_source"));
    assert!(!text.contains("CANARY_PRIVATE"));
}
