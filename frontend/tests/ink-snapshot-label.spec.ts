import { expect, test, type Page } from "@playwright/test";

type Harness = {
  snapshots: Map<string, Array<{ snapshot_id: string; label?: string | null; label_revision?: number; content_hash: string; manifest_json: string; byte_size: number }>>;
  documents: { a: string; b: string };
  behavior: { snapshotLabelConflict: string | null; snapshotLabelDelay: number };
  notes: Map<string, { lifecycle: string }>;
};
const harness = (page: Page) => page.evaluate(() => {
  const h = (window as unknown as { inkDesktopHarness: Harness }).inkDesktopHarness;
  return [...h.snapshots.get(h.documents.a)!].map(s => ({ id: s.snapshot_id, label: s.label ?? null, revision: s.label_revision ?? 0, hash: s.content_hash, manifest: s.manifest_json, size: s.byte_size }));
});

async function writeLine(page: Page, dy = 0): Promise<void> {
  const box = (await page.locator("#desktop-ink-board .ink-input").boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2 + dy;
  await page.mouse.move(x - 60, y); await page.mouse.down(); await page.mouse.move(x + 60, y, { steps: 8 }); await page.mouse.up();
}

const dialog = (page: Page) => page.locator("#ink-dialog");
const rows = (page: Page) => page.locator("#ink-snapshot-list .ink-snapshot-row");

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/ink-desktop.html");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
});

test("saving offers presets; the list shows name, step and time", async ({ page }) => {
  await page.getByRole("button", { name: "保存快照", exact: true }).click();
  await expect(dialog(page)).toBeVisible();
  await expect(page.locator("#ink-dialog-text")).toContainText("第 0 步");
  await expect(page.locator("#ink-dialog-text")).toContainText("也可以留空");
  await expect(page.getByLabel("快照名称")).toBeFocused();
  const presets = dialog(page).locator(".ink-snapshot-label-preset");
  await expect(presets).toHaveText(["第一次尝试", "整理稿", "卡住了", "换个思路", "最终版"]);
  await presets.filter({ hasText: "卡住了" }).click();
  await expect(page.getByLabel("快照名称")).toHaveValue("卡住了");
  await page.getByLabel("快照名称").press("Enter");
  await expect(dialog(page)).toBeHidden();
  await expect(rows(page)).toHaveCount(1);
  const main = rows(page).first().locator(".ink-session-item");
  await expect(main.locator("strong")).toHaveText("卡住了");
  await expect(main.locator("span")).toContainText("第 0 步 · ");
  await expect(main).not.toContainText("Invalid Date");
  await expect(page.locator("#desktop-ink-status")).toContainText("「卡住了」");
  const [saved] = await harness(page);
  expect(saved).toMatchObject({ label: "卡住了", revision: 1 });
});

test("an unnamed save stays unnamed; the same head explains it will name the existing snapshot", async ({ page }) => {
  await page.getByRole("button", { name: "保存快照", exact: true }).click();
  await dialog(page).locator("[data-choice=ok]").click();
  await expect(rows(page)).toHaveCount(1);
  await expect(rows(page).first().locator(".ink-session-item strong")).toHaveText("第 0 步");
  const before = await harness(page);
  expect(before[0].label).toBeNull();

  await page.getByRole("button", { name: "保存快照", exact: true }).click();
  await expect(page.locator("#ink-dialog-text")).toContainText("再次保存不会新建快照");
  await expect(page.locator("#ink-dialog-text")).toContainText("填写名称会给这个快照命名");
  await page.getByLabel("快照名称").fill("  整理稿  ");
  await dialog(page).locator("[data-choice=ok]").click();
  await expect(rows(page).first().locator(".ink-session-item strong")).toHaveText("整理稿");
  const after = await harness(page);
  expect(after).toHaveLength(1);
  // Naming never touches the snapshot content.
  expect(after[0]).toMatchObject({ id: before[0].id, hash: before[0].hash, manifest: before[0].manifest, size: before[0].size, label: "整理稿" });

  await page.getByRole("button", { name: "保存快照", exact: true }).click();
  await expect(page.locator("#ink-dialog-text")).toContainText("已保存为快照「整理稿」");
  await expect(page.locator("#ink-dialog-text")).toContainText("留空则保留原名");
  await expect(page.getByLabel("快照名称")).toHaveValue("整理稿");
  await dialog(page).getByRole("button", { name: "取消" }).click();
  await expect(rows(page)).toHaveCount(1);

  // After writing, the head moves on and a new snapshot is created.
  await writeLine(page);
  await page.getByRole("button", { name: "保存快照", exact: true }).click();
  await expect(page.locator("#ink-dialog-text")).toContainText("第 1 步");
  await expect(page.locator("#ink-dialog-text")).not.toContainText("再次保存");
  await dialog(page).locator("[data-choice=ok]").click();
  await expect(rows(page)).toHaveCount(2);
});

