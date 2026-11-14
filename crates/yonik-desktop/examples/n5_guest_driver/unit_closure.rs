//! Fresh loaded-manager closure перед Host; root capture не заменяет live query.
use super::{Result, VerifiedFixture, canonical, root_file};
use crate::{hash::sha256, protocol::strict_json};
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::Read,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

const CAPTURE: &str = "/run/yonik-acceptance/unit-closure.json";
const LIMIT: usize = 65536;
const DEADLINE: Duration = Duration::from_secs(3);
const UNITS: [&str; 5] = [
    "yonik-helper.service",
    "yonik-worker.service",
    "yonik-worker-check.service",
    "yonik-worker.socket",
    "yonik-worker-check.socket",
];
const COMMON: &[&str] = &[
    "Id",
    "LoadState",
    "FragmentPath",
    "DropInPaths",
    "SourcePath",
    "Transient",
    "NeedDaemonReload",
    "ActiveState",
    "SubState",
    "InvocationID",
];
const SERVICE: &[&str] = &[
    "ExecStart",
    "ExecStartPre",
    "ExecStartPost",
    "ExecCondition",
    "ExecReload",
    "ExecStop",
    "ExecStopPost",
    "User",
    "Group",
    "Type",
    "BusName",
    "ControlGroup",
    "MainPID",
    "ExecMainStartTimestampMonotonic",
    "Environment",
    "EnvironmentFiles",
    "RootDirectory",
    "RootImage",
    "WorkingDirectory",
    "DynamicUser",
    "PAMName",
    "NoNewPrivileges",
    "CapabilityBoundingSet",
    "AmbientCapabilities",
    "KillMode",
    "PrivateNetwork",
    "PrivateDevices",
    "ProtectSystem",
    "ProtectHome",
];
const SOCKET: &[&str] = &[
    "Listen",
    "SocketUser",
    "SocketGroup",
    "SocketMode",
    "DirectoryMode",
    "Accept",
    "RemoveOnStop",
    "Triggers",
];
type Properties = BTreeMap<String, String>;
type Manager = BTreeMap<String, Properties>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Provenance {
    source: String,
    manifest_sha256: String,
    nonce: String,
    case: String,
    kernel_boot_id: String,
    collector_sha256: String,
    authorization_sha256: String,
    manager_client_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Helper {
    pid: u32,
    start_ticks: u64,
    uid: u32,
    exe_sha256: String,
    exe_size: u64,
    invocation: String,
    exec_main_start_monotonic_usec: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capture {
    schema: String,
    provenance: Provenance,
    units: Manager,
    helper: Helper,
}

// Ни caller inputs, ни capture properties не становятся argv. Только fixed units.
fn properties(unit: &str) -> Vec<&'static str> {
    COMMON
        .iter()
        .chain(if unit.ends_with(".service") {
            SERVICE
        } else {
            SOCKET
        })
        .copied()
        .collect()
}
fn parse_properties(raw: &[u8], unit: &str) -> Result<Properties> {
    if raw.len() > LIMIT || !raw.ends_with(b"\n") {
        return Err("UNAVAILABLE_UNIT_PROPERTIES");
    }
    let raw = std::str::from_utf8(raw).map_err(|_| "UNIT_PROPERTY_UTF8")?;
    let allowed = properties(unit);
    let mut result = Properties::new();
    for line in raw.lines() {
        let (key, value) = line.split_once('=').ok_or("UNIT_PROPERTY_PROTOCOL")?;
        if !allowed.contains(&key) || result.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err("UNIT_PROPERTY_PROTOCOL");
        }
    }
    if result.len() != allowed.len() {
        return Err("UNAVAILABLE_UNIT_PROPERTIES");
    }
    Ok(result)
}
fn query(unit: &str) -> Result<Properties> {
    let mut child = Command::new("/usr/bin/systemctl")
        .args([
            "--system",
            "--no-pager",
            "--all",
            "show",
            unit,
            "--property",
            &properties(unit).join(","),
        ])
        .env_clear()
        .env("PATH", "/usr/bin")
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env(
            "DBUS_SYSTEM_BUS_ADDRESS",
            "unix:path=/run/dbus/system_bus_socket",
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "UNAVAILABLE_MANAGER_QUERY")?;
    let fd = match Pid::from_raw(child.id() as i32)
        .and_then(|pid| pidfd_open(pid, PidfdFlags::NONBLOCK).ok())
    {
        Some(fd) => fd,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("UNAVAILABLE_MANAGER_PIDFD");
        }
    };
    let Some(output) = child.stdout.take() else {
        let _ = pidfd_send_signal(&fd, Signal::KILL);
        child.wait().map_err(|_| "UNAVAILABLE_MANAGER_REAP")?;
        return Err("UNAVAILABLE_MANAGER_STDOUT");
    };
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut raw = Vec::new();
        let result = output
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut raw)
            .map(|_| raw);
        let _ = tx.send(result);
    });
    let result = (|| {
        let end = Instant::now() + DEADLINE;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|_| "MANAGER_WAIT")? {
                break status;
            }
            if Instant::now() >= end {
                return Err("UNAVAILABLE_MANAGER_TIMEOUT");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        if !status.success() {
            return Err("UNAVAILABLE_MANAGER_QUERY");
        }
        let raw = rx
            .recv_timeout(end.saturating_duration_since(Instant::now()))
            .map_err(|_| "UNAVAILABLE_MANAGER_OUTPUT")?
            .map_err(|_| "UNAVAILABLE_MANAGER_OUTPUT")?;
        parse_properties(&raw, unit)
    })();
    if result.is_err() {
        let _ = pidfd_send_signal(&fd, Signal::KILL);
    }
    // Только собственный systemctl child. Ошибка reap запрещает closure.
    child.wait().map_err(|_| "UNAVAILABLE_MANAGER_REAP")?;
    result
}
fn snapshot() -> Result<Manager> {
    UNITS
        .iter()
        .map(|unit| Ok(((*unit).to_owned(), query(unit)?)))
        .collect()
}
fn prop<'a>(p: &'a Properties, key: &str) -> Result<&'a str> {
    p.get(key)
        .map(String::as_str)
        .ok_or("UNIT_PROPERTY_MISSING")
}
fn equal(p: &Properties, key: &str, expected: &str) -> Result<()> {
    if prop(p, key)? != expected {
        return Err("UNIT_EFFECTIVE_CONTEXT_CHANGED");
    }
    Ok(())
}
fn exec(p: &Properties, key: &str, arguments: &[&str]) -> Result<()> {
    let raw = prop(p, key)?;
    // systemctl show добавляет runtime counters к ExecStart. Проверяются argv,
    // path и ignore_errors; весь raw остаётся связан с root snapshot и reread.
    let prefix = format!(
        "{{ path={} ; argv[]={} ; ignore_errors=no ;",
        arguments[0],
        arguments.join(" ")
    );
    if !raw.starts_with(&prefix) || !raw.ends_with(" }") || raw.matches("{ path=").count() != 1 {
        return Err("UNIT_EXEC_CHANGED");
    }
    Ok(())
}
fn canonical_manager(manager: &Manager) -> Result<()> {
    if manager.len() != UNITS.len() {
        return Err("UNIT_SET");
    }
    for unit in UNITS {
        let p = manager.get(unit).ok_or("UNIT_MISSING")?;
        if p.len() != properties(unit).len()
            || !properties(unit).iter().all(|key| p.contains_key(*key))
        {
            return Err("UNIT_PROPERTY_SET");
        }
        equal(p, "Id", unit)?;
        equal(p, "LoadState", "loaded")?;
        equal(
            p,
            "FragmentPath",
            &format!("/usr/lib/systemd/system/{unit}"),
        )?;
        for key in ["DropInPaths", "SourcePath"] {
            equal(p, key, "")?;
        }
        for key in ["Transient", "NeedDaemonReload"] {
            equal(p, key, "no")?;
        }
        if unit.ends_with(".socket") {
            let (path, service) = if unit == "yonik-worker.socket" {
                ("/run/yonik-worker/run.sock", "yonik-worker.service")
            } else {
                ("/run/yonik-worker/check.sock", "yonik-worker-check.service")
            };
            equal(p, "Listen", &format!("{path} (Stream)"))?;
            for key in ["SocketUser", "SocketGroup"] {
                equal(p, key, "root")?;
            }
            equal(p, "SocketMode", "0600")?;
            equal(p, "DirectoryMode", "0700")?;
            equal(p, "Accept", "no")?;
            equal(p, "RemoveOnStop", "yes")?;
            equal(p, "Triggers", service)?;
            continue;
        }
        let helper = unit == "yonik-helper.service";
        let check = unit == "yonik-worker-check.service";
        for (key, value) in [
            ("User", if helper { "root" } else { "yonik-worker" }),
            ("Group", if helper { "root" } else { "yonik-worker" }),
            ("Type", if helper { "dbus" } else { "exec" }),
            (
                "BusName",
                if helper {
                    "cloud.highpriority.yonik.Network1"
                } else {
                    ""
                },
            ),
            ("KillMode", "control-group"),
            ("ProtectSystem", "strict"),
            ("ProtectHome", "yes"),
            ("NoNewPrivileges", "yes"),
            ("PrivateNetwork", if check { "yes" } else { "no" }),
            ("PrivateDevices", if check || helper { "yes" } else { "no" }),
            (
                "CapabilityBoundingSet",
                if check { "" } else { "cap_net_admin" },
            ),
            (
                "AmbientCapabilities",
                if !helper && !check {
                    "cap_net_admin"
                } else {
                    ""
                },
            ),
        ] {
            equal(p, key, value)?;
        }
        for key in [
            "Environment",
            "EnvironmentFiles",
            "RootDirectory",
            "RootImage",
            "WorkingDirectory",
            "PAMName",
            "ExecStartPost",
            "ExecCondition",
            "ExecReload",
            "ExecStop",
            "ExecStopPost",
        ] {
            equal(p, key, "")?;
        }
        equal(p, "DynamicUser", "no")?;
        if helper {
            exec(p, "ExecStart", &["/usr/lib/yonik/yonik-helper"])?;
            exec(
                p,
                "ExecStartPre",
                &["/usr/lib/yonik/yonik-helper", "--check-worker-fence"],
            )?;
            equal(p, "ActiveState", "active")?;
            equal(p, "SubState", "running")?;
            equal(p, "ControlGroup", "/system.slice/yonik-helper.service")?;
        } else {
            exec(
                p,
                "ExecStart",
                &[
                    "/usr/lib/yonik/yonik-worker",
                    if check { "--check" } else { "--run" },
                ],
            )?;
            equal(p, "ExecStartPre", "")?;
            let group = prop(p, "ControlGroup")?;
            if !group.is_empty() && group != format!("/system.slice/{unit}") {
                return Err("UNIT_CGROUP_CHANGED");
            }
        }
    }
    Ok(())
}
fn validate(
    c: &Capture,
    f: &VerifiedFixture,
    case: &str,
    client_sha: &str,
    live: &Manager,
    fragments: &BTreeMap<String, String>,
) -> Result<()> {
    let p = &c.provenance;
    if c.schema != "N5_UNIT_CLOSURE_V1"
        || p.source != f.source_commit()
        || p.manifest_sha256 != f.manifest_sha256()
        || p.nonce != f.controls().capture_nonce
        || p.case != case
        || p.kernel_boot_id != f.controls().kernel_boot_id
        || p.collector_sha256 != f.controls().collector_sha256
        || p.authorization_sha256 != f.controls().capture_authorization_sha256
        || p.manager_client_sha256 != client_sha
    {
        return Err("UNIT_CLOSURE_PROVENANCE");
    }
    canonical_manager(&c.units)?;
    canonical_manager(live)?;
    if &c.units != live {
        return Err("UNIT_MANAGER_CHANGED_SINCE_ROOT_CAPTURE");
    }
    if fragments.len() != UNITS.len() {
        return Err("UNIT_FRAGMENT_SET");
    }
    for unit in UNITS {
        if f.manifest["units"][unit] != fragments[unit] {
            return Err("UNIT_FRAGMENT_CHANGED");
        }
    }
    let h = &c.helper;
    let service = &live["yonik-helper.service"];
    if h.pid <= 1
        || h.start_ticks == 0
        || h.uid != 0
        || h.exec_main_start_monotonic_usec == 0
        || h.invocation.len() != 32
        || !h.invocation.bytes().all(|v| v.is_ascii_hexdigit())
        || h.exe_sha256 != f.manifest["artifacts"]["helper"]["sha256"]
        || h.exe_size != f.manifest["artifacts"]["helper"]["size"]
        || prop(service, "MainPID")?.parse::<u32>().ok() != Some(h.pid)
        || prop(service, "ExecMainStartTimestampMonotonic")?
            .parse::<u64>()
            .ok()
            != Some(h.exec_main_start_monotonic_usec)
        || prop(service, "InvocationID")? != h.invocation
    {
        return Err("ACTUAL_HELPER_SOURCE_BINDING");
    }
    Ok(())
}

