import { expect, test, type Page } from "@playwright/test";
import { boardGeometry, isFittedCentered, settledGeometry } from "./support/ink-geometry";

// N3e on the desktop 手稿 workspace: H1 corner scaling, M4 lasso majority +
// live pre-selection, M1 keyboard shortcuts and H5 leaving with unsaved
// operations. Assertions read real geometry: stored strokes projected to the
// screen through the settled canvas' 2D transform, and canvas pixels.

type Rect = { minX: number; minY: number; maxX: number; maxY: number };

async function center(page: Page): Promise<{ x: number; y: number }> {
  const box = (await page.locator("#desktop-ink-board .ink-input").boundingBox())!;
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}

async function drag(page: Page, points: Array<[number, number]>, steps = 6): Promise<void> {
  await page.mouse.move(points[0][0], points[0][1]); await page.mouse.down();
  for (const [x, y] of points.slice(1)) await page.mouse.move(x, y, { steps });
  await page.mouse.up();
}

/** Pen stroke between two offsets from the board centre. */
async function stroke(page: Page, from: [number, number], to: [number, number]): Promise<void> {
  const c = await center(page);
  await drag(page, [[c.x + from[0], c.y + from[1]], [c.x + to[0], c.y + to[1]]], 8);
}

/** Closed rectangular lasso, in offsets from the board centre. */
async function lassoRect(page: Page, left: number, top: number, right: number, bottom: number, release = true): Promise<void> {
  const c = await center(page);
  await page.mouse.move(c.x + left, c.y + top); await page.mouse.down();
  for (const [x, y] of [[right, top], [right, bottom], [left, bottom], [left, top + 3]]) await page.mouse.move(c.x + x, c.y + y, { steps: 6 });
  if (release) await page.mouse.up();
}

/** Visible strokes of the open document (stored operations, transforms applied), on screen. */
async function screenBounds(page: Page): Promise<Rect> {
  const world = await page.evaluate(() => window.inkDesktopHarness.visibleBounds());
  expect(world).not.toBeNull();
  const g = await boardGeometry(page);
  const x = (v: number) => g.board.left + g.offsetX + v * g.scale, y = (v: number) => g.board.top + g.offsetY + v * g.scale;
  return { minX: x(world!.minX), minY: y(world!.minY), maxX: x(world!.maxX), maxY: y(world!.maxY) };
}

/** Bounding box (client px) of the painted pixels of one canvas layer. */
async function paintedBounds(page: Page, layer: "preview" | "input" | "settled"): Promise<Rect | null> {
  return page.evaluate(layer => {
    const canvas = document.querySelector<HTMLCanvasElement>(`#desktop-ink-board .ink-${layer}`)!;
    const data = canvas.getContext("2d")!.getImageData(0, 0, canvas.width, canvas.height).data;
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (let y = 0; y < canvas.height; y++) for (let x = 0; x < canvas.width; x++) {
      if (data[(y * canvas.width + x) * 4 + 3] < 24) continue;
      if (x < minX) minX = x; if (x > maxX) maxX = x; if (y < minY) minY = y; if (y > maxY) maxY = y;
    }
    if (minX === Infinity) return null;
    const rect = canvas.getBoundingClientRect(), k = rect.width / canvas.width;
    return { minX: rect.left + minX * k, minY: rect.top + minY * k, maxX: rect.left + (maxX + 1) * k, maxY: rect.top + (maxY + 1) * k };
  }, layer);
}

/** RGBA of one client pixel of a canvas layer. */
async function pixel(page: Page, layer: "input" | "settled", x: number, y: number): Promise<[number, number, number, number]> {
  return page.evaluate(({ layer, x, y }) => {
    const canvas = document.querySelector<HTMLCanvasElement>(`#desktop-ink-board .ink-${layer}`)!;
    const rect = canvas.getBoundingClientRect(), k = canvas.width / rect.width;
    return [...canvas.getContext("2d")!.getImageData(Math.round((x - rect.left) * k), Math.round((y - rect.top) * k), 1, 1).data] as [number, number, number, number];
  }, { layer, x, y });
}

