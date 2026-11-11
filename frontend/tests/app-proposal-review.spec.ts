import { expect, test, type Page } from "@playwright/test";

async function openReview(page: Page): Promise<void> {
  await page.setViewportSize({ width: 1400, height: 860 });
  await page.goto("/tests/app.html");
  await page.locator("#sandbox-button").click();
  page.once("dialog", (dialog) => void dialog.accept());
  await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
  await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
  const dock = page.getByRole("complementary", { name: "沙箱对话" });
  await expect(dock).toBeVisible();
  await dock.getByRole("button", { name: "审核变更" }).click();
}

test.describe("Phase2 proposal review workspace", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => sessionStorage.setItem("harness.proposals", "1"));
  });

  test("manual review opens the full workspace; accept goes through the batch accept", async ({ page }) => {
    await openReview(page);
    const workspace = page.locator(".review-workspace");
    await expect(workspace).toBeVisible();
    await expect(workspace.locator(".review-header h2")).toContainText("提案审核");
    await expect(page.locator(".change-set-review-dialog")).toHaveCount(0);
    // The snapshot is read through the investigation, not get_markdown_document.
    await expect.poll(() => page.evaluate(() => window.appHarness.proposalReview.calls.filter(c => c.command === "snapshot").map(c => c.args))).toEqual([
      { investigationId: "sandbox:sbx_demo", artifactId: "md-sbx" },
    ]);
    // Overview: structured Debt, head-object problems, folded history.
    const list = workspace.locator(".review-list");
    await expect(list.locator(".review-debt-body")).toHaveText("PDF 第 2–7 页所在快照缺少分页标记");
    await expect(list.locator(".review-debt-tag")).toHaveText("locator");
    await expect(list.locator(".review-debt .review-debt-id")).toContainText("chapter-pages-1-7");
    await expect(list).not.toContainText("{");
    await expect(list).toContainText("工作区已有对象的问题");
    await expect(list).toContainText("旧定理");
    const history = list.locator(".review-proposal-history");
    await expect(history.locator("summary")).toHaveText("历史版本 2 个（已折叠）");
    await expect(history).not.toHaveAttribute("open", /.*/);
    // Preview: highlighted anchor with KaTeX, server page info.
    const preview = workspace.locator(".review-preview");
    await expect(preview.locator(".review-fragment .katex").first()).toBeVisible();
    await expect(preview).toContainText("PDF 第 2 页");
    await expect(preview.locator(".review-stale")).toHaveCount(0);
    await expect(workspace.locator(".review-footer")).toContainText("新建 2");
    await page.screenshot({ path: "../.test-runs/reader/proposal-workspace.png" });
    await workspace.getByRole("button", { name: "接受本提案" }).click();
    await expect(workspace.locator(".review-list")).toContainText("没有待审核提案了");
    const calls = await page.evaluate(() => window.appHarness.proposalReview.calls.map(c => c.command));
    expect(calls).toContain("plan");
    expect(calls).toContain("accept");
  });

  test("a proposal over several snapshots shows change-set totals, every relation and cross-snapshot links", async ({ page }) => {
    await page.addInitScript(() => sessionStorage.setItem("harness.multiSnapshot", "1"));
    await openReview(page);
    const workspace = page.locator(".review-workspace");
    const list = workspace.locator(".review-list");
    // Totals are change-set-wide, not the shown snapshot's.
    await expect(list.locator(".review-proposal-totals .review-stat")).toHaveText(["4单元", "4关系", "1Debt"]);
    await expect(list.locator(".review-proposal-scope")).toContainText("整份提案（v2）的总数，另含 4 处原文证据，分布在 3 个快照");
    // Default snapshot: the one with the most units + relations (md-a).
    await expect(list.locator(".review-snapshot-scope")).toHaveText("快照 1/3 · 本快照 2 单元 / 3 关系（其中 1 条跨快照）");
    const switcher = list.locator(".review-snapshot-switcher button");
    await expect(switcher).toHaveCount(3);
    await expect(switcher.nth(1)).toContainText("快照 2/3");
    await expect(switcher.nth(1)).toContainText("1 单元 / 2 关系");
    // Every relation of the change set, with both ends and the relation kind.
    const relations = list.locator(".review-proposal-relation");
    await expect(relations).toHaveCount(4);
    await expect(relations.nth(0)).toContainText("定义 1");
    await expect(relations.nth(0)).toContainText("定理 2");
    await expect(relations.nth(1)).toContainText("跨快照 · 快照 1 → 2");
    await expect(list.locator(".review-proposal-relation.is-cross")).toHaveCount(2);
    await expect(relations.nth(3)).toContainText("旧定理");
    await expect(relations.nth(3)).toContainText("连接已有对象");
    const graph = list.locator(".review-proposal-graph");
    await expect(graph).toHaveAttribute("data-relation-count", "4");
    await expect(graph).toHaveAttribute("data-external-count", "1");
    await page.screenshot({ path: "../.test-runs/reader/proposal-multi-snapshot-overview.png" });
    await list.locator(".review-proposal-relations").scrollIntoViewIfNeeded();
    await list.screenshot({ path: "../.test-runs/reader/proposal-multi-snapshot-relations.png" });

    // Another snapshot: its own counts, cross-snapshot relations still visible.
    await switcher.nth(1).click();
    await expect(list.locator(".review-snapshot-scope")).toHaveText("快照 2/3 · 本快照 1 单元 / 2 关系（其中 2 条跨快照）");
    await expect.poll(() => page.evaluate(() => window.appHarness.proposalReview.calls.filter(c => c.command === "snapshot").map(c => c.args.artifactId))).toContain("md-b");
    await expect(list.locator(".review-proposal-totals .review-stat").first()).toHaveText("4单元");
    await workspace.getByRole("tab", { name: "本快照关系 2" }).click();
    await expect(list.locator(".review-relation")).toHaveCount(2);
    await expect(list.locator(".review-relation.is-cross")).toHaveCount(2);
    await expect(list.locator(".review-relation").first()).toContainText("跨快照 · 快照 1/3");
    await workspace.getByRole("tab", { name: "局部结构图" }).click();
    await expect(list.locator(".review-graph")).toHaveAttribute("data-external-count", "2");
    await expect(list.locator(".review-graph")).toHaveAttribute("data-relation-count", "2");
    await page.screenshot({ path: "../.test-runs/reader/proposal-multi-snapshot-graph.png" });
  });

  test("a snapshot that cannot be read shows the real reason and an unchecked KaTeX quote", async ({ page }) => {
    await page.addInitScript(() => sessionStorage.setItem("harness.failSnapshot", "snapshot_not_referenced: 快照 md-sbx 未被调查引用"));
    await openReview(page);
    const preview = page.locator(".review-workspace .review-preview");
    await expect(preview.locator(".review-stale")).toContainText("snapshot_not_referenced");
    await expect(preview.locator(".review-unverified")).toContainText("未与快照核对");
    await expect(preview.locator(".review-unverified .katex").first()).toBeVisible();
    await page.screenshot({ path: "../.test-runs/reader/proposal-workspace-fallback.png" });
  });

  test("leaving the sandbox keeps the blocking batch dialog: checkboxes, total impact, accept / discard", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 860 });
    await page.goto("/tests/app.html");
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "返回母工作区" }).click();
    const dialog = page.locator(".change-set-review-dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.locator("h2")).toHaveText("切换沙箱前审核提案");
    const check = dialog.locator(".change-set-review-row:not(.change-set-review-head) input[type=checkbox]");
    await expect(check).toBeVisible();
    await expect(check).toBeChecked();
    await expect(dialog.locator(".change-set-review-summary")).toContainText("所选 1 个提案合计");
    await expect(dialog.getByRole("button", { name: "接受所选" })).toBeVisible();
    await expect(dialog.getByRole("button", { name: "丢弃所选" })).toBeVisible();
    // Columns line up: the head and the row share one column definition.
    const head = await dialog.locator(".change-set-review-head > span").nth(1).boundingBox();
    const title = await dialog.locator(".change-set-review-row:not(.change-set-review-head) .change-set-review-title").boundingBox();
    expect(Math.abs(head!.x - title!.x)).toBeLessThan(1);
    expect(title!.width).toBeGreaterThanOrEqual(240);
    await dialog.getByRole("button", { name: "丢弃所选" }).click();
    await expect(dialog.locator(".proposal-discard-confirm")).toContainText("没有其他提案依赖它们");
    await page.screenshot({ path: "../.test-runs/reader/proposal-batch-dialog.png" });
    await dialog.getByRole("button", { name: "取消", exact: true }).click();
    await dialog.getByRole("button", { name: "取消切换" }).click();
    await expect(dialog).toHaveCount(0);
  });

  test("an empty queue opens the workspace on an explicit empty state instead of doing nothing", async ({ page }) => {
    await page.addInitScript(() => sessionStorage.setItem("harness.proposals", "0"));
    await openReview(page);
    const workspace = page.locator(".review-workspace");
    await expect(workspace).toBeVisible();
    await expect(workspace.locator(".review-header h2")).toHaveText("提案审核");
    await expect(workspace.locator(".review-list")).toContainText("当前没有待审核的提案");
    await expect(workspace.locator(".review-preview")).toContainText("已接受或已丢弃的提案");
    await page.screenshot({ path: "../.test-runs/reader/proposal-workspace-empty.png" });
    await workspace.locator(".review-footer").getByRole("button", { name: "关闭" }).click();
    await expect(workspace).toBeHidden();
  });

  test("a failing queue read surfaces a toast above the dialogue dock", async ({ page }) => {
    await page.addInitScript(() => sessionStorage.setItem("harness.failQueue", "investigation_not_found: 调查不存在"));
    await openReview(page);
    const toast = page.locator("#toast");
    await expect(toast).toHaveClass(/is-visible/);
    await expect(toast).toContainText("无法读取提案：investigation_not_found");
    // The toast must not sit under the dock (it used to: z-index 40 < 60).
    const topmost = await toast.evaluate((node) => {
      // The toast ignores pointer events; let hit-testing see it for this probe.
      (node as HTMLElement).style.pointerEvents = "auto";
      const box = node.getBoundingClientRect();
      const hits = [[0.1, 0.5], [0.5, 0.5], [0.9, 0.5]].map(([x, y]) => document.elementFromPoint(box.left + box.width * x, box.top + box.height * y));
      (node as HTMLElement).style.pointerEvents = "";
      return hits.every(hit => hit === node || node.contains(hit));
    });
    expect(topmost).toBe(true);
  });
});
