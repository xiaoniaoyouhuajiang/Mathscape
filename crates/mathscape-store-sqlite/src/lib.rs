//! SQLite-backed canonical metadata catalog for the Mathscape Artifact Pool.
//!
//! Artifact bytes remain in the managed filesystem pool. This crate owns only
//! authoritative metadata and provenance transactions.

mod canonical_ensure;
mod canonical_markdown;
mod discovery;
mod ink;
mod ink_evidence;
mod ink_graph;
mod ink_lan;
mod ink_link;
mod ink_note;
mod ink_recovery;
mod ink_snapshot_label;
mod map_edit;
mod markdown_patch;
mod proposals;
mod read;
mod registration;
mod run_disposition;
mod search;
mod source_map;

use std::path::Path;

use mathscape_domain::{
    ArtifactCheck, ArtifactCheckResult, ArtifactLocation, ArtifactUri, CommandRecordStatus,
    ContentHash, ConversionExitStatus, EntityEnvelope, LifecycleStatus, LocationRole,
    LocationStatus, MapPatch, PageIndexBase, SourceArtifact, SourcePayload, SourceRelation,
    StableId, StorageKind, Timestamp,
};
use mathscape_llm::LlmProfile;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

pub use canonical_ensure::{
    CanonicalEnsureClaim, CanonicalEnsureClaimOutcome, CanonicalEnsureLease, CanonicalEnsureLimits,
    CanonicalEnsureOutcome, CanonicalEnsureReceipt, unix_millis_now,
};
pub use canonical_markdown::project_canonical_markdown;
pub use discovery::{
    AcceptedRegistrationPlanRecord, DiscoveryAttemptCheckpoint, DiscoveryCheckpointStatus,
    DiscoveryDiagnosticCheckpoint, DiscoveryFactCheckpointRecord, DiscoveryRunCheckpoint,
    DiscoveryRunDispositionReceipt, DiscoveryUnitCheckpointRecord, DiscoveryValidationCheckpoint,
    PendingDiscoveryTask, StoredAcceptanceAuthority, StoredDiscoveryRun, StoredDiscoveryRunSummary,
};
pub use ink::{
    INK_STORAGE_VERSION, InkOperationReceipt, InkSnapshotReservation, StoredInkDocument,
    StoredInkDocumentSummary, StoredInkSession, StoredInkSnapshot, StoredInkSnapshotSummary,
};
pub use ink_evidence::InkEvidenceReceipt;
pub use ink_graph::{InkGraphNote, InkGraphRows, InkGraphSnapshot};
pub use ink_lan::{InkLanSettings, InkMirrorDelta, InkPairingOffer, InkRemoteLease, InkWriterView};
pub use ink_link::{
    INK_LINK_LABEL_MAX_CHARS, InkContextCreateReceipt, InkContextCreateRequest, InkLinkOverlay,
    InkLinkReceipt, InkLinkRequest, InkLinkRole, InkLinkTarget, InkLinkTargetStatus, InkLinkedNote,
    InkMarkdownTarget, InkNoteLink, InkOverlayCount, InkOverlayLink, InkSetPrimaryRequest,
    InkUnlinkRequest, format_link_title, markdown_excerpt, markdown_short_label,
    normalize_markdown_text, source_object_short_label, source_unit_kind_name,
};
pub use ink_note::{
    INK_NOTE_TITLE_MAX_CHARS, InkCleanBlankReceipt, InkCleanBlankRequest, InkCleanSkip,
    InkCreateReceipt, InkLifecycle, InkLifecycleRequest, InkNoteReceipt, InkNoteState,
    InkRenameRequest, InkTitleMode, InkTrashImpact, format_ink_auto_title, ink_auto_title,
    parse_ink_timestamp_ms,
};
pub use ink_recovery::{
    INK_UNSAVED_OPERATIONS_FORMAT, InkUnsavedOperation, InkUnsavedOperationsExport,
};
pub use ink_snapshot_label::{
    INK_SNAPSHOT_LABEL_MAX_CHARS, InkSnapshotLabelReceipt, InkSnapshotLabelRequest,
    normalize_snapshot_label,
};
pub use markdown_patch::{StoredMarkdownPatchSet, StoredPatchApplication};
pub use proposals::ProposedChangeRecord;
pub use proposals::{
    ChangeSetAcceptance, ChangeSetDecision, ChangeSetDependency, ChangeSetSelectionRef,
    DiscardOutcome, PendingChangeSet,
};
pub use read::{ArtifactCatalogRecord, ArtifactCatalogSnapshot};
pub use search::{
    SearchArtifactRow, SearchCorpus, SearchNoteRow, SearchSemanticEntityRow, SearchSnapshotRow,
    SearchSourceUnitRow, search_semantic_entity_row_from_payload,
};
pub use source_map::{
    MapObjectOutcome, SOURCE_REGISTRATION_RECEIPT_VERSION, SourceMapSnapshot,
    SourceRegistrationReceipt, StoredMapEndpoint, StoredMapObject,
};

const ARTIFACT_POOL_MIGRATION: &str = include_str!("../../../migrations/0001_artifact_pool.sql");
const LLM_PROFILES_MIGRATION: &str = include_str!("../../../migrations/0002_llm_profiles.sql");
const WORKSPACE_SETTINGS_MIGRATION: &str =
    include_str!("../../../migrations/0003_workspace_settings.sql");
const DISCOVERY_RUNS_MIGRATION: &str = include_str!("../../../migrations/0004_discovery_runs.sql");
const SOURCE_MAP_MIGRATION: &str = include_str!("../../../migrations/0005_source_map.sql");
const DISCOVERY_RUNS_V2_MIGRATION: &str =
    include_str!("../../../migrations/0006_discovery_runs_v2.sql");
const MARKDOWN_PATCHES_MIGRATION: &str =
    include_str!("../../../migrations/0008_markdown_patches.sql");
const DISCOVERY_PATCH_REVIEW_MIGRATION: &str =
    include_str!("../../../migrations/0009_discovery_patch_review.sql");
const DISCOVERY_RUN_RECOVERY_MIGRATION: &str =
    include_str!("../../../migrations/0010_discovery_run_recovery.sql");
const INK_MIGRATION: &str = include_str!("../../../migrations/0013_ink_p1.sql");
const INK_SNAPSHOT_MIGRATION: &str = include_str!("../../../migrations/0014_ink_snapshots_p2.sql");
const CANONICAL_MARKDOWN_MIGRATION: &str =
    include_str!("../../../migrations/0021_canonical_markdown.sql");
const CANONICAL_MARKDOWN_ENSURE_MIGRATION: &str =
    include_str!("../../../migrations/0022_canonical_markdown_ensure.sql");
const PROPOSED_CHANGE_SETS_MIGRATION: &str =
    include_str!("../../../migrations/0023_proposed_change_sets.sql");
