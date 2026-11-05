//! `material.grep`: coarse literal location inside named materials.
//!
//! Searches only the artifacts named in a non-empty `artifact_ids` list (no
//! workspace-wide grep). `pattern` is a case-sensitive literal substring, not
//! a regular expression; `terms` is not accepted. Matches do not overlap: the
//! next search starts at the end of the previous match.
//!
//! - `text/markdown` (and canonical markdown ids, resolved to their head
//!   snapshot): the body is scanned; each hit's [`Locator`] is an
//!   `md_byte_range` covering that match, so a follow-up `material.read`
//!   quotes it.
//! - `application/pdf` with a current-lineage canonical head (the lineage the
//!   map and search fold call 「规范解析正文」, chosen by
//!   [`CanonicalAliasTable`](mathscape_query::CanonicalAliasTable)) whose
//!   managed markdown copy is readable: every markdown snapshot in that
//!   lineage's coverage ledger is scanned, not only the head — the head
//!   first, then the newest snapshots that add pages the chosen ones do not
//!   cover yet (a snapshot that adds nothing is skipped, so overlapping
//!   re-parses are not scanned twice). Hits carry `artifact_id` = the
//!   physical snapshot that matched, an `md_byte_range` locator into it, plus
//!   `canonical_id`, `coverage` and `partial` (exactly as when that markdown
//!   id is grepped directly, so the pair goes to `material.read` unchanged).
//!   A `pdf_resolved_to_canonical` diagnostic lists the scanned snapshots,
//!   `searched_pages` (their page union), `text_layer_pages` and
//!   `unsearched_pages`.
//! - Pages of that PDF outside the snapshot union, and every page when no
//!   usable canonical head exists (no lineage with a head, head not in the
//!   catalog / not markdown / no managed copy): the native text layer
//!   extracted by `mathscape-pdf` (the same engine `pdf.outline` uses; never
//!   Marker) is scanned page by page. Each hit's locator is `pdf_pages` with
//!   `from == to ==` the page of the match, and `surface: "pdf_text_layer"`;
//!   the two locator shapes are never merged into one hit. The snippet is for
//!   localization only; the careful quote comes from `material.read` on that
//!   page, which serves canonical markdown. Matches spanning a page break are
//!   not found.
//!
//! A grep hit is a location, never quotable body text. `snippet` is a
//! verbatim window of the surrounding line (capped), not a second locator.
//! `score` is always [`LITERAL_SCORE`] — this scan does not rank.
//!
//! A missing id is business `not_found` with no partial hits. Other media is
//! `invalid_arg`. More matches than `max_hits`, or more than
//! [`MAX_SCAN_BYTES`] of one markdown snapshot's text or of one PDF's text
//! layer, is business `truncated`.

use std::collections::HashSet;

use mathscape_agent::{Locator, ToolError, ToolErrorCode, ToolStatus};
use mathscape_artifact::{ArtifactPool, ArtifactPoolError};
use mathscape_domain::{
    CanonicalMarkdownView, InclusivePageRange, MarkdownCoverage, StableId, covers_range,
    merge_page_ranges, missing_pages,
};
use mathscape_pdf::{PdfEngine, PdfOpenOptions, PdfPageIndex, PdfSelection};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::canonical::CanonicalIndex;
use super::material::{
    self, MARKDOWN_MEDIA_TYPE, ManagedPathFailure, Material, PDF_MEDIA_TYPE, find_material,
    find_materials, managed_path_failure,
};
use super::{MATERIAL_GREP, diagnostic, parse_args, to_object};
use crate::McpWorkspace;

/// Default and hard maximum number of hits.
pub const MAX_GREP_HITS: usize = 50;
/// Bytes of one markdown snapshot's text, or of one PDF's text layer, that
/// one call will scan.
pub const MAX_SCAN_BYTES: u64 = 1024 * 1024;
/// PDF pages are extracted in batches so a large PDF stops at the byte cap.
const PDF_SCAN_BATCH: u32 = 32;
/// Value of [`GrepHit::surface`] for hits found in the native PDF text layer.
pub const PDF_TEXT_LAYER_SURFACE: &str = "pdf_text_layer";
/// `pattern` longer than this (in Unicode scalar values) is `invalid_arg`.
pub const MAX_PATTERN_CHARS: usize = 200;
/// Maximum ids in one call.
pub const MAX_ARTIFACT_IDS: usize = 32;
/// Snippet window, in UTF-8 bytes. The match itself may be shorter.
pub const SNIPPET_MAX_BYTES: usize = 512;

const LITERAL_SCORE: u32 = 1;

