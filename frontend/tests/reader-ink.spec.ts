import { expect, test, type Page } from "@playwright/test";

// N2 reader integration (proposal §6.4, §6.5): block-level targets only.
const link = (documentId: string, title: string, blocks: string[], primary = true) => ({
  note: { document_id: documentId, title, title_mode: "auto", title_revision: 0, lifecycle: "active", lifecycle_revision: 0, trashed_at: null, last_used_at: 1, created_at: "unix:1" },
  link: {
    link_id: `link-${documentId}-${blocks.join("-")}`, document_id: documentId, target_kind: "markdown", target_key: "md:x", target_id: "md",
    target: { kind: "markdown", artifact_id: "md", content_hash: "test-hash", first_block: blocks[0], last_block: blocks.at(-1), block_ids: blocks, start: 0, end: 1, block_hash: "h", excerpt: "x", label: "x" },
    target_revision: "test-hash", target_label: "x", role: "attempt", is_primary: primary, state: "active", created_at: 1, removed_at: null, target_status: "active",
  },
});

async function openMarkdown(page: Page, setup?: (ink: any) => void): Promise<void> {
  await page.goto("/tests/reader.html");
  await page.waitForFunction(() => "harness" in window);
  if (setup) await page.evaluate(`(${setup.toString()})(window.harness.ink)`);
  await page.evaluate(async () => (window as any).harness.reader.open("md", "数学笔记"));
  await expect(page.getByLabel("原文块 2", { exact: true })).toBeVisible();
}

test("N2 block markers count notes per block and open the related list", async ({ page }) => {
  await page.goto("/tests/reader.html");
  await page.waitForFunction(() => "harness" in window);
  await page.evaluate(async ([a, b, c]) => {
    (window as any).harness.ink.links = [a, b, c];
    await (window as any).harness.reader.open("md", "数学笔记");
  }, [link("note-1", "尝试 · Inline", ["test-block-1", "test-block-2"]), link("note-2", "验算 · Inline", ["test-block-1"]), link("note-1", "尝试 · Inline", ["test-block-1"], false)]);
  const block2 = page.getByLabel("原文块 2", { exact: true });
  await expect(block2.locator(".ink-block-marker")).toHaveText("✎ 2");
  await expect(page.getByLabel("原文块 3", { exact: true }).locator(".ink-block-marker")).toHaveText("✎ 1");
  await expect(page.getByLabel("原文块 1", { exact: true }).locator(".ink-block-marker")).toHaveCount(0);
  await block2.locator(".ink-block-marker").click();
  const related = page.locator(".ink-related-popover");
  await expect(related.getByRole("listitem")).toHaveCount(2);
  await related.getByRole("button", { name: "验算 · Inline" }).click();
  expect(await page.evaluate(() => (window as any).harness.ink.opened)).toEqual(["note-2"]);
});

