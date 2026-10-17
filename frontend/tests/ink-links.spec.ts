import { expect, test, type Page } from "@playwright/test";
import type { InkOutboxItem } from "../src/ink/outbox";

// N2: note-page link panel and the A1 discard flow (proposal §6.2, §7.2, §14).
const dialog = (page: Page) => page.locator("#ink-dialog");
const choose = async (page: Page, label: string | RegExp) => {
  await expect(dialog(page)).toBeVisible();
  await dialog(page).getByRole("button", { name: label, exact: typeof label === "string" }).click();
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

async function addLink(page: Page, query: string, role?: string): Promise<void> {
  await page.locator("#ink-add-link").click();
  const popover = page.locator(".ink-add-link-popover");
  await popover.getByRole("searchbox", { name: "搜索关联目标" }).fill(query);
  await popover.getByRole("listbox", { name: "关联目标" }).getByRole("option").first().click();
  if (role) await popover.getByRole("combobox", { name: "用途" }).selectOption({ label: role });
  await popover.getByRole("button", { name: "关联", exact: true }).click();
  await expect(popover).toHaveCount(0);
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/ink-desktop.html");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
  await expect(page.locator("#ink-link-bar")).toBeVisible();
});

test("N2 terminology: the header says 手写 and the empty heading says 未打开手稿", async ({ page }) => {
  await expect(page.locator(".ink-document-heading .eyebrow")).toHaveText("手写");
  await expect(page.locator("#desktop-ink-board")).toHaveAttribute("aria-label", "手稿画板");
  await rowAction(page, "手稿 A").click();
  await choose(page, "移到回收站");
  await expect(page.locator("#ink-document-heading")).toHaveText("未打开手稿");
  await expect(page.locator("#ink-link-bar")).toBeHidden();
});

test("N2 note-page links: add with default primary, auto title, role change, set primary and unlink", async ({ page }) => {
  await page.evaluate(() => { const h = window.inkDesktopHarness; h.notes.get(h.documents.a)!.title_mode = "auto"; });
  const bar = page.locator("#ink-link-bar");
  await expect(bar).toContainText("还没有关联");

  // No primary yet, so "设为主关联" is checked by default (D12).
  await page.locator("#ink-add-link").click();
  await expect(page.locator(".ink-add-link-popover").getByRole("checkbox", { name: "设为主关联" })).toBeChecked();
  await page.keyboard.press("Escape");
  await addLink(page, "定理");
  const theorem = bar.locator(".ink-link-chip", { hasText: "定理 3.2" });
  await expect(theorem).toHaveClass(/is-primary/);
  await expect(page.locator("#ink-document-heading")).toHaveText("尝试 · 定理 3.2");

  // With a primary, the next link is not primary by default; other notes are candidates.
  await page.locator("#ink-add-link").click();
  await expect(page.locator(".ink-add-link-popover").getByRole("checkbox", { name: "设为主关联" })).not.toBeChecked();
  await expect(page.locator(".ink-add-link-popover").getByRole("listbox", { name: "关联目标" }).getByRole("option", { name: /手稿 B/ })).toBeVisible();
  await page.keyboard.press("Escape");
  await addLink(page, "例 4", "验算");
  const example = bar.locator(".ink-link-chip", { hasText: "例 4" });
  await expect(example).not.toHaveClass(/is-primary/);
  await expect(page.locator("#ink-document-heading")).toHaveText("尝试 · 定理 3.2");

  // Role change is one atomic replace that keeps the primary flag.
  await theorem.getByRole("combobox", { name: "用途" }).selectOption({ label: "另证" });
  await expect(page.locator("#ink-document-heading")).toHaveText("另证 · 定理 3.2");
  const replace = await page.evaluate(() => window.inkDesktopHarness.linkCalls.at(-1)!.request);
  expect(replace.replaces_link_id).toMatch(/^link-/);
  await expect(bar.locator(".ink-link-chip.is-primary")).toContainText("定理 3.2");

  await example.getByRole("button", { name: "设为主关联" }).click();
  await expect(page.locator("#ink-document-heading")).toHaveText("验算 · 例 4");
  await bar.locator(".ink-link-chip", { hasText: "例 4" }).getByRole("button", { name: "解除" }).click();
  await expect(bar.locator(".ink-link-chip")).toHaveCount(1);
  await expect(page.locator("#ink-document-heading")).toHaveText("手稿 · 自动");
  expect(await page.evaluate(() => window.inkDesktopHarness.links.filter(link => link.state === "removed").length)).toBe(2);
});