test("rename from the list validates length, clears, and resolves conflicts like note titles", async ({ page }) => {
  await page.getByRole("button", { name: "保存快照", exact: true }).click();
  await page.getByLabel("快照名称").fill("第一次尝试");
  await dialog(page).locator("[data-choice=ok]").click();
  await expect(rows(page).first().locator("strong")).toHaveText("第一次尝试");

  const rename = rows(page).first().getByRole("button", { name: "命名" });
  await rename.click();
  await expect(page.locator("#ink-dialog-text")).toContainText("不改变快照内容");
  await page.getByLabel("快照名称").fill("名".repeat(41));
  await dialog(page).locator("[data-choice=ok]").click();
  await expect(dialog(page).getByRole("alert")).toContainText("40");
  await expect(dialog(page)).toBeVisible();
  await page.getByLabel("快照名称").fill("𝔽".repeat(40)); // 40 chars, 80 UTF-16 units
  await expect(dialog(page).getByRole("alert")).toBeHidden();
  await dialog(page).locator("[data-choice=ok]").click();
  await expect(rows(page).first().locator("strong")).toHaveText("𝔽".repeat(40));

  // Another window renamed it meanwhile: keep mine.
  await page.evaluate(() => { (window as unknown as { inkDesktopHarness: Harness }).inkDesktopHarness.behavior.snapshotLabelConflict = "别处的名字"; });
  await rename.click();
  await page.getByLabel("快照名称").fill("我的名字");
  await dialog(page).locator("[data-choice=ok]").click();
  await expect(page.locator("#ink-dialog-text")).toContainText("快照名称已在别处改为「别处的名字」");
  await expect(dialog(page).getByRole("button", { name: "保留「别处的名字」" })).toBeVisible();
  await dialog(page).getByRole("button", { name: "使用「我的名字」" }).click();
  await expect(rows(page).first().locator("strong")).toHaveText("我的名字");
  expect((await harness(page))[0]).toMatchObject({ label: "我的名字", revision: 4 });

  // Conflict again: keep theirs.
  await page.evaluate(() => { (window as unknown as { inkDesktopHarness: Harness }).inkDesktopHarness.behavior.snapshotLabelConflict = "最终版"; });
  await rename.click();
  await page.getByLabel("快照名称").fill("不要这个");
  await dialog(page).locator("[data-choice=ok]").click();
  await dialog(page).getByRole("button", { name: "保留「最终版」" }).click();
  await expect(rows(page).first().locator("strong")).toHaveText("最终版");
  expect((await harness(page))[0].label).toBe("最终版");

  // Empty clears the name.
  await rename.click();
  await page.getByLabel("快照名称").fill("   ");
  await dialog(page).locator("[data-choice=ok]").click();
  await expect(rows(page).first().locator("strong")).toHaveText("第 0 步");
  await expect(page.locator("#desktop-ink-status")).toHaveText("已清除快照名称。");
  expect((await harness(page))[0].label).toBeNull();
});

test("the open snapshot shows its name and the name follows a rename", async ({ page }) => {
  await page.getByRole("button", { name: "保存快照", exact: true }).click();
  await page.getByLabel("快照名称").fill("整理稿");
  await dialog(page).locator("[data-choice=ok]").click();
  await rows(page).first().locator(".ink-session-item").click();
  await expect(page.locator("#desktop-ink-status")).toContainText("当前正在查看只读历史快照「整理稿」");
  await rows(page).first().getByRole("button", { name: "命名" }).click();
  await page.getByLabel("快照名称").fill("换个思路");
  await dialog(page).locator("[data-choice=ok]").click();
  await expect(rows(page).first().locator("strong")).toHaveText("换个思路");
});