pub(super) fn verify(f: &VerifiedFixture) -> Result<()> {
    let auth = root_file(
        "/run/yonik-acceptance/authorization.json",
        LIMIT,
        Some(0o444),
    )?;
    if sha256(&auth) != f.controls().capture_authorization_sha256 {
        return Err("ROOT_CAPTURE_GRANT");
    }
    let auth = strict_json(&auth)?;
    let case = auth["case"].as_str().ok_or("UNIT_CLOSURE_CASE")?;
    let input = root_file("/run/yonik-acceptance/input.json", LIMIT, Some(0o444))?;
    if auth
        != json!({"schema":"ROOT_CAPTURE_AUTH_V1","execution":"USER_GRANTED_GUEST_CAPTURE","nonce":f.controls().capture_nonce,"case":case,"stages":["baseline","connected","stopped"],"input_sha256":sha256(&input)})
        || strict_json(&input)?
            != json!({"schema":"ROOT_CAPTURE_INPUT_V1","source":f.source_commit(),"manifest_sha256":f.manifest_sha256()})
    {
        return Err("UNIT_CLOSURE_ROOT_GRANT");
    }
    let bytes =
        root_file(CAPTURE, 1024 * 1024, Some(0o444)).map_err(|_| "UNAVAILABLE_UNIT_CLOSURE")?;
    let capture: Capture =
        serde_json::from_value(strict_json(&bytes)?).map_err(|_| "UNIT_CLOSURE_SCHEMA")?;
    let client_sha = sha256(&root_file(
        "/usr/bin/systemctl",
        64 * 1024 * 1024,
        Some(0o755),
    )?);
    let before = snapshot()?;
    let mut fragments = BTreeMap::new();
    for unit in UNITS {
        fragments.insert(
            unit.to_owned(),
            sha256(&root_file(
                &format!("/usr/lib/systemd/system/{unit}"),
                LIMIT,
                None,
            )?),
        );
    }
    validate(&capture, f, case, &client_sha, &before, &fragments)?;
    if snapshot()? != before {
        return Err("UNIT_MANAGER_CHANGED_DURING_VALIDATION");
    }
    // Проверка final files не заменяет manager: оба источника обязательны.
    for unit in UNITS {
        if sha256(&root_file(
            &format!("/usr/lib/systemd/system/{unit}"),
            LIMIT,
            None,
        )?) != fragments[unit]
        {
            return Err("UNIT_FRAGMENT_CHANGED_DURING_VALIDATION");
        }
    }
    if root_file(CAPTURE, 1024 * 1024, Some(0o444))? != bytes {
        return Err("UNIT_CAPTURE_CHANGED");
    }
    // canonical() используется и producer для manifest binding, не raw echo.
    if sha256(&canonical(&f.manifest)?) != f.manifest_sha256() {
        return Err("UNIT_MANIFEST_BINDING");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{Controls, Launch};
    use super::*;
    use serde_json::Value;
    fn fixture() -> VerifiedFixture {
        let controls:Controls=serde_json::from_value(json!({"caller_uid":1000,"source_id":1,"profile_id":1,"private_bus":"unix:path=/run/user/1000/yonik-n5-private-bus","flows":[],"kernel_boot_id":"12345678-1234-1234-1234-123456789abc","capture_nonce":"1".repeat(64),"capture_authorization_sha256":"2".repeat(64),"collector_sha256":"3".repeat(64),"flow_observer":"10.0.0.1:9000","worker_cgroup":"/sys/fs/cgroup/system.slice/yonik-worker.service","python_sha256":"4".repeat(64),"python_client_sha256":"5".repeat(64)})).unwrap();
        let manifest = json!({"source_commit":"6".repeat(40),"units":super::super::unit_pins(),"artifacts":{"helper":{"sha256":"7".repeat(64),"size":128}}});
        let manifest_sha256 = sha256(&canonical(&manifest).unwrap());
        VerifiedFixture {
            runner_sha256: "8".repeat(64),
            source_sha256: "9".repeat(64),
            capture_sha256: "a".repeat(64),
            manifest,
            launch: Launch {
                schema: "N5_TRUSTED_LAUNCH_V1".into(),
                manifest_sha256,
                after_setup: Value::Null,
                controls,
            },
        }
    }
    fn command(args: &[&str]) -> String {
        format!(
            "{{ path={} ; argv[]={} ; ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; status=0/0 }}",
            args[0],
            args.join(" ")
        )
    }
    fn live() -> Manager {
        let mut units = Manager::new();
        for unit in UNITS {
            let mut p: Properties = properties(unit)
                .iter()
                .map(|key| ((*key).to_owned(), String::new()))
                .collect();
            for (k, v) in [
                ("Id", unit),
                ("LoadState", "loaded"),
                ("Transient", "no"),
                ("NeedDaemonReload", "no"),
                ("ActiveState", "inactive"),
                ("SubState", "dead"),
            ] {
                p.insert(k.into(), v.into());
            }
            p.insert(
                "FragmentPath".into(),
                format!("/usr/lib/systemd/system/{unit}"),
            );
            if unit.ends_with(".socket") {
                let (path, service) = if unit == "yonik-worker.socket" {
                    ("/run/yonik-worker/run.sock", "yonik-worker.service")
                } else {
                    ("/run/yonik-worker/check.sock", "yonik-worker-check.service")
                };
                p.insert("Listen".into(), format!("{path} (Stream)"));
                for (k, v) in [
                    ("SocketUser", "root"),
                    ("SocketGroup", "root"),
                    ("SocketMode", "0600"),
                    ("DirectoryMode", "0700"),
                    ("Accept", "no"),
                    ("RemoveOnStop", "yes"),
                    ("Triggers", service),
                ] {
                    p.insert(k.into(), v.into());
                }
            } else {
                let helper = unit == "yonik-helper.service";
                let check = unit == "yonik-worker-check.service";
                for (k, v) in [
                    ("User", if helper { "root" } else { "yonik-worker" }),
                    ("Group", if helper { "root" } else { "yonik-worker" }),
                    ("Type", if helper { "dbus" } else { "exec" }),
                    (
                        "BusName",
                        if helper {
                            "cloud.highpriority.yonik.Network1"
                        } else {
                            ""
                        },
                    ),
                    ("KillMode", "control-group"),
                    ("NoNewPrivileges", "yes"),
                    ("ProtectSystem", "strict"),
                    ("ProtectHome", "yes"),
                    ("DynamicUser", "no"),
                    ("PrivateNetwork", if check { "yes" } else { "no" }),
                    ("PrivateDevices", if helper || check { "yes" } else { "no" }),
                    (
                        "CapabilityBoundingSet",
                        if check { "" } else { "cap_net_admin" },
                    ),
                    (
                        "AmbientCapabilities",
                        if !helper && !check {
                            "cap_net_admin"
                        } else {
                            ""
                        },
                    ),
                    ("MainPID", if helper { "10" } else { "0" }),
                    (
                        "ExecMainStartTimestampMonotonic",
                        if helper { "12345" } else { "0" },
                    ),
                ] {
                    p.insert(k.into(), v.into());
                }
                if helper {
                    p.insert("ActiveState".into(), "active".into());
                    p.insert("SubState".into(), "running".into());
                    p.insert(
                        "ControlGroup".into(),
                        "/system.slice/yonik-helper.service".into(),
                    );
                    p.insert("InvocationID".into(), "b".repeat(32));
                    p.insert(
                        "ExecStart".into(),
                        command(&["/usr/lib/yonik/yonik-helper"]),
                    );
                    p.insert(
                        "ExecStartPre".into(),
                        command(&["/usr/lib/yonik/yonik-helper", "--check-worker-fence"]),
                    );
                } else {
                    p.insert(
                        "ExecStart".into(),
                        command(&[
                            "/usr/lib/yonik/yonik-worker",
                            if check { "--check" } else { "--run" },
                        ]),
                    );
                }
            }
            units.insert(unit.into(), p);
        }
        units
    }
    fn capture(f: &VerifiedFixture, manager: Manager) -> Capture {
        Capture {
            schema: "N5_UNIT_CLOSURE_V1".into(),
            provenance: Provenance {
                source: f.source_commit().into(),
                manifest_sha256: f.manifest_sha256().into(),
                nonce: f.controls().capture_nonce.clone(),
                case: "xray_positive".into(),
                kernel_boot_id: f.controls().kernel_boot_id.clone(),
                collector_sha256: f.controls().collector_sha256.clone(),
                authorization_sha256: f.controls().capture_authorization_sha256.clone(),
                manager_client_sha256: "c".repeat(64),
            },
            units: manager,
            helper: Helper {
                pid: 10,
                start_ticks: 42,
                uid: 0,
                exe_sha256: "7".repeat(64),
                exe_size: 128,
                invocation: "b".repeat(32),
                exec_main_start_monotonic_usec: 12345,
            },
        }
    }
    fn fragments(f: &VerifiedFixture) -> BTreeMap<String, String> {
        UNITS
            .iter()
            .map(|u| {
                (
                    (*u).into(),
                    f.manifest["units"][*u].as_str().unwrap().into(),
                )
            })
            .collect()
    }
    #[test]
    fn valid_live_root_tuple_passes_before_host() {
        let f = fixture();
        let live = live();
        let capture = capture(&f, live.clone());
        assert_eq!(
            validate(
                &capture,
                &f,
                "xray_positive",
                &"c".repeat(64),
                &live,
                &fragments(&f)
            ),
            Ok(())
        );
    }
    #[test]
    fn sealed_old_receipt_same_elf_changed_unit_rejected_before_host() {
        let f = fixture();
        let live = live();
        let capture = capture(&f, live.clone());
        let mut disk = fragments(&f);
        disk.insert("yonik-helper.service".into(), "d".repeat(64));
        assert_eq!(
            validate(&capture, &f, "xray_positive", &"c".repeat(64), &live, &disk),
            Err("UNIT_FRAGMENT_CHANGED")
        );
    }
    #[test]
    fn sealed_old_receipt_same_elf_loaded_override_rejected_before_host() {
        let f = fixture();
        let mut current = live();
        let capture = capture(&f, current.clone());
        current.get_mut("yonik-helper.service").unwrap().insert(
            "DropInPaths".into(),
            "/etc/systemd/system/yonik-helper.service.d/override.conf".into(),
        );
        assert!(
            validate(
                &capture,
                &f,
                "xray_positive",
                &"c".repeat(64),
                &current,
                &fragments(&f)
            )
            .is_err()
        );
    }
    #[test]
    fn pending_daemon_reload_and_changed_effective_exec_rejected() {
        let f = fixture();
        let mut current = live();
        let capture = capture(&f, current.clone());
        current
            .get_mut("yonik-helper.service")
            .unwrap()
            .insert("NeedDaemonReload".into(), "yes".into());
        assert!(
            validate(
                &capture,
                &f,
                "xray_positive",
                &"c".repeat(64),
                &current,
                &fragments(&f)
            )
            .is_err()
        );
        let mut current = live();
        current.get_mut("yonik-helper.service").unwrap().insert(
            "ExecStart".into(),
            command(&["/usr/lib/yonik/yonik-helper", "--unexpected"]),
        );
        assert!(
            validate(
                &capture,
                &f,
                "xray_positive",
                &"c".repeat(64),
                &current,
                &fragments(&f)
            )
            .is_err()
        );
    }
    #[test]
    fn actual_helper_wrong_exe_or_reused_invocation_rejected() {
        let f = fixture();
        let current = live();
        let mut capture = capture(&f, current.clone());
        capture.helper.exe_sha256 = "d".repeat(64);
        assert_eq!(
            validate(
                &capture,
                &f,
                "xray_positive",
                &"c".repeat(64),
                &current,
                &fragments(&f)
            ),
            Err("ACTUAL_HELPER_SOURCE_BINDING")
        );
        capture.helper.exe_sha256 = "7".repeat(64);
        capture.helper.invocation = "e".repeat(32);
        assert!(
            validate(
                &capture,
                &f,
                "xray_positive",
                &"c".repeat(64),
                &current,
                &fragments(&f)
            )
            .is_err()
        );
    }
    #[test]
    fn matching_capture_with_wrong_effective_context_is_not_source_closure() {
        let f = fixture();
        let mut current = live();
        current
            .get_mut("yonik-helper.service")
            .unwrap()
            .insert("Environment".into(), "YONIK_OVERRIDE=1".into());
        let capture = capture(&f, current.clone());
        assert!(
            validate(
                &capture,
                &f,
                "xray_positive",
                &"c".repeat(64),
                &current,
                &fragments(&f)
            )
            .is_err()
        );
    }
    #[test]
    fn complete_canonical_property_reply_round_trips_without_queries() {
        let manager = live();
        for unit in UNITS {
            let mut raw = String::new();
            for (key, value) in &manager[unit] {
                raw.push_str(key);
                raw.push('=');
                raw.push_str(value);
                raw.push('\n');
            }
            assert_eq!(
                parse_properties(raw.as_bytes(), unit).unwrap(),
                manager[unit]
            );
        }
    }
    #[test]
    fn manager_unknown_duplicate_or_missing_property_is_unavailable() {
        assert!(parse_properties(b"Id=x\nId=x\n", "yonik-helper.service").is_err());
        assert!(parse_properties(b"Id=yonik-helper.service\n", "yonik-helper.service").is_err());
        assert!(parse_properties(b"Unsupported=yes\n", "yonik-helper.service").is_err());
    }
}
