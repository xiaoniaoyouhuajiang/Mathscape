import { invoke } from "@tauri-apps/api/core";

/**
 * Unified read-only view of a canonical markdown node (「… · 规范解析正文」).
 *
 * The backend (`get_canonical_reader_projection`) plans the registered coverage
 * ledger into page-ordered pieces: snapshot segments and uncovered gaps. Each
 * segment's text is read from its immutable physical snapshot; nothing is
 * written. See docs/canonical-markdown.md §「统一阅读投影」.
 */
export interface PageRange { from: number; to: number }

export interface CanonicalReaderSegment {
  kind: "segment";
  conversion_id: string;
  artifact_id: string;
  snapshot_pages: PageRange | null;
  pages: PageRange | null;
  overlap: PageRange | null;
  overlap_trimmed: boolean;
  is_head: boolean;
  byte_start: number;
  markdown: string;
}
export interface CanonicalReaderGap { kind: "gap"; pages: PageRange }
export type CanonicalReaderPiece = CanonicalReaderSegment | CanonicalReaderGap;

export interface CanonicalReaderProjection {
  canonical_id: string;
  source_pdf_artifact_id: string;
  head_artifact_id: string | null;
  total_pages: number | null;
  coverage: { covered: PageRange[]; missing: PageRange[]; head_covered: PageRange[] };
  pieces: CanonicalReaderPiece[];
  skipped: { conversion_id: string; markdown_artifact_id: string; pages: PageRange; is_head: boolean }[];
}

export function canonicalReaderProjection(canonicalId: string): Promise<CanonicalReaderProjection> {
  return invoke("get_canonical_reader_projection", { canonicalId });
}

export function pagesLabel(range: PageRange): string {
  return range.from === range.to ? `第 ${range.from} 页` : `第 ${range.from}–${range.to} 页`;
}

export function shortArtifactId(id: string): string {
  const tail = id.split(/[/:]/).pop() ?? id;
  return tail.length > 12 ? tail.slice(0, 12) : tail;
}

/** Header line shown above one stitched segment. */
export function segmentLabel(segment: CanonicalReaderSegment): string {
  const parts = [segment.pages ? pagesLabel(segment.pages) : "页码未知"];
  parts.push(`快照 ${shortArtifactId(segment.artifact_id)}${segment.is_head ? "（head）" : ""}`);
  if (segment.overlap) {
    parts.push(segment.overlap_trimmed
      ? `已略去与前文重叠的${pagesLabel(segment.overlap)}`
      : `${pagesLabel(segment.overlap)}与前文重叠（未找到页标记，重复显示）`);
  }
  return parts.join(" · ");
}

export function gapLabel(gap: CanonicalReaderGap): string {
  return `${pagesLabel(gap.pages)}尚未解析（此处没有 Markdown，不补写内容）`;
}

export type MarkdownChunk = { kind: "text"; text: string } | { kind: "page"; pageId: number };

const PAGE_MARKER = /^\s*\{(\d+)\}-{16,}\s*$/;

/** Splits Marker output on `{N}----` page markers (N = 0-based PDF page). */
export function splitByPageMarkers(markdown: string): MarkdownChunk[] {
  const chunks: MarkdownChunk[] = [];
  let buffer: string[] = [];
  const flush = () => {
    const text = buffer.join("\n");
    if (text.trim()) chunks.push({ kind: "text", text });
    buffer = [];
  };
  for (const line of markdown.split("\n")) {
    const match = PAGE_MARKER.exec(line);
    if (match) { flush(); chunks.push({ kind: "page", pageId: Number(match[1]) }); }
    else buffer.push(line);
  }
  flush();
  return chunks;
}

/** One-line summary of the stitched view. */
export function projectionSummary(projection: CanonicalReaderProjection): string {
  const segments = projection.pieces.filter(piece => piece.kind === "segment").length;
  const missing = projection.coverage.missing.reduce((sum, range) => sum + range.to - range.from + 1, 0);
  const total = projection.total_pages === null ? "总页数未知" : `共 ${projection.total_pages} 页`;
  const parts = [`统一阅读视图 · 由 ${segments} 个解析快照按页序拼接（只读）`, total];
  if (missing) parts.push(`${missing} 页尚未解析`);
  if (projection.skipped.length) parts.push(`${projection.skipped.length} 个快照被更早的快照完全覆盖`);
  return parts.join(" · ");
}
