//! Authoritative Mathscape domain identities and mutation contracts.
//!
//! This crate is deliberately independent of SQLite, the desktop UI, LLM
//! providers, Marker, and Lean. It defines the narrow waist that all writers
//! must cross before data can enter the canonical map.

mod canonical_markdown;
mod canonical_reader;
mod discovery;
mod discovery_proposal;
mod entity;
mod error;
mod id;
mod ink;
mod map_edit;
mod markdown_patch;
mod patch;
mod registration;
mod relation;
mod run_disposition;
mod semantic;
mod source;
mod source_review;
mod staged_discovery;
mod trust;

pub use canonical_markdown::*;
pub use canonical_reader::*;
pub use discovery::*;
pub use discovery_proposal::*;
pub use entity::{
    EntityEnvelope, EntityHeader, EntityKind, EntityPayload, LifecycleStatus, PayloadShape,
};
pub use error::DomainError;
pub use id::{ActorRef, CommitId, ContentHash, RevisionId, SchemaVersion, StableId, Timestamp};
pub use ink::*;
pub use map_edit::*;
pub use markdown_patch::*;
pub use patch::{MapOperation, MapPatch};
pub use registration::*;
pub use relation::{EndpointRole, RelationEndpoint, RelationRecord};
pub use run_disposition::*;
pub use semantic::*;
pub use source::*;
pub use source_review::*;
pub use staged_discovery::*;
pub use trust::*;
