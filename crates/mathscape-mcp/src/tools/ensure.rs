//! Canonical markdown ensure: the internal path behind `material.read` on a
//! scanned PDF. Not an MCP tool.
//!
//! Order: plan → probe Marker → claim lease (per-PDF lock + global slot +
//! rolling page budget, one SQLite `BEGIN IMMEDIATE`) → Marker on one
//! contiguous range → validate inventory → promote new immutable artifacts →
//! scoped registry sync for this PDF → terminal receipt. Artifact bytes are
//! never rewritten; a new snapshot is a new content-addressed artifact.
//!
//! No stitching: a request is served only by one snapshot whose pages fully
//! contain it, so the Marker range is the whole request, never just its gaps.
//! The gaps are still reported as `missing`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use mathscape_agent::{
    Capability, CapabilityProbe, CapabilityState, locate_marker_executable, probe_capability,
};
use mathscape_artifact::ArtifactPool;
use mathscape_domain::{
    ActorRef, CanonicalMarkdownKey, CanonicalMarkdownView, InclusivePageRange, StableId, Timestamp,
    merge_page_ranges, missing_pages, parser_policy_from_tool_version,
};
use mathscape_orchestrator::promote_marker_inventory;
use mathscape_pdf::{PdfPageIndex, PdfSelection};
use mathscape_store_sqlite::{
    CanonicalEnsureClaim, CanonicalEnsureClaimOutcome, CanonicalEnsureLease, CanonicalEnsureLimits,
    CanonicalEnsureOutcome, SqliteArtifactStore, unix_millis_now,
};
use mathscape_tool_marker::{
    MarkerCancellation, MarkerExecutionOptions, MarkerInstallation, MarkerProductRole,
    MarkerProductScanner, MarkerProfile, MarkerRequest, MarkerRunStatus, MarkerRunner,
    MarkerStagingArea,
};
use serde::Serialize;

use crate::McpWorkspace;

/// Bumped when the ensure planning rules change; part of the idempotency key.
pub const ENSURE_POLICY_VERSION: &str = "ensure-v1";
const ENSURE_PROFILE: MarkerProfile = MarkerProfile::MathOcrV1;
const ENSURE_ACTOR: &str = "mathscape-mcp-ensure";

/// Budgets, locks and waits for ensure. Defaults are conservative: one Marker
/// at a time per workspace, at most 12 pages per request, 96 pages per hour.
#[derive(Clone, Copy, Debug)]
pub struct EnsurePolicy {
    /// Largest request (pages) a single read may ensure.
    pub max_pages_per_request: u32,
    /// Rolling page budget across all ensures in `budget_window`.
    pub window_pages: u32,
    pub budget_window: Duration,
    /// Global Marker semaphore (live ensures per workspace).
    pub max_running: u32,
    /// How long `material.read` waits for Marker before answering `started`.
    pub wait: Duration,
    /// Marker process timeout. The lease lasts this plus `lease_margin`.
    pub marker_timeout: Duration,
    pub lease_margin: Duration,
}

