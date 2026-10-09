import { expect, test, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { parseInkDocument, projectInk } from "../src/ink/model";

async function credentials(): Promise<{ port: number; hostUrl: string; padToken: string }> {
  return JSON.parse(await readFile(fileURLToPath(new URL("../../.test-runs/ink-browser/access.json", import.meta.url)), "utf8"));
}
async function writeStroke(page: Page): Promise<void> {
  const rect = (await page.locator("#ink-board").boundingBox())!;
  const x = rect.x + rect.width / 2 - 70, y = rect.y + rect.height / 2 - 50;
  await page.mouse.move(x, y); await page.mouse.down();
  await page.mouse.move(x + 30, y + 65, { steps: 15 });
  await page.mouse.move(x + 70, y, { steps: 15 }); await page.mouse.up();
  await expect(page.locator("#ink-summary")).toContainText("1 笔");
}
async function pixels(page: Page): Promise<string> {
  return page.locator(".ink-settled").evaluate((canvas: HTMLCanvasElement) => canvas.toDataURL());
}

test("QR launcher, writing, reversible edits, JSON download/import and desktop save", async ({ page }, info) => {
  const access = await credentials();
  const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
  await page.goto(access.hostUrl);
  await expect(page.locator("#lab-status")).toContainText("实验服务已就绪");
  await expect(page.locator("#lab-url")).toHaveValue(/\/pad#session=/);
  expect(await page.locator("#lab-qr").evaluate((canvas: HTMLCanvasElement) => canvas.getContext("2d")!.getImageData(0, 0, canvas.width, canvas.height).data.some(value => value === 0))).toBe(true);
  await page.screenshot({ path: info.outputPath("launcher.png"), fullPage: true });
  await page.goto(`/pad#session=${access.padToken}`);
  await expect(page.locator("#ink-connection")).toHaveText("电脑可连接");
  await writeStroke(page);
  const before = await pixels(page);
  await page.getByRole("button", { name: "撤销", exact: true }).click();
  await expect(page.locator("#ink-summary")).toContainText("0 笔");
  expect(await pixels(page)).not.toBe(before);
  await page.getByRole("button", { name: "重做", exact: true }).click();
  await expect(page.locator("#ink-summary")).toContainText("1 笔");
  expect(await pixels(page)).toBe(before);
  await page.getByRole("button", { name: "整笔擦除", exact: true }).click();
  const board = (await page.locator("#ink-board").boundingBox())!;
  await page.mouse.click(board.x + board.width / 2 - 70, board.y + board.height / 2 - 50);
  await expect(page.locator("#ink-summary")).toContainText("0 笔");
  await page.getByRole("button", { name: "撤销", exact: true }).click();
  await expect(page.locator("#ink-summary")).toContainText("1 笔");
  expect(await pixels(page)).toBe(before);
  await page.getByRole("button", { name: "笔", exact: true }).click();
  const downloaded = page.waitForEvent("download");
  await page.getByRole("button", { name: "导出 Ink JSON", exact: true }).click();
  const filename = info.outputPath("handwriting.json"); await (await downloaded).saveAs(filename);
  const document = parseInkDocument(JSON.parse(await readFile(filename, "utf8")));
  expect(document.operations.map(op => op.kind)).toEqual(["stroke", "undo", "redo", "erase", "undo"]);
  expect(projectInk(document).pointCount).toBeGreaterThan(20);
  await page.getByRole("button", { name: "新建画板", exact: true }).click();
  await expect(page.locator("#ink-summary")).toContainText("0 笔");
  await page.locator("#ink-file").setInputFiles(filename);
  await expect(page.locator("#ink-status")).toContainText("已导入 1 笔及完整编辑历史");
  expect(await pixels(page)).toBe(before);
  await page.getByRole("button", { name: "保存到电脑", exact: true }).click();
  await expect(page.locator("#ink-status")).toContainText("电脑已保存 1 笔");
  await page.screenshot({ path: info.outputPath("pad.png"), fullPage: true });
  await page.goto(access.hostUrl);
  await expect(page.locator("#lab-files")).toContainText("数学手稿");
  expect(errors).toEqual([]);
});

test("offline pad still exports a file, and invalid imports preserve the current drawing", async ({ page }, info) => {
  const access = await credentials();
  await page.goto(`/pad#session=${access.padToken}`); await writeStroke(page);
  const before = await pixels(page);
  await page.locator("#ink-file").setInputFiles({ name: "invalid.json", mimeType: "application/json", buffer: Buffer.from('{"version":"unknown"}') });
  await expect(page.locator("#ink-status")).toContainText("Ink 文件无效");
  expect(await pixels(page)).toBe(before);
  await page.route("**/api/export", route => route.abort());
  await page.getByRole("button", { name: "保存到电脑", exact: true }).click();
  await expect(page.locator("#ink-status")).toContainText("可先下载 Ink JSON");
  const downloaded = page.waitForEvent("download");
  await page.getByRole("button", { name: "导出 Ink JSON", exact: true }).click();
  const filename = info.outputPath("offline.json"); await (await downloaded).saveAs(filename);
  expect(projectInk(parseInkDocument(JSON.parse(await readFile(filename, "utf8")))).visible).toHaveLength(1);
});

test("pen pressure, cancellation and touch navigation do not invent additional ink", async ({ page }) => {
  const access = await credentials(); await page.goto(`/pad#session=${access.padToken}`);
  await page.locator(".ink-input").evaluate((canvas: HTMLCanvasElement) => {
    const rect = canvas.getBoundingClientRect();
    const send = (type: string, offset: number, pressure: number, pointerType = "pen", pointerId = 17) => {
      canvas.dispatchEvent(new PointerEvent(type, { pointerId, pointerType, clientX: rect.x + rect.width / 2 + offset,
        clientY: rect.y + rect.height / 2, pressure, tiltX: 12, tiltY: -8, buttons: 1, bubbles: true }));
    };
    // Synthetic pointers do not own real capture; stub capture for this protocol test.
    canvas.setPointerCapture = () => {};
    send("pointerdown", 0, 0.2); send("pointermove", 25, 0.7);
    send("pointerdown", 100, 0.5, "touch", 18); send("pointermove", 120, 0.5, "touch", 18);
    send("pointercancel", 25, 0);
  });
  await expect(page.locator("#ink-summary")).toContainText("1 笔");
  const downloaded = page.waitForEvent("download"); await page.getByRole("button", { name: "导出 Ink JSON", exact: true }).click();
  const download = await downloaded;
  const content = await readFile((await download.path())!, "utf8");
  const projection = projectInk(parseInkDocument(JSON.parse(content)));
  expect(projection.strokes).toHaveLength(1); expect(projection.strokes[0].status).toBe("cancelled");
  expect(projection.strokes[0].points[0].pressure).toBeCloseTo(0.2, 6);
  expect(projection.strokes[0].points[1].pressure).toBeCloseTo(0.7, 6);
});

test("tablet-sized viewport retains room for writing and file controls", async ({ page }, info) => {
  const access = await credentials(); await page.setViewportSize({ width: 800, height: 1100 });
  await page.goto(`/pad#session=${access.padToken}`); await writeStroke(page);
  const viewport = page.viewportSize()!, board = (await page.locator("#ink-board").boundingBox())!;
  expect(board.height).toBeGreaterThan(600);
  const footer = (await page.getByRole("button", { name: "导出 Ink JSON", exact: true }).boundingBox())!;
  expect(footer.y + footer.height).toBeLessThanOrEqual(viewport.height);
  await page.screenshot({ path: info.outputPath("tablet-pad.png"), fullPage: true });
});

test("a pad link copied after opening on the Mac still authenticates a fresh tablet", async ({ page, browser }) => {
  const access = await credentials();
  await page.goto(`/pad#session=${access.padToken}`);
  await expect(page.locator("#ink-connection")).toHaveText("电脑可连接");
  const copiedUrl = page.url();
  const tabletContext = await browser.newContext({ viewport: { width: 800, height: 1100 } });
  try {
    const tablet = await tabletContext.newPage(); await tablet.goto(copiedUrl);
    await writeStroke(tablet);
    await tablet.getByRole("button", { name: "保存到电脑", exact: true }).click();
    await expect(tablet.locator("#ink-status")).toContainText("电脑已保存 1 笔");
  } finally { await tabletContext.close(); }
});

test("a transient first connection failure is retried without losing the session URL", async ({ page }) => {
  const access = await credentials(); let failures = 0;
  await page.route("**/api/pad", route => {
    if (failures++ === 0) return route.abort("failed");
    return route.continue();
  });
  const url = `http://127.0.0.1:${access.port}/pad#session=${access.padToken}`;
  await page.goto(url);
  await expect(page.locator("#ink-connection")).toHaveText("电脑可连接");
  expect(page.url()).toBe(url);
  expect(failures).toBeGreaterThanOrEqual(2);
});
