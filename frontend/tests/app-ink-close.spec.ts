import { expect, test, type Page } from "@playwright/test";
import { settledGeometry } from "./support/ink-geometry";

// H5 on the real app shell: closing the desktop window (Tauri close request)
// while 手稿 operations are still unsaved asks first, and reveals the 手稿
// workspace when the user was on the map.
const destroyed = (page: Page) => page.evaluate(() => window.appHarness.windowEvents.destroyed);
const requestClose = (page: Page) => page.evaluate(() => window.appHarness.requestWindowClose());
const dialog = (page: Page) => page.locator("#ink-dialog");
const quarantinedCodes = (page: Page) => page.evaluate(() => new Promise<string[]>((resolve, reject) => {
  const request = indexedDB.open("mathscape-ink-p1", 1);
  request.onerror = () => reject(request.error);
  request.onsuccess = () => {
    const database = request.result;
    const transaction = database.transaction("operations", "readonly");
    const items = transaction.objectStore("operations").getAll();
    transaction.oncomplete = () => { database.close(); resolve(items.result.filter(item => item.quarantined).map(item => item.quarantined.code)); };
    transaction.onabort = () => { database.close(); reject(transaction.error); };
  };
}));

async function openNoteA(page: Page): Promise<void> {
  await page.locator("#ink-button").click();
  await page.getByRole("button", { name: /手稿 A/ }).click();
  await expect(page.locator("#ink-document-heading")).toContainText("手稿 A");
  await settledGeometry(page);
}

async function writeStroke(page: Page): Promise<void> {
  const box = (await page.locator("#desktop-ink-board .ink-input").boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2;
  await page.mouse.move(x - 60, y); await page.mouse.down();
  await page.mouse.move(x + 60, y, { steps: 8 }); await page.mouse.up();
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/app.html");
  await expect.poll(() => page.evaluate(() => window.appHarness?.labels()["r-unit-thm"])).toBe("定理 3.2");
});

test("H5 closing the window with nothing pending closes at once", async ({ page }) => {
  expect(await requestClose(page)).toBe(1);
  await expect.poll(() => destroyed(page)).toBe(1);
  await expect(dialog(page)).toBeHidden();
});

test("H5 closing the window with unsaved 手稿 operations asks, from the board and from the map", async ({ page }) => {
  await openNoteA(page);
  await page.evaluate(() => { window.appHarness.behavior.failAppend = true; });
  await writeStroke(page);

  // On the board: the prompt appears and 继续编辑 keeps the window.
  await requestClose(page);
  await expect(dialog(page)).toBeVisible();
  await expect(page.locator("#ink-dialog-text")).toContainText("1 个手稿操作尚未保存");
  await dialog(page).getByRole("button", { name: "继续编辑", exact: true }).click();
  await page.waitForTimeout(150);
  expect(await destroyed(page)).toBe(0);

  // Leave to the map anyway (kept in the recovery queue)…
  await page.locator("#ink-close").click();
  await dialog(page).getByRole("button", { name: "仍然离开", exact: true }).click();
  await expect(page.locator("#ink-session-list")).toBeHidden();

  // …then close the window from the map: the 手稿 workspace comes back to ask.
  await requestClose(page);
  await expect(dialog(page)).toBeVisible();
  await expect(page.locator("#ink-session-list")).toBeVisible();
  expect(await destroyed(page)).toBe(0);
  await dialog(page).getByRole("button", { name: "仍然关闭窗口", exact: true }).click();
  await expect.poll(() => destroyed(page)).toBe(1);
});

test("H5 a retried save that succeeds lets the window close", async ({ page }) => {
  await openNoteA(page);
  await page.evaluate(() => { window.appHarness.behavior.failAppend = true; });
  await writeStroke(page);
  await requestClose(page);
  await expect(dialog(page)).toBeVisible();
  await page.evaluate(() => { window.appHarness.behavior.failAppend = false; });
  await dialog(page).getByRole("button", { name: "重试保存", exact: true }).click();
  await expect.poll(() => destroyed(page)).toBe(1);
});

test("H5 quarantined operations with a persistent outbox do not block closing (kept for recovery)", async ({ page }) => {
  await openNoteA(page);
  await page.evaluate(() => { window.appHarness.behavior.appendErrorCode = "writer_epoch_conflict"; });
  await writeStroke(page);
  await expect.poll(() => quarantinedCodes(page)).toEqual(["writer_epoch_conflict"]);
  await requestClose(page);
  await expect.poll(() => destroyed(page)).toBe(1);
  await expect(dialog(page)).toBeHidden();
});

test.describe("without a persistent outbox", () => {
  test.beforeEach(async ({ page }) => {
    // IndexedDB unavailable → in-memory outbox (`persistent === false`).
    await page.addInitScript(() => { Object.defineProperty(window, "indexedDB", { value: undefined, configurable: true }); });
    await page.goto("/tests/app.html");
    await expect.poll(() => page.evaluate(() => window.appHarness?.labels()["r-unit-thm"])).toBe("定理 3.2");
  });

  test("H5 quarantined operations warn before closing and offer no retry", async ({ page }) => {
    await openNoteA(page);
    await page.evaluate(() => { window.appHarness.behavior.appendErrorCode = "writer_epoch_conflict"; });
    await writeStroke(page);
    await requestClose(page);
    await expect(dialog(page)).toBeVisible();
    await expect(page.locator("#ink-dialog-text")).toContainText("1 个手稿操作已被工作区拒绝");
    await expect(page.locator("#ink-dialog-text")).toContainText("现在离开会丢失这些操作");
    await expect(dialog(page).getByRole("button", { name: "重试保存", exact: true })).toHaveCount(0);
    await dialog(page).getByRole("button", { name: "继续编辑", exact: true }).click();
    await page.waitForTimeout(150);
    expect(await destroyed(page)).toBe(0);
    await requestClose(page);
    await dialog(page).getByRole("button", { name: "仍然关闭窗口", exact: true }).click();
    await expect.poll(() => destroyed(page)).toBe(1);
  });
});
