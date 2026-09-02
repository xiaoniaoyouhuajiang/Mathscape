use std::fmt;

/// A deterministic failure at the Mathscape domain boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DomainError {
    EmptyValue { field: &'static str },
    ControlCharacter { field: &'static str },
    InvalidStableId { value: String, reason: &'static str },
    InvalidRevisionId { value: String, reason: &'static str },
    InvalidEndpointRole { value: String },
    TooFewRelationEndpoints { actual: usize },
    MissingEndpointOrdinal { role: String },
    DuplicateEndpointSlot { role: String, ordinal: Option<u32> },
    EmptyPatch,
    EmptyPatchMessage,
    DuplicatePatchTarget { stable_id: String },
    RevisionTargetMismatch { stable_id: String },
    UnchangedRevision { stable_id: String },
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
            Self::TooFewRelationEndpoints { actual } => write!(
                formatter,
                "a relation needs at least two endpoints, but received {actual}"
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