async function pixelNear(page: Page, layer: "input" | "settled", x: number, y: number, radius = 8): Promise<[number, number, number, number]> {
  return page.evaluate(({ layer, x, y, radius }) => {
    const canvas = document.querySelector<HTMLCanvasElement>(`#desktop-ink-board .ink-${layer}`)!;
    const rect = canvas.getBoundingClientRect();
    const k = canvas.width / rect.width;
    const cx = Math.round((x - rect.left) * k);
    const cy = Math.round((y - rect.top) * k);
    const context = canvas.getContext("2d")!;
    let best: [number, number, number, number] = [0, 0, 0, 0];
    for (let dy = -radius; dy <= radius; dy++) for (let dx = -radius; dx <= radius; dx++) {
      const px = Math.max(0, Math.min(canvas.width - 1, cx + dx));
      const py = Math.max(0, Math.min(canvas.height - 1, cy + dy));
      const sample = [...context.getImageData(px, py, 1, 1).data] as [number, number, number, number];
      if (sample[1] > sample[0] && sample[3] > best[3]) best = sample;
    }
    return best;
  }, { layer, x, y, radius });
}

const ops = (page: Page) => page.evaluate(() => {
  const id = window.inkDesktopHarness.workspace.currentDocumentId();
  return id ? window.inkDesktopHarness.storedDocuments.get(id)!.operations.map(operation => ({ kind: operation.kind, stroke_ids: (operation as { stroke_ids?: string[] }).stroke_ids })) : [];
});
const visibleCount = (page: Page) => page.evaluate(() => window.inkDesktopHarness.visibleStrokeCount());
const head = (page: Page, n: number) => expect(page.locator("#ink-save-badge")).toContainText("head " + n);
const lassoTool = (page: Page) => page.getByRole("button", { name: "套索选择", exact: true }).click();

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/ink-desktop.html");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
  await settledGeometry(page);
});

