#![cfg(target_os = "linux")]

use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt, symlink},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use yonik_linux::managed_network::*;

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Area(PathBuf);
impl Area {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yonik-journal-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
    fn journal(&self) -> PathBuf {
        self.0.join("state")
    }
}
impl Drop for Area {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn owner() -> Ownership {
    Ownership::new(1000, 11, 1, 4, 99).unwrap()
}
fn tun() -> Resource {
    Resource::Tun { name_id: 7 }
}
fn open(area: &Area) -> Journal {
    Journal::open(&area.journal(), owner()).unwrap()
}

#[test]
fn open_existing_recovers_owner_without_trusting_a_callers_identity() {
    let area = Area::new();
    assert!(Journal::open_existing(&area.journal()).unwrap().is_none());
    let mut journal = open(&area);
    journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
    journal.reconcile(owner(), 1, Reconciled::Owned).unwrap();
    assert!(matches!(
        Journal::open_existing(&area.journal()),
        Err(JournalError::Busy)
    ));
    drop(journal);

    let mut recovered = Journal::open_existing(&area.journal()).unwrap().unwrap();
    assert_eq!(recovered.ownership(), owner());
    assert_eq!(recovered.records().len(), 1);
    assert_eq!(recovered.records()[0].state(), RecordState::Confirmed);
    assert!(recovered.needs_reconciliation(1));
    assert_eq!(
        recovered.cleanup(owner(), Scope::Session, |_| Ok(())),
        Err(JournalError::ReconcileRequired)
    );
}

#[test]
fn empty_owner_rotation_is_durable_and_rejects_the_previous_owner() {
    let area = Area::new();
    let next = Ownership::new(1001, 22, 2, 4, 99).unwrap();
    let mut journal = open(&area);
    journal.reassign_empty_owner(next).unwrap();
    assert_eq!(journal.ownership(), next);
    assert_eq!(
        journal.intent(owner(), Scope::Session, 1, tun()),
        Err(JournalError::WrongOwnership)
    );
    drop(journal);

    assert!(matches!(
        Journal::open(&area.journal(), owner()),
        Err(JournalError::WrongOwnership)
    ));
    let mut recovered = Journal::open_existing(&area.journal()).unwrap().unwrap();
    assert_eq!(recovered.ownership(), next);
    assert!(recovered.records().is_empty());
    assert!(recovered.intent(next, Scope::Session, 1, tun()).unwrap());
}

#[test]
fn owner_rotation_preserves_nonempty_resources_and_public_policy() {
    let next = Ownership::new(1001, 22, 2, 4, 99).unwrap();
    for public_policy in [false, true] {
        let area = Area::new();
        let mut journal = open(&area);
        if public_policy {
            let policy =
                firewall::PublicBlockPolicy::new(1, None, vec![], vec![], vec![], vec![]).unwrap();
            journal.begin_public_policy(owner(), 1, 7, policy).unwrap();
        } else {
            journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
        }
        let original = fs::read(area.journal().join("journal.json")).unwrap();
        assert_eq!(
            journal.reassign_empty_owner(next),
            Err(JournalError::Conflict)
        );
        assert_eq!(journal.ownership(), owner());
        assert_eq!(
            fs::read(area.journal().join("journal.json")).unwrap(),
            original
        );
        drop(journal);
        let recovered = Journal::open_existing(&area.journal()).unwrap().unwrap();
        assert_eq!(recovered.ownership(), owner());
        assert_eq!(recovered.records().len(), 1);
        assert_eq!(recovered.public_policy().is_some(), public_policy);
    }
}

#[test]
fn failed_owner_rotation_preserves_disk_and_poison_blocks_retry() {
    let area = Area::new();
    let mut journal = open(&area);
    let next = Ownership::new(1001, 22, 2, 4, 99).unwrap();
    let file = area.journal().join("journal.json");
    let original = fs::read(&file).unwrap();
    let target = area.0.join("untouched");
    fs::write(&target, b"preserve").unwrap();
    symlink(&target, area.journal().join("journal.next")).unwrap();
    assert_eq!(
        journal.reassign_empty_owner(next),
        Err(JournalError::UnsafePath)
    );
    assert_eq!(
        journal.reassign_empty_owner(next),
        Err(JournalError::UncertainWrite)
    );
    assert_eq!(journal.ownership(), owner());
    assert_eq!(fs::read(&file).unwrap(), original);
    assert_eq!(fs::read(&target).unwrap(), b"preserve");
    drop(journal);
    assert_eq!(
        Journal::open_existing(&area.journal())
            .unwrap()
            .unwrap()
            .ownership(),
        owner()
    );
}

#[test]
fn open_existing_rejects_unsafe_or_corrupt_storage_without_replacing_it() {
    for corrupt in [false, true] {
        let area = Area::new();
        drop(open(&area));
        let file = area.journal().join("journal.json");
        if corrupt {
            fs::write(&file, b"{broken").unwrap();
        } else {
            fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        }
        let original = fs::read(&file).unwrap();
        let expected = if corrupt {
            JournalError::Corrupt
        } else {
            JournalError::UnsafePath
        };
        assert!(matches!(
            Journal::open_existing(&area.journal()),
            Err(error) if error == expected
        ));
        assert_eq!(fs::read(&file).unwrap(), original);
    }
}

#[test]
fn recovery_requires_explicit_reconciliation_and_preserves_ambiguous_intent() {
    let area = Area::new();
    let mut journal = open(&area);
    assert!(journal.intent(owner(), Scope::Session, 1, tun()).unwrap());
    drop(journal);
    let mut journal = open(&area);
    assert_eq!(journal.records()[0].state(), RecordState::Intent);
    assert_eq!(
        journal
            .cleanup(owner(), Scope::Session, |_| Ok(()))
            .unwrap_err(),
        JournalError::ReconcileRequired
    );
    journal.reconcile(owner(), 1, Reconciled::Owned).unwrap();
    assert_eq!(journal.records()[0].state(), RecordState::Confirmed);
    journal
        .cleanup(owner(), Scope::Session, |_| Ok(()))
        .unwrap();
    drop(journal);
    assert!(open(&area).records().is_empty());
}

#[test]
fn async_cleanup_requires_fresh_reconciliation_and_is_durable_before_effect() {
    let area = Area::new();
    let mut journal = open(&area);
    journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
    assert_eq!(
        journal.mark_cleanup(owner(), 1).unwrap_err(),
        JournalError::ReconcileRequired
    );
    journal.reconcile(owner(), 1, Reconciled::Owned).unwrap();
    let foreign = Ownership::new(1001, 11, 1, 4, 99).unwrap();
    assert_eq!(
        journal.mark_cleanup(foreign, 1).unwrap_err(),
        JournalError::WrongOwnership
    );
    journal.mark_cleanup(owner(), 1).unwrap();
    assert!(journal.needs_reconciliation(1));
    assert_eq!(journal.records()[0].state(), RecordState::CleanupRemaining);
    assert_eq!(
        journal.mark_cleanup(owner(), 1).unwrap_err(),
        JournalError::ReconcileRequired
    );
    drop(journal);
    let mut journal = open(&area);
    assert_eq!(journal.records()[0].state(), RecordState::CleanupRemaining);
    journal.reconcile(owner(), 1, Reconciled::Owned).unwrap();
    journal.mark_cleanup(owner(), 1).unwrap();
    journal.reconcile(owner(), 1, Reconciled::Absent).unwrap();
    assert!(journal.records().is_empty());
    assert_eq!(
        journal.mark_cleanup(owner(), 1).unwrap_err(),
        JournalError::Missing
    );
}

#[test]
fn update_is_durable_and_cannot_reverse_cleanup() {
    let area = Area::new();
    let mut journal = open(&area);
    journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
    assert_eq!(
        journal.mark_update(owner(), 1).unwrap_err(),
        JournalError::ReconcileRequired
    );
    journal.reconcile(owner(), 1, Reconciled::Owned).unwrap();
    journal.mark_update(owner(), 1).unwrap();
    assert!(journal.needs_reconciliation(1));
    assert_eq!(journal.records()[0].state(), RecordState::Intent);
    drop(journal);
    let mut journal = open(&area);
    assert!(journal.needs_reconciliation(1));
    journal.reconcile(owner(), 1, Reconciled::Owned).unwrap();
    journal.mark_cleanup(owner(), 1).unwrap();
    journal.reconcile(owner(), 1, Reconciled::Owned).unwrap();
    assert_eq!(
        journal.mark_update(owner(), 1).unwrap_err(),
        JournalError::Conflict
    );
}

#[test]
fn partial_cleanup_is_durable_and_public_block_has_independent_lifetime() {
    let area = Area::new();
    let mut journal = open(&area);
    for (id, scope, resource) in [
        (1, Scope::Session, tun()),
        (
            2,
            Scope::Session,
            Resource::Dns {
                ifindex: 9,
                settings_digest: [2; 32],
            },
        ),
        (
            3,
            Scope::PublicBlock,
            Resource::NftTable {
                table_id: 42,
                family: NftFamily::Inet,
            },
        ),
    ] {
        journal.intent(owner(), scope, id, resource).unwrap();
        journal.reconcile(owner(), id, Reconciled::Owned).unwrap();
    }
    let report = journal
        .cleanup(owner(), Scope::Session, |record| {
            if record.id() == 2 {
                Err(CleanupFailed)
            } else {
                Ok(())
            }
        })
        .unwrap();
    assert_eq!((report.removed, report.remaining), (1, 1));
    drop(journal);
    let mut journal = open(&area);
    assert_eq!(journal.records().len(), 2);
    assert_eq!(journal.records()[0].state(), RecordState::CleanupRemaining);
    journal.reconcile(owner(), 2, Reconciled::Owned).unwrap();
    journal
        .cleanup(owner(), Scope::Session, |_| Ok(()))
        .unwrap();
    assert_eq!(journal.records()[0].scope(), Scope::PublicBlock);
}

#[test]
fn duplicate_and_wrong_identity_are_rejected_without_mutation() {
    let area = Area::new();
    let mut journal = open(&area);
    assert!(journal.intent(owner(), Scope::Session, 1, tun()).unwrap());
    assert!(!journal.intent(owner(), Scope::Session, 1, tun()).unwrap());
    assert_eq!(
        journal
            .intent(owner(), Scope::Session, 1, Resource::Tun { name_id: 8 })
            .unwrap_err(),
        JournalError::Conflict
    );
    for identity in [
        Ownership::new(1001, 11, 1, 4, 99).unwrap(),
        Ownership::new(1000, 11, 2, 4, 99).unwrap(),
    ] {
        assert_eq!(
            journal
                .reconcile(identity, 1, Reconciled::Owned)
                .unwrap_err(),
            JournalError::WrongOwnership
        );
    }
    assert_eq!(journal.records().len(), 1);
    drop(journal);
    assert!(matches!(
        Journal::open(&area.journal(), Ownership::new(1000, 11, 2, 4, 99).unwrap()),
        Err(JournalError::WrongOwnership)
    ));
}

#[test]
fn exclusive_writer_and_private_modes() {
    let area = Area::new();
    let journal = open(&area);
    assert!(matches!(
        Journal::open(&area.journal(), owner()),
        Err(JournalError::Busy)
    ));
    assert_eq!(
        fs::metadata(area.journal()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(area.journal().join("journal.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(journal);
    fs::set_permissions(area.journal(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        Journal::open(&area.journal(), owner()),
        Err(JournalError::UnsafePath)
    ));
}

#[test]
fn symlink_hardlink_and_insecure_file_are_preserved() {
    for attack in 0..3 {
        let area = Area::new();
        drop(open(&area));
        let file = area.journal().join("journal.json");
        let original = fs::read(&file).unwrap();
        let external = area.0.join("external");
        fs::rename(&file, &external).unwrap();
        match attack {
            0 => symlink(&external, &file).unwrap(),
            1 => fs::hard_link(&external, &file).unwrap(),
            _ => {
                fs::rename(&external, &file).unwrap();
                fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
            }
        }
        assert!(matches!(
            Journal::open(&area.journal(), owner()),
            Err(JournalError::UnsafePath)
        ));
        assert_eq!(fs::read(&file).unwrap(), original);
    }
    let area = Area::new();
    symlink(&area.0, area.journal()).unwrap();
    assert!(matches!(
        Journal::open(&area.journal(), owner()),
        Err(JournalError::UnsafePath)
    ));
}

#[test]
fn malformed_unknown_and_oversized_journals_are_preserved() {
    for content in [b"{".to_vec(), br#"{"version":99,"ownership":{"owner_uid":1000,"session":11,"generation":1},"records":[]}"#.to_vec(), br#"{"version":1,"ownership":{"owner_uid":1000,"session":11,"generation":1},"records":[],"command":"secret"}"#.to_vec(), vec![b' '; MAX_JOURNAL_BYTES + 1]] {
        let area = Area::new();
        drop(open(&area));
        let file = area.journal().join("journal.json");
        fs::write(&file, &content).unwrap();
        assert!(Journal::open(&area.journal(), owner()).is_err());
        assert_eq!(fs::read(&file).unwrap(), content);
    }
}

#[test]
fn unknown_fields_are_rejected_at_every_schema_level() {
    let area = Area::new();
    let mut journal = open(&area);
    journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
    drop(journal);
    let file = area.journal().join("journal.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    for pointer in ["", "/ownership", "/records/0", "/records/0/resource"] {
        let mut document = original.clone();
        document
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unexpected".into(), true.into());
        let content = serde_json::to_vec(&document).unwrap();
        fs::write(&file, &content).unwrap();
        assert!(matches!(
            Journal::open(&area.journal(), owner()),
            Err(JournalError::Corrupt)
        ));
        assert_eq!(fs::read(&file).unwrap(), content);
    }
    let mut document = original;
    document["version"] = 99.into();
    let content = serde_json::to_vec(&document).unwrap();
    fs::write(&file, &content).unwrap();
    assert!(matches!(
        Journal::open(&area.journal(), owner()),
        Err(JournalError::Version)
    ));
    assert_eq!(fs::read(&file).unwrap(), content);
}

#[test]
fn interrupted_cleanup_keeps_write_ahead_state_and_requires_recheck() {
    let area = Area::new();
    let mut journal = open(&area);
    journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
    journal.reconcile(owner(), 1, Reconciled::Owned).unwrap();
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = journal.cleanup(owner(), Scope::Session, |_| panic!("отмена backend"));
    }));
    assert!(interrupted.is_err());
    assert_eq!(journal.records()[0].state(), RecordState::CleanupRemaining);
    assert!(journal.needs_reconciliation(1));
    drop(journal);
    let mut journal = open(&area);
    assert_eq!(journal.records()[0].state(), RecordState::CleanupRemaining);
    journal.reconcile(owner(), 1, Reconciled::Absent).unwrap();
    assert!(journal.records().is_empty());
    assert_eq!(
        journal
            .cleanup(owner(), Scope::Session, |_| panic!("нет ресурса"))
            .unwrap(),
        CleanupReport {
            removed: 0,
            remaining: 0
        }
    );
}

#[test]
fn failed_write_never_authorizes_an_effect_and_poison_requires_reopen() {
    let area = Area::new();
    let mut journal = open(&area);
    let file = area.journal().join("journal.json");
    let original = fs::read(&file).unwrap();
    let target = area.0.join("untouched");
    fs::write(&target, "не менять".as_bytes()).unwrap();
    symlink(&target, area.journal().join("journal.next")).unwrap();
    assert_eq!(
        journal
            .intent(owner(), Scope::Session, 1, tun())
            .unwrap_err(),
        JournalError::UnsafePath
    );
    assert_eq!(
        journal
            .intent(owner(), Scope::Session, 1, tun())
            .unwrap_err(),
        JournalError::UncertainWrite
    );
    assert!(journal.records().is_empty());
    assert_eq!(fs::read(&file).unwrap(), original);
    assert_eq!(fs::read(&target).unwrap(), "не менять".as_bytes());
    drop(journal);
    assert!(open(&area).records().is_empty());
}

#[test]
fn directory_fd_pins_storage_even_if_parent_is_renamed() {
    let area = Area::new();
    let mut journal = open(&area);
    let pinned = area.0.join("pinned");
    fs::rename(area.journal(), &pinned).unwrap();
    let other = area.0.join("other");
    fs::DirBuilder::new().mode(0o700).create(&other).unwrap();
    symlink(&other, area.journal()).unwrap();
    journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
    assert!(fs::read_dir(&other).unwrap().next().is_none());
    drop(journal);
    assert_eq!(Journal::open(&pinned, owner()).unwrap().records().len(), 1);
    assert!(matches!(
        Journal::open(&area.journal(), owner()),
        Err(JournalError::UnsafePath)
    ));
}

#[test]
fn bounded_records_reject_conflicts_and_invalid_descriptors() {
    let area = Area::new();
    let mut journal = open(&area);
    assert_eq!(
        journal
            .intent(owner(), Scope::PublicBlock, 1, tun())
            .unwrap_err(),
        JournalError::Invalid
    );
    assert_eq!(
        journal
            .intent(owner(), Scope::Session, 0, tun())
            .unwrap_err(),
        JournalError::Invalid
    );
    for id in 1..=MAX_RECORDS as u64 {
        journal
            .intent(
                owner(),
                Scope::Session,
                id,
                Resource::Tun { name_id: id as u32 },
            )
            .unwrap();
    }
    assert_eq!(
        journal
            .intent(owner(), Scope::Session, 999, Resource::Tun { name_id: 999 })
            .unwrap_err(),
        JournalError::Limit
    );
    drop(journal);
    assert_eq!(open(&area).records().len(), MAX_RECORDS);
}

#[test]
fn route_descriptors_require_canonical_prefixes_and_duplicate_resources_conflict() {
    let area = Area::new();
    let mut journal = open(&area);
    let route = |destination, prefix_len| Resource::Route {
        table: 10001,
        destination,
        prefix_len,
        priority: 100,
        protocol: 99,
        ifindex: 3,
    };
    assert_eq!(
        journal
            .intent(
                owner(),
                Scope::Session,
                1,
                route("192.0.2.1".parse().unwrap(), 24)
            )
            .unwrap_err(),
        JournalError::Invalid
    );
    assert_eq!(
        journal
            .intent(
                owner(),
                Scope::Session,
                1,
                route("2001:db8::1".parse().unwrap(), 64)
            )
            .unwrap_err(),
        JournalError::Invalid
    );
    journal
        .intent(
            owner(),
            Scope::Session,
            1,
            route("192.0.2.0".parse().unwrap(), 24),
        )
        .unwrap();
    assert_eq!(
        journal
            .intent(
                owner(),
                Scope::Session,
                2,
                route("192.0.2.0".parse().unwrap(), 24)
            )
            .unwrap_err(),
        JournalError::Conflict
    );
    assert_eq!(journal.records().len(), 1);
}

#[test]
fn routing_rule_tracks_kernel_protocol_for_ownership_reconciliation() {
    let area = Area::new();
    let mut journal = open(&area);
    let rule = |protocol| Resource::Rule {
        priority: 10001,
        table: 10001,
        mark: 7,
        mask: u32::MAX,
        ipv6: false,
        protocol,
    };
    assert_eq!(
        journal
            .intent(owner(), Scope::Session, 1, rule(0))
            .unwrap_err(),
        JournalError::Invalid
    );
    journal
        .intent(owner(), Scope::Session, 1, rule(99))
        .unwrap();
    drop(journal);
    assert_eq!(open(&area).records()[0].resource(), &rule(99));
}

#[test]
fn unknown_ownership_remains_while_verified_resources_are_cleaned() {
    let area = Area::new();
    let mut journal = open(&area);
    journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
    journal
        .intent(owner(), Scope::Session, 2, Resource::Tun { name_id: 8 })
        .unwrap();
    journal.reconcile(owner(), 1, Reconciled::Owned).unwrap();
    let report = journal
        .cleanup(owner(), Scope::Session, |record| {
            assert_eq!(record.id(), 1);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        report,
        CleanupReport {
            removed: 1,
            remaining: 1
        }
    );
    assert_eq!(journal.records()[0].id(), 2);
    assert!(journal.needs_reconciliation(2));
}

#[test]
fn crash_writer_child() {
    let Some(path) = std::env::var_os("YONIK_TEST_JOURNAL_CRASH_PATH") else {
        return;
    };
    let mut journal = Journal::open(&PathBuf::from(path), owner()).unwrap();
    journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
    // Имитируем смерть helper после durable intent, без Drop и без эффекта.
    std::process::exit(73);
}

#[test]
fn process_exit_releases_lock_and_keeps_ambiguous_intent() {
    let area = Area::new();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_writer_child"])
        .env("YONIK_TEST_JOURNAL_CRASH_PATH", area.journal())
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(73));
    let journal = open(&area);
    assert_eq!(journal.records()[0].state(), RecordState::Intent);
    assert!(journal.needs_reconciliation(1));
}

#[test]
fn address_records_require_a_live_parent_in_the_same_document() {
    let area = Area::new();
    let mut journal = open(&area);
    let address = Resource::Address {
        tun_id: 1,
        ifindex: 7,
        address: "198.18.0.1".parse().unwrap(),
        prefix_len: 30,
    };
    assert_eq!(
        journal.intent(owner(), Scope::Session, 2, address.clone()),
        Err(JournalError::Invalid)
    );
    assert!(journal.records().is_empty());
    journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
    journal.intent(owner(), Scope::Session, 2, address).unwrap();
    assert_eq!(
        journal.reconcile(owner(), 1, Reconciled::Absent),
        Err(JournalError::Invalid)
    );
    assert_eq!(journal.records().len(), 2);
    drop(journal);
    let mut journal = open(&area);
    assert_eq!(journal.records().len(), 2);
    journal.reconcile(owner(), 2, Reconciled::Absent).unwrap();
    journal.reconcile(owner(), 1, Reconciled::Absent).unwrap();
    assert!(journal.records().is_empty());
}

#[test]
fn address_records_reject_invalid_address_scope_and_bounds() {
    let area = Area::new();
    let mut journal = open(&area);
    journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
    for (address, prefix_len) in [
        ("0.0.0.0", 24),
        ("127.0.0.1", 8),
        ("224.0.0.1", 24),
        ("255.255.255.255", 32),
        ("169.254.0.1", 16),
        ("::", 64),
        ("::1", 128),
        ("::ffff:127.0.0.1", 128),
        ("::192.0.2.1", 128),
        ("ff02::1", 64),
        ("fe80::1", 64),
        ("198.18.0.1", 0),
        ("198.18.0.1", 33),
        ("2001:db8::1", 129),
    ] {
        let resource = Resource::Address {
            tun_id: 1,
            ifindex: 7,
            address: address.parse().unwrap(),
            prefix_len,
        };
        assert_eq!(
            journal.intent(owner(), Scope::Session, 2, resource),
            Err(JournalError::Invalid)
        );
    }
    let valid = Resource::Address {
        tun_id: 1,
        ifindex: 7,
        address: "2001:db8::1".parse().unwrap(),
        prefix_len: 64,
    };
    assert_eq!(
        journal.intent(owner(), Scope::PublicBlock, 2, valid.clone()),
        Err(JournalError::Invalid)
    );
    journal.intent(owner(), Scope::Session, 2, valid).unwrap();
}

#[test]
fn cleanup_orders_children_first_and_never_calls_parent_after_child_failure() {
    for fail_child in [false, true] {
        let area = Area::new();
        let mut journal = open(&area);
        journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
        journal
            .intent(
                owner(),
                Scope::Session,
                2,
                Resource::Address {
                    tun_id: 1,
                    ifindex: 7,
                    address: "198.18.0.1".parse().unwrap(),
                    prefix_len: 30,
                },
            )
            .unwrap();
        journal.reconcile(owner(), 1, Reconciled::Owned).unwrap();
        journal.reconcile(owner(), 2, Reconciled::Owned).unwrap();
        let mut called = Vec::new();
        let report = journal
            .cleanup(owner(), Scope::Session, |record| {
                called.push(record.id());
                if fail_child && record.id() == 2 {
                    Err(CleanupFailed)
                } else {
                    Ok(())
                }
            })
            .unwrap();
        if fail_child {
            assert_eq!(called, [2]);
            assert_eq!((report.removed, report.remaining), (0, 2));
        } else {
            assert_eq!(called, [2, 1]);
            assert_eq!((report.removed, report.remaining), (2, 0));
        }
    }
}

#[test]
fn tun_rule_is_typed_parent_child_and_blocks_route_cleanup_until_removed() {
    let area = Area::new();
    let mut journal = open(&area);
    let rule = Resource::TunRule {
        tun_id: 1,
        ifindex: 7,
        priority: 10001,
        table: 1001,
        mark: 47,
        ipv6: false,
    };
    assert_eq!(
        journal.intent(owner(), Scope::Session, 3, rule.clone()),
        Err(JournalError::Invalid)
    );
    journal.intent(owner(), Scope::Session, 1, tun()).unwrap();
    journal
        .intent(
            owner(),
            Scope::Session,
            2,
            Resource::Route {
                table: 1001,
                destination: "0.0.0.0".parse().unwrap(),
                prefix_len: 0,
                priority: 47,
                protocol: 253,
                ifindex: 7,
            },
        )
        .unwrap();
    journal.intent(owner(), Scope::Session, 3, rule).unwrap();
    for id in [1, 2, 3] {
        journal.reconcile(owner(), id, Reconciled::Owned).unwrap();
    }
    let mut visited = Vec::new();
    let report = journal
        .cleanup(owner(), Scope::Session, |record| {
            visited.push(record.id());
            Err(CleanupFailed)
        })
        .unwrap();
    assert_eq!(report.removed, 0);
    assert_eq!(visited, vec![3]);
    drop(journal);
    let mut journal = open(&area);
    assert_eq!(journal.records().len(), 3);
    for id in [1, 2, 3] {
        journal.reconcile(owner(), id, Reconciled::Owned).unwrap();
    }
    let report = journal
        .cleanup(owner(), Scope::Session, |_| Ok(()))
        .unwrap();
    assert_eq!(report.removed, 3);
    assert_eq!(report.remaining, 0);
}
