//! A-08: production bridge/qrc; только собственные данные и test-support observer.
#![deny(unsafe_code)]
#[cfg(all(feature = "gui", feature = "test-support"))]
#[allow(unsafe_code)]
#[path = "../src/bridge.rs"]
mod bridge;
#[cfg(all(feature = "gui", feature = "test-support"))]
#[path = "../src/bridge_impl.rs"]
mod bridge_impl;
#[cfg(all(feature = "gui", feature = "test-support"))]
#[path = "../src/dispatch_error.rs"]
mod dispatch_error;
#[cfg(all(feature = "gui", feature = "test-support"))]
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[cfg(all(feature = "gui", feature = "test-support"))]
use yonik_linux::applications::{CatalogEvent, CatalogObserver};
#[cfg(all(feature = "gui", feature = "test-support"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("root")?);
    fs::create_dir_all(&root)?;
    // Error/panic проверяют именно RAII guard, не ручной release fixture.
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let probe = CatalogObserver::new(move |event| {
        captured.lock().expect("own events").push(event);
        if matches!(event, CatalogEvent::Read { .. }) {
            panic!("A08 expected read panic");
        }
    });
    let probe_catalog = yonik_linux::applications::Catalog::new(
        vec![],
        root.join("absent"),
        "ru".into(),
        yonik_linux::applications::Limits::default(),
    )?
    .with_observer(Some(probe));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || probe_catalog.process_snapshot()
        ))
        .is_err()
    );
    assert!(matches!(
        events.lock().expect("own events").last(),
        Some(CatalogEvent::Released { token: 1 })
    ));
    let errors = Arc::new(Mutex::new(Vec::new()));
    let captured = errors.clone();
    let probe = CatalogObserver::new(move |event| captured.lock().expect("own events").push(event));
    let probe_catalog = yonik_linux::applications::Catalog::new(
        vec![],
        root.join("absent"),
        "ru".into(),
        yonik_linux::applications::Limits::default(),
    )?
    .with_observer(Some(probe));
    for _ in 0..2 {
        assert!(probe_catalog.process_snapshot().is_err());
    }
    let errors = errors.lock().expect("own events");
    assert_eq!(
        errors
            .iter()
            .filter(|e| matches!(e, CatalogEvent::Released { .. }))
            .count(),
        2
    );
    assert!(matches!(
        errors.last(),
        Some(CatalogEvent::Released { token: 2 })
    ));
    let known = r#"{"version":2,"rules":[{"id":8,"enabled":true,"executables":[],"domains":["example.com"],"destinations":[],"action":"block","platform_sources":[{"kind":"android_package","package":"org.example.App"}]}],"default_action":"proxy"}"#;
    let unknown = known.replace("android_package", "future_platform");
    assert!(yonik_desktop::editor::Draft::parse(&unknown).is_err());
    let draft = yonik_desktop::editor::Draft::parse(known)?;
    assert_eq!(draft.diagnostics()["error"], "unsupported_selector");
    assert_eq!(draft.diagnostics()["rule_id"], 8);
    let policy = r#"{"version":2,"user_rules":[{"id":8,"action":"block","conditions":{"domain":["example.com"],"platform_sources":[{"kind":"future_platform","package":"org.example.App"}]}}]}"#;
    assert!(matches!(
        yonik_engine::policy_input::read_policy(policy.as_bytes()),
        Err(yonik_engine::policy_input::PolicyInputError::InvalidJson)
    ));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let repository = yonik_linux::EncryptedSubscriptionRepository::open(
            root.join("subscriptions.sqlite"),
            zeroize::Zeroizing::new([77; 32]),
        )
        .await?;
        repository.routing_draft().await?;
        repository.close().await?;
        let backup = root.join("format3-backup");
        yonik_linux::EncryptedSubscriptionRepository::backup_closed(
            root.join("subscriptions.sqlite"),
            backup.clone(),
            zeroize::Zeroizing::new([77; 32]),
        )
        .await?;
        let repository = yonik_linux::EncryptedSubscriptionRepository::open(
            root.join("subscriptions.sqlite"),
            zeroize::Zeroizing::new([77; 32]),
        )
        .await?;
        repository
            .save_routing_draft_v2(0, known.as_bytes().to_vec(), Some(backup))
            .await?;
        Ok::<_, yonik_linux::RepositoryError>(())
    })?;
    drop(runtime);
    let file = Arc::new(Mutex::new(fs::File::create(root.join("catalog.jsonl"))?));
    let barrier_root = root.clone();
    let observer = CatalogObserver::new(move |event| {
        let value = match event {
            CatalogEvent::Admitted { token, processes } => {
                serde_json::json!({"event":"admitted","token":token,"processes":processes})
            }
            CatalogEvent::Read { token, class } => {
                serde_json::json!({"event":"read","token":token,"class":format!("{class:?}")})
            }
            CatalogEvent::Released { token } => {
                if fs::remove_file(barrier_root.join("arm")).is_ok() {
                    fs::write(barrier_root.join("held"), token.to_string()).expect("own barrier");
                    let start = Instant::now();
                    while !barrier_root.join(format!("release-{token}")).exists()
                        && start.elapsed() < Duration::from_secs(8)
                    {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    assert!(
                        barrier_root.join(format!("release-{token}")).exists(),
                        "barrier deadline"
                    );
                }
                serde_json::json!({"event":"released","token":token})
            }
        };
        writeln!(file.lock().expect("trace lock"), "{value}").expect("own trace");
    });
    bridge_impl::configure(
        yonik_desktop::HostConfig::new(root.clone(), root.join("no-engines"))
            .with_ephemeral_key([77; 32])
            .with_catalog_observer(observer),
    )
    .map_err(|_| "config")?;
    let code = bridge::ffi::run_application(
        &root.to_str().ok_or("UTF8")?.into(),
        false,
        &"".into(),
        &"".into(),
        &"".into(),
    );
    if code != 0 {
        return Err(format!("Qt {code}").into());
    }
    Ok(())
}

#[cfg(not(all(feature = "gui", feature = "test-support")))]
fn main() {}
