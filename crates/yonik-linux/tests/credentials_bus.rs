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

#[tokio::test(flavor = "current_thread")]
async fn startup_owner_guard_child() {
    let Ok(case) = std::env::var("YONIK_STARTUP_OWNER_CASE") else {
        return;
    };
    let observed = yonik_linux::CredentialOwnerGuard::observe().await;
    if case == "absent" {
        assert!(observed.is_err());
        return;
    }
    let guard = observed.expect("реальный SS owner");
    guard.check().await.expect("начальный owner");
    use std::io::Write;
    println!("guard_ready");
    std::io::stdout().flush().unwrap();
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
    })
    .await
    .unwrap();
    assert!(guard.check().await.is_err(), "replacement закрывает epoch");
    assert!(guard.owner_lost(), "поздний ready не восстанавливает epoch");
    assert!(guard.check().await.is_err(), "нет rearm");
}

#[tokio::test(flavor = "current_thread")]
async fn startup_private_owner_replacement_and_absent() {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    for case in ["absent", "replacement"] {
        let mut fixture = fixtures::Fixture::start(if case == "absent" {
            "absent"
        } else {
            "missing"
        })
        .await;
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "startup_owner_guard_child", "--nocapture"])
            .env("DBUS_SESSION_BUS_ADDRESS", &fixture.address)
            .env("XDG_RUNTIME_DIR", &fixture.runtime)
            .env("YONIK_STARTUP_OWNER_CASE", case)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        if case == "replacement" {
            // Reader живёт до wait: libtest печатает summary после последнего assert.
            let mut lines = (&mut stdout).lines();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while lines
                    .next_line()
                    .await
                    .unwrap()
                    .expect("child closed before ready")
                    != "guard_ready"
                {}
            })
            .await
            .expect("guard ready deadline");
            fixture.replace_owner().await;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"continue\n")
                .await
                .unwrap();
        }
        let output =
            tokio::time::timeout(std::time::Duration::from_secs(10), child.wait_with_output())
                .await
                .unwrap()
                .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
