use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{ContentHash, DomainError, EntityKind, EntityPayload, StableId, Timestamp};

use super::{ensure_non_blank, ensure_optional_non_blank};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MediaType(String);

impl MediaType {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        ensure_non_blank(&value, "source_artifact.media_type")?;
        let Some((category, subtype)) = value.split_once('/') else {
            return Err(DomainError::InvalidMediaType { value });
        };
        if category.is_empty() || subtype.is_empty() || value.chars().any(char::is_whitespace) {
            return Err(DomainError::InvalidMediaType { value });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for MediaType {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl TryFrom<String> for MediaType {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<MediaType> for String {
    fn from(value: MediaType) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactAvailability {
    Managed,
    ExternallyManaged,
    Restricted,
    Missing,
    Quarantined,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityStatus {
    Unchecked,
    Valid,
    Corrupted,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightsStatus {
    Unknown,
    NotAssessed,
    DerivativeOfUnassessedSource,
    Permitted,
    Restricted,
    Prohibited,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RightsRecord {
    status: RightsStatus,
    permitted_uses: Vec<String>,
    note: Option<String>,
}

impl RightsRecord {
    pub fn new(
        status: RightsStatus,
        permitted_uses: Vec<String>,
        note: Option<String>,
    ) -> Result<Self, DomainError> {
        for permitted_use in &permitted_uses {
            ensure_non_blank(permitted_use, "rights.permitted_use")?;
        }
        ensure_optional_non_blank(note.as_deref(), "rights.note")?;
        Ok(Self {
            status,
            permitted_uses,
            note,
        })
    }

    pub fn status(&self) -> RightsStatus {
        self.status
    }

    pub fn permitted_uses(&self) -> &[String] {
        &self.permitted_uses
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }
}

/// Immutable content registered in the Artifact Pool catalog.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SourceArtifact {
    edition: Option<StableId>,
    media_type: MediaType,
    content_hash: ContentHash,
    byte_size: u64,
    availability: ArtifactAvailability,
    integrity: IntegrityStatus,
    original_filename: Option<String>,
    page_count: Option<u32>,
    rights: RightsRecord,
}

impl SourceArtifact {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        edition: Option<StableId>,
        media_type: MediaType,
        content_hash: ContentHash,
        byte_size: u64,
        availability: ArtifactAvailability,
        integrity: IntegrityStatus,
        original_filename: Option<String>,
        page_count: Option<u32>,
        rights: RightsRecord,
    ) -> Result<Self, DomainError> {
        ensure_optional_non_blank(
            original_filename.as_deref(),
            "source_artifact.original_filename",
        )?;
        Ok(Self {
            edition,
            media_type,
            content_hash,
            byte_size,
            availability,
            integrity,
            original_filename,
            page_count,
            rights,
        })
    }

    pub fn edition(&self) -> Option<&StableId> {
        self.edition.as_ref()
    }

    pub fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    /// The first Artifact Pool uses the content hash itself as its key.
    pub fn artifact_key(&self) -> &ContentHash {
        &self.content_hash
    }

    pub fn content_hash(&self) -> &ContentHash {
        &self.content_hash
    }

    pub fn byte_size(&self) -> u64 {
        self.byte_size
    }

    pub fn availability(&self) -> ArtifactAvailability {
        self.availability
    }

    pub fn integrity(&self) -> IntegrityStatus {
        self.integrity
    }

    pub fn original_filename(&self) -> Option<&str> {
        self.original_filename.as_deref()
    }

    pub fn page_count(&self) -> Option<u32> {
        self.page_count
    }

    pub fn rights(&self) -> &RightsRecord {
        &self.rights
    }
}

impl EntityPayload for SourceArtifact {
    fn entity_kind(&self) -> EntityKind {
        EntityKind::SourceArtifact
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ArtifactUri(String);

impl ArtifactUri {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        ensure_non_blank(&value, "artifact_location.uri")?;
        if !value.contains(':') {
            return Err(DomainError::InvalidArtifactUri { value });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ArtifactUri {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ArtifactUri> for String {
    fn from(value: ArtifactUri) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageKind {
    Local,
    ObjectStore,
    External,
    Archive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationStatus {
    Available,
    Missing,
    Corrupted,
    AccessDenied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationRole {
    Primary,
    Mirror,
    Cache,
    External,
}

/// Mutable operational location kept outside immutable artifact revisions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactLocation {
    artifact: StableId,
    storage_kind: StorageKind,
    uri: ArtifactUri,
    priority: i32,
    role: LocationRole,
    last_verified_at: Option<Timestamp>,
    status: LocationStatus,
}

impl ArtifactLocation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        artifact: StableId,
        storage_kind: StorageKind,
        uri: ArtifactUri,
        priority: i32,
        role: LocationRole,
        last_verified_at: Option<Timestamp>,
        status: LocationStatus,
    ) -> Self {
        Self {
            artifact,
            storage_kind,
            uri,
            priority,
            role,
            last_verified_at,
            status,
        }
    }

    pub fn artifact(&self) -> &StableId {
        &self.artifact
    }

    pub fn storage_kind(&self) -> StorageKind {
        self.storage_kind
    }

    pub fn uri(&self) -> &ArtifactUri {
        &self.uri
    }

    pub fn priority(&self) -> i32 {
        self.priority
    }

    pub fn role(&self) -> LocationRole {
        self.role
    }

    pub fn last_verified_at(&self) -> Option<&Timestamp> {
        self.last_verified_at.as_ref()
    }

    pub fn status(&self) -> LocationStatus {
        self.status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_style_pdf_artifact_is_valid() {
        let hash = ContentHash::parse(
            "sha256:8daeba7deb1fa29fedbaf6c5ebdf0db2e1e6b99e7fb227d6941f489c70ae61ef",
        )
        .unwrap();
        let artifact = SourceArtifact::new(
            Some(
                StableId::parse("emir-fixture:edition/analysis-principles-methods/local-pdf")
                    .unwrap(),
            ),
            MediaType::parse("application/pdf").unwrap(),
            hash.clone(),
            37_621_943,
            ArtifactAvailability::Managed,
            IntegrityStatus::Valid,
            Some("大学数学-数学分析原理与方法_含目录.pdf".to_owned()),
            Some(443),
            RightsRecord::new(
                RightsStatus::NotAssessed,
                vec!["local_research_fixture_reference".to_owned()],
                None,
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(artifact.artifact_key(), &hash);
        assert_eq!(artifact.page_count(), Some(443));
    }

    #[test]
    fn artifact_location_is_independent_of_content_identity() {
        let location = ArtifactLocation::new(
            StableId::parse("emir-fixture:artifact/source-a-pdf").unwrap(),
            StorageKind::Local,
            ArtifactUri::parse("workspace:data/textbooks/source-a.pdf").unwrap(),
            1,
            LocationRole::Primary,
            None,
            LocationStatus::Available,
        );
        assert_eq!(
            location.uri().as_str(),
            "workspace:data/textbooks/source-a.pdf"
        );
    }
}
