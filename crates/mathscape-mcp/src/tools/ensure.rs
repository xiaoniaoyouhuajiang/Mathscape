//! Canonical markdown ensure: the internal path behind `material.read` on a
//! scanned PDF. Not an MCP tool.
//!
//! Synchronous: the `material.read` call that needs Marker waits for it.
//! Order: plan → probe Marker → claim lease (per-PDF lock + global slot +
//! rolling page budget, one SQLite `BEGIN IMMEDIATE`) → Marker on one
//! contiguous range (lease renewed on a heartbeat) → validate inventory →
//! promote new immutable artifacts → scoped registry sync for this PDF →
//! terminal receipt → the same call serves the new snapshot. A read that
//! finds the PDF (or every Marker slot) busy queues behind the live lease,
//! re-plans when it is released (usually `reused`) and never answers
//! "read again later". Every wait is bounded by [`EnsurePolicy::sync_timeout`];
//! past it the ensure is failed, its lease cleared and the read fails.
//! Artifact bytes are never rewritten; a new snapshot is a new
//! content-addressed artifact.
//!
//! No stitching: a request is served only by one snapshot whose pages fully
//! contain it, so the Marker range is the whole request, never just its gaps.
//! The gaps are still reported as `missing`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

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

use super::canonical::CanonicalIndex;
use crate::McpWorkspace;

/// Bumped when the ensure planning rules change; part of the idempotency key.
pub const ENSURE_POLICY_VERSION: &str = "ensure-v1";
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
pub(super) enum EnsureStatus {
    /// An existing snapshot already contains the request (possibly one that
    /// another read's ensure promoted while this read waited). Marker not run.
    Reused,
    /// Marker ran in this call and a new snapshot containing the request was
    /// promoted.
    Expanded,
    /// The request or the rolling budget is too large.
    BlockedBudget,
    /// Marker is not installed or not usable.
    CapabilityUnavailable,
    /// Marker, inventory validation, promotion or the sync deadline failed.
    /// The lease is released (or failed) before the read returns.
    Failed,
}

impl EnsureStatus {
    pub(super) fn diagnostic_code(self) -> &'static str {
        match self {
            Self::Reused => "canonical_ensure_reused",
            Self::Expanded => "canonical_ensure_expanded",
            Self::BlockedBudget => "canonical_ensure_blocked_budget",
            Self::CapabilityUnavailable => "canonical_ensure_capability_unavailable",
            Self::Failed => "canonical_ensure_failed",
        }
    }
}

