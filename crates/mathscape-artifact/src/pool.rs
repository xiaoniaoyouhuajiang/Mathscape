use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_domain::{
    ActorRef, ArtifactCheck, ArtifactCheckResult, ArtifactLocation, ArtifactUri, ContentHash,
    EntityEnvelope, EntityHeader, EntityKind, LifecycleStatus, LocationRole, LocationStatus,
    MediaType, RevisionId, RightsRecord, RightsStatus, SchemaVersion, SourceArtifact,
    SourceRelation, StableId, StorageKind, Timestamp,
};
use mathscape_store_sqlite::{RegistrationOutcome, SqliteArtifactStore};
use sha2::{Digest, Sha256};

use crate::{ArtifactPoolError, WorkspaceRoot, error::io_error};

const CHECKER_VERSION: &str = concat!("mathscape-artifact/", env!("CARGO_PKG_VERSION"));
const SCHEMA_VERSION: &str = "mathscape-artifact-pool-1";
static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactMetadata {
    edition: Option<StableId>,
    media_type: MediaType,
    original_filename: Option<String>,
    page_count: Option<u32>,
    rights: RightsRecord,
}

impl ArtifactMetadata {
    pub fn new(
        edition: Option<StableId>,
        media_type: MediaType,
        original_filename: Option<String>,
        page_count: Option<u32>,
        rights: RightsRecord,
    ) -> Self {
        Self {
            edition,
            media_type,
            original_filename,
            page_count,
            rights,
        }
    }

