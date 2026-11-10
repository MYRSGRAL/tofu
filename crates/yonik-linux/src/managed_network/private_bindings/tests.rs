use super::*;
use crate::{
    Cancellation,
    managed_network::{
        Journal, Ownership,
        firewall::PublicBlockPolicy,
        nft::{NftExecutor, network_peer, network_probe},
    },
    network::{NetworkMonitor, read_topology},
};
use std::{os::unix::fs::MetadataExt, time::Duration};

#[tokio::test]
#[ignore = "Только отдельные user/net/mount/pid namespace с приватными /run /tmp /sys"]
async fn actual_wireguard_bindings_loss_and_foreign_preservation() {
    let status = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/private_bindings_fixture.py"
        ))
        .status()
        .unwrap();
    assert!(status.success());
    let _peer = network_peer();
    assert!(
        std::process::Command::new("nft")
            .args(["add", "table", "inet", "foreign_binding_fixture"])
            .status()
            .unwrap()
            .success()
    );
    let monitor = NetworkMonitor::start().unwrap();
    let mut watch = monitor.watch();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !watch.current().coverage.routes {
            watch.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let (_cancel, mut cancel) = Cancellation::new();
    let before = watch.current();
    let topology = read_topology(&mut cancel, Duration::from_secs(3))
        .await
        .unwrap();
    let snapshot = watch.current();
    let index = |name: &[u8]| {
        topology
            .links
            .iter()
            .find(|l| l.name == name)
            .unwrap()
            .index
    };
    let vpn = index(b"vpn");
    let prefix = |a: &str, n| IpPrefix::new(a.parse().unwrap(), n).unwrap();
    let requests = vec![
        BindingRequest {
            prefix: prefix("10.42.1.0", 24),
            ifindex: vpn,
            table: 254,
            wireguard: true,
        },
        BindingRequest {
            prefix: prefix("fd42:1::", 64),
            ifindex: vpn,
            table: 254,
            wireguard: true,
        },
        BindingRequest {
            prefix: prefix("198.18.1.0", 24),
            ifindex: index(b"uplink"),
            table: 254,
            wireguard: false,
        },
    ];
    let mut requests = requests;
    requests.push(BindingRequest {
        prefix: prefix("2001:db8:1::", 64),
        ifindex: index(b"uplink"),
        table: 254,
        wireguard: false,
    });
    let bindings =
        PrivateBindings::verify(before, snapshot, topology.clone(), requests.clone()).unwrap();
    let mut wrong = topology.clone();
    wrong
        .links
        .iter_mut()
        .find(|l| l.index == vpn)
        .unwrap()
        .kind = Some(b"dummy".to_vec());
    assert!(PrivateBindings::verify(before, snapshot, wrong, requests.clone()).is_err());
    assert!(matches!(
        PrivateBindings::verify(
            before,
            snapshot,
            topology.clone(),
            vec![requests[0].clone(); 65]
        ),
        Err(BindingError::Limit)
    ));
    let policy = |state, topo: &LinuxTopology| {
        let (private, lan) = bindings.resolve(state, topo);
        PublicBlockPolicy::new(index(b"lo"), None, private, lan, vec![], vec![])
            .unwrap()
            .with_link_control(vec![index(b"uplink"), vpn])
            .unwrap()
    };
    let meta = std::fs::metadata("/proc/self/ns/net").unwrap();
    let owner = Ownership::new(0, 701, 1, meta.dev(), meta.ino()).unwrap();
    let mut executor = NftExecutor::isolated_fixture(owner).unwrap();
    let path = std::env::temp_dir().join("yonik-private-bindings");
    let mut journal = Journal::open(&path, owner).unwrap();
    executor
        .apply_durable_public_policy(&mut journal, 2, 5, policy(snapshot, &topology))
        .await
        .unwrap();
    for address in ["10.42.1.2", "fd42:1::2", "198.18.1.2", "2001:db8:1::2"] {
        for protocol in ["tcp", "udp"] {
            assert!(
                network_probe(address, protocol, None, "24081"),
                "{address}/{protocol}"
            );
        }
    }
    // Потеря coverage, namespace mismatch и подмена link при том же ifindex.
    let mut lost = snapshot;
    lost.coverage.routes = false;
    let guard = policy(lost, &topology);
    assert_ne!(guard.digest(), policy(snapshot, &topology).digest());
    let mut foreign = topology.clone();
    foreign.namespace.inode += 1;
    assert_eq!(guard.digest(), policy(snapshot, &foreign).digest());
    foreign = topology.clone();
    foreign
        .links
        .iter_mut()
        .find(|l| l.index == vpn)
        .unwrap()
        .name = b"foreign".to_vec();
    assert_eq!(guard.digest(), policy(snapshot, &foreign).digest());
    assert!(PrivateBindings::verify(before, lost, topology.clone(), requests).is_err());
    executor
        .apply_durable_public_policy(&mut journal, 2, 5, guard)
        .await
        .unwrap();
    for address in ["10.42.1.2", "fd42:1::2"] {
        for protocol in ["tcp", "udp"] {
            assert!(!network_probe(address, protocol, None, "24081"));
        }
    }
    // Adapter не удаляет внешние route/link: сравниваем actual RTNETLINK.
    let after = read_topology(&mut cancel, Duration::from_secs(3))
        .await
        .unwrap();
    assert!(crate::network::same_external_topology(&topology, &after));
    // Реальный link-down и network epoch: старое разрешение не переживает событие.
    assert!(
        std::process::Command::new("ip")
            .args(["link", "set", "vpn", "down"])
            .status()
            .unwrap()
            .success()
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while watch.current().stamp == snapshot.stamp {
            watch.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let changed = read_topology(&mut cancel, Duration::from_secs(3))
        .await
        .unwrap();
    assert_eq!(
        policy(watch.current(), &changed).digest(),
        policy(lost, &topology).digest()
    );
    executor
        .apply_durable_public_policy(&mut journal, 2, 5, policy(watch.current(), &changed))
        .await
        .unwrap();
    for address in ["10.42.1.2", "fd42:1::2"] {
        for protocol in ["tcp", "udp"] {
            assert!(!network_probe(address, protocol, None, "24081"));
        }
    }
    assert!(
        std::process::Command::new("nft")
            .args(["list", "table", "inet", "foreign_binding_fixture"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(monitor.shutdown().await);
}