test.describe("H1 corner handles scale around the opposite corner", () => {
  const corners = [
    { handle: "nw", delta: [-50, -30] },
    { handle: "ne", delta: [50, -30] },
    { handle: "sw", delta: [-50, 30] },
    { handle: "se", delta: [50, 30] },
  ] as const;
  for (const { handle, delta } of corners) {
    test(`${handle}: the grabbed corner follows the pointer, the opposite corner stays put`, async ({ page }) => {
      await stroke(page, [-60, -40], [60, 40]);
      await head(page, 1);
      await lassoTool(page);
      await lassoRect(page, -110, -90, 110, 90);
      const before = await screenBounds(page);
      const west = handle.endsWith("w"), north = handle.startsWith("n");
      const grab = { x: (west ? before.minX : before.maxX) + (west ? -10 : 10), y: (north ? before.minY : before.maxY) + (north ? -10 : 10) };
      // A tiny drag first: the old bug turned 1–3 px on nw/ne/sw into a x-hundreds blow-up.
      await page.mouse.move(grab.x, grab.y); await page.mouse.down();
      await page.mouse.move(grab.x + delta[0] / 25, grab.y + delta[1] / 25, { steps: 2 });
      await page.waitForTimeout(50);
      const tiny = (await paintedBounds(page, "preview"))!;
      expect(tiny.maxX - tiny.minX).toBeLessThan(before.maxX - before.minX + 20);
      expect(tiny.maxY - tiny.minY).toBeLessThan(before.maxY - before.minY + 20);
      // Then the real drag; check the live preview pixels mid-drag.
      await page.mouse.move(grab.x + delta[0], grab.y + delta[1], { steps: 8 });
      await page.waitForTimeout(50);
      const expected: Rect = {
        minX: before.minX + (west ? delta[0] : 0), maxX: before.maxX + (west ? 0 : delta[0]),
        minY: before.minY + (north ? delta[1] : 0), maxY: before.maxY + (north ? 0 : delta[1]),
      };
      const live = (await paintedBounds(page, "preview"))!;
      const g = await boardGeometry(page), ink = 4 * g.scale / 2 + 3; // half brush width + antialiasing
      for (const side of ["minX", "minY", "maxX", "maxY"] as const) expect(Math.abs(live[side] - expected[side]), side).toBeLessThan(ink + (side.startsWith("min") ? 1 : 1));
      await page.mouse.up();
      await head(page, 2);
      expect((await ops(page)).at(-1)!.kind).toBe("transform");
      const after = await screenBounds(page);
      for (const side of ["minX", "minY", "maxX", "maxY"] as const) expect(Math.abs(after[side] - expected[side]), side).toBeLessThan(1.5);
    });
  }

  test("rotate, then scale: the rotated selection still scales from its opposite corner", async ({ page }) => {
    await stroke(page, [-60, -40], [60, 40]);
    await head(page, 1);
    await lassoTool(page);
    await lassoRect(page, -110, -90, 110, 90);
    const before = await screenBounds(page);
    const cx = (before.minX + before.maxX) / 2, cy = (before.minY + before.maxY) / 2;
    const handle = { x: cx, y: before.minY - 55 }, radius = cy - handle.y;
    // Quarter turn clockwise: the rotate handle travels from the top to the right.
    await page.mouse.move(handle.x, handle.y); await page.mouse.down();
    for (let i = 1; i <= 12; i++) {
      const angle = -Math.PI / 2 + (Math.PI / 2) * (i / 12);
      await page.mouse.move(cx + radius * Math.cos(angle), cy + radius * Math.sin(angle));
    }
    await page.mouse.up();
    await head(page, 2);
    const rotated = await screenBounds(page);
    expect(Math.abs((rotated.maxX - rotated.minX) - (before.maxY - before.minY))).toBeLessThan(3);
    expect(Math.abs((rotated.maxY - rotated.minY) - (before.maxX - before.minX))).toBeLessThan(3);
    expect(Math.abs((rotated.minX + rotated.maxX) / 2 - cx)).toBeLessThan(1.5);
    expect(Math.abs((rotated.minY + rotated.maxY) / 2 - cy)).toBeLessThan(1.5);
    // The selection survives the transform; scale it from the nw corner.
    await drag(page, [[rotated.minX - 10, rotated.minY - 10], [rotated.minX - 40, rotated.minY - 60]]);
    await head(page, 3);
    const scaled = await screenBounds(page);
    expect(Math.abs(scaled.maxX - rotated.maxX)).toBeLessThan(1.5);
    expect(Math.abs(scaled.maxY - rotated.maxY)).toBeLessThan(1.5);
    expect(Math.abs(scaled.minX - (rotated.minX - 30))).toBeLessThan(1.5);
    expect(Math.abs(scaled.minY - (rotated.minY - 50))).toBeLessThan(1.5);
  });
});

