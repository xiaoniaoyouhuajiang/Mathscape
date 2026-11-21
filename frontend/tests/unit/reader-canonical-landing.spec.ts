import { describe, expect, it } from "vitest";
import type { CanonicalReaderProjection, CanonicalReaderSegment } from "../../src/reader/canonical";
import {
  focusMatchesBlock,
  pageIndexOfBlock,
  pageMarkerIndex,
  planCanonicalLanding,
  snapshotPageSpan,
} from "../../src/reader/canonical-landing";

const DASHES = "-".repeat(48);
const A = `mathscape:artifact/sha256/${"a".repeat(64)}`;
const B = `mathscape:artifact/sha256/${"b".repeat(64)}`;
const SKIPPED = `mathscape:artifact/sha256/${"c".repeat(64)}`;

function segment(extra: Partial<CanonicalReaderSegment>): CanonicalReaderSegment {
  return {
    kind: "segment", conversion_id: "mathscape:conversion/marker/x", artifact_id: A,
    snapshot_pages: { from: 1, to: 4 }, pages: { from: 1, to: 4 }, overlap: null, overlap_trimmed: false,
    is_head: false, byte_start: 0, content_hash: "sha256:x", markdown: "", ...extra,
  };
}

// A covers pages 1–4. B was parsed as pages 3–6; its pages 3–4 overlap A and
// are trimmed, so only its blocks from byte 200 on are shown. Pages 7–8 are a gap.
const projection: Pick<CanonicalReaderProjection, "pieces"> = {
  pieces: [
    segment({
      blocks: [
        { id: "a-marker", kind: "page_marker", range: { start: 0, end: 52 } },
        { id: "a-1", kind: "text", range: { start: 52, end: 120 } },
        { id: "a-2", kind: "text", range: { start: 120, end: 260 } },
      ],
    }),
    segment({
      artifact_id: B, snapshot_pages: { from: 3, to: 6 }, pages: { from: 5, to: 6 },
      overlap: { from: 3, to: 4 }, overlap_trimmed: true, byte_start: 200,
      blocks: [
        { id: "b-3", kind: "text", range: { start: 200, end: 300 } },
        { id: "b-4", kind: "text", range: { start: 300, end: 420 } },
      ],
    }),
    { kind: "gap", pages: { from: 7, to: 8 } },
  ],
};

describe("planCanonicalLanding", () => {
  it("lands on the same block when its snapshot shows it", () => {
    expect(planCanonicalLanding(projection, { start: 300, end: 420, blockIds: ["b-4"], artifactId: B })).toEqual({ kind: "block" });
    expect(planCanonicalLanding(projection, { start: 60, end: 100, artifactId: A })).toEqual({ kind: "block" });
  });

  it("does not match another snapshot's block through an overlapping byte range", () => {
    // Bytes 60–100 exist in A, but the position belongs to B, whose bytes before 200 are trimmed.
    expect(planCanonicalLanding(projection, { start: 60, end: 100, artifactId: B })).toEqual({ kind: "needs_page" });
    expect(focusMatchesBlock({ start: 60, end: 100, artifactId: B }, A, { id: "a-1", range: { start: 52, end: 120 } })).toBe(false);
  });

  it("falls back to the PDF page for a block in a trimmed overlap", () => {
    const focus = { start: 20, end: 80, blockIds: ["b-1"], artifactId: B };
    expect(planCanonicalLanding(projection, focus)).toEqual({ kind: "needs_page" });
    expect(planCanonicalLanding(projection, { ...focus, pdfPageIndex: 2 })).toEqual({ kind: "page", pageIndex: 2 });
    expect(planCanonicalLanding(projection, focus, 3)).toEqual({ kind: "page", pageIndex: 3 });
  });

  it("falls back to the PDF page for a snapshot skipped as fully covered", () => {
    expect(planCanonicalLanding(projection, { start: 0, end: 40, blockIds: ["c-1"], artifactId: SKIPPED, pdfPageIndex: 1 }))
      .toEqual({ kind: "page", pageIndex: 1 });
  });

  it("keeps the snapshot when the page is not readable in the view", () => {
    expect(planCanonicalLanding(projection, { start: 0, end: 40, blockIds: ["c-1"], artifactId: SKIPPED, pdfPageIndex: 6 }))
      .toEqual({ kind: "snapshot" });
  });

  it("never treats a page marker as the located block", () => {
    expect(planCanonicalLanding(projection, { start: 0, end: 52, blockIds: ["a-marker"], artifactId: A, pdfPageIndex: 0 }))
      .toEqual({ kind: "page", pageIndex: 0 });
  });

  it("opens at the snapshot's segment when no block is asked for", () => {
    expect(planCanonicalLanding(projection, { start: 0, end: 0, artifactId: B })).toEqual({ kind: "segment", artifactId: B });
    expect(planCanonicalLanding(projection, { start: 0, end: 0, artifactId: SKIPPED })).toEqual({ kind: "none" });
    expect(planCanonicalLanding(projection, { start: 0, end: 0 })).toEqual({ kind: "none" });
  });

  it("keeps the old id / byte-range match when no owning snapshot is given", () => {
    expect(planCanonicalLanding(projection, { start: 0, end: 0, blockIds: ["b-3"] })).toEqual({ kind: "block" });
    expect(planCanonicalLanding(projection, { start: 0, end: 0, blockIds: ["missing"] })).toEqual({ kind: "none" });
  });
});

describe("snapshot page markers", () => {
  const texts = [`{32}${DASHES}`, "定理 4", `{33}${DASHES}\n`, "证明", "{9}-----"];
  const blocks = ["page_marker", "text", "page_marker", "text", "page_marker"].map(kind => ({ kind }));
  const textOf = (index: number) => texts[index];

  it("reads 0-based PDF pages from Marker page markers", () => {
    expect(pageMarkerIndex(`{32}${DASHES}`)).toBe(32);
    expect(pageMarkerIndex("{9}-----")).toBeUndefined();
    expect(pageMarkerIndex("定理 4")).toBeUndefined();
  });

  it("finds the pages a snapshot starts and ends on", () => {
    expect(snapshotPageSpan(blocks, textOf)).toEqual({ first: 32, last: 33 });
    expect(snapshotPageSpan([{ kind: "text" }], () => "无页标记")).toBeNull();
  });

  it("finds the page a block sits on", () => {
    expect(pageIndexOfBlock(blocks, textOf, 1)).toBe(32);
    expect(pageIndexOfBlock(blocks, textOf, 3)).toBe(33);
    expect(pageIndexOfBlock([{ kind: "text" }], () => "无页标记", 0)).toBeUndefined();
  });
});