    pub fn derived(
        media_type: MediaType,
        original_filename: Option<String>,
    ) -> Result<Self, ArtifactPoolError> {
        Ok(Self::new(
            None,
            media_type,
            original_filename,
            None,
            RightsRecord::new(RightsStatus::DerivativeOfUnassessedSource, vec![], None)?,
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactImportOutcome {
    Inserted,
    Deduplicated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedArtifact {
    stable_id: StableId,
    content_hash: ContentHash,
    byte_size: u64,
    uri: ArtifactUri,
    outcome: ArtifactImportOutcome,
}

impl ManagedArtifact {
    pub fn stable_id(&self) -> &StableId {
        &self.stable_id
    }

    pub fn content_hash(&self) -> &ContentHash {
        &self.content_hash
    }

    pub const fn byte_size(&self) -> u64 {
        self.byte_size
    }

    pub fn uri(&self) -> &ArtifactUri {
        &self.uri
    }

    pub const fn outcome(&self) -> ArtifactImportOutcome {
        self.outcome
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactVerification {
    artifact_id: StableId,
    result: ArtifactCheckResult,
    observed_hash: Option<ContentHash>,
    observed_size: Option<u64>,
}

impl ArtifactVerification {
    pub fn artifact_id(&self) -> &StableId {
        &self.artifact_id
    }

    pub const fn result(&self) -> ArtifactCheckResult {
        self.result
    }

    pub fn observed_hash(&self) -> Option<&ContentHash> {
        self.observed_hash.as_ref()
    }

    pub fn observed_size(&self) -> Option<u64> {
        self.observed_size
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PortabilityReport {
    managed_artifacts: u64,
    external_artifacts: u64,
    missing_artifacts: u64,
    corrupted_artifacts: u64,
    unavailable_artifacts: u64,
    bytes_required_to_hydrate: u64,
}

impl PortabilityReport {
    pub const fn managed_artifacts(&self) -> u64 {
        self.managed_artifacts
    }

    pub const fn external_artifacts(&self) -> u64 {
        self.external_artifacts
    }

    pub const fn missing_artifacts(&self) -> u64 {
        self.missing_artifacts
    }

    pub const fn corrupted_artifacts(&self) -> u64 {
        self.corrupted_artifacts
    }

    pub const fn unavailable_artifacts(&self) -> u64 {
        self.unavailable_artifacts
    }

    pub const fn bytes_required_to_hydrate(&self) -> u64 {
        self.bytes_required_to_hydrate
    }

    pub const fn is_self_contained(&self) -> bool {
        self.external_artifacts == 0
            && self.missing_artifacts == 0
            && self.corrupted_artifacts == 0
            && self.unavailable_artifacts == 0
    }
}

pub struct ArtifactPool {
    workspace: WorkspaceRoot,
    catalog: SqliteArtifactStore,
}

impl ArtifactPool {
    pub fn create(path: impl AsRef<Path>) -> Result<Self, ArtifactPoolError> {
        let workspace = WorkspaceRoot::create(path)?;
        let catalog = SqliteArtifactStore::prepare(&workspace.database_path())?;
        Ok(Self { workspace, catalog })
    }

    /// Prepare an existing workspace before serving concurrent application requests.
    pub fn prepare(path: impl AsRef<Path>) -> Result<Self, ArtifactPoolError> {
        let workspace = WorkspaceRoot::open(path)?;
        let catalog = SqliteArtifactStore::prepare(&workspace.database_path())?;
        Ok(Self { workspace, catalog })
    }

    /// Connect to an already prepared workspace without schema maintenance.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArtifactPoolError> {
        let workspace = WorkspaceRoot::open(path)?;
        let catalog = SqliteArtifactStore::open(&workspace.database_path())?;
        Ok(Self { workspace, catalog })
    }

    pub fn workspace(&self) -> &WorkspaceRoot {
        &self.workspace
    }

    pub fn import_file(
        &mut self,
        source: impl AsRef<Path>,
        metadata: ArtifactMetadata,
        actor: ActorRef,
    ) -> Result<ManagedArtifact, ArtifactPoolError> {
        self.import_file_inner(source.as_ref(), None, metadata, actor)
    }

    pub fn import_file_with_expected_hash(
        &mut self,
        source: impl AsRef<Path>,
        expected_hash: &ContentHash,
        metadata: ArtifactMetadata,
        actor: ActorRef,
    ) -> Result<ManagedArtifact, ArtifactPoolError> {
        self.import_file_inner(source.as_ref(), Some(expected_hash), metadata, actor)
    }

    /// Imports generated bytes through the same managed, content-addressed
    /// path as a user file. The temporary staging file is removed after the
    /// normal no-clobber promotion and catalog transaction complete.
    pub fn import_bytes(
        &mut self,
        bytes: &[u8],
        metadata: ArtifactMetadata,
        actor: ActorRef,
    ) -> Result<ManagedArtifact, ArtifactPoolError> {
        let staged = unique_path(&self.workspace.staging_path(), "bytes");
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged)
                .map_err(|error| io_error("create staged byte artifact", &staged, error))?;
            file.write_all(bytes)
                .map_err(|error| io_error("write staged byte artifact", &staged, error))?;
            file.sync_all()
                .map_err(|error| io_error("sync staged byte artifact", &staged, error))?;
            Ok::<(), ArtifactPoolError>(())
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&staged);
            return Err(error);
        }
        let result = self.import_file_inner(&staged, None, metadata, actor);
        let _ = fs::remove_file(&staged);
        result
    }

    fn import_file_inner(
        &mut self,
        source: &Path,
        expected_hash: Option<&ContentHash>,
        metadata: ArtifactMetadata,
        actor: ActorRef,
    ) -> Result<ManagedArtifact, ArtifactPoolError> {
        if !fs::metadata(source)
            .map_err(|error| io_error("inspect import source", source, error))?
            .is_file()
        {
            return Err(ArtifactPoolError::SourceNotFile(source.to_owned()));
        }

        let staged = unique_path(&self.workspace.staging_path(), "import");
        let (content_hash, byte_size) = match copy_and_hash(source, &staged) {
            Ok(observation) => observation,
            Err(error) => {
                if staged.exists() {
                    quarantine(&self.workspace, &staged, "copy-failed")?;
                }
                return Err(error);
            }
        };
        if let Some(expected_hash) = expected_hash
            && expected_hash != &content_hash
        {
            let quarantine = quarantine(&self.workspace, &staged, "digest-mismatch")?;
            return Err(ArtifactPoolError::DigestMismatch {
                expected: expected_hash.clone(),
                actual: content_hash,
                quarantine,
            });
        }

        let destination = self.workspace.object_path(&content_hash);
        if let Err(error) = promote_noclobber(
            &self.workspace,
            &staged,
            &destination,
            &content_hash,
            byte_size,
        ) {
            if staged.exists() {
                quarantine(&self.workspace, &staged, "promotion-failed")?;
            }
            return Err(error);
        }

        let digest = digest(&content_hash);
        let stable_id = StableId::parse(format!("mathscape:artifact/sha256/{digest}"))?;
        let location_id = StableId::parse(format!(
            "mathscape:artifact-location/sha256/{digest}/managed-primary"
        ))?;
        let now = timestamp_now()?;
        let artifact = SourceArtifact::new(
            metadata.edition,
            metadata.media_type,
            content_hash.clone(),
            byte_size,
            metadata.original_filename,
            metadata.page_count,
            metadata.rights,
        )?;
        let revision_id = metadata_revision(&artifact);
        let entity = EntityEnvelope::new(
            EntityHeader::new(
                stable_id.clone(),
                revision_id,
                EntityKind::SourceArtifact,
                SchemaVersion::parse(SCHEMA_VERSION)?,
                LifecycleStatus::Active,
                now.clone(),
                actor,
                None,
            ),
            artifact,
        )?;
        let uri = self.workspace.pool_uri(&content_hash)?;
        let location = ArtifactLocation::new(
            location_id.clone(),
            stable_id.clone(),
            StorageKind::Managed,
            uri.clone(),
            0,
            LocationRole::Primary,
            Some(now.clone()),
            LocationStatus::Available,
        );
        let check = ArtifactCheck::new(
            unique_stable_id("artifact-check", digest)?,
            location_id,
            now,
            Some(content_hash.clone()),
            Some(byte_size),
            ArtifactCheckResult::Valid,
            CHECKER_VERSION.to_owned(),
        )?;
        let outcome = self.catalog.register_artifact(&entity, &location, &check)?;

        Ok(ManagedArtifact {
            stable_id,
            content_hash,
            byte_size,
            uri,
            outcome: match outcome {
                RegistrationOutcome::Inserted => ArtifactImportOutcome::Inserted,
                RegistrationOutcome::Existing => ArtifactImportOutcome::Deduplicated,
            },
        })
    }

    pub fn verify(
        &mut self,
        artifact_id: &StableId,
    ) -> Result<ArtifactVerification, ArtifactPoolError> {
        let location = self
            .catalog
            .artifact_locations()?
            .into_iter()
            .find(|location| {
                location.artifact_id() == artifact_id
                    && location.storage_kind() == StorageKind::Managed
            })
            .ok_or_else(|| {
                ArtifactPoolError::MissingManagedLocation(artifact_id.as_str().to_owned())
            })?;
        let path = self.workspace.resolve_pool_uri(location.uri())?;
        let observation = hash_file(&path);
        let (result, status, observed_hash, observed_size) = match observation {
            Ok((observed_hash, observed_size))
                if observed_hash == *location.content_hash()
                    && observed_size == location.byte_size() =>
            {
                (
                    ArtifactCheckResult::Valid,
                    LocationStatus::Available,
                    Some(observed_hash),
                    Some(observed_size),
                )
            }
            Ok((observed_hash, observed_size)) => (
                ArtifactCheckResult::Corrupted,
                LocationStatus::Corrupted,
                Some(observed_hash),
                Some(observed_size),
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
                ArtifactCheckResult::Missing,
                LocationStatus::Missing,
                None,
                None,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => (
                ArtifactCheckResult::AccessDenied,
                LocationStatus::AccessDenied,
                None,
                None,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => (
                ArtifactCheckResult::Corrupted,
                LocationStatus::Corrupted,
                None,
                None,
            ),
            Err(_) => (
                ArtifactCheckResult::Unavailable,
                LocationStatus::Unavailable,
                None,
                None,
            ),
        };
        let checked_at = timestamp_now()?;
        let check = ArtifactCheck::new(
            unique_stable_id("artifact-check", digest(location.content_hash()))?,
            location.location_id().clone(),
            checked_at,
            observed_hash.clone(),
            observed_size,
            result,
            CHECKER_VERSION.to_owned(),
        )?;
        self.catalog.register_check(&check, status)?;
        Ok(ArtifactVerification {
            artifact_id: artifact_id.clone(),
            result,
            observed_hash,
            observed_size,
        })
    }

    pub fn audit_portability(&self) -> Result<PortabilityReport, ArtifactPoolError> {
        let mut report = PortabilityReport::default();
        let mut managed = HashSet::new();
        let mut external = HashSet::new();
        let mut missing = HashSet::new();
        let mut corrupted = HashSet::new();
        let mut unavailable = HashSet::new();
        for location in self.catalog.artifact_locations()? {
            match location.storage_kind() {
                StorageKind::Managed => {
                    managed.insert(location.artifact_id().clone());
                    let path = self.workspace.resolve_pool_uri(location.uri())?;
                    match fs::symlink_metadata(&path) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            missing.insert(location.artifact_id().clone());
                        }
                        Ok(metadata)
                            if metadata.file_type().is_symlink() || !metadata.is_file() =>
                        {
                            corrupted.insert(location.artifact_id().clone());
                        }
                        Err(_) => {
                            unavailable.insert(location.artifact_id().clone());
                        }
                        Ok(_) => {}
                    }
                    if location.status() == LocationStatus::Missing {
                        missing.insert(location.artifact_id().clone());
                    }
                    if location.status() == LocationStatus::Corrupted {
                        corrupted.insert(location.artifact_id().clone());
                    }
                    if matches!(
                        location.status(),
                        LocationStatus::AccessDenied | LocationStatus::Unavailable
                    ) {
                        unavailable.insert(location.artifact_id().clone());
                    }
                }
                StorageKind::ObjectStore | StorageKind::External | StorageKind::Archive => {
                    if external.insert(location.artifact_id().clone()) {
                        report.bytes_required_to_hydrate = report
                            .bytes_required_to_hydrate
                            .saturating_add(location.byte_size());
                    }
                }
            }
        }
        report.managed_artifacts = managed.len() as u64;
        report.external_artifacts = external.len() as u64;
        report.missing_artifacts = missing.len() as u64;
        report.corrupted_artifacts = corrupted.len() as u64;
        report.unavailable_artifacts = unavailable.len() as u64;
        Ok(report)
    }

    pub fn record_conversion(
        &mut self,
        relation: &SourceRelation,
    ) -> Result<(), ArtifactPoolError> {
        self.catalog.record_conversion(relation)?;
        Ok(())
    }

    pub fn artifact_count(&self) -> Result<u64, ArtifactPoolError> {
        self.catalog.artifact_count().map_err(Into::into)
    }

    pub fn check_count(&self) -> Result<u64, ArtifactPoolError> {
        self.catalog.check_count().map_err(Into::into)
    }

    pub fn conversion_count(&self) -> Result<u64, ArtifactPoolError> {
        self.catalog.conversion_count().map_err(Into::into)
    }

    /// Resolves a registered Managed Artifact without exposing persisted host paths.
    pub fn managed_path(&self, artifact_id: &StableId) -> Result<PathBuf, ArtifactPoolError> {
        let location = self
            .catalog
            .artifact_locations_for(artifact_id)?
            .into_iter()
            .find(|location| {
                location.artifact_id() == artifact_id
                    && location.storage_kind() == StorageKind::Managed
            })
            .ok_or_else(|| {
                ArtifactPoolError::MissingManagedLocation(artifact_id.as_str().to_owned())
            })?;
        let path = self.workspace.resolve_pool_uri(location.uri())?;
        ensure_regular_nonsymlink(&path)?;
        Ok(path)
    }

    /// Bounded in-memory reads of registered managed content. No caller-supplied path.
    pub fn read_bytes(
        &self,
        artifact_id: &StableId,
        offset: u64,
        length: u32,
    ) -> Result<Vec<u8>, ArtifactPoolError> {
        if length == 0 || length > 4 * 1024 * 1024 {
            return Err(ArtifactPoolError::InvalidReadRange);
        }
        let path = self.managed_path(artifact_id)?;
        let mut file =
            File::open(&path).map_err(|e| io_error("open artifact for reading", &path, e))?;
        let size = file
            .metadata()
            .map_err(|e| io_error("inspect artifact length", &path, e))?
            .len();
        if offset > size {
            return Err(ArtifactPoolError::InvalidReadRange);
        }
        let mut bytes = vec![0; u64::from(length).min(size - offset) as usize];
        file.seek(SeekFrom::Start(offset))
            .and_then(|_| file.read_exact(&mut bytes))
            .map_err(|e| io_error("read artifact bytes", &path, e))?;
        Ok(bytes)
    }
}

fn copy_and_hash(
    source: &Path,
    destination: &Path,
) -> Result<(ContentHash, u64), ArtifactPoolError> {
    let source_file =
        File::open(source).map_err(|error| io_error("open import source", source, error))?;
    let mut destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| io_error("create staged artifact", destination, error))?;
    let mut reader = BufReader::new(source_file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut byte_size = 0_u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| io_error("read import source", source, error))?;
        if read == 0 {
            break;
        }
        destination_file
            .write_all(&buffer[..read])
            .map_err(|error| io_error("write staged artifact", destination, error))?;
        hasher.update(&buffer[..read]);
        byte_size = byte_size.saturating_add(read as u64);
    }
    destination_file
        .sync_all()
        .map_err(|error| io_error("sync staged artifact", destination, error))?;
    let hash = ContentHash::parse(format!("sha256:{:x}", hasher.finalize()))?;
    Ok((hash, byte_size))
}

fn hash_file(path: &Path) -> std::io::Result<(ContentHash, u64)> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "managed object is not a regular non-symlink file",
        ));
    }
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut byte_size = 0_u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        byte_size = byte_size.saturating_add(read as u64);
    }
    let value = format!("sha256:{:x}", hasher.finalize());
    let hash = ContentHash::parse(value).expect("SHA-256 encoder produces a valid ContentHash");
    Ok((hash, byte_size))
}

