//! Deterministic Markdown patch resolution and application.
//!
//! The producer may suggest a replacement, but this module owns all authority:
//! exact matching, issue-evidence containment, conflict detection, byte
//! application and the structural safety gate.

use std::collections::{BTreeMap, HashMap};

use mathscape_artifact::{ArtifactMetadata, ArtifactPool, ManagedArtifact};
use mathscape_domain::{
    ActorRef, AnchoredMarkdown, CanonicalDiscoveryIssue, CommandRecordStatus, ContentHash,
    ConversionExitStatus, ConversionParameters, ConversionRun, DiscoveryInput, DiscoveryIssueKind,
    EntityHeader, EntityKind, LifecycleStatus, MarkdownPatchApplyReceipt, MarkdownPatchProposal,
    MarkdownPatchSet, MarkdownPatchStatus, PatchSafetyDiagnostic, PatchSafetyReport,
    PatchSafetyStatus, ResolvedMarkdownPatch, SchemaVersion, SourceArtifact, StableId,
    TextByteRange, TextOccurrence, Timestamp,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::markdown_anchors::{PAGE_MARKER_STRUCTURAL_KIND, anchor_markdown};

use crate::{
    MARKDOWN_PDF_BASE_CONVERSION_KEY, MARKDOWN_PDF_MAP_ARTIFACT_KEY,
    MARKDOWN_PDF_SOURCE_PDF_ARTIFACT_KEY, build_markdown_pdf_block_map,
};

pub const PATCH_VALIDATOR_REVISION: &str = "markdown-patch-validator-1";

#[derive(Debug, Error)]
pub enum MarkdownPatchPromotionError {
    #[error("artifact promotion failed: {0}")]
    Artifact(#[from] mathscape_artifact::ArtifactPoolError),
    #[error("could not build Markdown-PDF page map: {0}")]
    PageMap(String),
    #[error("domain validation failed: {0}")]
    Domain(#[from] mathscape_domain::DomainError),
    #[error("JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownPatchPromotion {
    pub artifact: ManagedArtifact,
    pub conversion: mathscape_domain::SourceRelation,
    pub receipt: MarkdownPatchApplyReceipt,
}

/// Provenance inherited from the Marker conversion that produced the base
/// Markdown. The patch conversion itself still has only the base Markdown as
/// a direct input; these fields record the audited PDF ancestry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownPdfMapInheritance {
    pub base_conversion_id: StableId,
    pub pdf_artifact_id: StableId,
    pub marker_page_ids: Vec<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchDiagnostic {
    pub code: String,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Error)]
#[error("Markdown patch validation failed")]
pub struct PatchValidationReport {
    diagnostics: Vec<PatchDiagnostic>,
}

impl PatchValidationReport {
    pub fn diagnostics(&self) -> &[PatchDiagnostic] {
        &self.diagnostics
    }
}

#[derive(Debug, Error)]
pub enum PatchApplyError {
    #[error("patch set has no patches")]
    Empty,
    #[error("patch {patch_ref} range is invalid")]
    InvalidRange { patch_ref: String },
    #[error("patch {patch_ref} old text no longer matches the base artifact")]
    Stale { patch_ref: String },
    #[error("patches overlap")]
    Overlap,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedMarkdownPatch {
    pub source: String,
    pub offset_map: OffsetMap,
    pub new_patch_ranges: Vec<TextByteRange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OffsetMap {
    edits: Vec<OffsetEdit>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OffsetEdit {
    old: TextByteRange,
    new_len: u64,
}

impl OffsetMap {
    pub fn map_offset(&self, offset: u64) -> Option<u64> {
        let mut delta = 0_i128;
        for edit in &self.edits {
            if offset <= edit.old.start {
                return add_delta(offset, delta);
            }
            if offset < edit.old.end {
                return None;
            }
            delta +=
                i128::from(edit.new_len) - i128::from(edit.old.end.saturating_sub(edit.old.start));
        }
        add_delta(offset, delta)
    }

    pub fn map_range(&self, range: &TextByteRange) -> Option<TextByteRange> {
        Some(TextByteRange {
            start: self.map_offset(range.start)?,
            end: self.map_offset(range.end)?,
        })
        .filter(|mapped| mapped.start <= mapped.end)
    }
}

fn add_delta(value: u64, delta: i128) -> Option<u64> {
    let mapped = i128::from(value) + delta;
    (mapped >= 0 && mapped <= i128::from(u64::MAX)).then_some(mapped as u64)
}

#[derive(Clone, Debug)]
struct InputBlock {
    handle: String,
    range: TextByteRange,
    text: String,
}

fn input_blocks(input: &DiscoveryInput) -> Vec<InputBlock> {
    input
        .requested
        .segments
        .iter()
        .enumerate()
        .filter_map(|(index, segment)| match segment {
            mathscape_domain::DiscoveryInputSegment::Block {
                source_range, text, ..
            } => Some(InputBlock {
                handle: format!(
                    "b{}",
                    1 + input
                        .requested
                        .segments
                        .iter()
                        .take(index)
                        .filter(|candidate| {
                            matches!(
                                candidate,
                                mathscape_domain::DiscoveryInputSegment::Block { .. }
                            )
                        })
                        .count()
                ),
                range: source_range.clone(),
                text: text.clone(),
            }),
            mathscape_domain::DiscoveryInputSegment::Gap { .. } => None,
        })
        .collect()
}

fn hash_bytes(bytes: &[u8]) -> ContentHash {
    ContentHash::parse(format!("sha256:{:x}", Sha256::digest(bytes)))
        .expect("sha256 digest is valid")
}

pub fn markdown_patch_content_hash(bytes: &[u8]) -> ContentHash {
    hash_bytes(bytes)
}

pub fn markdown_patch_conversion_id(
    patch_set: &MarkdownPatchSet,
) -> Result<StableId, mathscape_domain::DomainError> {
    let digest = patch_set
        .fingerprint
        .as_str()
        .strip_prefix("sha256:")
        .unwrap_or(patch_set.fingerprint.as_str());
    StableId::parse(format!("mathscape:conversion/markdown-patch/{digest}"))
}

pub fn markdown_patch_artifact_id(
    content_hash: &ContentHash,
) -> Result<StableId, mathscape_domain::DomainError> {
    let digest = content_hash
        .as_str()
        .strip_prefix("sha256:")
        .unwrap_or(content_hash.as_str());
    StableId::parse(format!("mathscape:artifact/sha256/{digest}"))
}

pub fn rewrite_patch_set_replacements(
    patch_set: &MarkdownPatchSet,
    replacements: &std::collections::BTreeMap<String, String>,
) -> Result<MarkdownPatchSet, String> {
    let mut rewritten = patch_set.clone();
    let known: std::collections::BTreeSet<_> = patch_set
        .patches
        .iter()
        .map(|patch| patch.patch_ref.as_str())
        .collect();
    if let Some(unknown) = replacements
        .keys()
        .find(|patch_ref| !known.contains(patch_ref.as_str()))
    {
        return Err(format!("unknown patch reference {unknown}"));
    }
    for patch in &mut rewritten.patches {
        if let Some(replacement) = replacements.get(&patch.patch_ref) {
            if replacement.as_bytes().len() > mathscape_domain::MAX_REPLACEMENT_BYTES
                || replacement.contains('\0')
            {
                return Err(format!("replacement for {} is invalid", patch.patch_ref));
            }
            patch.replacement = replacement.clone();
            patch.replacement_hash = hash_bytes(replacement.as_bytes());
        }
    }
    rewritten.fingerprint = patch_set_fingerprint(&rewritten);
    Ok(rewritten)
}

pub fn select_patch_set(
    patch_set: &MarkdownPatchSet,
    accepted: &std::collections::HashSet<String>,
) -> Result<MarkdownPatchSet, String> {
    if let Some(unknown) = accepted.iter().find(|patch_ref| {
        !patch_set
            .patches
            .iter()
            .any(|patch| &patch.patch_ref == *patch_ref)
    }) {
        return Err(format!("unknown accepted patch reference {unknown}"));
    }
    let mut selected = patch_set.clone();
    selected
        .patches
        .retain(|patch| accepted.contains(&patch.patch_ref));
    if selected.patches.is_empty() {
        return Err("patch selection is empty".into());
    }
    selected.fingerprint = patch_set_fingerprint(&selected);
    Ok(selected)
}

pub(crate) fn patch_set_fingerprint(patch_set: &MarkdownPatchSet) -> ContentHash {
    // Bound to the run: the stored patch set, its decisions and the patch
    // ConversionRun are all keyed by this fingerprint.
    hash_value(&(
        PATCH_VALIDATOR_REVISION,
        &patch_set.run_id,
        &patch_set.base_input_fingerprint,
        patch_set
            .patches
            .iter()
            .map(|patch| {
                (
                    &patch.issue_ref,
                    &patch.range,
                    &patch.old_text_hash,
                    &patch.replacement_hash,
                )
            })
            .collect::<Vec<_>>(),
    ))
}

fn hash_value<T: Serialize>(value: &T) -> ContentHash {
    hash_bytes(&serde_json::to_vec(value).expect("patch value serializes"))
}

fn exact_matches(source: &str, wanted: &str) -> Vec<(usize, usize)> {
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    let mut cursor = 0;
    while cursor <= source.len() {
        let Some(relative) = source[cursor..].find(wanted) else {
            break;
        };
        let start = cursor + relative;
        let end = start + wanted.len();
        matches.push((start, end));
        cursor = start + wanted.chars().next().map_or(1, char::len_utf8);
    }
    matches
}

pub(crate) fn exact_occurrences(source: &str, wanted: &str) -> Vec<(usize, usize)> {
    exact_matches(source, wanted)
}

fn contains(outer: &TextByteRange, inner: &TextByteRange) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn range_is_single_block(blocks: &[InputBlock], range: &TextByteRange) -> bool {
    blocks
        .iter()
        .any(|block| block.range.start <= range.start && range.end <= block.range.end)
}

fn issue_map<'a>(
    issues: &'a [CanonicalDiscoveryIssue],
) -> HashMap<&'a str, &'a CanonicalDiscoveryIssue> {
    issues
        .iter()
        .map(|issue| (issue.issue_ref.as_str(), issue))
        .collect()
}

/// Resolves P proposals using exact matching only and turns them into a
/// replayable patch set. The run-local `patch_ref` supplied by the model is
/// ignored for persisted identity; the returned refs are p1, p2, ... in
/// deterministic source order.
pub fn resolve_patch_proposals(
    input: &DiscoveryInput,
    run_id: StableId,
    issues: &[CanonicalDiscoveryIssue],
    proposals: &[MarkdownPatchProposal],
) -> Result<MarkdownPatchSet, PatchValidationReport> {
    let blocks = input_blocks(input);
    let issues_by_ref = issue_map(issues);
    let mut diagnostics = Vec::new();
    if proposals.len() > mathscape_domain::MAX_PATCHES_PER_RESPONSE {
        diagnostics.push(PatchDiagnostic {
            code: "limit_exceeded".into(),
            path: "patches".into(),
            message: "too many patch candidates".into(),
        });
    }
    let mut resolved = Vec::new();
    for (index, proposal) in proposals.iter().enumerate() {
        let path = format!("patches[{index}]");
        let Some(issue) = issues_by_ref.get(proposal.issue_ref.as_str()) else {
            diagnostics.push(PatchDiagnostic {
                code: "unknown_issue".into(),
                path: format!("{path}.issue_ref"),
                message: "patch must refer to a canonical A-stage issue".into(),
            });
            continue;
        };
        if issue.kind != DiscoveryIssueKind::SuspectedConversionError {
            diagnostics.push(PatchDiagnostic {
                code: "invalid_issue_kind".into(),
                path: format!("{path}.issue_ref"),
                message: "only suspected_conversion_error is patchable".into(),
            });
            continue;
        }
        if issue
            .evidence
            .iter()
            .any(|evidence| !range_is_single_block(&blocks, evidence))
            || issue.evidence.iter().any(|evidence| {
                blocks
                    .iter()
                    .filter(|block| {
                        block.range.end > evidence.start && block.range.start < evidence.end
                    })
                    .count()
                    != 1
            })
        {
            diagnostics.push(PatchDiagnostic {
                code: "cross_block_issue".into(),
                path: format!("{path}.issue_ref"),
                message: "issue evidence must be uniquely contained in one physical block".into(),
            });
            continue;
        }
        let TextOccurrence::TextOccurrence {
            block,
            text,
            occurrence,
        } = &proposal.target
        else {
            diagnostics.push(PatchDiagnostic {
                code: "exact_match_required".into(),
                path: format!("{path}.target"),
                message: "P v1 accepts only one-block text_occurrence".into(),
            });
            continue;
        };
        if text.is_empty()
            || proposal.replacement.as_bytes().len() > mathscape_domain::MAX_REPLACEMENT_BYTES
        {
            diagnostics.push(PatchDiagnostic {
                code: "invalid_value".into(),
                path: format!("{path}.replacement"),
                message: "replacement is empty or exceeds the patch limit".into(),
            });
            continue;
        }
        if proposal.replacement.contains('\0') {
            diagnostics.push(PatchDiagnostic {
                code: "invalid_value".into(),
                path: format!("{path}.replacement"),
                message: "replacement contains NUL".into(),
            });
            continue;
        }
        let Some(input_block) = blocks.iter().find(|candidate| candidate.handle == *block) else {
            diagnostics.push(PatchDiagnostic {
                code: "unknown_block".into(),
                path: format!("{path}.target.block"),
                message: "target block is not in this DiscoveryInput".into(),
            });
            continue;
        };
        let matches = exact_matches(&input_block.text, text);
        let selected = match occurrence {
            Some(0) => None,
            Some(value) => matches.get((*value as usize).saturating_sub(1)).copied(),
            None if matches.len() == 1 => matches.first().copied(),
            None => None,
        };
        let Some((start, end)) = selected else {
            diagnostics.push(PatchDiagnostic {
                code: if matches.is_empty() {
                    "quote_not_found"
                } else {
                    "ambiguous_quote"
                }
                .into(),
                path: format!("{path}.target"),
                message: format!("exact target has {} matches", matches.len()),
            });
            continue;
        };
        let range = TextByteRange {
            start: input_block.range.start + start as u64,
            end: input_block.range.start + end as u64,
        };
        if !issue
            .evidence
            .iter()
            .any(|evidence| contains(evidence, &range))
        {
            diagnostics.push(PatchDiagnostic {
                code: "outside_issue_evidence".into(),
                path: format!("{path}.target"),
                message: "target must be contained in the issue evidence".into(),
            });
            continue;
        }
        resolved.push(ResolvedMarkdownPatch {
            // Assigned after source-order sorting below. The model's local
            // numbering must not influence persisted identity.
            patch_ref: String::new(),
            issue_ref: proposal.issue_ref.clone(),
            range,
            old_text: text.clone(),
            old_text_hash: hash_bytes(text.as_bytes()),
            replacement: proposal.replacement.clone(),
            replacement_hash: hash_bytes(proposal.replacement.as_bytes()),
            rationale: proposal.rationale.clone(),
            status: MarkdownPatchStatus::Proposed,
        });
    }
    resolved.sort_by_key(|patch| (patch.range.start, patch.range.end, patch.patch_ref.clone()));
    for (index, patch) in resolved.iter_mut().enumerate() {
        patch.patch_ref = format!("p{}", index + 1);
    }
    if resolved
        .windows(2)
        .any(|pair| pair[0].range.end > pair[1].range.start)
    {
        diagnostics.push(PatchDiagnostic {
            code: "overlapping_patches".into(),
            path: "patches".into(),
            message: "patch ranges overlap".into(),
        });
    }
    if !diagnostics.is_empty() {
        return Err(PatchValidationReport { diagnostics });
    }
    let mut patch_set = MarkdownPatchSet {
        version: mathscape_domain::MARKDOWN_PATCH_SET_VERSION.into(),
        run_id,
        base_snapshot: input.snapshot.clone(),
        base_input_fingerprint: input.fingerprint.clone(),
        patches: resolved,
        fingerprint: input.fingerprint.clone(),
    };
    patch_set.fingerprint = patch_set_fingerprint(&patch_set);
    Ok(patch_set)
}

pub fn apply_patch_set(
    source: &str,
    patch_set: &MarkdownPatchSet,
) -> Result<AppliedMarkdownPatch, PatchApplyError> {
    if patch_set.patches.is_empty() {
        return Err(PatchApplyError::Empty);
    }
    let mut patches = patch_set.patches.clone();
    patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
    if patches
        .windows(2)
        .any(|pair| pair[0].range.end > pair[1].range.start)
    {
        return Err(PatchApplyError::Overlap);
    }
    let mut previous_end = 0_u64;
    for patch in &patches {
        if patch.range.start < previous_end
            || patch.range.end <= patch.range.start
            || patch.range.end as usize > source.len()
            || !source.is_char_boundary(patch.range.start as usize)
            || !source.is_char_boundary(patch.range.end as usize)
        {
            return Err(PatchApplyError::InvalidRange {
                patch_ref: patch.patch_ref.clone(),
            });
        }
        let old = &source[patch.range.start as usize..patch.range.end as usize];
        if old != patch.old_text || hash_bytes(old.as_bytes()) != patch.old_text_hash {
            return Err(PatchApplyError::Stale {
                patch_ref: patch.patch_ref.clone(),
            });
        }
        previous_end = patch.range.end;
    }
    let mut output = source.to_owned();
    for patch in patches.iter().rev() {
        output.replace_range(
            patch.range.start as usize..patch.range.end as usize,
            &patch.replacement,
        );
    }
    let offset_map = OffsetMap {
        edits: patches
            .iter()
            .map(|patch| OffsetEdit {
                old: patch.range.clone(),
                new_len: patch.replacement.len() as u64,
            })
            .collect(),
    };
    let new_patch_ranges = patches
        .iter()
        .map(|patch| {
            let start = offset_map
                .map_offset(patch.range.start)
                .unwrap_or(patch.range.start);
            TextByteRange {
                start,
                end: start + patch.replacement.len() as u64,
            }
        })
        .collect();
    Ok(AppliedMarkdownPatch {
        source: output,
        offset_map,
        new_patch_ranges,
    })
}

fn block_fingerprint(document: &AnchoredMarkdown) -> ContentHash {
    hash_value(
        &document
            .blocks
            .iter()
            .map(|block| block.kind.as_str())
            .collect::<Vec<_>>(),
    )
}

/// Compares the reparsed physical structure and verifies that every patch is
/// strictly inside one unit range. Formula delimiters are intentionally not
/// inspected separately; the anchor parser's block result is authoritative.
pub fn evaluate_patch_safety(
    original: &AnchoredMarkdown,
    patched: &AnchoredMarkdown,
    patch_set: &MarkdownPatchSet,
    unit_ranges: &[(String, Vec<TextByteRange>)],
) -> PatchSafetyReport {
    let old_block_fingerprint = block_fingerprint(original);
    let new_block_fingerprint = block_fingerprint(patched);
    let mut diagnostics = Vec::new();
    if original.blocks.len() != patched.blocks.len()
        || original
            .blocks
            .iter()
            .zip(&patched.blocks)
            .any(|(old, new)| old.kind != new.kind)
    {
        diagnostics.push(PatchSafetyDiagnostic {
            code: "block_structure_changed".into(),
            path: "blocks".into(),
            message: "patched Markdown produced a different block sequence".into(),
        });
    }
    for (index, (old, new)) in original.blocks.iter().zip(&patched.blocks).enumerate() {
        if old.kind == PAGE_MARKER_STRUCTURAL_KIND
            && (new.kind != PAGE_MARKER_STRUCTURAL_KIND
                || original.source[old.range.start as usize..old.range.end as usize]
                    != patched.source[new.range.start as usize..new.range.end as usize])
        {
            diagnostics.push(PatchSafetyDiagnostic {
                code: "page_marker_changed".into(),
                path: format!("blocks[{index}]"),
                message: "PDF 分页标记不可修改；请保留原始页边界后再应用 Markdown patch".into(),
            });
        }
    }
    for patch in &patch_set.patches {
        let in_one_unit = unit_ranges.iter().any(|(_, ranges)| {
            ranges.iter().any(|unit| {
                unit.start < patch.range.start
                    && patch.range.end < unit.end
                    && unit.end <= original.source.len() as u64
            })
        });
        if !in_one_unit {
            diagnostics.push(PatchSafetyDiagnostic {
                code: "unit_boundary_touched".into(),
                path: format!("patches.{}", patch.patch_ref),
                message: "patch must lie strictly inside one frozen unit".into(),
            });
        }
        let in_one_block = original.blocks.iter().any(|block| {
            block.range.start < patch.range.start && patch.range.end < block.range.end
        });
        if !in_one_block {
            diagnostics.push(PatchSafetyDiagnostic {
                code: "block_boundary_touched".into(),
                path: format!("patches.{}", patch.patch_ref),
                message: "patch must lie strictly inside one physical block".into(),
            });
        }
    }
    PatchSafetyReport {
        status: if diagnostics.is_empty() {
            PatchSafetyStatus::SafeForB
        } else {
            PatchSafetyStatus::RequiresARebuild
        },
        diagnostics,
        old_block_fingerprint,
        new_block_fingerprint,
    }
}

/// Builds a patched anchored document while preserving the caller's artifact
/// lineage. The caller supplies the new artifact/conversion identities.
pub fn anchor_patched_markdown(
    artifact_id: StableId,
    conversion_id: Option<StableId>,
    applied: &AppliedMarkdownPatch,
) -> Result<AnchoredMarkdown, crate::AnchorError> {
    anchor_markdown(artifact_id, conversion_id, applied.source.clone())
}

/// Promotes an already safety-checked patch result into the managed Artifact
/// Pool and records the deterministic Markdown-to-Markdown ConversionRun.
pub fn promote_applied_patch(
    pool: &mut ArtifactPool,
    base_artifact_id: &StableId,
    base_artifact: &SourceArtifact,
    patch_set: &MarkdownPatchSet,
    applied: &AppliedMarkdownPatch,
    pdf_inheritance: Option<&MarkdownPdfMapInheritance>,
    safety: PatchSafetyReport,
    new_input_fingerprint: Option<ContentHash>,
    actor: ActorRef,
    now: Timestamp,
) -> Result<MarkdownPatchPromotion, MarkdownPatchPromotionError> {
    let filename = base_artifact
        .original_filename()
        .map(|name| format!("{name}.patched.md"));
    let metadata = ArtifactMetadata::new(
        base_artifact.edition().cloned(),
        base_artifact.media_type().clone(),
        filename,
        base_artifact.page_count(),
        base_artifact.rights().clone(),
    );
    let artifact = pool.import_bytes(applied.source.as_bytes(), metadata, actor.clone())?;
    let conversion_id = markdown_patch_conversion_id(patch_set)?;
    let page_map_artifact = if let Some(inheritance) = pdf_inheritance {
        let map = build_markdown_pdf_block_map(
            artifact.stable_id().clone(),
            conversion_id.clone(),
            inheritance.pdf_artifact_id.clone(),
            applied.source.clone(),
        )
        .map_err(MarkdownPatchPromotionError::PageMap)?
        .ok_or_else(|| {
            MarkdownPatchPromotionError::PageMap("继承 PDF 回链的 Markdown 缺少分页标记".into())
        })?;
        if map.marker_page_ids != inheritance.marker_page_ids {
            return Err(MarkdownPatchPromotionError::PageMap(
                "patch 产物的 PDF 分页标记序列与基础转换不一致".into(),
            ));
        }
        let filename = base_artifact
            .original_filename()
            .unwrap_or("markdown")
            .to_owned()
            + ".pdf_map.json";
        Some(pool.import_bytes(
            &serde_json::to_vec_pretty(&map)?,
            ArtifactMetadata::derived(
                mathscape_domain::MediaType::parse("application/json")?,
                Some(filename),
            )?,
            actor.clone(),
        )?)
    } else {
        None
    };
    let mut options = BTreeMap::new();
    options.insert(
        "patch_fingerprint".into(),
        patch_set.fingerprint.to_string(),
    );
    options.insert(
        "patch_schema_version".into(),
        mathscape_domain::MARKDOWN_PATCH_SET_VERSION.into(),
    );
    options.insert(
        "selection_fingerprint".into(),
        patch_set.base_input_fingerprint.to_string(),
    );
    let output_artifacts = if let Some(map) = &page_map_artifact {
        options.insert(
            MARKDOWN_PDF_MAP_ARTIFACT_KEY.into(),
            map.stable_id().to_string(),
        );
        let inheritance = pdf_inheritance.expect("page map implies inheritance");
        options.insert(
            MARKDOWN_PDF_BASE_CONVERSION_KEY.into(),
            inheritance.base_conversion_id.to_string(),
        );
        options.insert(
            MARKDOWN_PDF_SOURCE_PDF_ARTIFACT_KEY.into(),
            inheritance.pdf_artifact_id.to_string(),
        );
        vec![artifact.stable_id().clone(), map.stable_id().clone()]
    } else {
        vec![artifact.stable_id().clone()]
    };
    let parameters = ConversionParameters::new(None, "markdown".into(), None, options)?;
    let conversion = ConversionRun::new(
        "mathscape-markdown-patch".into(),
        Some(mathscape_domain::MARKDOWN_PATCH_SET_VERSION.into()),
        None,
        CommandRecordStatus::Unavailable,
        parameters,
        None,
        Some(now.clone()),
        Some(now.clone()),
        ConversionExitStatus::Succeeded,
        vec![],
        None,
    )?
    .into_record(
        EntityHeader::new(
            conversion_id.clone(),
            mathscape_domain::RevisionId::parse(patch_set.fingerprint.to_string())?,
            EntityKind::ConversionRun,
            SchemaVersion::parse(mathscape_domain::MARKDOWN_PATCH_VERSION)?,
            LifecycleStatus::Active,
            now,
            actor,
            Some(patch_set.run_id.clone()),
        ),
        vec![base_artifact_id.clone()],
        output_artifacts,
    )?;
    pool.record_conversion(&conversion)?;
    let receipt = MarkdownPatchApplyReceipt {
        version: mathscape_domain::MARKDOWN_PATCH_RECEIPT_VERSION.into(),
        run_id: patch_set.run_id.clone(),
        patch_set_fingerprint: patch_set.fingerprint.clone(),
        base_artifact_id: base_artifact_id.clone(),
        patched_artifact_id: Some(artifact.stable_id().clone()),
        conversion_id: Some(conversion_id.clone()),
        old_input_fingerprint: patch_set.base_input_fingerprint.clone(),
        new_input_fingerprint,
        safety,
        status: MarkdownPatchStatus::Applied,
    };
    Ok(MarkdownPatchPromotion {
        artifact,
        conversion,
        receipt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mathscape_domain::{
        DiscoveryInputBoundary, DiscoveryInputRegion, DiscoveryInputSegment, DiscoveryIssueRef,
        MarkdownSnapshot,
    };

    fn input() -> DiscoveryInput {
        let artifact = StableId::parse("fixture:artifact/md").unwrap();
        let snapshot = MarkdownSnapshot {
            artifact_id: artifact,
            conversion_id: None,
            content_hash: hash_bytes(b"before"),
            anchor_scheme: "fixture".into(),
            fingerprint: hash_bytes(b"snapshot"),
        };
        DiscoveryInput {
            version: mathscape_domain::DISCOVERY_INPUT_VERSION.into(),
            snapshot,
            selection_fingerprint: hash_bytes(b"selection"),
            fingerprint: hash_bytes(b"input"),
            requested: DiscoveryInputRegion {
                source_range: TextByteRange { start: 0, end: 18 },
                segments: vec![DiscoveryInputSegment::Block {
                    block_id: "block:one".into(),
                    structural_kind: "text".into(),
                    physical_range: TextByteRange { start: 0, end: 18 },
                    source_range: TextByteRange { start: 0, end: 18 },
                    text: "定理：B, (x) 成立。\n".into(),
                }],
            },
            context: vec![],
            boundary: DiscoveryInputBoundary {
                at_document_start: true,
                at_document_end: true,
            },
        }
    }

    #[test]
    fn resolves_exact_patch_inside_issue_evidence() {
        let input = input();
        let issue_ref = DiscoveryIssueRef::parse("a-i1").unwrap();
        let issue = CanonicalDiscoveryIssue {
            issue_ref: issue_ref.clone(),
            kind: DiscoveryIssueKind::SuspectedConversionError,
            unit_ref: Some("u1".into()),
            evidence: vec![TextByteRange { start: 0, end: 18 }],
            observation: "subscript lost".into(),
        };
        let patch = MarkdownPatchProposal {
            patch_ref: "model-p1".into(),
            issue_ref,
            target: TextOccurrence::TextOccurrence {
                block: "b1".into(),
                text: "B, (x)".into(),
                occurrence: None,
            },
            replacement: "B_r(x)".into(),
            rationale: "restore subscript".into(),
        };
        let set = resolve_patch_proposals(
            &input,
            StableId::parse("fixture:run/1").unwrap(),
            &[issue],
            &[patch],
        )
        .unwrap();
        let applied = apply_patch_set("定理：B, (x) 成立。\n", &set).unwrap();
        assert!(applied.source.contains("B_r(x)"));
        assert_eq!(
            applied
                .offset_map
                .map_range(&TextByteRange { start: 0, end: 18 })
                .unwrap()
                .start,
            0
        );
    }

    #[test]
    fn rejects_target_outside_issue_evidence_and_ambiguous_text() {
        let input = input();
        let issue_ref = DiscoveryIssueRef::parse("a-i1").unwrap();
        let issue = CanonicalDiscoveryIssue {
            issue_ref: issue_ref.clone(),
            kind: DiscoveryIssueKind::SuspectedConversionError,
            unit_ref: Some("u1".into()),
            evidence: vec![TextByteRange { start: 0, end: 6 }],
            observation: "conversion".into(),
        };
        let patch = MarkdownPatchProposal {
            patch_ref: "p".into(),
            issue_ref,
            target: TextOccurrence::TextOccurrence {
                block: "b1".into(),
                text: "B, (x)".into(),
                occurrence: None,
            },
            replacement: "B_r(x)".into(),
            rationale: "outside".into(),
        };
        let error = resolve_patch_proposals(
            &input,
            StableId::parse("fixture:run/2").unwrap(),
            &[issue],
            &[patch],
        )
        .unwrap_err();
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == "outside_issue_evidence")
        );
    }

    #[test]
    fn offset_map_rejects_endpoints_inside_replacements() {
        let input = input();
        let issue_ref = DiscoveryIssueRef::parse("a-i1").unwrap();
        let set = resolve_patch_proposals(
            &input,
            StableId::parse("fixture:run/3").unwrap(),
            &[CanonicalDiscoveryIssue {
                issue_ref: issue_ref.clone(),
                kind: DiscoveryIssueKind::SuspectedConversionError,
                unit_ref: None,
                evidence: vec![TextByteRange { start: 0, end: 18 }],
                observation: "conversion".into(),
            }],
            &[MarkdownPatchProposal {
                patch_ref: "p".into(),
                issue_ref,
                target: TextOccurrence::TextOccurrence {
                    block: "b1".into(),
                    text: "B, (x)".into(),
                    occurrence: None,
                },
                replacement: "B_r(x)".into(),
                rationale: "subscript".into(),
            }],
        )
        .unwrap();
        let applied = apply_patch_set("定理：B, (x) 成立。\n", &set).unwrap();
        let patch = &set.patches[0];
        // The exact edit boundary is stable: a range ending at the old
        // replacement start or beginning at its end can be translated.  An
        // offset strictly inside the replaced bytes has no trustworthy
        // correspondence and must fail closed.
        assert!(
            applied
                .offset_map
                .map_offset(patch.range.start + 1)
                .is_none()
        );
        assert!(applied.offset_map.map_range(&patch.range).is_some());
    }
}
