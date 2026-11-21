//! Canonical markdown ensure: the internal path behind `material.read` on a
//! scanned PDF, and behind the desktop reader's 「解析这几页」
//! ([`ensure_canonical_pages`]). Not an MCP tool.
//!
//! Synchronous: the `material.read` call that needs Marker waits for it.
//! Order: plan → probe Marker → claim lease (per-PDF lock + global slot +
//! rolling page budget, one SQLite `BEGIN IMMEDIATE`) → Marker on one
//! contiguous range (lease renewed on a heartbeat) → validate inventory →
//! promote new immutable artifacts → scoped registry sync for this PDF →
//! terminal receipt → re-plan (next gap, if any) → the same call serves the
//! stitched snapshots. A read that
//! finds the PDF (or every Marker slot) busy queues behind the live lease,
//! re-plans when it is released (usually `reused`) and never answers
//! "read again later". Every wait is bounded by [`EnsurePolicy::sync_timeout`];
//! past it the ensure is failed, its lease cleared and the read fails.
//! Artifact bytes are never rewritten; a new snapshot is a new
//! content-addressed artifact.
//!
//! Gap-only parsing: a request whose pages are all covered by existing
//! snapshots (one containing snapshot, or several stitched page by page,
//! across every lineage of the PDF) never runs Marker. Otherwise Marker runs
//! once per uncovered gap (`missing`), in page order, each run its own lease,
//! budget charge, idempotency key and immutable snapshot, so covered pages
//! between two gaps are never parsed again. `material.read` then stitches old
//! and new snapshots. (Marker accepts a multi-range `--page_range`, but one
//! conversion run is one contiguous coverage segment; per-gap runs keep that
//! provenance intact at about the same wall time.)

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use mathscape_agent::{
    Capability, CapabilityProbe, CapabilityState, ToolError, ToolErrorCode,
    locate_marker_executable, probe_capability,
};
use mathscape_artifact::ArtifactPool;
use mathscape_domain::{
    ActorRef, CanonicalMarkdownKey, CanonicalMarkdownView, InclusivePageRange, StableId, Timestamp,
    parser_policy_from_tool_version, plan_page_cover,
};
use mathscape_orchestrator::promote_marker_inventory;
use mathscape_pdf::{PdfEngine, PdfOpenOptions, PdfPageIndex, PdfSelection};
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

use super::canonical::CanonicalIndex;
use crate::McpWorkspace;

/// Bumped when the ensure planning rules change; part of the idempotency key.
pub const ENSURE_POLICY_VERSION: &str = "ensure-v3";
const ENSURE_PROFILE: MarkerProfile = MarkerProfile::MathOcrV1;
const ENSURE_ACTOR: &str = "mathscape-mcp-ensure";
/// After the deadline the worker gets this long to stop Marker and write its
/// receipt before the read gives up on it and fails the lease itself.
const CANCEL_GRACE: Duration = Duration::from_secs(10);

/// Budgets, locks and waits for ensure. Defaults are conservative: one Marker
/// at a time per workspace, at most 12 pages per request, 96 pages per hour,
/// and one `material.read` waits at most 15 minutes end to end.
#[derive(Clone, Copy, Debug)]
pub struct EnsurePolicy {
    /// Largest request (pages) a single read may ensure.
    pub max_pages_per_request: u32,
    /// Rolling page budget across all ensures in `budget_window`.
    pub window_pages: u32,
    pub budget_window: Duration,
    /// Global Marker semaphore (live ensures per workspace).
    pub max_running: u32,
    /// Wall-clock bound of one synchronous ensure: queueing behind another
    /// lease plus the Marker run. Past it the ensure fails and its lease is
    /// cleared. Keep it below the agent's MCP tool timeout.
    pub sync_timeout: Duration,
    /// Upper bound of one Marker process (also capped by what is left of
    /// `sync_timeout`).
    pub marker_timeout: Duration,
    /// Lease TTL. The owner renews it every `heartbeat`, so a crashed MCP
    /// process stops holding the PDF (and showing 「解析中」) within one TTL.
    pub lease_ttl: Duration,
    pub heartbeat: Duration,
    /// How often a queued read re-checks the lease it waits behind.
    pub poll_interval: Duration,
}

