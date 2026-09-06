use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_artifact::{ArtifactMetadata, ArtifactPool, ArtifactPoolError, ManagedArtifact};
use mathscape_domain::{
    ActorRef, CommandRecordStatus, ContentHash, ConversionExitStatus, ConversionParameters,
    ConversionRun, EntityHeader, EntityKind, LifecycleStatus, PageIndexBase, PageRange, RevisionId,
    SchemaVersion, StableId, Timestamp,
};
use mathscape_tool_marker::{MarkerProductInventory, MarkerProductRole, MarkerRun};
use sha2::{Digest, Sha256};
use thiserror::Error;

const SCHEMA_VERSION: &str = "mathscape-marker-promotion-1";
static PROMOTION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum MarkerPromotionError {
    #[error("Marker product inventory is not promotable")]
    InventoryNotPromotable,
    #[error("Marker product inventory does not belong to the supplied run")]
    InventoryRunMismatch,
    #[error("Marker product digest is invalid: {0}")]
    InvalidProductDigest(String),
    #[error("artifact promotion failed: {0}")]
    Artifact(#[from] ArtifactPoolError),
    #[error("domain validation failed: {0}")]
    Domain(#[from] mathscape_domain::DomainError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerArtifactPromotion {
    role: MarkerProductRole,
    relative_path: std::path::PathBuf,
    artifact: ManagedArtifact,
}

impl MarkerArtifactPromotion {
    pub const fn role(&self) -> MarkerProductRole {
        self.role
    }

    pub fn relative_path(&self) -> &std::path::Path {
        &self.relative_path
    }

    pub const fn artifact(&self) -> &ManagedArtifact {
        &self.artifact
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerPromotionResult {
    conversion_id: StableId,
    outputs: Vec<MarkerArtifactPromotion>,
}

impl MarkerPromotionResult {
    pub fn conversion_id(&self) -> &StableId {
        &self.conversion_id
    }

    pub fn outputs(&self) -> &[MarkerArtifactPromotion] {
        &self.outputs
    }
}

/// Rehashes and promotes all non-diagnostic Marker products, then records one
/// ConversionRun relating the managed input Artifact to every managed output.
pub fn promote_marker_inventory(
    pool: &mut ArtifactPool,
    run: &MarkerRun,
    inventory: &MarkerProductInventory,
    input_artifact: &StableId,
    marker_version: Option<&str>,
    actor: ActorRef,
) -> Result<MarkerPromotionResult, MarkerPromotionError> {
    if !inventory.is_promotable() {
        return Err(MarkerPromotionError::InventoryNotPromotable);
    }
    if inventory.root() != run.staging().path()
        || inventory.run_status() != run.status()
        || inventory.profile() != run.command().profile()
    {
        return Err(MarkerPromotionError::InventoryRunMismatch);
    }

    let mut outputs = Vec::new();
    for product in inventory.derived_products() {
        let expected_hash =
            ContentHash::parse(product.digest().as_str().to_owned()).map_err(|_| {
                MarkerPromotionError::InvalidProductDigest(product.digest().as_str().to_owned())
            })?;
        let original_filename = product
            .relative_path()
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned);
        let metadata = ArtifactMetadata::derived(
            mathscape_domain::MediaType::parse(product.media_type())?,
            original_filename,
        )?;
        let artifact = pool.import_file_with_expected_hash(
            inventory.source_path(product),
            &expected_hash,
            metadata,
            actor.clone(),
        )?;
        outputs.push(MarkerArtifactPromotion {
            role: product.role(),
            relative_path: product.relative_path().to_owned(),
            artifact,
        });
    }

    let page_range = run.command().page_range();
    let mut additional_options = BTreeMap::new();
    additional_options.insert(
        "marker_profile".to_owned(),
        run.command().profile().id().to_owned(),
    );
    additional_options.insert(
        "marker_mode".to_owned(),
        run.command().profile().mode().cli_value().to_owned(),
    );
    additional_options.insert(
        "product_inventory_schema".to_owned(),
        inventory.schema_version().to_owned(),
    );
    let parameters = ConversionParameters::new(
        Some(PageRange::new(
            page_range.first(),
            page_range.last(),
            PageIndexBase::Zero,
        )?),
        "markdown".to_owned(),
        None,
        additional_options,
    )?;
    let command_template = format!(
        "marker_single {{source_pdf}} --page_range {} --output_format markdown --mode {} --output_dir {{run_root}}",
        page_range.as_str(),
        run.command().profile().mode().cli_value()
    );
    let conversion_id = unique_conversion_id()?;
    let output_ids = outputs
        .iter()
        .map(|output| output.artifact.stable_id().clone())
        .collect::<Vec<_>>();
    let revision_id = conversion_revision(&conversion_id, input_artifact, &output_ids);
    let conversion = ConversionRun::new(
        "marker_single".to_owned(),
        marker_version.map(str::to_owned),
        Some(command_template),
        CommandRecordStatus::Recorded,
        parameters,
        None,
        Some(timestamp(run.started_at())?),
        Some(timestamp(run.finished_at())?),
        ConversionExitStatus::Succeeded,
        vec![],
        None,
    )?
    .into_record(
        EntityHeader::new(
            conversion_id.clone(),
            revision_id,
            EntityKind::ConversionRun,
            SchemaVersion::parse(SCHEMA_VERSION)?,
            LifecycleStatus::Active,
            timestamp(run.finished_at())?,
            actor,
            None,
        ),
        vec![input_artifact.clone()],
        output_ids,
    )?;
    pool.record_conversion(&conversion)?;
    Ok(MarkerPromotionResult {
        conversion_id,
        outputs,
    })
}

fn unique_conversion_id() -> Result<StableId, mathscape_domain::DomainError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = PROMOTION_COUNTER.fetch_add(1, Ordering::Relaxed);
    StableId::parse(format!(
        "mathscape:conversion/marker/{now}-{}-{counter}",
        std::process::id()
    ))
}

fn timestamp(value: SystemTime) -> Result<Timestamp, mathscape_domain::DomainError> {
    let duration = value.duration_since(UNIX_EPOCH).unwrap_or_default();
    Timestamp::parse(format!(
        "unix:{}.{}",
        duration.as_secs(),
        duration.subsec_nanos()
    ))
}

fn conversion_revision(
    conversion_id: &StableId,
    input: &StableId,
    outputs: &[StableId],
) -> RevisionId {
    let mut hasher = Sha256::new();
    hasher.update(conversion_id.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(input.as_str().as_bytes());
    for output in outputs {
        hasher.update([0]);
        hasher.update(output.as_str().as_bytes());
    }
    RevisionId::parse(format!("sha256:{:x}", hasher.finalize()))
        .expect("SHA-256 encoder produces a valid RevisionId")
}
