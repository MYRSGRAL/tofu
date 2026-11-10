//! Скомпилированный профиль проходит TUN → VLESS server → echo, без внешней сети.
use super::*;
use std::{
    io::{BufRead, Write},
    num::NonZeroU16,
    path::{Path, PathBuf},
    process::{Child as StdChild, Command as StdCommand},
};
use yonik_engine::{Engine, SystemTun, SystemTunAddress, compile_system};

struct Process(StdChild);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn ip(args: &[&str]) -> serde_json::Value {
    let output = StdCommand::new("/usr/bin/ip")
        .arg("-j")
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    if output.stdout.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
fn core(binary: &Path, config: &Path) -> Process {
    Process(
        StdCommand::new(binary)
            .args(["run", "-c"])
            .arg(config)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    )
}
async fn listener(process: &mut Process, port: u16) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            assert!(
                process.0.try_wait().unwrap().is_none(),
                "Процесс завершился до готовности"
            );
            if tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
fn peer_config(engine: Engine) -> serde_json::Value {
    use serde_json::json;
    match engine {
        Engine::Xray => {
            json!({"log":{"loglevel":"none"},"inbounds":[{"listen":"127.0.0.1","port":24443,"protocol":"vless","settings":{"decryption":"none","clients":[{"id":"00112233-4455-6677-8899-aabbccddeeff"}]}}],"outbounds":[{"tag":"echo4","protocol":"freedom","settings":{"redirect":"127.0.0.1:24081","finalRules":[{"action":"allow","ip":["127.0.0.1/32"],"port":"24081"}]}},{"tag":"echo6","protocol":"freedom","settings":{"redirect":"[::1]:24081","finalRules":[{"action":"allow","ip":["::1/128"],"port":"24081"}]}}],"routing":{"rules":[{"type":"field","ip":["198.18.0.0/30"],"outboundTag":"echo4"},{"type":"field","ip":["2001:db8:746f::/126"],"outboundTag":"echo6"}]}})
        }
        Engine::SingBox => {
            json!({"log":{"disabled":true},"inbounds":[{"type":"vless","listen":"127.0.0.1","listen_port":24443,"users":[{"uuid":"00112233-4455-6677-8899-aabbccddeeff"}]}],"outbounds":[{"type":"direct","tag":"echo"}],"route":{"rules":[{"ip_cidr":["198.18.0.0/30"],"action":"route","outbound":"echo","override_address":"127.0.0.1","override_port":24081},{"ip_cidr":["2001:db8:746f::/126"],"action":"route","outbound":"echo","override_address":"::1","override_port":24081}]}})
        }
    }
}
async fn peer_signal(peer: &mut Process, expected: &str) {
    let output = peer.0.stdout.take().unwrap();
    let read = tokio::task::spawn_blocking(move || {
        let mut output = std::io::BufReader::new(output);
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        (output.into_inner(), line)
    });
    let (output, line) = tokio::time::timeout(DEADLINE, read).await.unwrap().unwrap();
    peer.0.stdout = Some(output);
    assert_eq!(line.trim(), expected);
}

async fn remote_peer(binary: &Path, config: &Path, tools: &Path) -> Process {
    // VLESS peer находится за veth: local rule не может скрыть отсутствие mark.
    let script = format!(
        r#"import sys, subprocess, socket, time
sys.path.insert(0, {tools:?})
from traffic_echo import echo_servers
print('namespace', flush=True)
assert sys.stdin.buffer.readline() == b'go\n'
def ip(*args):
 subprocess.run(['/usr/bin/ip', *args], check=True, timeout=5, stdout=subprocess.DEVNULL)
ip('link', 'set', 'lo', 'up')
ip('address', 'add', '10.203.0.2/30', 'dev', 'yonik-inner')
ip('-6', 'address', 'add', '2001:db8:7470::2/126', 'dev', 'yonik-inner', 'nodad')
ip('link', 'set', 'yonik-inner', 'up')
with echo_servers():
 with subprocess.Popen([{binary:?}, 'run', '-c', {config:?}], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) as core:
  try:
   deadline = time.monotonic() + 5
   while True:
    if core.poll() is not None: raise RuntimeError('VLESS peer stopped')
    try:
     with socket.create_connection(('127.0.0.1', 24443), timeout=0.1): break
    except OSError:
     if time.monotonic() >= deadline: raise TimeoutError('VLESS peer readiness')
     time.sleep(0.02)
   print('ready', flush=True)
   sys.stdin.buffer.read()
  finally:
   core.kill()
   core.wait(timeout=5)
"#
    );
    let mut peer = Process(
        StdCommand::new("/usr/bin/unshare")
            .args(["--net", "/usr/bin/python3", "-u", "-c", &script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    peer_signal(&mut peer, "namespace").await;
    ip(&[
        "link",
        "add",
        "yonik-outer",
        "type",
        "veth",
        "peer",
        "name",
        "yonik-inner",
    ]);
    ip(&[
        "link",
        "set",
        "yonik-inner",
        "netns",
        &peer.0.id().to_string(),
    ]);
    ip(&["address", "add", "10.203.0.1/30", "dev", "yonik-outer"]);
    ip(&[
        "-6",
        "address",
        "add",
        "2001:db8:7470::1/126",
        "dev",
        "yonik-outer",
        "nodad",
    ]);
    ip(&["link", "set", "yonik-outer", "up"]);
    peer.0.stdin.as_mut().unwrap().write_all(b"go\n").unwrap();
    peer_signal(&mut peer, "ready").await;
    peer
}
async fn address_state(index: u32) -> Vec<(std::net::IpAddr, u8, u32)> {
    let (_keep, mut cancel) = Cancellation::new();
    let snapshot = read_topology(&mut cancel, Duration::from_secs(5))
        .await
        .unwrap();
    let mut result: Vec<_> = snapshot
        .addresses
        .iter()
        .filter(|a| a.index == index && a.scope == 0)
        .map(|a| (a.address, a.prefix_len, a.flags))
        .collect();
    result.sort();
    result
}

fn probe_script() -> String {
    let tools = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools")
        .canonicalize()
        .unwrap();
    format!(
        "import sys,socket\nsys.path.insert(0,{tools:?})\nfrom engine_traffic import probe\nfor family,address in [(socket.AF_INET,'198.18.0.2'),(socket.AF_INET6,'2001:db8:746f::2')]:\n for kind in (socket.SOCK_STREAM,socket.SOCK_DGRAM):\n  probe(family,address,kind)\n"
    )
}
#[tokio::test]
#[ignore = "Требует отдельные namespaces и явно выбранный закреплённый engine"]
async fn compiled_profile_crosses_owned_tun_and_proxy() {
    let ns = File::open("/proc/thread-self/ns/net").unwrap();
    let (device, inode) = namespace(&ns).unwrap();
    let owner = Ownership::new(rustix::process::getuid().as_raw(), 7, 9, device, inode).unwrap();
    // Проверка host-netns до первого процесса или сетевого эффекта.
    let mut executor = TunExecutor::isolated_fixture(owner).unwrap();
    let engine = match std::env::var("YONIK_SYSTEM_TUN_ENGINE").unwrap().as_str() {
        "xray" => Engine::Xray,
        "sing-box" => Engine::SingBox,
        _ => panic!("Неизвестное тестовое ядро"),
    };
    let binary = PathBuf::from(std::env::var_os("YONIK_SYSTEM_TUN_BINARY").unwrap());
    let area = Path::new("/tmp/yonik-compiled-tun");
    std::fs::create_dir(area).unwrap();
    let mut journal = Journal::open(&area.join("network"), owner).unwrap();
    let index = executor.ensure_tun(&mut journal, 1, 1).await.unwrap();
    executor
        .configure_link(&mut journal, 1, 1500)
        .await
        .unwrap();
    for (id, address, prefix) in [(2, "198.18.0.1", 30), (3, "2001:db8:746f::1", 126)] {
        executor
            .ensure_address(
                &mut journal,
                1,
                id,
                TunAddress::new(address.parse().unwrap(), prefix).unwrap(),
            )
            .await
            .unwrap();
    }
    for (id, address) in [(4, "0.0.0.0"), (5, "::")] {
        executor
            .ensure_route(
                &mut journal,
                1,
                id,
                TunRoute::new(address.parse().unwrap(), 0, 1001, 100).unwrap(),
            )
            .await
            .unwrap();
    }
    ip(&["link", "set", "lo", "up"]);
    let before = address_state(index).await;
    let report = yonik_import::import_bytes(
        b"vless://00112233-4455-6677-8899-aabbccddeeff@10.203.0.2:24443".to_vec(),
    )
    .unwrap();
    let tun = SystemTun::new(
        1,
        1500,
        99,
        vec![
            "198.18.0.1/30".parse::<SystemTunAddress>().unwrap(),
            "2001:db8:746f::1/126".parse().unwrap(),
        ],
    )
    .unwrap();
    let config = compile_system(
        &report.servers[0],
        engine,
        NonZeroU16::new(10808).unwrap(),
        &tun,
    )
    .unwrap();
    let client_path = area.join("client.json");
    let mut bytes = Vec::new();
    config.write_sensitive(&mut bytes).unwrap();
    let mut client_value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut peer_value = peer_config(engine);
    peer_value["inbounds"][0]["listen"] = serde_json::json!("0.0.0.0");
    if std::env::var_os("YONIK_SYSTEM_TUN_DEBUG").is_some() && engine == Engine::Xray {
        client_value["log"] = serde_json::json!({"loglevel":"debug"});
        peer_value["log"] = serde_json::json!({"loglevel":"debug"});
    }
    std::fs::write(&client_path, serde_json::to_vec(&client_value).unwrap()).unwrap();
    let peer_path = area.join("peer.json");
    std::fs::write(&peer_path, serde_json::to_vec(&peer_value).unwrap()).unwrap();
    let check = match engine {
        Engine::Xray => vec!["run", "-test", "-c"],
        Engine::SingBox => vec!["check", "-c"],
    };
    let mut checker = Process(
        StdCommand::new(&binary)
            .args(check)
            .arg(&client_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let checked = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = checker.0.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(checked.success());
    let tools = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools")
        .canonicalize()
        .unwrap();
    let mut peer = remote_peer(&binary, &peer_path, &tools).await;
    for (id, ipv6) in [(6, false), (7, true)] {
        executor
            .ensure_rule(
                &mut journal,
                1,
                id,
                TunRule::new(10000, 1001, 99, ipv6).unwrap(),
            )
            .await
            .unwrap();
    }
    for (family, target) in [("-4", "10.203.0.2"), ("-6", "2001:db8:7470::2")] {
        assert_eq!(
            ip(&[family, "route", "get", target])[0]["dev"],
            "tofu00000001"
        );
        assert_eq!(
            ip(&[family, "route", "get", target, "mark", "99"])[0]["dev"],
            "yonik-outer"
        );
    }
    let mut client = core(&binary, &client_path);
    listener(&mut client, 10808).await;
    assert_eq!(
        address_state(index).await,
        before,
        "Ядро изменило глобальные адреса helper или NOPREFIXROUTE"
    );
    for family in ["-4", "-6"] {
        let target = if family == "-4" {
            "198.18.0.2"
        } else {
            "2001:db8:746f::2"
        };
        assert_eq!(
            ip(&[family, "route", "get", target])[0]["dev"],
            "tofu00000001"
        );
        let routes = ip(&[
            family,
            "route",
            "show",
            "table",
            "main",
            "dev",
            "tofu00000001",
        ]);
        assert!(
            routes
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["dst"] == "fe80::/64"),
            "Ядро добавило неучтённый main route"
        );
    }
    let successful = StdCommand::new("/usr/bin/python3")
        .args(["-c", &probe_script()])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .unwrap();
    assert!(successful.success(), "TUN→proxy→echo не прошёл");
    // EOF даёт supervisor завершить и reap peer core, а не оставить потомка.
    drop(peer.0.stdin.take());
    peer.0.wait().unwrap();
    // Все назначения отклоняются без VLESS server; прямого echo fallback нет.
    let negative = format!(
        "import sys,socket\nsys.path.insert(0,{tools:?})\nfrom engine_traffic import probe\nfor family,address in [(socket.AF_INET,'198.18.0.2'),(socket.AF_INET6,'2001:db8:746f::2')]:\n for kind in (socket.SOCK_STREAM,socket.SOCK_DGRAM):\n  try: probe(family,address,kind)\n  except OSError: continue\n  raise RuntimeError('Proxy bypass')\n"
    );
    assert!(
        StdCommand::new("/usr/bin/python3")
            .args(["-c", &negative])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    client.0.kill().unwrap();
    client.0.wait().unwrap();
    for id in [7, 6] {
        executor.remove_rule(&mut journal, 1, id).await.unwrap();
    }
    for id in [5, 4] {
        executor.remove_route(&mut journal, 1, id).await.unwrap();
    }
    for id in [3, 2] {
        executor.remove_address(&mut journal, 1, id).await.unwrap();
    }
    let snapshot = executor.configuration_snapshot().await.unwrap();
    let metadata:Vec<_>=snapshot.addresses.iter().filter(|a|a.index==index).map(|a|serde_json::json!({"address":a.address.to_string(),"family":a.family,"scope":a.scope,"flags":a.flags,"local":a.local.map(|ip|ip.to_string()),"unknown":a.unknown.iter().map(|v|(v.kind,v.value.clone())).collect::<Vec<_>>()})).collect();
    let route_metadata:Vec<_>=snapshot.routes.iter().filter(|r|r.output_interface==Some(index)).map(|r|serde_json::json!({"destination":r.destination.address.to_string(),"prefix":r.destination.length,"family":r.family,"table":r.table,"kind":r.kind,"protocol":r.protocol,"gateway":r.gateway.map(|ip|ip.to_string()),"scope":r.scope,"flags":r.flags,"priority":r.priority,"source":r.source.address.to_string(),"source_prefix":r.source.length,"preferred_source":r.preferred_source.map(|ip|ip.to_string()),"unknown":r.unknown.iter().map(|v|(v.kind,v.value.clone())).collect::<Vec<_>>()})).collect();
    let removed = executor.remove_tun(&mut journal, 1).await;
    assert!(
        removed.is_ok(),
        "{removed:?}: raw_addresses={metadata:?} raw_routes={route_metadata:?} addresses={} routes6all={} routes6local={} routes4main={} routes4local={}",
        ip(&["address", "show", "dev", "tofu00000001"]),
        ip(&["-6", "route", "show", "table", "all", "dev", "tofu00000001"]),
        ip(&[
            "-6",
            "route",
            "show",
            "table",
            "local",
            "dev",
            "tofu00000001"
        ]),
        ip(&[
            "-4",
            "route",
            "show",
            "table",
            "main",
            "dev",
            "tofu00000001"
        ]),
        ip(&[
            "-4",
            "route",
            "show",
            "table",
            "local",
            "dev",
            "tofu00000001"
        ])
    );
    assert!(journal.records().is_empty());
    assert!(
        ip(&["link", "show"])
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l["ifname"] != "tofu00000001")
    );
}

#[tokio::test]
#[ignore = "Требует отдельные namespaces и явно выбранный unpatched sing-box"]
async fn unpatched_engine_is_rejected_before_network_effects() {
    use std::io::Read;
    let ns = File::open("/proc/thread-self/ns/net").unwrap();
    let (device, inode) = namespace(&ns).unwrap();
    let owner = Ownership::new(rustix::process::getuid().as_raw(), 7, 9, device, inode).unwrap();
    let executor = TunExecutor::isolated_fixture(owner).unwrap();
    let before = executor.links().await.unwrap();
    let binary = PathBuf::from(std::env::var_os("YONIK_SYSTEM_TUN_BINARY").unwrap());
    assert_eq!(
        std::env::var("YONIK_SYSTEM_TUN_ENGINE").unwrap(),
        "sing-box"
    );
    let report = yonik_import::import_bytes(
        b"vless://00112233-4455-6677-8899-aabbccddeeff@127.0.0.1:24443".to_vec(),
    )
    .unwrap();
    let tun = SystemTun::new(
        1,
        1500,
        99,
        vec![
            "198.18.0.1/30".parse().unwrap(),
            "2001:db8:746f::1/126".parse().unwrap(),
        ],
    )
    .unwrap();
    let config = compile_system(
        &report.servers[0],
        Engine::SingBox,
        NonZeroU16::new(10808).unwrap(),
        &tun,
    )
    .unwrap();
    let path = Path::new("/tmp/yonik-unsupported-tun.json");
    config
        .write_sensitive(std::fs::File::create(path).unwrap())
        .unwrap();
    let mut checker = Process(
        StdCommand::new(binary)
            .args(["check", "-c"])
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = checker.0.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(!status.success());
    let mut error = String::new();
    checker
        .0
        .stderr
        .take()
        .unwrap()
        .take(16384)
        .read_to_string(&mut error)
        .unwrap();
    assert!(
        error.contains("external_configuration") && error.contains("unknown field"),
        "Неподтверждённая причина отказа"
    );
    assert_eq!(executor.links().await.unwrap(), before);
}