impl Default for EnsurePolicy {
    fn default() -> Self {
        Self {
            max_pages_per_request: 12,
            window_pages: 96,
            budget_window: Duration::from_secs(60 * 60),
            max_running: 1,
            wait: Duration::from_secs(50),
            marker_timeout: Duration::from_secs(30 * 60),
            lease_margin: Duration::from_secs(5 * 60),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EnsureStatus {
    /// An existing snapshot already contains the request. Marker not run.
    Reused,
    /// Marker ran and a new snapshot containing the request was promoted.
    Expanded,
    /// Marker was launched but is still running after the wait.
    Started,
    /// The request or the rolling budget is too large.
    BlockedBudget,
    /// This PDF (or every global Marker slot) is busy with another ensure.
    BlockedLock,
    /// Marker is not installed or not usable.
    CapabilityUnavailable,
    /// Marker, inventory validation or promotion failed.
    Failed,
}

/// Structured ensure outcome, embedded in `material.read` `details.ensure`.
#[derive(Clone, Debug, Serialize)]
pub(super) struct EnsureReport {
    pub status: EnsureStatus,
    pub requested: InclusivePageRange,
    /// Requested pages no snapshot covers (union of every lineage).
    pub missing: Vec<InclusivePageRange>,
    /// The one contiguous range given to Marker (always the whole request).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker_range: Option<InclusivePageRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ensure_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversion_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown_artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease: Option<CanonicalEnsureLease>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_pages_per_request: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<CapabilityProbe>,
    pub message: String,
}

impl EnsureReport {
    fn new(status: EnsureStatus, plan: &EnsurePlan, message: impl Into<String>) -> Self {
        Self {
            status,
            requested: plan.requested,
            missing: plan.missing.clone(),
            marker_range: None,
            ensure_id: None,
            idempotency_key: None,
            canonical_id: None,
            conversion_id: None,
            markdown_artifact_id: None,
            lease: None,
            max_pages_per_request: None,
            probe: None,
            message: message.into(),
        }
    }

    /// The 「解析中」 map/tool signal while Marker runs for this PDF.
    pub(super) fn map_signal(&self, pdf: &StableId) -> Option<serde_json::Value> {
        let pages = match (self.status, &self.lease) {
            (EnsureStatus::Started, _) => self.marker_range?,
            (EnsureStatus::BlockedLock, Some(lease)) if &lease.source_pdf_artifact_id == pdf => {
                lease.pages
            }
            _ => return None,
        };
        Some(serde_json::json!({
            "kind": "parsing",
            "label": "解析中",
            "node_id": pdf.as_str(),
            "pages": pages,
        }))
    }
}

/// Pure planning result for one request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EnsurePlan {
    pub requested: InclusivePageRange,
    pub missing: Vec<InclusivePageRange>,
    pub action: PlanAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PlanAction {
    /// A single existing segment contains the request.
    Reuse,
    /// Run Marker once on this contiguous range.
    Run(InclusivePageRange),
    /// The request exceeds the per-request page budget.
    OverBudget,
}

/// Plans an ensure against every canonical lineage of the PDF.
pub(super) fn plan_ensure(
    views: &[&CanonicalMarkdownView],
    requested: InclusivePageRange,
    total_pages: u32,
    max_pages_per_request: u32,
) -> EnsurePlan {
    let covered = merge_page_ranges(
        views
            .iter()
            .flat_map(|view| view.segments.iter().filter_map(|segment| segment.pages)),
    );
    let missing = missing_pages(total_pages, &covered)
        .into_iter()
        .filter_map(|gap| {
            let from = gap.from.max(requested.from);
            let to = gap.to.min(requested.to);
            InclusivePageRange::new(from, to).ok()
        })
        .collect();
    let contained = views.iter().any(|view| {
        view.segments
            .iter()
            .any(|segment| segment.pages.is_some_and(|pages| pages.contains(requested)))
    });
    let action = if contained {
        PlanAction::Reuse
    } else if requested.page_count() > max_pages_per_request {
        PlanAction::OverBudget
    } else {
        PlanAction::Run(requested)
    };
    EnsurePlan {
        requested,
        missing,
        action,
    }
}

/// `(pdf, canonical key, normalized range, profile, policy)`.
pub(super) fn idempotency_key(key: &CanonicalMarkdownKey, range: InclusivePageRange) -> String {
    format!(
        "{}|{}|{}-{}|{}|{}|{ENSURE_POLICY_VERSION}",
        key.source_pdf_artifact_id(),
        key.canonical_id(),
        range.from,
        range.to,
        key.parser_profile(),
        key.parser_policy_version(),
    )
}

/// Runs ensure for one PDF request. Never returns a call error for Marker
/// problems; they become `capability_unavailable` or `failed`.
pub(super) fn ensure(
    workspace: &McpWorkspace,
    pdf: &StableId,
    pdf_path: &Path,
    views: &[&CanonicalMarkdownView],
    requested: InclusivePageRange,
    total_pages: u32,
) -> EnsureReport {
    let policy = workspace.ensure_policy();
    let plan = plan_ensure(views, requested, total_pages, policy.max_pages_per_request);
    let range = match plan.action {
        PlanAction::Reuse => {
            return EnsureReport::new(
                EnsureStatus::Reused,
                &plan,
                "an existing immutable snapshot contains every requested page",
            );
        }
        PlanAction::OverBudget => {
            let mut report = EnsureReport::new(
                EnsureStatus::BlockedBudget,
                &plan,
                format!(
                    "request spans {} pages; ensure parses at most {} pages per read. Narrow pdf_pages",
                    requested.page_count(),
                    policy.max_pages_per_request
                ),
            );
            report.max_pages_per_request = Some(policy.max_pages_per_request);
            return report;
        }
        PlanAction::Run(range) => range,
    };

    let probe = probe_capability(workspace.capability_runner(), Capability::Marker);
    let unavailable = |message: String, probe: CapabilityProbe| {
        let mut report = EnsureReport::new(EnsureStatus::CapabilityUnavailable, &plan, message);
        report.probe = Some(probe);
        report
    };
    if probe.state != CapabilityState::Available {
        return unavailable(
            "Marker is not installed or its version is unsupported".into(),
            probe,
        );
    }
    let installation = match locate_marker_executable(workspace.capability_runner())
        .map(absolute)
        .ok_or_else(|| "marker_single was not found".to_owned())
        .and_then(|exe| installation_for(&exe))
    {
        Ok(installation) => installation,
        Err(message) => return unavailable(message, probe),
    };
    let marker_version = match MarkerRunner::default().probe(&installation) {
        Ok(found) => found.version().raw().to_owned(),
        Err(error) => return unavailable(format!("Marker probe failed: {error}"), probe),
    };

    let key = match CanonicalMarkdownKey::new(
        pdf.clone(),
        ENSURE_PROFILE.id(),
        parser_policy_from_tool_version(Some(&marker_version)),
    ) {
        Ok(key) => key,
        Err(error) => {
            return EnsureReport::new(EnsureStatus::Failed, &plan, error.to_string());
        }
    };
    let ensure_id = new_ensure_id();
    let claim = CanonicalEnsureClaim {
        ensure_id: ensure_id.clone(),
        idempotency_key: idempotency_key(&key, range),
        source_pdf_artifact_id: pdf.clone(),
        canonical_id: key.canonical_id(),
        parser_profile: key.parser_profile().to_owned(),
        parser_policy_version: key.parser_policy_version().to_owned(),
        pages: range,
        owner: format!("{ENSURE_ACTOR}/{}", std::process::id()),
    };
    let lineage_had_segments = views
        .iter()
        .any(|view| view.canonical_id == claim.canonical_id && !view.segments.is_empty());
    let mut report = EnsureReport::new(EnsureStatus::Started, &plan, "");
    report.marker_range = Some(range);
    report.idempotency_key = Some(claim.idempotency_key.clone());
    report.canonical_id = Some(claim.canonical_id.to_string());

    let limits = CanonicalEnsureLimits {
        max_running: policy.max_running,
        window_pages: policy.window_pages,
        window_ms: millis(policy.budget_window),
        lease_ms: millis(policy.marker_timeout).saturating_add(millis(policy.lease_margin)),
    };
    let outcome = workspace
        .store()
        .map_err(|error| error.to_string())
        .and_then(|mut store| {
            store
                .claim_canonical_ensure(&claim, limits, unix_millis_now())
                .map_err(|error| error.to_string())
        });
    match outcome {
        Err(error) => {
            report.status = EnsureStatus::Failed;
            report.message = format!("failed to claim the ensure lease: {error}");
            return report;
        }
        Ok(CanonicalEnsureClaimOutcome::PdfBusy(lease)) => {
            report.status = EnsureStatus::BlockedLock;
            report.message = format!(
                "this PDF is already being parsed (pages {}–{}); read again later",
                lease.pages.from, lease.pages.to
            );
            report.lease = Some(lease);
            return report;
        }
        Ok(CanonicalEnsureClaimOutcome::SlotsBusy { running }) => {
            report.status = EnsureStatus::BlockedLock;
            report.message = format!(
                "{running} Marker run(s) already active in this workspace; read again later"
            );
            return report;
        }
        Ok(CanonicalEnsureClaimOutcome::BudgetExhausted {
            used_pages,
            window_pages,
        }) => {
            report.status = EnsureStatus::BlockedBudget;
            report.message = format!(
                "rolling ensure budget used {used_pages} of {window_pages} pages; read again later"
            );
            return report;
        }
        Ok(CanonicalEnsureClaimOutcome::Acquired) => {}
    }
    report.ensure_id = Some(ensure_id.clone());

    let job = EnsureJob {
        workspace_root: workspace.path().to_owned(),
        database: workspace.database_path(),
        pdf: pdf.clone(),
        pdf_path: pdf_path.to_owned(),
        total_pages,
        range,
        installation,
        marker_version,
        marker_timeout: policy.marker_timeout,
        ensure_id,
    };
    let (sender, receiver) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("mathscape-ensure".into())
        .spawn(move || {
            let result = job.run();
            let _ = sender.send(result);
        });
    if let Err(error) = spawned {
        report.status = EnsureStatus::Failed;
        report.message = format!("failed to start the ensure worker: {error}");
        return report;
    }
    match receiver.recv_timeout(policy.wait) {
        Ok(Ok(done)) => {
            report.status = EnsureStatus::Expanded;
            report.conversion_id = Some(done.conversion_id);
            report.markdown_artifact_id = Some(done.markdown_artifact_id);
            report.message = if lineage_had_segments {
                "Marker parsed the requested pages; a new immutable snapshot was added to the canonical markdown".into()
            } else {
                "Marker parsed the requested pages; the first snapshot of this canonical markdown was promoted".into()
            };
        }
        Ok(Err(error)) => {
            report.status = EnsureStatus::Failed;
            report.message = error;
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            report.status = EnsureStatus::Started;
            report.message = format!(
                "Marker is parsing pages {}–{} (解析中); read again later",
                range.from, range.to
            );
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            report.status = EnsureStatus::Failed;
            report.message = "the ensure worker stopped without a result".into();
        }
    }
    report
}

struct EnsureJob {
    workspace_root: PathBuf,
    database: PathBuf,
    pdf: StableId,
    pdf_path: PathBuf,
    total_pages: u32,
    range: InclusivePageRange,
    installation: MarkerInstallation,
    marker_version: String,
    marker_timeout: Duration,
    ensure_id: String,
}

struct EnsureDone {
    conversion_id: String,
    markdown_artifact_id: String,
}

impl EnsureJob {
    /// Runs Marker and promotes, then always writes the terminal receipt.
    fn run(self) -> Result<EnsureDone, String> {
        let result = self.convert();
        let receipt = SqliteArtifactStore::open(&self.database)
            .map_err(|error| error.to_string())
            .and_then(|mut store| {
                let (outcome, conversion, markdown, error) = match &result {
                    Ok(done) => (
                        CanonicalEnsureOutcome::Succeeded,
                        Some(done.conversion_id.as_str()),
                        Some(done.markdown_artifact_id.as_str()),
                        None,
                    ),
                    Err(error) => (
                        CanonicalEnsureOutcome::Failed,
                        None,
                        None,
                        Some(error.as_str()),
                    ),
                };
                store
                    .finish_canonical_ensure(
                        &self.ensure_id,
                        outcome,
                        conversion,
                        markdown,
                        error,
                        unix_millis_now(),
                    )
                    .map_err(|error| error.to_string())
            });
        match (result, receipt) {
            (Ok(done), Ok(())) => Ok(done),
            (Ok(_), Err(error)) => Err(format!("snapshot promoted but receipt failed: {error}")),
            (Err(error), _) => Err(error),
        }
    }

    fn convert(&self) -> Result<EnsureDone, String> {
        let selection = PdfSelection::inclusive_range(
            self.total_pages,
            PdfPageIndex::new(self.range.from - 1),
            PdfPageIndex::new(self.range.to - 1),
        )
        .map_err(|error| format!("invalid page selection: {error}"))?;
        let staging_root = ArtifactPool::open(&self.workspace_root)
            .map_err(|error| error.to_string())?
            .workspace()
            .cache_path()
            .join("marker-runs");
        let staging = MarkerStagingArea::new(staging_root).map_err(|error| error.to_string())?;
        let request = MarkerRequest::new(self.pdf_path.clone(), selection, ENSURE_PROFILE)
            .map_err(|error| error.to_string())?;
        let options =
            MarkerExecutionOptions::new(self.marker_timeout).map_err(|error| error.to_string())?;
        let run = MarkerRunner::new(options)
            .execute(
                &self.installation,
                &request,
                &staging,
                &MarkerCancellation::new(),
            )
            .map_err(|error| format!("Marker could not run: {error}"))?;
        if run.status() != MarkerRunStatus::Succeeded {
            let detail = run.stderr().preview_lossy();
            let detail: String = detail.chars().take(500).collect();
            return Err(format!("Marker run {:?}: {detail}", run.status()));
        }
        let inventory = MarkerProductScanner::default()
            .scan(&run)
            .map_err(|error| format!("Marker output could not be scanned: {error}"))?;
        if !inventory.is_promotable() {
            return Err(format!(
                "Marker output is not promotable: {:?}",
                inventory.issues()
            ));
        }
        let mut pool =
            ArtifactPool::open(&self.workspace_root).map_err(|error| error.to_string())?;
        let actor = ActorRef::parse(ENSURE_ACTOR).map_err(|error| error.to_string())?;
        let promotion = promote_marker_inventory(
            &mut pool,
            &run,
            &inventory,
            &self.pdf,
            Some(&self.marker_version),
            actor,
        )
        .map_err(|error| format!("promotion failed: {error}"))?;
        let markdown = promotion
            .outputs()
            .iter()
            .find(|output| output.role() == MarkerProductRole::PrimaryMarkdown)
            .map(|output| output.artifact().stable_id().to_string())
            .ok_or_else(|| "promotion produced no primary markdown".to_owned())?;
        let conversion_id = promotion.conversion_id().to_string();
        drop(pool);
        let _ = run.cleanup();

        let mut store =
            SqliteArtifactStore::open(&self.database).map_err(|error| error.to_string())?;
        store
            .sync_canonical_markdown_for_pdf(&self.pdf, &now_timestamp()?)
            .map_err(|error| format!("canonical registry sync failed: {error}"))?;
        Ok(EnsureDone {
            conversion_id,
            markdown_artifact_id: markdown,
        })
    }
}

fn installation_for(executable: &Path) -> Result<MarkerInstallation, String> {
    let dir = executable
        .parent()
        .ok_or_else(|| "marker_single has no parent directory".to_owned())?;
    let python = ["python", "python3"]
        .iter()
        .map(|name| dir.join(name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| "no python next to marker_single".to_owned())?;
    MarkerInstallation::with_python(executable, python).map_err(|error| error.to_string())
}

fn absolute(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&path))
            .unwrap_or(path)
    }
}

fn now_timestamp() -> Result<Timestamp, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp::parse(format!("unix:{}.{}", now.as_secs(), now.subsec_nanos()))
        .map_err(|error| error.to_string())
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn new_ensure_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "ensure-{}-{}-{}",
        unix_millis_now(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use mathscape_domain::CanonicalSegment;

    use super::*;

    fn range(from: u32, to: u32) -> InclusivePageRange {
        InclusivePageRange::new(from, to).unwrap()
    }

    fn view(segments: &[(u32, u32)]) -> CanonicalMarkdownView {
        CanonicalMarkdownView {
            canonical_id: StableId::parse("mathscape:canonical-markdown/v1/p/x/pdf").unwrap(),
            source_pdf_artifact_id: StableId::parse("mathscape:artifact/pdf").unwrap(),
            parser_profile: "p".into(),
            parser_policy_version: "x".into(),
            head_artifact_id: None,
            head_revision: None,
            total_pages: Some(20),
            segments: segments
                .iter()
                .enumerate()
                .map(|(index, (from, to))| CanonicalSegment {
                    conversion_id: StableId::parse(format!("mathscape:conversion/{index}"))
                        .unwrap(),
                    markdown_artifact_id: StableId::parse(format!("mathscape:artifact/md{index}"))
                        .unwrap(),
                    pages: Some(range(*from, *to)),
                    finished_at: None,
                })
                .collect(),
        }
    }

    #[test]
    fn contained_request_is_reused() {
        let lineage = view(&[(1, 10)]);
        let plan = plan_ensure(&[&lineage], range(3, 4), 20, 12);
        assert_eq!(plan.action, PlanAction::Reuse);
        assert!(plan.missing.is_empty());
    }

    #[test]
    fn overlap_only_coverage_reruns_the_whole_request_and_reports_gaps() {
        let lineage = view(&[(1, 3), (6, 7)]);
        let plan = plan_ensure(&[&lineage], range(2, 8), 20, 12);
        assert_eq!(plan.action, PlanAction::Run(range(2, 8)));
        assert_eq!(plan.missing, vec![range(4, 5), range(8, 8)]);
    }

    #[test]
    fn union_coverage_without_one_containing_segment_is_not_reuse() {
        let lineage = view(&[(1, 3), (4, 6)]);
        let plan = plan_ensure(&[&lineage], range(2, 5), 20, 12);
        assert!(plan.missing.is_empty());
        assert_eq!(plan.action, PlanAction::Run(range(2, 5)));
    }

    #[test]
    fn oversized_request_is_blocked_by_the_page_budget() {
        let plan = plan_ensure(&[], range(1, 13), 20, 12);
        assert_eq!(plan.action, PlanAction::OverBudget);
        assert_eq!(plan.missing, vec![range(1, 13)]);
    }

    #[test]
    fn idempotency_key_names_pdf_lineage_range_profile_and_policy() {
        let key = CanonicalMarkdownKey::new(
            StableId::parse("mathscape:artifact/pdf").unwrap(),
            "mathscape.marker.math-ocr.v1",
            "marker-2.1.0",
        )
        .unwrap();
        let text = idempotency_key(&key, range(2, 3));
        assert!(text.starts_with("mathscape:artifact/pdf|"));
        assert!(text.contains("|2-3|mathscape.marker.math-ocr.v1|marker-2.1.0|ensure-v1"));
        assert_eq!(text, idempotency_key(&key, range(2, 3)));
    }
}
