use std::{
    env,
    path::{Path, PathBuf},
};

use mathscape_artifact::{
    ArtifactImportOutcome, ArtifactMetadata, ArtifactPool, ArtifactPoolError,
};
use mathscape_domain::{ActorRef, MediaType, RightsRecord, RightsStatus};
use mathscape_graph::{GraphProjectionError, IncidenceGraph, ProjectionLens};
use mathscape_pdf::{ClassificationMode, PdfContentKind, PdfEngine, PdfError, PdfOpenOptions};
use mathscape_query::{ArtifactGraphProjectionError, ArtifactGraphProjector};
use mathscape_store_sqlite::{SqliteArtifactStore, SqliteStoreError};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use thiserror::Error;

const WORKSPACE_ENVIRONMENT_VARIABLE: &str = "MATHSCAPE_WORKSPACE_ROOT";

#[derive(Clone, Debug)]
pub struct DesktopService {
    workspace_root: PathBuf,
}

impl DesktopService {
    pub fn open(workspace_root: impl AsRef<Path>) -> Result<Self, DesktopServiceError> {
        let requested = workspace_root.as_ref();
        let pool = if requested.join("workspace.toml").exists() {
            ArtifactPool::open(requested)?
        } else {
            ArtifactPool::create(requested)?
        };
        let workspace_root = pool.workspace().path().to_owned();
        drop(pool);
        Ok(Self { workspace_root })
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
        let projection = ArtifactGraphProjector.project(&catalog, ProjectionLens::Source)?;
        let graph = IncidenceGraph::project(&projection);

        Ok(DesktopSnapshot {
            workspace_id,
            workspace_root: self.workspace_root.display().to_string(),
            artifact_count,
            conversion_count,
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

    pub fn graph(&self) -> &IncidenceGraph {
        &self.graph
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

#[derive(Debug, Error)]
pub enum DesktopServiceError {
    #[error("only PDF files can be imported: {0}")]
    UnsupportedImportType(PathBuf),
    #[error("the selected PDF has no valid UTF-8 filename: {0}")]
    InvalidFilename(PathBuf),
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

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let root = workspace_root(app.handle())?;
            let service = DesktopService::open(root)?;
            app.manage(DesktopState { service });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_desktop_snapshot,
            choose_and_import_pdf
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
