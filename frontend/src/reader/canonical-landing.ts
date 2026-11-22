import type { CanonicalReaderProjection, CanonicalReaderSegment, PageRange } from "./canonical";

/**
 * Where a physical-snapshot position lands in the 「规范解析正文」 unified view.
 *
 * The unified view is the same immutable snapshots stitched in page order, so
 * a block keeps its id and byte range there. It is absent only when its page
 * is shown from an earlier snapshot (trimmed overlap, or a snapshot skipped as
 * fully covered); the landing then falls back to that PDF page, and to the
 * physical snapshot when the page is not readable in the view either.
 */
export interface LandingFocus {
  start: number;
  end: number;
  blockIds?: string[];
  /** Physical snapshot the coordinates belong to. */
  artifactId?: string;
  /** Zero-based PDF page of the position, when the caller knows it. */
  pdfPageIndex?: number;
}

export type CanonicalLanding =
  | { kind: "none" }
  | { kind: "block" }
  | { kind: "segment"; artifactId: string }
  | { kind: "page"; pageIndex: number }
  /** The block is not shown and its page is unknown: look the page up, then plan again. */
  | { kind: "needs_page" }
  /** Not readable at this place in the unified view: keep the physical snapshot. */
  | { kind: "snapshot" };

const PAGE_MARKER = /^\s*\{(\d+)\}-{16,}\s*$/;

/** Zero-based PDF page of a Marker `{N}----` page-marker block, if it is one. */
export function pageMarkerIndex(text: string): number | undefined {
  const page = PAGE_MARKER.exec(text)?.[1];
  return page === undefined ? undefined : Number(page);
}

export function focusMatchesBlock(
  focus: LandingFocus,
  segmentArtifactId: string,
  block: { id: string; range: { start: number; end: number } },
): boolean {
  if (focus.artifactId && focus.artifactId !== segmentArtifactId) return false;
  if (focus.blockIds?.length) return focus.blockIds.includes(block.id);
  return focus.end > focus.start && block.range.start < focus.end && block.range.end > focus.start;
}

function covers(pages: PageRange | null, pageIndex: number): boolean {
  return pages !== null && pages.from <= pageIndex + 1 && pageIndex + 1 <= pages.to;
}

export function planCanonicalLanding(
  projection: Pick<CanonicalReaderProjection, "pieces">,
  focus: LandingFocus,
  pageIndex: number | undefined = focus.pdfPageIndex,
): CanonicalLanding {
  const segments = projection.pieces.filter((piece): piece is CanonicalReaderSegment => piece.kind === "segment");
  const own = focus.artifactId ? segments.filter(segment => segment.artifact_id === focus.artifactId) : segments;
  const wantsBlock = Boolean(focus.blockIds?.length) || focus.end > focus.start;
  if (!wantsBlock) {
    if (focus.artifactId) return own.length ? { kind: "segment", artifactId: focus.artifactId } : { kind: "none" };
    // A place without a block (a gap card left for the PDF page): that page, parsed or not.
    return pageIndex === undefined ? { kind: "none" } : { kind: "page", pageIndex };
  }
  const shown = own.some(segment => Boolean(segment.content_hash) && (segment.blocks ?? []).some(
    block => block.kind !== "page_marker" && focusMatchesBlock(focus, segment.artifact_id, block),
  ));
  if (shown) return { kind: "block" };
  // Without the owning snapshot the coordinates cannot be told apart from another segment's.
  if (!focus.artifactId) return { kind: "none" };
  if (pageIndex === undefined) return { kind: "needs_page" };
  return segments.some(segment => covers(segment.pages, pageIndex)) ? { kind: "page", pageIndex } : { kind: "snapshot" };
}

/** Zero-based PDF pages a snapshot starts and ends on, read from its page markers. */
export function snapshotPageSpan(
  blocks: readonly { kind: string }[],
  textOf: (index: number) => string,
): { first: number; last: number } | null {
  let first: number | undefined;
  let last: number | undefined;
  blocks.forEach((block, index) => {
    if (block.kind !== "page_marker") return;
    const page = pageMarkerIndex(textOf(index));
    if (page === undefined) return;
    first ??= page;
    last = page;
  });
  return first === undefined || last === undefined ? null : { first, last };
}

/** Zero-based PDF page a block sits on: the nearest page marker at or before it. */
export function pageIndexOfBlock(
  blocks: readonly { kind: string }[],
  textOf: (index: number) => string,
  index: number,
): number | undefined {
  for (let i = Math.min(index, blocks.length - 1); i >= 0; i--) {
    if (blocks[i].kind !== "page_marker") continue;
    const page = pageMarkerIndex(textOf(i));
    if (page !== undefined) return page;
  }
  return undefined;
}
