//! Приватный trust boundary: только fixed root-owned sealed DATA.
#[path = "unit_closure.rs"]
mod unit_closure;
use super::hash::sha256;
use rustix::fs::{Mode, OFlags, open, openat};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs::File, io::Read, os::unix::fs::MetadataExt, path::PathBuf};
pub type Result<T> = std::result::Result<T, &'static str>;
pub const DRIVER: &str = "/usr/lib/yonik-acceptance/n5-guest-driver";
const AUTH: &str = "/usr/lib/yonik-acceptance/n5-authorization.json";
const FIXTURE: &str = "/usr/lib/yonik-acceptance/sealed-guest-fixture-v1.json";
const GRANT: &str = "/usr/lib/yonik-acceptance/n5-user-grant.json";
const LAUNCH: &str = "/usr/lib/yonik-acceptance/n5-trusted-launch.json";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Authorization {
    schema: String,
    execution: String,
    source_commit: String,
    runner_sha256: String,
    fixture_sha256: String,
    grant_sha256: String,
    launch_sha256: String,
    source_blockers: Vec<String>,
    missing: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    schema: String,
    explicit_user_runtime_grant: bool,
    source_commit: String,
    runner_sha256: String,
    fixture_sha256: String,
    launch_sha256: String,
    cases: Vec<String>,
}
#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    pub endpoint: std::net::SocketAddr,
    pub source_ip: std::net::IpAddr,
    pub expected_peer_ip: std::net::IpAddr,
    pub transport: String,
    pub application: String,
    pub action: String,
    pub token: String,
    // Ответ доверенного guest responder: nonce + реально наблюдённый route/owner.
    pub response: String,
    pub control: bool,
}
#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    pub caller_uid: u32,
    pub source_id: i64,
    pub profile_id: i64,
    pub private_bus: String,
    pub flows: Vec<Flow>,
    pub kernel_boot_id: String,
    pub capture_nonce: String,
    pub capture_authorization_sha256: String,
    pub collector_sha256: String,
    pub flow_observer: std::net::SocketAddr,
    pub worker_cgroup: String,
    pub python_sha256: String,
    pub python_client_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Launch {
    schema: String,
    manifest_sha256: String,
    after_setup: Value,
    controls: Controls,
}
// Поля доступны только внутри example; создание возможно только после verifier.
pub struct VerifiedFixture {
    runner_sha256: String,
    manifest: Value,
    launch: Launch,
    pub capture_sha256: String,
    pub source_sha256: String,
}
impl VerifiedFixture {
    pub fn source_commit(&self) -> &str {
        self.manifest["source_commit"].as_str().unwrap_or("")
    }
    pub fn manifest_sha256(&self) -> &str {
        &self.launch.manifest_sha256
    }
    pub fn capture_artifacts(&self) -> Value {
        let mut result = serde_json::Map::new();
        if let Some(artifacts) = self.manifest["artifacts"].as_object() {
            for a in artifacts.values().filter(|a| a["installed_path"] != DRIVER) {
                if let Some(path) = a["installed_path"].as_str() {
                    result.insert(
                        path.to_owned(),
                        json!({"sha256":a["sha256"],"size":a["size"]}),
                    );
                }
            }
        }
        Value::Object(result)
    }
    pub fn worker_uid(&self) -> Result<u32> {
        self.manifest["environment"]["worker_uid"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("WORKER_UID")
    }
    pub fn controls(&self) -> &Controls {
        &self.launch.controls
    }
    pub fn data(&self) -> PathBuf {
        PathBuf::from("/run/yonik-acceptance/data")
    }
    pub fn engines(&self) -> PathBuf {
        PathBuf::from("/usr/lib/yonik/engines")
    }
    pub fn verify_installed(&self) -> Result<()> {
        if sha256(&root_file(
            "/usr/lib/yonik-acceptance/runner.py",
            1024 * 1024,
            Some(0o444),
        )?) != self.runner_sha256
        {
            return Err("RUNNER_CHANGED");
        }
        let artifacts = self.manifest["artifacts"].as_object().ok_or("ARTIFACTS")?;
        for (name, artifact) in artifacts {
            let receipt = root_file(
                &format!("/usr/lib/yonik-acceptance/{name}.build.json"),
                65536,
                Some(0o444),
            )?;
            if sha256(&receipt) != artifact["build_receipt_sha256"] {
                return Err("BUILD_RECEIPT_HASH");
            }
            let build = super::protocol::strict_json(&receipt)?;
            if build
                != json!({"schema":"SOURCE_BOUND_ELF_V1","source_commit":artifact["source_commit"],"source_archive_sha256":artifact["source_archive_sha256"],"elf_sha256":artifact["sha256"]})
            {
                return Err("BUILD_SOURCE_BINDING");
            }
            let path = artifact["installed_path"].as_str().ok_or("ARTIFACT_PATH")?;
            let bytes = root_file(path, 512 * 1024 * 1024, Some(0o555))?;
            if bytes.len() as u64 != artifact["size"].as_u64().ok_or("ARTIFACT_SIZE")?
                || sha256(&bytes) != artifact["sha256"].as_str().ok_or("ARTIFACT_HASH")?
            {
                return Err("INSTALLED_ARTIFACT_CHANGED");
            }
        }
        for (path, digest) in [
            (
                "/usr/lib/yonik/native-source-v1.tar",
                self.source_sha256.as_str(),
            ),
            ("/usr/lib/yonik/native-profile-v1.json", ""),
        ] {
            let bytes = root_file(path, 512 * 1024 * 1024, Some(0o444))?;
            let expected = if digest.is_empty() {
                sha256(&canonical(&self.manifest["profile"])?)
            } else {
                digest.to_owned()
            };
            if sha256(&bytes) != expected {
                return Err("INSTALLED_DATA_CHANGED");
            }
        }
        for (path, pin) in [
            (
                "/usr/lib/yonik-acceptance/collect_observations.py",
                &self.launch.controls.collector_sha256,
            ),
            (
                "/usr/lib/yonik-acceptance/n5-python",
                &self.launch.controls.python_sha256,
            ),
            (
                "/usr/lib/yonik-acceptance/n5-flow-client.py",
                &self.launch.controls.python_client_sha256,
            ),
        ] {
            if sha256(&root_file(path, 64 * 1024 * 1024, None)?) != *pin {
                return Err("FLOW_CLIENT_CHANGED");
            }
        }
        unit_closure::verify(self)?;
        Ok(())
    }
    pub fn verify_runtime(&self) -> Result<()> {
        if sha256(&super::observe::bounded(
            "/proc/self/exe",
            512 * 1024 * 1024,
        )?) != self.manifest["artifacts"]["driver"]["sha256"]
        {
            return Err("EXECUTING_DRIVER_CHANGED");
        }
        let c = self.controls();
        if [
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "LD_AUDIT",
            "PYTHONPATH",
            "PYTHONHOME",
        ]
        .iter()
        .any(|key| std::env::var_os(key).is_some())
            || std::env::var("DBUS_SYSTEM_BUS_ADDRESS")
                .is_ok_and(|v| v != "unix:path=/run/dbus/system_bus_socket")
        {
            return Err("UNTRUSTED_LAUNCH_ENVIRONMENT");
        }

        if super::observe::bounded("/proc/sys/kernel/random/boot_id", 128)?.strip_suffix(b"\n")
            != Some(c.kernel_boot_id.as_bytes())
        {
            return Err("LIVE_BOOT_ID");
        }
        if rustix::process::getuid().as_raw() != c.caller_uid
            || std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().as_deref()
                != Some(c.private_bus.as_str())
            || std::env::var("XDG_RUNTIME_DIR").ok() != Some(format!("/run/user/{}", c.caller_uid))
        {
            return Err("GUEST_SESSION_BINDING");
        }
        if super::observe::bounded("/proc/sys/kernel/osrelease", 4096)?.strip_suffix(b"\n")
            != self.manifest["profile"]["kernel_release"]
                .as_str()
                .map(str::as_bytes)
        {
            return Err("LIVE_KERNEL_RELEASE");
        }
        let notes = super::observe::bounded("/sys/kernel/notes", 1024 * 1024)?;
        if kernel_build_id(&notes)? != self.manifest["kernel_build_id"] {
            return Err("LIVE_KERNEL_BUILD_ID");
        }
        let r = &self.launch.after_setup;
        let stat = super::observe::bounded("/proc/1/stat", 4096)?;
        if super::observe::start_time(std::str::from_utf8(&stat).map_err(|_| "PID1_STAT")?)?
            != number(&r["pid1"]["start"])?
            || sha256(&root_file(
                "/usr/lib/systemd/systemd",
                64 * 1024 * 1024,
                None,
            )?) != r["pid1"]["exe_sha256"]
        {
            return Err("LIVE_PID1");
        }
        for (name, path) in [
            ("network_namespace", "/proc/self/ns/net"),
            ("pid_namespace", "/proc/self/ns/pid"),
            ("mount_namespace", "/proc/self/ns/mnt"),
        ] {
            let m = std::fs::metadata(path).map_err(|_| "LIVE_NAMESPACE")?;
            if json!([m.dev(), m.ino()]) != r["topology"][name] {
                return Err("LIVE_NAMESPACE");
            }
        }
        self.verify_installed()
    }
}
fn kernel_build_id(notes: &[u8]) -> Result<String> {
    let mut offset = 0usize;
    while offset.checked_add(12).is_some_and(|n| n <= notes.len()) {
        let word =
            |i| u32::from_le_bytes([notes[i], notes[i + 1], notes[i + 2], notes[i + 3]]) as usize;
        let names = word(offset);
        let desc = word(offset + 4);
        let kind = word(offset + 8);
        let name = offset + 12;
        let data = name.checked_add((names + 3) & !3).ok_or("KERNEL_NOTES")?;
        let end = data
            .checked_add(desc)
            .filter(|n| *n <= notes.len())
            .ok_or("KERNEL_NOTES")?;
        if kind == 3 && notes.get(name..name + names) == Some(b"GNU\0") {
            return Ok(super::hash::hex(&notes[data..end]));
        }
        offset = data.checked_add((desc + 3) & !3).ok_or("KERNEL_NOTES")?;
    }
    Err("MISSING_KERNEL_BUILD_ID")
}
fn identity(m: &std::fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64, u32, u32, u32) {
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
        m.uid(),
        m.gid(),
        m.mode(),
    )
}
pub fn root_file(path: &str, limit: usize, mode: Option<u32>) -> Result<Vec<u8>> {
    root_file_since(path, limit, mode, 0)
}
pub fn root_file_since(
    path: &str,
    limit: usize,
    mode: Option<u32>,
    since: u128,
) -> Result<Vec<u8>> {
    let components: Vec<_> = path
        .strip_prefix('/')
        .ok_or("ABSOLUTE_PATH")?
        .split('/')
        .collect();
    if components
        .iter()
        .any(|s| s.is_empty() || *s == "." || *s == "..")
    {
        return Err("PATH_COMPONENT");
    }
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    let mut dir =
        File::from(open("/", flags | OFlags::DIRECTORY, Mode::empty()).map_err(|_| "OPEN_ROOT")?);
    for (index, name) in components.iter().enumerate() {
        let meta = dir.metadata().map_err(|_| "DIR_STAT")?;
        if !meta.is_dir() || meta.uid() != 0 || meta.gid() != 0 || meta.mode() & 0o022 != 0 {
            return Err("UNTRUSTED_ANCESTOR");
        }
        let last = index + 1 == components.len();
        let fd = openat(
            &dir,
            *name,
            if last {
                flags
            } else {
                flags | OFlags::DIRECTORY
            },
            Mode::empty(),
        )
        .map_err(|_| "SEALED_OPEN")?;
        let mut file = File::from(fd);
        if last {
            let before = file.metadata().map_err(|_| "SEALED_STAT")?;
            if !before.is_file()
                || before.uid() != 0
                || before.gid() != 0
                || before.mode() & 0o022 != 0
                || mode.is_some_and(|v| before.mode() & 0o7777 != v)
                || (mode == Some(0o444) && before.nlink() != 1)
                || (before.mtime() as i128 * 1_000_000_000 + before.mtime_nsec() as i128)
                    < since as i128
                || before.len() == 0
                || before.len() > limit as u64
            {
                return Err("SEALED_METADATA");
            }
            let mut bytes = Vec::new();
            (&mut file)
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "SEALED_READ")?;
            let after = file.metadata().map_err(|_| "SEALED_STAT")?;
            if identity(&before) != identity(&after) || bytes.len() as u64 != before.len() {
                return Err("SEALED_CHANGED");
            }
            return Ok(bytes);
        }
        dir = file;
    }
    Err("EMPTY_PATH")
}
fn canonical(value: &Value) -> Result<Vec<u8>> {
    // serde_json::Map без preserve_order сортирует ключи; ASCII pins/paths only.
    let utf8 = serde_json::to_string(value).map_err(|_| "CANONICAL_JSON")?;
    let mut ascii = String::new();
    for ch in utf8.chars() {
        if ch.is_ascii() {
            ascii.push(ch);
        } else {
            let mut pair = [0u16; 2];
            for code in ch.encode_utf16(&mut pair) {
                ascii.push_str(&format!("\\u{code:04x}"));
            }
        }
    }
    Ok(ascii.into_bytes())
}
fn pin(value: &Value) -> Result<&str> {
    let text = value.as_str().ok_or("MISSING_PIN")?;
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || text.bytes().all(|b| b == b'0')
    {
        return Err("INVALID_PIN");
    }
    Ok(text)
}
fn exact_keys(value: &Value, keys: &[&str]) -> Result<()> {
    let object = value.as_object().ok_or("OBJECT")?;
    if object.len() != keys.len() || !keys.iter().all(|k| object.contains_key(*k)) {
        return Err("UNKNOWN_OR_MISSING_FIELD");
    }
    Ok(())
}
fn filled(value: &Value) -> Result<()> {
    match value {
        Value::Null | Value::Bool(_) => Err("MISSING_RECEIPT"),
        Value::Array(values) => {
            for v in values {
                filled(v)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for v in values.values() {
                filled(v)?;
            }
            Ok(())
        }
        Value::Number(v) if v.as_u64().is_none() => Err("INTEGER_REQUIRED"),
        _ => Ok(()),
    }
}
fn number(value: &Value) -> Result<u64> {
    value.as_u64().filter(|v| *v > 0).ok_or("POSITIVE_INTEGER")
}

// Чистая часть fixture.py validate_receipt: все tuple/ancestor/closure bindings.
pub fn validate_receipt(manifest: &Value, r: &Value) -> Result<()> {
    filled(manifest)?;
    filled(r)?;
    exact_keys(
        manifest,
        &[
            "schema",
            "source_commit",
            "source_archive_sha256",
            "guest_base_sha256",
            "guest_provenance_sha256",
            "kernel_build_id",
            "coverage_policy_sha256",
            "artifacts",
            "units",
            "profile",
            "environment",
        ],
    )?;
    if manifest["schema"] != "SEALED_GUEST_FIXTURE_V1" {
        return Err("FIXTURE_SCHEMA");
    }
    exact_keys(
        r,
        &[
            "schema",
            "manifest_sha256",
            "guest_base_sha256",
            "guest_provenance_sha256",
            "setup_finished_ns",
            "captured_ns",
            "source_archive_sha256",
            "kernel",
            "files",
            "units",
            "profile",
            "profile_registration",
            "pid1",
            "system_bus",
            "identities",
            "cgroup",
            "topology",
        ],
    )?;
    if r["schema"] != "SEALED_GUEST_RECEIPT_V1"
        || r["manifest_sha256"] != sha256(&canonical(manifest)?)
        || number(&r["captured_ns"])? <= number(&r["setup_finished_ns"])?
    {
        return Err("RECEIPT_BINDING");
    }
    for key in [
        "guest_base_sha256",
        "guest_provenance_sha256",
        "source_archive_sha256",
        "units",
        "profile",
    ] {
        if r[key] != manifest[key] {
            return Err("TUPLE_MISMATCH");
        }
    }
    for key in [
        "source_archive_sha256",
        "guest_base_sha256",
        "guest_provenance_sha256",
        "coverage_policy_sha256",
    ] {
        pin(&manifest[key])?;
    }
    if r["kernel"]
        != json!({"build_id":manifest["kernel_build_id"],"release":manifest["profile"]["kernel_release"],"machine":"x86_64"})
    {
        return Err("KERNEL_TUPLE");
    }
    if manifest["coverage_policy_sha256"]
        != "bd5d7474108a2a1e051734ea4c9097d43380341025ed9cd60e0f1442527d2983"
    {
        return Err("FROZEN_POLICY");
    }
    let units = unit_pins();
    if manifest["units"] != units {
        return Err("CANONICAL_UNITS");
    }
    let profile = &manifest["profile"];
    exact_keys(
        profile,
        &[
            "schema",
            "worker_sha256",
            "source_archive_sha256",
            "policy_id",
            "policy_sha256",
            "kernel_release",
            "kernel_build_id",
            "kernel_machine",
            "kernel_abi",
        ],
    )?;
    if profile["schema"] != 1
        || profile["policy_id"] != "yonik-own-namespace-fixed16-v1"
        || profile["kernel_machine"] != "x86_64"
        || profile["kernel_abi"] != "linux-native-abi260-ynkch004-ynkns004-v1"
    {
        return Err("UNSUPPORTED_PROFILE");
    }
    for (name, expected, len) in [
        (
            "worker_sha256",
            &manifest["artifacts"]["worker"]["sha256"],
            32,
        ),
        (
            "source_archive_sha256",
            &manifest["source_archive_sha256"],
            32,
        ),
        ("policy_sha256", &manifest["coverage_policy_sha256"], 32),
        ("kernel_build_id", &manifest["kernel_build_id"], 20),
    ] {
        let bytes = profile[name].as_array().ok_or("PROFILE_BYTES")?;
        if bytes.len() != len {
            return Err("PROFILE_BYTES");
        }
        let mut hex = String::new();
        for b in bytes {
            let n = b.as_u64().filter(|n| *n <= 255).ok_or("PROFILE_BYTE")?;
            hex.push_str(&format!("{n:02x}"));
        }
        if &json!(hex) != expected {
            return Err("PROFILE_BINDING");
        }
    }
    for key in ["source_commit", "kernel_build_id"] {
        let value = manifest[key].as_str().ok_or("SOURCE_KERNEL_PIN")?;
        if value.len() != 40
            || !value.bytes().all(|v| v.is_ascii_hexdigit())
            || value.bytes().all(|v| v == b'0')
        {
            return Err("SOURCE_KERNEL_PIN");
        }
    }
    exact_keys(
        &manifest["environment"],
        &[
            "pid1_sha256",
            "worker_uid",
            "worker_gid",
            "topology_sha256",
            "uid_caps_sha256",
            "cgroup_controllers_sha256",
        ],
    )?;
    for key in [
        "pid1_sha256",
        "topology_sha256",
        "uid_caps_sha256",
        "cgroup_controllers_sha256",
    ] {
        pin(&manifest["environment"][key])?;
    }
    for key in ["worker_uid", "worker_gid"] {
        if manifest["environment"][key]
            .as_u64()
            .is_none_or(|n| n == 0 || n >= u32::MAX as u64)
        {
            return Err("WORKER_UID_RANGE");
        }
    }
    exact_keys(
        &r["files"],
        &[
            "helper", "worker", "desktop", "core_a", "core_b", "driver", "source", "profile",
        ],
    )?;
    let artifacts = manifest["artifacts"].as_object().ok_or("ARTIFACTS")?;
    let paths = [
        ("helper", "/usr/lib/yonik/yonik-helper"),
        ("worker", "/usr/lib/yonik/yonik-worker"),
        ("desktop", "/usr/bin/yonik-desktop"),
        ("core_a", "/usr/lib/yonik/engines/sing-box"),
        ("core_b", "/usr/lib/yonik/engines/xray"),
        ("driver", DRIVER),
    ];
    if artifacts.len() != paths.len() {
        return Err("ARTIFACT_SET");
    }
    for (name, path) in paths {
        let a = &manifest["artifacts"][name];
        let f = &r["files"][name];
        exact_keys(
            a,
            &[
                "sha256",
                "size",
                "source_archive_sha256",
                "source_commit",
                "build_receipt_sha256",
                "installed_path",
                "uid",
                "gid",
                "mode",
            ],
        )?;
        exact_keys(
            f,
            &[
                "sha256",
                "size",
                "installed_path",
                "uid",
                "gid",
                "mode",
                "kind",
                "ancestors",
            ],
        )?;
        for key in ["sha256", "source_archive_sha256", "build_receipt_sha256"] {
            pin(&a[key])?;
        }
        if a["installed_path"] != path
            || a["uid"] != 0
            || a["gid"] != 0
            || a["mode"] != 0o555
            || number(&a["size"])? > 512 * 1024 * 1024
        {
            return Err("ELF_METADATA");
        }
        // CORE engines могут иметь собственную provenance, first-party ELF — exact source.
        if ["helper", "worker", "desktop", "driver"].contains(&name)
            && (a["source_commit"] != manifest["source_commit"]
                || a["source_archive_sha256"] != manifest["source_archive_sha256"])
        {
            return Err("SOURCE_CLOSURE");
        }
        for key in ["sha256", "size", "installed_path", "uid", "gid", "mode"] {
            if f[key] != a[key] {
                return Err("ELF_RECEIPT");
            }
        }
        if f["kind"] != "regular" {
            return Err("ELF_KIND");
        }
        let parents: Vec<_> = std::path::Path::new(path).ancestors().skip(1).collect();
        let actual = f["ancestors"].as_array().ok_or("ANCESTORS")?;
        if actual.len() != parents.len() {
            return Err("ANCESTOR_COUNT");
        }
        for (a, p) in actual.iter().zip(parents) {
            exact_keys(a, &["path", "uid", "gid", "mode", "kind"])?;
            if a["path"] != p.to_str().ok_or("UTF8")?
                || a["uid"] != 0
                || a["gid"] != 0
                || a["kind"] != "directory"
                || a["mode"]
                    .as_u64()
                    .is_none_or(|n| n > 0o7777 || n & 0o022 != 0)
            {
                return Err("ANCESTOR_TRUST");
            }
        }
    }
    for (name, path, hash) in [
        (
            "source",
            "/usr/lib/yonik/native-source-v1.tar",
            manifest["source_archive_sha256"].clone(),
        ),
        (
            "profile",
            "/usr/lib/yonik/native-profile-v1.json",
            json!(sha256(&canonical(&manifest["profile"])?)),
        ),
    ] {
        if r["files"][name]
            != json!({"installed_path":path,"sha256":hash,"uid":0,"gid":0,"mode":0o444,"kind":"regular"})
        {
            return Err("DATA_RECEIPT");
        }
    }
    if r["profile_registration"]
        != json!({"scope":"OWN_GUEST_ONLY","grant_manifest_sha256":r["manifest_sha256"],"grant_source_sha256":manifest["source_archive_sha256"]})
    {
        return Err("REGISTRATION_BINDING");
    }
    let env = &manifest["environment"];
    let pid = &r["pid1"];
    exact_keys(pid, &["pid", "uid", "comm", "exe_sha256", "start"])?;
    if pid["pid"] != 1
        || pid["uid"] != 0
        || pid["comm"] != "systemd"
        || pid["exe_sha256"] != env["pid1_sha256"]
    {
        return Err("PID1_BINDING");
    }
    number(&pid["start"])?;
    let bus = &r["system_bus"];
    exact_keys(
        bus,
        &["address", "uid", "pid", "start", "socket_inode", "owner"],
    )?;
    let owner = bus["owner"]
        .as_str()
        .and_then(|s| s.strip_prefix(":1."))
        .ok_or("BUS_OWNER")?;
    if bus["address"] != "unix:path=/run/dbus/system_bus_socket"
        || bus["uid"] != 0
        || owner.is_empty()
        || !owner.bytes().all(|v| v.is_ascii_digit())
    {
        return Err("SYSTEM_BUS");
    }
    for key in ["pid", "start", "socket_inode"] {
        number(&bus[key])?;
    }
    let ids = &r["identities"];
    exact_keys(
        ids,
        &[
            "helper_uid",
            "worker_uid",
            "worker_gid",
            "uid_caps_sha256",
            "capture",
        ],
    )?;
    if ids["helper_uid"] != 0
        || ids["worker_uid"] != env["worker_uid"]
        || ids["worker_gid"] != env["worker_gid"]
        || ids["uid_caps_sha256"] != env["uid_caps_sha256"]
    {
        return Err("IDENTITY_BINDING");
    }
    let cg = &r["cgroup"];
    exact_keys(
        cg,
        &[
            "version",
            "helper",
            "worker",
            "controllers_sha256",
            "capture",
        ],
    )?;
    if cg["version"] != 2
        || cg["helper"] != "/system.slice/yonik-helper.service"
        || cg["worker"] != "/system.slice/yonik-worker.service"
        || cg["controllers_sha256"] != env["cgroup_controllers_sha256"]
    {
        return Err("CGROUP_BINDING");
    }
    let topology = &r["topology"];
    exact_keys(
        topology,
        &[
            "sha256",
            "network_namespace",
            "pid_namespace",
            "mount_namespace",
            "capture",
        ],
    )?;
    if topology["sha256"] != env["topology_sha256"] {
        return Err("TOPOLOGY_BINDING");
    }
    for (capture, expected) in [
        (&ids["capture"], &env["uid_caps_sha256"]),
        (&cg["capture"], &env["cgroup_controllers_sha256"]),
        (&topology["capture"], &env["topology_sha256"]),
    ] {
        let raw = capture.as_str().ok_or("CAPTURE")?;
        if raw.is_empty() || raw.len() > 1024 * 1024 || sha256(raw.as_bytes()) != pin(expected)? {
            return Err("CAPTURE_HASH");
        }
    }
    for key in ["network_namespace", "pid_namespace", "mount_namespace"] {
        let pair = topology[key].as_array().ok_or("NAMESPACE")?;
        if pair.len() != 2 {
            return Err("NAMESPACE_PAIR");
        }
        for n in pair {
            number(n)?;
        }
    }
    Ok(())
}

fn unit_pins() -> Value {
    json!({
        "yonik-helper.service":"d1fc8b14578e7ed96a68208794bcd4320dbb043c6223fa6088cccb0d05056c4b",
        "yonik-worker.service":"a6669a5c1be21b70a5dc950611881e8fa823976660993840724a4b2ae497e3d0",
        "yonik-worker.socket":"80f5d6aaeb742c0da4ab58944a7977ba4c802cbf5e025e98cb46bb8d7a2d02d4",
        "yonik-worker-check.service":"14966746029044e65691b0529c8a4e849eb04babf4a6185a83c1181047b40424",
        "yonik-worker-check.socket":"4c667e20d0ebc738193f3bdd3b1b0beff7d38930d45c8d86deb879b841a4e904"})
}

pub fn verify() -> Result<VerifiedFixture> {
    // Только эти четыре root-owned fixed inputs читаются до verified token.
    let auth: Authorization = serde_json::from_value(super::protocol::strict_json(&root_file(
        AUTH,
        65536,
        Some(0o444),
    )?)?)
    .map_err(|_| "AUTH_PROTOCOL")?;
    if auth.schema != "N5_AUTHORIZATION_V1"
        || auth.execution != "AUTHORIZED"
        || !auth.source_blockers.is_empty()
        || !auth.missing.is_empty()
    {
        return Err("EXECUTION_CLOSED");
    }
    let fixture = root_file(FIXTURE, 1024 * 1024, Some(0o444))?;
    let grant = root_file(GRANT, 65536, Some(0o444))?;
    let launch = root_file(LAUNCH, 4 * 1024 * 1024, Some(0o444))?;
    verify_material(auth, &fixture, &grant, &launch)
}
fn verify_material(
    auth: Authorization,
    fixture: &[u8],
    grant: &[u8],
    launch: &[u8],
) -> Result<VerifiedFixture> {
    if auth.schema != "N5_AUTHORIZATION_V1"
        || auth.execution != "AUTHORIZED"
        || !auth.source_blockers.is_empty()
        || !auth.missing.is_empty()
    {
        return Err("EXECUTION_CLOSED");
    }
    for (bytes, expected) in [
        (&fixture, &auth.fixture_sha256),
        (&grant, &auth.grant_sha256),
        (&launch, &auth.launch_sha256),
    ] {
        pin(&json!(expected))?;
        if sha256(bytes) != *expected {
            return Err("SEALED_PIN");
        }
    }
    let grant: Grant = serde_json::from_value(super::protocol::strict_json(grant)?)
        .map_err(|_| "GRANT_PROTOCOL")?;
    let launch: Launch = serde_json::from_value(super::protocol::strict_json(launch)?)
        .map_err(|_| "LAUNCH_PROTOCOL")?;
    let manifest: Value = super::protocol::strict_json(fixture)?;
    if grant.schema != "N5_USER_GRANT_V1"
        || !grant.explicit_user_runtime_grant
        || grant.source_commit != auth.source_commit
        || manifest["source_commit"] != auth.source_commit
        || grant.runner_sha256 != auth.runner_sha256
        || grant.fixture_sha256 != auth.fixture_sha256
        || grant.launch_sha256 != auth.launch_sha256
        || launch.schema != "N5_TRUSTED_LAUNCH_V1"
        || launch.manifest_sha256 != sha256(&canonical(&manifest)?)
    {
        return Err("GRANT_BINDING");
    }
    let cases: Vec<String> = ["xray", "sing-box"]
        .iter()
        .flat_map(|core| {
            [
                "positive",
                "missing-profile",
                "tampered-profile",
                "tampered-report",
                "revoke",
                "cancel-before-handoff",
                "cancel-after-handoff",
                "host-loss",
                "unknown-stop",
            ]
            .iter()
            .map(move |case| format!("{core}:{case}"))
        })
        .collect();
    if grant.cases != cases {
        return Err("CASE_GRANT");
    }
    validate_receipt(&manifest, &launch.after_setup)?;
    validate_controls(&launch.controls)?;
    Ok(VerifiedFixture {
        source_sha256: pin(&manifest["source_archive_sha256"])?.to_owned(),
        runner_sha256: auth.runner_sha256,
        manifest,
        launch,
        capture_sha256: auth.launch_sha256,
    })
}
fn validate_controls(c: &Controls) -> Result<()> {
    if c.caller_uid == 0
        || c.source_id <= 0
        || c.profile_id <= 0
        || c.worker_cgroup != "/sys/fs/cgroup/system.slice/yonik-worker.service"
        || c.private_bus != format!("unix:path=/run/user/{}/yonik-n5-private-bus", c.caller_uid)
        || c.flows.is_empty()
        || c.flows.len() > 32
    {
        return Err("CONTROLS");
    }
    for flow in &c.flows {
        if !["tcp", "udp"].contains(&flow.transport.as_str())
            || !["rust", "python"].contains(&flow.application.as_str())
            || !["proxy", "direct", "block"].contains(&flow.action.as_str())
            || flow.endpoint.port() == 0
            || flow.source_ip.is_unspecified()
            || flow.expected_peer_ip.is_unspecified()
            || (flow.action == "direct" && flow.source_ip != flow.expected_peer_ip)
            || (flow.action == "proxy" && flow.source_ip == flow.expected_peer_ip)
            || flow.token.is_empty()
            || flow.token.len() > 128
            || flow.response.is_empty()
            || flow.response.len() > 512
        {
            return Err("FLOW_INPUT");
        }
    }
    for transport in ["tcp", "udp"] {
        for app in ["rust", "python"] {
            for action in ["proxy", "direct", "block"] {
                if !c
                    .flows
                    .iter()
                    .any(|f| f.transport == transport && f.application == app && f.action == action)
                {
                    return Err("FLOW_MATRIX");
                }
            }
        }
    }
    for blocked in c.flows.iter().filter(|f| f.action == "block") {
        if !c.flows.iter().any(|f| {
            f.control
                && f.action != "block"
                && f.endpoint == blocked.endpoint
                && f.transport == blocked.transport
                && f.application != blocked.application
        }) {
            return Err("BLOCK_WITHOUT_ALLOW_CONTROL");
        }
    }
    pin(&json!(c.capture_nonce))?;
    pin(&json!(c.capture_authorization_sha256))?;
    pin(&json!(c.collector_sha256))?;
    pin(&json!(c.python_sha256))?;
    pin(&json!(c.python_client_sha256))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn null_fixture_never_constructs_proof() {
        assert!(
            validate_receipt(&json!({"schema":"SEALED_GUEST_FIXTURE_V1"}), &Value::Null).is_err()
        );
    }
    #[test]
    fn noninteger_receipt_rejected() {
        assert!(filled(&json!({"pid":true})).is_err());
        assert!(filled(&json!({"pid":1.0})).is_err());
    }
    #[test]
    fn source_echo_is_not_receipt() {
        assert!(
            validate_receipt(&json!({}), &json!({"schema":"SEALED_GUEST_RECEIPT_V1"})).is_err()
        );
    }
}

#[cfg(test)]
mod material_tests {
    use super::*;
    fn example() -> (Value, Value) {
        let mut m: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/proposals/n5-guest-acceptance/manifest.pending.json"
        )))
        .unwrap();
        for key in [
            "source_archive_sha256",
            "guest_base_sha256",
            "guest_provenance_sha256",
        ] {
            m[key] = json!("a".repeat(64));
        }
        m["source_commit"] = json!("c".repeat(40));
        m["kernel_build_id"] = json!("a".repeat(40));
        m["coverage_policy_sha256"] =
            json!("bd5d7474108a2a1e051734ea4c9097d43380341025ed9cd60e0f1442527d2983");
        m["units"] = unit_pins();
        m["artifacts"]["driver"] = json!({"installed_path":DRIVER,"uid":0,"gid":0,"mode":0o555});
        for a in m["artifacts"].as_object_mut().unwrap().values_mut() {
            for key in ["sha256", "source_archive_sha256", "build_receipt_sha256"] {
                a[key] = json!("a".repeat(64));
            }
            a["source_commit"] = json!("c".repeat(40));
            a["size"] = json!(128);
        }
        for (field, expected) in [
            ("worker_sha256", m["artifacts"]["worker"]["sha256"].clone()),
            ("source_archive_sha256", m["source_archive_sha256"].clone()),
            ("policy_sha256", m["coverage_policy_sha256"].clone()),
            ("kernel_build_id", m["kernel_build_id"].clone()),
        ] {
            let text = expected.as_str().unwrap();
            let bytes = (0..text.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
                .collect::<Vec<_>>();
            m["profile"][field] = json!(bytes);
        }
        m["profile"]["kernel_release"] = json!("6.12-test");
        for key in [
            "uid_caps_sha256",
            "topology_sha256",
            "cgroup_controllers_sha256",
        ] {
            m["environment"][key] = json!(sha256(b"capture"));
        }
        m["environment"]["pid1_sha256"] = json!("a".repeat(64));
        m["environment"]["worker_uid"] = json!(997);
        m["environment"]["worker_gid"] = json!(997);
        let mut files = serde_json::Map::new();
        for a in m["artifacts"].as_object().unwrap().values() {
            let path = a["installed_path"].as_str().unwrap();
            let ancestors=std::path::Path::new(path).ancestors().skip(1).map(|p|json!({"path":p.to_str().unwrap(),"uid":0,"gid":0,"mode":0o755,"kind":"directory"})).collect::<Vec<_>>();
            let name = m["artifacts"]
                .as_object()
                .unwrap()
                .iter()
                .find(|(_, v)| *v == a)
                .unwrap()
                .0
                .clone();
            // Совпадающие hashes допустимы; installed_path делает artifacts различными.
            files.insert(name,json!({"sha256":a["sha256"],"size":128,"installed_path":path,"uid":0,"gid":0,"mode":0o555,"kind":"regular","ancestors":ancestors}));
        }
        files.insert("source".into(),json!({"installed_path":"/usr/lib/yonik/native-source-v1.tar","sha256":m["source_archive_sha256"],"uid":0,"gid":0,"mode":0o444,"kind":"regular"}));
        files.insert("profile".into(),json!({"installed_path":"/usr/lib/yonik/native-profile-v1.json","sha256":sha256(&canonical(&m["profile"]).unwrap()),"uid":0,"gid":0,"mode":0o444,"kind":"regular"}));
        let r = json!({"schema":"SEALED_GUEST_RECEIPT_V1","manifest_sha256":sha256(&canonical(&m).unwrap()),"guest_base_sha256":m["guest_base_sha256"],"guest_provenance_sha256":m["guest_provenance_sha256"],"setup_finished_ns":1,"captured_ns":2,"source_archive_sha256":m["source_archive_sha256"],"kernel":{"build_id":m["kernel_build_id"],"release":"6.12-test","machine":"x86_64"},"files":files,"units":m["units"],"profile":m["profile"],"profile_registration":{"scope":"OWN_GUEST_ONLY","grant_manifest_sha256":sha256(&canonical(&m).unwrap()),"grant_source_sha256":m["source_archive_sha256"]},"pid1":{"pid":1,"uid":0,"comm":"systemd","exe_sha256":m["environment"]["pid1_sha256"],"start":1},"system_bus":{"address":"unix:path=/run/dbus/system_bus_socket","uid":0,"pid":3,"start":2,"socket_inode":10,"owner":":1.4"},"identities":{"helper_uid":0,"worker_uid":997,"worker_gid":997,"uid_caps_sha256":sha256(b"capture"),"capture":"capture"},"cgroup":{"version":2,"helper":"/system.slice/yonik-helper.service","worker":"/system.slice/yonik-worker.service","controllers_sha256":sha256(b"capture"),"capture":"capture"},"topology":{"sha256":sha256(b"capture"),"network_namespace":[1,2],"pid_namespace":[1,3],"mount_namespace":[1,4],"capture":"capture"}});
        (m, r)
    }
    #[test]
    fn valid_in_memory_material_constructs_private_fixture_without_io() {
        let (m, r) = example();
        let mut flows = Vec::new();
        for transport in ["tcp", "udp"] {
            for app in ["rust", "python"] {
                for action in ["proxy", "direct", "block"] {
                    let port = if (action == "block" && app == "rust")
                        || (action == "direct" && app == "python")
                    {
                        100
                    } else {
                        101
                    };
                    flows.push(json!({"endpoint":format!("10.0.0.4:{port}"),"source_ip":"10.0.0.2","expected_peer_ip":if action=="proxy"{"10.0.0.3"}else{"10.0.0.2"},"transport":transport,"application":app,"action":action,"token":"flow","response":"endpoint","control":action=="direct"}));
                }
            }
        }
        let launch = json!({"schema":"N5_TRUSTED_LAUNCH_V1","manifest_sha256":sha256(&canonical(&m).unwrap()),"after_setup":r,"controls":{"caller_uid":1000,"source_id":1,"profile_id":1,"private_bus":"unix:path=/run/user/1000/yonik-n5-private-bus","flows":flows,"flow_observer":"10.0.0.5:9000","worker_cgroup":"/sys/fs/cgroup/system.slice/yonik-worker.service","python_sha256":"a".repeat(64),"python_client_sha256":"a".repeat(64),"collector_sha256":"a".repeat(64),"capture_nonce":"a".repeat(64),"capture_authorization_sha256":"a".repeat(64),"kernel_boot_id":"12345678-1234-1234-1234-123456789abc"}});
        let fixture = canonical(&m).unwrap();
        let launch = canonical(&launch).unwrap();
        let cases: Vec<String> = ["xray", "sing-box"]
            .iter()
            .flat_map(|core| {
                [
                    "positive",
                    "missing-profile",
                    "tampered-profile",
                    "tampered-report",
                    "revoke",
                    "cancel-before-handoff",
                    "cancel-after-handoff",
                    "host-loss",
                    "unknown-stop",
                ]
                .iter()
                .map(move |case| format!("{core}:{case}"))
            })
            .collect();
        let grant = json!({"schema":"N5_USER_GRANT_V1","explicit_user_runtime_grant":true,"source_commit":m["source_commit"],"runner_sha256":"a".repeat(64),"fixture_sha256":sha256(&fixture),"launch_sha256":sha256(&launch),"cases":cases});
        let grant = canonical(&grant).unwrap();
        let auth = Authorization {
            schema: "N5_AUTHORIZATION_V1".into(),
            execution: "AUTHORIZED".into(),
            source_commit: "c".repeat(40),
            runner_sha256: "a".repeat(64),
            fixture_sha256: sha256(&fixture),
            launch_sha256: sha256(&launch),
            grant_sha256: sha256(&grant),
            source_blockers: vec![],
            missing: vec![],
        };
        assert!(verify_material(auth, &fixture, &grant, &launch).is_ok());
    }
    #[test]
    fn valid_full_receipt_is_admissible_not_constant_deny() {
        let (m, r) = example();
        assert_eq!(validate_receipt(&m, &r), Ok(()));
    }
    #[test]
    fn wrong_profile_and_writable_ancestor_are_rejected() {
        let (m, mut r) = example();
        r["files"]["driver"]["ancestors"][0]["mode"] = json!(0o777);
        assert!(validate_receipt(&m, &r).is_err());
    }
    #[test]
    fn canonical_unicode_matches_python_ascii() {
        assert_eq!(canonical(&json!({"x":"я"})).unwrap(), br#"{"x":"\u044f"}"#);
    }
}