const PHASE2_PR_B_MIGRATION: &str = include_str!("../../../migrations/0024_phase2_pr_b.sql");
const SUPPORTED_SCHEMA_VERSION: i64 = 24;

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
    #[error("SQLite schema is missing required {object_kind} {name}")]
    MissingSchemaObject {
        object_kind: &'static str,
        name: &'static str,
    },
    #[error("artifact invariant failed: {0}")]
    ArtifactInvariant(&'static str),
    #[error("expected a ConversionRun relation")]
    ExpectedConversionRun,
    #[error("LLM profile is invalid: {0}")]
    InvalidLlmProfile(String),
    #[error("discovery checkpoint invariant failed: {0}")]
    DiscoveryInvariant(String),
    #[error("source registration invariant failed: {0}")]
    SourceRegistrationInvariant(String),
    /// A Phase2 change set decision was refused. The message starts with a
    /// stable code (`base_commit_conflict`, `plan_conflict`,
    /// `dependency_not_accepted`, `proposal_stale`, ...).
    #[error("{0}")]
    ChangeSetConflict(String),
    #[error("ink document was not found: {0}")]
    InkDocumentNotFound(String),
    #[error("ink session was not found: {0}")]
    InkSessionNotFound(String),
    #[error("ink invariant failed: {0}")]
    InkInvariant(String),
    #[error("ink document validation failed: {0}")]
    InkValidation(String),
    #[error("ink request has already been used with different content: {0}")]
    InkRequestConflict(String),
    #[error("ink head conflict: expected {expected}, actual {actual}")]
    InkHeadConflict { expected: u64, actual: u64 },
    #[error("ink sequence conflict: expected {expected}, actual {actual}")]
    InkSequenceConflict { expected: u64, actual: u64 },
    #[error(
        "ink writer conflict for document {document_id}: session {session_id} by {writer_id} is active"
    )]
    InkWriterConflict {
        document_id: String,
        session_id: String,
        writer_id: String,
    },
    #[error(
        "ink writer epoch conflict for document {document_id}: expected {expected}, actual {actual}"
    )]
    InkWriterEpochConflict {
        document_id: String,
        expected: u64,
        actual: u64,
    },
    /// The document is in the trash; it accepts no new writes, sessions or pairings.
    #[error("ink document_trashed: {0}")]
    InkDocumentTrashed(String),
    #[error(
        "ink title_conflict for document {document_id}: current title revision {current_revision}"
    )]
    InkTitleConflict {
        document_id: String,
        current_title: String,
        current_title_mode: String,
        current_revision: u64,
    },
    #[error(
        "ink lifecycle_conflict for document {document_id}: {lifecycle} at revision {current_revision}"
    )]
    InkLifecycleConflict {
        document_id: String,
        lifecycle: String,
        current_revision: u64,
    },
    #[error("ink title_invalid: {0}")]
    InkTitleInvalid(String),
    /// The link target does not exist, is not linkable or no longer matches.
    #[error("ink link_target_invalid: {0}")]
    InkLinkTargetInvalid(String),
    /// The source object or note has been retired or trashed.
    #[error("ink link_target_retired: {0}")]
    InkLinkTargetRetired(String),
    #[error("ink link_not_found: {0}")]
    InkLinkNotFound(String),
    #[error("ink snapshot_not_found: {0}")]
    InkSnapshotNotFound(String),
    #[error(
        "ink snapshot_label_conflict for snapshot {snapshot_id}: current label revision {current_revision}"
    )]
    InkSnapshotLabelConflict {
        snapshot_id: String,
        current_label: Option<String>,
        current_revision: u64,
    },
    #[error("ink snapshot_label_invalid: {0}")]
    InkSnapshotLabelInvalid(String),
    #[error(
        "canonical markdown identity {source_pdf_artifact_id} / {parser_profile} / {parser_policy_version} is already bound to {existing_canonical_id}"
    )]
    CanonicalMarkdownConflict {
        source_pdf_artifact_id: String,
        parser_profile: String,
        parser_policy_version: String,
        existing_canonical_id: String,
    },
    #[error("canonical markdown invariant failed: {0}")]
    CanonicalMarkdownInvariant(&'static str),
}

impl SqliteStoreError {
    /// Stable machine-readable code for Ink errors, used by the desktop
    /// commands and the LAN service instead of matching on display text.
    pub fn ink_code(&self) -> &'static str {
        match self {
            Self::InkDocumentTrashed(_) => "document_trashed",
            Self::InkTitleConflict { .. } => "title_conflict",
            Self::InkLifecycleConflict { .. } => "lifecycle_conflict",
            Self::InkTitleInvalid(_) => "title_invalid",
            Self::InkLinkTargetInvalid(_) => "link_target_invalid",
            Self::InkLinkTargetRetired(_) => "link_target_retired",
            Self::InkLinkNotFound(_) => "link_not_found",
            Self::InkSnapshotNotFound(_) => "snapshot_not_found",
            Self::InkSnapshotLabelConflict { .. } => "snapshot_label_conflict",
            Self::InkSnapshotLabelInvalid(_) => "snapshot_label_invalid",
            Self::InkRequestConflict(_) => "idempotency_conflict",
            Self::InkHeadConflict { .. } => "head_conflict",
            Self::InkSequenceConflict { .. } => "sequence_conflict",
            Self::InkWriterConflict { .. } => "writer_conflict",
            Self::InkWriterEpochConflict { .. } => "writer_epoch_conflict",
            Self::InkDocumentNotFound(_) => "document_not_found",
            Self::InkSessionNotFound(_) => "session_not_found",
            Self::InkValidation(_) => "validation_failed",
            Self::InkInvariant(message) => {
                for code in [
                    "writer_revoked",
                    "credential_expired",
                    "authentication_failed",
                    "writer_epoch_conflict",
                    "takeover_required",
                    "pairing_already_used",
                    "pairing_expired",
                    "pairing_invalid",
                    "document_byte_limit",
                ] {
                    if message.contains(code) {
                        return code;
                    }
                }
                "invariant_failed"
            }
            _ => "storage_failed",
        }
    }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredLlmProfile {
    profile: LlmProfile,
    is_default: bool,
}

/// Transactionally consistent input for cross-view landscape projection.
#[derive(Clone, Debug, PartialEq)]
pub struct LandscapeSnapshot {
    artifacts: ArtifactCatalogSnapshot,
    sources: SourceMapSnapshot,
    /// N3d: snapshot Artifact id → snapshot name, for map labels.
    ink_snapshot_labels: std::collections::BTreeMap<String, String>,
    /// Persisted canonical markdown views. Empty until a registry sync.
    canonical_markdown: Vec<mathscape_domain::CanonicalMarkdownView>,
    /// Live canonical markdown ensures (「解析中」).
    canonical_ensures: Vec<CanonicalEnsureLease>,
}

impl LandscapeSnapshot {
    /// Same snapshot with a different source map (the investigation overlay).
    pub fn with_sources(mut self, sources: SourceMapSnapshot) -> Self {
        self.sources = sources;
        self
    }

    pub fn canonical_ensures(&self) -> &[CanonicalEnsureLease] {
        &self.canonical_ensures
    }

    pub fn artifacts(&self) -> &ArtifactCatalogSnapshot {
        &self.artifacts
    }

    pub fn sources(&self) -> &SourceMapSnapshot {
        &self.sources
    }

    pub fn ink_snapshot_labels(&self) -> &std::collections::BTreeMap<String, String> {
        &self.ink_snapshot_labels
    }

    pub fn canonical_markdown(&self) -> &[mathscape_domain::CanonicalMarkdownView] {
        &self.canonical_markdown
    }
}

impl StoredLlmProfile {
    pub fn profile(&self) -> &LlmProfile {
        &self.profile
    }

    pub const fn is_default(&self) -> bool {
        self.is_default
    }
}

pub struct SqliteArtifactStore {
    connection: Connection,
}

impl SqliteArtifactStore {
    /// Connect to an existing, prepared catalog. Never creates, migrates or repairs it.
    pub fn open(path: &Path) -> Result<Self, SqliteStoreError> {
        let connection = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Self::configure_connection(&connection)?;
        let actual: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if actual != SUPPORTED_SCHEMA_VERSION {
            return Err(SqliteStoreError::UnsupportedSchemaVersion {
                actual,
                supported: SUPPORTED_SCHEMA_VERSION,
            });
        }
        validate_markdown_patch_schema(&connection)?;
        validate_discovery_run_recovery_schema(&connection)?;
        ink::validate_schema(&connection)?;
        ink_evidence::validate_schema(&connection)?;
        ink_lan::validate_schema(&connection)?;
        ink_note::validate_schema(&connection)?;
        canonical_ensure::validate_schema(&connection)?;
        validate_registration_schema(&connection)?;
        validate_proposed_change_schema(&connection)?;
        Ok(Self { connection })
    }

