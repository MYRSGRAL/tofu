//! Production helper на системной шине собственной одноразовой VM.
//! Это отдельный gate: private bus и подмена authorizer здесь запрещены.
#![cfg(target_os = "linux")]

use serde_json::{Value, json};
use std::{fs, io, os::unix::fs::MetadataExt, process::Command, time::Duration};
const SERVICE_NAME: &str = "cloud.highpriority.yonik.Network1";
const SERVICE_INTERFACE: &str = SERVICE_NAME;
const SERVICE_PATH: &str = "/cloud/highpriority/yonik/Network";

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn require(value: bool, reason: &str) -> Result {
    if value {
        Ok(())
    } else {
        Err(io::Error::other(reason).into())
    }
}

fn guard() -> Result {
    require(
        std::env::var("YONIK_PRODUCTION_HELPER_VM").as_deref() == Ok("1"),
        "Нет явного VM opt-in",
    )?;
    require(
        fs::read_to_string("/sys/class/dmi/id/sys_vendor")?.trim() == "QEMU",
        "Требуется QEMU",
    )?;
    require(
        fs::read_to_string("/proc/1/comm")?.trim() == "systemd",
        "Требуется systemd PID 1",
    )?;
    let marker = fs::symlink_metadata("/etc/yonik-production-helper-vm")?;
    require(
        marker.is_file() && marker.uid() == 0 && marker.mode() & 0o022 == 0,
        "Небезопасный VM marker",
    )?;
    require(
        fs::read_to_string("/etc/yonik-production-helper-vm")?
            == "yonik-production-helper-disposable-v1\n",
        "Неверный VM marker",
    )?;
    require(
        std::env::var_os("DBUS_SYSTEM_BUS_ADDRESS").is_none(),
        "Подмена system bus запрещена",
    )?;
    Ok(())
}

fn helper_pid() -> Result<u32> {
    let output = Command::new("/usr/bin/systemctl")
        .args([
            "show",
            "--property=MainPID",
            "--value",
            "yonik-helper.service",
        ])
        .output()?;
    require(output.status.success(), "Не прочитан MainPID")?;
    let pid: u32 = std::str::from_utf8(&output.stdout)?.trim().parse()?;
    require(pid > 1, "Helper не запущен")?;
    require(
        fs::read_link(format!("/proc/{pid}/exe"))?
            == std::path::Path::new("/usr/lib/yonik/yonik-helper"),
        "Запущен fixture вместо production helper",
    )?;
    Ok(pid)
}

async fn call(connection: &zbus::Connection, method: &str, value: Value) -> Result<Value> {
    let request = value.to_string();
    let response = tokio::time::timeout(
        Duration::from_secs(40),
        connection.call_method(
            Some(SERVICE_NAME),
            SERVICE_PATH,
            Some(SERVICE_INTERFACE),
            method,
            &(request,),
        ),
    )
    .await??;
    let (body,): (String,) = response.body().deserialize()?;
    let value: Value = serde_json::from_str(&body)?;
    require(value["version"] == 1, "Неверная версия ответа")?;
    Ok(value)
}

#[tokio::test]
#[ignore = "Только выделенный непривилегированный UID внутри disposable VM"]
async fn production_helper_denies_unprivileged_uid() -> Result {
    guard()?;
    require(
        !rustix::process::geteuid().is_root(),
        "Отрицательный тест требует отдельный UID",
    )?;
    let connection = zbus::Connection::system().await?;
    for method in ["Prepare", "Stop", "Unlock"] {
        let request = if method == "Prepare" {
            json!({"version":1,"mode":"public_protection","allow_lan":false})
        } else {
            json!({"version":1})
        };
        let result = call(&connection, method, request).await;
        let error = result.expect_err("Неавторизованный UID получил доступ");
        // Отказ transport/timeout не является доказательством авторизации.
        require(
            error.to_string().contains("authorization"),
            "Ожидался именно отказ polkit",
        )?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "Root controller только в собственной disposable VM, настоящий установленный helper"]
async fn production_helper_public_policy_survives_sigkill() -> Result {
    guard()?;
    require(
        rustix::process::geteuid().is_root(),
        "Требуется VM root controller",
    )?;
    let _lock = vm_lock::acquire()?;
    let connection = zbus::Connection::system().await?;
    let empty = json!({"version":1});
    require(
        call(&connection, "State", empty.clone()).await?["state"] == "no_managed_policy",
        "VM не чистая",
    )?;
    let original_pid = helper_pid()?;
    let prepared = call(
        &connection,
        "Prepare",
        json!({"version":1,"mode":"public_protection","allow_lan":false}),
    )
    .await?;
    require(
        prepared["handle"].as_array().is_some_and(|v| v.len() == 32),
        "Неверный handle",
    )?;
    require(
        call(
            &connection,
            "Apply",
            json!({"version":1,"handle":prepared["handle"]}),
        )
        .await?["state"]
            == "verified_rules",
        "Apply не подтвердил policy",
    )?;
    require(
        call(&connection, "Stop", empty.clone()).await?["state"] == "recovery_required",
        "Stop снял защиту",
    )?;
    require(
        Command::new("/usr/bin/systemctl")
            .args([
                "kill",
                "--kill-whom=main",
                "--signal=SIGKILL",
                "yonik-helper.service",
            ])
            .status()?
            .success(),
        "SIGKILL не выполнен",
    )?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if helper_pid().is_ok_and(|pid| pid != original_pid) {
            if let Ok(state) = call(&connection, "State", empty.clone()).await {
                require(
                    state["state"] == "recovery_required",
                    "Журнал не восстановлен",
                )?;
                break;
            }
        }
        require(
            tokio::time::Instant::now() < deadline,
            "Helper не восстановился",
        )?;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    require(
        call(&connection, "Unlock", empty.clone()).await?["state"] == "no_managed_policy",
        "Unlock не очистил policy",
    )?;
    require(
        call(&connection, "State", empty).await?["state"] == "no_managed_policy",
        "Остались ресурсы",
    )?;
    Ok(())
}

#[path = "common/vm_lock.rs"]
mod vm_lock;

#[test]
fn host_without_opt_in_is_rejected() {
    if std::env::var_os("YONIK_PRODUCTION_HELPER_VM").is_none() {
        assert!(guard().is_err());
    }
}