test("N2 a duplicate link returns the existing one and a manual title never changes", async ({ page }) => {
  await addLink(page, "定理");
  await expect(page.locator("#ink-document-heading")).toHaveText("手稿 A");
  await addLink(page, "定理");
  await expect(page.locator("#desktop-ink-status")).toContainText("这条关联已存在");
  await expect(page.locator("#ink-link-bar .ink-link-chip")).toHaveCount(1);
});

test("N2 a trashed note shows its links read-only", async ({ page }) => {
  await addLink(page, "定理");
  await rowAction(page, "手稿 A").click();
  await choose(page, "移到回收站");
  await page.locator("#ink-tab-trash").click();
  await page.locator("#ink-session-list .ink-session-item", { hasText: "手稿 A" }).click();
  await expect(page.locator("#ink-trash-banner")).toBeVisible();
  const bar = page.locator("#ink-link-bar");
  await expect(bar).toContainText("定理 3.2");
  await expect(bar.getByRole("button", { name: "解除" })).toHaveCount(0);
  await expect(page.locator("#ink-add-link")).toHaveCount(0);
});

test("N2 A1 quarantined writing can be exported, then discarded and trashed after a second confirmation", async ({ page }) => {
  await page.evaluate(() => { window.inkDesktopHarness.behavior.appendErrorCode = "writer_epoch_conflict"; });
  await writeLine(page);
  await expect.poll(async () => (await localQueue(page))[0]?.quarantined?.code).toBe("writer_epoch_conflict");

  await rowAction(page, "手稿 A").click();
  await expect(page.locator("#ink-dialog-text")).toContainText("暂时不能移到回收站");
  await choose(page, "放弃这些未保存的笔迹并移入回收站");
  await expect(page.locator("#ink-dialog-text")).toContainText("确定放弃 1 个未保存的操作");
  // Cancel at the second step keeps everything.
  await choose(page, "取消");
  expect(await localQueue(page)).toHaveLength(1);

  await rowAction(page, "手稿 A").click();
  await choose(page, "放弃这些未保存的笔迹并移入回收站");
  await choose(page, "导出到文件…");
  await expect(page.locator("#desktop-ink-status")).toContainText("未保存的笔迹已导出");
  const exported = await page.evaluate(() => window.inkDesktopHarness.exports);
  expect(exported).toHaveLength(1);
  const recovery = JSON.parse(exported[0].contents);
  expect(recovery.operations ?? recovery.items).toHaveLength(1);
  // Export returns to the second confirmation.
  await choose(page, "放弃并移入回收站");
  await expect(page.locator("#ink-dialog-text")).toContainText("移到回收站？");
  await choose(page, "移到回收站");
  await expect(page.locator("#ink-empty-state")).toBeVisible();
  expect(await localQueue(page)).toHaveLength(0);
  expect(await page.evaluate(() => { const h = window.inkDesktopHarness; return h.notes.get(h.documents.a)!.lifecycle; })).toBe("trashed");
});

test("N2 A1 failed (retryable) writing can also be discarded; clean-blank still skips it", async ({ page }) => {
  await page.evaluate(() => { const h = window.inkDesktopHarness; h.behavior.failAppend = true; h.notes.get(h.documents.a)!.title_mode = "auto"; });
  await writeLine(page);
  await expect(page.locator("#ink-save-badge")).toContainText("保存失败");
  await page.locator("#ink-session-list .ink-session-item", { hasText: "手稿 B" }).click();
  await expect(page.locator("#ink-document-heading")).toHaveText("手稿 B");
  await page.locator("#ink-clean-blank").click();
  await expect(page.locator("#desktop-ink-status")).toContainText("没有可清理的空白稿");

  await rowAction(page, "手稿 A").click();
  await choose(page, "放弃这些未保存的笔迹并移入回收站");
  await choose(page, "放弃并移入回收站");
  await choose(page, "移到回收站");
  await expect.poll(() => page.evaluate(() => { const h = window.inkDesktopHarness; return h.notes.get(h.documents.a)!.lifecycle; })).toBe("trashed");
  expect(await localQueue(page)).toHaveLength(0);
  expect(await page.evaluate(() => { const h = window.inkDesktopHarness; return h.storedDocuments.get(h.documents.a)!.operations.length; })).toBe(0);
});