const DESCRIPTION: &str = "Coarse locate: search specific materials for a case-sensitive literal substring (not a regular expression). artifact_ids is required and non-empty; this tool does not scan the whole workspace. text/markdown: the body is searched and each hit locator is an md_byte_range of that match. A canonical markdown id is resolved to its head snapshot; hits on a canonical snapshot include canonical_id, coverage and partial. application/pdf, canonical first: when the PDF has a current canonical markdown head, every markdown snapshot in that lineage's coverage is searched (the union of coverage.covered, not only the head): each such hit has artifact_id = the physical snapshot that matched and an md_byte_range locator into it (pass both to material.read unchanged), plus canonical_id, coverage and partial; a pdf_resolved_to_canonical diagnostic lists the searched snapshots, searched_pages, text_layer_pages and unsearched_pages. Pages outside that union, or every page when there is no usable canonical head, are searched in the native PDF text layer page by page (fast, no OCR; scanned pages without a text layer never match, nor do matches across a page break); each such hit has surface pdf_text_layer and a pdf_pages locator {from,to} of the page of the match, never an md_byte_range. A grep hit is a location, not quotable body text: snippets (text-layer ones especially) are for localization only; to quote or build nodes, call material.read with the hit's artifact_id and locator (a pdf_pages read serves canonical markdown, parsing those pages first when needed). A missing artifact_id yields status not_found and no partial hits. Stops at max_hits (default and maximum 50), after the first 1 MiB of one markdown snapshot's text, or after 1 MiB of one PDF's text layer; either limit returns status truncated and truncated true. This tool never runs Marker. Read-only.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrepArgs {
    artifact_ids: Vec<String>,
    pattern: String,
    #[serde(default)]
    max_hits: Option<usize>,
}

#[derive(Debug, Serialize)]
struct GrepResult {
    status: ToolStatus,
    hits: Vec<GrepHit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diagnostics: Vec<Value>,
}

#[derive(Debug, Serialize)]
struct GrepHit {
    artifact_id: String,
    locator: Locator,
    snippet: String,
    score: u32,
    revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    canonical_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage: Option<MarkdownCoverage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    partial: Option<bool>,
    /// `pdf_text_layer` for PDF hits: the snippet is native extract, not
    /// canonical markdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    surface: Option<&'static str>,
}

struct RawHit {
    start: u64,
    end: u64,
    snippet: String,
}

pub(super) fn definition() -> Value {
    json!({
        "name": MATERIAL_GREP,
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "artifact_ids": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_ARTIFACT_IDS,
                    "items": { "type": "string" },
                    "description": "Stable ids to scan, in order. Required and non-empty. Duplicates are rejected. Every id must be text/markdown, a canonical markdown id, or application/pdf (the markdown snapshots of its current canonical lineage are scanned, covering every page in that lineage's coverage, plus the native text layer of the pages they do not cover; without a usable canonical head, the whole native text layer).",
                },
                "pattern": {
                    "type": "string",
                    "minLength": 1,
                    "description": format!(
                        "Case-sensitive literal substring, not a regular expression. At most {MAX_PATTERN_CHARS} characters."
                    ),
                },
                "max_hits": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_GREP_HITS,
                    "default": MAX_GREP_HITS,
                    "description": format!(
                        "Maximum matches to return (1 to {MAX_GREP_HITS}). Defaults to {MAX_GREP_HITS}. Further matches yield status truncated."
                    ),
                },
            },
            "required": ["artifact_ids", "pattern"],
            "additionalProperties": false,
        },
    })
}

pub(super) fn call(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: GrepArgs = parse_args(arguments)?;
    let max_hits = validated_max_hits(args.max_hits)?;
    validate_pattern(&args.pattern)?;
    let ids = parse_ids(&args.artifact_ids)?;
    let index = CanonicalIndex::load(workspace)?;
    let resolved: Vec<StableId> = ids
        .iter()
        .map(|id| index.markdown_target(id).clone())
        .collect();
    let materials = load_materials(workspace, &resolved)?;
    if let Some(missing) = materials.missing {
        return not_found("artifact_not_found", "no artifact with this id", missing);
    }
    let materials = materials.found;
    ensure_supported(&materials)?;

    let pool = workspace.pool()?;
    let mut out = HitSink {
        pattern: &args.pattern,
        hits: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut hit_limited = false;
    let mut scan_capped = false;
    for material in &materials {
        let room = max_hits - out.hits.len();
        let scan = if material.media_type == PDF_MEDIA_TYPE {
            grep_pdf(workspace, &pool, &index, material, room, &mut out)?
        } else {
            let attachment = index.for_markdown(material.id.as_str()).map(|view| {
                let coverage = view.coverage(None);
                (view.canonical_id.to_string(), coverage.partial, coverage)
            });
            scan_markdown(&pool, material, attachment, room, &mut out)?
        };
        match scan {
            Scan::Scanned { limited, capped } => {
                scan_capped |= capped;
                if limited {
                    hit_limited = true;
                    break;
                }
            }
            Scan::Finished(value) => return Ok(value),
        }
    }
    let HitSink {
        hits,
        mut diagnostics,
        ..
    } = out;
    if hit_limited {
        diagnostics.push(diagnostic(
            "hit_limit",
            format!("stopped after {max_hits} matches; more matches may exist"),
        ));
    }
    let truncated = hit_limited || scan_capped;
    result(GrepResult {
        status: if truncated {
            ToolStatus::Truncated
        } else {
            ToolStatus::Ok
        },
        hits,
        truncated: Some(truncated),
        details: None,
        diagnostics,
    })
}

type Attachment = (String, bool, MarkdownCoverage);

/// Pattern plus the hits and diagnostics accumulated across materials.
struct HitSink<'p> {
    pattern: &'p str,
    hits: Vec<GrepHit>,
    diagnostics: Vec<Value>,
}

/// Outcome of scanning one material (or one surface of it).
enum Scan {
    Scanned { limited: bool, capped: bool },
    Finished(Value),
}