test.describe("M4 lasso majority rule and live pre-selection", () => {
  test("a stroke is lassoed only when most of it is inside, and is highlighted before release", async ({ page }) => {
    const c = await center(page);
    await stroke(page, [-60, -45], [60, -45]); // A
    await stroke(page, [-60, 45], [60, 45]);   // B
    await head(page, 2);
    await lassoTool(page);
    expect(await pixel(page, "input", c.x - 30, c.y - 45)).toEqual([0, 0, 0, 0]);
    // Lasso around ~70% of A and ~30% of B (B's right end), still held down.
    const c2 = await center(page);
    await page.mouse.move(c2.x - 80, c2.y - 70); await page.mouse.down();
    for (const [x, y] of [[25, -70], [25, 20], [80, 20], [80, 70], [20, 70], [20, -20], [-80, -20], [-80, -67]] as const) await page.mouse.move(c2.x + x, c2.y + y, { steps: 6 });
    await page.waitForTimeout(80);
    // Live pre-highlight: a teal halo on A (inside and just beside the line), nothing on B.
    const onA = await pixel(page, "input", c.x - 30, c.y - 45);
    await expect.poll(async () => (await pixel(page, "input", c.x - 30, c.y - 45))[3], { timeout: 1500 }).toBeGreaterThan(100);
    await expect.poll(async () => (await pixelNear(page, "input", c.x - 30, c.y - 45))[3], { timeout: 1500 }).toBeGreaterThan(40);
    const besideA = await pixelNear(page, "input", c.x - 30, c.y - 45);
    expect(onA[3]).toBeGreaterThan(100);
    expect(besideA[3]).toBeGreaterThan(40);
    expect(besideA[1]).toBeGreaterThan(besideA[0]); // greenish teal, not the ink colour
    expect(await pixel(page, "input", c.x + 45, c.y + 45)).toEqual([0, 0, 0, 0]);
    await page.mouse.up();
    // After release the selection is exactly A: deleting it leaves B.
    const remove = page.getByRole("button", { name: "删除所选", exact: true });
    await expect(remove).toBeEnabled();
    await remove.click();
    await head(page, 3);
    expect(await visibleCount(page)).toBe(1);
    const left = await screenBounds(page);
    expect(Math.abs(left.minY - (c.y + 45))).toBeLessThan(2);
    // The halo is gone once the lasso ends.
    expect(await pixel(page, "input", c.x - 30, c.y - 45)).toEqual([0, 0, 0, 0]);
  });

  test("a lasso around a minority of a stroke selects nothing and shows no halo", async ({ page }) => {
    const c = await center(page);
    await stroke(page, [-60, 0], [60, 0]);
    await head(page, 1);
    await lassoTool(page);
    await lassoRect(page, 20, -40, 90, 40, false);
    await page.waitForTimeout(80);
    expect(await pixel(page, "input", c.x + 45, c.y)).toEqual([0, 0, 0, 0]);
    await page.mouse.up();
    await expect(page.getByRole("button", { name: "删除所选", exact: true })).toBeDisabled();
    // Widening the lasso past the middle selects it.
    await lassoRect(page, -20, -40, 90, 40);
    await expect(page.getByRole("button", { name: "删除所选", exact: true })).toBeEnabled();
  });
});

