import { expect, test, type Page } from "@playwright/test";

// N3.0 quick fixes on the desktop Ink workspace: shared preset palette and
// deleting a lasso selection with one undoable erase.

async function writeLine(page: Page, dy = 0): Promise<void> {
  const box = (await page.locator("#desktop-ink-board .ink-input").boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2 + dy;
  await page.mouse.move(x - 60, y); await page.mouse.down(); await page.mouse.move(x + 60, y, { steps: 8 }); await page.mouse.up();
}

async function lassoAll(page: Page): Promise<void> {
  const box = (await page.locator("#desktop-ink-board .ink-input").boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2;
  await page.mouse.move(x - 110, y - 90); await page.mouse.down();
  for (const [px, py] of [[x + 110, y - 90], [x + 110, y + 90], [x - 110, y + 90], [x - 110, y - 85]]) await page.mouse.move(px, py, { steps: 6 });
  await page.mouse.up();
}

/** Operations of the document that received writing in this test. */
async function operations(page: Page): Promise<Array<{ kind: string; stroke?: { brush: { color: string } }; stroke_ids?: string[]; color?: string }>> {
  return page.evaluate(() => [...window.inkDesktopHarness.storedDocuments.values()].flatMap(document => document.operations).map(operation => structuredClone(operation)) as never);
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/ink-desktop.html");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
});

test("N3.0 the desktop palette defaults to #193a66, remembers its choice and never recolours old strokes", async ({ page }) => {
  const palette = page.getByRole("group", { name: "笔迹颜色" });
  await expect(palette.locator(".ink-swatch")).toHaveCount(6);
  await expect(palette).toHaveAttribute("data-color", "#193a66");
  await expect(palette.getByRole("button", { name: "深蓝（默认）" })).toHaveAttribute("aria-pressed", "true");
  await writeLine(page, -45);
  await expect(page.locator("#ink-save-badge")).toContainText("head 1");

  await palette.getByRole("button", { name: "红" }).click();
  await expect(palette.getByRole("button", { name: "红" })).toHaveAttribute("aria-pressed", "true");
  await expect(palette.getByRole("button", { name: "深蓝（默认）" })).toHaveAttribute("aria-pressed", "false");
  await writeLine(page, 45);
  await expect(page.locator("#ink-save-badge")).toContainText("head 2");
  expect((await operations(page)).filter(operation => operation.kind === "stroke").map(operation => operation.stroke!.brush.color)).toEqual(["#193a66", "#c62828"]);

  await page.locator("#desktop-ink-color").evaluate((input: HTMLInputElement) => { input.value = "#123456"; input.dispatchEvent(new Event("input", { bubbles: true })); });
  await expect(palette).toHaveAttribute("data-color", "#123456");
  await expect(palette.locator(".ink-swatch[aria-pressed='true']")).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("group", { name: "笔迹颜色" })).toHaveAttribute("data-color", "#123456");
  await expect(page.locator("#desktop-ink-color")).toHaveValue("#123456");
  expect(await page.evaluate(() => localStorage.getItem("mathscape:ink-desktop-color"))).toBe("#123456");
  expect(await page.evaluate(() => localStorage.getItem("mathscape:ink-pad-color"))).toBeNull();
});

test("N3.0 deleting a lasso selection is one erase that a single undo restores", async ({ page }) => {
  const remove = page.getByRole("button", { name: "删除所选", exact: true });
  await writeLine(page, -45); await writeLine(page, 45);
  await expect(page.locator("#ink-save-badge")).toContainText("head 2");
  await expect(remove).toBeDisabled();
  await page.getByRole("button", { name: "套索选择", exact: true }).click();
  await expect(remove).toBeDisabled();
  await lassoAll(page);
  await expect(remove).toBeEnabled();
  await remove.click();
  await expect(page.locator("#desktop-ink-summary")).toContainText("0 笔");
  await expect(remove).toBeDisabled();
  await expect(page.locator("#ink-save-badge")).toContainText("head 3");
  const erase = (await operations(page)).at(-1)!;
  expect(erase.kind).toBe("erase");
  expect(erase.stroke_ids).toHaveLength(2);
  await page.getByRole("button", { name: "撤销", exact: true }).click();
  await expect(page.locator("#desktop-ink-summary")).toContainText("2 笔");
  await expect(page.locator("#ink-save-badge")).toContainText("head 4");
});

test("N3.0 Delete/Backspace remove the selection but never while typing in a field", async ({ page }) => {
  await writeLine(page, -45); await writeLine(page, 45);
  await expect(page.locator("#ink-save-badge")).toContainText("head 2");
  // Outside the select tool the shortcut does nothing.
  await page.keyboard.press("Delete");
  await expect(page.locator("#desktop-ink-summary")).toContainText("2 笔");
  await page.getByRole("button", { name: "套索选择", exact: true }).click();
  await lassoAll(page);
  await expect(page.getByRole("button", { name: "删除所选", exact: true })).toBeEnabled();

  const title = page.locator("#desktop-ink-title");
  await title.focus();
  await title.press("End");
  await title.press("Backspace");
  await title.press("Delete");
  await title.press("Escape");
  await expect(page.locator("#desktop-ink-summary")).toContainText("2 笔");
  expect((await operations(page)).some(operation => operation.kind === "erase")).toBe(false);

  await page.locator("#desktop-ink-board .ink-input").focus().catch(() => undefined);
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press("Backspace");
  await expect(page.locator("#desktop-ink-summary")).toContainText("0 笔");
  await page.getByRole("button", { name: "撤销", exact: true }).click();
  await expect(page.locator("#desktop-ink-summary")).toContainText("2 笔");
});