impl EnsurePolicy {
    /// A person asking for a few pages from the desktop reader: same locks,
    /// lease and per-request size, but no rolling page budget. The budget
    /// guards against an agent reading through a whole book; a reader's own
    /// click must not be refused because an agent just used the window.
    pub fn desktop() -> Self {
        Self {
            window_pages: u32::MAX,
            ..Self::default()
        }
    }
}

impl Default for EnsurePolicy {
    fn default() -> Self {
        Self {
            max_pages_per_request: 12,
            window_pages: 96,
            budget_window: Duration::from_secs(60 * 60),
            max_running: 1,
            sync_timeout: Duration::from_secs(15 * 60),
            marker_timeout: Duration::from_secs(15 * 60),
            lease_ttl: Duration::from_secs(60),
            heartbeat: Duration::from_secs(15),
            poll_interval: Duration::from_millis(500),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnsureStatus {
    /// Existing snapshots already cover every requested page: one contains
    /// the request, or several are stitched by page (possibly including one
    /// that another read's ensure promoted while this read waited). Marker
    /// not run.
    Reused,
    /// Marker ran in this call on every uncovered gap (one run per gap) and
    /// the new snapshots were promoted; together with existing snapshots
    /// they cover the request.
    Expanded,
    /// The request or the rolling budget is too large.
    BlockedBudget,
    /// Marker is not installed or not usable.
    CapabilityUnavailable,
    /// Marker, inventory validation, promotion or the sync deadline failed.
    /// The lease is released (or failed) before the read returns.
    Failed,
    /// Only for an ensure scoped to one canonical view
    /// ([`ensure_canonical_pages`]): the installed Marker writes a different
    /// lineage, so its pages would never appear in that view. Marker not run.
    LineageMismatch,
}

impl EnsureStatus {
    pub(super) fn diagnostic_code(self) -> &'static str {
        match self {
            Self::Reused => "canonical_ensure_reused",
            Self::Expanded => "canonical_ensure_expanded",
            Self::BlockedBudget => "canonical_ensure_blocked_budget",
            Self::CapabilityUnavailable => "canonical_ensure_capability_unavailable",
            Self::Failed => "canonical_ensure_failed",
            Self::LineageMismatch => "canonical_ensure_lineage_mismatch",
        }
    }
}

/// Structured ensure outcome, embedded in `material.read` `details.ensure`.
#[derive(Clone, Debug, Serialize)]
pub struct EnsureReport {
    pub status: EnsureStatus,
    pub requested: InclusivePageRange,
    /// Requested pages no snapshot covers (union of every lineage), as
    /// planned before this call parsed or waited.
    pub missing: Vec<InclusivePageRange>,
    /// The ranges given to Marker in this call, one run each, in order: the
    /// gaps of `missing` (re-planned before each run). Never a covered page.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub marker_ranges: Vec<InclusivePageRange>,
    /// Every Marker run this call claimed, in order (successful ones carry
    /// their snapshot). The singular fields below repeat the latest run.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<EnsureRun>,
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
    /// The live lease this read queued behind (same PDF or global slot).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waited_for: Option<CanonicalEnsureLease>,
    /// Wall-clock milliseconds this call spent queued plus parsing.
    pub waited_ms: u64,
    /// True when `sync_timeout` ran out (queued or parsing).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub timed_out: bool,
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
            marker_ranges: Vec::new(),
            runs: Vec::new(),
            ensure_id: None,
            idempotency_key: None,
            canonical_id: None,
            conversion_id: None,
            markdown_artifact_id: None,
            waited_for: None,
            waited_ms: 0,
            timed_out: false,
            max_pages_per_request: None,
            probe: None,
            message: message.into(),
        }
    }

