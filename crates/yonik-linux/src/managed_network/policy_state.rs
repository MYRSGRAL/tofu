//! Желаемое состояние публичной блокировки в той же атомарной записи журнала.
use super::firewall::PublicBlockPolicy;
use super::{
    Document, Journal, JournalError, NftFamily, Ownership, Record, RecordState, Resource, Scope,
};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", deny_unknown_fields)]
pub enum PublicPolicyPhase {
    Present {
        previous: Option<PublicBlockPolicy>,
        desired: PublicBlockPolicy,
    },
    Stable {
        policy: PublicBlockPolicy,
    },
    Removing {
        candidates: Vec<PublicBlockPolicy>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicPolicyRecord {
    id: u64,
    table_id: u64,
    revision: u64,
    phase: PublicPolicyPhase,
}
impl fmt::Debug for PublicPolicyPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PublicPolicyPhase(<скрыто>)")
    }
}
impl fmt::Debug for PublicPolicyRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PublicPolicyRecord(<скрыто>)")
    }
}
impl PublicPolicyRecord {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn table_id(&self) -> u64 {
        self.table_id
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn phase(&self) -> &PublicPolicyPhase {
        &self.phase
    }
    pub(super) fn validate(&self, document: &Document) -> Result<(), JournalError> {
        if self.id == 0
            || self.table_id == 0
            || self.revision == 0
            || self.revision > document.policy_sequence
        {
            return Err(JournalError::Invalid);
        }
        let record = document
            .records
            .iter()
            .find(|record| record.id == self.id)
            .ok_or(JournalError::Invalid)?;
        if record.scope != Scope::PublicBlock
            || record.resource
                != (Resource::NftTable {
                    table_id: self.table_id,
                    family: NftFamily::Inet,
                })
        {
            return Err(JournalError::Conflict);
        }
        match &self.phase {
            PublicPolicyPhase::Removing { candidates } => {
                if candidates.is_empty()
                    || candidates.len() > 2
                    || (candidates.len() == 2 && candidates[0] == candidates[1])
                {
                    return Err(JournalError::Invalid);
                }
                if record.state != RecordState::CleanupRemaining {
                    return Err(JournalError::Conflict);
                }
            }
            PublicPolicyPhase::Present { .. } => {
                if record.state == RecordState::CleanupRemaining {
                    return Err(JournalError::Conflict);
                }
            }
            PublicPolicyPhase::Stable { .. } => {
                if record.state != RecordState::Confirmed {
                    return Err(JournalError::Conflict);
                }
            }
        }
        Ok(())
    }
}
impl Journal {
    pub fn public_policy(&self) -> Option<&PublicPolicyRecord> {
        self.document.public_policy.as_ref()
    }
    pub(super) fn check_public_removal(&self, id: u64) -> Result<(), JournalError> {
        if self.public_policy().is_some_and(|policy| {
            policy.id == id && !matches!(policy.phase, PublicPolicyPhase::Removing { .. })
        }) {
            return Err(JournalError::Conflict);
        }
        Ok(())
    }
    /// Intent и типизированная политика сохраняются до возможного эффекта nft.
    pub fn begin_public_policy(
        &mut self,
        ownership: Ownership,
        id: u64,
        table_id: u64,
        desired: PublicBlockPolicy,
    ) -> Result<u64, JournalError> {
        self.check(ownership)?;
        if id == 0 || table_id == 0 {
            return Err(JournalError::Invalid);
        }
        let resource = Resource::NftTable {
            table_id,
            family: NftFamily::Inet,
        };
        let previous = if let Some(policy) = self.public_policy() {
            if policy.id != id || policy.table_id != table_id {
                return Err(JournalError::Conflict);
            }
            match &policy.phase {
                PublicPolicyPhase::Present {
                    desired: current, ..
                } if *current == desired => return Ok(policy.revision),
                PublicPolicyPhase::Stable { policy } => Some(policy.clone()),
                _ => return Err(JournalError::Conflict),
            }
        } else {
            None
        };
        let mut document = self.document.clone();
        if let Some(record) = document.records.iter_mut().find(|record| record.id == id) {
            if record.scope != Scope::PublicBlock
                || record.resource != resource
                || record.state == RecordState::CleanupRemaining
            {
                return Err(JournalError::Conflict);
            }
            // Старые записи без политики не доказывают содержимое существующей таблицы.
            if previous.is_none() {
                return Err(JournalError::Conflict);
            }
            record.state = RecordState::Intent;
        } else {
            if document.records.len() == super::MAX_RECORDS {
                return Err(JournalError::Limit);
            }
            document.records.push(Record {
                id,
                scope: Scope::PublicBlock,
                resource,
                state: RecordState::Intent,
            });
        }
        let revision = document
            .policy_sequence
            .checked_add(1)
            .ok_or(JournalError::Limit)?;
        document.policy_sequence = revision;
        document.public_policy = Some(PublicPolicyRecord {
            id,
            table_id,
            revision,
            phase: PublicPolicyPhase::Present { previous, desired },
        });
        self.persist(document)?;
        self.reconciled.remove(&id);
        Ok(revision)
    }
    /// Сохраняет все возможные результаты прерванной замены перед удалением.
    pub fn begin_public_unlock(
        &mut self,
        ownership: Ownership,
        id: u64,
    ) -> Result<u64, JournalError> {
        self.check(ownership)?;
        let policy = self.public_policy().ok_or(JournalError::Missing)?;
        if policy.id != id {
            return Err(JournalError::Conflict);
        }
        let candidates = match &policy.phase {
            PublicPolicyPhase::Removing { .. } => return Ok(policy.revision),
            PublicPolicyPhase::Stable { policy } => vec![policy.clone()],
            PublicPolicyPhase::Present { previous, desired } => {
                let mut candidates: Vec<_> = previous.iter().cloned().collect();
                if !candidates.contains(desired) {
                    candidates.push(desired.clone());
                }
                candidates
            }
        };
        let mut document = self.document.clone();
        let revision = document
            .policy_sequence
            .checked_add(1)
            .ok_or(JournalError::Limit)?;
        document.policy_sequence = revision;
        let policy = document
            .public_policy
            .as_mut()
            .ok_or(JournalError::Missing)?;
        policy.revision = revision;
        policy.phase = PublicPolicyPhase::Removing { candidates };
        document
            .records
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or(JournalError::Missing)?
            .state = RecordState::CleanupRemaining;
        self.persist(document)?;
        self.reconciled.remove(&id);
        Ok(revision)
    }
    /// Подтверждение разрешено только после независимой сверки backend.
    pub fn confirm_public_policy(
        &mut self,
        ownership: Ownership,
        id: u64,
        revision: u64,
    ) -> Result<(), JournalError> {
        self.check(ownership)?;
        let policy = self.public_policy().ok_or(JournalError::Missing)?;
        if policy.id != id || policy.revision != revision {
            return Err(JournalError::Conflict);
        }
        if !self.reconciled.contains(&id) {
            return Err(JournalError::ReconcileRequired);
        }
        let desired = match &policy.phase {
            PublicPolicyPhase::Present { desired, .. } => desired.clone(),
            PublicPolicyPhase::Stable { .. } => return Ok(()),
            PublicPolicyPhase::Removing { .. } => return Err(JournalError::Conflict),
        };
        let mut document = self.document.clone();
        document
            .public_policy
            .as_mut()
            .ok_or(JournalError::Missing)?
            .phase = PublicPolicyPhase::Stable { policy: desired };
        document
            .records
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or(JournalError::Missing)?
            .state = RecordState::Confirmed;
        self.persist(document)
    }
}

#[cfg(test)]
mod tests {
    use super::super::Reconciled;
    use super::*;
    use std::{fs, os::unix::fs::DirBuilderExt, path::PathBuf};
    struct Area(PathBuf);
    impl Area {
        fn new() -> Self {
            let mut nonce = [0; 8];
            getrandom::getrandom(&mut nonce).expect("Случайность теста");
            let path = std::env::temp_dir()
                .join(format!("yonik-policy-{:016x}", u64::from_le_bytes(nonce)));
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .expect("Каталог теста");
            Self(path)
        }
        fn open(&self) -> Journal {
            Journal::open(&self.0.join("state"), owner()).expect("Журнал")
        }
    }
    impl Drop for Area {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("Очистка собственного теста");
        }
    }
    fn owner() -> Ownership {
        Ownership::new(1000, 1, 1, 4, 99).expect("Владелец")
    }
    fn policy(tun: u32) -> PublicBlockPolicy {
        PublicBlockPolicy::new(1, Some(tun), vec![], vec![], vec![], vec![]).expect("Политика")
    }
    #[test]
    fn pending_replacement_unlock_and_monotonic_reopen_are_atomic() {
        let area = Area::new();
        let mut journal = area.open();
        let first = journal
            .begin_public_policy(owner(), 7, 9, policy(2))
            .unwrap();
        assert_eq!(first, 1);
        assert_eq!(
            journal
                .begin_public_policy(owner(), 7, 9, policy(2))
                .unwrap(),
            first
        );
        assert_eq!(
            journal.begin_public_policy(owner(), 7, 9, policy(3)),
            Err(JournalError::Conflict)
        );
        assert_eq!(
            journal.confirm_public_policy(owner(), 7, first),
            Err(JournalError::ReconcileRequired)
        );
        journal.reconcile(owner(), 7, Reconciled::Owned).unwrap();
        journal.confirm_public_policy(owner(), 7, first).unwrap();
        let second = journal
            .begin_public_policy(owner(), 7, 9, policy(3))
            .unwrap();
        assert_eq!(second, 2);
        assert!(journal.needs_reconciliation(7));
        drop(journal);
        let mut journal = area.open();
        assert!(
            matches!(journal.public_policy().unwrap().phase(), PublicPolicyPhase::Present { previous: Some(previous), desired } if *previous == policy(2) && *desired == policy(3))
        );
        let third = journal.begin_public_unlock(owner(), 7).unwrap();
        assert_eq!(third, 3);
        assert!(
            matches!(journal.public_policy().unwrap().phase(), PublicPolicyPhase::Removing { candidates } if candidates == &vec![policy(2), policy(3)])
        );
        assert_eq!(journal.begin_public_unlock(owner(), 7).unwrap(), third);
        assert_eq!(
            journal.begin_public_policy(owner(), 7, 9, policy(2)),
            Err(JournalError::Conflict)
        );
        drop(journal);
        let mut journal = area.open();
        assert_eq!(journal.records()[0].state(), RecordState::CleanupRemaining);
        journal.reconcile(owner(), 7, Reconciled::Absent).unwrap();
        assert!(journal.public_policy().is_none());
        assert!(journal.records().is_empty());
        drop(journal);
        let mut journal = area.open();
        assert_eq!(
            journal
                .begin_public_policy(owner(), 7, 9, policy(4))
                .unwrap(),
            4
        );
    }
    #[test]
    fn generic_removal_and_revision_replay_cannot_discard_desired_policy() {
        let area = Area::new();
        let mut journal = area.open();
        let revision = journal
            .begin_public_policy(owner(), 7, 9, policy(2))
            .unwrap();
        assert_eq!(
            journal.reconcile(owner(), 7, Reconciled::Absent),
            Err(JournalError::Conflict)
        );
        journal.reconcile(owner(), 7, Reconciled::Owned).unwrap();
        assert_eq!(
            journal.mark_cleanup(owner(), 7),
            Err(JournalError::Conflict)
        );
        assert_eq!(
            journal.cleanup(owner(), Scope::PublicBlock, |_| panic!("Callback запрещён")),
            Err(JournalError::Conflict)
        );
        assert_eq!(
            journal.confirm_public_policy(owner(), 7, revision + 1),
            Err(JournalError::Conflict)
        );
        let foreign = Ownership::new(1001, 1, 1, 4, 99).unwrap();
        assert_eq!(
            journal.confirm_public_policy(foreign, 7, revision),
            Err(JournalError::WrongOwnership)
        );
        journal.confirm_public_policy(owner(), 7, revision).unwrap();
        let next = journal
            .begin_public_policy(owner(), 7, 9, policy(2))
            .unwrap();
        assert_eq!(next, revision + 1);
        journal.begin_public_unlock(owner(), 7).unwrap();
        assert!(
            matches!(journal.public_policy().unwrap().phase(), PublicPolicyPhase::Removing { candidates } if candidates.len() == 1)
        );
        journal.reconcile(owner(), 7, Reconciled::Owned).unwrap();
        assert_eq!(
            journal
                .cleanup(owner(), Scope::PublicBlock, |_| Ok(()))
                .unwrap()
                .removed,
            1
        );
        assert!(journal.public_policy().is_none());
    }
    #[test]
    fn strict_migration_and_invalid_policy_binding_preserve_file() {
        let area = Area::new();
        drop(area.open());
        let file = area.0.join("state/journal.json");
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        legacy["version"] = 1.into();
        legacy.as_object_mut().unwrap().remove("public_policy");
        legacy.as_object_mut().unwrap().remove("policy_sequence");
        fs::write(&file, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let mut journal = area.open();
        let migrated: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        assert_eq!(migrated["version"], 2);
        journal
            .begin_public_policy(owner(), 7, 9, policy(2))
            .unwrap();
        drop(journal);
        let valid: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        for pointer in [
            "/public_policy/id",
            "/public_policy/table_id",
            "/public_policy/revision",
        ] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(pointer).unwrap() = 0.into();
            let bytes = serde_json::to_vec(&invalid).unwrap();
            fs::write(&file, &bytes).unwrap();
            assert!(Journal::open(&area.0.join("state"), owner()).is_err());
            assert_eq!(fs::read(&file).unwrap(), bytes);
        }
        legacy["policy_sequence"] = 0.into();
        let bytes = serde_json::to_vec(&legacy).unwrap();
        fs::write(&file, &bytes).unwrap();
        assert!(matches!(
            Journal::open(&area.0.join("state"), owner()),
            Err(JournalError::Corrupt)
        ));
    }
    #[test]
    fn removing_schema_rejects_ambiguous_candidates_and_mismatched_resource() {
        let area = Area::new();
        let mut journal = area.open();
        journal
            .begin_public_policy(owner(), 7, 9, policy(2))
            .unwrap();
        journal.begin_public_unlock(owner(), 7).unwrap();
        let valid = serde_json::to_value(&journal.document).unwrap();
        for candidates in [
            vec![],
            vec![policy(2), policy(2)],
            vec![policy(2), policy(3), policy(4)],
        ] {
            let mut value = valid.clone();
            value["public_policy"]["phase"]["candidates"] =
                serde_json::to_value(candidates).unwrap();
            let document: Document = serde_json::from_value(value).unwrap();
            assert!(document.validate().is_err());
        }
        for (pointer, replacement) in [
            ("/records/0/resource/family", serde_json::json!("Ip")),
            ("/records/0/scope", serde_json::json!("Session")),
            ("/records/0/state", serde_json::json!("Confirmed")),
        ] {
            let mut value = valid.clone();
            *value.pointer_mut(pointer).unwrap() = replacement;
            let document: Document = serde_json::from_value(value).unwrap();
            assert!(document.validate().is_err());
        }
        let mut value = valid;
        value["public_policy"]["phase"]["unexpected"] = true.into();
        assert!(serde_json::from_value::<Document>(value).is_err());
    }
    #[test]
    fn revision_overflow_capacity_and_debug_are_bounded() {
        let area = Area::new();
        let mut journal = area.open();
        journal.document.policy_sequence = u64::MAX;
        assert_eq!(
            journal.begin_public_policy(owner(), 7, 9, policy(2)),
            Err(JournalError::Limit)
        );
        assert!(journal.records().is_empty());
        journal.document.policy_sequence = 0;
        for id in 1..=super::super::MAX_RECORDS as u64 {
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
            journal.begin_public_policy(owner(), 999, 9, policy(2)),
            Err(JournalError::Limit)
        );
        let record = PublicPolicyRecord {
            id: 7,
            table_id: 9,
            revision: 1,
            phase: PublicPolicyPhase::Stable { policy: policy(2) },
        };
        assert_eq!(format!("{record:?}"), "PublicPolicyRecord(<скрыто>)");
    }
}
