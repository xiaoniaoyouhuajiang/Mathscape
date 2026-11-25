//! Server-side anchors of a staged proposal (Phase2 review, §5.6).
//!
//! The review UI never rebuilds a span from the raw proposal payload: every
//! anchor it shows — the snapshot, the conversion it is read through, the
//! byte range, the canonical block ids and the page — comes from here.
//!
//! * New spans use the compiled `CreateSourceSpan` drafts of the version
//!   (block ids and page are the compiler's, not the agent's). A blocked
//!   version that never compiled falls back to the proposal's own range and
//!   says so (`compiled: false`).
//! * Referenced spans (`mathscape:source-span/…`) are resolved against the
//!   head map, then against the compiled drafts of the investigation's other
//!   versions (a span staged by a dependency).
//! * The excerpt is read from the snapshot bytes at the range
//!   (`verified: true`); when that read fails the proposal's quote is shown
//!   instead and the UI labels it as unchecked.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use mathscape_artifact::ArtifactPool;
use mathscape_domain::{AnchoredMarkdown, MapEditSnapshot, StableId, UserChangeOperation};
use mathscape_mcp::investigation::record_operations;
use mathscape_store_sqlite::{PendingChangeSet, SqliteArtifactStore};
use serde::Serialize;
use serde_json::Value;

use crate::content::{content_info_at, markdown_document_at};

/// Longest excerpt read back for one anchor (spans created through MCP are
/// at most 4 KiB; older registered spans may be longer).
const MAX_EXCERPT_BYTES: u64 = 16 * 1024;
const SPAN_PREFIX: &str = "mathscape:source-span/";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AnchorRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AnchorPage {
    /// Zero-based PDF page index.
    pub pdf_page_index: u32,
    /// One-based page number shown to the user.
    pub page_number: u32,
}

/// One Markdown anchor of a proposal.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AnchorView {
    /// The span as the proposal names it: a local ref (without `$new:`) for
    /// a span the proposal creates, the stable id for a referenced one.
    pub span_ref: String,
    pub span_id: Option<String>,
    /// `new` (created by this version), `head` (already in the map) or
    /// `staged` (created by another staged version of the investigation).
    pub origin: String,
    pub artifact: String,
    pub conversion: Option<String>,
    pub range: AnchorRange,
    pub block_ids: Vec<String>,
    pub page: Option<AnchorPage>,
    pub granularity: Option<String>,
    pub excerpt: String,
    /// `excerpt` was read from the snapshot at `range`.
    pub verified: bool,
    /// Block ids and page come from the compiled operations.
    pub compiled: bool,
}

/// A snapshot read for review, with the conversion it was read through.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSetSnapshot {
    pub document: AnchoredMarkdown,
    pub conversion_id: Option<String>,
}

fn page_of(page: Option<&Value>) -> Option<AnchorPage> {
    let index = page?.get("pdf_page_index")?.as_u64()?;
    let index = u32::try_from(index).ok()?;
    Some(AnchorPage {
        pdf_page_index: index,
        page_number: index.saturating_add(1),
    })
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// A span locator (head payload or serialized draft) as an anchor skeleton.
/// `None` for a span without a Markdown locator (ink-only evidence).
fn locator_anchor(
    span_ref: &str,
    span_id: &str,
    origin: &str,
    locator: &Value,
) -> Option<AnchorView> {
    let artifact = locator.pointer("/markdown/artifact")?.as_str()?.to_owned();
    let range = locator.pointer("/markdown/byte_range")?;
    Some(AnchorView {
        span_ref: span_ref.to_owned(),
        span_id: Some(span_id.to_owned()),
        origin: origin.to_owned(),
        artifact,
        conversion: None,
        range: AnchorRange {
            start: range.get("start")?.as_u64()?,
            end: range.get("end")?.as_u64()?,
        },
        block_ids: strings(locator.get("marker_block_ids")),
        page: page_of(locator.get("page")),
        granularity: locator
            .get("granularity")
            .and_then(Value::as_str)
            .map(str::to_owned),
        excerpt: String::new(),
        verified: false,
        compiled: true,
    })
}

/// Compiled span drafts of one version: object id → serialized draft.
fn compiled_spans(item: &PendingChangeSet) -> BTreeMap<String, Value> {
    record_operations(&item.record)
        .into_iter()
        .filter_map(|operation| match operation {
            UserChangeOperation::CreateSourceSpan { object_id, draft } => {
                Some((object_id, serde_json::to_value(draft).ok()?))
            }
            _ => None,
        })
        .collect()
}

fn collect_span_ids(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) if text.starts_with(SPAN_PREFIX) => {
            if !out.contains(text) {
                out.push(text.clone());
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect_span_ids(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_span_ids(item, out)),
        _ => {}
    }
}

/// Resolution context shared by every version of one listing.
pub struct AnchorContext<'a> {
    root: &'a Path,
    store: &'a SqliteArtifactStore,
    head: &'a MapEditSnapshot,
    /// Compiled span drafts of every version of the investigation.
    staged: BTreeMap<String, Value>,
    conversions: BTreeMap<String, Option<String>>,
    pool: Option<ArtifactPool>,
    lang: mathscape_domain::DisplayLang,
}

