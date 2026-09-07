mod content;
pub use content::{ArtifactContentInfo, ContentContext};

use std::{
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use mathscape_artifact::{
    ArtifactImportOutcome, ArtifactMetadata, ArtifactPool, ArtifactPoolError,
};
use mathscape_domain::{ActorRef, MediaType, RightsRecord, RightsStatus, StableId};
use mathscape_graph::{GraphProjectionError, IncidenceGraph, ProjectionLens};
use mathscape_orchestrator::{MarkerPromotionError, promote_marker_inventory};
use mathscape_pdf::{
    ClassificationMode, PdfContentKind, PdfEngine, PdfError, PdfOpenOptions, PdfPageIndex,
    PdfSelection,
};
use mathscape_query::{ArtifactGraphProjectionError, ArtifactGraphProjector};
use mathscape_store_sqlite::{SqliteArtifactStore, SqliteStoreError};
use mathscape_tool_marker::{
    MarkerCancellation, MarkerExecutionError, MarkerInstallation, MarkerProductRole,
    MarkerProductScanError, MarkerProductScanner, MarkerProfile, MarkerRequest, MarkerRequestError,
    MarkerRunStatus, MarkerRunner, MarkerStagingArea,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use thiserror::Error;

const WORKSPACE_ENVIRONMENT_VARIABLE: &str = "MATHSCAPE_WORKSPACE_ROOT";
const MARKER_EXECUTABLE_ENVIRONMENT_VARIABLE: &str = "MATHSCAPE_MARKER_EXECUTABLE";

#[derive(Clone, Debug)]
pub struct DesktopService {
    workspace_root: PathBuf,
    marker: Option<MarkerInstallation>,
    active_parse: Arc<Mutex<Option<ParsePdfRequest>>>,
}

impl DesktopService {
    pub fn open(workspace_root: impl AsRef<Path>) -> Result<Self, DesktopServiceError> {
        Self::open_configured(workspace_root, None)
    }

    pub fn open_with_marker(
        workspace_root: impl AsRef<Path>,
        marker_executable: impl Into<PathBuf>,
    ) -> Result<Self, DesktopServiceError> {
        Self::open_configured(
            workspace_root,
            Some(MarkerInstallation::new(marker_executable.into())?),
        )
    }

    fn open_configured(
        workspace_root: impl AsRef<Path>,
        marker: Option<MarkerInstallation>,
    ) -> Result<Self, DesktopServiceError> {
        let requested = workspace_root.as_ref();
        let pool = if requested.join("workspace.toml").exists() {
            ArtifactPool::open(requested)?
        } else {
            ArtifactPool::create(requested)?
        };
        let workspace_root = pool.workspace().path().to_owned();
        drop(pool);
        Ok(Self {
            workspace_root,
            marker,
            active_parse: Arc::new(Mutex::new(None)),
        })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn snapshot(&self) -> Result<DesktopSnapshot, DesktopServiceError> {
        let pool = ArtifactPool::open(&self.workspace_root)?;
        let workspace_id = pool.workspace().workspace_id().as_str().to_owned();
        drop(pool);

        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let catalog = store.read_catalog_snapshot()?;
        let artifact_count = catalog.artifacts().len();
        let conversion_count = catalog.conversions().len();
        let pdf_artifacts = catalog
            .artifacts()
            .iter()
            .filter_map(|record| {
                let entity = record.entity();
                let artifact = entity.payload();
                (artifact.media_type().as_str() == "application/pdf").then(|| PdfArtifactSummary {
                    artifact_id: entity.header().stable_id().as_str().to_owned(),
                    filename: artifact
                        .original_filename()
                        .unwrap_or("Unnamed PDF")
                        .to_owned(),
                    page_count: artifact.page_count(),
                })
            })
            .collect();
        let projection = ArtifactGraphProjector.project(&catalog, ProjectionLens::Source)?;
        let graph = IncidenceGraph::project(&projection);

        Ok(DesktopSnapshot {
            workspace_id,
            workspace_root: self.workspace_root.display().to_string(),
            artifact_count,
            conversion_count,
            marker: self.marker_status(),
            pdf_artifacts,
            graph,
        })
    }

    pub fn import_pdf(
        &self,
        source_path: impl AsRef<Path>,
    ) -> Result<PdfImportResult, DesktopServiceError> {
        let source_path = source_path.as_ref();
        let filename = source_path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| DesktopServiceError::InvalidFilename(source_path.to_owned()))?
            .to_owned();
        let is_pdf_extension = source_path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("pdf"));
        if !is_pdf_extension {
            return Err(DesktopServiceError::UnsupportedImportType(
                source_path.to_owned(),
            ));
        }

        let classification = PdfEngine.classify(
            source_path,
            &PdfOpenOptions::new(),
            ClassificationMode::Fast,
        )?;
        let metadata = ArtifactMetadata::new(
            None,
            MediaType::parse("application/pdf")?,
            Some(filename.clone()),
            Some(classification.page_count()),
            RightsRecord::new(RightsStatus::NotAssessed, vec![], None)?,
        );
        let actor = ActorRef::parse("mathscape-desktop")?;
        let mut pool = ArtifactPool::open(&self.workspace_root)?;
        let imported = pool.import_file(source_path, metadata, actor)?;
        let outcome = match imported.outcome() {
            ArtifactImportOutcome::Inserted => PdfImportOutcome::Inserted,
            ArtifactImportOutcome::Deduplicated => PdfImportOutcome::Deduplicated,
        };
        let artifact_id = imported.stable_id().as_str().to_owned();
        let content_hash = imported.content_hash().as_str().to_owned();
        let byte_size = imported.byte_size();
        drop(pool);

        Ok(PdfImportResult {
            artifact_id,
            content_hash,
            byte_size,
            filename,
            page_count: classification.page_count(),
            document_title: classification.title().map(str::to_owned),
            content_kind: pdf_content_kind_label(classification.kind()),
            classification_confidence: classification.confidence(),
            sampled_ocr_page_count: classification.ocr_recommendations().len(),
            outcome,
            snapshot: self.snapshot()?,
        })
    }

    pub fn parse_pdf(
        &self,
        request: &ParsePdfRequest,
    ) -> Result<PdfParseResult, DesktopServiceError> {
        let _permit = self.begin_parse(request)?;
        let installation = self
            .marker
            .as_ref()
            .ok_or(DesktopServiceError::MarkerNotConfigured)?;
        let artifact_id = StableId::parse(request.artifact_id.clone())?;
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let catalog = store.read_catalog_snapshot()?;
        let source = catalog
            .artifacts()
            .iter()
            .find(|record| record.entity().header().stable_id() == &artifact_id)
            .ok_or_else(|| DesktopServiceError::UnknownArtifact(artifact_id.clone()))?;
        if source.entity().payload().media_type().as_str() != "application/pdf" {
            return Err(DesktopServiceError::ArtifactIsNotPdf(artifact_id));
        }
        let page_count = source
            .entity()
            .payload()
            .page_count()
            .ok_or_else(|| DesktopServiceError::MissingPdfPageCount(artifact_id.clone()))?;
        if request.first_page == 0
            || request.last_page < request.first_page
            || request.last_page > page_count
        {
            return Err(DesktopServiceError::InvalidParsePageRange {
                first: request.first_page,
                last: request.last_page,
                page_count,
            });
        }
        let selection = PdfSelection::inclusive_range(
            page_count,
            PdfPageIndex::new(request.first_page - 1),
            PdfPageIndex::new(request.last_page - 1),
        )?;
        drop(store);

        let pool = ArtifactPool::open(&self.workspace_root)?;
        let managed_pdf = pool.managed_path(&artifact_id)?;
        let staging = MarkerStagingArea::new(pool.workspace().cache_path().join("marker-runs"))?;
        drop(pool);

        let profile = request.profile.marker_profile();
        let marker_request = MarkerRequest::new(managed_pdf, selection, profile)?;
        let runner = MarkerRunner::default();
        let probe = runner.probe(installation)?;
        let run = runner.execute(
            installation,
            &marker_request,
            &staging,
            &MarkerCancellation::new(),
        )?;
        if run.status() != MarkerRunStatus::Succeeded {
            return Err(DesktopServiceError::MarkerRunFailed {
                status: run.status(),
                staging: run.staging().path().to_owned(),
                diagnostics: bounded_diagnostics(run.stderr().preview_lossy()),
            });
        }
        let elapsed_ms = u64::try_from(run.elapsed().as_millis()).unwrap_or(u64::MAX);
        let inventory = MarkerProductScanner::default().scan(&run)?;
        if !inventory.is_promotable() {
            return Err(DesktopServiceError::MarkerInventoryNotPromotable {
                staging: run.staging().path().to_owned(),
                issues: format!("{:?}", inventory.issues()),
            });
        }

        let mut pool = ArtifactPool::open(&self.workspace_root)?;
        let promotion = promote_marker_inventory(
            &mut pool,
            &run,
            &inventory,
            &artifact_id,
            Some(probe.version().raw()),
            ActorRef::parse("mathscape-desktop")?,
        )?;
        let outputs = promotion
            .outputs()
            .iter()
            .map(|output| ParsedArtifactOutput {
                artifact_id: output.artifact().stable_id().as_str().to_owned(),
                role: marker_product_role_label(output.role()),
                relative_path: output.relative_path().display().to_string(),
                byte_size: output.artifact().byte_size(),
            })
            .collect();
        let conversion_id = promotion.conversion_id().as_str().to_owned();
        drop(pool);
        let cleanup_warning = run.cleanup().err().map(|error| error.to_string());

        Ok(PdfParseResult {
            conversion_id,
            input_artifact_id: artifact_id.as_str().to_owned(),
            first_page: request.first_page,
            last_page: request.last_page,
            profile: request.profile,
            marker_version: probe.version().raw().to_owned(),
            elapsed_ms,
            outputs,
            cleanup_warning,
            snapshot: self.snapshot()?,
        })
    }

    fn marker_status(&self) -> MarkerRuntimeStatus {
        let Some(installation) = self.marker.as_ref() else {
            return MarkerRuntimeStatus {
                configured: false,
                ready: false,
                executable: None,
                version: None,
                problem: Some(format!(
                    "set {MARKER_EXECUTABLE_ENVIRONMENT_VARIABLE} to an absolute marker_single path"
                )),
            };
        };
        match MarkerRunner::default().probe(installation) {
            Ok(probe) => MarkerRuntimeStatus {
                configured: true,
                ready: true,
                executable: Some(probe.executable().display().to_string()),
                version: Some(probe.version().raw().to_owned()),
                problem: None,
            },
            Err(error) => MarkerRuntimeStatus {
                configured: true,
                ready: false,
                executable: Some(installation.executable().display().to_string()),
                version: None,
                problem: Some(error.to_string()),
            },
        }
    }

    pub fn active_parse(&self) -> Option<ParsePdfRequest> {
        self.active_parse
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn begin_parse(&self, request: &ParsePdfRequest) -> Result<ParsePermit, DesktopServiceError> {
        let mut active = self.active_parse.lock().unwrap_or_else(|e| e.into_inner());
        if active.is_some() {
            return Err(DesktopServiceError::ParseAlreadyRunning);
        }
        *active = Some(request.clone());
        Ok(ParsePermit(self.active_parse.clone()))
    }
}

struct ParsePermit(Arc<Mutex<Option<ParsePdfRequest>>>);

#[cfg(test)]
mod task_tests {
    use super::*;

    #[test]
    fn cloned_services_exclude_concurrent_submissions_and_release_on_drop() {
        let service = DesktopService {
            workspace_root: PathBuf::new(),
            marker: None,
            active_parse: Arc::new(Mutex::new(None)),
        };
        let request = ParsePdfRequest::new("test:pdf", 2, 4, MarkerProfileChoice::MathOcrV1);
        let permit = service.begin_parse(&request).unwrap();
        let other = service.clone();
        let submitted = request.clone();
        std::thread::spawn(move || {
            assert_eq!(other.active_parse(), Some(submitted.clone()));
            assert!(matches!(
                other.parse_pdf(&submitted),
                Err(DesktopServiceError::ParseAlreadyRunning)
            ));
        })
        .join()
        .unwrap();
        drop(permit);
        assert_eq!(service.active_parse(), None);
        assert!(matches!(
            service.parse_pdf(&request),
            Err(DesktopServiceError::MarkerNotConfigured)
        ));
        assert_eq!(
            service.active_parse(),
            None,
            "failure must release the reservation"
        );
        assert!(service.begin_parse(&request).is_ok());
    }
}

impl Drop for ParsePermit {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PdfImportOutcome {
    Inserted,
    Deduplicated,
}

#[derive(Debug, Serialize)]
pub struct DesktopSnapshot {
    workspace_id: String,
    workspace_root: String,
    artifact_count: usize,
    conversion_count: usize,
    marker: MarkerRuntimeStatus,
    pdf_artifacts: Vec<PdfArtifactSummary>,
    graph: IncidenceGraph,
}

impl DesktopSnapshot {
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    pub fn artifact_count(&self) -> usize {
        self.artifact_count
    }

    pub fn conversion_count(&self) -> usize {
        self.conversion_count
    }

    pub fn marker(&self) -> &MarkerRuntimeStatus {
        &self.marker
    }

    pub fn pdf_artifacts(&self) -> &[PdfArtifactSummary] {
        &self.pdf_artifacts
    }

    pub fn graph(&self) -> &IncidenceGraph {
        &self.graph
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MarkerRuntimeStatus {
    configured: bool,
    ready: bool,
    executable: Option<String>,
    version: Option<String>,
    problem: Option<String>,
}

impl MarkerRuntimeStatus {
    pub fn is_configured(&self) -> bool {
        self.configured
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PdfArtifactSummary {
    artifact_id: String,
    filename: String,
    page_count: Option<u32>,
}

impl PdfArtifactSummary {
    pub fn artifact_id(&self) -> &str {
        &self.artifact_id
    }

    pub fn page_count(&self) -> Option<u32> {
        self.page_count
    }
}

#[derive(Debug, Serialize)]
pub struct PdfImportResult {
    artifact_id: String,
    content_hash: String,
    byte_size: u64,
    filename: String,
    page_count: u32,
    document_title: Option<String>,
    content_kind: &'static str,
    classification_confidence: f32,
    sampled_ocr_page_count: usize,
    outcome: PdfImportOutcome,
    snapshot: DesktopSnapshot,
}

impl PdfImportResult {
    pub fn artifact_id(&self) -> &str {
        &self.artifact_id
    }

    pub fn page_count(&self) -> u32 {
        self.page_count
    }

    pub fn outcome(&self) -> PdfImportOutcome {
        self.outcome
    }

    pub fn snapshot(&self) -> &DesktopSnapshot {
        &self.snapshot
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerProfileChoice {
    FastNativeV1,
    MathOcrV1,
}

impl MarkerProfileChoice {
    const fn marker_profile(self) -> MarkerProfile {
        match self {
            Self::FastNativeV1 => MarkerProfile::FastNativeV1,
            Self::MathOcrV1 => MarkerProfile::MathOcrV1,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ParsePdfRequest {
    artifact_id: String,
    first_page: u32,
    last_page: u32,
    profile: MarkerProfileChoice,
}

impl ParsePdfRequest {
    pub fn new(
        artifact_id: impl Into<String>,
        first_page: u32,
        last_page: u32,
        profile: MarkerProfileChoice,
    ) -> Self {
        Self {
            artifact_id: artifact_id.into(),
            first_page,
            last_page,
            profile,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ParsedArtifactOutput {
    artifact_id: String,
    role: &'static str,
    relative_path: String,
    byte_size: u64,
}

#[derive(Debug, Serialize)]
pub struct PdfParseResult {
    conversion_id: String,
    input_artifact_id: String,
    first_page: u32,
    last_page: u32,
    profile: MarkerProfileChoice,
    marker_version: String,
    elapsed_ms: u64,
    outputs: Vec<ParsedArtifactOutput>,
    cleanup_warning: Option<String>,
    snapshot: DesktopSnapshot,
}

impl PdfParseResult {
    pub fn conversion_id(&self) -> &str {
        &self.conversion_id
    }

    pub fn outputs(&self) -> &[ParsedArtifactOutput] {
        &self.outputs
    }

    pub fn snapshot(&self) -> &DesktopSnapshot {
        &self.snapshot
    }
}

#[derive(Debug, Error)]
pub enum DesktopServiceError {
    #[error("已有 PDF 解析任务正在运行，请等待完成后再提交")]
    ParseAlreadyRunning,
    #[error("only PDF files can be imported: {0}")]
    UnsupportedImportType(PathBuf),
    #[error("the selected PDF has no valid UTF-8 filename: {0}")]
    InvalidFilename(PathBuf),
    #[error("Marker is not configured; set {MARKER_EXECUTABLE_ENVIRONMENT_VARIABLE}")]
    MarkerNotConfigured,
    #[error("Artifact does not exist in the current Workspace: {0}")]
    UnknownArtifact(StableId),
    #[error("Artifact is not a PDF and cannot be sent to Marker: {0}")]
    ArtifactIsNotPdf(StableId),
    #[error("PDF Artifact has no stored page count: {0}")]
    MissingPdfPageCount(StableId),
    #[error("invalid 1-based PDF page range {first}-{last}; document contains {page_count} pages")]
    InvalidParsePageRange {
        first: u32,
        last: u32,
        page_count: u32,
    },
    #[error("Marker run ended with {status:?}; diagnostics remain at {staging}: {diagnostics}")]
    MarkerRunFailed {
        status: MarkerRunStatus,
        staging: PathBuf,
        diagnostics: String,
    },
    #[error("Marker products are incomplete at {staging}; issues: {issues}")]
    MarkerInventoryNotPromotable { staging: PathBuf, issues: String },
    #[error("Artifact Pool operation failed: {0}")]
    Artifact(#[from] ArtifactPoolError),
    #[error("PDF inspection failed: {0}")]
    Pdf(#[from] PdfError),
    #[error("SQLite catalog read failed: {0}")]
    Store(#[from] SqliteStoreError),
    #[error("Artifact graph projection failed: {0}")]
    Projection(#[from] ArtifactGraphProjectionError),
    #[error("graph projection failed: {0}")]
    Graph(#[from] GraphProjectionError),
    #[error("domain validation failed: {0}")]
    Domain(#[from] mathscape_domain::DomainError),
    #[error("Marker request is invalid: {0}")]
    MarkerRequest(#[from] MarkerRequestError),
    #[error("Marker execution failed: {0}")]
    MarkerExecution(#[from] MarkerExecutionError),
    #[error("Marker product scan failed: {0}")]
    MarkerScan(#[from] MarkerProductScanError),
    #[error("Marker product promotion failed: {0}")]
    MarkerPromotion(#[from] MarkerPromotionError),
}

#[derive(Clone)]
struct DesktopState {
    service: DesktopService,
}

#[tauri::command]
async fn get_desktop_snapshot(state: State<'_, DesktopState>) -> Result<DesktopSnapshot, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.snapshot())
        .await
        .map_err(|error| format!("catalog read worker failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn choose_and_import_pdf(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<Option<PdfImportResult>, String> {
    let selected = app
        .dialog()
        .file()
        .add_filter("PDF document", &["pdf"])
        .blocking_pick_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let source_path = selected
        .into_path()
        .map_err(|error| format!("selected file is not a local path: {error}"))?;
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.import_pdf(source_path))
        .await
        .map_err(|error| format!("PDF import worker failed: {error}"))?
        .map(Some)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_active_parse(state: State<'_, DesktopState>) -> Option<ParsePdfRequest> {
    state.service.active_parse()
}

#[tauri::command]
async fn parse_pdf(
    state: State<'_, DesktopState>,
    request: ParsePdfRequest,
) -> Result<PdfParseResult, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.parse_pdf(&request))
        .await
        .map_err(|error| format!("PDF parse worker failed: {error}"))?
        .map_err(|error| error.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let root = workspace_root(app.handle())?;
            let service = match env::var_os(MARKER_EXECUTABLE_ENVIRONMENT_VARIABLE) {
                Some(path) if !path.is_empty() => DesktopService::open_with_marker(root, path)?,
                _ => DesktopService::open(root)?,
            };
            app.manage(DesktopState { service });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            content::get_markdown_document,
            content::resolve_markdown_selection,
            content::get_artifact_content_info,
            content::read_artifact_bytes,
            get_desktop_snapshot,
            choose_and_import_pdf,
            get_active_parse,
            parse_pdf
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Mathscape desktop application");
}

fn workspace_root(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    match env::var_os(WORKSPACE_ENVIRONMENT_VARIABLE) {
        Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        _ => Ok(app.path().app_data_dir()?.join("workspace")),
    }
}

fn pdf_content_kind_label(kind: PdfContentKind) -> &'static str {
    match kind {
        PdfContentKind::TextBased => "text_based",
        PdfContentKind::Scanned => "scanned",
        PdfContentKind::ImageBased => "image_based",
        PdfContentKind::Mixed => "mixed",
    }
}

fn marker_product_role_label(role: MarkerProductRole) -> &'static str {
    match role {
        MarkerProductRole::PrimaryMarkdown => "primary_markdown",
        MarkerProductRole::MarkdownCandidate => "markdown_candidate",
        MarkerProductRole::Metadata => "metadata",
        MarkerProductRole::Image => "image",
        MarkerProductRole::SupportingFile => "supporting_file",
        MarkerProductRole::DiagnosticLog => "diagnostic_log",
    }
}

fn bounded_diagnostics(value: String) -> String {
    const MAXIMUM_CHARACTERS: usize = 4_000;
    let character_count = value.chars().count();
    if character_count <= MAXIMUM_CHARACTERS {
        return value;
    }
    let tail = value
        .chars()
        .skip(character_count - MAXIMUM_CHARACTERS)
        .collect::<String>();
    format!("…{tail}")
}
