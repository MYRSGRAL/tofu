//! Admission проверяется без запуска DesktopHost и системных служб.
use super::{Command, validate};

#[test]
fn backup_accepts_only_authoritative_source_selection() {
    let parse = |raw: &str| serde_json::from_str::<Command>(raw).expect("JSON fixture");
    let allowed = parse(
        r#"{"command":"maintenance_start","request_id":1,"operation":"backup","destination":"/private/new.sqlite","revision":0}"#,
    );
    assert!(validate(&allowed).is_ok());
    let supplied = parse(
        r#"{"command":"maintenance_start","request_id":1,"operation":"backup","source":"/private/live.sqlite","destination":"/private/new.sqlite","revision":0}"#,
    );
    assert_eq!(
        validate(&supplied)
            .expect_err("источник Backup не задаётся GUI")
            .key(),
        "invalid_input"
    );
}

#[test]
fn rejects_nonlocal_restore_and_inexact_request_identity() {
    for raw in [
        r#"{"command":"maintenance_start","request_id":1,"operation":"restore","source":"file:///candidate","destination":"/private/new.sqlite","revision":0}"#,
        r#"{"command":"maintenance_cancel","request_id":9007199254740992}"#,
    ] {
        let command = serde_json::from_str::<Command>(raw).expect("JSON fixture");
        assert_eq!(
            validate(&command).expect_err("отказ admission").key(),
            "invalid_input"
        );
    }
}