    /// Post-hoc 「解析中」 record for the scene player: this call blocked on a
    /// Marker parse of this PDF (its own or one it queued behind). `phase` is
    /// how the block ended. The live signal during the call is the landscape
    /// node summary (running lease), since MCP results cannot stream.
    pub(super) fn map_signal(&self, pdf: &StableId) -> Option<serde_json::Value> {
        let hull = self
            .marker_ranges
            .first()
            .zip(self.marker_ranges.last())
            .and_then(|(first, last)| InclusivePageRange::new(first.from, last.to).ok());
        let pages = hull.or_else(|| {
            self.waited_for
                .as_ref()
                .filter(|lease| &lease.source_pdf_artifact_id == pdf)
                .map(|lease| lease.pages)
        })?;
        let phase = match self.status {
            EnsureStatus::Expanded | EnsureStatus::Reused => "done",
            EnsureStatus::Failed => "failed",
            _ => return None,
        };
        Some(serde_json::json!({
            "kind": "parsing",
            "label": "解析中",
            "node_id": pdf.as_str(),
            "pages": pages,
            "phase": phase,
            "waited_ms": self.waited_ms,
        }))
    }
}

/// One Marker run claimed by an ensure call.
#[derive(Clone, Debug, Serialize)]
pub struct EnsureRun {
    pub marker_range: InclusivePageRange,
    pub ensure_id: String,
    pub idempotency_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversion_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown_artifact_id: Option<String>,
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
    /// No single segment contains the request, but existing segments cover
    /// every requested page; `material.read` stitches them by page.
    Stitch,
    /// Run Marker on this gap (the first uncovered gap); the caller re-plans
    /// afterwards and runs the next gap the same way.
    Run(InclusivePageRange),
    /// The uncovered pages (sum of the gaps) exceed the per-request budget.
    OverBudget,
}

/// Plans an ensure against every canonical lineage of the PDF (`views`,
/// best-ranked first). Coverage is the page-by-page cover of
/// [`plan_page_cover`]: covered pages are never parsed again; Marker runs
/// on the gaps one at a time, first gap first. `total_pages` bounds the
/// request upstream and is kept for the signature's callers.
pub(super) fn plan_ensure(
    views: &[&CanonicalMarkdownView],
    requested: InclusivePageRange,
    _total_pages: u32,
    max_pages_per_request: u32,
) -> EnsurePlan {
    let cover = plan_page_cover(views, requested);
    let missing = cover.gaps.clone();
    let contained = views.iter().any(|view| {
        view.segments
            .iter()
            .any(|segment| segment.pages.is_some_and(|pages| pages.contains(requested)))
    });
    let action = if contained {
        PlanAction::Reuse
    } else {
        let uncovered: u32 = missing.iter().map(|gap| gap.page_count()).sum();
        match missing.first() {
            None => PlanAction::Stitch,
            Some(_) if uncovered > max_pages_per_request => PlanAction::OverBudget,
            Some(gap) => PlanAction::Run(*gap),
        }
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

/// Marker found usable for this workspace (probed once per ensure call).
struct MarkerReady {
    installation: MarkerInstallation,
    version: String,
}

/// Runs ensure for one PDF request synchronously. Never returns a call
/// error for Marker problems; they become `capability_unavailable` or
/// `failed`. Never returns while a lease it holds is still `running`.
pub(super) fn ensure(
    workspace: &McpWorkspace,
    pdf: &StableId,
    pdf_path: &Path,
    requested: InclusivePageRange,
    total_pages: u32,
) -> EnsureReport {
    ensure_scoped(
        workspace,
        pdf,
        pdf_path,
        requested,
        total_pages,
        None,
        ENSURE_ACTOR,
    )
}

/// Ensures pages of one registered canonical view, for a caller that shows
/// exactly that view (the desktop reader's gap card). Same locks, lease,
/// idempotency and promotion as `material.read`, with two differences:
/// coverage is judged against this view alone, so pages parsed only by
/// another lineage (an older Marker) still count as missing here; and when
/// the installed Marker would write a different lineage, nothing is run
/// ([`EnsureStatus::LineageMismatch`]). `owner` labels the lease.
///
/// Synchronous: returns after Marker finished, failed, or
/// [`EnsurePolicy::sync_timeout`] passed. `Err` only for a request that
/// cannot be planned (unknown view, pages outside the PDF, PDF not readable).
pub fn ensure_canonical_pages(
    workspace: &McpWorkspace,
    canonical_id: &StableId,
    pages: InclusivePageRange,
    owner: &str,
) -> Result<EnsureReport, ToolError> {
    let index = CanonicalIndex::load(workspace)?;
    let view = index.by_id(canonical_id.as_str()).ok_or_else(|| {
        ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("canonical markdown view {canonical_id} is not registered"),
        )
    })?;
    let pdf = view.source_pdf_artifact_id.clone();
    let pool = workspace.pool()?;
    let pdf_path = pool.managed_path(&pdf).map_err(|error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("the source PDF of {canonical_id} is not readable: {error}"),
        )
    })?;
    drop(pool);
    let total_pages = match view.total_pages {
        Some(total) => total,
        None => PdfEngine
            .probe(&pdf_path, &PdfOpenOptions::new())
            .map(|probe| probe.page_count())
            .map_err(|error| {
                ToolError::new(
                    ToolErrorCode::Internal,
                    format!("failed to read the PDF page count: {error}"),
                )
            })?,
    };
    if pages.to > total_pages {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!(
                "pages {}–{} are outside the PDF ({total_pages} pages)",
                pages.from, pages.to
            ),
        ));
    }
    drop(index);
    Ok(ensure_scoped(
        workspace,
        &pdf,
        &pdf_path,
        pages,
        total_pages,
        Some(canonical_id),
        owner,
    ))
}

