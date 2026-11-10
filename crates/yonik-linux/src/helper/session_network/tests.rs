use super::*;
use crate::{NetworkNamespace, managed_network::Reconciled};
use std::{fs, os::unix::fs::DirBuilderExt, path::PathBuf};
struct Area(PathBuf);
impl Drop for Area {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn owner() -> Ownership {
    Ownership::new(1000, 1, 1, 1, 1).unwrap()
}
fn baseline() -> LinuxTopology {
    LinuxTopology {
        namespace: NetworkNamespace {
            device: 1,
            inode: 1,
        },
        links: vec![],
        addresses: vec![],
        routes: vec![],
        rules: vec![],
        nexthops: vec![],
    }
}
#[test]
fn reserved_descriptors_are_helper_chosen_and_compiler_uses_same_tun() {
    let mut topology = baseline();
    let plan = SessionNetworkPlan::reserve(owner(), &topology).unwrap();
    assert_eq!(
        plan.compiler_tun().unwrap().interface_name(),
        format!("tofu{:08x}", plan.name)
    );
    assert!(plan.available(&topology));
    let mut rule = crate::LinuxRule {
        family: 2,
        destination: crate::IpPrefix {
            address: Ipv4Addr::UNSPECIFIED.into(),
            length: 0,
        },
        source: crate::IpPrefix {
            address: Ipv4Addr::UNSPECIFIED.into(),
            length: 0,
        },
        tos: 0,
        table: plan.table,
        action: 1,
        flags: 0,
        priority: None,
        goto: None,
        fwmark: None,
        fwmask: None,
        uid_range: None,
        input_interface: None,
        output_interface: None,
        protocol: None,
        ip_protocol: None,
        source_ports: None,
        destination_ports: None,
        suppress_prefix_len: None,
        suppress_interface_group: None,
        l3mdev: None,
        unknown: vec![],
    };
    topology.rules.push(rule.clone());
    assert!(!plan.available(&topology));
    rule.table = 254;
    rule.priority = Some(plan.priority);
    topology.rules[0] = rule;
    assert!(!plan.available(&topology));
    topology.namespace.inode = 2;
    assert!(matches!(
        SessionNetworkPlan::reserve(owner(), &topology),
        Err(TunError::Namespace)
    ));
}
#[test]
fn exact_generation_and_all_seven_resources_required_before_external_verification() {
    let mut nonce = [0; 8];
    getrandom::getrandom(&mut nonce).unwrap();
    let path = std::env::temp_dir().join(format!(
        "yonik-session-plan-{}-{}",
        std::process::id(),
        u64::from_le_bytes(nonce)
    ));
    fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
    let _area = Area(path.clone());
    let plan = SessionNetworkPlan::candidate(owner(), [1; 24]).unwrap();
    let mut journal = Journal::open(&path.join("network"), owner()).unwrap();
    assert!(plan.check_records(&journal, 7, Some(7)).is_err());
    let mut records = plan.resources(7);
    records.sort_unstable_by_key(|(id, _)| *id);
    for (id, resource) in records {
        journal
            .intent(owner(), Scope::Session, id, resource)
            .unwrap();
        journal.reconcile(owner(), id, Reconciled::Owned).unwrap();
    }
    plan.check_records(&journal, 7, Some(7)).unwrap();
    assert!(plan.check_records(&journal, 8, Some(7)).is_err());
    let other = SessionNetworkPlan::candidate(owner(), [2; 24]).unwrap();
    assert!(other.check_records(&journal, 7, Some(7)).is_err());
    journal.require_reconciliation(owner(), RULES + 1).unwrap();
    // Старый Confirmed не заменяет свежую сверку executor после отмены/ошибки.
    assert!(journal.needs_reconciliation(RULES + 1));
    assert!(plan.check_records(&journal, 7, Some(7)).is_err());
    drop(journal);
}
#[test]
fn prefix_collision_handles_both_families_and_different_lengths() {
    assert!(overlaps(
        "198.18.1.1".parse().unwrap(),
        30,
        "198.18.0.0".parse().unwrap(),
        15
    ));
    assert!(!overlaps(
        "198.18.1.1".parse().unwrap(),
        30,
        "198.18.2.0".parse().unwrap(),
        24
    ));
    assert!(overlaps(
        "fd79:6f6e::1".parse().unwrap(),
        126,
        "fd79:6f6e::".parse().unwrap(),
        32
    ));
    assert!(!overlaps(
        "198.18.1.1".parse().unwrap(),
        30,
        "fd79:6f6e::".parse().unwrap(),
        32
    ));
}

#[test]
fn session_ids_do_not_collide_with_existing_public_protection_record() {
    let plan = SessionNetworkPlan::candidate(owner(), [1; 24]).unwrap();
    // Действующий SystemProtectionHost использует record ID 1 для public policy.
    assert!(plan.resources(7).iter().all(|(id, _)| *id != 1));
}
