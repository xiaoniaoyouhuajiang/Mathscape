import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/ink-desktop.html");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
});

test("saved snapshots follow their document across session switches", async ({ page }) => {
  await page.getByRole("button", { name: "保存快照" }).click();
  await expect(page.locator("#ink-snapshot-list .ink-session-item")).toHaveCount(1);
  await expect(page.locator("#ink-snapshot-list")).not.toContainText("Invalid Date");

  await page.getByRole("button", { name: /手稿 B/ }).click();
  await expect(page.locator("#ink-session-list .ink-session-item.is-active")).toContainText("手稿 B");
  await expect(page.locator("#ink-snapshot-list")).toContainText("还没有历史快照");

  await page.getByRole("button", { name: /手稿 A/ }).click();
  await expect(page.locator("#ink-session-list .ink-session-item.is-active")).toContainText("手稿 A");
  await expect(page.locator("#ink-snapshot-list .ink-session-item")).toHaveCount(1);
  await expect(page.locator("#ink-snapshot-list")).toContainText("head 0");
});

test("an older snapshot query cannot overwrite the current document list", async ({ page }) => {
  await page.getByRole("button", { name: "保存快照" }).click();
  await page.evaluate(() => {
    const harness = (window as unknown as { inkDesktopHarness: { documents: { a: string }; behavior: { delayedDocumentId: string; delayMs: number } } }).inkDesktopHarness;
    harness.behavior.delayedDocumentId = harness.documents.a;
    harness.behavior.delayMs = 250;
  });
  await page.getByRole("button", { name: /手稿 A/ }).click();
  await page.getByRole("button", { name: /手稿 B/ }).click();
  await expect(page.locator("#ink-session-list .ink-session-item.is-active")).toContainText("手稿 B");
  await expect(page.locator("#ink-snapshot-list")).toContainText("还没有历史快照");
  await page.waitForTimeout(300);
  await expect(page.locator("#ink-snapshot-list")).toContainText("还没有历史快照");
});

test("the reader makes snapshot mode and persistence state explicit", async ({ page }) => {
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
  await expect(page.locator("#ink-editability-badge")).toHaveText("可编辑");
  await page.getByRole("button", { name: "保存快照" }).click();
  const snapshot = page.locator("#ink-snapshot-list .ink-session-item");
  await expect(snapshot).toHaveCount(1);
  await snapshot.click();
  await expect(page.locator("#ink-mode-badge")).toHaveText("历史快照");
  await expect(page.locator("#ink-editability-badge")).toHaveText("只读");
  await expect(page.getByRole("button", { name: "继续编辑当前手稿" })).toBeVisible();
  await expect(page.locator("#desktop-ink-pen")).toBeDisabled();
  await expect(page.locator("#desktop-ink-export-json")).toBeEnabled();
  await page.getByRole("button", { name: "继续编辑当前手稿" }).click();
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
  await expect(page.locator("#ink-editability-badge")).toHaveText("可编辑");
});
