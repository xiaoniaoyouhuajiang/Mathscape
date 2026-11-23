mod backlink;
mod canonical_parse;
mod canonical_reader;
mod change_set_anchors;
mod change_sets;
mod content;
mod discovery;
mod discovery_view;
mod external_link;
mod ink;
mod ink_lan;
mod ink_link;
mod ink_problem_card;
pub use ink_link::{
    CreateInkNoteFromContextRequest, InkLinkTargetRequest, InkMarkdownLinks,
    InkMarkdownTargetRequest, LinkInkNoteRequest,
};
pub use ink_problem_card::{InkProblemCard, ProblemCardBodyFormat, ProblemCardStatus};
mod ink_tls;
pub use ink_lan::{
    AddressKind, InkLanMirror, InkLanStatus, InkPairingLink, InkRecoveryImport, InkSetupCheck,
    InkSetupStatus,
};
mod llm_settings;
mod loogle_host;
mod map_edit;
mod markdown_errata;
mod markdown_patch;
mod registration;
mod resident_log;
mod sandbox;
mod sandbox_continuity;
mod sandbox_dialogue;
mod search;
mod semantic;
mod skill_pack;
pub use registration::RegistrationSourceContext;
mod workspace_settings;
pub use backlink::{SourceBacklink, SourceBacklinkRequest};
pub use canonical_parse::{CanonicalParseLeases, ParseCanonicalPagesRequest};
pub use canonical_reader::{CanonicalReaderPiece, CanonicalReaderProjection};
pub use content::{ArtifactContentInfo, ContentContext, SourceNodeDetail};
pub use discovery::{
    AcceptDiscoveryRequest, ActiveDiscoveryPhase, ActiveDiscoveryTask, DesktopDiscoveryAcceptance,
    DesktopDiscoveryAttemptDiagnostic, DesktopDiscoveryError, DesktopDiscoveryOutcome,
    DesktopDiscoveryProgress, DesktopDiscoveryReviewEnvelope, DesktopDiscoveryRun,
    DesktopDiscoveryRunDiagnostics, DesktopDiscoveryRunSummary,
    DesktopDiscoveryValidationDiagnostic, DiscardDiscoveryRunRequest, ListDiscoveryRunsRequest,
    RestoreDiscoveryReviewRequest, ResumeDiscoveryRequest, RunDiscoveryRequest,
};
pub use discovery_view::{DESKTOP_DISCOVERY_VIEW_VERSION, DesktopDiscoveryView};
pub use ink::{
    AppendInkOperationRequest, CreateInkDocumentRequest, CreateInkSnapshotRequest, InkCommandError,
    InkTrashImpactView, StartInkSessionRequest,
};
pub use llm_settings::{
    LlmConnectionTest, LlmProfileView, LlmSettingsError, LlmSettingsSnapshot, SaveLlmProfileRequest,
};
pub use markdown_errata::{
    RevertMarkdownErrataResponse, SaveMarkdownErrataRequest, SaveMarkdownErrataResponse,
};
pub use markdown_patch::{
    DesktopMarkdownPatchPreview, DesktopMarkdownPatchReview, PreviewMarkdownPatchRequest,
    RunMarkdownPatchRequest, SubmitMarkdownPatchDecisionRequest,
};
pub use workspace_settings::{
    DiscoveryAdmissionPreference, SaveWorkspaceSettingsRequest, WORKSPACE_SETTINGS_VERSION,
    WorkspaceSettings, WorkspaceSettingsError,
};

