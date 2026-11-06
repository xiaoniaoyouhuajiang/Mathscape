//! Read-only unified reader projection of one canonical markdown lineage.
//!
//! Desktop 「阅读文件」 on a folded 「… · 规范解析正文」 node shows every
//! registered coverage segment in page order as one view. This module only
//! *plans* that view from the coverage ledger: which immutable snapshot
//! supplies which pages, where the uncovered gaps are, and which snapshots are
//! redundant. It never creates an Artifact and never rewrites snapshot bytes;
//! the caller reads each snapshot and may cut its leading overlap at a Marker
//! page marker (see [`markdown_offset_of_page`]).
//!
//! Overlap policy (deterministic): segments are ordered by first page
//! ascending, then by last page descending (the wider snapshot first), then
//! the head snapshot first, then artifact id. A sweep keeps a cursor at the
//! first page not yet shown:
//!
//! - a segment that ends before the cursor is fully contained in earlier
//!   segments and is listed in `skipped`, not shown;
//! - a segment that starts after the cursor leaves a [`ReaderPiece::Gap`];
//! - a segment that starts before the cursor contributes only the pages from
//!   the cursor on (`pages`), and records the overlap it repeats
//!   (`overlap`). Whether the bytes can actually be trimmed depends on page
//!   markers and is decided by the caller.
//!
//! "Earliest page start wins" keeps the plan stable when a later ensure adds a
//! snapshot: earlier text never shifts, new pages are appended where they
//! belong.

use serde::{Deserialize, Serialize};

use crate::{CanonicalMarkdownView, InclusivePageRange, StableId};

/// One snapshot chosen to supply pages of the unified view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReaderSegmentPlan {
    pub conversion_id: StableId,
    pub markdown_artifact_id: StableId,
    /// The snapshot's own coverage (ledger range). `None` only for the head
    /// fallback when no segment recorded pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_pages: Option<InclusivePageRange>,
    /// Pages this segment newly contributes to the view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<InclusivePageRange>,
    /// Leading pages already shown by an earlier segment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlap: Option<InclusivePageRange>,
    pub is_head: bool,
}

/// A piece of the unified view, in page order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReaderPiece {
    Segment(ReaderSegmentPlan),
    /// Pages no registered snapshot covers. Never filled with invented text.
    Gap {
        pages: InclusivePageRange,
    },
}

/// A snapshot left out because earlier segments already show all its pages.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SkippedReaderSegment {
    pub conversion_id: StableId,
    pub markdown_artifact_id: StableId,
    pub pages: InclusivePageRange,
    pub is_head: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CanonicalReaderPlan {
    pub pieces: Vec<ReaderPiece>,
    pub skipped: Vec<SkippedReaderSegment>,
}