    /// Prepare a workspace catalog before accepting business requests.
    /// May create, migrate and reconcile historical schemas; fully validates the result.
    pub fn prepare(path: &Path) -> Result<Self, SqliteStoreError> {
        Self::initialize(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, SqliteStoreError> {
        Self::initialize(Connection::open_in_memory()?)
    }

    pub fn create_ink_document(
        &mut self,
        document: &mathscape_domain::InkLabDocument,
        session_id: &str,
        writer_id: &str,
        device: &serde_json::Value,
    ) -> Result<StoredInkSession, SqliteStoreError> {
        ink::create_document(
            &mut self.connection,
            document,
            session_id,
            writer_id,
            device,
        )
    }

    pub fn start_ink_session(
        &mut self,
        document_id: &str,
        session_id: &str,
        writer_id: &str,
        device: &serde_json::Value,
    ) -> Result<StoredInkSession, SqliteStoreError> {
        ink::start_session(
            &mut self.connection,
            document_id,
            session_id,
            writer_id,
            device,
        )
    }

    pub fn close_ink_session(&mut self, session_id: &str) -> Result<(), SqliteStoreError> {
        ink::close_session(&mut self.connection, session_id)
    }

    pub fn append_ink_operation(
        &mut self,
        request_id: &str,
        document_id: &str,
        session_id: &str,
        writer_epoch: u64,
        expected_head: u64,
        metadata: &mathscape_domain::InkLabDocumentMetadata,
        operation: &mathscape_domain::InkLabOperation,
    ) -> Result<InkOperationReceipt, SqliteStoreError> {
        ink::append_operation(
            &mut self.connection,
            request_id,
            document_id,
            session_id,
            writer_epoch,
            expected_head,
            metadata,
            operation,
        )
    }

    pub fn open_ink_document(
        &self,
        document_id: &str,
    ) -> Result<Option<StoredInkDocument>, SqliteStoreError> {
        ink::open_document(&self.connection, document_id)
    }

    /// Active documents, most recently edited first. Trashed documents are
    /// listed separately by [`Self::list_trashed_ink_documents`].
    pub fn list_ink_documents(&self) -> Result<Vec<StoredInkDocumentSummary>, SqliteStoreError> {
        ink::list_documents(&self.connection, InkLifecycle::Active)
    }

    pub fn list_trashed_ink_documents(
        &self,
    ) -> Result<Vec<StoredInkDocumentSummary>, SqliteStoreError> {
        ink::list_documents(&self.connection, InkLifecycle::Trashed)
    }

    /// Creates a document with note metadata; `request_id` makes it idempotent.
    pub fn create_ink_note(
        &mut self,
        document: &mathscape_domain::InkLabDocument,
        session_id: &str,
        writer_id: &str,
        device: &serde_json::Value,
        title_mode: InkTitleMode,
        request_id: Option<&str>,
    ) -> Result<InkCreateReceipt, SqliteStoreError> {
        ink::create_note(
            &mut self.connection,
            document,
            session_id,
            writer_id,
            device,
            title_mode,
            request_id,
        )
    }

    pub fn ink_session(
        &self,
        session_id: &str,
    ) -> Result<Option<StoredInkSession>, SqliteStoreError> {
        ink::read_session(&self.connection, session_id)
    }

    pub fn list_ink_sessions(
        &self,
        document_id: &str,
    ) -> Result<Vec<StoredInkSession>, SqliteStoreError> {
        ink::list_sessions(&self.connection, document_id)
    }

    pub fn record_ink_snapshot(
        &mut self,
        request_id: &str,
        document_id: &str,
        through_seq: u64,
        snapshot_id: &str,
        artifact_id: &str,
        content_hash: &str,
        byte_size: u64,
        manifest_json: &str,
        created_at: &str,
    ) -> Result<StoredInkSnapshot, SqliteStoreError> {
        ink::record_snapshot(
            &mut self.connection,
            request_id,
            document_id,
            through_seq,
            snapshot_id,
            artifact_id,
            content_hash,
            byte_size,
            manifest_json,
            created_at,
        )
    }

    pub fn reserve_ink_snapshot(
        &mut self,
        request_id: &str,
        document_id: &str,
        expected_head: u64,
        created_at: &str,
    ) -> Result<InkSnapshotReservation, SqliteStoreError> {
        ink::reserve_snapshot(
            &mut self.connection,
            request_id,
            document_id,
            expected_head,
            created_at,
        )
    }

    pub fn ink_snapshot_for_artifact(
        &self,
        artifact_id: &str,
    ) -> Result<Option<StoredInkSnapshot>, SqliteStoreError> {
        ink::snapshot_for_artifact(&self.connection, artifact_id)
    }

    pub fn ink_snapshot_for_request(
        &self,
        request_id: &str,
    ) -> Result<Option<StoredInkSnapshot>, SqliteStoreError> {
        ink::snapshot_for_request(&self.connection, request_id)
    }

    pub fn open_ink_snapshot(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<StoredInkSnapshot>, SqliteStoreError> {
        ink::read_snapshot(&self.connection, snapshot_id)
    }

    pub fn list_ink_snapshots(
        &self,
        document_id: &str,
    ) -> Result<Vec<StoredInkSnapshotSummary>, SqliteStoreError> {
        ink::list_snapshots(&self.connection, document_id)
    }

    fn configure_connection(connection: &Connection) -> Result<(), SqliteStoreError> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(())
    }

    fn initialize(connection: Connection) -> Result<Self, SqliteStoreError> {
        Self::configure_connection(&connection)?;
        let journal_mode: String =
            connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            connection.pragma_update(None, "journal_mode", "WAL")?;
        }
        let version = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => {
                connection.execute_batch(ARTIFACT_POOL_MIGRATION)?;
                connection.execute_batch(LLM_PROFILES_MIGRATION)?;
                connection.execute_batch(WORKSPACE_SETTINGS_MIGRATION)?;
                connection.execute_batch(DISCOVERY_RUNS_MIGRATION)?;
                connection.execute_batch(SOURCE_MAP_MIGRATION)?;
                validate_schema(&connection)?;
            }
            1 => {
                validate_artifact_schema(&connection)?;
                connection.execute_batch(LLM_PROFILES_MIGRATION)?;
                connection.execute_batch(WORKSPACE_SETTINGS_MIGRATION)?;
                connection.execute_batch(DISCOVERY_RUNS_MIGRATION)?;
                connection.execute_batch(SOURCE_MAP_MIGRATION)?;
                validate_schema(&connection)?;
            }
            2 => {
                validate_llm_profiles_schema(&connection)?;
                connection.execute_batch(WORKSPACE_SETTINGS_MIGRATION)?;
                connection.execute_batch(DISCOVERY_RUNS_MIGRATION)?;
                connection.execute_batch(SOURCE_MAP_MIGRATION)?;
                validate_schema(&connection)?;
            }
            3 => {
                validate_workspace_settings_schema(&connection)?;
                connection.execute_batch(DISCOVERY_RUNS_MIGRATION)?;
                connection.execute_batch(SOURCE_MAP_MIGRATION)?;
                validate_schema(&connection)?;
            }
            4 => {
                validate_discovery_schema(&connection)?;
                connection.execute_batch(SOURCE_MAP_MIGRATION)?;
                validate_schema(&connection)?;
            }
            5 => {
                validate_schema(&connection)?;
            }
            6..=24 => {
                validate_schema(&connection)?;
            }
            actual => {
                return Err(SqliteStoreError::UnsupportedSchemaVersion {
                    actual,
                    supported: SUPPORTED_SCHEMA_VERSION,
                });
            }
        }
        if version < 7 {
            reconcile_schema_v6(&connection)?;
            connection.execute_batch(include_str!("../../../migrations/0007_map_edits.sql"))?;
        } else {
            // Re-run the historical repair checks for already upgraded
            // catalogs, then restore the current schema marker.
            reconcile_schema_v6(&connection)?;
            if version < 8 {
                connection.execute_batch(MARKDOWN_PATCHES_MIGRATION)?;
            }
            connection.pragma_update(None, "user_version", version.max(8))?;
        }
        if version < 7 {
            connection.execute_batch(MARKDOWN_PATCHES_MIGRATION)?;
        }
        if version < 9 {
            migrate_discovery_patch_review(&connection)?;
        }
        if version < 10 {
            migrate_discovery_run_recovery(&connection)?;
        }
        if version < 11 {
            connection.execute_batch(include_str!(
                "../../../migrations/0011_registration_reversal.sql"
            ))?;
        }
        if version < 12 {
            connection.execute_batch(include_str!(
                "../../../migrations/0012_discovery_review_restore.sql"
            ))?;
        }
        if version < 13 {
            let ink_migration_exists: Option<String> = connection
                .query_row(
                    "SELECT name FROM schema_migrations WHERE version = 13",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if ink_migration_exists.is_none() {
                connection.execute_batch(INK_MIGRATION)?;
            } else {
                // A manually restored catalog may have an older user_version
                // marker while retaining the already applied Ink migration.
                connection.pragma_update(None, "user_version", 13)?;
            }
        }
        if version < 14 {
            let snapshot_migration_exists: Option<String> = connection
                .query_row(
                    "SELECT name FROM schema_migrations WHERE version = 14",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if snapshot_migration_exists.is_none() {
                connection.execute_batch(INK_SNAPSHOT_MIGRATION)?;
            } else {
                connection.pragma_update(None, "user_version", 14)?;
            }
        }
        if version < 15 {
            let applied: Option<String> = connection
                .query_row(
                    "SELECT name FROM schema_migrations WHERE version = 15",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if applied.is_none() {
                connection
                    .execute_batch(include_str!("../../../migrations/0015_ink_evidence_p2.sql"))?;
            } else {
                connection.pragma_update(None, "user_version", 15)?;
            }
        }
        if version < 16 {
            let applied: Option<String> = connection
                .query_row(
                    "SELECT name FROM schema_migrations WHERE version=16",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if applied.is_none() {
                connection
                    .execute_batch(include_str!("../../../migrations/0016_ink_lan_p3.sql"))?;
            } else {
                connection.pragma_update(None, "user_version", 16)?;
            }
        }
        if version < 17 {
            let applied: Option<String> = connection
                .query_row(
                    "SELECT name FROM schema_migrations WHERE version=17",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if applied.is_none() {
                ink_note::migrate_v17(&connection)?;
            } else {
                connection.pragma_update(None, "user_version", 17)?;
            }
        }
        if version < 18 {
            let applied: Option<String> = connection
                .query_row(
                    "SELECT name FROM schema_migrations WHERE version=18",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if applied.is_none() {
                ink_link::migrate_v18(&connection)?;
            } else {
                connection.pragma_update(None, "user_version", 18)?;
            }
        }
        if version < 19 {
            let applied: Option<String> = connection
                .query_row(
                    "SELECT name FROM schema_migrations WHERE version=19",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if applied.is_none() {
                ink_lan::migrate_v19(&connection)?;
            } else {
                connection.pragma_update(None, "user_version", 19)?;
            }
        }
        if version < 20 {
            let applied: Option<String> = connection
                .query_row(
                    "SELECT name FROM schema_migrations WHERE version=20",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if applied.is_none() {
                ink_snapshot_label::migrate_v20(&connection)?;
            } else {
                connection.pragma_update(None, "user_version", 20)?;
            }
        }
        if version < 21 {
            let applied: Option<String> = connection
                .query_row(
                    "SELECT name FROM schema_migrations WHERE version = 21",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if applied.is_none() {
                connection.execute_batch(CANONICAL_MARKDOWN_MIGRATION)?;
            } else {
                connection.pragma_update(None, "user_version", 21)?;
            }
        }
        if version < 22 {
            let applied: Option<String> = connection
                .query_row(
                    "SELECT name FROM schema_migrations WHERE version = 22",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if applied.is_none() {
                connection.execute_batch(CANONICAL_MARKDOWN_ENSURE_MIGRATION)?;
            } else {
                connection.pragma_update(None, "user_version", 22)?;
            }
        }
        if version < 23 {
            let applied: Option<String> = connection
                .query_row(
                    "SELECT name FROM schema_migrations WHERE version = 23",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if applied.is_none() {
                connection.execute_batch(PROPOSED_CHANGE_SETS_MIGRATION)?;
            } else {
                connection.pragma_update(None, "user_version", 23)?;
            }
        }
        if version < 24 {
            let applied: Option<String> = connection
                .query_row(
                    "SELECT name FROM schema_migrations WHERE version = 24",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if applied.is_none() {
                connection.execute_batch(PHASE2_PR_B_MIGRATION)?;
            } else {
                connection.pragma_update(None, "user_version", 24)?;
            }
        }
        validate_registration_schema(&connection)?;
        connection.prepare("SELECT request_id, request_json, plan_fingerprint, commit_id, receipt_json FROM map_change_requests LIMIT 0")?;
        connection.prepare("SELECT commit_id, object_id, previous_revision, current_revision, action, reason FROM map_edit_operations LIMIT 0")?;
        connection.prepare(
            "SELECT patch_set_id, patch_fingerprint, status FROM markdown_patch_sets LIMIT 0",
        )?;
        validate_markdown_patch_schema(&connection)?;
        validate_discovery_run_recovery_schema(&connection)?;
        ink::validate_schema(&connection)?;
        ink_evidence::validate_schema(&connection)?;
        ink_lan::validate_schema(&connection)?;
        ink_note::validate_schema(&connection)?;
        ink_link::validate_schema(&connection)?;
        ink_snapshot_label::validate_schema(&connection)?;
        canonical_ensure::validate_schema(&connection)?;
        validate_proposed_change_schema(&connection)?;
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

    /// Atomically replaces the replay/audit checkpoint for one immutable
    /// discovery input. This does not accept or register any map entity.
    pub fn save_discovery_checkpoint(
        &mut self,
        checkpoint: &DiscoveryRunCheckpoint,
    ) -> Result<(), SqliteStoreError> {
        discovery::save_discovery_checkpoint(&mut self.connection, checkpoint)
    }

    /// Atomically persists a paused discovery checkpoint and its pending
    /// Markdown patch set. A run must never become `awaiting_patch_review`
    /// without a recoverable patch set.
    pub fn save_discovery_checkpoint_with_patch_set(
        &mut self,
        checkpoint: &DiscoveryRunCheckpoint,
        patch_set_id: &StableId,
        patch_set: &mathscape_domain::MarkdownPatchSet,
        status: mathscape_domain::MarkdownPatchStatus,
        now: &Timestamp,
    ) -> Result<(), SqliteStoreError> {
        let transaction = self.connection.transaction()?;
        discovery::save_discovery_checkpoint_in(&transaction, checkpoint)?;
        markdown_patch::save_patch_set_in(
            &transaction,
            patch_set_id,
            patch_set,
            status,
            None,
            None,
            now,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn discovery_checkpoint(
        &self,
        run_id: &StableId,
    ) -> Result<Option<StoredDiscoveryRun>, SqliteStoreError> {
        discovery::read_discovery_checkpoint(&self.connection, run_id)
    }

    /// Most recently updated run over `artifact_id` that is paused for
    /// Markdown patch review, if any.
    pub fn latest_discovery_run_awaiting_patch_review(
        &self,
        artifact_id: &StableId,
    ) -> Result<Option<StableId>, SqliteStoreError> {
        let run_id = self
            .connection
            .query_row(
                "SELECT run_id FROM discovery_runs
                 WHERE artifact_id = ?1 AND status = 'awaiting_patch_review'
                 ORDER BY updated_at DESC, run_id DESC LIMIT 1",
                params![artifact_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        run_id.map(StableId::parse).transpose().map_err(Into::into)
    }

    pub fn list_discovery_run_summaries(
        &self,
        artifact_id: Option<&StableId>,
    ) -> Result<Vec<StoredDiscoveryRunSummary>, SqliteStoreError> {
        let tx = self.connection.unchecked_transaction()?;
        let summaries = discovery::list_discovery_run_summaries(&tx, artifact_id)?;
        tx.commit()?;
        Ok(summaries)
    }

    pub fn discard_discovery_run(
        &mut self,
        run_id: &StableId,
        request_id: &StableId,
        expected_result_fingerprint: &ContentHash,
        actor: &mathscape_domain::ActorRef,
        reason: &str,
        decided_at: &Timestamp,
    ) -> Result<DiscoveryRunDispositionReceipt, SqliteStoreError> {
        discovery::discard_discovery_run(
            &mut self.connection,
            run_id,
            request_id,
            expected_result_fingerprint,
            actor,
            reason,
            decided_at,
        )
    }

    pub fn discovery_run_is_discarded(&self, run_id: &StableId) -> Result<bool, SqliteStoreError> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM discovery_run_dispositions WHERE run_id = ?1 AND disposition='discarded')",
                params![run_id.as_str()],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn discovery_run_count(&self) -> Result<u64, SqliteStoreError> {
        let count =
            self.connection
                .query_row("SELECT COUNT(*) FROM discovery_runs", [], |row| {
                    row.get::<_, i64>(0)
                })?;
        rust_u64(count, "discovery_run_count")
    }

    pub fn save_registration_plan(
        &mut self,
        record: &AcceptedRegistrationPlanRecord,
    ) -> Result<(), SqliteStoreError> {
        discovery::save_registration_plan(&mut self.connection, record)
    }

    pub fn registration_plan(
        &self,
        decision_id: &StableId,
    ) -> Result<Option<AcceptedRegistrationPlanRecord>, SqliteStoreError> {
        discovery::read_registration_plan(&self.connection, decision_id)
    }

    /// Atomically creates or reuses every Source object in one accepted patch.
    /// A receipt is committed in the same transaction; retries return it.
    pub fn commit_source_registration(
        &mut self,
        decision_id: &StableId,
        plan_fingerprint: &ContentHash,
        patch: &MapPatch<SourcePayload>,
        committed_at: Timestamp,
    ) -> Result<SourceRegistrationReceipt, SqliteStoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let receipt = source_map::commit_source_registration(
            &transaction,
            decision_id,
            plan_fingerprint,
            patch,
            committed_at,
        )?;
        transaction.commit()?;
        Ok(receipt)
    }

    pub fn source_registration_receipt(
        &self,
        decision_id: &StableId,
    ) -> Result<Option<SourceRegistrationReceipt>, SqliteStoreError> {
        source_map::source_registration_receipt(&self.connection, decision_id)
    }

    pub fn map_objects(&self) -> Result<Vec<StoredMapObject>, SqliteStoreError> {
        source_map::map_objects(&self.connection)
    }

    pub fn source_map_snapshot(&self) -> Result<SourceMapSnapshot, SqliteStoreError> {
        source_map::source_map_snapshot(&self.connection)
    }

    /// Readonly canonical views derived from conversion history. Does not write.
    pub fn project_canonical_markdown(
        &self,
    ) -> Result<Vec<mathscape_domain::CanonicalMarkdownView>, SqliteStoreError> {
        canonical_markdown::project_canonical_markdown(&self.read_catalog_snapshot()?)
    }

    pub fn list_canonical_markdown(
        &self,
    ) -> Result<Vec<mathscape_domain::CanonicalMarkdownView>, SqliteStoreError> {
        canonical_markdown::list_canonical_markdown(&self.connection)
    }

    pub fn upsert_canonical_markdown(
        &mut self,
        key: &mathscape_domain::CanonicalMarkdownKey,
        head_artifact_id: Option<&StableId>,
        head_revision: Option<&str>,
        total_pages: Option<u32>,
        recorded_at: &Timestamp,
    ) -> Result<StableId, SqliteStoreError> {
        canonical_markdown::upsert_canonical_markdown(
            &self.connection,
            key,
            head_artifact_id,
            head_revision,
            total_pages,
            recorded_at,
        )
    }

    pub fn record_canonical_coverage_segment(
        &mut self,
        canonical_id: &StableId,
        segment: &mathscape_domain::CanonicalSegment,
        recorded_at: &Timestamp,
    ) -> Result<(), SqliteStoreError> {
        canonical_markdown::record_canonical_coverage_segment(
            &self.connection,
            canonical_id,
            segment,
            recorded_at,
        )
    }

    /// Rebuilds the registry from the current catalog. Replaces every canonical
    /// row. Does not run Marker or modify artifact bytes.
    pub fn sync_canonical_markdown_from_catalog(
        &mut self,
        recorded_at: &Timestamp,
    ) -> Result<Vec<mathscape_domain::CanonicalMarkdownView>, SqliteStoreError> {
        let views = self.project_canonical_markdown()?;
        canonical_markdown::replace_canonical_markdown(&mut self.connection, &views, recorded_at)
    }

    /// Silent normalization on open / landscape load: rebuilds the registry
    /// only when it differs from the catalog projection (ignoring live-only
    /// fields such as `finished_at`). Returns whether a rebuild happened.
    /// Idempotent; never runs Marker or touches artifact bytes.
    pub fn sync_canonical_markdown_if_stale(
        &mut self,
        recorded_at: &Timestamp,
    ) -> Result<bool, SqliteStoreError> {
        let projected = self.project_canonical_markdown()?;
        let registered = self.list_canonical_markdown()?;
        if canonical_markdown::registry_key(&projected)
            == canonical_markdown::registry_key(&registered)
        {
            return Ok(false);
        }
        canonical_markdown::replace_canonical_markdown(
            &mut self.connection,
            &projected,
            recorded_at,
        )?;
        Ok(true)
    }

    /// Rebuilds the registry rows of one source PDF from the current catalog.
    /// Other PDFs' rows are untouched. Returns that PDF's views.
    pub fn sync_canonical_markdown_for_pdf(
        &mut self,
        source_pdf_artifact_id: &StableId,
        recorded_at: &Timestamp,
    ) -> Result<Vec<mathscape_domain::CanonicalMarkdownView>, SqliteStoreError> {
        let views = self.project_canonical_markdown()?;
        canonical_markdown::replace_canonical_markdown_for_pdf(
            &mut self.connection,
            source_pdf_artifact_id,
            &views,
            recorded_at,
        )
    }

    /// Claims the per-PDF ensure lock, a global Marker slot and page budget in
    /// one `BEGIN IMMEDIATE` transaction. Expired leases become `abandoned`.
    pub fn claim_canonical_ensure(
        &mut self,
        claim: &CanonicalEnsureClaim,
        limits: CanonicalEnsureLimits,
        now_ms: i64,
    ) -> Result<CanonicalEnsureClaimOutcome, SqliteStoreError> {
        canonical_ensure::claim(&mut self.connection, claim, limits, now_ms)
    }

    /// Writes the terminal receipt of a claimed ensure and releases its lock.
    pub fn finish_canonical_ensure(
        &mut self,
        ensure_id: &str,
        outcome: CanonicalEnsureOutcome,
        conversion_id: Option<&str>,
        markdown_artifact_id: Option<&str>,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<(), SqliteStoreError> {
        canonical_ensure::finish(
            &self.connection,
            ensure_id,
            outcome,
            conversion_id,
            markdown_artifact_id,
            error,
            now_ms,
        )
    }

    /// Heartbeat for a claimed ensure: extends its lease deadline. False when
    /// the lease is gone (finished, abandoned or already expired).
    pub fn renew_canonical_ensure(
        &mut self,
        ensure_id: &str,
        lease_expires_at_ms: i64,
        now_ms: i64,
    ) -> Result<bool, SqliteStoreError> {
        canonical_ensure::renew(&self.connection, ensure_id, lease_expires_at_ms, now_ms)
    }

    /// Live (`running`, unexpired) ensures: the 「解析中」 signal.
    pub fn active_canonical_ensures(
        &self,
        now_ms: i64,
    ) -> Result<Vec<CanonicalEnsureLease>, SqliteStoreError> {
        canonical_ensure::live_leases(&self.connection, now_ms)
    }

    pub fn canonical_ensure_receipt(
        &self,
        ensure_id: &str,
    ) -> Result<Option<CanonicalEnsureReceipt>, SqliteStoreError> {
        canonical_ensure::receipt(&self.connection, ensure_id)
    }

    pub fn landscape_snapshot(&self) -> Result<LandscapeSnapshot, SqliteStoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        let snapshot = LandscapeSnapshot {
            artifacts: read::catalog_snapshot_from(&transaction)?,
            sources: source_map::source_map_snapshot(&transaction)?,
            ink_snapshot_labels: ink::snapshot_labels_by_artifact(&transaction)?,
            canonical_markdown: canonical_markdown::list_canonical_markdown(&transaction)?,
            canonical_ensures: canonical_ensure::live_leases(
                &transaction,
                canonical_ensure::unix_millis_now(),
            )?,
        };
        transaction.commit()?;
        Ok(snapshot)
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
        // Acquire the writer before reading the expected identity. Concurrent
        // verification must not try to upgrade an older WAL read snapshot.
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
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

    pub fn save_markdown_patch_set(
        &mut self,
        patch_set_id: &StableId,
        patch_set: &mathscape_domain::MarkdownPatchSet,
        status: mathscape_domain::MarkdownPatchStatus,
        patched_artifact_id: Option<&StableId>,
        conversion_id: Option<&StableId>,
        now: &Timestamp,
    ) -> Result<(), SqliteStoreError> {
        markdown_patch::save_patch_set(
            &mut self.connection,
            patch_set_id,
            patch_set,
            status,
            patched_artifact_id,
            conversion_id,
            now,
        )
    }

    pub fn save_markdown_patch_draft(
        &mut self,
        patch_set_id: &StableId,
        patch_ref: &str,
        replacement: &str,
        revision: u64,
        actor: &StableId,
        updated_at: &Timestamp,
    ) -> Result<(), SqliteStoreError> {
        markdown_patch::save_patch_draft(
            &mut self.connection,
            patch_set_id,
            patch_ref,
            replacement,
            revision,
            actor,
            updated_at,
        )
    }

    pub fn update_markdown_patch_status(
        &mut self,
        patch_set_id: &StableId,
        status: mathscape_domain::MarkdownPatchStatus,
        patched_artifact_id: Option<&StableId>,
        conversion_id: Option<&StableId>,
        updated_at: &Timestamp,
    ) -> Result<(), SqliteStoreError> {
        markdown_patch::update_patch_set_status(
            &self.connection,
            patch_set_id,
            status,
            patched_artifact_id,
            conversion_id,
            updated_at,
        )
    }

    pub fn record_markdown_patch_decision(
        &mut self,
        decision: &mathscape_domain::MarkdownPatchDecision,
        created_at: &Timestamp,
        patch_set_id: &StableId,
    ) -> Result<(), SqliteStoreError> {
        markdown_patch::record_patch_decision(
            &mut self.connection,
            decision,
            created_at,
            patch_set_id,
        )
    }

    pub fn record_markdown_patch_application(
        &mut self,
        application_id: &StableId,
        patch_set_id: &StableId,
        receipt: &mathscape_domain::MarkdownPatchApplyReceipt,
        error: Option<&str>,
        created_at: &Timestamp,
    ) -> Result<(), SqliteStoreError> {
        markdown_patch::record_patch_application(
            &mut self.connection,
            application_id,
            patch_set_id,
            receipt,
            error,
            created_at,
        )
    }

    pub fn markdown_patch_set_json(
        &self,
        patch_set_id: &StableId,
    ) -> Result<Option<serde_json::Value>, SqliteStoreError> {
        markdown_patch::patch_set_json(&self.connection, patch_set_id)
    }

    pub fn markdown_patch_set_status(
        &self,
        patch_set_id: &StableId,
    ) -> Result<Option<mathscape_domain::MarkdownPatchStatus>, SqliteStoreError> {
        markdown_patch::patch_set_status(&self.connection, patch_set_id)
    }

    pub fn markdown_patch_drafts(
        &self,
        patch_set_id: &StableId,
    ) -> Result<std::collections::BTreeMap<String, String>, SqliteStoreError> {
        markdown_patch::patch_drafts(&self.connection, patch_set_id)
    }

    pub fn latest_markdown_patch_application(
        &self,
        patch_set_id: &StableId,
    ) -> Result<Option<StoredPatchApplication>, SqliteStoreError> {
        markdown_patch::latest_patch_application(&self.connection, patch_set_id)
    }

    pub fn latest_markdown_patch_set_json_for_run(
        &self,
        run_id: &StableId,
    ) -> Result<Option<(StableId, serde_json::Value, String)>, SqliteStoreError> {
        markdown_patch::latest_patch_set_json_for_run(&self.connection, run_id)
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

    pub fn list_llm_profiles(&self) -> Result<Vec<StoredLlmProfile>, SqliteStoreError> {
        let mut statement = self.connection.prepare(
            "SELECT profile_id, profile_json, is_default
             FROM llm_profiles
             ORDER BY is_default DESC, profile_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut profiles = Vec::new();
        for row in rows {
            let (profile_id, profile_json, is_default) = row?;
            let profile: LlmProfile = serde_json::from_str(&profile_json)?;
            profile
                .validate()
                .map_err(|error| SqliteStoreError::InvalidLlmProfile(error.to_string()))?;
            if profile.id != profile_id {
                return Err(SqliteStoreError::InvalidLlmProfile(format!(
                    "row id '{profile_id}' does not match payload id '{}'",
                    profile.id
                )));
            }
            profiles.push(StoredLlmProfile {
                profile,
                is_default: sqlite_bool(is_default, "llm_profiles.is_default")?,
            });
        }
        Ok(profiles)
    }

    pub fn llm_profile(
        &self,
        profile_id: &str,
    ) -> Result<Option<StoredLlmProfile>, SqliteStoreError> {
        Ok(self
            .list_llm_profiles()?
            .into_iter()
            .find(|stored| stored.profile.id == profile_id))
    }

    pub fn default_llm_profile(&self) -> Result<Option<StoredLlmProfile>, SqliteStoreError> {
        Ok(self
            .list_llm_profiles()?
            .into_iter()
            .find(StoredLlmProfile::is_default))
    }

    pub fn upsert_llm_profile(
        &mut self,
        profile: &LlmProfile,
        make_default: bool,
    ) -> Result<StoredLlmProfile, SqliteStoreError> {
        profile
            .validate()
            .map_err(|error| SqliteStoreError::InvalidLlmProfile(error.to_string()))?;
        let profile_json = serde_json::to_string(profile)?;
        let transaction = self.connection.transaction()?;
        let existing_default = transaction
            .query_row(
                "SELECT is_default FROM llm_profiles WHERE profile_id = ?1",
                [profile.id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .map(|value| sqlite_bool(value, "llm_profiles.is_default"))
            .transpose()?
            .unwrap_or(false);
        let profile_count =
            transaction.query_row("SELECT COUNT(*) FROM llm_profiles", [], |row| {
                row.get::<_, i64>(0)
            })?;
        let is_default = make_default || existing_default || profile_count == 0;
        if is_default {
            transaction.execute("UPDATE llm_profiles SET is_default = 0", [])?;
        }
        transaction.execute(
            "INSERT INTO llm_profiles (profile_id, profile_json, is_default)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(profile_id) DO UPDATE SET
                profile_json = excluded.profile_json,
                is_default = excluded.is_default,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            params![profile.id, profile_json, i64::from(is_default)],
        )?;
        transaction.commit()?;
        Ok(StoredLlmProfile {
            profile: profile.clone(),
            is_default,
        })
    }

    /// Loads the singleton versioned settings document owned by the
    /// application layer. Absence means the application should use defaults.
    pub fn workspace_settings<T: DeserializeOwned>(&self) -> Result<Option<T>, SqliteStoreError> {
        let settings = self
            .connection
            .query_row(
                "SELECT settings_json FROM workspace_settings WHERE singleton_id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        settings
            .map(|settings| serde_json::from_str(&settings).map_err(SqliteStoreError::from))
            .transpose()
    }

    /// Atomically replaces the singleton settings document. Secrets must not
    /// be represented by the caller's settings type.
    pub fn save_workspace_settings<T: Serialize>(
        &mut self,
        settings: &T,
    ) -> Result<(), SqliteStoreError> {
        let settings = serde_json::to_string(settings)?;
        self.connection.execute(
            "INSERT INTO workspace_settings (singleton_id, settings_json)
             VALUES (1, ?1)
             ON CONFLICT(singleton_id) DO UPDATE SET
                settings_json = excluded.settings_json,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            [settings],
        )?;
        Ok(())
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

fn validate_artifact_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
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

fn validate_workspace_settings_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    validate_llm_profiles_schema(connection)?;
    let name = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 3",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if name.as_deref() != Some("workspace_settings_v1") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 3 is absent or has an unexpected name",
        ));
    }
    Ok(())
}

fn validate_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    validate_discovery_schema(connection)?;
    let name = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 5",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if name.as_deref() != Some("source_map_v1") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 5 is absent or has an unexpected name",
        ));
    }
    Ok(())
}

fn validate_discovery_schema_v2(connection: &Connection) -> Result<(), SqliteStoreError> {
    let name = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 6",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if name.as_deref() != Some("discovery_runs_v2") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 6 is absent or has an unexpected name",
        ));
    }
    if !table_has_column(connection, "discovery_runs", "result_fingerprint")? {
        return Err(SqliteStoreError::MissingSchemaObject {
            object_kind: "column",
            name: "discovery_runs.result_fingerprint",
        });
    }
    validate_required_schema_objects(connection)?;
    Ok(())
}

fn reconcile_schema_v6(connection: &Connection) -> Result<(), SqliteStoreError> {
    // Opening a healthy catalog is a read operation (including concurrent PDF
    // range reads). Do not replay DDL/user_version writes on every connection.
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 6 && validate_discovery_schema_v2(connection).is_ok() {
        return Ok(());
    }
    // Reconciliation reads before writing. Acquire the writer slot up front;
    // upgrading a deferred WAL snapshot can fail immediately with BUSY_SNAPSHOT.
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)?;
    if !table_has_column(&transaction, "discovery_runs", "result_fingerprint")? {
        transaction
            .execute_batch("ALTER TABLE discovery_runs ADD COLUMN result_fingerprint TEXT;")?;
    }
    transaction.execute_batch(DISCOVERY_RUNS_V2_MIGRATION)?;
    let migration_name = transaction
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 6",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    match migration_name.as_deref() {
        None => {
            transaction.execute(
                "INSERT INTO schema_migrations (version, name) VALUES (6, 'discovery_runs_v2')",
                [],
            )?;
        }
        Some("discovery_runs_v2") => {}
        Some(_) => {
            return Err(SqliteStoreError::InvalidSchema(
                "migration 6 has an unexpected name",
            ));
        }
    }
    transaction.pragma_update(None, "user_version", 6)?;
    validate_discovery_schema_v2(&transaction)?;
    transaction.commit()?;
    validate_discovery_schema_v2(connection)
}

/// Rebuilds `discovery_runs` to admit `awaiting_patch_review`. Child tables
/// keep referencing the table by name, so foreign keys must be off while the
/// old table is dropped; integrity is re-proved before commit.
fn migrate_discovery_patch_review(connection: &Connection) -> Result<(), SqliteStoreError> {
    connection.pragma_update(None, "foreign_keys", false)?;
    let migrated = (|| {
        let transaction = rusqlite::Transaction::new_unchecked(
            connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        transaction.execute_batch(DISCOVERY_PATCH_REVIEW_MIGRATION)?;
        let violation = transaction
            .query_row("PRAGMA foreign_key_check", [], |row| {
                row.get::<_, String>(0)
            })
            .optional()?;
        if violation.is_some() {
            return Err(SqliteStoreError::InvalidSchema(
                "discovery_runs rebuild left dangling foreign keys",
            ));
        }
        transaction.commit()?;
        Ok(())
    })();
    connection.pragma_update(None, "foreign_keys", true)?;
    migrated
}

fn migrate_discovery_run_recovery(connection: &Connection) -> Result<(), SqliteStoreError> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)?;
    transaction.execute_batch(DISCOVERY_RUN_RECOVERY_MIGRATION)?;
    transaction.commit()?;
    validate_discovery_run_recovery_schema(connection)
}

const REQUIRED_TABLE_COLUMNS: &[(&str, &[&str])] = &[
    ("schema_migrations", &["version", "name"]),
    (
        "artifacts",
        &[
            "artifact_id",
            "content_hash",
            "byte_size",
            "current_revision",
        ],
    ),
    (
        "artifact_revisions",
        &[
            "artifact_id",
            "revision_id",
            "edition_id",
            "media_type",
            "original_filename",
            "page_count",
            "rights_status",
            "permitted_uses",
            "rights_note",
            "schema_version",
            "lifecycle",
            "created_at",
            "created_by",
            "generation_run",
        ],
    ),
    (
        "artifact_locations",
        &[
            "location_id",
            "artifact_id",
            "storage_kind",
            "uri",
            "priority",
            "role",
            "last_verified_at",
            "current_status",
        ],
    ),
    (
        "artifact_checks",
        &[
            "check_id",
            "location_id",
            "checked_at",
            "observed_hash",
            "observed_size",
            "result",
            "checker_version",
        ],
    ),
    (
        "conversion_runs",
        &[
            "run_id",
            "revision_id",
            "schema_version",
            "lifecycle",
            "created_at",
            "created_by",
            "generation_run",
            "tool_name",
            "tool_version",
            "command_template",
            "command_record_status",
            "page_start",
            "page_end",
            "page_index_base",
            "output_format",
            "redo_inline_math",
            "additional_options",
            "environment",
            "started_at",
            "finished_at",
            "exit_status",
            "warnings",
            "cost",
        ],
    ),
    (
        "conversion_endpoints",
        &["run_id", "revision_id", "role", "ordinal", "artifact_id"],
    ),
    (
        "llm_profiles",
        &["profile_id", "profile_json", "is_default", "updated_at"],
    ),
    (
        "workspace_settings",
        &["singleton_id", "settings_json", "updated_at"],
    ),
    (
        "discovery_runs",
        &[
            "run_id",
            "artifact_id",
            "conversion_id",
            "pipeline_version",
            "input_fingerprint",
            "selection_fingerprint",
            "unit_set_fingerprint",
            "result_fingerprint",
            "status",
            "admission",
            "profile_id",
            "snapshot_json",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "discovery_attempts",
        &[
            "run_id",
            "ordinal",
            "stage",
            "batch_fingerprint",
            "repair",
            "status",
            "request_json",
            "raw_text",
            "run_json",
            "error",
        ],
    ),
    (
        "discovery_validation_reports",
        &["run_id", "report_ordinal", "report_json"],
    ),
    (
        "discovery_diagnostics",
        &[
            "run_id",
            "report_ordinal",
            "diagnostic_ordinal",
            "code",
            "path",
            "message",
            "checked",
        ],
    ),
    (
        "discovery_units",
        &[
            "run_id",
            "unit_ref",
            "parent_ref",
            "unit_kind",
            "completeness",
            "unit_set_fingerprint",
            "payload_json",
        ],
    ),
    (
        "discovery_fact_tasks",
        &[
            "run_id",
            "task_ref",
            "owner_unit_ref",
            "processed",
            "requires_unit_revision",
            "payload_json",
        ],
    ),
    (
        "discovery_pending_tasks",
        &["run_id", "task_ref", "owner_unit_ref"],
    ),
    (
        "discovery_acceptance_decisions",
        &[
            "decision_id",
            "run_id",
            "actor",
            "decided_at",
            "authority",
            "expected_result_fingerprint",
            "selection_json",
            "plan_fingerprint",
            "plan_json",
            "created_at",
        ],
    ),
    (
        "map_objects",
        &[
            "object_id",
            "object_shape",
            "entity_kind",
            "current_revision",
        ],
    ),
    (
        "map_object_revisions",
        &[
            "object_id",
            "revision_id",
            "schema_version",
            "lifecycle",
            "created_at",
            "created_by",
            "generation_run",
            "payload_json",
        ],
    ),
    (
        "map_relation_endpoints",
        &[
            "relation_id",
            "revision_id",
            "role",
            "ordinal_key",
            "ordinal",
            "entity_id",
        ],
    ),
    (
        "map_commits",
        &[
            "commit_id",
            "author",
            "generation_run",
            "message",
            "committed_at",
        ],
    ),
    (
        "map_commit_objects",
        &[
            "commit_id",
            "ordinal",
            "object_id",
            "revision_id",
            "outcome",
        ],
    ),
    (
        "source_registration_commits",
        &[
            "decision_id",
            "plan_fingerprint",
            "commit_id",
            "receipt_json",
            "committed_at",
        ],
    ),
];

const REQUIRED_INDEXES: &[&str] = &[
    "artifact_locations_by_artifact",
    "artifact_checks_by_location",
    "llm_profiles_one_default",
    "discovery_runs_by_source",
    "discovery_acceptance_by_run",
    "map_objects_by_kind",
    "map_relation_endpoints_by_entity",
];

fn validate_required_schema_objects(connection: &Connection) -> Result<(), SqliteStoreError> {
    for &(table, columns) in REQUIRED_TABLE_COLUMNS {
        if !schema_object_exists(connection, "table", table)? {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "table",
                name: table,
            });
        }
        for &column in columns {
            if !table_has_column(connection, table, column)? {
                return Err(SqliteStoreError::MissingSchemaObject {
                    object_kind: "column",
                    name: column,
                });
            }
        }
    }
    for &index in REQUIRED_INDEXES {
        if !schema_object_exists(connection, "index", index)? {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "index",
                name: index,
            });
        }
    }
    Ok(())
}

fn validate_markdown_patch_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    for table in [
        "markdown_patch_sets",
        "markdown_patch_operations",
        "markdown_patch_decisions",
        "markdown_patch_applications",
    ] {
        if !schema_object_exists(connection, "table", table)? {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "table",
                name: table,
            });
        }
    }
    Ok(())
}

fn validate_proposed_change_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    for table in ["proposed_change_sets", "proposed_change_set_versions"] {
        if !schema_object_exists(connection, "table", table)? {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "table",
                name: table,
            });
        }
    }
    connection.prepare(
        "SELECT change_set_id, version, investigation_id, round_id, generation,
                request_id, request_fingerprint, proposal_json, result_json,
                state, base_commit, created_at, proposed_by
         FROM proposed_change_set_versions LIMIT 0",
    )?;
    for table in [
        "change_set_dependencies",
        "change_set_decisions",
        "investigation_rounds",
    ] {
        if !schema_object_exists(connection, "table", table)? {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "table",
                name: table,
            });
        }
    }
    Ok(())
}

