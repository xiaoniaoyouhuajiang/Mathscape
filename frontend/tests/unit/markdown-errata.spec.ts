import { describe, expect, it } from "vitest";
import {
  buildBlockErrataLookup,
  computeErrataCoverRange,
  computeLineDiff,
  containsPageMarkerLine,
  type ErrataBlockSourceItem,
  type MarkdownBlockErrata,
} from "../../src/reader/markdown-errata";

function makeItem(id: string, start: number, end: number, rawText: string, kind = "text"): ErrataBlockSourceItem {
  return { block: { id, kind, range: { start, end } }, rawText };
}

function makeErrata(partial: Partial<MarkdownBlockErrata> & Pick<MarkdownBlockErrata, "errata_id" | "first_block_id" | "last_block_id" | "covered_block_ids" | "base_byte_start" | "base_byte_end" | "original_text" | "corrected_text">): MarkdownBlockErrata {
  return {
    artifact_id: "md",
    base_content_hash: "sha256:abc",
    base_text_hash: "sha256:def",
    status: "active",
    source_kind: "manual",
    actor: "mathscape:actor/desktop-user",
    created_at: "2026-11-19T11:00:00+08:00",
    updated_at: "2026-11-19T11:00:00+08:00",
    ...partial,
  };
}

describe("markdown-errata pure helpers", () => {
  it("expands overlapping active errata ranges into a clean union and composes initial effective text", () => {
    const items = [
      makeItem("b0", 0, 10, "Block 0\n"),
      makeItem("b1", 12, 24, "OCR $x^a$\n"),
      makeItem("b2", 26, 38, "OCR part 2\n"),
      makeItem("b3", 40, 56, "{1}----------------\n", "page_marker"),
      makeItem("b4", 58, 70, "Page 2\n"),
    ];
    const existing = [
      makeErrata({
        errata_id: "e1",
        first_block_id: "b0",
        last_block_id: "b1",
        covered_block_ids: ["b0", "b1"],
        base_byte_start: 0,
        base_byte_end: 24,
        original_text: "Block 0\n\nOCR $x^a$",
        corrected_text: "Merged 0+1: $x^n$\n",
      }),
    ];

    // Selecting [1..=2] expands backward to [0..=2] to cover e1 cleanly.
    const cover = computeErrataCoverRange(items, 1, 2, existing);
    expect(cover.ok).toBe(true);
    if (cover.ok) {
      expect(cover.firstBlockId).toBe("b0");
      expect(cover.lastBlockId).toBe("b2");
      expect(cover.coveredBlockIds).toEqual(["b0", "b1", "b2"]);
      expect(cover.initialText).toBe("Merged 0+1: $x^n$\n\nOCR part 2");
      expect(cover.overlappingErrata.map((e) => e.errata_id)).toEqual(["e1"]);
    }

    // Crossing a page_marker is rejected cleanly.
    const bad = computeErrataCoverRange(items, 2, 4, existing);
    expect(bad.ok).toBe(false);
  });

  it("indexes primary slot blocks and hidden secondary blocks for multi-block errata", () => {
    const lookup = buildBlockErrataLookup([
      makeErrata({
        errata_id: "e-merge",
        first_block_id: "b1",
        last_block_id: "b3",
        covered_block_ids: ["b1", "b2", "b3"],
        base_byte_start: 10,
        base_byte_end: 90,
        original_text: "a",
        corrected_text: "b",
      }),
      makeErrata({
        errata_id: "e-reverted",
        first_block_id: "b4",
        last_block_id: "b4",
        covered_block_ids: ["b4"],
        base_byte_start: 95,
        base_byte_end: 110,
        original_text: "c",
        corrected_text: "d",
        status: "reverted",
      }),
    ]);

    expect(lookup.primaryByBlockId.get("b1")?.errata_id).toBe("e-merge");
    expect(lookup.hiddenSecondaryBlockIds.has("b1")).toBe(false);
    expect(lookup.hiddenSecondaryBlockIds.has("b2")).toBe(true);
    expect(lookup.hiddenSecondaryBlockIds.has("b3")).toBe(true);
    expect(lookup.primaryByBlockId.has("b4")).toBe(false);
  });

  it("computes deterministic LCS line diffs and detects Marker page separator lines", () => {
    const diff = computeLineDiff("Theorem 1\nLet $x^a = 1$.\nEnd.", "Theorem 1\nLet $x^n = 1$.\nExtra line.\nEnd.");
    expect(diff.changed).toBe(true);
    expect(diff.removed).toBe(1);
    expect(diff.added).toBe(2);
    expect(diff.lines.some((l) => l.kind === "del" && l.text.includes("x^a"))).toBe(true);
    expect(diff.lines.some((l) => l.kind === "add" && l.text.includes("x^n"))).toBe(true);

    expect(containsPageMarkerLine("ok\n{12}--------------------------------\nnext")).toBe(true);
    expect(containsPageMarkerLine("normal line --- dashes")).toBe(false);
  });
});
