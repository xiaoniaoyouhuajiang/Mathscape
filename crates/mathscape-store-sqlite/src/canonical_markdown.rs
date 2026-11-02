//! Canonical markdown registry and the readonly coverage projection.
//!
//! The projection reads `ConversionRun` page ranges already stored on the
//! catalog. It does not open Markdown bytes, parse a page map, or run Marker.
//! `sync_canonical_markdown_from_catalog` is an explicit rebuild of the view.

use std::collections::BTreeMap;

use mathscape_domain::{
    CanonicalMarkdownKey, CanonicalMarkdownView, CanonicalSegment, ConversionExitStatus,
    ConversionRun, InclusivePageRange, SourcePayload, StableId, Timestamp,
    UNSPECIFIED_PARSER_PROFILE, normalize_to_one_based, parser_policy_from_tool_version,
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{ArtifactCatalogSnapshot, SqliteStoreError};

const PDF_MEDIA_TYPE: &str = "application/pdf";
const MARKDOWN_MEDIA_TYPE: &str = "text/markdown";
const MARKER_TOOL: &str = "marker_single";
const MARKER_PROFILE_OPTION: &str = "marker_profile";

struct Candidate {
    conversion_id: StableId,
    markdown_artifact_id: StableId,
    ordinal: u32,
    finished_at: Option<String>,
    pages: Option<InclusivePageRange>,
    revision: Option<String>,
}

/// Groups successful Marker markdown conversions into logical canonical views.
///
/// Coverage pages come from `ConversionRun.page_range`, normalized to 1-based
/// closed intervals. A parsed markdown-pdf map is not opened here; callers that
/// already have `marker_page_ids` should use `pages_from_marker_page_ids`.
pub fn project_canonical_markdown(
    catalog: &ArtifactCatalogSnapshot,
) -> Result<Vec<CanonicalMarkdownView>, SqliteStoreError> {
    let mut media = BTreeMap::new();
    let mut pages = BTreeMap::new();
    let mut revisions = BTreeMap::new();
    for record in catalog.artifacts() {
        let id = record.entity().header().stable_id().clone();
        let artifact = record.entity().payload();
        media.insert(id.clone(), artifact.media_type().as_str().to_owned());
        pages.insert(id.clone(), artifact.page_count());
        revisions.insert(id, record.entity().header().revision_id().to_string());
    }

    let mut grouped: BTreeMap<String, (CanonicalMarkdownKey, Option<u32>, Vec<Candidate>)> =
        BTreeMap::new();
    for relation in catalog.conversions() {
        let SourcePayload::ConversionRun(run) = relation.payload() else {
            continue;
        };
        if !contributes(run.exit_status()) || run.parameters().output_format() != "markdown" {
            continue;
        }
        let Some(profile) = profile_of(run) else {
            continue;
        };
        let policy = parser_policy_from_tool_version(run.tool_version());

        let pdfs: Vec<_> = relation
            .endpoints()
            .iter()
            .filter(|endpoint| endpoint.role().as_str() == "input")
            .map(|endpoint| endpoint.entity().clone())
            .filter(|id| media.get(id).map(String::as_str) == Some(PDF_MEDIA_TYPE))
            .collect();
        if pdfs.len() != 1 {
            continue;
        }
        let pdf = &pdfs[0];
        let markdowns: Vec<_> = relation
            .endpoints()
            .iter()
            .filter(|endpoint| endpoint.role().as_str() == "output")
            .filter(|endpoint| {
                media.get(endpoint.entity()).map(String::as_str) == Some(MARKDOWN_MEDIA_TYPE)
            })
            .collect();
        if markdowns.is_empty() {
            continue;
        }

        let Ok(key) = CanonicalMarkdownKey::new(pdf.clone(), profile, policy) else {
            continue;
        };
        let page_range = run
            .parameters()
            .page_range()
            .map(normalize_to_one_based)
            .transpose()?;
        let finished_at = run
            .finished_at()
            .map(|timestamp| timestamp.as_str().to_owned());
        let entry = grouped
            .entry(key.canonical_id().to_string())
            .or_insert_with(|| (key, pages.get(pdf).copied().flatten(), Vec::new()));
        for endpoint in markdowns {
            entry.2.push(Candidate {
                conversion_id: relation.header().stable_id().clone(),
                markdown_artifact_id: endpoint.entity().clone(),
                ordinal: endpoint.ordinal().unwrap_or(u32::MAX),
                finished_at: finished_at.clone(),
                pages: page_range,
                revision: revisions.get(endpoint.entity()).cloned(),
            });
        }
    }

    let mut views = Vec::with_capacity(grouped.len());
    for (_, (key, total_pages, mut candidates)) in grouped {
        candidates.sort_by(|left, right| {
            head_rank(left)
                .cmp(&head_rank(right))
                .then_with(|| left.markdown_artifact_id.cmp(&right.markdown_artifact_id))
        });
        let head = candidates.last();
        let mut view = CanonicalMarkdownView::from_key(&key, total_pages);
        if let Some(head) = head {
            view.head_artifact_id = Some(head.markdown_artifact_id.clone());
            view.head_revision = head.revision.clone();
        }
        view.segments = candidates
            .into_iter()
            .map(|candidate| CanonicalSegment {
                conversion_id: candidate.conversion_id,
                markdown_artifact_id: candidate.markdown_artifact_id,
                pages: candidate.pages,
                finished_at: candidate.finished_at,
            })
            .collect();
        view.segments.sort_by(|left, right| {
            left.conversion_id
                .cmp(&right.conversion_id)
                .then_with(|| left.markdown_artifact_id.cmp(&right.markdown_artifact_id))
        });
        views.push(view);
    }
    views.sort_by(|left, right| left.canonical_id.cmp(&right.canonical_id));
    Ok(views)
}

fn contributes(status: ConversionExitStatus) -> bool {
    matches!(
        status,
        ConversionExitStatus::Succeeded | ConversionExitStatus::Partial
    )
}

fn profile_of(run: &ConversionRun) -> Option<String> {
    match run
        .parameters()
        .additional_options()
        .get(MARKER_PROFILE_OPTION)
    {
        Some(profile) => Some(profile.clone()),
        None if run.tool_name() == MARKER_TOOL => Some(UNSPECIFIED_PARSER_PROFILE.to_owned()),
        None => None,
    }
}

/// Later finish time wins; then the smaller endpoint ordinal (primary markdown
/// is imported first); then the conversion id. `unix:` timestamps are zero-padded
/// so they sort numerically. RFC3339 values sort as text inside their own prefix.
fn head_rank(candidate: &Candidate) -> (String, std::cmp::Reverse<u32>, String) {
    (
        timestamp_sort_key(candidate.finished_at.as_deref()),
        std::cmp::Reverse(candidate.ordinal),
        candidate.conversion_id.to_string(),
    )
}

fn timestamp_sort_key(value: Option<&str>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if let Some(rest) = value.strip_prefix("unix:") {
        let (seconds, nanos) = rest.split_once('.').unwrap_or((rest, "0"));
        let seconds: u64 = seconds.parse().unwrap_or(0);
        let nanos: u32 = nanos.parse().unwrap_or(0);
        return format!("unix:{seconds:020}.{nanos:09}");
    }
    format!("text:{value}")
}

pub(super) fn list_canonical_markdown(
    connection: &Connection,
) -> Result<Vec<CanonicalMarkdownView>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT canonical_id, source_pdf_artifact_id, parser_profile, parser_policy_version,
                head_artifact_id, head_revision, total_pages
         FROM canonical_markdown
         ORDER BY canonical_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<i64>>(6)?,
        ))
    })?;
    let mut views = Vec::new();
    for row in rows {
        let (canonical_id, pdf, profile, policy, head, revision, total_pages) = row?;
        let total_pages = match total_pages {
            Some(value) => {
                Some(
                    u32::try_from(value).map_err(|_| SqliteStoreError::IntegerRange {
                        field: "canonical_markdown.total_pages",
                        value: value as u64,
                    })?,
                )
            }
            None => None,
        };
        views.push(CanonicalMarkdownView {
            canonical_id: StableId::parse(canonical_id)?,
            source_pdf_artifact_id: StableId::parse(pdf)?,
            parser_profile: profile,
            parser_policy_version: policy,
            head_artifact_id: head.map(StableId::parse).transpose()?,
            head_revision: revision,
            total_pages,
            segments: Vec::new(),
        });
    }
    drop(statement);

    let mut segments = connection.prepare(
        "SELECT canonical_id, conversion_id, markdown_artifact_id, page_from, page_to
         FROM canonical_markdown_coverage
         ORDER BY canonical_id, conversion_id, markdown_artifact_id",
    )?;
    let segment_rows = segments.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    for row in segment_rows {
        let (canonical_id, conversion_id, markdown_id, page_from, page_to) = row?;
        let Some(view) = views
            .iter_mut()
            .find(|view| view.canonical_id.as_str() == canonical_id)
        else {
            continue;
        };
        let pages = InclusivePageRange::new(sqlite_page(page_from)?, sqlite_page(page_to)?)?;
        view.segments.push(CanonicalSegment {
            conversion_id: StableId::parse(conversion_id)?,
            markdown_artifact_id: StableId::parse(markdown_id)?,
            pages: Some(pages),
            finished_at: None,
        });
    }
    Ok(views)
}