fn ensure_scoped(
    workspace: &McpWorkspace,
    pdf: &StableId,
    pdf_path: &Path,
    requested: InclusivePageRange,
    total_pages: u32,
    scope: Option<&StableId>,
    owner: &str,
) -> EnsureReport {
    let policy = workspace.ensure_policy();
    let started = Instant::now();
    let deadline = started + policy.sync_timeout;
    let mut first_plan: Option<EnsurePlan> = None;
    let mut waited_for: Option<CanonicalEnsureLease> = None;
    let mut marker: Option<MarkerReady> = None;
    // Marker runs this call claimed, in order. Successful runs stay promoted
    // even when a later gap fails.
    let mut runs: Vec<EnsureRun> = Vec::new();
    let mut canonical_id: Option<String> = None;
    loop {
        let plan = match CanonicalIndex::load(workspace) {
            Ok(index) => {
                let views = match scope {
                    // One view: another lineage's pages are not shown there.
                    Some(canonical) => index.by_id(canonical.as_str()).into_iter().collect(),
                    None => index.all_for_pdf(pdf.as_str(), Some(requested)),
                };
                plan_ensure(&views, requested, total_pages, policy.max_pages_per_request)
            }
            Err(error) => {
                let plan = first_plan.unwrap_or_else(|| EnsurePlan {
                    requested,
                    missing: vec![requested],
                    action: PlanAction::Run(requested),
                });
                let report = EnsureReport::new(
                    EnsureStatus::Failed,
                    &plan,
                    format!("failed to load the canonical registry: {}", error.message),
                );
                return finish(report, started, waited_for, runs, canonical_id);
            }
        };
        // Report what was missing when the call started, not after a wait or
        // an earlier gap's run.
        let shown = first_plan.get_or_insert_with(|| plan.clone()).clone();
        let parsed = runs
            .iter()
            .filter(|run| run.markdown_artifact_id.is_some())
            .count();
        let range = match plan.action {
            PlanAction::Reuse | PlanAction::Stitch => {
                let report = if parsed > 0 {
                    EnsureReport::new(
                        EnsureStatus::Expanded,
                        &shown,
                        format!(
                            "Marker parsed the {} uncovered gap(s) in this call, one run each; new immutable snapshots were added to the canonical markdown and are stitched with the existing ones",
                            parsed
                        ),
                    )
                } else {
                    let message = match (waited_for.is_some(), &plan.action) {
                        (true, _) => {
                            "waited for the running parse of this PDF; existing immutable snapshots now cover every requested page"
                        }
                        (false, PlanAction::Reuse) => {
                            "an existing immutable snapshot contains every requested page"
                        }
                        _ => {
                            "existing immutable snapshots together cover every requested page; they are stitched by page without running Marker"
                        }
                    };
                    EnsureReport::new(EnsureStatus::Reused, &shown, message)
                };
                return finish(report, started, waited_for, runs, canonical_id);
            }
            PlanAction::OverBudget => {
                let uncovered: u32 = plan.missing.iter().map(|gap| gap.page_count()).sum();
                let mut report = EnsureReport::new(
                    EnsureStatus::BlockedBudget,
                    &shown,
                    format!(
                        "this request has {uncovered} uncovered pages; ensure parses at most {} new pages per read. Narrow pdf_pages",
                        policy.max_pages_per_request
                    ),
                );
                report.max_pages_per_request = Some(policy.max_pages_per_request);
                return finish(report, started, waited_for, runs, canonical_id);
            }
            PlanAction::Run(range) => range,
        };
        if runs.iter().any(|run| run.marker_range == range) {
            // A run of ours promoted this gap but the registry still reports
            // it uncovered; never loop on Marker.
            let report = EnsureReport::new(
                EnsureStatus::Failed,
                &shown,
                format!(
                    "Marker parsed pages {}–{} in this call but the canonical registry still reports them uncovered",
                    range.from, range.to
                ),
            );
            return finish(report, started, waited_for, runs, canonical_id);
        }

        let ready = match marker.take() {
            Some(ready) => ready,
            None => match probe_marker(workspace) {
                Ok(ready) => ready,
                Err((message, probe)) => {
                    let mut report =
                        EnsureReport::new(EnsureStatus::CapabilityUnavailable, &shown, message);
                    report.probe = Some(probe);
                    return finish(report, started, waited_for, runs, canonical_id);
                }
            },
        };
        let key = match CanonicalMarkdownKey::new(
            pdf.clone(),
            ENSURE_PROFILE.id(),
            parser_policy_from_tool_version(Some(&ready.version)),
        ) {
            Ok(key) => key,
            Err(error) => {
                let report = EnsureReport::new(EnsureStatus::Failed, &shown, error.to_string());
                return finish(report, started, waited_for, runs, canonical_id);
            }
        };
        if let Some(canonical) = scope
            && &key.canonical_id() != canonical
        {
            let report = EnsureReport::new(
                EnsureStatus::LineageMismatch,
                &shown,
                format!(
                    "the installed Marker ({}) writes canonical markdown {}, not the requested view {canonical}; its pages would not appear in that view, so Marker was not run",
                    ready.version,
                    key.canonical_id()
                ),
            );
            return finish(report, started, waited_for, runs, canonical_id);
        }
        let ensure_id = new_ensure_id();
        let claim = CanonicalEnsureClaim {
            ensure_id: ensure_id.clone(),
            idempotency_key: idempotency_key(&key, range),
            source_pdf_artifact_id: pdf.clone(),
            canonical_id: key.canonical_id(),
            parser_profile: key.parser_profile().to_owned(),
            parser_policy_version: key.parser_policy_version().to_owned(),
            pages: range,
            owner: format!("{owner}/{}", std::process::id()),
        };
        canonical_id = Some(claim.canonical_id.to_string());

        let limits = CanonicalEnsureLimits {
            max_running: policy.max_running,
            window_pages: policy.window_pages,
            window_ms: millis(policy.budget_window),
            lease_ms: millis(policy.lease_ttl),
        };
        let outcome = workspace
            .store()
            .map_err(|error| error.message)
            .and_then(|mut store| {
                store
                    .claim_canonical_ensure(&claim, limits, unix_millis_now())
                    .map_err(|error| error.to_string())
            });
        let busy = match outcome {
            Err(error) => {
                let report = EnsureReport::new(
                    EnsureStatus::Failed,
                    &shown,
                    format!("failed to claim the ensure lease: {error}"),
                );
                return finish(report, started, waited_for, runs, canonical_id);
            }
            Ok(CanonicalEnsureClaimOutcome::BudgetExhausted {
                used_pages,
                window_pages,
            }) => {
                let report = EnsureReport::new(
                    EnsureStatus::BlockedBudget,
                    &shown,
                    format!(
                        "rolling ensure budget used {used_pages} of {window_pages} pages in the last {} minutes; wait for the window or read pages that are already parsed",
                        policy.budget_window.as_secs() / 60
                    ),
                );
                return finish(report, started, waited_for, runs, canonical_id);
            }
            Ok(CanonicalEnsureClaimOutcome::PdfBusy(lease)) => Some(lease),
            Ok(CanonicalEnsureClaimOutcome::SlotsBusy { .. }) => None,
            Ok(CanonicalEnsureClaimOutcome::Acquired) => {
                let job = EnsureJob {
                    workspace_root: workspace.path().to_owned(),
                    database: workspace.database_path(),
                    pdf: pdf.clone(),
                    pdf_path: pdf_path.to_owned(),
                    total_pages,
                    range,
                    installation: ready.installation.clone(),
                    marker_version: ready.version.clone(),
                    marker_timeout: policy
                        .marker_timeout
                        .min(deadline.saturating_duration_since(Instant::now()))
                        .max(Duration::from_secs(1)),
                    ensure_id: ensure_id.clone(),
                    cancellation: MarkerCancellation::new(),
                };
                let mut run = EnsureRun {
                    marker_range: range,
                    ensure_id,
                    idempotency_key: claim.idempotency_key.clone(),
                    conversion_id: None,
                    markdown_artifact_id: None,
                };
                match run_owned(job, &policy, deadline) {
                    Ok(done) => {
                        run.conversion_id = Some(done.conversion_id);
                        run.markdown_artifact_id = Some(done.markdown_artifact_id);
                        runs.push(run);
                        // Re-plan: the next gap, or stitch.
                        marker = Some(ready);
                        continue;
                    }
                    Err(failure) => {
                        runs.push(run);
                        let earlier = parsed;
                        let mut report = EnsureReport::new(
                            EnsureStatus::Failed,
                            &shown,
                            if earlier > 0 {
                                format!(
                                    "{} (the {earlier} gap(s) parsed earlier in this call stay promoted)",
                                    failure.message
                                )
                            } else {
                                failure.message
                            },
                        );
                        report.timed_out = failure.timed_out;
                        return finish(report, started, waited_for, runs, canonical_id);
                    }
                }
            }
        };

        // Queue: the PDF or every Marker slot is busy. Wait for the lease to
        // go away (finished, failed or expired), then re-plan.
        let what = match &busy {
            Some(lease) => format!(
                "this PDF is being parsed (pages {}–{})",
                lease.pages.from, lease.pages.to
            ),
            None => "every Marker slot in this workspace is busy".to_owned(),
        };
        if let Some(lease) = busy.clone() {
            waited_for = Some(lease);
        } else if waited_for.is_none() {
            waited_for = live_lease(workspace);
        }
        marker = Some(ready);
        if Instant::now() >= deadline {
            let mut report = EnsureReport::new(
                EnsureStatus::Failed,
                &shown,
                format!(
                    "{what}; gave up after waiting {}s (sync ensure deadline). Nothing of this read is left running",
                    policy.sync_timeout.as_secs()
                ),
            );
            report.timed_out = true;
            return finish(report, started, waited_for, runs, canonical_id);
        }
        wait_for_release(workspace, busy.as_ref(), &policy, deadline);
    }
}

