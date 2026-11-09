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