impl<'a> AnchorContext<'a> {
    pub fn new(
        root: &'a Path,
        store: &'a SqliteArtifactStore,
        head: &'a MapEditSnapshot,
        versions: &[PendingChangeSet],
        lang: mathscape_domain::DisplayLang,
    ) -> Self {
        Self {
            root,
            store,
            head,
            staged: versions.iter().flat_map(compiled_spans).collect(),
            conversions: BTreeMap::new(),
            pool: ArtifactPool::open(root).ok(),
            lang,
        }
    }

    /// Every Markdown anchor of `item`, new spans first (proposal order),
    /// then referenced spans (first mention order).
    pub fn anchors(&mut self, item: &PendingChangeSet) -> Vec<AnchorView> {
        let record = &item.record;
        let compiled = compiled_spans(item);
        let staged_ids: BTreeMap<String, String> = record
            .result
            .get("staged_ids")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let mut anchors = Vec::new();
        let mut claimed = BTreeSet::new();
        let objects = record
            .proposal
            .get("proposed_objects")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for object in &objects {
            if object.get("kind").and_then(Value::as_str) != Some("source_span") {
                continue;
            }
            let Some(local_ref) = object.get("local_ref").and_then(Value::as_str) else {
                continue;
            };
            let payload = object.get("payload").cloned().unwrap_or(Value::Null);
            let artifact = payload.get("artifact_id").and_then(Value::as_str);
            let range = payload.get("md_byte_range").and_then(|range| {
                Some(AnchorRange {
                    start: range.get("start")?.as_u64()?,
                    end: range.get("end")?.as_u64()?,
                })
            });
            // The compiled draft for this local ref: by staged id, else by
            // the same snapshot and range.
            let draft = staged_ids
                .get(local_ref)
                .and_then(|id| compiled.get(id).map(|draft| (id.clone(), draft)))
                .or_else(|| {
                    compiled.iter().find_map(|(id, draft)| {
                        let same_artifact =
                            draft.pointer("/markdown/artifact").and_then(Value::as_str) == artifact;
                        let same_range = range.as_ref().is_some_and(|range| {
                            draft
                                .pointer("/markdown/byte_range/start")
                                .and_then(Value::as_u64)
                                == Some(range.start)
                                && draft
                                    .pointer("/markdown/byte_range/end")
                                    .and_then(Value::as_u64)
                                    == Some(range.end)
                        });
                        (same_artifact && same_range).then(|| (id.clone(), draft))
                    })
                });
            let quote = payload
                .get("quote")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let anchor = match draft {
                Some((id, draft)) => {
                    claimed.insert(id.clone());
                    let origin = if self.head_span(&id).is_some() {
                        "head"
                    } else {
                        "new"
                    };
                    locator_anchor(local_ref, &id, origin, draft).map(|mut anchor| {
                        anchor.excerpt = quote.clone();
                        if anchor.granularity.is_none() {
                            anchor.granularity = payload
                                .get("granularity")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                        }
                        anchor
                    })
                }
                None => match (artifact, range) {
                    (Some(artifact), Some(range)) => Some(AnchorView {
                        span_ref: local_ref.to_owned(),
                        span_id: None,
                        origin: "new".into(),
                        artifact: artifact.to_owned(),
                        conversion: None,
                        range,
                        block_ids: Vec::new(),
                        page: None,
                        granularity: payload
                            .get("granularity")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        excerpt: quote,
                        verified: false,
                        compiled: false,
                    }),
                    _ => None,
                },
            };
            if let Some(anchor) = anchor {
                anchors.push(anchor);
            }
        }
        let mut referenced = Vec::new();
        collect_span_ids(&record.proposal, &mut referenced);
        for object in &objects {
            let op = object.get("op").and_then(Value::as_str).unwrap_or("create");
            if matches!(op, "edit" | "retire")
                && let Some(target_id) = object.get("target_id").and_then(Value::as_str)
                && let Some(payload) = self.head_payload(target_id)
                && let Some(spans) = payload.get("evidence_spans")
            {
                collect_span_ids(spans, &mut referenced);
            }
        }
        for id in referenced {
            if claimed.contains(&id) {
                continue;
            }
            let anchor = match self.head_span(&id) {
                Some(locator) => locator_anchor(&id, &id, "head", &locator),
                None => self
                    .staged
                    .get(&id)
                    .and_then(|draft| locator_anchor(&id, &id, "staged", draft)),
            };
            if let Some(anchor) = anchor {
                anchors.push(anchor);
            }
        }
        for anchor in &mut anchors {
            anchor.conversion = self.conversion(&anchor.artifact);
            if let Some(text) = self.read_range(&anchor.artifact, &anchor.range) {
                anchor.excerpt = text;
                anchor.verified = true;
            }
        }
        anchors
    }

