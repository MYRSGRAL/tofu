//! Root collector consumer. Не запускает collector и не создаёт ACTUAL capture.
use super::{
    hash::sha256,
    protocol::strict_json,
    sealed::{self, Result, VerifiedFixture},
};
use serde_json::{json, Value};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
pub fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(u128::MAX, |v| v.as_nanos())
}
pub fn read(stage: &str, core: &str, since: u128, f: &VerifiedFixture) -> Result<Value> {
    if !["baseline", "connected", "stopped"].contains(&stage) {
        return Err("CAPTURE_STAGE");
    }
    let case = format!("{}_positive", core.replace('-', "_"));
    let auth = sealed::root_file(
        "/run/yonik-acceptance/authorization.json",
        65536,
        Some(0o444),
    )?;
    if sha256(&auth) != f.controls().capture_authorization_sha256 {
        return Err("ROOT_CAPTURE_GRANT");
    }
    let authorization = strict_json(&auth)?;
    let input = sealed::root_file("/run/yonik-acceptance/input.json", 65536, Some(0o444))?;
    if authorization
        != json!({"schema":"ROOT_CAPTURE_AUTH_V1","execution":"USER_GRANTED_GUEST_CAPTURE","nonce":f.controls().capture_nonce,"case":case,"stages":["baseline","connected","stopped"],"input_sha256":sha256(&input)})
        || strict_json(&input)?
            != json!({"schema":"ROOT_CAPTURE_INPUT_V1","source":f.source_commit(),"manifest_sha256":f.manifest_sha256()})
    {
        return Err("ROOT_CAPTURE_BINDING");
    }
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(bytes) = sealed::root_file_since(
            &format!("/run/yonik-acceptance/capture-{stage}.json"),
            1024 * 1024,
            Some(0o444),
            since,
        ) {
            let value = strict_json(&bytes)?;
            validate(
                &value,
                stage,
                &case,
                &f.controls().capture_nonce,
                f.source_commit(),
                f.manifest_sha256(),
                &f.controls().capture_authorization_sha256,
            )?;
            if value["provenance"]["collector_sha256"] != f.controls().collector_sha256
                || value["provenance"]["kernel_boot_id"] != f.controls().kernel_boot_id
            {
                return Err("CAPTURE_BOOT_OR_COLLECTOR");
            }
            if value["observations"]["artifacts"] != f.capture_artifacts() {
                return Err("CAPTURE_ELF_TUPLE");
            }
            return Ok(value);
        }
        if Instant::now() >= end {
            return Err("UNAVAILABLE_ROOT_CAPTURE");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
pub fn validate(
    v: &Value,
    stage: &str,
    case: &str,
    nonce: &str,
    source: &str,
    manifest: &str,
    auth: &str,
) -> Result<()> {
    let top = v.as_object().ok_or("CAPTURE_OBJECT")?;
    if top.len() != 5
        || ![
            "schema",
            "status",
            "provenance",
            "observations",
            "inference",
        ]
        .iter()
        .all(|k| top.contains_key(*k))
        || v["schema"] != 1
        || v["status"] != "ACTUAL"
    {
        return Err("UNAVAILABLE_ROOT_CAPTURE");
    }
    let p = &v["provenance"];
    if p.as_object().is_none_or(|p| p.len() != 9)
        || p["source"] != source
        || p["manifest_sha256"] != manifest
        || p["case"] != case
        || p["stage"] != stage
        || p["nonce"] != nonce
        || p["authorization_sha256"] != auth
        || p["collector_pid"].as_u64().is_none_or(|pid| pid == 0)
        || p["collector_pid"] != v["observations"]["collector"]["pid"]
    {
        return Err("CAPTURE_PROVENANCE");
    }
    let o = &v["observations"];
    let keys = if stage == "stopped" {
        vec![
            "collector",
            "units",
            "network",
            "artifacts",
            "tracked_processes",
        ]
    } else {
        vec!["collector", "units", "network", "artifacts"]
    };
    if o.as_object()
        .is_none_or(|o| o.len() != keys.len() || !keys.iter().all(|k| o.contains_key(*k)))
    {
        return Err("CAPTURE_OBSERVATIONS");
    }
    if o["collector"]["start_ticks"]
        .as_u64()
        .is_none_or(|n| n == 0)
        || o["collector"]["uids"] != json!([0, 0, 0, 0])
    {
        return Err("COLLECTOR_NOT_ROOT");
    }
    let network = o["network"].as_object().ok_or("ROOT_NETWORK")?;
    if network.len() != 7 {
        return Err("ROOT_NETWORK");
    }
    for key in [
        "links",
        "addresses",
        "routes",
        "rules",
        "routes6",
        "rules6",
        "nft",
    ] {
        let values = network
            .get(key)
            .and_then(Value::as_array)
            .ok_or("ROOT_NETWORK")?;
        for hash in values {
            let s = hash.as_str().ok_or("ROOT_NETWORK_HASH")?;
            if s.len() != 64 || !s.bytes().all(|v| v.is_ascii_hexdigit()) {
                return Err("ROOT_NETWORK_HASH");
            }
        }
    }
    let units = o["units"].as_object().ok_or("ROOT_UNITS")?;
    if units.len() != 2
        || !units.contains_key("yonik-worker.service")
        || !units.contains_key("yonik-helper.service")
    {
        return Err("ROOT_UNITS");
    }
    for name in ["yonik-helper.service", "yonik-worker.service"] {
        let unit = &o["units"][name];
        if unit.as_object().is_none_or(|m| {
            m.len() != 5
                || !["main", "invocation", "cgroup", "populated", "members"]
                    .iter()
                    .all(|k| m.contains_key(*k))
        }) {
            return Err("UNIT_FIELDS");
        }
        let invocation = unit["invocation"].as_str().ok_or("UNIT_INVOCATION")?;
        if !invocation.is_empty()
            && (invocation.len() != 32 || !invocation.bytes().all(|v| v.is_ascii_hexdigit()))
        {
            return Err("UNIT_INVOCATION");
        }
        if unit["cgroup"] != "" && unit["cgroup"] != format!("/system.slice/{name}") {
            return Err("UNIT_CGROUP");
        }
        if !unit["populated"].is_null() && unit["populated"].as_u64().is_none_or(|n| n > 1) {
            return Err("UNIT_POPULATED");
        }
        if !unit["main"].is_null() {
            process(&unit["main"])?;
        }
        let members = unit["members"].as_array().ok_or("UNIT_MEMBERS")?;
        if members.len() > 256 {
            return Err("UNIT_MEMBER_BOUND");
        }
        for member in members {
            process(member)?;
        }
    }
    if stage == "stopped" {
        let tracked = o["tracked_processes"]
            .as_array()
            .ok_or("TRACKED_PROCESSES")?;
        if tracked.len() > 512 {
            return Err("TRACKED_BOUND");
        }
        for p in tracked {
            if p.as_object().is_none_or(|p| p.len() != 3)
                || p["pid"]
                    .as_u64()
                    .is_none_or(|n| n == 0 || n >= i32::MAX as u64)
                || p["start_ticks"].as_u64().is_none_or(|n| n == 0)
                || !p["absent"].is_boolean()
            {
                return Err("TRACKED_PROCESS_TYPE");
            }
        }
    }
    Ok(())
}
fn process(p: &Value) -> Result<()> {
    if p.as_object().is_none_or(|p| p.len() != 3)
        || p["pid"]
            .as_u64()
            .is_none_or(|n| n == 0 || n >= i32::MAX as u64)
        || p["start_ticks"].as_u64().is_none_or(|n| n == 0)
    {
        return Err("PROCESS_IDENTITY");
    }
    let ids = p["uids"]
        .as_array()
        .filter(|v| v.len() == 4)
        .ok_or("PROCESS_UIDS")?;
    if ids
        .iter()
        .any(|v| v.as_u64().is_none_or(|n| n >= u32::MAX as u64))
    {
        return Err("PROCESS_UID");
    }
    Ok(())
}
pub fn compare(baseline: &Value, connected: &Value, stopped: &Value) -> Result<Value> {
    for value in [connected, stopped] {
        for key in [
            "source",
            "manifest_sha256",
            "case",
            "nonce",
            "authorization_sha256",
        ] {
            if value["provenance"][key] != baseline["provenance"][key] {
                return Err("CAPTURE_RUN_CHANGED");
            }
        }
    }
    let before = &baseline["observations"];
    let live = &connected["observations"];
    let after = &stopped["observations"];
    if before["network"] != after["network"] || live["network"] == before["network"] {
        return Err("ROOT_NETWORK_NOT_RESTORED");
    }
    let worker = &after["units"]["yonik-worker.service"];
    if worker["main"] != Value::Null || worker["members"] != json!([]) || worker["populated"] != 0 {
        return Err("ROOT_WORKER_NONEMPTY");
    }
    // Persistent helper — чужой для driver lifecycle ресурс, обязан сохраниться.
    if before["units"]["yonik-helper.service"] != after["units"]["yonik-helper.service"] {
        return Err("HELPER_NOT_PRESERVED");
    }
    let members = live["units"]["yonik-worker.service"]["members"]
        .as_array()
        .filter(|m| !m.is_empty())
        .ok_or("MISSING_LIVE_MEMBERS")?;
    let tracked = after["tracked_processes"]
        .as_array()
        .ok_or("MISSING_REAP")?;
    for member in members {
        if !tracked.iter().any(|p| {
            p["pid"] == member["pid"]
                && p["start_ticks"] == member["start_ticks"]
                && p["absent"] == true
        }) {
            return Err("ROOT_PROCESS_NOT_REAPED");
        }
    }
    Ok(json!({"class":"ACTUAL","root_captures":[baseline,connected,stopped]}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn echo_without_root_observations_rejected() {
        assert!(validate(
            &json!({"schema":1,"status":"ACTUAL"}),
            "baseline",
            "xray_positive",
            "n",
            "s",
            "m",
            "a"
        )
        .is_err());
    }
    #[test]
    fn empty_cgroup_alone_cannot_prove_cleanup() {
        assert!(compare(&json!({}), &json!({}), &json!({})).is_err());
    }
}