use std::{
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use mathscape_artifact::{
    ArtifactImportOutcome, ArtifactMetadata, ArtifactPool, ArtifactPoolError,
};
use mathscape_domain::{ActorRef, MediaType, RightsRecord, RightsStatus, StableId};
use mathscape_graph::IncidenceGraph;
use mathscape_llm::{CredentialStore, KeyringCredentialStore};
use mathscape_orchestrator::{MarkerPromotionError, promote_marker_inventory};
use mathscape_pdf::{
    ClassificationMode, PdfContentKind, PdfEngine, PdfError, PdfOpenOptions, PdfPageIndex,
    PdfSelection,
};
use mathscape_query::{LandscapeProjectionInput, LandscapeProjector, SourceGraphProjectionError};
use mathscape_store_sqlite::{SqliteArtifactStore, SqliteStoreError};
use mathscape_tool_lean::{
    LEAN_PROFILE_ROOT_ENV, LeanFingerprintStatus, LeanProfileRootSource, LeanProfileState,
    discover_profile_root, list_profiles,
};
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
const KEYRING_SERVICE: &str = "app.mathscape.desktop.llm";

#[derive(Clone, Debug)]
pub struct DesktopService {
    workspace_root: PathBuf,
    marker: Option<MarkerInstallation>,
    credentials: Arc<dyn CredentialStore>,
    active_parse: Arc<Mutex<Option<ParsePdfRequest>>>,
    active_discovery: Arc<Mutex<Option<discovery::ActiveDiscoveryControl>>>,
    ink_lan: Arc<ink_lan::InkLanRuntime>,
    /// Investigation of the bound workspace (`sandbox:{session}` inside an
    /// in-window sandbox, `None` on the mother workspace). Server-owned:
    /// the frontend never builds it.
    investigation_id: Option<String>,
    /// UI language, stamped from [`DesktopState`] on each command.
    display_lang: mathscape_domain::DisplayLang,
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
        let credentials = Arc::new(KeyringCredentialStore::new(KEYRING_SERVICE)?);
        Self::open_with_components(workspace_root, marker, credentials)
    }

    pub fn open_with_credential_store(
        workspace_root: impl AsRef<Path>,
        credentials: Arc<dyn CredentialStore>,
    ) -> Result<Self, DesktopServiceError> {
        Self::open_with_components(workspace_root, None, credentials)
    }

    fn open_with_components(
        workspace_root: impl AsRef<Path>,
        marker: Option<MarkerInstallation>,
        credentials: Arc<dyn CredentialStore>,
    ) -> Result<Self, DesktopServiceError> {
        let requested = workspace_root.as_ref();
        let pool = if requested.join("workspace.toml").exists() {
            ArtifactPool::prepare(requested)?
        } else {
            ArtifactPool::create(requested)?
        };
        let workspace_root = pool.workspace().path().to_owned();
        drop(pool);
        // Pool creation/preparation has completed all schema maintenance before
        // independent frontend commands can connect to this workspace.
        Ok(Self {
            workspace_root,
            marker,
            credentials,
            active_parse: Arc::new(Mutex::new(None)),
            active_discovery: Arc::new(Mutex::new(None)),
            ink_lan: Arc::new(ink_lan::InkLanRuntime::default()),
            investigation_id: None,
            display_lang: Default::default(),
        })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn investigation_id(&self) -> Option<&str> {
        self.investigation_id.as_deref()
    }

    /// Same service bound to `investigation_id` (map overlay + review).
    pub fn with_investigation(mut self, investigation_id: Option<String>) -> Self {
        self.investigation_id = investigation_id;
        self
    }

    /// Opens another workspace with the same Marker install and credential
    /// store, but fresh per-workspace runtime state (parse/discovery slots,
    /// InkPad LAN runtime). Used for the S1 in-window sandbox switch.
    pub fn reopen_at(&self, workspace_root: impl AsRef<Path>) -> Result<Self, DesktopServiceError> {
        Self::open_with_components(
            workspace_root,
            self.marker.clone(),
            Arc::clone(&self.credentials),
        )
    }

    pub fn snapshot(&self) -> Result<DesktopSnapshot, DesktopServiceError> {
        let pool = ArtifactPool::open(&self.workspace_root)?;
        let workspace_id = pool.workspace().workspace_id().as_str().to_owned();
        drop(pool);

        let mut store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        // Silent normalization: keep the canonical markdown registry in step
        // with the catalog so every lineage folds on the map / in search with
        // no button. A failure must not block opening the sandbox.
        if let Err(error) = store.sync_canonical_markdown_if_stale(&canonical_sync_timestamp()) {
            eprintln!("mathscape: canonical markdown registry sync skipped: {error}");
        }
        // Inside a sandbox the map shows the single investigation projection
        // (accepted graph + 「未审批」 staged objects), the same one MCP
        // `graph.*` uses with scope=investigation.
        let (landscape, investigation) = match &self.investigation_id {
            Some(investigation_id) => {
                let (landscape, marks) = mathscape_mcp::investigation::investigation_landscape(
                    &store,
                    investigation_id,
                )?;
                (
                    landscape,
                    Some(InvestigationSnapshot {
                        investigation_id: investigation_id.clone(),
                        marks,
                    }),
                )
            }
            None => (store.landscape_snapshot()?, None),
        };
        let catalog = landscape.artifacts();
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
        let landscape_projection = LandscapeProjector.project(
            LandscapeProjectionInput::from_snapshot(&landscape)
                .with_display_lang(self.display_lang),
        )?;
        let landscape_graph = IncidenceGraph::project(&landscape_projection);

        Ok(DesktopSnapshot {
            workspace_id,
            workspace_root: self.workspace_root.display().to_string(),
            artifact_count,
            conversion_count,
            marker: self.marker_status(),
            lean: lean_runtime_status(&self.workspace_root),
            pdf_artifacts,
            landscape_graph,
            investigation,
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
        // A reader's 「解析这几页」 or an agent's read holds a lease while its
        // Marker runs; this parse would start a second one beside it.
        if !store
            .active_canonical_ensures(mathscape_store_sqlite::unix_millis_now())?
            .is_empty()
        {
            return Err(DesktopServiceError::ParseAlreadyRunning);
        }
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
            credentials: Arc::new(mathscape_llm::MemoryCredentialResolver::default()),
            active_parse: Arc::new(Mutex::new(None)),
            active_discovery: Arc::new(Mutex::new(None)),
            ink_lan: Arc::new(ink_lan::InkLanRuntime::default()),
            investigation_id: None,
            display_lang: Default::default(),
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
    lean: LeanRuntimeStatus,
    pdf_artifacts: Vec<PdfArtifactSummary>,
    landscape_graph: IncidenceGraph,
    /// Present inside a sandbox: which nodes/relations of `landscape_graph`
    /// are staged (「未审批」) and which accepted objects have competing
    /// staged replacements.
    investigation: Option<InvestigationSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InvestigationSnapshot {
    investigation_id: String,
    #[serde(flatten)]
    marks: mathscape_mcp::investigation::InvestigationMarks,
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

    pub fn landscape_graph(&self) -> &IncidenceGraph {
        &self.landscape_graph
    }
}

/// Lean profiles visible to this workspace, shown in the source layer as an
/// environment / external library. Read-only: reads `profile.json` records
/// and never creates, builds, or re-probes a profile.
#[derive(Clone, Debug, Serialize)]
pub struct LeanRuntimeStatus {
    root: Option<String>,
    source: Option<LeanProfileRootSource>,
    root_exists: bool,
    profiles: Vec<LeanProfileSummary>,
    problem: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LeanProfileSummary {
    id: String,
    state: Option<LeanProfileState>,
    lean_toolchain: Option<String>,
    mathlib_reference: Option<String>,
    detected_mathlib_revision: Option<String>,
    fingerprint_status: Option<LeanFingerprintStatus>,
    fingerprint_digest: Option<String>,
    problem: Option<String>,
}

impl LeanRuntimeStatus {
    pub fn profiles(&self) -> &[LeanProfileSummary] {
        &self.profiles
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }
}

impl LeanProfileSummary {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn state(&self) -> Option<LeanProfileState> {
        self.state
    }
}

fn lean_runtime_status(workspace_root: &Path) -> LeanRuntimeStatus {
    let Some(found) = discover_profile_root(Some(workspace_root)) else {
        return LeanRuntimeStatus {
            root: None,
            source: None,
            root_exists: false,
            profiles: Vec::new(),
            problem: Some(format!(
                "未找到 Lean profile 目录（可设置 {LEAN_PROFILE_ROOT_ENV}）"
            )),
        };
    };
    let root_exists = found.root.is_dir();
    let (profiles, problem) = if root_exists {
        match list_profiles(&found.root) {
            Ok(list) => (
                list.into_iter()
                    .map(|profile| {
                        let meta = profile.metadata.as_ref();
                        LeanProfileSummary {
                            id: profile.id,
                            state: meta.map(|m| m.state),
                            lean_toolchain: meta.map(|m| m.lean_toolchain.clone()),
                            mathlib_reference: meta.map(|m| m.mathlib_reference.clone()),
                            detected_mathlib_revision: meta
                                .and_then(|m| m.detected_mathlib_revision.clone()),
                            fingerprint_status: meta.map(|m| m.fingerprint.status),
                            fingerprint_digest: meta.map(|m| m.fingerprint.digest.clone()),
                            problem: profile.problem,
                        }
                    })
                    .collect(),
                None,
            ),
            Err(error) => (Vec::new(), Some(error.to_string())),
        }
    } else {
        (Vec::new(), Some("Lean profile 目录不存在".to_owned()))
    };
    LeanRuntimeStatus {
        root: Some(found.root.display().to_string()),
        source: Some(found.source),
        root_exists,
        profiles,
        problem,
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
    #[error("Source graph projection failed: {0}")]
    SourceGraph(#[from] SourceGraphProjectionError),
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
    #[error("LLM credential store failed: {0}")]
    Credential(#[from] mathscape_llm::CredentialError),
}

/// The workspace this window is bound to. S1 lets the window switch into a
/// sandbox session's isolated copy and back to the workspace it was launched
/// with ("mother"), so commands read the current service through
/// [`DesktopState::service`] instead of holding one for the app's lifetime.
#[derive(Clone)]
struct DesktopState {
    slot: Arc<std::sync::RwLock<WorkspaceSlot>>,
    mother_root: PathBuf,
    /// UI language (0 = zh, 1 = en), set by `set_display_locale`.
    display_lang: Arc<std::sync::atomic::AtomicU8>,
}

struct WorkspaceSlot {
    service: DesktopService,
    sandbox: Option<sandbox::InWindowSandbox>,
}

impl DesktopState {
    fn new(service: DesktopService) -> Self {
        let mother_root = service.workspace_root().to_path_buf();
        Self {
            slot: Arc::new(std::sync::RwLock::new(WorkspaceSlot {
                service,
                sandbox: None,
            })),
            mother_root,
            display_lang: Default::default(),
        }
    }

    /// Cheap clone of the currently bound service (all fields are `Arc`s).
    fn service(&self) -> DesktopService {
        let mut service = self
            .slot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .service
            .clone();
        service.display_lang = self.display_lang();
        service
    }

    fn display_lang(&self) -> mathscape_domain::DisplayLang {
        match self.display_lang.load(std::sync::atomic::Ordering::Relaxed) {
            1 => mathscape_domain::DisplayLang::En,
            _ => mathscape_domain::DisplayLang::Zh,
        }
    }

    fn in_window_sandbox(&self) -> Option<sandbox::InWindowSandbox> {
        self.slot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sandbox
            .clone()
    }

    fn mother_root(&self) -> &Path {
        &self.mother_root
    }

    /// Rebinds the window to `root`. Refuses while a parse or discovery run
    /// is in flight (they hold the old workspace), stops the old workspace's
    /// InkPad LAN service, and leaves the old workspace's files untouched.
    async fn switch_to(
        &self,
        root: PathBuf,
        sandbox: Option<sandbox::InWindowSandbox>,
    ) -> Result<(), String> {
        let current = self.service();
        if current.active_parse().is_some() {
            return Err("当前工作区有 PDF 解析正在进行，请等它结束后再切换".into());
        }
        if current.active_discovery().is_some() {
            return Err("当前工作区有内容发现正在进行，请等它结束或取消后再切换".into());
        }
        let opener = current.clone();
        let next = tauri::async_runtime::spawn_blocking(move || opener.reopen_at(root))
            .await
            .map_err(|error| format!("workspace switch worker failed: {error}"))?
            .map_err(|error| format!("无法打开目标工作区：{error}"))?;
        current.shutdown_ink_lan(false).await.ok();
        let returning_home = sandbox.is_none();
        {
            let mut slot = self
                .slot
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let investigation = sandbox.as_ref().map(|current| {
                mathscape_mcp::investigation::sandbox_investigation_id(&current.sandbox_session_id)
            });
            let next_bound = next.clone().with_investigation(investigation);
            slot.service = next_bound;
            slot.sandbox = sandbox;
        }
        if returning_home {
            // Same resume rule as app start; sandbox copies never auto-start
            // the LAN service even if their checkpoint had it enabled.
            tauri::async_runtime::spawn(async move {
                if next
                    .ink_lan_status()
                    .await
                    .is_ok_and(|status| status.enabled)
                {
                    let _ = next.start_ink_lan(None).await;
                }
            });
        }
        Ok(())
    }
}

/// The UI locale, so names the backend picks for shared objects match it.
#[tauri::command]
fn set_display_locale(locale: String, state: State<'_, DesktopState>) {
    if let Some(lang) = mathscape_domain::DisplayLang::from_tag(&locale) {
        let value = u8::from(lang == mathscape_domain::DisplayLang::En);
        state
            .display_lang
            .store(value, std::sync::atomic::Ordering::Relaxed);
    }
}

#[tauri::command]
async fn get_desktop_snapshot(state: State<'_, DesktopState>) -> Result<DesktopSnapshot, String> {
    let service = state.service();
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
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.import_pdf(source_path))
        .await
        .map_err(|error| format!("PDF import worker failed: {error}"))?
        .map(Some)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_active_parse(state: State<'_, DesktopState>) -> Option<ParsePdfRequest> {
    state.service().active_parse()
}

#[tauri::command]
async fn parse_pdf(
    state: State<'_, DesktopState>,
    request: ParsePdfRequest,
) -> Result<PdfParseResult, String> {
    let service = state.service();
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
            let mother = root.clone();
            let service = match env::var_os(MARKER_EXECUTABLE_ENVIRONMENT_VARIABLE) {
                Some(path) if !path.is_empty() => DesktopService::open_with_marker(root, path)?,
                _ => DesktopService::open(root)?,
            };
            let resume = service.clone();
            tauri::async_runtime::spawn(async move {
                if resume
                    .ink_lan_status()
                    .await
                    .is_ok_and(|status| status.enabled)
                {
                    let _ = resume.start_ink_lan(None).await;
                }
            });
            // Resident Loogle: bind the socket now, start Loogle lazily on the
            // first mathlib.search. A failure only disables resident mode.
            let loogle_resident = service
                .workspace_settings()
                .map(|settings| settings.loogle_resident())
                .unwrap_or(true);
            let log_dir = app.path().app_log_dir().ok();
            if let Err(error) = loogle_host::start(log_dir.as_deref(), loogle_resident) {
                eprintln!("mathscape: {error}");
            }
            app.manage(DesktopState::new(service));
            app.manage(sandbox_dialogue::SandboxDialogueState::from_environment(
                resident_log::log_file(log_dir.as_deref()),
            ));
            let legacy_sandbox = sandbox::legacy_sandbox_root(app.handle());
            app.manage(sandbox::open_for_workspace(&mother, legacy_sandbox)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            content::get_markdown_document,
            content::resolve_markdown_selection,
            content::prepare_discovery_input,
            content::get_artifact_content_info,
            content::get_source_node_detail,
            content::get_markdown_unit_markers,
            content::read_artifact_bytes,
            canonical_reader::get_canonical_reader_projection,
            canonical_parse::parse_canonical_pages,
            canonical_parse::get_canonical_parse_leases,
            change_sets::get_change_set_investigation,
            change_sets::list_pending_change_sets,
            change_sets::get_change_set_snapshot,
            change_sets::preview_change_set_impact,
            change_sets::plan_change_set_batch,
            change_sets::accept_change_set_batch,
            change_sets::discard_change_set_batch,
            backlink::get_source_backlink,
            llm_settings::get_llm_settings,
            llm_settings::set_default_llm_profile,
            llm_settings::save_llm_profile,
            llm_settings::test_llm_profile,
            workspace_settings::get_workspace_settings,
            workspace_settings::save_workspace_settings,
            set_display_locale,
            loogle_host::get_loogle_resident_status,
            discovery::get_active_discovery,
            discovery::list_discovery_runs,
            discovery::get_discovery_review,
            discovery::get_discovery_diagnostics,
            discovery::discard_discovery_run,
            discovery::restore_discovery_review,
            registration::list_discovery_registrations,
            registration::get_map_head,
            registration::get_pending_source_registration_plans,
            registration::plan_discovery_registration_reversal,
            registration::commit_discovery_registration_reversal,
            registration::get_discovery_registration_reversal,
            registration::get_registration_history,
            registration::get_registration_object_detail,
            registration::get_source_registration_context,
            registration::plan_source_registration_commit,
            registration::commit_source_registration_commit,
            discovery::cancel_discovery,
            discovery::accept_discovery_result,
            discovery::run_discovery,
            discovery::resume_discovery_after_patch,
            markdown_errata::list_markdown_errata,
            markdown_errata::list_markdown_errata_history,
            markdown_errata::save_markdown_errata,
            markdown_errata::revert_markdown_errata,
            markdown_errata::mark_errata_unit_reviewed,
            markdown_errata::list_unreviewed_errata_units,
            markdown_patch::run_markdown_patch,
            markdown_patch::get_markdown_patch_review,
            markdown_patch::get_pending_markdown_patch_review,
            markdown_patch::preview_markdown_patch,
            markdown_patch::submit_markdown_patch_decision,
            map_edit::get_map_object_edit_context,
            map_edit::plan_map_change,
            map_edit::commit_map_change,
            map_edit::recheck_mathlib_baselines,
            semantic::get_semantic_object,
            semantic::semantic_learning_view,
            semantic::set_study_marks,
            external_link::open_external_link,
            get_desktop_snapshot,
            choose_and_import_pdf,
            get_active_parse,
            parse_pdf,
            ink::create_ink_document,
            ink::start_ink_session,
            ink::append_ink_operation,
            ink::create_ink_snapshot,
            ink::list_ink_snapshots,
            ink::get_ink_snapshot,
            ink::get_ink_snapshot_for_artifact,
            ink::register_ink_evidence,
            ink::list_ink_evidence,
            ink::get_ink_document,
            ink::list_ink_documents,
            ink::list_ink_sessions,
            ink::close_ink_session,
            ink::get_ink_note,
            ink::get_ink_trash_impact,
            ink::list_trashed_ink_documents,
            ink::rename_ink_note,
            ink::rename_ink_snapshot,
            search::search_workspace,
            ink::trash_ink_note,
            ink::restore_ink_note,
            ink::clean_blank_ink_notes,
            ink_link::create_ink_note_from_context,
            ink_link::link_ink_note,
            ink_link::unlink_ink_note,
            ink_link::set_ink_note_primary_link,
            ink_link::list_ink_note_links,
            ink_link::list_ink_links_to_target,
            ink_link::get_ink_markdown_links,
            ink_link::get_ink_link_overlay,
            ink_link::export_ink_unsaved_writing,
            ink_link::export_ink_file,
            ink_lan::start_ink_lan,
            ink_lan::stop_ink_lan,
            ink_lan::get_ink_lan_status,
            ink_lan::create_ink_pairing,
            ink_lan::push_ink_to_tablet,
            ink_lan::set_ink_lan_preferred_host,
            ink_lan::cancel_ink_pairing,
            ink_lan::get_ink_lan_mirror,
            ink_lan::reclaim_ink_writer,
            ink_lan::export_ink_trust_certificate,
            ink_lan::choose_ink_recovery,
            ink_lan::start_ink_setup,
            ink_lan::get_ink_setup_status,
            ink_lan::prepare_ink_setup_handoff,
            ink_lan::cancel_ink_setup,
            sandbox::sandbox_overview,
            sandbox::sandbox_list_checkpoints,
            sandbox::sandbox_register_checkpoint,
            sandbox::sandbox_unregister_checkpoint,
            sandbox::sandbox_start_session,
            sandbox::sandbox_list_sessions,
            sandbox::sandbox_discard_session,
            sandbox::sandbox_export_session,
            sandbox::sandbox_workspace_status,
            sandbox::sandbox_enter_session,
            sandbox::sandbox_return_to_mother,
            sandbox_dialogue::sandbox_agent_status,
            sandbox_dialogue::sandbox_set_model,
            sandbox_dialogue::sandbox_set_effort,
            sandbox_dialogue::sandbox_session_dialogue,
            sandbox_dialogue::sandbox_new_dialogue_thread,
            sandbox_dialogue::sandbox_activate_dialogue_thread,
            sandbox_dialogue::sandbox_send_message,
            sandbox_dialogue::sandbox_cancel_round
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Mathscape desktop application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                loogle_host::shutdown();
                // Stop every sandbox session's Codex app-server (whole group).
                if let Some(dialogue) = app.try_state::<sandbox_dialogue::SandboxDialogueState>() {
                    dialogue.close_all();
                }
                if let Some(state) = app.try_state::<DesktopState>() {
                    tauri::async_runtime::block_on(state.service().shutdown_ink_lan(false)).ok();
                }
            }
        });
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
        MarkerProductRole::MarkdownPdfMap => "markdown_pdf_map",
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

fn canonical_sync_timestamp() -> mathscape_domain::Timestamp {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    mathscape_domain::Timestamp::parse(format!(
        "unix:{}.{}",
        duration.as_secs(),
        duration.subsec_nanos()
    ))
    .expect("unix timestamp")
}
