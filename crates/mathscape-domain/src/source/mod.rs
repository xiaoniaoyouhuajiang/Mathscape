mod artifact;
mod conversion;
mod identity;
mod span;
mod unit;

use std::collections::HashSet;

use serde::Serialize;

use crate::{
    DomainError, EntityEnvelope, EntityKind, EntityPayload, PayloadShape, RelationRecord, StableId,
};

pub use artifact::*;
pub use conversion::*;
pub use identity::*;
pub use span::*;
pub use unit::*;

/// Heterogeneous payload accepted by the first source-layer slice.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "payload_kind", content = "payload", rename_all = "snake_case")]
pub enum SourcePayload {
    Work(SourceWork),
    Edition(SourceEdition),
    Artifact(SourceArtifact),
    ConversionRun(ConversionRun),
    Span(SourceSpan),
    SemanticUnit(SourceSemanticUnit),
    UnitRelation(SourceUnitRelation),
}

impl EntityPayload for SourcePayload {
    fn entity_kind(&self) -> EntityKind {
        match self {
            Self::Work(payload) => payload.entity_kind(),
            Self::Edition(payload) => payload.entity_kind(),
            Self::Artifact(payload) => payload.entity_kind(),
            Self::ConversionRun(payload) => payload.entity_kind(),
            Self::Span(payload) => payload.entity_kind(),
            Self::SemanticUnit(payload) => payload.entity_kind(),
            Self::UnitRelation(payload) => payload.entity_kind(),
        }
    }

    fn payload_shape(&self) -> PayloadShape {
        match self {
            Self::ConversionRun(payload) => payload.payload_shape(),
            Self::UnitRelation(payload) => payload.payload_shape(),
            _ => PayloadShape::Entity,
        }
    }
}

macro_rules! source_payload_from {
    ($payload:ty, $variant:ident) => {
        impl From<$payload> for SourcePayload {
            fn from(payload: $payload) -> Self {
                Self::$variant(payload)
            }
        }
    };
}

source_payload_from!(SourceWork, Work);
source_payload_from!(SourceEdition, Edition);
source_payload_from!(SourceArtifact, Artifact);
source_payload_from!(ConversionRun, ConversionRun);
source_payload_from!(SourceSpan, Span);
source_payload_from!(SourceSemanticUnit, SemanticUnit);
source_payload_from!(SourceUnitRelation, UnitRelation);

pub type SourceEntity = EntityEnvelope<SourcePayload>;
pub type SourceRelation = RelationRecord<SourcePayload>;

pub(super) fn ensure_non_blank(value: &str, field: &'static str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(DomainError::EmptyValue { field });
    }
    if value.chars().any(char::is_control) {
        return Err(DomainError::ControlCharacter { field });
    }
    Ok(())
}

pub(super) fn ensure_optional_non_blank(
    value: Option<&str>,
    field: &'static str,
) -> Result<(), DomainError> {
    if let Some(value) = value {
        ensure_non_blank(value, field)?;
    }
    Ok(())
}

pub(super) fn ensure_non_empty_unique_refs(
    values: &[StableId],
    field: &'static str,
) -> Result<(), DomainError> {
    if values.is_empty() {
        return Err(DomainError::EmptyCollection { field });
    }
    ensure_unique_refs(values, field)
}

pub(super) fn ensure_unique_refs(
    values: &[StableId],
    field: &'static str,
) -> Result<(), DomainError> {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(DomainError::DuplicateReference {
                field,
                stable_id: value.to_string(),
            });
        }
    }
    Ok(())
}