/// Attaches what the call did (waits, runs) to its terminal report.
fn finish(
    mut report: EnsureReport,
    started: Instant,
    waited_for: Option<CanonicalEnsureLease>,
    runs: Vec<EnsureRun>,
    canonical_id: Option<String>,
) -> EnsureReport {
    report.waited_for = waited_for;
    report.waited_ms = elapsed_ms(started);
    if let Some(last) = runs.last() {
        report.ensure_id = Some(last.ensure_id.clone());
        report.idempotency_key = Some(last.idempotency_key.clone());
        report.canonical_id = canonical_id;
    }
    if let Some(done) = runs
        .iter()
        .rev()
        .find(|run| run.markdown_artifact_id.is_some())
    {
        report.conversion_id = done.conversion_id.clone();
        report.markdown_artifact_id = done.markdown_artifact_id.clone();
    }
    report.marker_ranges = runs.iter().map(|run| run.marker_range).collect();
    report.runs = runs;
    report
}

/// Sleeps in `poll_interval` steps until no live lease blocks this read (or
/// the deadline). Expired leases stop counting as soon as they lapse.
fn wait_for_release(
    workspace: &McpWorkspace,
    lease: Option<&CanonicalEnsureLease>,
    policy: &EnsurePolicy,
    deadline: Instant,
) {
    while Instant::now() < deadline {
        thread::sleep(
            policy
                .poll_interval
                .min(deadline.saturating_duration_since(Instant::now())),
        );
        let Ok(store) = workspace.store() else {
            return;
        };
        let Ok(live) = store.active_canonical_ensures(unix_millis_now()) else {
            return;
        };
        let still = match lease {
            Some(lease) => live.iter().any(|other| other.ensure_id == lease.ensure_id),
            None => u32::try_from(live.len()).unwrap_or(u32::MAX) >= policy.max_running,
        };
        if !still {
            return;
        }
    }
}