/// Scans one markdown material (a markdown id, or a snapshot of a PDF's
/// canonical lineage) and appends `md_byte_range` hits on that material.
fn scan_markdown(
    pool: &ArtifactPool,
    material: &Material,
    attachment: Option<Attachment>,
    room: usize,
    out: &mut HitSink<'_>,
) -> Result<Scan, ToolError> {
    let (text, capped) = match load_text(pool, material)? {
        Loaded::Text { text, capped } => (text, capped),
        Loaded::Finished(value) => return Ok(Scan::Finished(value)),
    };
    if capped {
        out.diagnostics.push(json!({
            "code": "scan_capped",
            "message": format!(
                "scanned the first {} bytes of this markdown; the remainder was not searched",
                text.len()
            ),
            "artifact_id": material.id.to_string(),
            "scanned_bytes": text.len(),
            "total_bytes": material.byte_size,
        }));
    }
    let (found, limited) = scan_text(&text, out.pattern, room);
    for found in found {
        out.hits.push(GrepHit {
            artifact_id: material.id.to_string(),
            locator: Locator::MdByteRange {
                start: found.start,
                end: found.end,
            },
            snippet: found.snippet,
            score: LITERAL_SCORE,
            revision: material.revision.clone(),
            canonical_id: attachment.as_ref().map(|(id, _, _)| id.clone()),
            partial: attachment.as_ref().map(|(_, partial, _)| *partial),
            coverage: attachment.as_ref().map(|(_, _, coverage)| coverage.clone()),
            surface: None,
        });
    }
    Ok(Scan::Scanned { limited, capped })
}

/// A PDF id: its current canonical lineage's snapshots, then the native
/// text layer of every page those snapshots do not cover. Without a usable
/// canonical head, the whole text layer.
fn grep_pdf(
    workspace: &McpWorkspace,
    pool: &ArtifactPool,
    index: &CanonicalIndex,
    material: &Material,
    room: usize,
    out: &mut HitSink<'_>,
) -> Result<Scan, ToolError> {
    match canonical_plan(workspace, pool, index, material)? {
        CanonicalPlan::Usable(plan) => grep_canonical_pdf(pool, &plan, material, room, out),
        CanonicalPlan::Unusable {
            canonical_id,
            reason,
        } => {
            out.diagnostics.push(json!({
                "code": "canonical_head_unavailable",
                "message": "the PDF's canonical markdown head is not readable; fell back to the native text layer",
                "artifact_id": material.id.to_string(),
                "canonical_id": canonical_id,
                "reason": reason,
            }));
            grep_whole_text_layer(pool, index, material, room, out)
        }
        CanonicalPlan::None => grep_whole_text_layer(pool, index, material, room, out),
    }
}

/// Every page's text layer (no usable canonical head). A missing managed PDF
/// copy is the call's business `not_found`, as before.
fn grep_whole_text_layer(
    pool: &ArtifactPool,
    index: &CanonicalIndex,
    material: &Material,
    room: usize,
    out: &mut HitSink<'_>,
) -> Result<Scan, ToolError> {
    let scan = match scan_pdf(pool, material, out.pattern, room, &[])? {
        PdfScan::Scanned(scan) => scan,
        PdfScan::Finished(value) => return Ok(Scan::Finished(value)),
    };
    let capped = scan.capped.is_some();
    let limited = scan.limited;
    if let Some(cap) = &scan.capped {
        out.diagnostics.push(text_layer_capped(
            material,
            &scan.scanned_pages,
            cap,
            scan.total_pages,
        ));
    }
    push_text_layer_hits(
        material,
        scan.hits,
        |page| {
            let pages = InclusivePageRange::new(page, page).ok();
            index.for_pdf(material.id.as_str(), pages).map(|view| {
                let coverage = view.coverage(pages);
                (view.canonical_id.to_string(), coverage.partial, coverage)
            })
        },
        out,
    );
    Ok(Scan::Scanned { limited, capped })
}

