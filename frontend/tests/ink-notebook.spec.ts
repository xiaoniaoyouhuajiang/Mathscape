import { expect, test, type Page } from "@playwright/test";
import type { InkOutboxItem } from "../src/ink/outbox";

const dialog = (page: Page) => page.locator("#ink-dialog");
const choose = async (page: Page, label: string | RegExp) => {
  await expect(dialog(page)).toBeVisible();
  await dialog(page).getByRole("button", { name: label }).click();
};
const rowAction = (page: Page, title: string) => page.locator(".ink-session-row", { hasText: title }).locator(".ink-session-action");

async function writeLine(page: Page): Promise<void> {
  const canvas = page.locator("#desktop-ink-board .ink-input");
  await expect(canvas).toBeVisible();
  const box = (await canvas.boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2;
  await page.mouse.move(x - 60, y);
  await page.mouse.down();
  await page.mouse.move(x + 60, y, { steps: 8 });
  await page.mouse.up();
}

async function localQueue(page: Page): Promise<InkOutboxItem[]> {
  return page.evaluate(() => new Promise<InkOutboxItem[]>((resolve, reject) => {
    const request = indexedDB.open("mathscape-ink-p1", 1);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const database = request.result;
      const transaction = database.transaction("operations", "readonly");
      const items = transaction.objectStore("operations").getAll();
      transaction.oncomplete = () => { database.close(); resolve(items.result); };
      transaction.onabort = () => { database.close(); reject(transaction.error); };
    };
  }));
}

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

test("N1 failed local sync blocks trash until the writing is durably saved", async ({ page }) => {
  await page.evaluate(() => { window.inkDesktopHarness.behavior.failAppend = true; });
  await writeLine(page);
  await expect(page.locator("#ink-save-badge")).toContainText("保存失败");
  await rowAction(page, "手稿 A").click();
  await expect(page.locator("#ink-dialog-text")).toContainText("暂时不能移到回收站");
  await expect(dialog(page).getByRole("button", { name: /仍移到回收站/ })).toHaveCount(0);
  await choose(page, "取消");
  expect((await localQueue(page))).toHaveLength(1);
  expect(await page.evaluate(() => window.inkDesktopHarness.noteCalls.filter(call => call.command === "trash_ink_note"))).toHaveLength(0);
  await expect(page.locator("#desktop-ink-summary")).toContainText("1 笔");

  await rowAction(page, "手稿 A").click();
  await expect(dialog(page).getByRole("button", { name: "重试保存" })).toBeVisible();
  await page.evaluate(() => { window.inkDesktopHarness.behavior.failAppend = false; });
  await choose(page, "重试保存");
  await expect(page.locator("#ink-dialog-text")).toContainText("1 个已保存操作");
  expect(await localQueue(page)).toHaveLength(0);
  await choose(page, "移到回收站");
  await expect(page.locator("#ink-empty-state")).toBeVisible();
  expect(await page.evaluate(() => {
    const h = window.inkDesktopHarness;
    return { lifecycle: h.notes.get(h.documents.a)!.lifecycle, operations: h.storedDocuments.get(h.documents.a)!.operations.length };
  })).toEqual({ lifecycle: "trashed", operations: 1 });
});

test("N1 quarantined local writing blocks trash and blank cleanup", async ({ page }) => {
  await page.evaluate(() => {
    const h = window.inkDesktopHarness;
    h.behavior.appendErrorCode = "writer_epoch_conflict";
    h.notes.get(h.documents.a)!.title_mode = "auto";
  });
  await writeLine(page);
  await expect.poll(async () => (await localQueue(page))[0]?.quarantined?.code).toBe("writer_epoch_conflict");
  await rowAction(page, "手稿 A").click();
  await expect(page.locator("#ink-dialog-text")).toContainText("暂时不能移到回收站");
  await choose(page, "取消");

  await page.locator("#ink-session-list .ink-session-item", { hasText: "手稿 B" }).click();
  await expect(page.locator("#ink-document-heading")).toHaveText("手稿 B");
  await page.locator("#ink-clean-blank").click();
  await expect(page.locator("#desktop-ink-status")).toContainText("没有可清理的空白稿");
  expect(await localQueue(page)).toHaveLength(1);
  expect(await page.evaluate(() => {
    const h = window.inkDesktopHarness;
    return { lifecycle: h.notes.get(h.documents.a)!.lifecycle, calls: h.noteCalls.filter(call => call.command === "trash_ink_note" || call.command === "clean_blank_ink_notes").length };
  })).toEqual({ lifecycle: "active", calls: 0 });
});

for (const connected of [true, false]) {
  test(`N1 one click switches notes across ${connected ? "connected" : "offline"} tablet mirror refreshes`, async ({ page }) => {
    await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
    await page.evaluate(connected => {
      const h = window.inkDesktopHarness;
      Object.assign(h.behavior, { remoteDocumentId: h.documents.a, remoteConnected: connected });
    }, connected);
    await expect(page.locator("#ink-mode-badge")).toHaveText("平板镜像");
    const button = page.locator("#ink-session-list .ink-session-item", { hasText: "手稿 B" });
    const pressed = (await button.elementHandle())!;
    const box = (await button.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    try {
      const before = await page.evaluate(() => window.inkDesktopHarness.behavior.mirrorPolls);
      // Keep the pointer down until polling has crossed two refreshes.
      await expect.poll(() => page.evaluate(() => window.inkDesktopHarness.behavior.mirrorPolls)).toBeGreaterThanOrEqual(before + 2);
      expect(await pressed.evaluate(button => button.isConnected)).toBe(true);
    } finally {
      await page.mouse.up();
    }
    await expect(page.locator("#ink-document-heading")).toHaveText("手稿 B");
    await expect(page.locator("#ink-session-list .ink-session-item.is-active")).toContainText("手稿 B");
  });
}
