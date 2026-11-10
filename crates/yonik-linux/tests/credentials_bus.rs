//! Изолированные вызовы production API: адрес передаётся только дочернему процессу.
#[path = "credentials_bus/fixtures.rs"]
mod fixtures;

use yonik_linux::{CredentialError, subscription_master_key};

#[tokio::test(flavor = "current_thread")]
async fn production_api_child() {
    let Ok(case) = std::env::var("YONIK_CREDENTIAL_CASE") else {
        return;
    };
    let expected = match case.as_str() {
        "collection" if std::env::var_os("YONIK_CREATE").is_none() => CredentialError::Missing,
        "locked" | "collection" | "cancel" | "race" => CredentialError::Locked,
        "missing" => CredentialError::Missing,
        "ambiguous" => CredentialError::Ambiguous,
        _ => CredentialError::Unavailable,
    };
    let result = subscription_master_key(std::env::var_os("YONIK_CREATE").is_some()).await;
    assert_eq!(result.err(), Some(expected));
}

#[tokio::test(flavor = "current_thread")]
async fn private_bus_failures() {
    let mut failures = Vec::new();
    for case in [
        "absent",
        "missing",
        "locked",
        "collection",
        "cancel",
        "race",
        "failing",
        "malformed",
        "cipher",
        "ambiguous",
    ] {
        for create in [false, true] {
            if case == "missing" && create {
                continue;
            }
            let fixture = fixtures::Fixture::start(case).await;
            let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
            child
                .args(["--exact", "production_api_child", "--nocapture"])
                .env("DBUS_SESSION_BUS_ADDRESS", &fixture.address)
                .env("XDG_RUNTIME_DIR", &fixture.runtime)
                .env("YONIK_CREDENTIAL_CASE", case)
                .env_remove("YONIK_CREATE");
            if create {
                child.env("YONIK_CREATE", "1");
            }
            let output = tokio::time::timeout(
                std::time::Duration::from_secs(25),
                child.kill_on_drop(true).output(),
            )
            .await
            .unwrap()
            .unwrap();
            if !output.status.success() {
                failures.push(format!(
                    "{case}, create={create}: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            fixture.assert_readonly();
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
