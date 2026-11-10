use super::{
    BindingInvalidation, BindingRequest, IpPrefix, LanException, LinuxTopology, PrivateBindings,
    PrivateRoute,
};
use crate::{
    Cancellation,
    managed_network::{
        Journal, Ownership,
        firewall::PublicBlockPolicy,
        nft::{NftExecutor, network_peer, network_probe},
    },
    network::{NetworkMonitor, NetworkWatch, read_topology},
};
use std::{os::unix::fs::MetadataExt, time::Duration};

fn fixture(action: &str) {
    assert!(
        std::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tools/private_bindings_fixture.py"
            ))
            .arg(action)
            .status()
            .unwrap()
            .success()
    );
}
fn ip(args: &[&str]) {
    assert!(
        std::process::Command::new("ip")
            .args(args)
            .status()
            .unwrap()
            .success()
    );
}
async fn ready(watch: &mut NetworkWatch) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !watch.current().coverage.routes {
            watch.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
fn request(topo: &LinuxTopology) -> Vec<BindingRequest> {
    let index = topo.links.iter().find(|l| l.name == b"vpn").unwrap().index;
    [("10.42.1.0", 24), ("fd42:1::", 64)]
        .into_iter()
        .map(|(a, n)| BindingRequest {
            prefix: IpPrefix::new(a.parse().unwrap(), n).unwrap(),
            ifindex: index,
            table: 254,
            wireguard: true,
        })
        .collect()
}
fn policy(private: Vec<PrivateRoute>, uplink: u32, tun: u32) -> PublicBlockPolicy {
    // Широкие fixture LAN/TUN не должны обходить terminal private guards.
    PublicBlockPolicy::new(
        1,
        Some(tun),
        private,
        vec![
            LanException::new(
                IpPrefix::new("10.42.0.0".parse().unwrap(), 16).unwrap(),
                uplink,
            )
            .unwrap(),
            LanException::new(
                IpPrefix::new("198.18.1.0".parse().unwrap(), 24).unwrap(),
                uplink,
            )
            .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap()
    .with_link_control(vec![uplink])
    .unwrap()
}
fn traffic(expected: bool) {
    for a in ["10.42.1.2", "fd42:1::2"] {
        for p in ["tcp", "udp"] {
            assert_eq!(network_probe(a, p, None, "24081"), expected, "{a}/{p}");
        }
    }
}
async fn scenario(wg_first: bool) {
    fixture(if wg_first { "setup" } else { "underlay" });
    ip(&["tuntap", "add", "dev", "fixture-tun", "mode", "tun"]);
    ip(&["link", "set", "fixture-tun", "up"]);
    let _peer = network_peer();
    assert!(
        std::process::Command::new("nft")
            .args(["add", "table", "inet", "foreign_events"])
            .status()
            .unwrap()
            .success()
    );
    ip(&[
        "route",
        "add",
        "203.0.113.0/24",
        "dev",
        "uplink",
        "proto",
        "99",
    ]);
    let monitor = NetworkMonitor::start().unwrap();
    let mut watch = monitor.watch();
    ready(&mut watch).await;
    let (cancel_sender, mut cancel) = Cancellation::new();
    let topo = read_topology(&mut cancel, Duration::from_secs(3))
        .await
        .unwrap();
    let uplink = topo
        .links
        .iter()
        .find(|l| l.name == b"uplink")
        .unwrap()
        .index;
    let tun = topo
        .links
        .iter()
        .find(|l| l.name == b"fixture-tun")
        .unwrap()
        .index;
    let meta = std::fs::metadata("/proc/self/ns/net").unwrap();
    let owner = Ownership::new(0, 702, 1, meta.dev(), meta.ino()).unwrap();
    let mut executor = NftExecutor::isolated_fixture(owner).unwrap();
    let mut journal = Journal::open(&std::env::temp_dir().join("binding-events"), owner).unwrap();
    let guards = || {
        ["10.42.1.0", "fd42:1::"]
            .into_iter()
            .zip([24, 64])
            .map(|(a, n)| {
                PrivateRoute::new(IpPrefix::new(a.parse().unwrap(), n).unwrap(), None).unwrap()
            })
            .collect()
    };
    let guard = policy(guards(), uplink, tun);
    if !wg_first {
        executor
            .apply_durable_public_policy(&mut journal, 2, 5, guard.clone())
            .await
            .unwrap();
        traffic(false);
        fixture("connect");
        // Получаем фактическое событие, не присваиваем свежий stamp вручную.
        tokio::time::timeout(Duration::from_secs(3), watch.changed())
            .await
            .unwrap()
            .unwrap();
    }
    let topo = read_topology(&mut cancel, Duration::from_secs(3))
        .await
        .unwrap();
    let requests = request(&topo);
    let old_index = requests[0].ifindex;
    let mut binding = PrivateBindings::capture(&watch, &mut cancel, requests)
        .await
        .unwrap();
    let active = policy(binding.resolve(watch.current(), &topo).0, uplink, tun);
    executor
        .apply_durable_public_policy(&mut journal, 2, 5, active)
        .await
        .unwrap();
    traffic(true);
    let old_stamp = watch.current().stamp;
    fixture("disconnect");
    if !wg_first {
        create_foreign(old_index);
        // Kernel bound predicate закрывает другое имя ДО обработки события.
        traffic(false);
    }
    let event = tokio::time::timeout(
        Duration::from_secs(3),
        binding.wait_invalidation(&mut watch, &mut cancel),
    )
    .await
    .unwrap();
    assert_eq!(event, BindingInvalidation::NetworkChanged);
    assert_ne!(old_stamp, watch.current().stamp);
    let absent = read_topology(&mut cancel, Duration::from_secs(3))
        .await
        .unwrap();
    let blocked = policy(binding.resolve(watch.current(), &absent).0, uplink, tun);
    assert_eq!(blocked.digest(), guard.digest());
    executor
        .apply_durable_public_policy(&mut journal, 2, 5, blocked)
        .await
        .unwrap();
    if wg_first {
        create_foreign(old_index);
    }
    let foreign = read_topology(&mut cancel, Duration::from_secs(3))
        .await
        .unwrap();
    assert_eq!(
        foreign
            .links
            .iter()
            .find(|l| l.name == b"foreign-reuse")
            .unwrap()
            .index,
        old_index
    );
    assert_eq!(
        policy(binding.resolve(watch.current(), &foreign).0, uplink, tun).digest(),
        guard.digest()
    );
    assert!(
        PrivateBindings::capture(
            &watch,
            &mut cancel,
            vec![BindingRequest {
                prefix: IpPrefix::new("10.42.1.0".parse().unwrap(), 24).unwrap(),
                ifindex: old_index,
                table: 254,
                wireguard: true,
            }]
        )
        .await
        .is_err()
    );
    traffic(false);
    // Только fixture убирает собственный rogue link; adapter ничего не удаляет.
    ip(&["link", "del", "foreign-reuse"]);
    fixture("connect");
    tokio::time::timeout(Duration::from_secs(3), watch.changed())
        .await
        .unwrap()
        .unwrap();
    let returned = read_topology(&mut cancel, Duration::from_secs(3))
        .await
        .unwrap();
    assert_eq!(
        policy(binding.resolve(watch.current(), &returned).0, uplink, tun).digest(),
        guard.digest()
    );
    // Возврат разрешается только новым capture, старый witness остаётся закрыт.
    let mut fresh = PrivateBindings::capture(&watch, &mut cancel, request(&returned))
        .await
        .unwrap();
    let active = policy(fresh.resolve(watch.current(), &returned).0, uplink, tun);
    executor
        .apply_durable_public_policy(&mut journal, 2, 5, active)
        .await
        .unwrap();
    traffic(true);
    assert!(monitor.shutdown().await);
    assert_eq!(
        fresh.wait_invalidation(&mut watch, &mut cancel).await,
        BindingInvalidation::ObservationLost
    );
    executor
        .apply_durable_public_policy(
            &mut journal,
            2,
            5,
            policy(fresh.resolve(watch.current(), &returned).0, uplink, tun),
        )
        .await
        .unwrap();
    traffic(false);
    let replacement_monitor = NetworkMonitor::start().unwrap();
    let mut replacement_watch = replacement_monitor.watch();
    ready(&mut replacement_watch).await;
    let mut replacement =
        PrivateBindings::capture(&replacement_watch, &mut cancel, request(&returned))
            .await
            .unwrap();
    assert_eq!(
        policy(
            fresh.resolve(replacement_watch.current(), &returned).0,
            uplink,
            tun
        )
        .digest(),
        guard.digest()
    );
    executor
        .apply_durable_public_policy(
            &mut journal,
            2,
            5,
            policy(
                replacement
                    .resolve(replacement_watch.current(), &returned)
                    .0,
                uplink,
                tun,
            ),
        )
        .await
        .unwrap();
    traffic(true);
    cancel_sender.send_replace(true);
    assert_eq!(
        replacement
            .wait_invalidation(&mut replacement_watch, &mut cancel)
            .await,
        BindingInvalidation::Cancelled
    );
    executor
        .apply_durable_public_policy(
            &mut journal,
            2,
            5,
            policy(
                replacement
                    .resolve(replacement_watch.current(), &returned)
                    .0,
                uplink,
                tun,
            ),
        )
        .await
        .unwrap();
    traffic(false);
    assert!(replacement_monitor.shutdown().await);
    let (_keep, mut final_cancel) = Cancellation::new();
    let preserved = read_topology(&mut final_cancel, Duration::from_secs(3))
        .await
        .unwrap();
    assert!(
        preserved
            .routes
            .iter()
            .any(|r| r.destination.address.to_string() == "203.0.113.0"
                && r.protocol == 99
                && r.output_interface == Some(uplink))
    );
    assert!(
        preserved
            .links
            .iter()
            .any(|l| l.index == tun && l.name == b"fixture-tun")
    );
    assert!(
        std::process::Command::new("nft")
            .args(["list", "table", "inet", "foreign_events"])
            .output()
            .unwrap()
            .status
            .success()
    );
    eprintln!(
        "PASS wg_first={wg_first}: monitor→invalidation→nft guard; actual ifindex reuse; return requires fresh capture; coverage loss/restart/cancel; actual TUN"
    );
}
#[tokio::test]
#[ignore = "Только private namespace runner"]
async fn events_wg_before_guard() {
    scenario(true).await;
}
#[tokio::test]
#[ignore = "Только private namespace runner"]
async fn events_wg_after_guard() {
    scenario(false).await;
}

fn create_foreign(old_index: u32) {
    // Настоящий reuse: kernel link с прежним index, другой name/kind/identity.
    ip(&[
        "link",
        "add",
        "name",
        "foreign-reuse",
        "index",
        &old_index.to_string(),
        "type",
        "veth",
        "peer",
        "name",
        "foreign-peer",
    ]);
    ip(&["link", "set", "foreign-peer", "netns", "yonik-peer"]);
    ip(&["link", "set", "foreign-reuse", "up"]);
    ip(&["-n", "yonik-peer", "link", "set", "foreign-peer", "up"]);
    for (local, remote) in [
        ("10.42.1.1/24", "10.42.1.2/24"),
        ("fd42:1::1/64", "fd42:1::2/64"),
    ] {
        ip(&["address", "add", local, "dev", "foreign-reuse", "nodad"]);
        ip(&[
            "-n",
            "yonik-peer",
            "address",
            "add",
            remote,
            "dev",
            "foreign-peer",
            "nodad",
        ]);
    }
}
