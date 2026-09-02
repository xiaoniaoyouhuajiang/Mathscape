//! Authoritative Mathscape domain types and mutation contracts.

/// Identifies a logical object across all of its revisions.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct StableId(pub String);

/// Identifies one immutable revision of a logical object.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RevisionId(pub String);

/// Identifies the atomic commit that accepted a map patch.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CommitId(pub String);