pub(super) fn upsert_canonical_markdown(
    connection: &Connection,
    key: &CanonicalMarkdownKey,
    head_artifact_id: Option<&StableId>,
    head_revision: Option<&str>,
    total_pages: Option<u32>,
    recorded_at: &Timestamp,
) -> Result<StableId, SqliteStoreError> {
    let canonical_id = key.canonical_id();
    let existing: Option<String> = connection
        .query_row(
            "SELECT canonical_id FROM canonical_markdown
             WHERE source_pdf_artifact_id = ?1 AND parser_profile = ?2 AND parser_policy_version = ?3",
            params![
                key.source_pdf_artifact_id().as_str(),
                key.parser_profile(),
                key.parser_policy_version()
            ],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(existing) = existing {
        if existing != canonical_id.as_str() {
            return Err(SqliteStoreError::CanonicalMarkdownConflict {
                source_pdf_artifact_id: key.source_pdf_artifact_id().to_string(),
                parser_profile: key.parser_profile().to_owned(),
                parser_policy_version: key.parser_policy_version().to_owned(),
                existing_canonical_id: existing,
            });
        }
        connection.execute(
            "UPDATE canonical_markdown
             SET head_artifact_id = ?2, head_revision = ?3, total_pages = ?4, updated_at = ?5
             WHERE canonical_id = ?1",
            params![
                canonical_id.as_str(),
                head_artifact_id.map(StableId::as_str),
                head_revision,
                total_pages.map(i64::from),
                recorded_at.as_str(),
            ],
        )?;
        return Ok(canonical_id);
    }
    connection.execute(
        "INSERT INTO canonical_markdown (
            canonical_id, source_pdf_artifact_id, parser_profile, parser_policy_version,
            head_artifact_id, head_revision, total_pages, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            canonical_id.as_str(),
            key.source_pdf_artifact_id().as_str(),
            key.parser_profile(),
            key.parser_policy_version(),
            head_artifact_id.map(StableId::as_str),
            head_revision,
            total_pages.map(i64::from),
            recorded_at.as_str(),
        ],
    )?;
    Ok(canonical_id)
}