fn grep_canonical_pdf(
    pool: &ArtifactPool,
    plan: &LineagePlan<'_>,
    material: &Material,
    room: usize,
    out: &mut HitSink<'_>,
) -> Result<Scan, ToolError> {
    let view = plan.view;
    let coverage = view.coverage(None);
    let partial = coverage.partial;
    let attachment: Attachment = (view.canonical_id.to_string(), partial, coverage);
    let base = out.hits.len();
    let left = |out: &HitSink<'_>| room.saturating_sub(out.hits.len() - base);
    // Filled in once both surfaces are known, ahead of per-surface notes.
    let slot = out.diagnostics.len();
    out.diagnostics.push(Value::Null);

    let mut searched = Vec::new();
    let mut searched_snapshots = Vec::new();
    let mut capped = false;
    let mut limited = false;
    for snapshot in &plan.snapshots {
        match scan_markdown(
            pool,
            &snapshot.material,
            Some(attachment.clone()),
            left(out),
            out,
        )? {
            Scan::Scanned {
                limited: snapshot_limited,
                capped: snapshot_capped,
            } => {
                capped |= snapshot_capped;
                searched.extend(snapshot.pages.iter().copied());
                searched_snapshots.push(json!({
                    "artifact_id": snapshot.material.id.to_string(),
                    "pages": snapshot.pages,
                    "head": snapshot.material.id == plan.head_id,
                }));
                if snapshot_limited {
                    limited = true;
                    break;
                }
            }
            Scan::Finished(value) => return Ok(Scan::Finished(value)),
        }
    }
    let searched = merge_page_ranges(searched);

    // Text layer of the pages no scanned snapshot covers. Failing to read it
    // must not discard the canonical hits, so problems become a diagnostic.
    let mut total_pages = view.total_pages;
    let mut text_layer_pages = Vec::new();
    let mut text_layer_unavailable = None;
    let gap_unknown_or_open =
        total_pages.is_none_or(|total| !missing_pages(total, &searched).is_empty());
    if !limited && gap_unknown_or_open {
        match scan_pdf(pool, material, out.pattern, left(out), &searched) {
            Ok(PdfScan::Scanned(scan)) => {
                total_pages = Some(scan.total_pages);
                if let Some(cap) = &scan.capped {
                    capped = true;
                    out.diagnostics.push(text_layer_capped(
                        material,
                        &scan.scanned_pages,
                        cap,
                        scan.total_pages,
                    ));
                }
                limited |= scan.limited;
                text_layer_pages = scan.scanned_pages;
                push_text_layer_hits(
                    material,
                    scan.hits,
                    |page| {
                        let pages = InclusivePageRange::new(page, page).ok();
                        let coverage = view.coverage(pages);
                        Some((view.canonical_id.to_string(), coverage.partial, coverage))
                    },
                    out,
                );
            }
            Ok(PdfScan::Finished(_)) => {
                text_layer_unavailable = Some("content_unavailable".to_owned());
            }
            Err(error) if error.code == ToolErrorCode::Internal => {
                text_layer_unavailable = Some(error.message);
            }
            Err(error) => return Err(error),
        }
    }
    let reached = merge_page_ranges(searched.iter().chain(&text_layer_pages).copied());
    let unsearched = total_pages.map_or_else(Vec::new, |total| missing_pages(total, &reached));

    let mut message = if searched.is_empty() {
        format!(
            "searched {} canonical markdown snapshot(s) of this PDF (no recorded page range)",
            searched_snapshots.len()
        )
    } else {
        format!(
            "searched {} canonical markdown snapshot(s) of this PDF covering pages {}",
            searched_snapshots.len(),
            ranges_label(&searched)
        )
    };
    if !text_layer_pages.is_empty() {
        message.push_str(&format!(
            "; pages {} have no canonical markdown and were searched in the native text layer (locate only, read them with material.read pdf_pages before quoting)",
            ranges_label(&text_layer_pages)
        ));
    }
    if text_layer_unavailable.is_some() {
        message.push_str("; the native text layer could not be read");
    }
    if !unsearched.is_empty() {
        message.push_str(&format!(
            "; pages {} were not searched",
            ranges_label(&unsearched)
        ));
    }
    let mut resolved = json!({
        "code": "pdf_resolved_to_canonical",
        "message": message,
        "artifact_id": material.id.to_string(),
        "canonical_id": view.canonical_id.to_string(),
        "head_artifact_id": plan.head_id.to_string(),
        "searched_pages": searched,
        "searched_snapshots": searched_snapshots,
        "text_layer_pages": text_layer_pages,
        "unsearched_pages": unsearched,
        "partial": partial,
    });
    if !plan.skipped.is_empty() {
        resolved["skipped_snapshots"] = Value::Array(plan.skipped.clone());
    }
    if let Some(reason) = text_layer_unavailable {
        resolved["text_layer_unavailable"] = json!(reason);
    }
    out.diagnostics[slot] = resolved;
    Ok(Scan::Scanned { limited, capped })
}

fn text_layer_capped(
    material: &Material,
    scanned_pages: &[InclusivePageRange],
    cap: &PdfScanCap,
    total_pages: u32,
) -> Value {
    json!({
        "code": "scan_capped",
        "message": format!(
            "scanned the text layer of pages {} ({} bytes); later pages were not searched",
            ranges_label(scanned_pages),
            cap.scanned_bytes
        ),
        "artifact_id": material.id.to_string(),
        "scanned_pages": scanned_pages,
        "scanned_bytes": cap.scanned_bytes,
        "total_pages": total_pages,
    })
}

/// Appends `pdf_pages` + `surface: pdf_text_layer` hits (never an
/// `md_byte_range`), and the locate-only diagnostic when there are any.
fn push_text_layer_hits(
    material: &Material,
    found: Vec<(u32, RawHit)>,
    attachment_for: impl Fn(u32) -> Option<Attachment>,
    out: &mut HitSink<'_>,
) {
    if found.is_empty() {
        return;
    }
    out.diagnostics.push(json!({
        "code": "pdf_text_layer_hits",
        "message": "hits come from the native PDF text layer (coarse locate, not canonical text); read the hit's pdf_pages with material.read for canonical markdown before quoting",
        "artifact_id": material.id.to_string(),
    }));
    for (page, found) in found {
        let attachment = attachment_for(page);
        out.hits.push(GrepHit {
            artifact_id: material.id.to_string(),
            locator: Locator::PdfPages {
                from: page,
                to: page,
            },
            snippet: found.snippet,
            score: LITERAL_SCORE,
            revision: material.revision.clone(),
            canonical_id: attachment.as_ref().map(|(id, _, _)| id.clone()),
            partial: attachment.as_ref().map(|(_, partial, _)| *partial),
            coverage: attachment.map(|(_, _, coverage)| coverage),
            surface: Some(PDF_TEXT_LAYER_SURFACE),
        });
    }
}

/// One physical markdown snapshot of the current lineage to scan.
struct PlannedSnapshot {
    material: Material,
    /// Merged 1-based pages this snapshot records (empty when none).
    pages: Vec<InclusivePageRange>,
}