/// Plans the unified reader view of `view` (see module docs for the policy).
///
/// Leading and inner gaps are always reported; a trailing gap only when
/// `total_pages` is known. When no segment has pages but a head exists, the
/// head is shown as one segment without page information.
pub fn plan_canonical_reader(view: &CanonicalMarkdownView) -> CanonicalReaderPlan {
    let head = view.head_artifact_id.as_ref();
    let is_head = |id: &StableId| head.is_some_and(|head| head == id);
    let mut candidates: Vec<_> = view
        .segments
        .iter()
        .filter_map(|segment| segment.pages.map(|pages| (segment, pages)))
        .collect();
    candidates.sort_by(|(left, lp), (right, rp)| {
        lp.from
            .cmp(&rp.from)
            .then_with(|| rp.to.cmp(&lp.to))
            .then_with(|| {
                is_head(&right.markdown_artifact_id).cmp(&is_head(&left.markdown_artifact_id))
            })
            .then_with(|| left.markdown_artifact_id.cmp(&right.markdown_artifact_id))
            .then_with(|| left.conversion_id.cmp(&right.conversion_id))
    });
    candidates
        .dedup_by(|(a, ap), (b, bp)| a.markdown_artifact_id == b.markdown_artifact_id && ap == bp);

    let mut plan = CanonicalReaderPlan::default();
    if candidates.is_empty() {
        if let Some(head) = head {
            let conversion = view
                .segments
                .iter()
                .find(|segment| &segment.markdown_artifact_id == head)
                .map(|segment| segment.conversion_id.clone());
            if let Some(conversion_id) = conversion {
                plan.pieces.push(ReaderPiece::Segment(ReaderSegmentPlan {
                    conversion_id,
                    markdown_artifact_id: head.clone(),
                    snapshot_pages: None,
                    pages: None,
                    overlap: None,
                    is_head: true,
                }));
            }
        }
        if let Some(total) = view.total_pages.filter(|total| *total > 0) {
            plan.pieces.push(ReaderPiece::Gap {
                pages: InclusivePageRange { from: 1, to: total },
            });
        }
        return plan;
    }

    let mut cursor = 1_u32;
    for (segment, pages) in candidates {
        let head = is_head(&segment.markdown_artifact_id);
        if pages.to < cursor {
            plan.skipped.push(SkippedReaderSegment {
                conversion_id: segment.conversion_id.clone(),
                markdown_artifact_id: segment.markdown_artifact_id.clone(),
                pages,
                is_head: head,
            });
            continue;
        }
        if pages.from > cursor {
            plan.pieces.push(ReaderPiece::Gap {
                pages: InclusivePageRange {
                    from: cursor,
                    to: pages.from - 1,
                },
            });
        }
        let start = pages.from.max(cursor);
        let overlap = (pages.from < cursor).then(|| InclusivePageRange {
            from: pages.from,
            to: cursor - 1,
        });
        plan.pieces.push(ReaderPiece::Segment(ReaderSegmentPlan {
            conversion_id: segment.conversion_id.clone(),
            markdown_artifact_id: segment.markdown_artifact_id.clone(),
            snapshot_pages: Some(pages),
            pages: Some(InclusivePageRange {
                from: start,
                to: pages.to,
            }),
            overlap,
            is_head: head,
        }));
        cursor = pages.to.saturating_add(1);
    }
    if let Some(total) = view.total_pages
        && cursor <= total
    {
        plan.pieces.push(ReaderPiece::Gap {
            pages: InclusivePageRange {
                from: cursor,
                to: total,
            },
        });
    }
    plan
}

/// One snapshot chosen to serve part of a careful page read
/// ([`plan_page_cover`]).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageCoverPiece {
    /// Index into the `views` slice given to [`plan_page_cover`].
    pub view_index: usize,
    pub conversion_id: StableId,
    pub markdown_artifact_id: StableId,
    /// The snapshot's own ledger range.
    pub snapshot_pages: InclusivePageRange,
    /// Requested pages this snapshot supplies (inside `snapshot_pages`).
    pub pages: InclusivePageRange,
    pub is_head: bool,
}

/// How registered snapshots cover one requested page range.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageCover {
    /// In page order, non-overlapping; together with `gaps` they tile the
    /// request exactly.
    pub pieces: Vec<PageCoverPiece>,
    /// Requested pages no snapshot of any given view covers.
    pub gaps: Vec<InclusivePageRange>,
}

impl PageCover {
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty() && !self.pieces.is_empty()
    }

    /// The one contiguous range spanning every gap (what a single Marker run
    /// must parse so that the request becomes coverable), if any.
    pub fn gap_hull(&self) -> Option<InclusivePageRange> {
        let first = self.gaps.first()?;
        let last = self.gaps.last()?;
        Some(InclusivePageRange {
            from: first.from,
            to: last.to,
        })
    }
}

