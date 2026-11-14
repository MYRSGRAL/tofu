//! N5 driver: sealed verifier → normal DesktopHost → actual bounded witness.
#![forbid(unsafe_code)]
#[path = "n5_guest_driver/capture.rs"]
mod capture;
#[path = "n5_guest_driver/hash.rs"]
mod hash;
#[path = "n5_guest_driver/logs.rs"]
mod logs;
#[path = "n5_guest_driver/observe.rs"]
mod observe;
#[path = "n5_guest_driver/protocol.rs"]
mod protocol;
#[path = "n5_guest_driver/sealed.rs"]
mod sealed;
#[path = "n5_guest_driver/session.rs"]
mod session;
use sealed::Result;
use serde_json::{Value, json};
use std::{
    io::{BufRead, Read, Write},
    sync::mpsc,
    time::Duration,
};
fn frames() -> mpsc::Receiver<Result<Vec<u8>>> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut input = stdin.lock();
        let mut total = 0usize;
        loop {
            let mut bytes = Vec::new();
            let result = (&mut input).take(4097).read_until(b'\n', &mut bytes);
            if result.is_err() || bytes.is_empty() {
                let _ = tx.send(Err("STDIN_EOF"));
                return;
            }
            total += bytes.len();
            if bytes.len() > 4096 || total > 1048576 || !bytes.ends_with(b"\n") {
                let _ = tx.send(Err("BYTE_BOUND"));
                return;
            }
            if tx.send(Ok(bytes)).is_err() {
                return;
            }
        }
    });
    rx
}
fn receive(rx: &mpsc::Receiver<Result<Vec<u8>>>) -> Result<Vec<u8>> {
    rx.recv_timeout(Duration::from_secs(60))
        .map_err(|_| "INPUT_DEADLINE")?
}
fn write(value: &Value) -> Result<()> {
    let data = serde_json::to_vec(value).map_err(|_| "OUTPUT_JSON")?;
    if data.len() > 4194304 {
        return Err("OUTPUT_BOUND");
    }
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(&data)
        .and_then(|_| stdout.write_all(b"\n"))
        .and_then(|_| stdout.flush())
        .map_err(|_| "OUTPUT_IO")
}
fn witness(
    f: &sealed::VerifiedFixture,
    seq: u64,
    core: &str,
    desktop: Value,
    flows: Value,
    cleanup: Value,
) -> Result<Value> {
    let actual = |observation: Value| json!({"class":"ACTUAL","capture_sha256":f.capture_sha256,"observation":observation});
    let observations = json!({"flows":flows,"root_captures":cleanup["root_captures"]});
    let witness_hash =
        hash::sha256(&serde_json::to_vec(&observations).map_err(|_| "WITNESS_JSON")?);
    let reference = json!({"witness_sha256":witness_hash});
    let inference = json!({"class":"SOURCE_INVARIANT","source_sha256":f.source_sha256,"anchor_sha256":f.capture_sha256});
    let unavailable = json!({"class":"UNAVAILABLE","reason":"SOURCE_ONLY"});
    Ok(
        json!({"schema":2,"seq":seq,"core":core,"case":"positive","stage":"witness","observations":observations,"claims":{
            "desktop_system_connected":actual(desktop),
            "trusted_execution_tuple":actual(json!({"source_sha256":f.source_sha256,"launch_sha256":f.capture_sha256,"live_verified":true})),
            "tcp_udp_policy":actual(reference.clone()),"app_owner_policy":actual(reference.clone()),
            "worker_reaped":actual(reference.clone()),"cgroup_empty":actual(reference.clone()),
            "network_restored":actual(reference.clone()),"foreign_resources_preserved":actual(reference),
            "full_v4_report":inference.clone(),"custody_roles_attach_ack":inference,
            "tampered_internal_report":unavailable.clone(),"revoke_between_internal_stages":unavailable
        }}),
    )
}
fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args != ["--receipt-protocol=n5-v1"] && args != ["--flow-child=n5-v1"] {
        return Err("ARGUMENT_PROTOCOL");
    }
    let fixture = sealed::verify()?;
    // Ни DBus, ни proc/network/Host до приватного verified token.
    if args == ["--receipt-protocol=n5-v1"] {
        fixture.verify_runtime()?;
    } else if rustix::process::getuid().as_raw() != fixture.controls().caller_uid {
        return Err("CHILD_UID");
    }
    let rx = frames();
    if args == ["--flow-child=n5-v1"] {
        let value = protocol::strict_json(&receive(&rx)?)?;
        let flow: sealed::Flow =
            serde_json::from_value(value["flow"].clone()).map_err(|_| "FLOW_PROTOCOL")?;
        let nonce = value["nonce"]
            .as_str()
            .filter(|n| n.len() == 32 && n.bytes().all(|v| v.is_ascii_hexdigit()))
            .ok_or("FLOW_NONCE")?;
        if !fixture
            .controls()
            .flows
            .iter()
            .any(|f| serde_json::to_value(f).ok() == serde_json::to_value(&flow).ok())
        {
            return Err("FLOW_NOT_SEALED");
        }
        return write(&observe::flow_child(&flow, nonce)?);
    }
    let mut root_baseline = None;
    let mut root_connected = None;
    let mut connected_since = 0;
    let mut stopped_since = 0;
    let mut seq = 0;
    let mut host = None;
    let mut baseline = None;
    let mut core = None;
    let mut desktop = None;
    let mut flows = None;
    let mut phase = 0;
    loop {
        let request = protocol::decode(&receive(&rx)?, seq)?;
        let renewed = sealed::verify()?;
        if renewed.capture_sha256 != fixture.capture_sha256
            || renewed.source_sha256 != fixture.source_sha256
        {
            return Err("GRANT_CHANGED");
        }
        seq = request.seq;
        if !matches!(request.case, protocol::Case::Positive) {
            write(
                &json!({"schema":2,"seq":seq,"core":request.core.name(),"case":request.case,"stage":"unavailable","class":"UNAVAILABLE","reason":"SOURCE_ONLY"}),
            )?;
            continue;
        }
        if core.as_deref().is_some_and(|c| c != request.core.name()) {
            return Err("CORE_CHANGED");
        }
        match request.action {
            protocol::Action::Start if phase == 0 => {
                core = Some(request.core.name().to_owned());
                root_baseline = Some(capture::read("baseline", request.core.name(), 0, &fixture)?);
                baseline = Some(observe::Baseline::capture(&fixture)?);
                host = Some(session::Session::initialize(&fixture, &request.core)?);
                phase = 1;
            }
            protocol::Action::Await if phase == 1 => {
                let snapshot = host.as_mut().ok_or("NO_HOST")?.connect()?;
                if snapshot["status"] != "Connected" || snapshot["connection_mode"] != "system" {
                    return Err("NOT_SYSTEM_TUN_CONNECTED");
                }
                connected_since = capture::now();
                let worker = baseline
                    .as_mut()
                    .ok_or("NO_BASELINE")?
                    .bind_worker(&fixture)?;
                desktop = Some(
                    json!({"status":"Connected","connection_mode":"system","policy_requires_attribution":true,"legacy_system_tun_available":snapshot["system_tun_available"],"selected_profile":snapshot["selected_profile"],"source_id":snapshot["source_id"],"engine":snapshot["engine"],"worker":worker}),
                );
                flows = Some(observe::run_flows(&fixture)?);
                phase = 2;
            }
            protocol::Action::Stop if phase == 1 || phase == 2 => {
                root_connected = Some(capture::read(
                    "connected",
                    request.core.name(),
                    connected_since,
                    &fixture,
                )?);
                let stopped = host.as_mut().ok_or("NO_HOST")?.stop()?;
                if stopped["status"] != "Disconnected" {
                    return Err("RECOVERY_REQUIRED");
                }
                phase = 3;
            }
            protocol::Action::CloseDesktopOwner if phase == 3 => {
                host.as_mut().ok_or("NO_HOST")?.close()?;
                let _owned_kernel = baseline.as_ref().ok_or("NO_BASELINE")?.cleanup()?;
                stopped_since = capture::now();
                phase = 4;
            }
            protocol::Action::Cleanup if phase == 4 => {
                let stopped =
                    capture::read("stopped", request.core.name(), stopped_since, &fixture)?;
                let cleanup = capture::compare(
                    root_baseline.as_ref().ok_or("NO_ROOT_BASELINE")?,
                    root_connected.as_ref().ok_or("NO_ROOT_CONNECTED")?,
                    &stopped,
                )?;
                write(&witness(
                    &fixture,
                    seq,
                    request.core.name(),
                    desktop.take().ok_or("NO_CONNECTED")?,
                    flows.take().ok_or("NO_FLOWS")?,
                    cleanup,
                )?)?;
                return Ok(());
            }
            _ => return Err("ACTION_ORDER"),
        }
        write(
            &json!({"schema":2,"seq":seq,"core":request.core.name(),"case":"positive","stage":request.action}),
        )?;
    }
}
fn main() {
    if let Err(reason) = run() {
        let _ = write(&json!({"schema":2,"stage":"denied","reason":reason}));
        std::process::exit(2);
    }
}