struct LineagePlan<'a> {
    view: &'a CanonicalMarkdownView,
    head_id: StableId,
    /// In page order (a head without a recorded range first).
    snapshots: Vec<PlannedSnapshot>,
    /// Snapshots that would add pages but cannot be read; their pages fall
    /// back to the text layer.
    skipped: Vec<Value>,
}

/// Whether a PDF id can be served from its current canonical lineage.
enum CanonicalPlan<'a> {
    Usable(LineagePlan<'a>),
    /// A current lineage exists, but its head cannot be scanned.
    Unusable {
        canonical_id: String,
        reason: &'static str,
    },
    None,
}

/// Head first, then the newest snapshots (finished last) that add pages not
/// covered by those already chosen. A snapshot whose pages are already
/// covered is not scanned, so overlapping re-parses do not double hits.
fn canonical_plan<'a>(
    workspace: &McpWorkspace,
    pool: &ArtifactPool,
    index: &'a CanonicalIndex,
    pdf: &Material,
) -> Result<CanonicalPlan<'a>, ToolError> {
    let Some(view) = index.current_for_pdf(pdf.id.as_str()) else {
        return Ok(CanonicalPlan::None);
    };
    let Some(head_id) = view.head_artifact_id.clone() else {
        return Ok(CanonicalPlan::None);
    };
    let head = match snapshot_material(workspace, pool, &head_id)? {
        Ok(head) => head,
        Err(reason) => {
            return Ok(CanonicalPlan::Unusable {
                canonical_id: view.canonical_id.to_string(),
                reason: match reason {
                    "not_in_catalog" => "head_not_in_catalog",
                    "not_markdown" => "head_not_markdown",
                    other => other,
                },
            });
        }
    };
    let pages_of = |id: &StableId| {
        merge_page_ranges(
            view.segments
                .iter()
                .filter(|segment| &segment.markdown_artifact_id == id)
                .filter_map(|segment| segment.pages),
        )
    };
    let mut union = pages_of(&head_id);
    let mut snapshots = vec![PlannedSnapshot {
        material: head,
        pages: union.clone(),
    }];
    let mut others: Vec<_> = view
        .segments
        .iter()
        .filter(|segment| segment.pages.is_some() && segment.markdown_artifact_id != head_id)
        .collect();
    others.sort_by(|left, right| {
        right
            .finished_at
            .cmp(&left.finished_at)
            .then_with(|| right.conversion_id.cmp(&left.conversion_id))
    });
    let mut seen = HashSet::from([head_id.clone()]);
    let mut skipped = Vec::new();
    for segment in others {
        let id = &segment.markdown_artifact_id;
        if !seen.insert(id.clone()) {
            continue;
        }
        let pages = pages_of(id);
        if pages.iter().all(|range| covers_range(&union, *range)) {
            continue;
        }
        match snapshot_material(workspace, pool, id)? {
            Ok(material) => {
                union = merge_page_ranges(union.iter().chain(&pages).copied());
                snapshots.push(PlannedSnapshot { material, pages });
            }
            Err(reason) => skipped.push(json!({
                "artifact_id": id.to_string(),
                "pages": pages,
                "reason": reason,
            })),
        }
    }
    snapshots.sort_by_key(|snapshot| snapshot.pages.first().map(|range| (range.from, range.to)));
    Ok(CanonicalPlan::Usable(LineagePlan {
        view,
        head_id,
        snapshots,
        skipped,
    }))
}

/// A readable markdown snapshot, or why it cannot be scanned. An unsafe or
/// unreadable managed path is still a call error.
fn snapshot_material(
    workspace: &McpWorkspace,
    pool: &ArtifactPool,
    id: &StableId,
) -> Result<Result<Material, &'static str>, ToolError> {
    let Some(snapshot) = find_material(workspace, id)? else {
        return Ok(Err("not_in_catalog"));
    };
    if snapshot.media_type != MARKDOWN_MEDIA_TYPE {
        return Ok(Err("not_markdown"));
    }
    if let Err(error) = material::open_managed_path(pool, &snapshot.id) {
        return match managed_path_failure(&error) {
            ManagedPathFailure::NotFound => Ok(Err("content_unavailable")),
            _ => fail_load(&snapshot, error).map(|_| Err("content_unavailable")),
        };
    }
    Ok(Ok(snapshot))
}

/// `"1–3, 5"` for diagnostics.
fn ranges_label(ranges: &[InclusivePageRange]) -> String {
    ranges
        .iter()
        .map(|range| {
            if range.from == range.to {
                range.from.to_string()
            } else {
                format!("{}–{}", range.from, range.to)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn validated_max_hits(max_hits: Option<usize>) -> Result<usize, ToolError> {
    match max_hits {
        None => Ok(MAX_GREP_HITS),
        Some(value) if (1..=MAX_GREP_HITS).contains(&value) => Ok(value),
        Some(value) => Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("max_hits must be from 1 to {MAX_GREP_HITS}, got {value}"),
        )),
    }
}

fn validate_pattern(pattern: &str) -> Result<(), ToolError> {
    if pattern.is_empty() {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "pattern must be a non-empty literal substring",
        ));
    }
    let chars = pattern.chars().count();
    if chars > MAX_PATTERN_CHARS {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("pattern is longer than {MAX_PATTERN_CHARS} characters"),
        )
        .with_details(json!({ "characters": chars, "maximum": MAX_PATTERN_CHARS })));
    }
    Ok(())
}

