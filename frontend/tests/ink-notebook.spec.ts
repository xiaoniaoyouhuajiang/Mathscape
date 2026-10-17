import { expect, test, type Page } from "@playwright/test";

const dialog = (page: Page) => page.locator("#ink-dialog");
const choose = async (page: Page, label: string | RegExp) => {
  await expect(dialog(page)).toBeVisible();
  await dialog(page).getByRole("button", { name: label }).click();
};
const rowAction = (page: Page, title: string) => page.locator(".ink-session-row", { hasText: title }).locator(".ink-session-action");

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/ink-desktop.html");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
});

test("N1 renaming the open note updates header and list; an empty title restores the auto title", async ({ page }) => {
  const title = page.locator("#desktop-ink-title");
  await expect(title).toHaveValue("手稿 A");
  await title.fill("  极限的定义  ");
  await title.press("Enter");
  await expect(page.locator("#ink-document-heading")).toHaveText("极限的定义");
  await expect(page.locator("#ink-session-list .ink-session-item.is-active")).toContainText("极限的定义");
  expect(await page.evaluate(() => { const h = window.inkDesktopHarness; return h.notes.get(h.documents.a); })).toMatchObject({ title_mode: "manual", title_revision: 1 });
  await title.fill("草稿");
  await title.press("Escape");
  await expect(title).toHaveValue("极限的定义");
  await title.fill("");
  await title.press("Enter");
  await expect(page.locator("#ink-document-heading")).toHaveText("手稿 · 自动");
  expect(await page.evaluate(() => { const h = window.inkDesktopHarness; return h.notes.get(h.documents.a)!.title_mode; })).toBe("auto");
});

test("N1 a concurrent rename asks which title to keep", async ({ page }) => {
  await page.evaluate(() => { window.inkDesktopHarness.behavior.renameConflictTitle = "平板起的名字"; });
  const title = page.locator("#desktop-ink-title");
  await title.fill("电脑起的名字");
  await title.press("Enter");
  await expect(page.locator("#ink-dialog-text")).toContainText("平板起的名字");
  await choose(page, "使用「电脑起的名字」");
  await expect(page.locator("#ink-document-heading")).toHaveText("电脑起的名字");
  const renames = await page.evaluate(() => window.inkDesktopHarness.noteCalls.filter(call => call.command === "rename_ink_note").map(call => call.request.expected_title_revision));
  expect(renames).toEqual([0, 1]);
});

test("N1 trashing asks first, offers undo and a read-only trash view with restore", async ({ page }) => {
  await rowAction(page, "手稿 B").click();
  await expect(page.locator("#ink-dialog-text")).toContainText("将「手稿 B」移到回收站？");
  await choose(page, "取消");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);

  await rowAction(page, "手稿 B").click();
  await choose(page, "移到回收站");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(1);
  await expect(page.locator("#ink-undo-bar")).toBeVisible();
  await page.locator("#ink-undo-trash").click();
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
  await expect(page.locator("#ink-document-heading")).toHaveText("手稿 B");

  await rowAction(page, "手稿 B").click();
  await choose(page, "移到回收站");
  await expect(page.locator("#ink-empty-state")).toBeVisible();
  // Trashing the open note never jumps to another note on its own.
  await page.waitForTimeout(200);
  await expect(page.locator("#ink-empty-state")).toBeVisible();
  await expect(page.locator("#ink-session-list .ink-session-item.is-active")).toHaveCount(0);
  await page.locator("#ink-tab-trash").click();
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(1);
  await page.locator("#ink-session-list .ink-session-item", { hasText: "手稿 B" }).click();
  await expect(page.locator("#ink-mode-badge")).toHaveText("回收站");
  await expect(page.locator("#ink-trash-banner")).toBeVisible();
  await expect(page.locator("#desktop-ink-pen")).toBeDisabled();
  await expect(page.locator("#desktop-ink-title")).toHaveJSProperty("readOnly", true);
  await page.locator("#ink-trash-restore").click();
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
  await expect(page.locator("#ink-tab-active")).toHaveAttribute("aria-selected", "true");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
});

test("N1 a tablet-held note warns that the tablet must be re-paired", async ({ page }) => {
  await page.evaluate(() => { const h = window.inkDesktopHarness; h.behavior.remoteDocumentId = h.documents.b; h.behavior.remotePending = 3; });
  await rowAction(page, "手稿 B").click();
  await expect(page.locator("#ink-dialog-text")).toContainText("平板上还有 3 个操作");
  await choose(page, "先去平板同步");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
  await page.evaluate(() => { window.inkDesktopHarness.behavior.remotePending = 0; });
  await rowAction(page, "手稿 B").click();
  await expect(page.locator("#ink-dialog-text")).toContainText("重新配对");
  await choose(page, "移到回收站");
  await page.locator("#ink-tab-trash").click();
  await page.locator(".ink-session-row", { hasText: "手稿 B" }).getByRole("button", { name: "恢复" }).click();
  await expect(page.locator("#desktop-ink-status")).toContainText("重新配对");
});

test("N1 the empty state never auto-creates, and clean blank skips the open and written notes", async ({ page }) => {
  for (const name of ["手稿 B", "手稿 A"]) { await rowAction(page, name).click(); await choose(page, "移到回收站"); }
  await expect(page.locator("#ink-empty-state")).toBeVisible();
  expect(await page.evaluate(() => window.inkDesktopHarness.noteCalls.filter(call => call.command === "create_ink_document").length)).toBe(0);

  await page.evaluate(() => { window.inkDesktopHarness.behavior.failCreate = true; });
  await page.locator("#ink-empty-create").click();
  await expect(page.locator("#desktop-ink-status")).toContainText("创建手稿失败");
  await page.locator("#ink-empty-create").click();
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
  const creates = await page.evaluate(() => window.inkDesktopHarness.noteCalls.filter(call => call.command === "create_ink_document").map(call => call.request));
  expect(creates).toHaveLength(2);
  expect(creates[0].request_id).toBe(creates[1].request_id);
  expect(creates[1].title_mode).toBe("auto");

  await page.getByRole("button", { name: "新建", exact: true }).click();
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
  await page.locator("#ink-clean-blank").click();
  await expect(page.locator("#ink-dialog-text")).toContainText("将 1 份空白稿");
  await choose(page, "移到回收站");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(1);
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
});

test("N1 snapshots of a trashed note stay readable but cannot register evidence", async ({ page }) => {
  await page.getByRole("button", { name: "保存快照", exact: true }).click();
  await expect(page.locator("#ink-snapshot-list .ink-session-item")).toHaveCount(1);
  await page.evaluate(() => { const h = window.inkDesktopHarness; Object.assign(h.notes.get(h.documents.a)!, { lifecycle: "trashed", lifecycle_revision: 1, trashed_at: Date.now() }); });
  await page.locator("#ink-snapshot-list .ink-session-item").click();
  await expect(page.locator("#ink-mode-badge")).toHaveText("历史快照");
  await expect(page.locator(".ink-evidence-source")).toContainText("回收站");
  await expect(page.getByRole("button", { name: "登记选区为来源证据" })).toBeDisabled();
});
