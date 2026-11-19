import { describe, expect, it } from "vitest";
import {
  formatPadDraftLabel,
  reconcilePadDrafts,
  type PadDraftSummary,
} from "../../src/ink/pad-drafts";

function draft(partial: Partial<PadDraftSummary> & Pick<PadDraftSummary, "key" | "documentId">): PadDraftSummary {
  return {
    title: "测试手稿",
    writerEpoch: 1,
    updatedAt: "2026-11-19T12:00:00.000Z",
    pendingCount: 0,
    needsReauth: false,
    ...partial,
  };
}

describe("reconcilePadDrafts", () => {
  it("auto-prunes older zero-pending epochs of the same document while keeping the newest", () => {
    const plan = reconcilePadDrafts(
      [
        draft({ key: "s1:2", documentId: "doc-a", title: "RDTX建模", writerEpoch: 2, needsReauth: true, updatedAt: "2026-11-18T10:00:00Z" }),
        draft({ key: "s2:3", documentId: "doc-a", title: "RDTX建模", writerEpoch: 3, needsReauth: true, updatedAt: "2026-11-19T08:00:00Z" }),
        draft({ key: "s3:4", documentId: "doc-a", title: "RDTX建模", writerEpoch: 4, needsReauth: false, updatedAt: "2026-11-19T12:00:00Z" }),
      ],
      "s3:4",
    );

    expect(plan.pruneKeys.sort()).toEqual(["s1:2", "s2:3"]);
    expect(plan.active.map((d) => d.key)).toEqual(["s3:4"]);
    expect(plan.pendingOffline).toEqual([]);
    expect(plan.readonlyHistory).toEqual([]);
    expect(plan.cleanableStaleKeys).toEqual([]);
  });

  it("NEVER prunes or marks cleanable any draft with pendingCount > 0", () => {
    const plan = reconcilePadDrafts(
      [
        draft({ key: "s1:2", documentId: "doc-a", title: "柯西准则", writerEpoch: 2, needsReauth: true, pendingCount: 3 }),
        draft({ key: "s2:3", documentId: "doc-a", title: "柯西准则", writerEpoch: 3, needsReauth: true, pendingCount: 0 }),
        draft({ key: "s3:4", documentId: "doc-a", title: "柯西准则", writerEpoch: 4, needsReauth: false, pendingCount: 0 }),
        draft({ key: "s9:1", documentId: "doc-b", title: "离线未同步", writerEpoch: 1, needsReauth: true, pendingCount: 1 }),
      ],
      "s3:4",
    );

    // Only zero-pending superseded s2:3 is pruned; s1:2 and s9:1 MUST survive!
    expect(plan.pruneKeys).toEqual(["s2:3"]);
    expect(plan.cleanableStaleKeys).toEqual([]);
    expect(plan.active.map((d) => d.key)).toEqual(["s3:4"]);
    expect(plan.pendingOffline.map((d) => d.key).sort()).toEqual(["s1:2", "s9:1"]);
    expect(plan.readonlyHistory).toEqual([]);
  });

  it("NEVER prunes or marks cleanable the currently active key even if older or needsReauth", () => {
    const plan = reconcilePadDrafts(
      [
        draft({ key: "s1:2", documentId: "doc-a", title: "例 1.2.7", writerEpoch: 2, needsReauth: true, updatedAt: "2026-11-19T10:00:00Z" }),
        draft({ key: "s2:3", documentId: "doc-a", title: "例 1.2.7", writerEpoch: 3, needsReauth: true, updatedAt: "2026-11-19T11:00:00Z" }),
        draft({ key: "s9:1", documentId: "doc-b", title: "cauchy抄写", writerEpoch: 1, needsReauth: true, updatedAt: "2026-11-18T11:00:00Z" }),
      ],
      "s1:2",
    );

    // s1:2 is currently open -> kept untouched! s2:3 is newest epoch of doc-a (non-active, zero-pending, needsReauth) -> cleanable.
    expect(plan.pruneKeys).toEqual([]);
    expect(plan.cleanableStaleKeys.sort()).toEqual(["s2:3", "s9:1"]);
    expect(plan.readonlyHistory.map((d) => d.key)).toEqual(["s2:3", "s1:2", "s9:1"]);
  });

  it("identifies remaining zero-pending needsReauth shells for one-tap cleanup while keeping active drafts", () => {
    const plan = reconcilePadDrafts(
      [
        draft({ key: "a:1", documentId: "doc-a", title: "当前手稿", writerEpoch: 1, needsReauth: false, updatedAt: "2026-11-19T12:00:00Z" }),
        draft({ key: "b:2", documentId: "doc-b", title: "已失效旧壳1", writerEpoch: 2, needsReauth: true, updatedAt: "2026-11-19T09:00:00Z" }),
        draft({ key: "c:1", documentId: "doc-c", title: "已失效旧壳2", writerEpoch: 1, needsReauth: true, updatedAt: "2026-11-18T09:00:00Z" }),
      ],
      "a:1",
    );

    expect(plan.pruneKeys).toEqual([]);
    expect(plan.active.map((d) => d.key)).toEqual(["a:1"]);
    expect(plan.readonlyHistory.map((d) => d.key)).toEqual(["b:2", "c:1"]);
    expect(plan.cleanableStaleKeys.sort()).toEqual(["b:2", "c:1"]);
  });

  it("formats clear Chinese labels preserving 最近手稿 for active entries", () => {
    expect(formatPadDraftLabel(draft({ key: "a:1", documentId: "a", title: "尝试证明" }))).toBe("尝试证明 · 最近手稿");
    expect(formatPadDraftLabel(draft({ key: "a:1", documentId: "a", title: "尝试证明", pendingCount: 2 }))).toBe("尝试证明 · 最近手稿（待同步 2）");
    expect(formatPadDraftLabel(draft({ key: "a:1", documentId: "a", title: "尝试证明", needsReauth: true, pendingCount: 2 }))).toBe("尝试证明 · 待同步 2（需导出恢复）");
    expect(formatPadDraftLabel(draft({ key: "a:1", documentId: "a", title: "尝试证明", needsReauth: true, pendingCount: 0 }))).toBe("尝试证明 · 仅本机历史（需电脑重新推送）");
  });
});