fn parse_ids(raw: &[String]) -> Result<Vec<StableId>, ToolError> {
    if raw.is_empty() {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "artifact_ids must be a non-empty list; material.grep does not scan the whole workspace",
        ));
    }
    if raw.len() > MAX_ARTIFACT_IDS {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("artifact_ids accepts at most {MAX_ARTIFACT_IDS} ids"),
        )
        .with_details(json!({ "count": raw.len(), "maximum": MAX_ARTIFACT_IDS })));
    }
    let mut ids = Vec::with_capacity(raw.len());
    let mut seen = HashSet::new();
    for (index, raw_id) in raw.iter().enumerate() {
        let id = StableId::parse(raw_id.as_str()).map_err(|error| {
            ToolError::new(
                ToolErrorCode::InvalidArg,
                format!("artifact_ids[{index}] is not a valid stable id: {error}"),
            )
            .with_details(json!({ "artifact_id": raw_id }))
        })?;
        if !seen.insert(id.clone()) {
            return Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                "artifact_ids contains a duplicate",
            )
            .with_details(json!({ "artifact_id": id.to_string() })));
        }
        ids.push(id);
    }
    Ok(ids)
}

struct LoadedMaterials {
    found: Vec<Material>,
    missing: Option<Vec<String>>,
}

fn load_materials(
    workspace: &McpWorkspace,
    ids: &[StableId],
) -> Result<LoadedMaterials, ToolError> {
    let mut found = Vec::with_capacity(ids.len());
    let mut missing = Vec::new();
    for (id, material) in ids.iter().zip(find_materials(workspace, ids)?) {
        match material {
            Some(material) => found.push(material),
            None => missing.push(id.to_string()),
        }
    }
    Ok(LoadedMaterials {
        found,
        missing: if missing.is_empty() {
            None
        } else {
            Some(missing)
        },
    })
}

fn ensure_supported(materials: &[Material]) -> Result<(), ToolError> {
    let unsupported: Vec<Value> = materials
        .iter()
        .filter(|material| {
            material.media_type != MARKDOWN_MEDIA_TYPE && material.media_type != PDF_MEDIA_TYPE
        })
        .map(|material| {
            json!({
                "artifact_id": material.id.to_string(),
                "media_type": material.media_type,
            })
        })
        .collect();
    if unsupported.is_empty() {
        return Ok(());
    }
    Err(ToolError::new(
        ToolErrorCode::InvalidArg,
        format!("material.grep only scans {MARKDOWN_MEDIA_TYPE} and {PDF_MEDIA_TYPE}"),
    )
    .with_details(json!({
        "supported_media_types": [MARKDOWN_MEDIA_TYPE, PDF_MEDIA_TYPE],
        "artifacts": unsupported,
    })))
}

/// Text-layer scan of one PDF, or a finished business result (content missing).
enum PdfScan {
    Scanned(PdfScanned),
    Finished(Value),
}

struct PdfScanned {
    /// `(1-based page, hit)` in page order.
    hits: Vec<(u32, RawHit)>,
    limited: bool,
    capped: Option<PdfScanCap>,
    total_pages: u32,
    /// Merged pages whose text layer was actually scanned.
    scanned_pages: Vec<InclusivePageRange>,
}

struct PdfScanCap {
    scanned_bytes: u64,
}

fn pdf_error(error: mathscape_pdf::PdfError) -> ToolError {
    ToolError::new(
        ToolErrorCode::Internal,
        format!("failed to read PDF text layer: {error}"),
    )
}

/// Scans the native text layer page by page (never Marker), skipping the
/// 1-based pages in `skip` (already served from canonical markdown). Stops
/// at `room` hits (then only checks whether another match exists) or once
/// [`MAX_SCAN_BYTES`] of page text have been scanned.
fn scan_pdf(
    pool: &ArtifactPool,
    material: &Material,
    pattern: &str,
    room: usize,
    skip: &[InclusivePageRange],
) -> Result<PdfScan, ToolError> {
    let path = match material::open_managed_path(pool, &material.id) {
        Ok(path) => path,
        Err(error) => {
            return match fail_load(material, error)? {
                Loaded::Finished(value) => Ok(PdfScan::Finished(value)),
                Loaded::Text { .. } => unreachable!("fail_load never yields text"),
            };
        }
    };
    let engine = PdfEngine;
    let options = PdfOpenOptions::new();
    let total_pages = engine
        .probe(&path, &options)
        .map_err(pdf_error)?
        .page_count();
    let mut hits = Vec::new();
    let mut scanned = Vec::new();
    let mut scanned_bytes: u64 = 0;
    let finish = |hits, limited, capped, scanned: Vec<InclusivePageRange>| {
        Ok(PdfScan::Scanned(PdfScanned {
            hits,
            limited,
            capped,
            total_pages,
            scanned_pages: merge_page_ranges(scanned),
        }))
    };
    for target in missing_pages(total_pages, skip) {
        let mut batch_from = target.from;
        while batch_from <= target.to {
            let batch_to = batch_from.saturating_add(PDF_SCAN_BATCH - 1).min(target.to);
            let selection = PdfSelection::inclusive_range(
                total_pages,
                PdfPageIndex::new(batch_from - 1),
                PdfPageIndex::new(batch_to - 1),
            )
            .map_err(pdf_error)?;
            let pages = engine
                .extract_positioned_text(&path, &selection, &options)
                .map_err(pdf_error)?
                .page_texts();
            for (index, page_text) in pages {
                let page = index.get() + 1;
                if scanned_bytes + page_text.len() as u64 > MAX_SCAN_BYTES && scanned_bytes > 0 {
                    return finish(hits, false, Some(PdfScanCap { scanned_bytes }), scanned);
                }
                scanned_bytes += page_text.len() as u64;
                if let Ok(range) = InclusivePageRange::new(page, page) {
                    scanned.push(range);
                }
                let left = room.saturating_sub(hits.len());
                let (found, limited) = scan_text(&page_text, pattern, left);
                hits.extend(found.into_iter().map(|hit| (page, hit)));
                if limited {
                    return finish(hits, true, None, scanned);
                }
            }
            batch_from = batch_to + 1;
        }
    }
    finish(hits, false, None, scanned)
}