/// Plans which immutable snapshots serve `request`, across every lineage in
/// `views` (best-ranked first). Greedy interval cover with a cursor at the
/// first requested page not yet supplied: among snapshots that contain the
/// cursor page, the one reaching furthest into the request wins; ties prefer the better-ranked
/// view, then its head, then the newest `finished_at`, then ids. Fewest
/// pieces for a contiguous request; deterministic for a given ledger.
/// Pages no snapshot contains become `gaps` (never invented text).
pub fn plan_page_cover(views: &[&CanonicalMarkdownView], request: InclusivePageRange) -> PageCover {
    let mut candidates = Vec::new();
    for (view_index, view) in views.iter().enumerate() {
        let head = view.head_artifact_id.as_ref();
        for segment in &view.segments {
            let Some(pages) = segment.pages else {
                continue;
            };
            if !pages.overlaps(request) {
                continue;
            }
            candidates.push((
                view_index,
                segment,
                pages,
                head == Some(&segment.markdown_artifact_id),
            ));
        }
    }
    let better =
        |left: &(usize, &crate::CanonicalSegment, InclusivePageRange, bool),
         right: &(usize, &crate::CanonicalSegment, InclusivePageRange, bool)| {
            let reach = |pages: InclusivePageRange| pages.to.min(request.to);
            reach(left.2)
                .cmp(&reach(right.2))
                .then_with(|| right.0.cmp(&left.0))
                .then_with(|| left.3.cmp(&right.3))
                .then_with(|| left.1.finished_at.cmp(&right.1.finished_at))
                .then_with(|| {
                    right
                        .1
                        .markdown_artifact_id
                        .cmp(&left.1.markdown_artifact_id)
                })
                .then_with(|| right.1.conversion_id.cmp(&left.1.conversion_id))
        };
    let mut cover = PageCover::default();
    let mut cursor = request.from;
    while cursor <= request.to {
        let best = candidates
            .iter()
            .filter(|(_, _, pages, _)| pages.from <= cursor && cursor <= pages.to)
            .max_by(|left, right| better(left, right));
        match best {
            Some((view_index, segment, pages, is_head)) => {
                let to = pages.to.min(request.to);
                cover.pieces.push(PageCoverPiece {
                    view_index: *view_index,
                    conversion_id: segment.conversion_id.clone(),
                    markdown_artifact_id: segment.markdown_artifact_id.clone(),
                    snapshot_pages: *pages,
                    pages: InclusivePageRange { from: cursor, to },
                    is_head: *is_head,
                });
                cursor = to.saturating_add(1);
            }
            None => {
                let next = candidates
                    .iter()
                    .map(|(_, _, pages, _)| pages.from)
                    .filter(|from| *from > cursor)
                    .min();
                let to = next.map_or(request.to, |from| (from - 1).min(request.to));
                cover.gaps.push(InclusivePageRange { from: cursor, to });
                cursor = to.saturating_add(1);
            }
        }
        if cursor == 0 {
            break;
        }
    }
    cover
}

/// Marker page id (0-based) of a `{N}------…` page-marker line, if `line` is one.
pub fn marker_page_id(line: &str) -> Option<u32> {
    let line = line.trim();
    let rest = line.strip_prefix('{')?;
    let (digits, dashes) = rest.split_once('}')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if dashes.len() < 16 || !dashes.bytes().all(|b| b == b'-') {
        return None;
    }
    digits.parse().ok()
}