test("N2 right-click menu targets the block under the cursor, or the selection when inside it", async ({ page }) => {
  await openMarkdown(page);
  await page.getByLabel("原文块 3", { exact: true }).click({ button: "right" });
  const menu = page.locator(".graph-context-menu");
  await expect(menu.getByRole("menuitem").or(menu.locator("button"))).toHaveText(["新建关联手稿…", "关联到已有手稿…", "查看相关手稿（0）"]);
  await expect(page.getByLabel("原文块 3", { exact: true })).toHaveClass(/is-ink-menu-target/);
  await menu.locator("button", { hasText: "新建关联手稿…" }).click();
  const popover = page.locator(".ink-create-popover");
  await expect(popover).toContainText("关联到");
  await popover.getByRole("button", { name: "在电脑上书写" }).click();
  await expect(popover).toHaveCount(0);
  let creates = await page.evaluate(() => (window as any).harness.ink.creates);
  expect(creates[0].target).toEqual({ kind: "markdown", artifact_id: "md", content_hash: "test-hash", first_block: "test-block-2", last_block: "test-block-2" });
  expect(creates[0].choice).toMatchObject({ role: "attempt", mode: "desktop" });
  await expect(page.locator(".is-ink-menu-target")).toHaveCount(0);

  // A selection of blocks 2..4; right-clicking inside uses the whole selection.
  await page.getByLabel("原文块 2", { exact: true }).click();
  await page.getByRole("button", { name: "设置起点" }).click();
  await page.getByLabel("原文块 4", { exact: true }).click();
  await page.getByRole("button", { name: "设置终点" }).click();
  await page.getByLabel("原文块 3", { exact: true }).click({ button: "right" });
  await expect(page.locator(".is-ink-menu-target")).toHaveCount(3);
  await page.locator(".graph-context-menu button", { hasText: "新建关联手稿…" }).click();
  await page.locator(".ink-create-popover").getByRole("combobox", { name: "用途" }).selectOption({ label: "验算" });
  await page.locator(".ink-create-popover").getByRole("button", { name: "用平板书写" }).click();
  creates = await page.evaluate(() => (window as any).harness.ink.creates);
  expect(creates[1].target).toMatchObject({ first_block: "test-block-1", last_block: "test-block-3" });
  expect(creates[1].choice).toMatchObject({ role: "check", mode: "tablet" });

  // Outside the selection: the block under the cursor.
  await page.getByLabel("原文块 6", { exact: true }).click({ button: "right" });
  await expect(page.locator(".is-ink-menu-target")).toHaveCount(1);
  await expect(page.getByLabel("原文块 6", { exact: true })).toHaveClass(/is-ink-menu-target/);
  await page.keyboard.press("Escape");
});

test("N2 手写这段 uses the selection panel range", async ({ page }) => {
  await openMarkdown(page);
  await page.getByLabel("原文块 2", { exact: true }).click();
  await page.getByRole("button", { name: "设置起点" }).click();
  await page.getByLabel("原文块 3", { exact: true }).click();
  await page.getByRole("button", { name: "设置终点" }).click();
  await page.getByRole("button", { name: "手写这段" }).click();
  await page.locator(".ink-create-popover").getByRole("button", { name: "在电脑上书写" }).click();
  const creates = await page.evaluate(() => (window as any).harness.ink.creates);
  expect(creates[0].target).toMatchObject({ first_block: "test-block-1", last_block: "test-block-2" });
});

test("N2 no ink menu on page-boundary blocks or in raw mode", async ({ page }) => {
  await openMarkdown(page, ink => { ink.pageMarkerBlock = 4; });
  const marker = page.locator(".source-page-marker");
  await expect(marker).toHaveCount(1);
  await marker.click({ button: "right" });
  await expect(page.locator(".graph-context-menu")).toBeHidden();
  await page.getByRole("button", { name: "查看原文" }).click();
  await page.locator(".reader-content pre, article pre").first().click({ button: "right" });
  await expect(page.locator(".graph-context-menu")).toBeHidden();
});

test("N2 link to an existing note lists active notes by last use and sends one request", async ({ page }) => {
  await openMarkdown(page);
  await page.getByLabel("原文块 2", { exact: true }).click({ button: "right" });
  await page.locator(".graph-context-menu button", { hasText: "关联到已有手稿…" }).click();
  const popover = page.locator(".ink-existing-popover");
  await expect(popover.getByRole("listbox", { name: "已有手稿" }).getByRole("option")).toHaveText(["旧手稿"]);
  await popover.getByRole("option", { name: "旧手稿" }).click();
  await expect(popover.getByRole("checkbox", { name: "设为主关联" })).toBeChecked();
  await popover.getByRole("button", { name: "关联", exact: true }).click();
  await expect(popover).toContainText("已关联到「旧手稿」");
  const requests = await page.evaluate(() => (window as any).harness.ink.linkRequests);
  expect(requests).toHaveLength(1);
  expect(requests[0]).toMatchObject({ document_id: "note-old", role: "attempt", primary: true, target: { kind: "markdown", first_block: "test-block-1" } });
});

test("N2 a patch-derived document shows only the original-version hint", async ({ page }) => {
  await openMarkdown(page, ink => { ink.originalNoteCount = 2; });
  await expect(page.locator(".ink-patch-hint")).toHaveText("原版本有 2 份相关手稿");
  await expect(page.locator(".ink-block-marker")).toHaveCount(0);
});