/// Scanned text, or a finished business result (content missing).
enum Loaded {
    Text { text: String, capped: bool },
    Finished(Value),
}

fn load_text(pool: &ArtifactPool, material: &Material) -> Result<Loaded, ToolError> {
    // Same managed-path check and error classes as material.read / pdf.outline.
    if let Err(error) = material::open_managed_path(pool, &material.id) {
        return fail_load(material, error);
    }
    if material.byte_size == 0 {
        return Ok(Loaded::Text {
            text: String::new(),
            capped: false,
        });
    }
    let capped = material.byte_size > MAX_SCAN_BYTES;
    let length = u32::try_from(material.byte_size.min(MAX_SCAN_BYTES))
        .map_err(|_| ToolError::new(ToolErrorCode::Internal, "markdown scan window exceeds u32"))?;
    let bytes = match pool.read_bytes(&material.id, 0, length) {
        Ok(bytes) => bytes,
        Err(error) => return fail_load(material, error),
    };
    if bytes.len() as u64 != u64::from(length) {
        return Err(ToolError::new(
            ToolErrorCode::Internal,
            "managed artifact is shorter than its catalog byte size",
        ));
    }
    let text = decode_scanned(&bytes, capped)?;
    Ok(Loaded::Text { text, capped })
}

/// Business `not_found` (`Ok`) or a call error (`Err`), classified by the
/// shared [`managed_path_failure`].
fn fail_load(material: &Material, error: ArtifactPoolError) -> Result<Loaded, ToolError> {
    match managed_path_failure(&error) {
        ManagedPathFailure::NotFound => content_unavailable(material).map(Loaded::Finished),
        ManagedPathFailure::Forbidden => Err(ToolError::new(
            ToolErrorCode::Forbidden,
            "managed artifact path is unsafe or is a symbolic link",
        )
        .with_details(json!({ "artifact_id": material.id.to_string() }))),
        ManagedPathFailure::Internal => Err(ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to read managed artifact: {error}"),
        )),
    }
}

fn content_unavailable(material: &Material) -> Result<Value, ToolError> {
    not_found(
        "content_unavailable",
        "the artifact is registered but has no readable managed copy",
        vec![material.id.to_string()],
    )
}

fn not_found(reason: &str, message: &str, artifact_ids: Vec<String>) -> Result<Value, ToolError> {
    let diagnostics = artifact_ids
        .iter()
        .map(|id| {
            json!({
                "code": reason,
                "message": message,
                "artifact_id": id,
            })
        })
        .collect();
    result(GrepResult {
        status: ToolStatus::NotFound,
        hits: Vec::new(),
        truncated: None,
        details: Some(json!({
            "reason": reason,
            "artifact_ids": artifact_ids,
        })),
        diagnostics,
    })
}

fn result(value: GrepResult) -> Result<Value, ToolError> {
    to_object(value).map(Value::Object)
}

/// Decodes a scanned prefix. When `capped` is set, an incomplete UTF-8
/// sequence at the cut is dropped; any other invalid sequence is an error.
fn decode_scanned(bytes: &[u8], capped: bool) -> Result<String, ToolError> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_owned()),
        Err(error) if capped && error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()])
                .map(str::to_owned)
                .map_err(utf8_error)
        }
        Err(error) => Err(utf8_error(error)),
    }
}

fn utf8_error(error: std::str::Utf8Error) -> ToolError {
    ToolError::new(
        ToolErrorCode::Internal,
        format!("markdown material is not valid UTF-8: {error}"),
    )
}

/// Non-overlapping literal search. `limited` is true when another match
/// exists after the ones returned.
fn scan_text(text: &str, pattern: &str, limit: usize) -> (Vec<RawHit>, bool) {
    if pattern.is_empty() {
        return (Vec::new(), false);
    }
    let mut hits = Vec::new();
    let mut from = 0;
    while hits.len() < limit {
        let Some(relative) = text[from..].find(pattern) else {
            return (hits, false);
        };
        let start = from + relative;
        let end = start + pattern.len();
        let (snippet_start, snippet_end) = snippet_bounds(text, start, end);
        hits.push(RawHit {
            start: start as u64,
            end: end as u64,
            snippet: text[snippet_start..snippet_end].to_owned(),
        });
        from = end;
    }
    (hits, text[from..].contains(pattern))
}

