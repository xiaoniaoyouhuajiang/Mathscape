//! Authoritative Mathscape domain identities and mutation contracts.
//!
//! This crate is deliberately independent of SQLite, the desktop UI, LLM
//! providers, Marker, and Lean. It defines the narrow waist that all writers
//! must cross before data can enter the canonical map.

mod entity;
mod error;
mod id;
mod patch;
mod relation;
mod source;
mod trust;

pub use entity::{
    EntityEnvelope, EntityHeader, EntityKind, EntityPayload, LifecycleStatus, PayloadShape,
};
pub use error::DomainError;
pub use id::{ActorRef, CommitId, ContentHash, RevisionId, SchemaVersion, StableId, Timestamp};
pub use patch::{MapOperation, MapPatch};
pub use relation::{EndpointRole, RelationEndpoint, RelationRecord};
pub use source::*;
pub use trust::*;
