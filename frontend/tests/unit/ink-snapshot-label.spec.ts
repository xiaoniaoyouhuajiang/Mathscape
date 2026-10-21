import { describe, expect, it } from "vitest";
import { INK_SNAPSHOT_LABEL_MAX_CHARS, inkSnapshotLabelText, normalizeInkSnapshotLabel } from "../../src/ink/persistence";

describe("snapshot labels (mirrors ink_snapshot_label.rs)", () => {
  it("trims, and an empty or blank name clears", () => {
    expect(normalizeInkSnapshotLabel("  整理稿 \n")).toBe("整理稿");
    expect(normalizeInkSnapshotLabel("   ")).toBe("");
    expect(normalizeInkSnapshotLabel("")).toBe("");
  });
  it("counts characters, not UTF-16 units or bytes", () => {
    expect(INK_SNAPSHOT_LABEL_MAX_CHARS).toBe(40);
    expect(normalizeInkSnapshotLabel("𝔽".repeat(40))).toHaveLength(80);
    expect(normalizeInkSnapshotLabel("名".repeat(40))).toBe("名".repeat(40));
    expect(() => normalizeInkSnapshotLabel("名".repeat(41))).toThrow("最多 40 个字符");
    expect(() => normalizeInkSnapshotLabel("a\u0007b")).toThrow("控制字符");
  });
  it("formats the name for status lines", () => {
    expect(inkSnapshotLabelText({ label: "最终版" } as never)).toBe("「最终版」");
    expect(inkSnapshotLabelText({ label: null } as never)).toBe("");
  });
});