fn live_lease(workspace: &McpWorkspace) -> Option<CanonicalEnsureLease> {
    workspace
        .store()
        .ok()?
        .active_canonical_ensures(unix_millis_now())
        .ok()?
        .into_iter()
        .next()
}

fn probe_marker(workspace: &McpWorkspace) -> Result<MarkerReady, (String, CapabilityProbe)> {
    let probe = probe_capability(workspace.capability_runner(), Capability::Marker);
    if probe.state != CapabilityState::Available {
        return Err((
            "Marker is not installed or its version is unsupported".into(),
            probe,
        ));
    }
    let installation = match locate_marker_executable(workspace.capability_runner())
        .map(absolute)
        .ok_or_else(|| "marker_single was not found".to_owned())
        .and_then(|exe| installation_for(&exe))
    {
        Ok(installation) => installation,
        Err(message) => return Err((message, probe)),
    };
    match MarkerRunner::default().probe(&installation) {
        Ok(found) => Ok(MarkerReady {
            installation,
            version: found.version().raw().to_owned(),
        }),
        Err(error) => Err((format!("Marker probe failed: {error}"), probe)),
    }
}

/// Why a claimed run did not produce a snapshot. Its lease is failed.
struct RunFailure {
    message: String,
    timed_out: bool,
}

