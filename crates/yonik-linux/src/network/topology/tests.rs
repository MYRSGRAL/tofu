use super::*;

fn attr(kind: u16, value: &[u8]) -> Vec<u8> {
    let mut result = ((value.len() + 4) as u16).to_ne_bytes().to_vec();
    result.extend(kind.to_ne_bytes());
    result.extend(value);
    result.resize((result.len() + 3) & !3, 0);
    result
}
fn route(family: u8, bits: u8) -> Vec<u8> {
    vec![family, bits, 0, 0, 254, 4, 0, 1, 0, 0, 0, 0]
}
#[test]
fn prefixes_tables_and_unknowns_are_explicit() {
    let mut bytes = route(2, 24);
    bytes.extend(attr(1, &[192, 0, 2, 77]));
    bytes.extend(attr(15, &1000_u32.to_ne_bytes()));
    bytes.extend(attr(500, &[4, 5]));
    let parsed = parse_route(&bytes).unwrap();
    assert_eq!(
        parsed.destination.address,
        "192.0.2.0".parse::<IpAddr>().unwrap()
    );
    assert_eq!(parsed.table, 1000);
    assert_eq!(parsed.unknown.len(), 1);
    let parsed = parse_route(&route(10, 0)).unwrap();
    assert_eq!(parsed.destination.address, "::".parse::<IpAddr>().unwrap());
}
#[test]
fn rules_keep_missing_priority_and_distinct_priorities() {
    let bytes = route(2, 0);
    assert_eq!(parse_rule(&bytes).unwrap().priority, None);
    for priority in [0_u32, 100, 200] {
        let mut bytes = bytes.clone();
        bytes.extend(attr(6, &priority.to_ne_bytes()));
        bytes.extend(attr(20, &[1, 0, 0, 0, 10, 0, 0, 0]));
        assert_eq!(parse_rule(&bytes).unwrap().priority, Some(priority));
    }
}
#[test]
fn ecmp_and_groups_preserve_weights_and_references() {
    let mut bytes = route(2, 0);
    let mut hop = vec![16, 0, 0, 2, 3, 0, 0, 0];
    hop.extend(attr(5, &[192, 0, 2, 1]));
    bytes.extend(attr(9, &hop));
    let parsed = parse_route(&bytes).unwrap();
    assert_eq!(parsed.multipath[0].weight, 3);
    assert_eq!(parsed.multipath[0].output_interface, 3);
    let mut nh = vec![0; 8];
    nh.extend(attr(1, &12_u32.to_ne_bytes()));
    nh.extend(attr(2, &[8, 0, 0, 0, 2, 0, 0, 0]));
    let parsed = parse_nexthop(&nh).unwrap();
    assert_eq!(parsed.group[0].id, 8);
    assert_eq!(parsed.group[0].weight, 3);
}
#[test]
fn malformed_and_conflicting_attributes_fail_closed() {
    let mut bytes = route(2, 24);
    bytes.extend(attr(1, &[1, 2, 3]));
    assert!(matches!(
        parse_route(&bytes),
        Err(NetworkReadError::Malformed)
    ));
    let mut bytes = route(2, 0);
    bytes.extend(attr(4, &1_u32.to_ne_bytes()));
    bytes.extend(attr(4, &2_u32.to_ne_bytes()));
    assert!(matches!(
        parse_route(&bytes),
        Err(NetworkReadError::Malformed)
    ));
    assert!(parse_route(&route(2, 33)).is_err());
    assert!(parse_nexthop(&[0; 8]).is_err());
}
#[test]
fn link_and_address_keep_native_facts_and_debug_is_redacted() {
    let mut link = vec![0; 16];
    link[4..8].copy_from_slice(&3_i32.to_ne_bytes());
    link.extend(attr(3, b"secret0\0"));
    link.extend(attr(4, &1500_u32.to_ne_bytes()));
    let parsed = parse_link(&link).unwrap();
    assert_eq!(parsed.name, b"secret0");
    assert!(!format!("{parsed:?}").contains("secret0"));
    let mut address = vec![10, 64, 0x80, 0, 3, 0, 0, 0];
    address.extend(attr(
        1,
        &[0x20, 1, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    ));
    let parsed = parse_address(&address).unwrap();
    assert_eq!(parsed.prefix_len, 64);
    assert_eq!(parsed.flags, 0x80);
}

#[test]
fn flagged_scalar_is_retained_and_never_decoded_as_native_endian() {
    let mut bytes = route(2, 0);
    bytes.extend(attr(4 | 0x4000, &3_u32.to_be_bytes()));
    let parsed = parse_route(&bytes).unwrap();
    assert_eq!(parsed.output_interface, None);
    assert_eq!(parsed.unknown[0].kind, 4 | 0x4000);
}
#[test]
fn nested_link_details_are_preserved_and_conflicting_address_flags_fail() {
    let mut link = vec![0; 16];
    link[4..8].copy_from_slice(&1_u32.to_ne_bytes());
    link.extend(attr(3, b"wg0\0"));
    let mut info = attr(1, b"wireguard\0");
    info.extend(attr(2, &[1, 2, 3, 4]));
    link.extend(attr(18 | 0x8000, &info));
    let link = parse_link(&link).unwrap();
    assert_eq!(link.kind, Some(b"wireguard".to_vec()));
    assert_eq!(link.unknown.len(), 1);
    let mut address = vec![2, 24, 0x80, 0, 1, 0, 0, 0];
    address.extend(attr(1, &[192, 0, 2, 1]));
    address.extend(attr(8, &0_u32.to_ne_bytes()));
    assert!(matches!(
        parse_address(&address),
        Err(NetworkReadError::Malformed)
    ));
}
#[tokio::test]
async fn invalid_deadline_and_cancellation_are_checked_before_opening_socket() {
    let (sender, mut cancel) = Cancellation::new();
    assert_eq!(
        read_topology(&mut cancel, Duration::ZERO).await,
        Err(NetworkReadError::Limit)
    );
    assert_eq!(
        read_topology(&mut cancel, Duration::from_secs(6)).await,
        Err(NetworkReadError::Limit)
    );
    sender.send_replace(true);
    assert_eq!(
        read_topology(&mut cancel, Duration::from_secs(1)).await,
        Err(NetworkReadError::Cancelled)
    );
}
#[test]
fn rule_selectors_are_not_collapsed_into_table_priority() {
    let mut bytes = route(10, 0);
    bytes.extend(attr(3, b"eth0\0"));
    bytes.extend(attr(17, b"tun0\0"));
    bytes.extend(attr(10, &9_u32.to_ne_bytes()));
    bytes.extend(attr(16, &255_u32.to_ne_bytes()));
    bytes.extend(attr(19, &[1]));
    bytes.extend(attr(
        20,
        &[1000_u32.to_ne_bytes(), 2000_u32.to_ne_bytes()].concat(),
    ));
    bytes.extend(attr(21, &[4]));
    bytes.extend(attr(22, &[6]));
    bytes.extend(attr(
        23,
        &[80_u16.to_ne_bytes(), 81_u16.to_ne_bytes()].concat(),
    ));
    bytes.extend(attr(
        24,
        &[443_u16.to_ne_bytes(), 443_u16.to_ne_bytes()].concat(),
    ));
    bytes.extend(attr(14, &32_u32.to_ne_bytes()));
    let parsed = parse_rule(&bytes).unwrap();
    assert_eq!(parsed.fwmark, Some(9));
    assert_eq!(parsed.fwmask, Some(255));
    assert_eq!(parsed.uid_range, Some((1000, 2000)));
    assert_eq!(parsed.source_ports, Some((80, 81)));
    assert_eq!(parsed.destination_ports, Some((443, 443)));
    assert_eq!(parsed.input_interface, Some(b"eth0".to_vec()));
    assert_eq!(parsed.output_interface, Some(b"tun0".to_vec()));
    assert_eq!(parsed.protocol, Some(4));
    assert_eq!(parsed.ip_protocol, Some(6));
    assert_eq!(parsed.l3mdev, Some(true));
    assert_eq!(parsed.suppress_prefix_len, Some(32));
}

#[test]
fn ipv6_source_destination_and_gateway_keep_distinct_meaning() {
    let mut bytes = route(10, 64);
    bytes[2] = 48;
    for (kind, ip) in [
        (1, "2001:db8:1::17"),
        (2, "2001:db8:2::18"),
        (5, "fe80::1"),
        (7, "2001:db8:1::19"),
    ] {
        let ip = ip.parse::<Ipv6Addr>().unwrap();
        bytes.extend(attr(kind, &ip.octets()));
    }
    let parsed = parse_route(&bytes).unwrap();
    assert_eq!(
        parsed.destination.address,
        "2001:db8:1::".parse::<IpAddr>().unwrap()
    );
    assert_eq!(
        parsed.source.address,
        "2001:db8:2::".parse::<IpAddr>().unwrap()
    );
    assert_eq!(parsed.gateway, Some("fe80::1".parse().unwrap()));
    assert_eq!(
        parsed.preferred_source,
        Some("2001:db8:1::19".parse().unwrap())
    );
}
#[test]
fn unsupported_data_marks_snapshot_incomplete_and_debug_only_has_counts() {
    let mut bytes = route(2, 0);
    bytes.extend(attr(500, b"sensitive"));
    let topology = LinuxTopology {
        namespace: NetworkNamespace {
            device: 1,
            inode: 2,
        },
        links: vec![],
        addresses: vec![],
        routes: vec![parse_route(&bytes).unwrap()],
        rules: vec![],
        nexthops: vec![],
    };
    assert_eq!(topology.unsupported_count(), 1);
    assert!(topology.incomplete());
    let debug = format!("{topology:?}");
    assert!(debug.contains("routes: 1"));
    assert!(!debug.contains("sensitive"));
}