pub(super) fn record_canonical_coverage_segment(
    connection: &Connection,
    canonical_id: &StableId,
    segment: &CanonicalSegment,
    recorded_at: &Timestamp,
) -> Result<(), SqliteStoreError> {
    let Some(pages) = segment.pages else {
        return Err(SqliteStoreError::CanonicalMarkdownInvariant(
            "a coverage segment requires a 1-based page range",
        ));
    };
    if pages.from > pages.to || pages.from == 0 {
        return Err(SqliteStoreError::CanonicalMarkdownInvariant(
            "coverage pages must be a non-empty 1-based range",
        ));
    }
    let written = connection.execute(
        "INSERT INTO canonical_markdown_coverage (
            canonical_id, conversion_id, markdown_artifact_id, page_from, page_to,
            page_index_base, recorded_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'one', ?6)
         ON CONFLICT (canonical_id, conversion_id, markdown_artifact_id) DO UPDATE SET
            page_from = excluded.page_from,
            page_to = excluded.page_to,
            page_index_base = excluded.page_index_base,
            recorded_at = excluded.recorded_at",
        params![
            canonical_id.as_str(),
            segment.conversion_id.as_str(),
            segment.markdown_artifact_id.as_str(),
            i64::from(pages.from),
            i64::from(pages.to),
            recorded_at.as_str(),
        ],
    )?;
    if written == 0 {
        return Err(SqliteStoreError::CanonicalMarkdownInvariant(
            "coverage segment was not recorded",
        ));
    }
    Ok(())
}