test.describe("M1 keyboard shortcuts", () => {
  test("Ctrl/⌘+Z undoes, Ctrl+Shift+Z / Ctrl+Y redo, never while typing in a field", async ({ page }) => {
    await stroke(page, [-60, -45], [60, -45]);
    await stroke(page, [-60, 45], [60, 45]);
    await head(page, 2);
    await page.keyboard.press("Control+z");
    await head(page, 3);
    expect(await visibleCount(page)).toBe(1);
    await page.keyboard.press("Meta+z");
    await head(page, 4);
    expect(await visibleCount(page)).toBe(0);
    await page.keyboard.press("Control+Shift+z");
    await head(page, 5);
    await page.keyboard.press("Control+y");
    await head(page, 6);
    expect(await visibleCount(page)).toBe(2);
    // Typing in the title: Ctrl+Z and tool letters belong to the field.
    const title = page.locator("#desktop-ink-title");
    await title.click();
    await page.keyboard.press("Control+z");
    await page.keyboard.press("e");
    await page.keyboard.press("Escape");
    await page.waitForTimeout(150);
    expect((await ops(page)).length).toBe(6);
    expect(await visibleCount(page)).toBe(2);
    // The pen is still the tool: a new stroke is written, not erased.
    await stroke(page, [-60, 0], [60, 0]);
    await head(page, 7);
    expect(await visibleCount(page)).toBe(3);
  });

  test("tool keys switch tools; Esc cancels a lasso, then clears the selection; Delete is one undoable erase", async ({ page }) => {
    const c = await center(page);
    await stroke(page, [-60, -45], [60, -45]);
    await stroke(page, [-60, 45], [60, 45]);
    await head(page, 2);
    // E: whole-stroke eraser — dragging across A erases it.
    await page.keyboard.press("e");
    await drag(page, [[c.x - 10, c.y - 70], [c.x - 10, c.y - 20]]);
    await head(page, 3);
    expect(await visibleCount(page)).toBe(1);
    await page.keyboard.press("Control+z");
    await head(page, 4);
    // L: lasso. Esc during the drag cancels it — no selection, no halo.
    await page.keyboard.press("l");
    await lassoRect(page, -110, -90, 110, 90, false);
    await page.waitForTimeout(60);
    expect((await pixel(page, "input", c.x - 30, c.y - 45))[3]).toBeGreaterThan(0);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(60);
    expect(await paintedBounds(page, "input")).toBeNull();
    await page.mouse.up();
    const remove = page.getByRole("button", { name: "删除所选", exact: true });
    await expect(remove).toBeDisabled();
    // A real lasso, then Esc clears the selection (its frame disappears).
    await lassoRect(page, -110, -90, 110, 90);
    await expect(remove).toBeEnabled();
    await expect.poll(() => paintedBounds(page, "input")).not.toBeNull();
    await page.keyboard.press("Escape");
    await expect(remove).toBeDisabled();
    await expect.poll(() => paintedBounds(page, "input")).toBeNull();
    // Delete: one multi-stroke erase; one Ctrl+Z brings both back.
    await lassoRect(page, -110, -90, 110, 90);
    await page.keyboard.press("Delete");
    await head(page, 5);
    const erase = (await ops(page)).at(-1)!;
    expect(erase.kind).toBe("erase");
    expect(erase.stroke_ids).toHaveLength(2);
    expect(await visibleCount(page)).toBe(0);
    await page.keyboard.press("Control+z");
    await head(page, 6);
    expect(await visibleCount(page)).toBe(2);
    // Shift+E: partial eraser cuts B in two.
    await page.keyboard.press("Shift+E");
    await drag(page, [[c.x, c.y + 20], [c.x, c.y + 70]]);
    await head(page, 7);
    expect((await ops(page)).at(-1)!.kind).toBe("partial_erase");
    // P: back to the pen.
    await page.keyboard.press("p");
    await stroke(page, [-60, 0], [60, 0]);
    await head(page, 8);
    expect((await ops(page)).at(-1)!.kind).toBe("stroke");
  });

  test("H pans without writing; Ctrl/⌘ +/−/0 zoom around the board", async ({ page }) => {
    const c = await center(page);
    const fitted = await boardGeometry(page);
    expect(isFittedCentered(fitted)).toBe(true);
    await page.keyboard.press("h");
    await drag(page, [[c.x, c.y], [c.x + 80, c.y + 40]]);
    const panned = await boardGeometry(page);
    expect(panned.offsetX - fitted.offsetX).toBeCloseTo(80, 0);
    expect(panned.offsetY - fitted.offsetY).toBeCloseTo(40, 0);
    expect((await ops(page)).length).toBe(0);
    await page.keyboard.press("Escape"); // leaves the hand tool
    await stroke(page, [-60, 0], [60, 0]);
    await head(page, 1);
    await page.keyboard.press("Control+0");
    expect(isFittedCentered(await settledGeometry(page))).toBe(true);
    await page.keyboard.press("Control+=");
    expect((await boardGeometry(page)).scale / fitted.scale).toBeCloseTo(1.3, 2);
    await page.keyboard.press("Meta+-");
    await page.keyboard.press("Meta+-");
    expect((await boardGeometry(page)).scale / fitted.scale).toBeCloseTo(1 / 1.3, 2);
    await page.keyboard.press("Control+0");
    expect(isFittedCentered(await settledGeometry(page))).toBe(true);
    // Zoom keys are ignored while a field has focus.
    await page.locator("#desktop-ink-page").focus();
    await page.keyboard.press("Control+=");
    expect((await boardGeometry(page)).scale).toBeCloseTo(fitted.scale, 4);
  });

  test("editing shortcuts are disabled on a read-only snapshot; viewing shortcuts still work", async ({ page }) => {
    const c = await center(page);
    await stroke(page, [-60, 0], [60, 0]);
    await head(page, 1);
    await page.getByRole("button", { name: "保存快照" }).click(); await page.locator("#ink-dialog [data-choice=ok]").click();
    await page.locator("#ink-snapshot-list .ink-session-item").click();
    await expect(page.locator("#ink-editability-badge")).toHaveText("只读");
    await page.mouse.click(c.x, c.y + 200);
    const before = await page.evaluate(() => [...window.inkDesktopHarness.storedDocuments.values()].map(document => document.operations.length));
    await page.keyboard.press("Control+z");
    await page.keyboard.press("e");
    await drag(page, [[c.x, c.y - 30], [c.x, c.y + 30]]);
    await page.keyboard.press("l");
    await lassoRect(page, -110, -90, 110, 90);
    await page.keyboard.press("Delete");
    await page.waitForTimeout(150);
    expect(await page.evaluate(() => [...window.inkDesktopHarness.storedDocuments.values()].map(document => document.operations.length))).toEqual(before);
    const fitted = await boardGeometry(page);
    await page.keyboard.press("Control+=");
    expect((await boardGeometry(page)).scale / fitted.scale).toBeCloseTo(1.3, 2);
  });
});

