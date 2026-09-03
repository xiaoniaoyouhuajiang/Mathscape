use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{DomainError, EntityHeader, EntityPayload, PayloadShape, StableId};

/// Semantic role occupied by one endpoint in a relation or hyperedge.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EndpointRole(String);

impl EndpointRole {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let mut characters = value.chars();
        let valid = characters
            .next()
            .is_some_and(|character| character.is_ascii_lowercase())
            && characters.all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
            });

        if !valid {
            return Err(DomainError::InvalidEndpointRole { value });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for EndpointRole {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<EndpointRole> for String {
    fn from(value: EndpointRole) -> Self {
        value.0
    }
}

/// One role-labelled endpoint of a first-class relation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RelationEndpoint {
    role: EndpointRole,
    entity: StableId,
    ordinal: Option<u32>,
}

impl RelationEndpoint {
    pub fn new(role: EndpointRole, entity: StableId, ordinal: Option<u32>) -> Self {
        Self {
            role,
            entity,
            ordinal,
        }
    }

    pub fn role(&self) -> &EndpointRole {
        &self.role
    }

    pub fn entity(&self) -> &StableId {
        &self.entity
    }

    pub fn ordinal(&self) -> Option<u32> {
        self.ordinal
    }
}

/// A first-class relation, hyperedge, or provenance event with its own revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RelationRecord<P> {
    header: EntityHeader,
    endpoints: Vec<RelationEndpoint>,
    payload: P,
}

impl<P: EntityPayload> RelationRecord<P> {
    pub fn new(
        header: EntityHeader,
        endpoints: Vec<RelationEndpoint>,
        payload: P,
    ) -> Result<Self, DomainError> {
        let actual_shape = payload.payload_shape();
        if actual_shape != PayloadShape::Relation {
            return Err(DomainError::PayloadShapeMismatch {
                expected: PayloadShape::Relation,
                actual: actual_shape,
            });
        }
        let expected = payload.entity_kind();
        let actual = header.kind();
        if expected != actual {
            return Err(DomainError::EntityKindMismatch { expected, actual });
        }

        let relation = Self {
            header,
            endpoints,
            payload,
        };
        relation.validate()?;
        Ok(relation)
    }
}

impl<P> RelationRecord<P> {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.endpoints.is_empty() {
            return Err(DomainError::TooFewRelationEndpoints {
                actual: self.endpoints.len(),
            });
        }

        let mut ordinals_by_role: HashMap<&str, Vec<Option<u32>>> = HashMap::new();
        for endpoint in &self.endpoints {
            ordinals_by_role
                .entry(endpoint.role.as_str())
                .or_default()
                .push(endpoint.ordinal);
        }

        for (role, ordinals) in ordinals_by_role {
            if ordinals.len() > 1 && ordinals.iter().any(Option::is_none) {
                return Err(DomainError::MissingEndpointOrdinal {
                    role: role.to_owned(),
                });
            }

            let mut occupied = HashSet::new();
            for ordinal in ordinals {
                if !occupied.insert(ordinal) {
                    return Err(DomainError::DuplicateEndpointSlot {
                        role: role.to_owned(),
                        ordinal,
                    });
                }
            }
        }

        Ok(())
    }

    pub fn header(&self) -> &EntityHeader {
        &self.header
    }

    pub fn endpoints(&self) -> &[RelationEndpoint] {
        &self.endpoints
    }

    pub fn payload(&self) -> &P {
        &self.payload
    }

    pub fn into_parts(self) -> (EntityHeader, Vec<RelationEndpoint>, P) {
        (self.header, self.endpoints, self.payload)
    }
}

#[cfg(test)]
mod tests {
    use crate::{ActorRef, EntityKind, LifecycleStatus, RevisionId, SchemaVersion, Timestamp};

    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TestInference;

    impl EntityPayload for TestInference {
        fn entity_kind(&self) -> EntityKind {
            EntityKind::Inference
        }

        fn payload_shape(&self) -> PayloadShape {
            PayloadShape::Relation
        }
    }

    fn header() -> EntityHeader {
        EntityHeader::new(
            StableId::parse("mathscape:inference/example").unwrap(),
            RevisionId::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
            EntityKind::Inference,
            SchemaVersion::parse("mathscape-domain-0.1").unwrap(),
            LifecycleStatus::Candidate,
            Timestamp::parse("2026-09-02T00:00:00+08:00").unwrap(),
            ActorRef::parse("domain-test").unwrap(),
            None,
        )
    }

    fn endpoint(role: &str, target: &str, ordinal: Option<u32>) -> RelationEndpoint {
        RelationEndpoint::new(
            EndpointRole::parse(role).unwrap(),
            StableId::parse(target).unwrap(),
            ordinal,
        )
    }

    #[test]
    fn accepts_role_labelled_hyperedges() {
        let relation = RelationRecord::new(
            header(),
            vec![
                endpoint("premise", "mathscape:state/a", Some(0)),
                endpoint("premise", "mathscape:state/b", Some(1)),
                endpoint("conclusion", "mathscape:state/c", None),
            ],
            TestInference,
        )
        .unwrap();

        assert_eq!(relation.endpoints().len(), 3);
    }

    #[test]
    fn rejects_relations_without_endpoints() {
        let error = RelationRecord::new(header(), vec![], TestInference).unwrap_err();

        assert_eq!(error, DomainError::TooFewRelationEndpoints { actual: 0 });
    }

    #[test]
    fn repeated_roles_require_unique_ordinals() {
        let missing = RelationRecord::new(
            header(),
            vec![
                endpoint("premise", "mathscape:state/a", None),
                endpoint("premise", "mathscape:state/b", None),
            ],
            TestInference,
        )
        .unwrap_err();
        assert_eq!(
            missing,
            DomainError::MissingEndpointOrdinal {
                role: "premise".to_owned()
            }
        );

        let duplicate = RelationRecord::new(
            header(),
            vec![
                endpoint("premise", "mathscape:state/a", Some(0)),
                endpoint("premise", "mathscape:state/b", Some(0)),
            ],
            TestInference,
        )
        .unwrap_err();
        assert_eq!(
            duplicate,
            DomainError::DuplicateEndpointSlot {
                role: "premise".to_owned(),
                ordinal: Some(0)
            }
        );
    }
}
