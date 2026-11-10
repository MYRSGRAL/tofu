//! Только собственная disposable QEMU VM: настоящий host/registry/systemd/nft/TUN,
//! D-Bus/polkit transport fixture и синтетический VLESS peer за отдельным veth.
use super::*;
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
};
#[path = "../../tests/common/vm_lock.rs"]
mod gate_lock;
fn guest_guard() -> Result<std::fs::File, Box<dyn std::error::Error>> {
    if std::env::var_os("YONIK_SYSTEMD_VM_GATE").as_deref() != Some(std::ffi::OsStr::new("1"))
        || !rustix::process::geteuid().is_root()
    {
        return Err("Требуется opt-in root disposable VM".into());
    }
    let marker = OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open("/run/yonik-vm-gate")?;
    let metadata = marker.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.nlink() != 1
        || metadata.mode() & 0o7777 != 0o600
    {
        return Err("Небезопасный VM marker".into());
    }
    let mut contents = String::new();
    marker.take(128).read_to_string(&mut contents)?;
    if contents != "yonik-disposable-systemd-gate-v1\n" {
        return Err("Неизвестная VM".into());
    }
    fn bounded(path: &str) -> std::io::Result<String> {
        let mut text = String::new();
        File::open(path)?.take(256).read_to_string(&mut text)?;
        Ok(text)
    }
    if bounded("/sys/class/dmi/id/sys_vendor")?.trim() != "QEMU"
        || !bounded("/sys/class/dmi/id/product_name")?
            .trim()
            .starts_with("Standard PC")
        || bounded("/proc/1/comm")?.trim() != "systemd"
    {
        return Err("Нужна QEMU VM с systemd PID1".into());
    }
    let directory = rustix::fs::open(
        "/sys/fs/cgroup",
        rustix::fs::OFlags::RDONLY,
        rustix::fs::Mode::empty(),
    )?;
    if rustix::fs::fstatfs(directory)?.f_type as u64 != 0x6367_7270 {
        return Err("Требуется cgroup v2".into());
    }
    Ok(gate_lock::acquire()?)
}
async fn endpoint_reachable() -> bool {
    tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::new(10, 203, 0, 2), 24443)),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}
async fn traffic() {
    let script = r#"import sys,socket
sys.path.insert(0,'/opt/yonik-gate/tools')
from engine_traffic import probe
for family,address in [(socket.AF_INET,'198.18.0.2'),(socket.AF_INET6,'2001:db8:746f::2')]:
 for kind in (socket.SOCK_STREAM,socket.SOCK_DGRAM):
  probe(family,address,kind)
print('actual TUN TCP/UDP IPv4/IPv6: PASS')
"#;
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new("/usr/bin/python3")
            .args(["-c", script])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "Traffic gate: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&output.stdout).trim());
}
fn profile(engine: &str, kill_switch: bool) -> String {
    let imported = yonik_import::import_bytes(
        b"vless://00112233-4455-6677-8899-aabbccddeeff@10.203.0.2:24443?security=none".to_vec(),
    )
    .unwrap();
    serde_json::json!({"version":1,"mode":"system_profile","engine":engine,"kill_switch":kill_switch,"server":yonik_import::ServerIntent::from_server(&imported.servers[0]).unwrap()}).to_string()
}
async fn fixture() -> Fixture {
    let (_stop, cancel) = Cancellation::new();
    Fixture::real_host(
        crate::helper::SystemSessionHost::system(cancel)
            .await
            .unwrap(),
    )
    .await
}
async fn assert_clean() {
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read("/run/yonik/network/journal.json").unwrap()).unwrap();
    assert_eq!(document["records"], serde_json::json!([]));
    assert!(document["public_policy"].is_null());
    assert!(document["session_recovery"].is_null());
    for name in ["yonik-worker", "yonik-worker-check"] {
        let group = std::path::Path::new("/sys/fs/cgroup/system.slice")
            .join(format!("{name}.service/cgroup.events"));
        if let Ok(events) = std::fs::read_to_string(group) {
            assert!(events.lines().any(|line| line == "populated 0"));
        }
    }
}
async fn wait_automatic_cleanup() {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    loop {
        let document: serde_json::Value =
            serde_json::from_slice(&std::fs::read("/run/yonik/network/journal.json").unwrap())
                .unwrap();
        if document["session_recovery"].is_null() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Автоматический cleanup не завершён без RPC State/Stop"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
#[test]
fn installed_session_guard_rejects_host_without_opt_in() {
    if std::env::var_os("YONIK_SYSTEMD_VM_GATE").is_none() {
        assert!(guest_guard().is_err());
    }
}
#[tokio::test]
#[ignore = "Требует явно созданную disposable systemd VM и синтетический VLESS peer"]
async fn real_installed_profile_rpc_tunnel_and_restart_cleanup() {
    let _gate = guest_guard().unwrap();
    assert!(
        endpoint_reachable().await,
        "Синтетический peer недоступен до effects"
    );
    for engine in ["xray", "sing-box"] {
        for kill_switch in [false, true] {
            for ending in ["stop", "restart", "worker_exit"] {
                eprintln!("profile RPC: engine={engine} kill_switch={kill_switch} ending={ending}");
                let f = fixture().await;
                let prepared = f
                    .call("Prepare", &profile(engine, kill_switch))
                    .await
                    .unwrap();
                let request =
                    serde_json::json!({"version":1,"handle":prepared["handle"]}).to_string();
                let result = f.call("Apply", &request).await.unwrap();
                assert_eq!(result["state"], "tunnel_starting");
                assert!(f.call("Apply", &request).await.is_err());
                traffic().await;
                assert_eq!(
                    f.call("State", r#"{"version":1}"#).await.unwrap()["state"],
                    "tunnel_starting"
                );
                let f = if ending == "restart" {
                    f.shutdown().await;
                    let recovered = fixture().await;
                    wait_automatic_cleanup().await;
                    recovered
                } else {
                    if ending == "worker_exit" {
                        let status = tokio::process::Command::new("/usr/bin/systemctl")
                            .args([
                                "kill",
                                "--kill-whom=all",
                                "--signal=KILL",
                                "yonik-worker.service",
                            ])
                            .status()
                            .await
                            .unwrap();
                        assert!(status.success());
                        wait_automatic_cleanup().await;
                    }
                    f
                };
                let state = f.call("Stop", r#"{"version":1}"#).await.unwrap();
                assert_eq!(
                    state["state"],
                    if kill_switch {
                        "recovery_required"
                    } else {
                        "no_managed_policy"
                    }
                );
                f.call("Stop", r#"{"version":1}"#).await.unwrap();
                if kill_switch {
                    assert!(
                        !endpoint_reachable().await,
                        "Kill switch допускает DIRECT к peer"
                    );
                    f.call("Unlock", r#"{"version":1}"#).await.unwrap();
                }
                assert!(endpoint_reachable().await, "Обычная сеть не восстановлена");
                assert_clean().await;
                f.shutdown().await;
            }
        }
    }
}