    /// Locator of a head span (`payload.payload` or `payload`), retired or not.
    fn head_span(&self, id: &str) -> Option<Value> {
        self.head
            .objects
            .iter()
            .find(|object| object.object_id == id)
            .map(|object| {
                object
                    .payload
                    .get("payload")
                    .unwrap_or(&object.payload)
                    .clone()
            })
    }

    /// Display title of a head object (for diagnostics and semantic/source references).
    pub fn head_title(&self, id: &str) -> Option<String> {
        let object = self
            .head
            .objects
            .iter()
            .find(|object| object.object_id == id)?;
        let title = mathscape_orchestrator::semantic_view::semantic_object_title_in(
            self.head, object, self.lang,
        );
        Some(if title == object.object_id {
            object.kind.clone()
        } else {
            title
        })
    }

    pub fn head_payload(&self, id: &str) -> Option<Value> {
        let object = self
            .head
            .objects
            .iter()
            .find(|object| object.object_id == id)?;
        Some(
            object
                .payload
                .get("payload")
                .unwrap_or(&object.payload)
                .clone(),
        )
    }

    pub fn head_object(&self, id: &str) -> Option<(String, Value)> {
        let object = self
            .head
            .objects
            .iter()
            .find(|object| object.object_id == id)?;
        let inner = object.payload.get("payload").unwrap_or(&object.payload);
        if object.kind == "SourceUnitRelation" {
            // A relation keeps its ends as endpoints, not in its payload; the
            // review needs them in the shape a proposal writes.
            let ends = |role: &str| {
                let mut ends: Vec<_> = object
                    .endpoints
                    .iter()
                    .filter(|end| end.role().as_str() == role)
                    .collect();
                ends.sort_by_key(|end| end.ordinal());
                ends.into_iter()
                    .map(|end| end.entity().to_string())
                    .collect::<Vec<_>>()
            };
            return Some((
                "source_relation".to_owned(),
                serde_json::json!({
                    "relation": inner.get("relation"),
                    "source": ends("source").first(),
                    "target": ends("target").first(),
                    "evidence_spans": ends("evidence"),
                }),
            ));
        }
        let kind = match object
            .payload
            .get("payload_kind")
            .and_then(Value::as_str)
            .unwrap_or(object.kind.as_str())
        {
            "source_semantic_unit" | "SourceSemanticUnit" => "source_unit",
            "semantic_entity" | "SemanticEntity" => "semantic_entity",
            "representation_core" | "RepresentationCore" => "representation_core",
            "alignment" | "Alignment" => "alignment",
            "representation_binding" | "RepresentationBinding" => "representation_binding",
            "debt" | "Debt" => "debt",
            "dialect_scope" | "DialectScope" => "dialect_scope",
            "source_span" | "SourceSpan" => "source_span",
            other => other,
        }
        .to_owned();
        Some((kind, inner.clone()))
    }

    pub fn lang(&self) -> mathscape_domain::DisplayLang {
        self.lang
    }

    pub fn head(&self) -> &MapEditSnapshot {
        self.head
    }

    fn conversion(&mut self, artifact: &str) -> Option<String> {
        if let Some(cached) = self.conversions.get(artifact) {
            return cached.clone();
        }
        let resolved = snapshot_conversion(self.root, self.store, artifact)
            .ok()
            .flatten();
        self.conversions
            .insert(artifact.to_owned(), resolved.clone());
        resolved
    }

