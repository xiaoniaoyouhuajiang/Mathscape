use std::fmt;

use crate::entity::{EntityKind, PayloadShape};

/// A deterministic failure at the Mathscape domain boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DomainError {
    EmptyValue {
        field: &'static str,
    },
    ControlCharacter {
        field: &'static str,
    },
    InvalidStableId {
        value: String,
        reason: &'static str,
    },
    InvalidRevisionId {
        value: String,
        reason: &'static str,
    },
    InvalidEndpointRole {
        value: String,
    },
    InvalidMediaType {
        value: String,
    },
    InvalidArtifactUri {
        value: String,
    },
    DuplicateValue {
        field: &'static str,
        value: String,
    },
    TooManyItems {
        field: &'static str,
        maximum: u64,
    },
    InconsistentState {
        field: &'static str,
    },
    EntityKindMismatch {
        expected: EntityKind,
        actual: EntityKind,
    },
    PayloadShapeMismatch {
        expected: PayloadShape,
        actual: PayloadShape,
    },
    EmptyCollection {
        field: &'static str,
    },
    DuplicateReference {
        field: &'static str,
        stable_id: String,
    },
    InvalidConfidence,
    InvalidRange {
        field: &'static str,
        start: u64,
        end: u64,
    },
    ArtifactMismatch {
        field: &'static str,
    },
    InvalidBoundingBox,
    SuccessfulRunWithoutOutput,
    MissingEvidence,
    TooFewRelationEndpoints {
        actual: usize,
    },
    MissingEndpointOrdinal {
        role: String,
    },
    DuplicateEndpointSlot {
        role: String,
        ordinal: Option<u32>,
    },
    EmptyPatch,
    EmptyPatchMessage,
    DuplicatePatchTarget {
        stable_id: String,
    },
    RevisionTargetMismatch {
        stable_id: String,
    },
    UnchangedRevision {
        stable_id: String,
    },
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyValue { field } => write!(formatter, "{field} must not be empty"),
            Self::ControlCharacter { field } => {
                write!(formatter, "{field} must not contain control characters")
            }
            Self::InvalidStableId { value, reason } => {
                write!(formatter, "invalid stable ID {value:?}: {reason}")
            }
            Self::InvalidRevisionId { value, reason } => {
                write!(formatter, "invalid revision ID {value:?}: {reason}")
            }
            Self::InvalidEndpointRole { value } => write!(
                formatter,
                "invalid endpoint role {value:?}: expected lower_snake_case"
            ),
            Self::InvalidMediaType { value } => {
                write!(
                    formatter,
                    "invalid media type {value:?}: expected type/subtype"
                )
            }
            Self::InvalidArtifactUri { value } => {
                write!(
                    formatter,
                    "invalid artifact URI {value:?}: a scheme is required"
                )
            }
            Self::DuplicateValue { field, value } => {
                write!(formatter, "{field} contains duplicate value {value:?}")
            }
            Self::TooManyItems { field, maximum } => {
                write!(
                    formatter,
                    "{field} exceeds the supported maximum of {maximum}"
                )
            }
            Self::InconsistentState { field } => {
                write!(formatter, "{field} contains an inconsistent state")
            }
            Self::EntityKindMismatch { expected, actual } => write!(
                formatter,
                "payload kind {expected:?} does not match header kind {actual:?}"
            ),
            Self::PayloadShapeMismatch { expected, actual } => write!(
                formatter,
                "payload shape {actual:?} cannot be stored as {expected:?}"
            ),
            Self::EmptyCollection { field } => {
                write!(formatter, "{field} must contain at least one item")
            }
            Self::DuplicateReference { field, stable_id } => {
                write!(
                    formatter,
                    "{field} contains duplicate reference {stable_id:?}"
                )
            }
            Self::InvalidConfidence => {
                write!(formatter, "confidence must be finite and between 0 and 1")
            }
            Self::InvalidRange { field, start, end } => write!(
                formatter,
                "{field} range must be non-empty and ordered, received {start}..{end}"
            ),
            Self::ArtifactMismatch { field } => {
                write!(formatter, "{field} refers to a different artifact")
            }
            Self::InvalidBoundingBox => write!(
                formatter,
                "bounding box coordinates must be normalized and strictly ordered"
            ),
            Self::SuccessfulRunWithoutOutput => {
                write!(
                    formatter,
                    "a successful conversion run must have output artifacts"
                )
            }
            Self::MissingEvidence => {
                write!(
                    formatter,
                    "a source-unit relation requires an evidence span"
                )
            }
            Self::TooFewRelationEndpoints { actual } => write!(
                formatter,
                "a relation or provenance event needs at least one endpoint, but received {actual}"
            ),
            Self::MissingEndpointOrdinal { role } => write!(
                formatter,
                "repeated endpoint role {role:?} requires an ordinal on every endpoint"
            ),
            Self::DuplicateEndpointSlot { role, ordinal } => write!(
                formatter,
                "duplicate endpoint slot for role {role:?} and ordinal {ordinal:?}"
            ),
            Self::EmptyPatch => write!(formatter, "a map patch must contain an operation"),
            Self::EmptyPatchMessage => {
                write!(formatter, "a patch message must not be blank when present")
            }
            Self::DuplicatePatchTarget { stable_id } => write!(
                formatter,
                "a map patch may mutate {stable_id:?} at most once"
            ),
            Self::RevisionTargetMismatch { stable_id } => write!(
                formatter,
                "replacement stable ID does not match revision target {stable_id:?}"
            ),
            Self::UnchangedRevision { stable_id } => write!(
                formatter,
                "revision of {stable_id:?} must introduce a new revision ID"
            ),
        }
    }
}

impl std::error::Error for DomainError {}