test("N3.0 pairing offers LAN addresses by default, keeps others behind a toggle and remembers a manual pick", async ({ page }) => {
  const address = page.locator("[data-address]"), link = page.locator("[data-link]");
  const toggle = page.getByRole("checkbox", { name: /显示其他地址/ }).first();
  await page.getByRole("button", { name: "平板书写", exact: true }).click();
  await page.getByRole("button", { name: "生成配对二维码", exact: true }).click();
  await expect(page.locator("[data-qr]")).toBeVisible();
  await expect(address.locator("option")).toHaveCount(1);
  await expect(link).toHaveValue(/^https:\/\/192\.168\.31\.20:1443\/pad\/#pair=/);
  await expect(page.locator("[data-address-hint='pair']")).toBeHidden();

  await toggle.check();
  await expect(address.locator("option")).toHaveCount(3);
  await expect(address.locator("optgroup[label='其他地址'] option")).toHaveText(["100.90.0.2（VPN / Tailscale）", "127.0.0.1（本机测试）"]);
  await expect(link).toHaveValue(/192\.168\.31\.20/);
  await address.selectOption({ label: "100.90.0.2（VPN / Tailscale）" });
  await expect(link).toHaveValue(/^https:\/\/100\.90\.0\.2:1443\//);
  expect(await page.evaluate(() => localStorage.getItem("mathscape:ink-lan-preferred-host"))).toBe("100.90.0.2");

  await page.getByRole("button", { name: "取消待配对", exact: true }).click();
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
  await page.reload();
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
  await page.getByRole("button", { name: "平板书写", exact: true }).click();
  await page.getByRole("button", { name: "生成配对二维码", exact: true }).click();
  await expect(link).toHaveValue(/^https:\/\/100\.90\.0\.2:1443\//);
  await expect(page.getByRole("checkbox", { name: /显示其他地址/ }).first()).toBeChecked();
});

test("N3.0 without a LAN address no QR is drawn until an address is picked by hand", async ({ page }) => {
  await page.evaluate(() => { (window.inkDesktopHarness.behavior as { noLanAddress: boolean }).noLanAddress = true; });
  const address = page.locator("[data-address]"), link = page.locator("[data-link]");
  await page.getByRole("button", { name: "平板书写", exact: true }).click();
  await page.getByRole("button", { name: "生成配对二维码", exact: true }).click();
  await expect(page.locator("[data-address-hint='pair']")).toContainText("未发现局域网地址");
  await expect(link).toHaveValue("");
  await expect(address).toHaveValue("");
  await page.getByRole("checkbox", { name: /显示其他地址/ }).first().check();
  await expect(address).toHaveValue("");
  await address.selectOption({ label: "192.168.31.20（VPN / Tailscale）" });
  await expect(link).toHaveValue(/^https:\/\/192\.168\.31\.20:1443\/pad\/#pair=/);
});

test("N3a restyle recolours a lasso selection in one undoable command", async ({ page }) => {
  const restyle = page.getByRole("button", { name: "改颜色", exact: true });
  const palette = page.getByRole("group", { name: "笔迹颜色" });
  await writeLine(page, -45); await writeLine(page, 45);
  await expect(page.locator("#ink-save-badge")).toContainText("head 2");
  await expect(restyle).toBeDisabled();
  await page.getByRole("button", { name: "套索选择", exact: true }).click();
  await lassoAll(page);
  await expect(restyle).toBeEnabled();
  await palette.getByRole("button", { name: "红" }).click();
  await restyle.click();
  await expect(page.locator("#ink-save-badge")).toContainText("head 3");
  const ops = await operations(page);
  const last = ops.at(-1)!;
  expect(last.kind).toBe("restyle");
  expect(last.stroke_ids).toHaveLength(2);
  expect(last.color).toBe("#c62828");
  await page.getByRole("button", { name: "撤销", exact: true }).click();
  await expect(page.locator("#ink-save-badge")).toContainText("head 4");
  expect((await operations(page)).at(-1)!.kind).toBe("undo");
});

test("N3a push to tablet falls back to the QR when the tablet is offline", async ({ page }) => {
  await page.evaluate(() => { (window.inkDesktopHarness.behavior as { pushOffline: boolean }).pushOffline = true; });
  await page.getByRole("button", { name: "推送到平板", exact: true }).click();
  await expect(page.locator("#ink-lan-panel")).toBeVisible();
  await expect(page.locator("[data-qr]")).toBeVisible();
  await expect(page.locator("[data-link]")).toHaveValue(/#pair=/);
});

test("N3a push asks before taking over a remote writer", async ({ page }) => {
  await page.evaluate(() => { (window.inkDesktopHarness.behavior as { pushNeedsTakeover: boolean }).pushNeedsTakeover = true; });
  const push = page.getByRole("button", { name: "推送到平板", exact: true });
  await push.click();
  await expect(page.getByRole("button", { name: /仍然推送/ })).toBeVisible();
  await page.getByRole("button", { name: /仍然推送/ }).click();
  await expect.poll(() => page.evaluate(() => (window.inkDesktopHarness.behavior as { pushArgs: unknown[] }).pushArgs.length)).toBe(2);
  const args = await page.evaluate(() => (window.inkDesktopHarness.behavior as { pushArgs: Array<{ allowTransfer?: boolean }> }).pushArgs);
  expect(args[0].allowTransfer).toBeFalsy();
  expect(args[1].allowTransfer).toBe(true);
});
