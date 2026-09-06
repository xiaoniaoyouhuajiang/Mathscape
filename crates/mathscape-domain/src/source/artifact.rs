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
    Managed,
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
    Unavailable,
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
    location_id: StableId,
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
        location_id: StableId,
        artifact: StableId,
        storage_kind: StorageKind,
        uri: ArtifactUri,
        priority: i32,
        role: LocationRole,
        last_verified_at: Option<Timestamp>,
        status: LocationStatus,
    ) -> Self {
        Self {
            location_id,
            artifact,
            storage_kind,
            uri,
            priority,
            role,
            last_verified_at,
            status,
        }
    }

    pub fn location_id(&self) -> &StableId {
        &self.location_id
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactCheckResult {
    Valid,
    Missing,
    Corrupted,
    AccessDenied,
    Unavailable,
}

/// One append-only observation of bytes reachable through an ArtifactLocation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ArtifactCheck {
    check_id: StableId,
    location: StableId,
    checked_at: Timestamp,
    observed_hash: Option<ContentHash>,
    observed_size: Option<u64>,
    result: ArtifactCheckResult,
    checker_version: String,
}

impl ArtifactCheck {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        check_id: StableId,
        location: StableId,
        checked_at: Timestamp,
        observed_hash: Option<ContentHash>,
        observed_size: Option<u64>,
        result: ArtifactCheckResult,
        checker_version: String,
    ) -> Result<Self, DomainError> {
        ensure_non_blank(&checker_version, "artifact_check.checker_version")?;
        if result == ArtifactCheckResult::Valid
            && (observed_hash.is_none() || observed_size.is_none())
        {
            return Err(DomainError::InconsistentState {
                field: "artifact_check.valid_observation",
            });
        }
        Ok(Self {
            check_id,
            location,
            checked_at,
            observed_hash,
            observed_size,
            result,
            checker_version,
        })
    }

    pub fn check_id(&self) -> &StableId {
        &self.check_id
    }

    pub fn location(&self) -> &StableId {
        &self.location
    }

    pub fn checked_at(&self) -> &Timestamp {
        &self.checked_at
    }

    pub fn observed_hash(&self) -> Option<&ContentHash> {
        self.observed_hash.as_ref()
    }

    pub fn observed_size(&self) -> Option<u64> {
        self.observed_size
    }

    pub fn result(&self) -> ArtifactCheckResult {
        self.result
    }

    pub fn checker_version(&self) -> &str {
        &self.checker_version
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
            StableId::parse("mathscape:location/source-a-pdf-primary").unwrap(),
            StableId::parse("emir-fixture:artifact/source-a-pdf").unwrap(),
            StorageKind::Managed,
            ArtifactUri::parse(
                "pool:sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
            1,
            LocationRole::Primary,
            None,
            LocationStatus::Available,
        );
        assert_eq!(
            location.uri().as_str(),
            "pool:sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn valid_check_requires_a_complete_observation() {
        let error = ArtifactCheck::new(
            StableId::parse("mathscape:check/example").unwrap(),
            StableId::parse("mathscape:location/example").unwrap(),
            Timestamp::parse("2026-09-04T00:00:00+08:00").unwrap(),
            None,
            Some(42),
            ArtifactCheckResult::Valid,
            "mathscape-artifact/0.1.0".to_owned(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            DomainError::InconsistentState {
                field: "artifact_check.valid_observation"
            }
        );
    }
}
