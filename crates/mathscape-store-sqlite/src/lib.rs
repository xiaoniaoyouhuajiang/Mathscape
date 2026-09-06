//! SQLite-backed canonical metadata catalog for the Mathscape Artifact Pool.
//!
//! Artifact bytes remain in the managed filesystem pool. This crate owns only
//! authoritative metadata and provenance transactions.

mod read;

use std::path::Path;

use mathscape_domain::{
    ArtifactCheck, ArtifactCheckResult, ArtifactLocation, ArtifactUri, CommandRecordStatus,
    ContentHash, ConversionExitStatus, EntityEnvelope, LifecycleStatus, LocationRole,
    LocationStatus, PageIndexBase, SourceArtifact, SourcePayload, SourceRelation, StableId,
    StorageKind,
};
use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

pub use read::{ArtifactCatalogRecord, ArtifactCatalogSnapshot};

const ARTIFACT_POOL_MIGRATION: &str = include_str!("../../../migrations/0001_artifact_pool.sql");

#[derive(Debug, Error)]
pub enum SqliteStoreError {
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("domain value is invalid: {0}")]
    Domain(#[from] mathscape_domain::DomainError),
    #[error("JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{field} cannot be represented by SQLite: {value}")]
    IntegerRange { field: &'static str, value: u64 },
    #[error("stored {field} contains unsupported value {value:?}")]
    InvalidStoredValue { field: &'static str, value: String },
    #[error("unsupported SQLite schema version {actual}; this build supports version {supported}")]
    UnsupportedSchemaVersion { actual: i64, supported: i64 },
    #[error("SQLite schema metadata is inconsistent: {0}")]
    InvalidSchema(&'static str),
    #[error("artifact invariant failed: {0}")]
    ArtifactInvariant(&'static str),
    #[error("expected a ConversionRun relation")]
    ExpectedConversionRun,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredArtifactIdentity {
    stable_id: StableId,
    content_hash: ContentHash,
    byte_size: u64,
}

impl StoredArtifactIdentity {
    pub fn stable_id(&self) -> &StableId {
        &self.stable_id
    }

    pub fn content_hash(&self) -> &ContentHash {
        &self.content_hash
    }

    pub const fn byte_size(&self) -> u64 {
        self.byte_size
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredArtifactLocation {
    location_id: StableId,
    artifact_id: StableId,
    content_hash: ContentHash,
    byte_size: u64,
    storage_kind: StorageKind,
    uri: ArtifactUri,
    status: LocationStatus,
}

impl StoredArtifactLocation {
    pub fn location_id(&self) -> &StableId {
        &self.location_id
    }

    pub fn artifact_id(&self) -> &StableId {
        &self.artifact_id
    }

    pub fn content_hash(&self) -> &ContentHash {
        &self.content_hash
    }

    pub const fn byte_size(&self) -> u64 {
        self.byte_size
    }

    pub const fn storage_kind(&self) -> StorageKind {
        self.storage_kind
    }

    pub fn uri(&self) -> &ArtifactUri {
        &self.uri
    }

    pub const fn status(&self) -> LocationStatus {
        self.status
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistrationOutcome {
    Inserted,
    Existing,
}

pub struct SqliteArtifactStore {
    connection: Connection,
}

impl SqliteArtifactStore {
    pub fn open(path: &Path) -> Result<Self, SqliteStoreError> {
        Self::initialize(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, SqliteStoreError> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(connection: Connection) -> Result<Self, SqliteStoreError> {
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        let version = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => connection.execute_batch(ARTIFACT_POOL_MIGRATION)?,
            1 => validate_schema(&connection)?,
            actual => {
                return Err(SqliteStoreError::UnsupportedSchemaVersion {
                    actual,
                    supported: 1,
                });
            }
        }
        Ok(Self { connection })
    }

    pub fn register_artifact(
        &mut self,
        entity: &EntityEnvelope<SourceArtifact>,
        location: &ArtifactLocation,
        check: &ArtifactCheck,
    ) -> Result<RegistrationOutcome, SqliteStoreError> {
        let header = entity.header();
        let artifact = entity.payload();
        if location.artifact() != header.stable_id() {
            return Err(SqliteStoreError::ArtifactInvariant(
                "location refers to a different artifact",
            ));
        }
        if check.location() != location.location_id() {
            return Err(SqliteStoreError::ArtifactInvariant(
                "check refers to a different location",
            ));
        }
        if check.result() == ArtifactCheckResult::Valid
            && (check.observed_hash() != Some(artifact.content_hash())
                || check.observed_size() != Some(artifact.byte_size()))
        {
            return Err(SqliteStoreError::ArtifactInvariant(
                "valid check does not match artifact content",
            ));
        }

        let byte_size = sqlite_u64(artifact.byte_size(), "artifact.byte_size")?;
        let page_count = artifact.page_count().map(i64::from);
        let observed_size = check
            .observed_size()
            .map(|value| sqlite_u64(value, "artifact_check.observed_size"))
            .transpose()?;
        let permitted_uses = serde_json::to_string(artifact.rights().permitted_uses())?;
        let lifecycle = lifecycle(header.lifecycle())?;
        let transaction = self.connection.transaction()?;

        let existing = find_artifact_by_hash_in(&transaction, artifact.content_hash())?;
        let outcome = if let Some(existing) = existing {
            if existing.stable_id() != header.stable_id()
                || existing.byte_size() != artifact.byte_size()
            {
                return Err(SqliteStoreError::ArtifactInvariant(
                    "content hash is registered under conflicting identity",
                ));
            }
            RegistrationOutcome::Existing
        } else {
            transaction.execute(
                "INSERT INTO artifacts (artifact_id, content_hash, byte_size, current_revision) VALUES (?1, ?2, ?3, ?4)",
                params![
                    header.stable_id().as_str(),
                    artifact.content_hash().as_str(),
                    byte_size,
                    header.revision_id().as_str(),
                ],
            )?;
            RegistrationOutcome::Inserted
        };

        transaction.execute(
            "INSERT OR IGNORE INTO artifact_revisions (
                artifact_id, revision_id, edition_id, media_type, original_filename,
                page_count, rights_status, permitted_uses, rights_note, schema_version,
                lifecycle, created_at, created_by, generation_run
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                header.stable_id().as_str(),
                header.revision_id().as_str(),
                artifact.edition().map(StableId::as_str),
                artifact.media_type().as_str(),
                artifact.original_filename(),
                page_count,
                rights_status(artifact.rights().status()),
                permitted_uses,
                artifact.rights().note(),
                header.schema_version().as_str(),
                lifecycle,
                header.created_at().as_str(),
                header.created_by().as_str(),
                header.generation_run().map(StableId::as_str),
            ],
        )?;
        transaction.execute(
            "UPDATE artifacts SET current_revision = ?2 WHERE artifact_id = ?1",
            params![header.stable_id().as_str(), header.revision_id().as_str()],
        )?;

        let existing_location = transaction
            .query_row(
                "SELECT artifact_id, storage_kind, uri FROM artifact_locations WHERE location_id = ?1",
                [location.location_id().as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((artifact_id, kind, uri)) = existing_location
            && (artifact_id != location.artifact().as_str()
                || kind != storage_kind(location.storage_kind())
                || uri != location.uri().as_str())
        {
            return Err(SqliteStoreError::ArtifactInvariant(
                "location ID is registered with conflicting identity",
            ));
        }

        transaction.execute(
            "INSERT INTO artifact_locations (
                location_id, artifact_id, storage_kind, uri, priority, role,
                last_verified_at, current_status
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(location_id) DO UPDATE SET
                priority = excluded.priority,
                role = excluded.role,
                last_verified_at = excluded.last_verified_at,
                current_status = excluded.current_status",
            params![
                location.location_id().as_str(),
                location.artifact().as_str(),
                storage_kind(location.storage_kind()),
                location.uri().as_str(),
                location.priority(),
                location_role(location.role()),
                location.last_verified_at().map(|value| value.as_str()),
                location_status(location.status()),
            ],
        )?;

        transaction.execute(
            "INSERT OR IGNORE INTO artifact_checks (
                check_id, location_id, checked_at, observed_hash, observed_size,
                result, checker_version
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                check.check_id().as_str(),
                check.location().as_str(),
                check.checked_at().as_str(),
                check.observed_hash().map(ContentHash::as_str),
                observed_size,
                check_result(check.result()),
                check.checker_version(),
            ],
        )?;

        transaction.commit()?;
        Ok(outcome)
    }

    pub fn find_artifact_by_hash(
        &self,
        hash: &ContentHash,
    ) -> Result<Option<StoredArtifactIdentity>, SqliteStoreError> {
        find_artifact_by_hash_in(&self.connection, hash)
    }

    /// Reads one transactionally consistent snapshot for graph projection and
    /// other read-only application views.
    pub fn read_catalog_snapshot(&self) -> Result<ArtifactCatalogSnapshot, SqliteStoreError> {
        read::catalog_snapshot(&self.connection)
    }

    pub fn register_check(
        &mut self,
        check: &ArtifactCheck,
        status: LocationStatus,
    ) -> Result<(), SqliteStoreError> {
        let observed_size = check
            .observed_size()
            .map(|value| sqlite_u64(value, "artifact_check.observed_size"))
            .transpose()?;
        let transaction = self.connection.transaction()?;
        if check.result() == ArtifactCheckResult::Valid {
            let expected = transaction
                .query_row(
                    "SELECT a.content_hash, a.byte_size
                     FROM artifact_locations l
                     JOIN artifacts a ON a.artifact_id = l.artifact_id
                     WHERE l.location_id = ?1",
                    [check.location().as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()?;
            let Some((expected_hash, expected_size)) = expected else {
                return Err(SqliteStoreError::ArtifactInvariant(
                    "check refers to an unknown location",
                ));
            };
            if check.observed_hash().map(ContentHash::as_str) != Some(expected_hash.as_str())
                || observed_size != Some(expected_size)
            {
                return Err(SqliteStoreError::ArtifactInvariant(
                    "valid check does not match artifact content",
                ));
            }
        }
        let changed = transaction.execute(
            "UPDATE artifact_locations SET current_status = ?2, last_verified_at = ?3 WHERE location_id = ?1",
            params![
                check.location().as_str(),
                location_status(status),
                check.checked_at().as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(SqliteStoreError::ArtifactInvariant(
                "check refers to an unknown location",
            ));
        }
        transaction.execute(
            "INSERT INTO artifact_checks (
                check_id, location_id, checked_at, observed_hash, observed_size,
                result, checker_version
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                check.check_id().as_str(),
                check.location().as_str(),
                check.checked_at().as_str(),
                check.observed_hash().map(ContentHash::as_str),
                observed_size,
                check_result(check.result()),
                check.checker_version(),
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn artifact_locations(&self) -> Result<Vec<StoredArtifactLocation>, SqliteStoreError> {
        self.query_artifact_locations(None)
    }

    /// Indexed lookup for bounded content reads; avoids loading the whole pool catalog.
    pub fn artifact_locations_for(
        &self,
        artifact_id: &StableId,
    ) -> Result<Vec<StoredArtifactLocation>, SqliteStoreError> {
        self.query_artifact_locations(Some(artifact_id))
    }

    fn query_artifact_locations(
        &self,
        artifact_id: Option<&StableId>,
    ) -> Result<Vec<StoredArtifactLocation>, SqliteStoreError> {
        let filter = if artifact_id.is_some() {
            "WHERE l.artifact_id = ?1"
        } else {
            "WHERE ?1 IS NULL"
        };
        let mut statement = self.connection.prepare(&format!(
            "SELECT l.location_id, l.artifact_id, a.content_hash, a.byte_size,
                    l.storage_kind, l.uri, l.current_status
             FROM artifact_locations l
             JOIN artifacts a ON a.artifact_id = l.artifact_id
             {filter}
             ORDER BY l.artifact_id, l.priority, l.location_id",
        ))?;
        let rows = statement.query_map([artifact_id.map(StableId::as_str)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        let mut locations = Vec::new();
        for row in rows {
            let (location_id, artifact_id, content_hash, byte_size, kind, uri, status) = row?;
            locations.push(StoredArtifactLocation {
                location_id: StableId::parse(location_id)?,
                artifact_id: StableId::parse(artifact_id)?,
                content_hash: ContentHash::parse(content_hash)?,
                byte_size: rust_u64(byte_size, "artifact.byte_size")?,
                storage_kind: parse_storage_kind(&kind)?,
                uri: ArtifactUri::parse(uri)?,
                status: parse_location_status(&status)?,
            });
        }
        Ok(locations)
    }

    pub fn record_conversion(&mut self, relation: &SourceRelation) -> Result<(), SqliteStoreError> {
        relation.validate()?;
        let SourcePayload::ConversionRun(run) = relation.payload() else {
            return Err(SqliteStoreError::ExpectedConversionRun);
        };
        let header = relation.header();
        let parameters = run.parameters();
        let (page_start, page_end, page_index_base) =
            parameters.page_range().map_or((None, None, None), |range| {
                (
                    Some(i64::from(range.start())),
                    Some(i64::from(range.end())),
                    Some(page_index_base(range.index_base())),
                )
            });
        let additional_options = serde_json::to_string(parameters.additional_options())?;
        let warnings = serde_json::to_string(run.warnings())?;
        let lifecycle = lifecycle(header.lifecycle())?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO conversion_runs (
                run_id, revision_id, schema_version, lifecycle, created_at, created_by,
                generation_run, tool_name, tool_version, command_template,
                command_record_status, page_start, page_end, page_index_base,
                output_format, redo_inline_math, additional_options, environment,
                started_at, finished_at, exit_status, warnings, cost
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                       ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
            params![
                header.stable_id().as_str(),
                header.revision_id().as_str(),
                header.schema_version().as_str(),
                lifecycle,
                header.created_at().as_str(),
                header.created_by().as_str(),
                header.generation_run().map(StableId::as_str),
                run.tool_name(),
                run.tool_version(),
                run.command_template(),
                command_record_status(run.command_record_status()),
                page_start,
                page_end,
                page_index_base,
                parameters.output_format(),
                parameters.redo_inline_math().map(i64::from),
                additional_options,
                run.environment().map(StableId::as_str),
                run.started_at().map(|value| value.as_str()),
                run.finished_at().map(|value| value.as_str()),
                conversion_exit_status(run.exit_status()),
                warnings,
                run.cost().map(StableId::as_str),
            ],
        )?;
        for endpoint in relation.endpoints() {
            transaction.execute(
                "INSERT INTO conversion_endpoints (run_id, revision_id, role, ordinal, artifact_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    header.stable_id().as_str(),
                    header.revision_id().as_str(),
                    endpoint.role().as_str(),
                    endpoint.ordinal().map_or(-1_i64, i64::from),
                    endpoint.entity().as_str(),
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn artifact_count(&self) -> Result<u64, SqliteStoreError> {
        let count = self
            .connection
            .query_row("SELECT COUNT(*) FROM artifacts", [], |row| {
                row.get::<_, i64>(0)
            })?;
        rust_u64(count, "artifact_count")
    }

    pub fn check_count(&self) -> Result<u64, SqliteStoreError> {
        let count =
            self.connection
                .query_row("SELECT COUNT(*) FROM artifact_checks", [], |row| {
                    row.get::<_, i64>(0)
                })?;
        rust_u64(count, "artifact_check_count")
    }

    pub fn conversion_count(&self) -> Result<u64, SqliteStoreError> {
        let count =
            self.connection
                .query_row("SELECT COUNT(*) FROM conversion_runs", [], |row| {
                    row.get::<_, i64>(0)
                })?;
        rust_u64(count, "conversion_count")
    }
}

fn find_artifact_by_hash_in(
    connection: &Connection,
    hash: &ContentHash,
) -> Result<Option<StoredArtifactIdentity>, SqliteStoreError> {
    let row = connection
        .query_row(
            "SELECT artifact_id, content_hash, byte_size FROM artifacts WHERE content_hash = ?1",
            [hash.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    row.map(|(stable_id, content_hash, byte_size)| {
        Ok(StoredArtifactIdentity {
            stable_id: StableId::parse(stable_id)?,
            content_hash: ContentHash::parse(content_hash)?,
            byte_size: rust_u64(byte_size, "artifact.byte_size")?,
        })
    })
    .transpose()
}

fn validate_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    let name = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if name.as_deref() != Some("artifact_pool_v1") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 1 is absent or has an unexpected name",
        ));
    }
    Ok(())
}

fn sqlite_u64(value: u64, field: &'static str) -> Result<i64, SqliteStoreError> {
    i64::try_from(value).map_err(|_| SqliteStoreError::IntegerRange { field, value })
}

fn rust_u64(value: i64, field: &'static str) -> Result<u64, SqliteStoreError> {
    u64::try_from(value).map_err(|_| SqliteStoreError::InvalidStoredValue {
        field,
        value: value.to_string(),
    })
}

fn rights_status(value: mathscape_domain::RightsStatus) -> &'static str {
    use mathscape_domain::RightsStatus::*;
    match value {
        Unknown => "unknown",
        NotAssessed => "not_assessed",
        DerivativeOfUnassessedSource => "derivative_of_unassessed_source",
        Permitted => "permitted",
        Restricted => "restricted",
        Prohibited => "prohibited",
    }
}

fn parse_rights_status(value: &str) -> Result<mathscape_domain::RightsStatus, SqliteStoreError> {
    use mathscape_domain::RightsStatus;
    match value {
        "unknown" => Ok(RightsStatus::Unknown),
        "not_assessed" => Ok(RightsStatus::NotAssessed),
        "derivative_of_unassessed_source" => Ok(RightsStatus::DerivativeOfUnassessedSource),
        "permitted" => Ok(RightsStatus::Permitted),
        "restricted" => Ok(RightsStatus::Restricted),
        "prohibited" => Ok(RightsStatus::Prohibited),
        _ => Err(invalid("artifact_revision.rights_status", value)),
    }
}

fn storage_kind(value: StorageKind) -> &'static str {
    match value {
        StorageKind::Managed => "managed",
        StorageKind::ObjectStore => "object_store",
        StorageKind::External => "external",
        StorageKind::Archive => "archive",
    }
}

fn parse_storage_kind(value: &str) -> Result<StorageKind, SqliteStoreError> {
    match value {
        "managed" => Ok(StorageKind::Managed),
        "object_store" => Ok(StorageKind::ObjectStore),
        "external" => Ok(StorageKind::External),
        "archive" => Ok(StorageKind::Archive),
        _ => Err(invalid("artifact_location.storage_kind", value)),
    }
}

fn location_role(value: LocationRole) -> &'static str {
    match value {
        LocationRole::Primary => "primary",
        LocationRole::Mirror => "mirror",
        LocationRole::Cache => "cache",
        LocationRole::External => "external",
    }
}

fn parse_location_role(value: &str) -> Result<LocationRole, SqliteStoreError> {
    match value {
        "primary" => Ok(LocationRole::Primary),
        "mirror" => Ok(LocationRole::Mirror),
        "cache" => Ok(LocationRole::Cache),
        "external" => Ok(LocationRole::External),
        _ => Err(invalid("artifact_location.role", value)),
    }
}

fn location_status(value: LocationStatus) -> &'static str {
    match value {
        LocationStatus::Available => "available",
        LocationStatus::Missing => "missing",
        LocationStatus::Corrupted => "corrupted",
        LocationStatus::AccessDenied => "access_denied",
        LocationStatus::Unavailable => "unavailable",
    }
}

fn parse_location_status(value: &str) -> Result<LocationStatus, SqliteStoreError> {
    match value {
        "available" => Ok(LocationStatus::Available),
        "missing" => Ok(LocationStatus::Missing),
        "corrupted" => Ok(LocationStatus::Corrupted),
        "access_denied" => Ok(LocationStatus::AccessDenied),
        "unavailable" => Ok(LocationStatus::Unavailable),
        _ => Err(invalid("artifact_location.current_status", value)),
    }
}

fn check_result(value: ArtifactCheckResult) -> &'static str {
    match value {
        ArtifactCheckResult::Valid => "valid",
        ArtifactCheckResult::Missing => "missing",
        ArtifactCheckResult::Corrupted => "corrupted",
        ArtifactCheckResult::AccessDenied => "access_denied",
        ArtifactCheckResult::Unavailable => "unavailable",
    }
}

fn lifecycle(value: LifecycleStatus) -> Result<&'static str, SqliteStoreError> {
    match value {
        LifecycleStatus::Draft => Ok("draft"),
        LifecycleStatus::Candidate => Ok("candidate"),
        LifecycleStatus::Active => Ok("active"),
        LifecycleStatus::Superseded => Ok("superseded"),
        LifecycleStatus::Rejected => Ok("rejected"),
        LifecycleStatus::Deprecated => Ok("deprecated"),
        LifecycleStatus::ParseError => Ok("parse_error"),
        LifecycleStatus::SemanticError => Ok("semantic_error"),
        _ => Err(SqliteStoreError::ArtifactInvariant(
            "unsupported lifecycle status",
        )),
    }
}

fn parse_lifecycle(value: &str) -> Result<LifecycleStatus, SqliteStoreError> {
    match value {
        "draft" => Ok(LifecycleStatus::Draft),
        "candidate" => Ok(LifecycleStatus::Candidate),
        "active" => Ok(LifecycleStatus::Active),
        "superseded" => Ok(LifecycleStatus::Superseded),
        "rejected" => Ok(LifecycleStatus::Rejected),
        "deprecated" => Ok(LifecycleStatus::Deprecated),
        "parse_error" => Ok(LifecycleStatus::ParseError),
        "semantic_error" => Ok(LifecycleStatus::SemanticError),
        _ => Err(invalid("entity.lifecycle", value)),
    }
}

fn page_index_base(value: PageIndexBase) -> &'static str {
    match value {
        PageIndexBase::Zero => "zero",
        PageIndexBase::One => "one",
    }
}

fn parse_page_index_base(value: &str) -> Result<PageIndexBase, SqliteStoreError> {
    match value {
        "zero" => Ok(PageIndexBase::Zero),
        "one" => Ok(PageIndexBase::One),
        _ => Err(invalid("conversion.page_index_base", value)),
    }
}

fn command_record_status(value: CommandRecordStatus) -> &'static str {
    match value {
        CommandRecordStatus::Recorded => "recorded",
        CommandRecordStatus::ReconstructedFromArtifacts => "reconstructed_from_artifacts",
        CommandRecordStatus::Unavailable => "unavailable",
    }
}

fn parse_command_record_status(value: &str) -> Result<CommandRecordStatus, SqliteStoreError> {
    match value {
        "recorded" => Ok(CommandRecordStatus::Recorded),
        "reconstructed_from_artifacts" => Ok(CommandRecordStatus::ReconstructedFromArtifacts),
        "unavailable" => Ok(CommandRecordStatus::Unavailable),
        _ => Err(invalid("conversion.command_record_status", value)),
    }
}

fn conversion_exit_status(value: ConversionExitStatus) -> &'static str {
    match value {
        ConversionExitStatus::Succeeded => "succeeded",
        ConversionExitStatus::Failed => "failed",
        ConversionExitStatus::Partial => "partial",
        ConversionExitStatus::TimedOut => "timed_out",
        ConversionExitStatus::Cancelled => "cancelled",
    }
}

fn parse_conversion_exit_status(value: &str) -> Result<ConversionExitStatus, SqliteStoreError> {
    match value {
        "succeeded" => Ok(ConversionExitStatus::Succeeded),
        "failed" => Ok(ConversionExitStatus::Failed),
        "partial" => Ok(ConversionExitStatus::Partial),
        "timed_out" => Ok(ConversionExitStatus::TimedOut),
        "cancelled" => Ok(ConversionExitStatus::Cancelled),
        _ => Err(invalid("conversion.exit_status", value)),
    }
}

fn invalid(field: &'static str, value: &str) -> SqliteStoreError {
    SqliteStoreError::InvalidStoredValue {
        field,
        value: value.to_owned(),
    }
}