/// Structured ensure outcome, embedded in `material.read` `details.ensure`.
#[derive(Clone, Debug, Serialize)]
pub(super) struct EnsureReport {
    pub status: EnsureStatus,
    pub requested: InclusivePageRange,
    /// Requested pages no snapshot covers (union of every lineage), as
    /// planned before this call parsed or waited.
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
            marker_range: None,
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
        let pages = self.marker_range.or_else(|| {
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
    let policy = workspace.ensure_policy();
    let started = Instant::now();
    let deadline = started + policy.sync_timeout;
    let mut first_plan: Option<EnsurePlan> = None;
    let mut waited_for: Option<CanonicalEnsureLease> = None;
    let mut marker: Option<MarkerReady> = None;
    loop {
        let plan = match CanonicalIndex::load(workspace) {
            Ok(index) => plan_ensure(
                &index.all_for_pdf(pdf.as_str(), Some(requested)),
                requested,
                total_pages,
                policy.max_pages_per_request,
            ),
            Err(error) => {
                let plan = first_plan.unwrap_or_else(|| EnsurePlan {
                    requested,
                    missing: vec![requested],
                    action: PlanAction::Run(requested),
                });
                let mut report = EnsureReport::new(
                    EnsureStatus::Failed,
                    &plan,
                    format!("failed to load the canonical registry: {}", error.message),
                );
                report.waited_for = waited_for;
                report.waited_ms = elapsed_ms(started);
                return report;
            }
        };
        // Report what was missing when the call started, not after a wait.
        let shown = first_plan.get_or_insert_with(|| plan.clone()).clone();
        let finish = |mut report: EnsureReport, waited_for: Option<CanonicalEnsureLease>| {
            report.waited_for = waited_for;
            report.waited_ms = elapsed_ms(started);
            report
        };
        let range = match plan.action {
            PlanAction::Reuse => {
                let message = if waited_for.is_some() {
                    "waited for the running parse of this PDF; its new immutable snapshot contains every requested page"
                } else {
                    "an existing immutable snapshot contains every requested page"
                };
                return finish(
                    EnsureReport::new(EnsureStatus::Reused, &shown, message),
                    waited_for,
                );
            }
            PlanAction::OverBudget => {
                let mut report = EnsureReport::new(
                    EnsureStatus::BlockedBudget,
                    &shown,
                    format!(
                        "request spans {} pages; ensure parses at most {} pages per read. Narrow pdf_pages",
                        requested.page_count(),
                        policy.max_pages_per_request
                    ),
                );
                report.max_pages_per_request = Some(policy.max_pages_per_request);
                return finish(report, waited_for);
            }
            PlanAction::Run(range) => range,
        };

        let ready = match marker.take() {
            Some(ready) => ready,
            None => match probe_marker(workspace) {
                Ok(ready) => ready,
                Err((message, probe)) => {
                    let mut report =
                        EnsureReport::new(EnsureStatus::CapabilityUnavailable, &shown, message);
                    report.probe = Some(probe);
                    return finish(report, waited_for);
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
                return finish(
                    EnsureReport::new(EnsureStatus::Failed, &shown, error.to_string()),
                    waited_for,
                );
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
        let mut report = EnsureReport::new(EnsureStatus::Failed, &shown, "");
        report.marker_range = Some(range);
        report.idempotency_key = Some(claim.idempotency_key.clone());
        report.canonical_id = Some(claim.canonical_id.to_string());

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
                report.message = format!("failed to claim the ensure lease: {error}");
                return finish(report, waited_for);
            }
            Ok(CanonicalEnsureClaimOutcome::BudgetExhausted {
                used_pages,
                window_pages,
            }) => {
                report.status = EnsureStatus::BlockedBudget;
                report.marker_range = None;
                report.message = format!(
                    "rolling ensure budget used {used_pages} of {window_pages} pages in the last {} minutes; wait for the window or read pages that are already parsed",
                    policy.budget_window.as_secs() / 60
                );
                return finish(report, waited_for);
            }
            Ok(CanonicalEnsureClaimOutcome::PdfBusy(lease)) => Some(lease),
            Ok(CanonicalEnsureClaimOutcome::SlotsBusy { .. }) => None,
            Ok(CanonicalEnsureClaimOutcome::Acquired) => {
                report.ensure_id = Some(ensure_id.clone());
                let job = EnsureJob {
                    workspace_root: workspace.path().to_owned(),
                    database: workspace.database_path(),
                    pdf: pdf.clone(),
                    pdf_path: pdf_path.to_owned(),
                    total_pages,
                    range,
                    installation: ready.installation,
                    marker_version: ready.version,
                    marker_timeout: policy
                        .marker_timeout
                        .min(deadline.saturating_duration_since(Instant::now()))
                        .max(Duration::from_secs(1)),
                    ensure_id,
                    cancellation: MarkerCancellation::new(),
                };
                let lineage_had_segments = CanonicalIndex::load(workspace).is_ok_and(|index| {
                    index.all_for_pdf(pdf.as_str(), None).iter().any(|view| {
                        view.canonical_id == claim.canonical_id && !view.segments.is_empty()
                    })
                });
                run_owned(job, &policy, deadline, &mut report, lineage_had_segments);
                return finish(report, waited_for);
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
            return finish(report, waited_for);
        }
        wait_for_release(workspace, busy.as_ref(), &policy, deadline);
    }
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

/// Runs a claimed job on a worker thread while this thread renews the lease
/// every heartbeat. On the deadline: cancel Marker, give the worker
/// [`CANCEL_GRACE`] to write its failed receipt, else fail the lease here.
fn run_owned(
    job: EnsureJob,
    policy: &EnsurePolicy,
    deadline: Instant,
    report: &mut EnsureReport,
    lineage_had_segments: bool,
) {
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
        report.status = EnsureStatus::Failed;
        report.message = format!("failed to start the ensure worker: {error}");
        return;
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
        Some(Ok(done)) => {
            report.status = EnsureStatus::Expanded;
            report.conversion_id = Some(done.conversion_id);
            report.markdown_artifact_id = Some(done.markdown_artifact_id);
            report.message = if lineage_had_segments {
                "Marker parsed the requested pages in this call; a new immutable snapshot was added to the canonical markdown".into()
            } else {
                "Marker parsed the requested pages in this call; the first snapshot of this canonical markdown was promoted".into()
            };
        }
        None => {
            // The worker did not answer within the cancel grace: fail the
            // lease here so nothing stays `running`.
            fail_lease(&database, &ensure_id, "sync ensure deadline exceeded");
            report.status = EnsureStatus::Failed;
            report.timed_out = true;
            report.message = timeout_message(range, policy);
        }
        Some(Err(error)) => {
            fail_lease(&database, &ensure_id, &error);
            report.status = EnsureStatus::Failed;
            report.timed_out = timed_out;
            report.message = if timed_out {
                format!("{}: {error}", timeout_message(range, policy))
            } else {
                error
            };
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
