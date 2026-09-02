use std::collections::HashSet;

use serde::Serialize;

use crate::{
    ActorRef, CommitId, DomainError, EntityEnvelope, RelationRecord, RevisionId, StableId,
};

/// One requested mutation in an atomic map patch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum MapOperation<P> {
    CreateEntity {
        entity: EntityEnvelope<P>,
    },
    ReviseEntity {
        stable_id: StableId,
        expected_revision: RevisionId,
        replacement: EntityEnvelope<P>,
    },
    CreateRelation {
        relation: RelationRecord<P>,
    },
    ReviseRelation {
        stable_id: StableId,
        expected_revision: RevisionId,
        replacement: RelationRecord<P>,
    },
}

impl<P> MapOperation<P> {
    pub fn target(&self) -> &StableId {
        match self {
            Self::CreateEntity { entity } => entity.header().stable_id(),
            Self::ReviseEntity { stable_id, .. } | Self::ReviseRelation { stable_id, .. } => {
                stable_id
            }
            Self::CreateRelation { relation } => relation.header().stable_id(),
        }
    }

    fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::CreateEntity { .. } => Ok(()),
            Self::CreateRelation { relation } => relation.validate(),
            Self::ReviseEntity {
                stable_id,
                expected_revision,
                replacement,
            } => validate_revision(
                stable_id,
                expected_revision,
                replacement.header().stable_id(),
                replacement.header().revision_id(),
            ),
            Self::ReviseRelation {
                stable_id,
                expected_revision,
                replacement,
            } => {
                replacement.validate()?;
                validate_revision(
                    stable_id,
                    expected_revision,
                    replacement.header().stable_id(),
                    replacement.header().revision_id(),
                )
            }
        }
    }
}

fn validate_revision(
    stable_id: &StableId,
    expected_revision: &RevisionId,
    replacement_stable_id: &StableId,
    replacement_revision: &RevisionId,
) -> Result<(), DomainError> {
    if stable_id != replacement_stable_id {
        return Err(DomainError::RevisionTargetMismatch {
            stable_id: stable_id.to_string(),
        });
    }
    if expected_revision == replacement_revision {
        return Err(DomainError::UnchangedRevision {
            stable_id: stable_id.to_string(),
        });
    }
    Ok(())
}

/// Candidate mutations that must either commit atomically or be rejected.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MapPatch<P> {
    base_commit: Option<CommitId>,
    author: ActorRef,
    generation_run: Option<StableId>,
    message: Option<String>,
    operations: Vec<MapOperation<P>>,
}

impl<P> MapPatch<P> {
    pub fn new(
        base_commit: Option<CommitId>,
        author: ActorRef,
        generation_run: Option<StableId>,
        message: Option<String>,
        operations: Vec<MapOperation<P>>,
    ) -> Result<Self, DomainError> {
        let patch = Self {
            base_commit,
            author,
            generation_run,
            message,
            operations,
        };
        patch.validate()?;
        Ok(patch)
    }

    /// Rechecks cross-field invariants after an in-memory transformation.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.operations.is_empty() {
            return Err(DomainError::EmptyPatch);
        }
        if self
            .message
            .as_ref()
            .is_some_and(|message| message.trim().is_empty())
        {
            return Err(DomainError::EmptyPatchMessage);
        }

        let mut targets = HashSet::new();
        for operation in &self.operations {
            operation.validate()?;
            if !targets.insert(operation.target()) {
                return Err(DomainError::DuplicatePatchTarget {
                    stable_id: operation.target().to_string(),
                });
            }
        }

        Ok(())
    }

    pub fn base_commit(&self) -> Option<&CommitId> {
        self.base_commit.as_ref()
    }

    pub fn author(&self) -> &ActorRef {
        &self.author
    }

    pub fn generation_run(&self) -> Option<&StableId> {
        self.generation_run.as_ref()
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn operations(&self) -> &[MapOperation<P>] {
        &self.operations
    }

    pub fn into_operations(self) -> Vec<MapOperation<P>> {
        self.operations
    }
}

#[cfg(test)]
mod tests {
    use crate::{EntityHeader, EntityKind, LifecycleStatus, SchemaVersion, Timestamp};

    use super::*;

    fn revision(digit: char) -> RevisionId {
        RevisionId::parse(format!("sha256:{}", digit.to_string().repeat(64))).unwrap()
    }

    fn entity(id: &str, revision_id: RevisionId) -> EntityEnvelope<()> {
        EntityEnvelope::new(
            EntityHeader::new(
                StableId::parse(id).unwrap(),
                revision_id,
                EntityKind::SemanticEntity,
                SchemaVersion::parse("mathscape-domain-0.1").unwrap(),
                LifecycleStatus::Candidate,
                Timestamp::parse("2026-09-02T00:00:00+08:00").unwrap(),
                ActorRef::parse("domain-test").unwrap(),
                None,
            ),
            (),
        )
    }

    #[test]
    fn rejects_empty_patches() {
        let error = MapPatch::<()>::new(
            None,
            ActorRef::parse("domain-test").unwrap(),
            None,
            None,
            vec![],
        )
        .unwrap_err();
        assert_eq!(error, DomainError::EmptyPatch);
    }

    #[test]
    fn rejects_multiple_mutations_of_one_target() {
        let id = "mathscape:concept/sequence-convergence";
        let error = MapPatch::new(
            None,
            ActorRef::parse("domain-test").unwrap(),
            None,
            Some("duplicate target".to_owned()),
            vec![
                MapOperation::CreateEntity {
                    entity: entity(id, revision('1')),
                },
                MapOperation::CreateEntity {
                    entity: entity(id, revision('2')),
                },
            ],
        )
        .unwrap_err();

        assert_eq!(
            error,
            DomainError::DuplicatePatchTarget {
                stable_id: id.to_owned()
            }
        );
    }

    #[test]
    fn revision_requires_matching_identity_and_new_content() {
        let id = StableId::parse("mathscape:concept/sequence-convergence").unwrap();
        let old_revision = revision('1');

        let mismatch = MapPatch::new(
            None,
            ActorRef::parse("domain-test").unwrap(),
            None,
            None,
            vec![MapOperation::ReviseEntity {
                stable_id: id.clone(),
                expected_revision: old_revision.clone(),
                replacement: entity("mathscape:concept/other", revision('2')),
            }],
        )
        .unwrap_err();
        assert!(matches!(
            mismatch,
            DomainError::RevisionTargetMismatch { .. }
        ));

        let unchanged = MapPatch::new(
            None,
            ActorRef::parse("domain-test").unwrap(),
            None,
            None,
            vec![MapOperation::ReviseEntity {
                stable_id: id,
                expected_revision: old_revision.clone(),
                replacement: entity("mathscape:concept/sequence-convergence", old_revision),
            }],
        )
        .unwrap_err();
        assert!(matches!(unchanged, DomainError::UnchangedRevision { .. }));
    }
}