fn promote_noclobber(
    workspace: &WorkspaceRoot,
    staged: &Path,
    destination: &Path,
    expected_hash: &ContentHash,
    expected_size: u64,
) -> Result<(), ArtifactPoolError> {
    let parent = destination
        .parent()
        .expect("managed object path always has a parent");
    fs::create_dir_all(parent)
        .map_err(|error| io_error("create managed object directory", parent, error))?;
    workspace.validate_object_parent(parent)?;
    match fs::hard_link(staged, destination) {
        Ok(()) => {
            protect_managed_object(destination)?;
            fs::remove_file(staged)
                .map_err(|error| io_error("remove staged artifact", staged, error))?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            ensure_regular_nonsymlink(destination)?;
            let (actual_hash, actual_size) = hash_file(destination).map_err(|source| {
                io_error("verify existing managed object", destination, source)
            })?;
            if &actual_hash != expected_hash || actual_size != expected_size {
                return Err(ArtifactPoolError::ManagedObjectConflict {
                    path: destination.to_owned(),
                    expected: expected_hash.clone(),
                });
            }
            fs::remove_file(staged)
                .map_err(|source| io_error("remove duplicate staged artifact", staged, source))?;
            Ok(())
        }
        Err(source) => Err(io_error(
            "atomically promote staged artifact",
            destination,
            source,
        )),
    }
}