/// Verbatim snippet window: the whole line when it fits, otherwise a
/// char-aligned window around the match that stays within [`SNIPPET_MAX_BYTES`].
fn snippet_bounds(text: &str, start: usize, end: usize) -> (usize, usize) {
    debug_assert!(start < end && end <= text.len());
    debug_assert!(text.is_char_boundary(start) && text.is_char_boundary(end));

    let line_start = match text[..start].rfind('\n') {
        Some(index) => index + 1,
        None => 0,
    };
    let line_end = match text[end..].find('\n') {
        Some(index) => end + index,
        None => text.len(),
    };
    if end - start <= SNIPPET_MAX_BYTES && line_end - line_start <= SNIPPET_MAX_BYTES {
        return (line_start, line_end);
    }
    if end - start > SNIPPET_MAX_BYTES {
        let mut snippet_end = start + SNIPPET_MAX_BYTES;
        while snippet_end > start && !text.is_char_boundary(snippet_end) {
            snippet_end -= 1;
        }
        if snippet_end == start {
            snippet_end = text[start..]
                .chars()
                .next()
                .map(|ch| start + ch.len_utf8())
                .unwrap_or(end);
        }
        return (start, snippet_end);
    }

    let slack = SNIPPET_MAX_BYTES - (end - start);
    let avail_before = start - line_start;
    let avail_after = line_end - end;
    let mut before = slack / 2;
    if before > avail_before {
        before = avail_before;
    }
    let mut after = slack - before;
    if after > avail_after {
        let extra = after - avail_after;
        after = avail_after;
        before += extra.min(avail_before - before);
    }
    let mut snippet_start = start - before;
    let mut snippet_end = end + after;
    while !text.is_char_boundary(snippet_start) {
        snippet_start += 1;
    }
    while !text.is_char_boundary(snippet_end) {
        snippet_end -= 1;
    }
    if snippet_start > start {
        snippet_start = start;
    }
    if snippet_end < end {
        snippet_end = end;
    }
    while snippet_end - snippet_start > SNIPPET_MAX_BYTES && snippet_end > end {
        let mut next = snippet_end - 1;
        while next > end && !text.is_char_boundary(next) {
            next -= 1;
        }
        if next == snippet_end {
            break;
        }
        snippet_end = next;
    }
    while snippet_end - snippet_start > SNIPPET_MAX_BYTES && snippet_start < start {
        let mut next = snippet_start + 1;
        while next < start && !text.is_char_boundary(next) {
            next += 1;
        }
        if next == snippet_start {
            break;
        }
        snippet_start = next;
    }
    debug_assert!(snippet_start <= start && snippet_end >= end);
    debug_assert!(text.is_char_boundary(snippet_start) && text.is_char_boundary(snippet_end));
    debug_assert!(snippet_end - snippet_start <= SNIPPET_MAX_BYTES);
    (snippet_start, snippet_end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_scan_is_case_sensitive_non_overlapping_and_not_a_regex() {
        let (hits, limited) = scan_text("aaaa", "aa", 10);
        assert!(!limited);
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].start, hits[0].end), (0, 2));
        assert_eq!((hits[1].start, hits[1].end), (2, 4));

        let (hits, limited) = scan_text("aaaa", "aa", 1);
        assert!(limited);
        assert_eq!(hits.len(), 1);

        assert!(scan_text("Lemma", "lemma", 10).0.is_empty());
        assert!(scan_text("lemma", "l.mma", 10).0.is_empty());
        assert!(scan_text("aaa", "aa", 10).0.len() == 1);
    }

    #[test]
    fn multibyte_match_locator_is_the_exact_byte_range() {
        let text = "# 引理 Lemma\nnext\n";
        let (hits, limited) = scan_text(text, "引理", 10);
        assert!(!limited);
        assert_eq!(hits.len(), 1);
        let start = hits[0].start as usize;
        let end = hits[0].end as usize;
        assert_eq!(&text[start..end], "引理");
        assert_eq!(hits[0].snippet, "# 引理 Lemma");
        assert_eq!(hits[0].snippet.find("引理"), Some(2));
    }

    #[test]
    fn long_line_snippet_contains_the_match_and_stays_bounded() {
        let text = format!("{}NEEDLE{}", "α".repeat(400), "β".repeat(400));
        let start = text.find("NEEDLE").unwrap();
        let end = start + "NEEDLE".len();
        let (snippet_start, snippet_end) = snippet_bounds(&text, start, end);
        let snippet = &text[snippet_start..snippet_end];
        assert!(snippet.contains("NEEDLE"));
        assert!(snippet_start <= start && snippet_end >= end);
        assert!(snippet.len() <= SNIPPET_MAX_BYTES);
        assert!(text.is_char_boundary(snippet_start));
        assert!(text.is_char_boundary(snippet_end));

        let long = "a".repeat(600);
        let (snippet_start, snippet_end) = snippet_bounds(&long, 0, 600);
        assert_eq!(snippet_start, 0);
        assert!(snippet_end <= SNIPPET_MAX_BYTES);
        assert!(snippet_end > 0);
    }

    #[test]
    fn capped_prefix_drops_only_a_trailing_partial_character() {
        assert_eq!(decode_scanned(b"ab", false).unwrap(), "ab");
        assert_eq!(decode_scanned("α".as_bytes(), false).unwrap(), "α");
        assert!(decode_scanned(&[0xFF], false).is_err());
        assert!(decode_scanned(&[b'a', 0xFF], true).is_err());
        assert_eq!(decode_scanned(&[b'a', 0xE5, 0xBC], true).unwrap(), "a");
        assert_eq!(decode_scanned(&[0xCE], true).unwrap(), "");
    }
}
