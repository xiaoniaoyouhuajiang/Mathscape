import { describe, expect, it } from "vitest";
import {
  buildDocumentOutline,
  extractMarkdownHeadings,
  matchUnitsToBlocks,
} from "../../src/reader/markdown-outline";
import type { MarkdownUnitMarker } from "../../src/reader/content";

describe("markdown-outline pure helpers", () => {
  it("extracts ATX headings across levels while ignoring fenced code blocks and page markers", () => {
    const blocks = [
      { id: "b0", domKey: "0:b0", text: "{0}--------------------\n", kind: "page_marker" },
      { id: "b1", domKey: "0:b1", text: "# 第二章 **形式幂级数**\n", kind: "heading" },
      { id: "b2", domKey: "0:b2", text: "```sh\n# comment not heading\n```\n", kind: "code" },
      { id: "b3", domKey: "0:b3", text: "## 2.1 定义与记号 ###\n", kind: "heading" },
      { id: "b4", domKey: "0:b4", text: "普通正文段落 $a_n$。\n", kind: "text" },
    ];
    expect(extractMarkdownHeadings(blocks)).toEqual([
      { domKey: "0:b1", blockId: "b1", level: 1, title: "第二章 形式幂级数" },
      { domKey: "0:b3", blockId: "b3", level: 2, title: "2.1 定义与记号" },
    ]);
  });

  it("matches registered SourceSemanticUnits by block_ids or byte_range and merges section headings without duplication", () => {
    const blocks = [
      { id: "b1", domKey: "0:b1", artifactId: "art-a", range: { start: 0, end: 30 }, text: "# 第二章 形式幂级数\n", kind: "heading" },
      { id: "b2", domKey: "0:b2", artifactId: "art-a", range: { start: 30, end: 90 }, text: "**定理 1** $(1-x)^{-n}$ 的展开。\n", kind: "text" },
      { id: "b3", domKey: "1:b0", artifactId: "art-b", range: { start: 0, end: 60 }, text: "## 2.2 组合应用\n", kind: "heading" },
    ];
    const units: MarkdownUnitMarker[] = [
      {
        unit_id: "unit-sec",
        title: "第二章 形式幂级数",
        unit_kind: "section",
        artifact_id: "art-a",
        byte_range: { start: 0, end: 30 },
        block_ids: ["b1"],
      },
      {
        unit_id: "unit-thm1",
        title: "定理 1：(1-x)^(-n) 的展开",
        unit_kind: "statement",
        artifact_id: "art-a",
        byte_range: { start: 35, end: 80 },
        block_ids: [],
      },
    ];

    const headings = extractMarkdownHeadings(blocks);
    const matched = matchUnitsToBlocks(units, blocks);
    expect(matched.map((m) => m.domKey)).toEqual(["0:b1", "0:b2"]);

    const outline = buildDocumentOutline(
      headings,
      matched,
      blocks.map((b) => b.domKey),
    );
    expect(outline).toEqual([
      {
        kind: "unit",
        domKey: "0:b1",
        blockId: "b1",
        level: 1,
        title: "第二章 形式幂级数",
        unitId: "unit-sec",
        unitKind: "section",
      },
      {
        kind: "unit",
        domKey: "0:b2",
        blockId: "b2",
        level: 3,
        title: "定理 1：(1-x)^(-n) 的展开",
        unitId: "unit-thm1",
        unitKind: "statement",
      },
      {
        kind: "heading",
        domKey: "1:b0",
        blockId: "b3",
        level: 2,
        title: "2.2 组合应用",
      },
    ]);
  });
});