#[cfg(unix)]
fn protect_managed_object(path: &Path) -> Result<(), ArtifactPoolError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|source| io_error("inspect managed object", path, source))?
        .permissions();
    permissions.set_mode(0o444);
    fs::set_permissions(path, permissions)
        .map_err(|source| io_error("protect managed object", path, source))
}

#[cfg(not(unix))]
fn protect_managed_object(_path: &Path) -> Result<(), ArtifactPoolError> {
    // Windows read-only files complicate atomic workspace removal and upgrade.
    // Immutability remains enforced by content addressing and no-clobber writes.
    Ok(())
}

fn ensure_regular_nonsymlink(path: &Path) -> Result<(), ArtifactPoolError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| io_error("inspect managed object", path, source))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ArtifactPoolError::UnsafeManagedPath(path.to_owned()));
    }
    Ok(())
}

fn quarantine(
    workspace: &WorkspaceRoot,
    staged: &Path,
    reason: &str,
) -> Result<PathBuf, ArtifactPoolError> {
    let destination = unique_path(&workspace.quarantine_path(), reason);
    fs::rename(staged, &destination)
        .map_err(|error| io_error("quarantine staged artifact", &destination, error))?;
    Ok(destination)
}

fn unique_path(parent: &Path, prefix: &str) -> PathBuf {
    let unique = unique_number();
    parent.join(format!("{prefix}-{}-{unique}", process::id()))
}

