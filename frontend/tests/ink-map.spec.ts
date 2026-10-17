import { expect, test } from "@playwright/test";

// N2 map overlay (§6.3, §14.7) and inspector "相关手稿".
test.beforeEach(async ({ page }) => {
  await page.goto("/tests/ink-map.html");
  await page.waitForFunction(() => "inkMap" in window);
});

test("N2 count markers come from the overlay and survive re-renders", async ({ page }) => {
  const labels = () => page.evaluate(() => window.inkMap.labels());
  await expect.poll(async () => (await labels())["r-thm"]).toBe("定理 3.2 · ✎2");
  expect((await labels())["r-art"]).toBe("数学笔记.md · ✎1");
  expect((await labels())["r-ex"]).toBe("例 4");
  await page.evaluate(() => window.inkMap.rerender());
  expect((await labels())["r-thm"]).toBe("定理 3.2 · ✎2");
  expect(await page.evaluate(() => window.inkMap.inkNodes())).toEqual([]);
});

test("N2 我的手稿 toggle draws note nodes, persists, caps at 60 and opens notes", async ({ page }) => {
  const toggle = page.getByRole("checkbox", { name: "我的手稿" });
  await expect(toggle).not.toBeChecked();
  await toggle.check();
  await expect.poll(() => page.evaluate(() => window.inkMap.inkNodes().sort())).toEqual(["ink-note:note-1", "ink-note:note-2", "ink-note:note-md"]);
  const edges = await page.evaluate(() => window.inkMap.inkEdges());
  expect(edges).toContainEqual(["ink-note:note-2", "r-thm"]);
  expect(edges).toContainEqual(["ink-note:note-2", "r-ex"]);
  // Markdown-linked notes attach to the file node.
  expect(edges).toContainEqual(["ink-note:note-md", "r-art"]);
  await expect(page.locator("#legend")).toContainText("我的手稿");
  // Synthetic nodes are rebuilt, not duplicated, on re-render.
  await page.evaluate(() => window.inkMap.rerender());
  expect(await page.evaluate(() => window.inkMap.inkNodes().length)).toBe(3);

  await page.evaluate(() => window.inkMap.tap("ink-note:note-1"));
  expect(await page.evaluate(() => window.inkMap.state.opened)).toEqual(["note-1"]);

  await page.evaluate(() => { window.inkMap.bulk.enabled = true; window.inkMap.reload(); });
  expect(await page.evaluate(() => window.inkMap.inkNodes().length)).toBe(60);
  await expect(page.locator("label", { has: toggle })).toContainText("另有 8 份未显示");

  await page.reload();
  await page.waitForFunction(() => "inkMap" in window);
  await expect(page.getByRole("checkbox", { name: "我的手稿" })).toBeChecked();
  await expect.poll(() => page.evaluate(() => window.inkMap.inkNodes().length)).toBe(3);
  await page.getByRole("checkbox", { name: "我的手稿" }).uncheck();
  expect(await page.evaluate(() => window.inkMap.inkNodes().length)).toBe(0);
});

test("N2 inspector lists related notes and creates a note for a source unit", async ({ page }) => {
  await page.evaluate(() => window.inkMap.select("r-thm"));
  const section = page.getByRole("region", { name: "相关手稿" });
  await expect(section).toContainText("相关手稿（2）");
  await expect(section.getByRole("button", { name: "尝试 · 定理 3.2" })).toBeVisible();
  await expect(section).toContainText("另证 · 主关联");
  await section.getByRole("button", { name: "另证 · 定理 3.2" }).click();
  expect(await page.evaluate(() => window.inkMap.state.opened)).toEqual(["note-2"]);

  await page.getByRole("button", { name: "新建手稿" }).click();
  const popover = page.locator(".ink-create-popover");
  await expect(popover).toContainText("关联到：定理 3.2");
  await popover.getByRole("button", { name: "在电脑上书写" }).click();
  expect(await page.evaluate(() => window.inkMap.state.creates)).toEqual([{ target: { kind: "source_object", object_id: "unit-thm" }, choice: { role: "attempt", title: "", mode: "desktop" } }]);

  // Markdown files list their notes but offer no "新建手稿" (that is block-level in the reader).
  await page.evaluate(() => window.inkMap.select("r-art"));
  await expect(page.getByRole("region", { name: "相关手稿" })).toContainText("学习笔记 · 第一章");
  await expect(page.getByRole("button", { name: "新建手稿" })).toHaveCount(0);
  // A retired unit can list notes but cannot create new ones.
  await page.evaluate(() => window.inkMap.select("r-old"));
  await expect(page.getByRole("region", { name: "相关手稿" })).toContainText("还没有相关手稿");
  await expect(page.getByRole("button", { name: "新建手稿" })).toHaveCount(0);
});