/// Byte offset where 1-based `page` begins in a Marker markdown snapshot whose
/// ledger range is `snapshot_pages`, found by its `{page-1}------…` marker.
///
/// Returns `None` (do not trim) unless the snapshot is paginated, every marker
/// lies inside `snapshot_pages` (so the ids are absolute PDF pages, not
/// relative to the run's `--page_range`), and the page's marker is present.
pub fn markdown_offset_of_page(
    source: &str,
    snapshot_pages: InclusivePageRange,
    page: u32,
) -> Option<usize> {
    let wanted = page.checked_sub(1)?;
    let mut offset = 0_usize;
    let mut found = None;
    let mut any = false;
    for line in source.split_inclusive('\n') {
        if let Some(id) = marker_page_id(line) {
            any = true;
            let one_based = id.checked_add(1)?;
            if one_based < snapshot_pages.from || one_based > snapshot_pages.to {
                return None;
            }
            if id == wanted && found.is_none() {
                found = Some(offset);
            }
        }
        offset += line.len();
    }
    if any { found } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CanonicalSegment;

    fn id(value: &str) -> StableId {
        StableId::parse(value).unwrap()
    }

    fn range(from: u32, to: u32) -> InclusivePageRange {
        InclusivePageRange::new(from, to).unwrap()
    }

    fn segment(name: &str, pages: Option<(u32, u32)>) -> CanonicalSegment {
        CanonicalSegment {
            conversion_id: id(&format!("mathscape:conversion/{name}")),
            markdown_artifact_id: id(&format!("mathscape:artifact/{name}")),
            pages: pages.map(|(from, to)| range(from, to)),
            finished_at: None,
        }
    }

    fn view(
        total: Option<u32>,
        head: Option<&str>,
        segments: Vec<CanonicalSegment>,
    ) -> CanonicalMarkdownView {
        CanonicalMarkdownView {
            canonical_id: id("mathscape:canonical-markdown/v1/p/q/mathscape:artifact/pdf"),
            source_pdf_artifact_id: id("mathscape:artifact/pdf"),
            parser_profile: "p".into(),
            parser_policy_version: "q".into(),
            head_artifact_id: head.map(|name| id(&format!("mathscape:artifact/{name}"))),
            head_revision: None,
            total_pages: total,
            segments,
        }
    }

    fn describe(plan: &CanonicalReaderPlan) -> Vec<String> {
        plan.pieces
            .iter()
            .map(|piece| match piece {
                ReaderPiece::Gap { pages } => format!("gap {}-{}", pages.from, pages.to),
                ReaderPiece::Segment(segment) => {
                    let name = segment
                        .markdown_artifact_id
                        .as_str()
                        .trim_start_matches("mathscape:artifact/");
                    let pages = segment
                        .pages
                        .map_or("?".into(), |p| format!("{}-{}", p.from, p.to));
                    let overlap = segment
                        .overlap
                        .map_or(String::new(), |o| format!(" overlap {}-{}", o.from, o.to));
                    format!(
                        "{name} {pages}{overlap}{}",
                        if segment.is_head { " head" } else { "" }
                    )
                }
            })
            .collect()
    }

    #[test]
    fn stitches_segments_in_page_order_and_surfaces_gaps() {
        // Ledger order (by conversion id) differs from page order on purpose.
        let plan = plan_canonical_reader(&view(
            Some(60),
            Some("a"),
            vec![
                segment("a", Some((41, 50))),
                segment("b", Some((5, 12))),
                segment("c", Some((13, 20))),
            ],
        ));
        assert_eq!(
            describe(&plan),
            [
                "gap 1-4",
                "b 5-12",
                "c 13-20",
                "gap 21-40",
                "a 41-50 head",
                "gap 51-60"
            ]
        );
        assert!(plan.skipped.is_empty());
    }

    #[test]
    fn unknown_total_reports_no_trailing_gap() {
        let plan = plan_canonical_reader(&view(None, Some("a"), vec![segment("a", Some((3, 4)))]));
        assert_eq!(describe(&plan), ["gap 1-2", "a 3-4 head"]);
    }

    #[test]
    fn overlap_keeps_earliest_start_and_trims_the_later_segment() {
        let plan = plan_canonical_reader(&view(
            Some(30),
            Some("late"),
            vec![
                segment("late", Some((10, 30))),
                segment("early", Some((1, 15))),
            ],
        ));
        assert_eq!(
            describe(&plan),
            ["early 1-15", "late 16-30 overlap 10-15 head"]
        );
    }

    #[test]
    fn contained_snapshots_are_skipped_and_ties_prefer_wider_then_head() {
        let plan = plan_canonical_reader(&view(
            Some(20),
            Some("h"),
            vec![
                segment("inner", Some((3, 5))),
                segment("x", Some((1, 20))),
                segment("h", Some((1, 20))),
                segment("short", Some((1, 4))),
            ],
        ));
        assert_eq!(describe(&plan), ["h 1-20 head"]);
        let skipped: Vec<_> = plan
            .skipped
            .iter()
            .map(|s| {
                s.markdown_artifact_id
                    .as_str()
                    .trim_start_matches("mathscape:artifact/")
                    .to_owned()
            })
            .collect();
        assert_eq!(skipped, ["x", "short", "inner"]);
    }

    #[test]
    fn plan_is_independent_of_ledger_order() {
        let segments = vec![
            segment("a", Some((1, 10))),
            segment("b", Some((8, 20))),
            segment("c", Some((25, 30))),
            segment("d", Some((1, 10))),
        ];
        let forward = plan_canonical_reader(&view(Some(30), Some("b"), segments.clone()));
        let mut reversed = segments;
        reversed.reverse();
        assert_eq!(
            forward,
            plan_canonical_reader(&view(Some(30), Some("b"), reversed))
        );
        assert_eq!(
            describe(&forward),
            [
                "a 1-10",
                "b 11-20 overlap 8-10 head",
                "gap 21-24",
                "c 25-30"
            ]
        );
    }

    #[test]
    fn head_without_pages_is_shown_alone_and_the_book_is_a_gap() {
        let plan = plan_canonical_reader(&view(Some(9), Some("h"), vec![segment("h", None)]));
        assert_eq!(describe(&plan), ["h ? head", "gap 1-9"]);
        assert!(
            plan_canonical_reader(&view(None, None, vec![]))
                .pieces
                .is_empty()
        );
    }

    fn cover_desc(cover: &PageCover) -> Vec<String> {
        let mut out: Vec<String> = cover
            .pieces
            .iter()
            .map(|piece| {
                format!(
                    "{}:{} {}-{}",
                    piece.view_index,
                    piece
                        .markdown_artifact_id
                        .as_str()
                        .trim_start_matches("mathscape:artifact/"),
                    piece.pages.from,
                    piece.pages.to
                )
            })
            .collect();
        out.extend(
            cover
                .gaps
                .iter()
                .map(|g| format!("gap {}-{}", g.from, g.to)),
        );
        out
    }

    #[test]
    fn page_cover_stitches_adjacent_snapshots_without_gaps() {
        // The 2026-11-06 sandbox case: 1-4, 5-10, 11-14 already parsed.
        let lineage = view(
            Some(14),
            Some("c"),
            vec![
                segment("a", Some((1, 4))),
                segment("b", Some((5, 10))),
                segment("c", Some((11, 14))),
            ],
        );
        let cover = plan_page_cover(&[&lineage], range(1, 8));
        assert_eq!(cover_desc(&cover), ["0:a 1-4", "0:b 5-8"]);
        assert!(cover.is_complete());
        let cover = plan_page_cover(&[&lineage], range(9, 14));
        assert_eq!(cover_desc(&cover), ["0:b 9-10", "0:c 11-14"]);
        assert_eq!(cover.gap_hull(), None);
    }

    #[test]
    fn page_cover_prefers_the_furthest_reach_then_better_view() {
        let current = view(Some(30), Some("n"), vec![segment("n", Some((1, 6)))]);
        let old = view(
            Some(30),
            None,
            vec![segment("o1", Some((1, 9))), segment("o2", Some((3, 6)))],
        );
        // o1 reaches further than n: one piece instead of two.
        assert_eq!(
            cover_desc(&plan_page_cover(&[&current, &old], range(2, 8))),
            ["1:o1 2-8"]
        );
        // Same reach: the better-ranked (current) view wins.
        assert_eq!(
            cover_desc(&plan_page_cover(&[&current, &old], range(2, 5))),
            ["0:n 2-5"]
        );
    }

    #[test]
    fn page_cover_reports_gaps_and_their_hull() {
        let lineage = view(
            Some(30),
            None,
            vec![segment("a", Some((1, 3))), segment("b", Some((6, 7)))],
        );
        let cover = plan_page_cover(&[&lineage], range(2, 9));
        assert_eq!(
            cover_desc(&cover),
            ["0:a 2-3", "0:b 6-7", "gap 4-5", "gap 8-9"]
        );
        assert!(!cover.is_complete());
        assert_eq!(cover.gap_hull(), Some(range(4, 9)));
        let empty = plan_page_cover(&[], range(5, 6));
        assert_eq!(cover_desc(&empty), ["gap 5-6"]);
        assert_eq!(empty.gap_hull(), Some(range(5, 6)));
    }

    #[test]
    fn page_offsets_follow_absolute_marker_ids_only() {
        let source = "{9}------------------------------------------------\n\nnine\n\n{10}------------------------------------------------\n\nten\n";
        let at = markdown_offset_of_page(source, range(10, 11), 11).unwrap();
        assert!(source[at..].starts_with("{10}"));
        assert_eq!(markdown_offset_of_page(source, range(10, 11), 10), Some(0));
        // Missing page marker or relative ids (outside the ledger range): no trim.
        assert_eq!(markdown_offset_of_page(source, range(10, 11), 12), None);
        assert_eq!(markdown_offset_of_page(source, range(1, 2), 2), None);
        assert_eq!(
            markdown_offset_of_page("plain text\n", range(1, 2), 2),
            None
        );
        assert_eq!(
            marker_page_id("{3}---------------"),
            None,
            "needs 16 dashes"
        );
        assert_eq!(marker_page_id("  {3}----------------  \n"), Some(3));
    }
}