fn unique_stable_id(kind: &str, digest: &str) -> Result<StableId, ArtifactPoolError> {
    StableId::parse(format!(
        "mathscape:{kind}/sha256/{digest}/{}",
        unique_number()
    ))
    .map_err(Into::into)
}

fn unique_number() -> u128 {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = u128::from(UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed));
    timestamp.saturating_mul(1_000_000).saturating_add(sequence)
}

fn timestamp_now() -> Result<Timestamp, ArtifactPoolError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp::parse(format!(
        "unix:{}.{}",
        duration.as_secs(),
        duration.subsec_nanos()
    ))
    .map_err(Into::into)
}

fn metadata_revision(artifact: &SourceArtifact) -> RevisionId {
    let mut hasher = Sha256::new();
    hash_part(&mut hasher, artifact.content_hash().as_str());
    hash_part(&mut hasher, &artifact.byte_size().to_string());
    hash_part(&mut hasher, artifact.media_type().as_str());
    hash_part(&mut hasher, artifact.edition().map_or("", StableId::as_str));
    hash_part(&mut hasher, artifact.original_filename().unwrap_or(""));
    hash_part(
        &mut hasher,
        &artifact
            .page_count()
            .map_or_else(String::new, |v| v.to_string()),
    );
    hash_part(&mut hasher, rights_status(artifact.rights().status()));
    for permitted_use in artifact.rights().permitted_uses() {
        hash_part(&mut hasher, permitted_use);
    }
    hash_part(&mut hasher, artifact.rights().note().unwrap_or(""));
    RevisionId::parse(format!("sha256:{:x}", hasher.finalize()))
        .expect("SHA-256 encoder produces a valid RevisionId")
}

fn hash_part(hasher: &mut Sha256, value: &str) {
    hasher.update(value.len().to_le_bytes());
    hasher.update(value.as_bytes());
}

fn digest(hash: &ContentHash) -> &str {
    hash.as_str()
        .strip_prefix("sha256:")
        .expect("ContentHash currently validates the sha256 scheme")
}

fn rights_status(status: RightsStatus) -> &'static str {
    match status {
        RightsStatus::Unknown => "unknown",
        RightsStatus::NotAssessed => "not_assessed",
        RightsStatus::DerivativeOfUnassessedSource => "derivative_of_unassessed_source",
        RightsStatus::Permitted => "permitted",
        RightsStatus::Restricted => "restricted",
        RightsStatus::Prohibited => "prohibited",
    }
}
