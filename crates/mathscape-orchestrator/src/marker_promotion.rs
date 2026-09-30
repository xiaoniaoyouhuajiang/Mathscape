use std::{
    collections::BTreeMap,
    fs,
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
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const PRODUCT_PATHS_KEY: &str = "mathscape.product-paths.v1";
pub const MARKDOWN_PDF_MAP_ARTIFACT_KEY: &str = "mathscape.markdown-pdf-map-artifact.v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProductPath {
    pub path: String,
    pub artifact_id: StableId,
}

const SCHEMA_VERSION: &str = "mathscape-marker-promotion-1";
static PROMOTION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum MarkerPromotionError {
    #[error("could not serialize product paths: {0}")]
    ProductPaths(#[from] serde_json::Error),
    #[error("Marker product inventory is not promotable")]
    InventoryNotPromotable,
    #[error("Marker product inventory does not belong to the supplied run")]
    InventoryRunMismatch,
    #[error("Marker product digest is invalid: {0}")]
    InvalidProductDigest(String),
    #[error("artifact promotion failed: {0}")]
    Artifact(#[from] ArtifactPoolError),
    #[error("could not build Markdown-PDF page map: {0}")]
    PageMap(String),
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
    let conversion_id = unique_conversion_id()?;
    let map_output = if let Some(markdown) = outputs
        .iter()
        .find(|output| output.role() == MarkerProductRole::PrimaryMarkdown)
    {
        let source = fs::read_to_string(inventory.root().join(markdown.relative_path()))
            .map_err(|error| MarkerPromotionError::PageMap(error.to_string()))?;
        let map = crate::build_markdown_pdf_block_map(
            markdown.artifact().stable_id().clone(),
            conversion_id.clone(),
            input_artifact.clone(),
            source,
        )
        .map_err(MarkerPromotionError::PageMap)?;
        map.map(|map| (markdown, map))
    } else {
        None
    };
    if let Some((markdown, map)) = map_output {
        let map_name = markdown
            .relative_path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| format!("{stem}_pdf_map.json"))
            .ok_or_else(|| MarkerPromotionError::PageMap("Markdown filename is invalid".into()))?;
        let map_relative_path = markdown
            .relative_path()
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""))
            .join(map_name);
        let artifact = pool.import_bytes(
            &serde_json::to_vec_pretty(&map)?,
            ArtifactMetadata::derived(
                mathscape_domain::MediaType::parse("application/json")?,
                map_relative_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned),
            )?,
            actor.clone(),
        )?;
        outputs.push(MarkerArtifactPromotion {
            role: MarkerProductRole::MarkdownPdfMap,
            relative_path: map_relative_path,
            artifact,
        });
    }
    let mut additional_options = BTreeMap::new();
    let paths: Vec<_> = outputs
        .iter()
        .map(|output| ProductPath {
            path: output
                .relative_path
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
            artifact_id: output.artifact.stable_id().clone(),
        })
        .collect();
    additional_options.insert(PRODUCT_PATHS_KEY.to_owned(), serde_json::to_string(&paths)?);
    if let Some(map) = outputs
        .iter()
        .find(|output| output.role() == MarkerProductRole::MarkdownPdfMap)
    {
        additional_options.insert(
            MARKDOWN_PDF_MAP_ARTIFACT_KEY.to_owned(),
            map.artifact().stable_id().to_string(),
        );
    }
    additional_options.insert(
        "marker_profile".to_owned(),
        run.command().profile().id().to_owned(),
    );
    additional_options.insert(
        "marker_mode".to_owned(),
        run.command().profile().mode().cli_value().to_owned(),
    );
    additional_options.insert("marker_paginate_output".to_owned(), "true".to_owned());
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
        "marker_single {{source_pdf}} --page_range {} --output_format markdown --paginate_output --mode {} --output_dir {{run_root}}",
        page_range.as_str(),
        run.command().profile().mode().cli_value()
    );
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