test("M1 editing shortcuts are disabled while mirroring the tablet", async ({ page }) => {
  const c = await center(page);
  await stroke(page, [-60, 0], [60, 0]);
  await head(page, 1);
  await page.getByRole("button", { name: "平板书写", exact: true }).click();
  await page.getByRole("button", { name: "生成配对二维码", exact: true }).click();
  await expect(page.locator("#ink-editability-badge")).toHaveText("电脑只读");
  await page.mouse.click(c.x + 150, c.y + 150);
  const before = await page.evaluate(() => [...window.inkDesktopHarness.storedDocuments.values()].map(document => document.operations.length));
  await page.keyboard.press("Control+z");
  await page.keyboard.press("Delete");
  await page.keyboard.press("e");
  await drag(page, [[c.x, c.y - 30], [c.x, c.y + 30]]);
  await page.waitForTimeout(150);
  expect(await page.evaluate(() => [...window.inkDesktopHarness.storedDocuments.values()].map(document => document.operations.length))).toEqual(before);
  expect(await visibleCount(page)).toBe(1);
});

test.describe("H5 leaving with unsaved operations", () => {
  test("返回地图 waits for the save and asks while operations are still pending", async ({ page }) => {
    const host = page.locator("#ink-test-host");
    await page.evaluate(() => { window.inkDesktopHarness.behavior.failAppend = true; });
    await stroke(page, [-60, 0], [60, 0]);
    await page.locator("#ink-close").click();
    const dialog = page.locator("#ink-dialog");
    await expect(dialog).toBeVisible();
    await expect(page.locator("#ink-dialog-text")).toContainText("1 个手稿操作尚未保存");
    await expect(page.locator("#ink-dialog-text")).toContainText("恢复队列");
    // Keep editing: still open, stroke still on the board.
    await dialog.getByRole("button", { name: "继续编辑", exact: true }).click();
    await expect(dialog).toBeHidden();
    await expect(host).toBeVisible();
    expect(await paintedBounds(page, "settled")).not.toBeNull();
    // Esc on the prompt also keeps editing.
    await page.locator("#ink-close").click();
    await expect(dialog).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(host).toBeVisible();
    // Retry after the store recovers: saves, then closes without asking again.
    await page.locator("#ink-close").click();
    await expect(dialog).toBeVisible();
    await page.evaluate(() => { window.inkDesktopHarness.behavior.failAppend = false; });
    await dialog.getByRole("button", { name: "重试保存", exact: true }).click();
    await expect(host).toBeHidden();
    await expect(dialog).toBeHidden();
    expect((await ops(page)).length).toBe(1);
  });

  test("仍然离开 closes with the operations kept in the recovery queue; nothing pending closes at once", async ({ page }) => {
    const host = page.locator("#ink-test-host");
    await page.locator("#ink-close").click();
    await expect(host).toBeHidden();
    await expect(page.locator("#ink-dialog")).toBeHidden();
    await page.evaluate(() => { document.querySelector<HTMLElement>("#ink-test-host")!.hidden = false; });
    await page.evaluate(() => { window.inkDesktopHarness.behavior.failAppend = true; });
    await stroke(page, [-60, 0], [60, 0]);
    await page.locator("#ink-close").click();
    await page.locator("#ink-dialog").getByRole("button", { name: "仍然离开", exact: true }).click();
    await expect(host).toBeHidden();
    expect((await ops(page)).length).toBe(0);
  });
});