    fn read_range(&self, artifact: &str, range: &AnchorRange) -> Option<String> {
        if range.end < range.start || range.end - range.start > MAX_EXCERPT_BYTES {
            return None;
        }
        let id = StableId::parse(artifact).ok()?;
        let pool = self.pool.as_ref()?;
        let active_errata = self
            .store
            .list_active_markdown_errata(artifact)
            .ok()
            .unwrap_or_default();
        let has_overlap = active_errata
            .iter()
            .any(|row| row.base_byte_end > range.start && row.base_byte_start < range.end);
        if has_overlap
            && let Ok(info) = content_info_at(self.root, artifact)
            && info.byte_size <= 16 * 1024 * 1024
            && let Ok(full_len) = u32::try_from(info.byte_size)
            && let Ok(full_bytes) = pool.read_bytes(&id, 0, full_len)
            && let Ok(full_source) = String::from_utf8(full_bytes)
            && (range.end as usize) <= full_source.len()
            && full_source.is_char_boundary(range.start as usize)
            && full_source.is_char_boundary(range.end as usize)
        {
            let entries = crate::markdown_errata::to_overlay_entries(&active_errata);
            let view = mathscape_orchestrator::render_effective_slice(
                &full_source,
                range.start as usize..range.end as usize,
                &entries,
            );
            return Some(view.effective_source);
        }
        let length = u32::try_from(range.end - range.start).ok()?;
        let bytes = pool.read_bytes(&id, range.start, length).ok()?;
        (bytes.len() == length as usize)
            .then(|| String::from_utf8(bytes).ok())
            .flatten()
    }
}

/// The conversion a snapshot is read through for review: the unique
/// conversion `canonical_markdown_coverage` records for it, else its unique
/// conversion context, else none when it has no context at all.
pub fn snapshot_conversion(
    root: &Path,
    store: &SqliteArtifactStore,
    artifact: &str,
) -> Result<Option<String>, String> {
    let info = content_info_at(root, artifact)?;
    let contexts: Vec<&str> = info
        .contexts
        .iter()
        .map(|context| context.conversion_id.as_str())
        .collect();
    let covered: BTreeSet<String> = store
        .list_canonical_markdown()
        .map_err(|error| error.to_string())?
        .iter()
        .flat_map(|view| view.segments.iter())
        .filter(|segment| segment.markdown_artifact_id.as_str() == artifact)
        .map(|segment| segment.conversion_id.to_string())
        .filter(|conversion| contexts.contains(&conversion.as_str()))
        .collect();
    if covered.len() == 1 {
        return Ok(covered.into_iter().next());
    }
    match contexts.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some((*only).to_owned())),
        many => Err(format!(
            "conversion_ambiguous: 快照 {artifact} 有 {} 个转换来源，canonical 覆盖登记也不能唯一确定",
            many.len()
        )),
    }
}

/// Reads a snapshot for reviewing the proposals of `investigation_id` in
/// `root` (the investigation's own workspace, possibly a sandbox copy).
/// Only snapshots some version of the investigation anchors into are
/// served, so this never becomes a general file reader.
pub fn change_set_snapshot(
    root: &Path,
    investigation_id: &str,
    artifact_id: &str,
    lang: mathscape_domain::DisplayLang,
) -> Result<ChangeSetSnapshot, String> {
    let store = SqliteArtifactStore::open(&root.join("mathscape.sqlite3"))
        .map_err(|error| error.to_string())?;
    let versions = store
        .list_change_set_versions(investigation_id)
        .map_err(|error| error.to_string())?;
    let head = store
        .map_edit_snapshot()
        .map_err(|error| error.to_string())?;
    let mut context = AnchorContext::new(root, &store, &head, &versions, lang);
    let referenced = versions.iter().any(|item| {
        context
            .anchors(item)
            .iter()
            .any(|anchor| anchor.artifact == artifact_id)
    });
    if !referenced {
        return Err(format!(
            "snapshot_not_referenced: 快照 {artifact_id} 未被调查 {investigation_id} 的任何提案引用"
        ));
    }
    let conversion_id = snapshot_conversion(root, &store, artifact_id)?;
    let document = markdown_document_at(root, artifact_id, conversion_id.as_deref())?;
    Ok(ChangeSetSnapshot {
        document,
        conversion_id,
    })
}
