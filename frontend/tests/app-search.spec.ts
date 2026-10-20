import { expect, test, type Page } from "@playwright/test";

const mod = process.platform === "darwin" ? "Meta" : "Control";

async function openSearch(page: Page, text?: string): Promise<void> {
  await page.keyboard.press(`${mod}+k`);
  await expect(page.getByRole("dialog", { name: "搜索工作区" })).toBeVisible();
  if (text !== undefined) await page.locator("#workspace-search-input").fill(text);
}

const options = (page: Page) => page.locator("#workspace-search-results [role=option]");

async function nodeView(page: Page, id: string) {
  return await page.evaluate(nodeId => window.appHarness.nodeView(nodeId), id);
}

test.describe("N3d workspace search", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 860 });
    await page.goto("/tests/app.html");
    await expect(page.locator("#ink-button")).toBeVisible();
    await expect.poll(() => page.evaluate(() => Object.keys(window.appHarness.labels()).length)).toBeGreaterThan(0);
  });

  test("⌘K opens a centred, bounded panel; Esc closes it and restores focus", async ({ page }) => {
    await page.locator("#workspace-search-button").focus();
    await openSearch(page);
    await expect(page.locator("#workspace-search-input")).toBeFocused();
    const geometry = async () => page.evaluate(() => {
      const box = document.querySelector(".workspace-search-box")!.getBoundingClientRect();
      const results = document.querySelector("#workspace-search-results")!.getBoundingClientRect();
      return { left: box.left, right: box.right, top: box.top, bottom: box.bottom, width: box.width, resultsBottom: results.bottom, vw: innerWidth, vh: innerHeight };
    });
    let g = await geometry();
    expect(Math.abs((g.left + g.right) / 2 - g.vw / 2)).toBeLessThan(1.5);
    expect(g.width).toBeCloseTo(640, 0);
    expect(g.top).toBeCloseTo(g.vh * 0.12, 0);
    // Many results stay inside the viewport: the list scrolls, not the page.
    await page.evaluate(() => {
      const h = window.appHarness;
      for (let i = 0; i < 40; i++) h.storedDocuments.set("many-" + i, { ...structuredClone(h.storedDocuments.get(h.documents.a)!), document_id: "many-" + i, title: "批量手稿 " + i });
      for (let i = 0; i < 40; i++) h.notes.set("many-" + i, { title_mode: "manual", title_revision: 0, lifecycle: "active", lifecycle_revision: 0, trashed_at: null, last_used_at: i });
    });
    await page.locator("#workspace-search-input").fill("批量");
    await expect(options(page)).toHaveCount(40);
    g = await geometry();
    expect(g.bottom).toBeLessThanOrEqual(g.vh);
    expect(g.resultsBottom).toBeLessThanOrEqual(g.bottom);
    expect(await page.evaluate(() => { const r = document.querySelector("#workspace-search-results")!; return r.scrollHeight > r.clientHeight; })).toBe(true);
    // ArrowUp from the first wraps to the last and scrolls it into view.
    await page.keyboard.press("ArrowUp");
    const last = options(page).last();
    await expect(last).toHaveAttribute("aria-selected", "true");
    const inView = await last.evaluate(el => { const r = el.getBoundingClientRect(), list = document.querySelector("#workspace-search-results")!.getBoundingClientRect(); return r.top >= list.top - 1 && r.bottom <= list.bottom + 1; });
    expect(inView).toBe(true);
    // Narrow window: the box shrinks to the viewport minus margins.
    await page.setViewportSize({ width: 500, height: 700 });
    g = await geometry();
    expect(g.width).toBeCloseTo(500 - 32, 0);
    expect(g.left).toBeCloseTo(16, 0);
    await page.keyboard.press("Escape");
    await expect(page.locator("#workspace-search")).toBeHidden();
    await expect(page.locator("#workspace-search-button")).toBeFocused();
  });

  test("results are grouped by type with highlights; Enter on a source unit opens the reader at its source", async ({ page }) => {
    await page.locator("#workspace-search-button").click();
    await page.locator("#workspace-search-input").fill("定理");
    const group = page.locator(".workspace-search-group[data-kind=source_unit]");
    await expect(group.locator(".workspace-search-group-title")).toHaveText("来源单元 · 2");
    await expect(group.locator("mark").first()).toHaveText("定理");
    await expect(options(page).first()).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("Enter");
    await expect(page.locator("#workspace-search")).toBeHidden();
    // source_object → detail lookup → the containing Markdown with focus.
    await expect.poll(() => page.evaluate(() => window.appHarness.detailRequests)).toContain("unit-thm");
    await expect(page.locator("#reader .markdown-body")).toContainText("正确表述");
    await expect(page.locator("#reader .source-block-focus")).toHaveAttribute("data-source-block", "b1");
    await expect(page.locator("#reader")).toBeVisible();
    await expect(page.locator("#reader")).toContainText("设");
    await expect(page.locator("#reader")).not.toContainText("正在读取");
    const view = await nodeView(page, "r-unit-thm");
    expect(view?.selected).toBe(true);
    await expect(page.locator("#node-detail")).toContainText("定理 3.2");
  });

  test("Alt+Enter and 定位 select and centre the map node without opening the reader", async ({ page }) => {
    // Move the viewport far away first so centring is observable.
    await page.evaluate(() => window.appHarness.panTo(-3000, -2000));
    const before = await nodeView(page, "r-unit-ex");
    expect(Math.hypot(before!.dx, before!.dy)).toBeGreaterThan(200);
    await openSearch(page, "例 4");
    await expect(options(page)).toHaveCount(1);
    await page.keyboard.press("Alt+Enter");
    await expect(page.locator("#workspace-search")).toBeHidden();
    await expect.poll(async () => { const v = await nodeView(page, "r-unit-ex"); return v && Math.hypot(v.dx, v.dy) < 2 && v.selected; }).toBe(true);
    await expect(page.locator("#reader")).toBeHidden();
    // Only the inspector looked the unit up; no reader content was fetched.
    expect(await page.evaluate(() => window.appHarness.contentRequests)).toEqual([]);
    // The 定位 button does the same for a material.
    await openSearch(page, "notes");
    await page.locator(".workspace-search-option[data-id=pdf-notes] .workspace-search-locate").click();
    await expect.poll(async () => (await nodeView(page, "r-pdf-notes"))?.selected).toBe(true);
    expect((await nodeView(page, "r-unit-ex"))?.selected).toBe(false);
    await expect(page.locator("#reader")).toBeHidden();
  });

  test("a note opens the handwriting page; from 手稿 a source result returns to the map", async ({ page }) => {
    await openSearch(page, "手稿 b");
    await expect(page.locator(".workspace-search-group[data-kind=note] [role=option]")).toHaveCount(1);
    await page.keyboard.press("Enter");
    await expect(page.locator("#ink-workspace")).toBeVisible();
    await expect(page.locator("#ink-document-heading")).toContainText("手稿 B");
    // The 搜索 button in the top bar stays available on the handwriting page.
    await page.locator("#workspace-search-button").click();
    await expect(page.locator("#workspace-search")).toBeVisible();
    await page.locator("#workspace-search-input").fill("例 4");
    await page.keyboard.press("Alt+Enter");
    await expect(page.locator("#ink-workspace")).toBeHidden();
    await expect(page.locator("#graph")).toBeVisible();
    await expect.poll(async () => { const v = await nodeView(page, "r-unit-ex"); return v && v.width > 0 && Math.hypot(v.dx, v.dy) < 2 && v.selected; }).toBe(true);
  });

  test("in split mode source results open in the left pane and the note stays", async ({ page }) => {
    await page.locator("#ink-button").click();
    await page.getByRole("button", { name: /手稿 A/ }).click();
    await page.locator("#ink-split-toggle").click();
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    await openSearch(page, "定理 3.2");
    await page.keyboard.press("Enter");
    const left = page.locator("#ink-split-reader");
    await expect(left.locator("#reader")).toBeVisible();
    await expect(left.locator("#reader")).toContainText("设");
    await expect(page.locator("#ink-document-heading")).toContainText("手稿 A");
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    const panes = await page.evaluate(() => {
      const l = document.querySelector("#ink-split-reader")!.getBoundingClientRect();
      const r = document.querySelector("#ink-desktop-host")!.getBoundingClientRect();
      const reader = document.querySelector("#reader")!.getBoundingClientRect();
      return { l, r, reader };
    });
    expect(panes.l.width).toBeGreaterThan(200);
    expect(panes.r.left).toBeGreaterThanOrEqual(panes.l.right - 1);
    expect(panes.reader.left).toBeGreaterThanOrEqual(panes.l.left - 1);
    expect(panes.reader.right).toBeLessThanOrEqual(panes.l.right + 1);
    // The extra 搜索 button must not push the header actions out of the
    // (narrowest allowed) pane.
    await page.setViewportSize({ width: 980, height: 860 });
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    const clipped = await page.evaluate(() => {
      const host = document.querySelector("#ink-desktop-host")!.getBoundingClientRect();
      return [...document.querySelectorAll<HTMLElement>("#ink-desktop-host .ink-desktop-actions button")]
        .filter(button => button.offsetParent && (button.getBoundingClientRect().right > host.right + 1 || button.getBoundingClientRect().left < host.left - 1))
        .map(button => button.textContent);
    });
    expect(clipped).toEqual([]);
    await expect(page.locator("#workspace-search-button")).toBeInViewport();
  });

  test("a slow source lookup is dropped when the user navigates elsewhere", async ({ page }) => {
    await page.evaluate(() => { window.appHarness.sourceDetailBehavior.delayMs = 500; });
    await openSearch(page, "定理 3.2");
    await page.keyboard.press("Enter");
    await expect.poll(() => page.evaluate(() => window.appHarness.detailRequests.length)).toBeGreaterThan(0);
    await openSearch(page, "手稿 b");
    await expect(options(page)).toHaveCount(1);
    await page.keyboard.press("Enter");
    await expect(page.locator("#ink-document-heading")).toContainText("手稿 B");
    await page.waitForTimeout(700);
    await expect(page.locator("#ink-workspace")).toBeVisible();
    await expect(page.locator("#reader")).toBeHidden();
    expect(await page.evaluate(() => window.appHarness.contentRequests)).toEqual([]);
  });

  test("stale search responses never replace newer ones; Enter waits for the current query", async ({ page }) => {
    await page.evaluate(() => { window.appHarness.searchBehavior.delays = { "例": 400, "定理": 150 }; });
    await openSearch(page);
    const input = page.locator("#workspace-search-input");
    await input.fill("例");
    await page.waitForTimeout(160); // 例 is now in flight
    await input.fill("定理");
    // Enter before 定理 answered: acts on the 定理 results, not on 例.
    await page.keyboard.press("Enter");
    await expect(page.locator("#workspace-search")).toBeHidden();
    await expect(page.locator("#reader .markdown-body")).toContainText("正确表述");
    expect(await page.evaluate(() => window.appHarness.detailRequests)).not.toContain("unit-ex");
    expect(await page.evaluate(() => window.appHarness.searchBehavior.calls.map(call => call.text))).toEqual(["例", "定理"]);
    await page.waitForTimeout(450); // the stale 例 answer lands now
    await expect(page.locator("#workspace-search")).toBeHidden();
    expect(await page.evaluate(() => window.appHarness.detailRequests)).not.toContain("unit-ex");
    expect((await nodeView(page, "r-unit-thm"))?.selected).toBe(true);
    // A slow answer shows a status, and a stale one is ignored.
    await page.evaluate(() => { window.appHarness.searchBehavior.delays = { "例 4": 500, "notes": 0 }; });
    await openSearch(page, "例 4");
    await expect(page.locator("#workspace-search-status")).toHaveText("正在搜索…");
    await input.fill("notes");
    await expect(options(page)).toHaveCount(1);
    await expect(options(page).first()).toContainText("notes");
    await page.waitForTimeout(600);
    await expect(options(page)).toHaveCount(1);
    await expect(options(page).first()).toContainText("notes");
  });

  test("open and search failures are reported, never a blank pane", async ({ page }) => {
    await page.evaluate(() => { window.appHarness.contentFailures.add("md-1"); });
    await openSearch(page, "正确表述");
    await page.keyboard.press("Enter");
    await expect(page.locator("#toast")).toContainText("无法打开原文");
    await expect(page.locator("#reader")).toContainText("Artifact 不存在");
    await expect.poll(async () => (await nodeView(page, "r-md-1"))?.selected).toBe(true);
    // A source object whose detail lookup fails.
    await page.evaluate(() => { window.appHarness.sourceDetailBehavior.failIds = ["unit-ex"]; });
    await openSearch(page, "例 4");
    await page.keyboard.press("Enter");
    await expect(page.locator("#toast")).toContainText("来源对象读取失败");
    // A result whose node is not on the map.
    await openSearch(page, "不在地图");
    await page.keyboard.press("Enter");
    await expect(page.locator("#toast")).toContainText("不在当前地图中");
    // The search itself failing.
    await page.evaluate(() => { window.appHarness.searchBehavior.failNext = "数据库忙"; });
    await openSearch(page, "定理");
    await expect(page.locator("#workspace-search-status")).toHaveText("搜索失败：数据库忙");
    await expect(page.locator("#workspace-search-status")).toHaveClass(/is-error/);
    await expect(options(page)).toHaveCount(0);
  });

  test("trash is excluded by default and included with the toggle", async ({ page }) => {
    await page.evaluate(() => { const h = window.appHarness; Object.assign(h.notes.get(h.documents.a)!, { lifecycle: "trashed", lifecycle_revision: 1, trashed_at: Date.now() }); });
    await openSearch(page, "手稿 a");
    await expect(page.locator("#workspace-search-status")).toContainText("没有找到");
    await expect(page.locator("#workspace-search-status")).toContainText("包含回收站");
    await page.getByLabel("包含回收站").check();
    await expect(options(page)).toHaveCount(1);
    await expect(options(page).first()).toHaveClass(/is-trashed/);
    await expect(options(page).first()).toContainText("回收站");
    const calls = await page.evaluate(() => window.appHarness.searchBehavior.calls);
    expect(calls.at(-1)).toEqual({ text: "手稿 a", include_trashed: true });
  });

  test("a named snapshot is found by name and opens in the snapshot reader", async ({ page }) => {
    await page.locator("#ink-button").click();
    await page.getByRole("button", { name: /手稿 A/ }).click();
    await page.getByRole("button", { name: "保存快照", exact: true }).click();
    await page.locator("#ink-dialog .ink-snapshot-label-preset", { hasText: "整理稿" }).click();
    await page.locator("#ink-dialog [data-choice=ok]").click();
    await expect(page.locator("#ink-snapshot-list")).toContainText("整理稿");
    await openSearch(page, "整理");
    const hit = page.locator(".workspace-search-group[data-kind=snapshot] [role=option]");
    await expect(hit).toHaveCount(1);
    await expect(hit).toContainText("第 0 步");
    await page.keyboard.press("Enter");
    await expect(page.locator("#ink-workspace")).toBeHidden();
    await expect(page.locator("#reader .ink-reader-snapshot-label")).toHaveText("「整理稿」 · 历史快照 · 只读 · 第 0 步");
  });
});