/// Runs a claimed job on a worker thread while this thread renews the lease
/// every heartbeat. On the deadline: cancel Marker, give the worker
/// [`CANCEL_GRACE`] to write its failed receipt, else fail the lease here.
fn run_owned(
    job: EnsureJob,
    policy: &EnsurePolicy,
    deadline: Instant,
) -> Result<EnsureDone, RunFailure> {
    let database = job.database.clone();
    let ensure_id = job.ensure_id.clone();
    let cancellation = job.cancellation.clone();
    let range = job.range;
    let (sender, receiver) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("mathscape-ensure".into())
        .spawn(move || {
            let result = job.run();
            let _ = sender.send(result);
        });
    if let Err(error) = spawned {
        fail_lease(&database, &ensure_id, "failed to start the ensure worker");
        return Err(RunFailure {
            message: format!("failed to start the ensure worker: {error}"),
            timed_out: false,
        });
    }
    let ttl = millis(policy.lease_ttl);
    let mut cancelled_at: Option<Instant> = None;
    let result = loop {
        let now = Instant::now();
        let wake = match cancelled_at {
            Some(at) => at + CANCEL_GRACE,
            None => deadline,
        };
        let step = policy
            .heartbeat
            .min(wake.saturating_duration_since(now))
            .max(Duration::from_millis(10));
        match receiver.recv_timeout(step) {
            Ok(result) => break Some(result),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Some(Err("the ensure worker stopped without a result".into()));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let now = Instant::now();
        match cancelled_at {
            None if now >= deadline => {
                cancellation.cancel();
                cancelled_at = Some(now);
            }
            Some(at) if now >= at + CANCEL_GRACE => break None,
            _ => {}
        }
        // Heartbeat (also while cancelling, so the lock stays ours until the
        // receipt is written).
        if let Ok(mut store) = SqliteArtifactStore::open(&database) {
            let now_ms = unix_millis_now();
            let _ = store.renew_canonical_ensure(&ensure_id, now_ms.saturating_add(ttl), now_ms);
        }
    };
    let timed_out = cancelled_at.is_some();
    match result {
        // A run that lands inside the cancel grace still counts.
        Some(Ok(done)) => Ok(done),
        None => {
            // The worker did not answer within the cancel grace: fail the
            // lease here so nothing stays `running`.
            fail_lease(&database, &ensure_id, "sync ensure deadline exceeded");
            Err(RunFailure {
                message: timeout_message(range, policy),
                timed_out: true,
            })
        }
        Some(Err(error)) => {
            fail_lease(&database, &ensure_id, &error);
            Err(RunFailure {
                message: if timed_out {
                    format!("{}: {error}", timeout_message(range, policy))
                } else {
                    error
                },
                timed_out,
            })
        }
    }
}

fn timeout_message(range: InclusivePageRange, policy: &EnsurePolicy) -> String {
    format!(
        "Marker did not finish pages {}–{} within the {}s sync ensure limit; the run was stopped and its lease cleared. Request fewer pages",
        range.from,
        range.to,
        policy.sync_timeout.as_secs()
    )
}

/// Idempotent: marks a still-`running` (or `abandoned`) lease `failed`.
fn fail_lease(database: &Path, ensure_id: &str, error: &str) {
    if let Ok(mut store) = SqliteArtifactStore::open(database) {
        let _ = store.finish_canonical_ensure(
            ensure_id,
            CanonicalEnsureOutcome::Failed,
            None,
            None,
            Some(error),
            unix_millis_now(),
        );
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
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
    cancellation: MarkerCancellation,
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
            .execute(&self.installation, &request, &staging, &self.cancellation)
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
    fn partial_coverage_runs_marker_on_each_gap_never_on_covered_pages() {
        let lineage = view(&[(1, 3), (6, 7)]);
        let plan = plan_ensure(&[&lineage], range(2, 8), 20, 12);
        assert_eq!(plan.missing, vec![range(4, 5), range(8, 8)]);
        // First gap first; pages 6–7 between the gaps are never parsed.
        assert_eq!(plan.action, PlanAction::Run(range(4, 5)));
        // After 4–5 is promoted the re-plan runs the second gap only.
        let after = view(&[(1, 3), (6, 7), (4, 5)]);
        let plan = plan_ensure(&[&after], range(2, 8), 20, 12);
        assert_eq!(plan.action, PlanAction::Run(range(8, 8)));
        let done = view(&[(1, 3), (6, 7), (4, 5), (8, 8)]);
        let plan = plan_ensure(&[&done], range(2, 8), 20, 12);
        assert_eq!(plan.action, PlanAction::Stitch);
    }

    #[test]
    fn union_coverage_is_stitched_without_marker() {
        // Sandbox sbx-20261101T235040-bc83e994: 1-4, 5-10, 11-14 were parsed,
        // then reads of 1-8 and 9-14 re-ran Marker on the whole request.
        let lineage = view(&[(1, 4), (5, 10), (11, 14)]);
        for request in [range(1, 8), range(9, 14), range(1, 14)] {
            let plan = plan_ensure(&[&lineage], request, 14, 12);
            assert!(plan.missing.is_empty());
            assert_eq!(plan.action, PlanAction::Stitch, "{request:?}");
        }
    }

    #[test]
    fn coverage_from_another_lineage_counts() {
        let old = view(&[(1, 6)]);
        let current = view(&[(7, 9)]);
        let plan = plan_ensure(&[&current, &old], range(5, 9), 20, 12);
        assert_eq!(plan.action, PlanAction::Stitch);
        let plan = plan_ensure(&[&current, &old], range(5, 11), 20, 12);
        assert_eq!(plan.action, PlanAction::Run(range(10, 11)));
    }

    #[test]
    fn budget_applies_to_the_pages_marker_would_parse() {
        let plan = plan_ensure(&[], range(1, 13), 20, 12);
        assert_eq!(plan.action, PlanAction::OverBudget);
        assert_eq!(plan.missing, vec![range(1, 13)]);
        // 16 requested pages, only 2 uncovered: within budget.
        let lineage = view(&[(1, 14)]);
        let plan = plan_ensure(&[&lineage], range(1, 16), 20, 12);
        assert_eq!(plan.action, PlanAction::Run(range(15, 16)));
        // Budget counts the gaps, not their hull: 1 + 1 uncovered pages
        // around 17 covered ones stay within a budget of 2.
        let lineage = view(&[(2, 18)]);
        let plan = plan_ensure(&[&lineage], range(1, 19), 20, 2);
        assert_eq!(plan.action, PlanAction::Run(range(1, 1)));
        let plan = plan_ensure(&[&lineage], range(1, 20), 20, 2);
        assert_eq!(plan.action, PlanAction::OverBudget);
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
        assert!(text.contains("|2-3|mathscape.marker.math-ocr.v1|marker-2.1.0|ensure-v3"));
        assert_eq!(text, idempotency_key(&key, range(2, 3)));
    }
}