pub(super) fn replace_canonical_markdown(
    connection: &mut Connection,
    views: &[CanonicalMarkdownView],
    recorded_at: &Timestamp,
) -> Result<Vec<CanonicalMarkdownView>, SqliteStoreError> {
    let transaction = connection.transaction()?;
    transaction.execute("DELETE FROM canonical_markdown_coverage", [])?;
    transaction.execute("DELETE FROM canonical_markdown", [])?;
    insert_views(&transaction, views.iter(), recorded_at)?;
    transaction.commit()?;
    list_canonical_markdown(connection)
}

/// Rebuilds only the registry rows of one source PDF. Rows of every other PDF
/// are left untouched, so an ensure on one book never rewrites another's head.
pub(super) fn replace_canonical_markdown_for_pdf(
    connection: &mut Connection,
    source_pdf_artifact_id: &StableId,
    views: &[CanonicalMarkdownView],
    recorded_at: &Timestamp,
) -> Result<Vec<CanonicalMarkdownView>, SqliteStoreError> {
    let transaction = connection.transaction()?;
    transaction.execute(
        "DELETE FROM canonical_markdown_coverage WHERE canonical_id IN (
            SELECT canonical_id FROM canonical_markdown WHERE source_pdf_artifact_id = ?1
         )",
        [source_pdf_artifact_id.as_str()],
    )?;
    transaction.execute(
        "DELETE FROM canonical_markdown WHERE source_pdf_artifact_id = ?1",
        [source_pdf_artifact_id.as_str()],
    )?;
    insert_views(
        &transaction,
        views
            .iter()
            .filter(|view| &view.source_pdf_artifact_id == source_pdf_artifact_id),
        recorded_at,
    )?;
    transaction.commit()?;
    Ok(list_canonical_markdown(connection)?
        .into_iter()
        .filter(|view| &view.source_pdf_artifact_id == source_pdf_artifact_id)
        .collect())
}

fn insert_views<'a>(
    transaction: &Connection,
    views: impl Iterator<Item = &'a CanonicalMarkdownView>,
    recorded_at: &Timestamp,
) -> Result<(), SqliteStoreError> {
    for view in views {
        let key = CanonicalMarkdownKey::new(
            view.source_pdf_artifact_id.clone(),
            view.parser_profile.clone(),
            view.parser_policy_version.clone(),
        )?;
        upsert_canonical_markdown(
            transaction,
            &key,
            view.head_artifact_id.as_ref(),
            view.head_revision.as_deref(),
            view.total_pages,
            recorded_at,
        )?;
        for segment in &view.segments {
            if segment.pages.is_some() {
                record_canonical_coverage_segment(
                    transaction,
                    &view.canonical_id,
                    segment,
                    recorded_at,
                )?;
            }
        }
    }
    Ok(())
}

fn sqlite_page(value: i64) -> Result<u32, SqliteStoreError> {
    u32::try_from(value).map_err(|_| SqliteStoreError::IntegerRange {
        field: "canonical_markdown_coverage.page",
        value: value.unsigned_abs(),
    })
}
