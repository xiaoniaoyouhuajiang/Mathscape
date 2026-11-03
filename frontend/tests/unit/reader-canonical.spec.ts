import { describe, expect, it } from "vitest";
import {
  gapLabel,
  pagesLabel,
  projectionSummary,
  segmentLabel,
  splitByPageMarkers,
  type CanonicalReaderProjection,
  type CanonicalReaderSegment,
} from "../../src/reader/canonical";

const DASHES = "-".repeat(48);
const A = `mathscape:artifact/${"a".repeat(64)}`;
const B = `mathscape:artifact/${"b".repeat(64)}`;

function segment(extra: Partial<CanonicalReaderSegment>): CanonicalReaderSegment {
  return {
    kind: "segment", conversion_id: "mathscape:conversion/marker/x", artifact_id: A,
    snapshot_pages: { from: 1, to: 4 }, pages: { from: 1, to: 4 }, overlap: null, overlap_trimmed: false,
    is_head: false, byte_start: 0, markdown: "", ...extra,
  };
}

describe("splitByPageMarkers", () => {
  it("splits Marker output into text and 0-based page markers", () => {
    const text = `{4}${DASHES}\n\n# 第五页\n\n正文\n\n{5}${DASHES}\n\n第六页\n`;
    expect(splitByPageMarkers(text)).toEqual([
      { kind: "page", pageId: 4 },
      { kind: "text", text: "\n# 第五页\n\n正文\n" },
      { kind: "page", pageId: 5 },
      { kind: "text", text: "\n第六页\n" },
    ]);
  });

  it("leaves short dash runs and unpaginated markdown alone", () => {
    expect(splitByPageMarkers("# 无页标记\n\n{3}-----\n")).toEqual([{ kind: "text", text: "# 无页标记\n\n{3}-----\n" }]);
    expect(splitByPageMarkers("")).toEqual([]);
  });
});

describe("labels", () => {
  it("names page ranges, snapshots and the overlap policy", () => {
    expect(pagesLabel({ from: 3, to: 3 })).toBe("第 3 页");
    expect(segmentLabel(segment({}))).toBe("第 1–4 页 · 快照 aaaaaaaaaaaa");
    expect(segmentLabel(segment({ artifact_id: B, is_head: true, pages: { from: 5, to: 6 }, overlap: { from: 3, to: 4 }, overlap_trimmed: true })))
      .toBe("第 5–6 页 · 快照 bbbbbbbbbbbb（head） · 已略去与前文重叠的第 3–4 页");
    expect(segmentLabel(segment({ overlap: { from: 3, to: 3 } }))).toContain("重复显示");
    expect(segmentLabel(segment({ pages: null }))).toContain("页码未知");
  });

  it("marks gaps without inventing text", () => {
    expect(gapLabel({ kind: "gap", pages: { from: 7, to: 8 } })).toBe("第 7–8 页尚未解析（此处没有 Markdown，不补写内容）");
  });

  it("summarizes the stitched view", () => {
    const projection: CanonicalReaderProjection = {
      canonical_id: "mathscape:canonical-markdown/v1/p/m/" + A, source_pdf_artifact_id: A, head_artifact_id: B, total_pages: 10,
      coverage: { covered: [{ from: 1, to: 6 }, { from: 9, to: 9 }], missing: [{ from: 7, to: 8 }, { from: 10, to: 10 }], head_covered: [] },
      pieces: [segment({}), segment({ artifact_id: B }), { kind: "gap", pages: { from: 7, to: 8 } }],
      skipped: [{ conversion_id: "c", markdown_artifact_id: A, pages: { from: 2, to: 3 }, is_head: false }],
    };
    expect(projectionSummary(projection)).toBe("统一阅读视图 · 由 2 个解析快照按页序拼接（只读） · 共 10 页 · 3 页尚未解析 · 1 个快照被更早的快照完全覆盖");
  });
});