fn validate_discovery_run_recovery_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    for column in [
        "run_id",
        "disposition",
        "reason",
        "actor",
        "request_id",
        "decided_at",
    ] {
        if !table_has_column(connection, "discovery_run_dispositions", column)? {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "column",
                name: "discovery_run_dispositions",
            });
        }
    }
    for index in ["discovery_run_dispositions_by_decision"] {
        if !schema_object_exists(connection, "index", index)? {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "index",
                name: index,
            });
        }
    }
    let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version >= 12 {
        connection.prepare("SELECT request_id,run_id,disposition,previous_request_id,result_fingerprint,actor,reason,decided_at FROM discovery_run_disposition_events LIMIT 0")?;
    }
    Ok(())
}

fn schema_object_exists(
    connection: &Connection,
    object_kind: &str,
    name: &str,
) -> Result<bool, SqliteStoreError> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2",
            params![object_kind, name],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(SqliteStoreError::from)
}

fn table_has_column(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, SqliteStoreError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_discovery_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    validate_workspace_settings_schema(connection)?;
    let name = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 4",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if name.as_deref() != Some("discovery_runs_v1") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 4 is absent or has an unexpected name",
        ));
    }
    Ok(())
}

fn validate_llm_profiles_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    validate_artifact_schema(connection)?;
    let name = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 2",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if name.as_deref() != Some("llm_profiles_v1") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 2 is absent or has an unexpected name",
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

fn sqlite_bool(value: i64, field: &'static str) -> Result<bool, SqliteStoreError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(SqliteStoreError::InvalidStoredValue {
            field,
            value: value.to_string(),
        }),
    }
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

fn validate_registration_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    connection.prepare("SELECT request_id,decision_id,registration_commit_id,base_commit,plan_fingerprint,request_json,map_commit_id,receipt_json,decided_at FROM discovery_registration_reversals LIMIT 0")?;
    connection.prepare("SELECT request_id,decision_id,request_json,plan_fingerprint,receipt_json FROM source_registration_requests LIMIT 0")?;
    connection.prepare(
        "SELECT commit_id,ordinal,object_id,revision_id,outcome FROM map_commit_objects LIMIT 0",
    )?;
    Ok(())
}
