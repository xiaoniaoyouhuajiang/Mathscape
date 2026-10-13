import { expect, test, type Page } from "@playwright/test";

async function writeLine(page: Page, dy = 0): Promise<void> {
  const box = (await page.locator("#desktop-ink-board .ink-input").boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2 + dy;
  await page.mouse.move(x - 60,y); await page.mouse.down(); await page.mouse.move(x + 60,y,{steps:8}); await page.mouse.up();
}

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

test("snapshot selection registers evidence and reopens it from the map after later erasing", async ({ page }) => {
  await writeLine(page,-45); await writeLine(page,45);
  await expect(page.locator("#ink-save-badge")).toContainText("桌面已保存 · head 2");
  await page.getByRole("button",{name:"保存快照",exact:true}).click();
  await page.locator("#ink-snapshot-list .ink-session-item").click();
  await page.getByRole("button",{name:"选择本页全部笔迹"}).click();
  await expect(page.locator("#desktop-ink-evidence .ink-selection-summary")).toContainText("已选 2 笔");
  await page.evaluate(() => { window.inkDesktopHarness.behavior.loseEvidenceAck = true; });
  await page.getByRole("button",{name:"登记选区为来源证据"}).click();
  await expect(page.locator(".ink-evidence-message")).toContainText("响应丢失");
  await page.getByRole("button",{name:"登记选区为来源证据"}).click();
  await expect(page.locator(".ink-evidence-message")).toContainText("已登记");
  expect(await page.evaluate(() => window.inkDesktopHarness.evidenceRequests.map(request => request.request_id))).toEqual(expect.arrayContaining([expect.any(String)]));
  expect(await page.evaluate(() => new Set(window.inkDesktopHarness.evidenceRequests.map(request => request.request_id)).size)).toBe(1);
  await page.getByRole("button",{name:"继续编辑当前手稿"}).click();
  await page.getByRole("button",{name:"整笔擦除",exact:true}).click();
  const box = (await page.locator("#desktop-ink-board .ink-input").boundingBox())!;
  await page.mouse.click(box.x + box.width / 2,box.y + box.height / 2 - 45);
  await expect(page.locator("#ink-save-badge")).toContainText("head 3");
  await page.getByRole("button",{name:"返回地图"}).click();
  await page.getByRole("button",{name:"查看地图证据"}).click();
  await page.getByRole("button",{name:"打开并定位手稿证据"}).click();
  await expect(page.locator("#ink-reader-test")).toContainText("历史快照 · 只读 · 截止 head 2");
  await expect(page.locator("#ink-reader-test .ink-selection-summary")).toContainText("已选 2 笔");
  await expect(page.locator("#ink-reader-test .ink-input")).toBeVisible();
  await page.getByRole("button",{name:"继续编辑当前手稿"}).click();
  await expect(page.locator("#ink-save-badge")).toContainText("head 3");
  await expect(page.locator("#desktop-ink-summary")).toContainText("1 笔");
});

test("snapshot waits for operations added during an in-flight save", async ({ page }) => {
  await page.evaluate(() => {window.inkDesktopHarness.behavior.appendDelay = 180;});
  await writeLine(page,-25); await writeLine(page,25);
  await page.getByRole("button",{name:"保存快照",exact:true}).click();
  await expect(page.locator("#ink-snapshot-list")).toContainText("head 2");
  const saved = await page.evaluate(() => {
    const h = window.inkDesktopHarness;
    return { operations: h.storedDocuments.get(h.documents.a)!.operations.length, snapshot: JSON.parse(h.snapshots.get(h.documents.a)![0].manifest_json).document.operations.length };
  });
  expect(saved).toEqual({operations:2,snapshot:2});
});

test("a delayed snapshot open cannot override a newer document selection", async ({ page }) => {
  await page.getByRole("button",{name:"保存快照",exact:true}).click();
  await page.evaluate(() => {window.inkDesktopHarness.behavior.snapshotDelay = 250;});
  await page.locator("#ink-snapshot-list .ink-session-item").click();
  await page.getByRole("button",{name:/手稿 B/}).click();
  await expect(page.locator("#ink-document-heading")).toHaveText("手稿 B");
  await page.waitForTimeout(300);
  await expect(page.locator("#ink-document-heading")).toHaveText("手稿 B");
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
});
