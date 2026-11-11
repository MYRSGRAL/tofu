use yonik_desktop::editor::{Draft, RuleInput};
#[test]
fn compound_roundtrip_and_invalid_input_preserve_draft() {
    let mut draft = Draft::default();
    let rule = RuleInput {
        id: 1,
        enabled: true,
        executables: vec!["/usr/bin/browser".into()],
        domains: vec!["*.example.org".into()],
        destinations: vec!["192.0.2.0/24".into()],
        action: "direct".into(),
    };
    draft.put(rule.clone(), false).expect("составное правило");
    let json = draft.json();
    let restored = Draft::parse(&json).expect("roundtrip");
    assert_eq!(restored.json(), json);
    assert_eq!(restored.policy().expect("Policy").user_rules.len(), 1);
    let mut invalid = rule;
    invalid.domains = vec!["bad domain".into()];
    assert!(draft.put(invalid, false).is_err());
    assert_eq!(draft.json(), json);
}
#[test]
fn broad_confirmation_ids_and_limits() {
    let mut draft = Draft::default();
    let mut rule = RuleInput {
        id: 1,
        enabled: true,
        executables: vec![],
        domains: vec![],
        destinations: vec![],
        action: "block".into(),
    };
    assert!(draft.put(rule.clone(), false).is_err());
    draft.put(rule.clone(), true).expect("подтверждение");
    rule.id = 0;
    assert!(draft.put(rule.clone(), true).is_err());
    rule.id = 2;
    rule.executables = vec!["/bin/a".into(); 65];
    assert!(draft.put(rule, true).is_err());
    assert!(Draft::parse("{broken").is_err());
}

#[tokio::test]
async fn encrypted_draft_roundtrip_and_cas() {
    let dir = std::env::temp_dir().join(format!("yonik-editor-db-{}", std::process::id()));
    std::fs::create_dir(&dir).expect("fixture");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("права");
    let repo = yonik_linux::EncryptedSubscriptionRepository::open(
        dir.join("draft.sqlite"),
        zeroize::Zeroizing::new([17; 32]),
    )
    .await
    .expect("БД");
    assert!(repo.routing_draft().await.expect("чтение").is_none());
    let raw = Draft::default().json().into_bytes();
    assert_eq!(
        repo.save_routing_draft(0, raw.clone())
            .await
            .expect("запись"),
        1
    );
    assert!(repo.save_routing_draft(0, b"stale".to_vec()).await.is_err());
    let (revision, restored) = repo.routing_draft().await.expect("чтение").expect("draft");
    assert_eq!(revision, 1);
    assert_eq!(*restored, raw);
    repo.close().await.expect("закрытие");
    let repo = yonik_linux::EncryptedSubscriptionRepository::open(
        dir.join("draft.sqlite"),
        zeroize::Zeroizing::new([17; 32]),
    )
    .await
    .expect("повторный запуск");
    let (_, bytes) = repo.routing_draft().await.expect("чтение").expect("запись");
    assert!(Draft::parse(std::str::from_utf8(&bytes).expect("UTF8")).is_ok());
    repo.save_routing_draft(1, b"{broken".to_vec())
        .await
        .expect("синтетическое повреждение");
    let (_, bytes) = repo
        .routing_draft()
        .await
        .expect("исходник")
        .expect("запись");
    assert!(Draft::parse(std::str::from_utf8(&bytes).expect("UTF8")).is_err());
    assert_eq!(bytes.as_slice(), b"{broken");
    repo.close().await.expect("закрытие");
    std::fs::remove_dir_all(dir).expect("очистка fixture");
}

#[test]
fn pid_is_not_a_persistent_selector() {
    assert!(Draft::parse(r#"{"version":1,"default_action":"proxy","rules":[{"id":1,"enabled":true,"executables":[],"domains":[],"destinations":[],"action":"direct","pid":1234}]}"#).is_err());
}

#[test]
fn bounded_diagnostics_keep_identity_and_disabled_semantics() {
    let mut draft = Draft::default();
    for id in 1..=3 {
        draft
            .put(
                RuleInput {
                    id,
                    enabled: id != 3,
                    executables: vec![],
                    domains: vec!["example.org".into()],
                    destinations: vec![],
                    action: "direct".into(),
                },
                false,
            )
            .expect("правило");
    }
    let report = draft.diagnostics();
    assert_eq!(report["warnings"][0]["rule_id"], 2);
    assert_eq!(report["warnings"][0]["kind"], "shadowed");
    assert_eq!(report["warnings"].as_array().expect("массив").len(), 1);
    assert_eq!(report["incomplete"], false);
    draft.rules[0].destinations = vec!["192.0.2.0/24".into()];
    assert_eq!(
        draft.diagnostics()["warnings"][0]["kind"],
        "potential_overlap"
    );
    draft.rules[0].domains = (0..33).map(|i| format!("a{i}.example.org")).collect();
    assert_eq!(draft.diagnostics()["incomplete"], true);
    assert!(
        draft.diagnostics()["warnings"]
            .as_array()
            .expect("массив")
            .is_empty()
    );
}

#[test]
fn explanation_uses_evaluator_without_claiming_runtime_witness() {
    let mut draft = Draft::default();
    assert_eq!(draft.explain("192.0.2.1").expect("IP")["origin"], "Default");
    assert_eq!(
        draft.explain("example.org").expect("имя")["status"],
        "unknown"
    );
    draft
        .put(
            RuleInput {
                id: 7,
                enabled: true,
                executables: vec![],
                domains: vec![],
                destinations: vec!["192.0.2.0/24".into()],
                action: "block".into(),
            },
            false,
        )
        .expect("правило");
    let before = draft.json();
    assert_eq!(draft.explain("192.0.2.1").expect("IP")["origin"], "User(7)");
    assert_eq!(draft.explain("192.0.2.1").expect("IP")["action"], "Block");
    assert!(draft.explain("bad domain").is_err());
    assert_eq!(draft.json(), before);
    draft.rules[0].executables.push("/fixture/app".into());
    assert_eq!(
        draft.explain("192.0.2.1").expect("нет атрибуции")["reason"],
        "application_witness_missing"
    );
}

#[test]
fn truncated_report_is_explicit_and_preserves_policy() {
    let mut draft = Draft::default();
    for id in 1..=25 {
        draft
            .put(
                RuleInput {
                    id,
                    enabled: true,
                    executables: vec![],
                    domains: vec![],
                    destinations: vec![],
                    action: "proxy".into(),
                },
                true,
            )
            .expect("широкое правило");
    }
    let before = draft.json();
    let report = draft.diagnostics();
    assert_eq!(report["incomplete"], true);
    assert_eq!(report["warnings"].as_array().expect("массив").len(), 256);
    assert_eq!(draft.json(), before);
}
