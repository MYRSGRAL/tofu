use super::*;
use crate::{LinuxLink, LinuxRoute, LinuxTopology, NetworkNamespace};

pub(super) fn link(index: u32, kind: Option<&[u8]>, loopback: bool) -> LinuxLink {
    LinuxLink {
        index,
        link_type: if loopback { 772 } else { ARPHRD_ETHER },
        flags: IFF_UP | if loopback { IFF_LOOPBACK } else { 0 },
        name: format!("link{index}").into_bytes(),
        alias: None,
        mtu: Some(1500),
        master: None,
        parent: None,
        kind: kind.map(Vec::from),
        operational_state: Some(6),
        unknown: vec![],
    }
}
fn route(address: &str, length: u8, index: u32) -> LinuxRoute {
    let address: std::net::IpAddr = address.parse().unwrap();
    LinuxRoute {
        family: if address.is_ipv4() { 2 } else { 10 },
        destination: crate::IpPrefix { address, length },
        source: crate::IpPrefix {
            address: if address.is_ipv4() {
                "0.0.0.0".parse().unwrap()
            } else {
                "::".parse().unwrap()
            },
            length: 0,
        },
        table: RT_TABLE_MAIN,
        preferred_source: None,
        gateway: None,
        output_interface: Some(index),
        input_interface: None,
        priority: None,
        protocol: RTPROT_KERNEL,
        kind: 1,
        scope: if address.is_ipv4() { RT_SCOPE_LINK } else { 0 },
        tos: 0,
        flags: 0,
        multipath: vec![],
        nexthop_id: None,
        unknown: vec![],
    }
}
#[test]
fn lan_is_derived_only_from_live_connected_non_vpn_links() {
    let topology = LinuxTopology {
        namespace: NetworkNamespace {
            device: 1,
            inode: 2,
        },
        links: vec![
            link(1, None, true),
            link(2, None, false),
            link(3, Some(b"tun"), false),
            link(4, Some(b"wireguard"), false),
        ],
        addresses: vec![],
        routes: vec![
            route("192.168.7.0", 24, 2),
            route("2001:db8:7::", 64, 2),
            route("10.0.0.0", 8, 3),
            route("172.16.0.0", 12, 4),
        ],
        rules: vec![],
        nexthops: vec![],
    };
    let expected = PublicBlockPolicy::new(
        1,
        None,
        vec![],
        vec![
            LanException::new(
                IpPrefix::new("192.168.7.0".parse().unwrap(), 24).unwrap(),
                2,
            )
            .unwrap(),
            LanException::new(
                IpPrefix::new("2001:db8:7::".parse().unwrap(), 64).unwrap(),
                2,
            )
            .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap()
    .with_link_control(vec![2])
    .unwrap();
    assert_eq!(compile_protection(&topology, true).unwrap(), expected);
    assert_eq!(
        compile_protection(&topology, false).unwrap(),
        PublicBlockPolicy::new(1, None, vec![], vec![], vec![], vec![])
            .unwrap()
            .with_link_control(vec![2])
            .unwrap()
    );
    let mut missing = topology;
    missing.links.remove(0);
    assert_eq!(
        compile_protection(&missing, true),
        Err(ServiceError::Unavailable)
    );
}
#[tokio::test]
async fn artifact_debug_hides_payload() {
    let monitor = NetworkMonitor::start().unwrap();
    let artifact = GuardArtifact {
        bytes: b"fixture-private-value".to_vec(),
        stamp: monitor.watch().current().stamp,
        session: None,
    };
    assert_eq!(format!("{artifact:?}"), "GuardArtifact(<скрыто>)");
    monitor.shutdown().await;
}

#[tokio::test]
#[ignore = "Требует отдельные user/net/mount/pid namespaces и приватные /run, /tmp"]
async fn isolated_service_applies_preserves_and_unlocks_real_rules() {
    use crate::managed_network::nft::{network_fixture, network_peer, network_probe};
    // Guard fixture отказывает в host netns до любой записи.
    network_fixture(&["setup"]);
    let _peer = network_peer();
    for address in ["198.18.1.2", "2001:db8:1::2"] {
        for protocol in ["tcp", "udp"] {
            assert!(network_probe(address, protocol, None, "24081"));
        }
    }
    let directory = PathBuf::from("/tmp/yonik-service-fixture");
    let (_keep, cancel) = Cancellation::new();
    let host = SystemProtectionHost::fixture(directory.clone(), cancel)
        .await
        .unwrap();
    let fixture = crate::helper::service::tests::Fixture::real_host(host).await;
    let prepared = fixture
        .call(
            "Prepare",
            r#"{"version":1,"mode":"public_protection","allow_lan":false}"#,
        )
        .await
        .unwrap();
    assert_eq!(fixture.check_count(), 1);
    let listing = std::process::Command::new("/usr/bin/nft")
        .args(["-j", "list", "tables"])
        .output()
        .unwrap();
    assert!(listing.status.success());
    let listing: serde_json::Value = serde_json::from_slice(&listing.stdout).unwrap();
    assert!(
        !listing["nftables"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.get("table").is_some())
    );
    let apply = serde_json::json!({"version":1,"handle":prepared["handle"]}).to_string();
    assert_eq!(
        fixture.call("Apply", &apply).await.unwrap()["state"],
        "verified_rules"
    );
    assert_eq!(fixture.check_count(), 2);
    assert_eq!(
        fixture.call("Stop", r#"{"version":1}"#).await.unwrap()["state"],
        "verified_rules"
    );
    for address in ["198.18.1.2", "2001:db8:1::2"] {
        for protocol in ["tcp", "udp"] {
            assert!(!network_probe(address, protocol, None, "24081"));
        }
    }
    assert_eq!(fixture.check_count(), 3);
    fixture.shutdown().await;
    // Новый backend читает durable intent, но не объявляет старые bindings
    // проверенными для нового процесса/снимка сети.
    let (_keep, cancel) = Cancellation::new();
    let host = SystemProtectionHost::fixture(directory, cancel)
        .await
        .unwrap();
    let fixture = crate::helper::service::tests::Fixture::real_host(host).await;
    assert_eq!(
        fixture.call("State", r#"{"version":1}"#).await.unwrap()["state"],
        "recovery_required"
    );
    assert_eq!(
        fixture.call("Unlock", r#"{"version":1}"#).await.unwrap()["state"],
        "no_managed_policy"
    );
    assert_eq!(fixture.check_count(), 1);
    for address in ["198.18.1.2", "2001:db8:1::2"] {
        for protocol in ["tcp", "udp"] {
            assert!(network_probe(address, protocol, None, "24081"));
        }
    }
    fixture.shutdown().await;
}
