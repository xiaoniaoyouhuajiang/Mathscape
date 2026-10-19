import { expect, test, type Page } from "@playwright/test";

// Regression (PR #4 review, bug 2) on the real app shell (src/main.ts):
// create a linked note from a source node, switch between the map and the
// 手写 workspace, and check the map overlay cache and the inspector follow
// trash / undo / restore / rename without a restart.
const map = {
  label: (page: Page) => page.evaluate(() => window.appHarness?.labels()["r-unit-thm"]),
  notes: (page: Page) => page.evaluate(() => window.appHarness?.inkNodes()),
};
const rowAction = (page: Page, title: string) => page.locator(".ink-session-row", { hasText: title }).locator(".ink-session-action");
const choose = async (page: Page, label: string) => {
  await expect(page.locator("#ink-dialog")).toBeVisible();
  await page.locator("#ink-dialog").getByRole("button", { name: label, exact: true }).click();
};
const toMap = async (page: Page) => {
  await page.locator("#ink-close").click();
  await expect(page.locator("#graph, .graph-container").first()).toBeVisible();
};
const toInk = async (page: Page) => {
  await page.getByRole("button", { name: "手写", exact: true }).click();
  await expect(page.locator("#ink-session-list")).toBeVisible();
};

test("N2 the map and inspector follow note lifecycle and renames across workspace switches", async ({ page }) => {
  await page.goto("/tests/app.html");
  await expect.poll(() => map.label(page)).toBe("定理 3.2");
  await page.getByRole("checkbox", { name: "我的手稿" }).check();

  // From the source node: 新建手稿 → 在电脑上书写 opens the new linked note.
  await page.evaluate(() => window.appHarness.selectNode("r-unit-thm"));
  await page.getByRole("button", { name: "新建手稿" }).click();
  await page.locator(".ink-create-popover").getByRole("button", { name: "在电脑上书写" }).click();
  await expect(page.locator("#ink-document-heading")).toHaveText("尝试 · 定理 3.2");
  await toMap(page);
  await expect.poll(() => map.label(page)).toBe("定理 3.2 · ✎1");
  await expect.poll(() => map.notes(page)).toEqual([expect.stringMatching(/=尝试 · 定理 3\.2$/)]);
  await expect(page.getByText("相关手稿（1）")).toBeVisible();

  // Trash in the workspace: the cached overlay is already updated before
  // returning to the map (event path), and stays correct after switching.
  await toInk(page);
  await rowAction(page, "尝试 · 定理 3.2").click();
  await choose(page, "移到回收站");
  await expect.poll(() => map.notes(page)).toEqual([]);
  await expect.poll(() => map.label(page)).toBe("定理 3.2");
  await toMap(page);
  await expect.poll(() => map.notes(page)).toEqual([]);
  await expect(page.getByText("还没有相关手稿")).toBeVisible();

  // Undo the trash.
  await toInk(page);
  await page.locator("#ink-undo-trash").click();
  await expect.poll(() => map.label(page)).toBe("定理 3.2 · ✎1");
  await toMap(page);
  await expect.poll(() => map.notes(page)).toHaveLength(1);
  await expect(page.getByText("相关手稿（1）")).toBeVisible();

  // Trash again, then restore from the 回收站 tab.
  await toInk(page);
  await rowAction(page, "尝试 · 定理 3.2").click();
  await choose(page, "移到回收站");
  await page.locator("#ink-tab-trash").click();
  await rowAction(page, "尝试 · 定理 3.2").click();
  await expect(page.locator("#desktop-ink-status")).toContainText("「尝试 · 定理 3.2」已恢复");
  await toMap(page);
  await expect.poll(() => map.notes(page)).toEqual([expect.stringMatching(/=尝试 · 定理 3\.2$/)]);

  // Rename: the note node and the inspector list show the new title.
  await toInk(page);
  await page.locator("#ink-tab-active").click();
  // Restoring reopened the note (status above is set after the reopen).
  await expect(page.locator("#ink-document-heading")).toHaveText("尝试 · 定理 3.2");
  const title = page.locator("#desktop-ink-title");
  await expect(title).toHaveValue("尝试 · 定理 3.2");
  await expect(title).toBeEditable();
  await title.fill("我的证明");
  await title.press("Enter");
  await expect(page.locator("#ink-document-heading")).toHaveText("我的证明");
  await toMap(page);
  await expect.poll(() => map.notes(page)).toEqual([expect.stringMatching(/=我的证明$/)]);
  await expect(page.getByText("我的证明").first()).toBeVisible();
});
