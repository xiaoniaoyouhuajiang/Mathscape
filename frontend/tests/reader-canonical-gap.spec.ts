import { test, expect, type Page } from "@playwright/test";

// 「解析这几页」: a gap of the 「规范解析正文」 view offers the pages next to
// what is being read, and their text arrives in place.
const CANONICAL = "mathscape:canonical-markdown/v1/mathscape.marker.math-ocr.v1/marker-1/pdf";
const TITLE = "母函数 · 规范解析正文";

type Piece = { seg: string; pages: [number, number] } | { gap: [number, number] };

/** Pages are 1-based and inclusive, as on screen. */
async function setUp(page: Page, pieces: Piece[], totalPages: number): Promise<void> {
  await page.setViewportSize({ width: 1200, height: 800 });
  await page.goto("/tests/reader.html");
  await page.waitForFunction(() => "harness" in window);
  await page.evaluate(([canonicalId, total]) => {
    const h = (window as any).harness;
    const enc = new TextEncoder();
    const dashes = "-".repeat(48);
    const built = new Map<string, any>();
    const segment = (id: string, from: number, to: number) => {
      if (!built.has(id)) {
        const parts: { text: string; kind: string }[] = [];
        for (let pageNumber = from; pageNumber <= to; pageNumber++) {
          parts.push({ text: `{${pageNumber - 1}}${dashes}\n\n`, kind: "page_marker" });
          for (let i = 0; i < 6; i++) {
            parts.push({ text: `${id} 第 ${pageNumber} 页 第 ${i + 1} 段：` + "母函数把数列的性质变成幂级数的运算。".repeat(5) + "\n\n", kind: "text" });
          }
        }
        let offset = 0;
        const blocks = parts.map((part, index) => {
          const length = enc.encode(part.text).length;
          const block = { id: `${id}-${index}`, kind: part.kind, range: { start: offset, end: offset + length } };
          offset += length;
          return block;
        });
        const source = parts.map(part => part.text).join("");
        h.setMarkdownSnapshot(id, {
          snapshot: { artifact_id: id, conversion_id: `conv-${id}`, content_hash: `hash-${id}`, anchor_scheme: "mathscape-markdown-blocks-1", fingerprint: `fp-${id}` },
          source,
          blocks,
        });
        built.set(id, { source, blocks });
      }
      const doc = built.get(id);
      return {
        kind: "segment", conversion_id: `conv-${id}`, artifact_id: id,
        snapshot_pages: { from, to }, pages: { from, to }, overlap: null, overlap_trimmed: false,
        is_head: false, byte_start: 0, content_hash: `hash-${id}`, blocks: doc.blocks, markdown: doc.source,
      };
    };
    const project = (pieces: any[]) => {
      const gaps = pieces.filter(piece => "gap" in piece).map(piece => ({ from: piece.gap[0], to: piece.gap[1] }));
      const covered = pieces.filter(piece => "seg" in piece).map(piece => ({ from: piece.pages[0], to: piece.pages[1] }));
      h.setCanonicalProjection({
        canonical_id: canonicalId,
        source_pdf_artifact_id: "pdf",
        head_artifact_id: null,
        total_pages: total,
        coverage: { covered, missing: gaps, head_covered: [] },
        skipped: [],
        pieces: pieces.map(piece => "gap" in piece
          ? { kind: "gap", pages: { from: piece.gap[0], to: piece.gap[1] } }
          : segment(piece.seg, piece.pages[0], piece.pages[1])),
      });
    };
    // In the app the reader panel is the scroll container.
    const host = document.querySelector<HTMLElement>("#reader")!;
    // The desktop webview (WebKit) has no scroll anchoring; do not rely on Chromium's.
    host.style.cssText = "height: 620px; overflow: auto; position: relative; overflow-anchor: none;";
    (window as any).fixture = {
      project,
      top: (selector: string) =>
        document.querySelector<HTMLElement>(selector)!.getBoundingClientRect().top - host.getBoundingClientRect().top,
      report: (status: string, from: number, to: number, extra: object = {}) => ({
        status, requested: { from, to }, missing: [{ from, to }],
        marker_ranges: status === "expanded" ? [{ from, to }] : [], waited_ms: 1, message: "", ...extra,
      }),
      lease: (from: number, to: number, agoMs: number, pdf = "pdf") => ({
        ensure_id: `ensure-${from}`, source_pdf_artifact_id: pdf, canonical_id: canonicalId,
        pages: { from, to }, started_at_ms: Date.now() - agoMs, lease_expires_at_ms: Date.now() + 60_000,
      }),
      // A parse that ends when the test says so.
      hold: () => {
        h.gapParse.respond = () => new Promise(resolve => { (window as any).fixture.finish = resolve; });
      },
    };
  }, [CANONICAL, totalPages] as const);
  await project(page, pieces);
}

const project = (page: Page, pieces: Piece[]) => page.evaluate(p => (window as any).fixture.project(p), pieces);
const open = (page: Page, options: object = {}) =>
  page.evaluate(([i, t, o]) => (window as any).harness.reader.open(i, t, o), [CANONICAL, TITLE, options] as const);
const top = (page: Page, selector: string) => page.evaluate(s => (window as any).fixture.top(s), selector);
const requests = (page: Page) => page.evaluate(() => (window as any).harness.gapParse.requests);

const BOOK: Piece[] = [
  { seg: "sa", pages: [1, 2] },
  { gap: [3, 12] },
  { seg: "sc", pages: [13, 14] },
  { gap: [15, 16] },
];

test("a large gap offers the pages at each end, a small one the whole gap", async ({ page }) => {
  await setUp(page, BOOK, 16);
  await open(page);
  const big = page.locator('.canonical-gap[data-pages="3-12"]');
  await expect(big.locator(".canonical-gap-label")).toHaveText("第 3–12 页尚未解析（此处没有 Markdown，不补写内容）");
  const head = big.locator('.canonical-gap-actions[data-end="head"]');
  await expect(head.locator(".canonical-gap-caption")).toHaveText("接着上文");
  await expect(head.getByRole("button", { name: "解析第 3–6 页" })).toBeEnabled();
  await expect(head.getByRole("button", { name: "先看 PDF 第 3 页" })).toBeVisible();
  const tail = big.locator('.canonical-gap-actions[data-end="tail"]');
  await expect(tail.locator(".canonical-gap-caption")).toHaveText("紧邻下文");
  await expect(tail.getByRole("button", { name: "解析第 9–12 页" })).toBeEnabled();
  await expect(tail.getByRole("button", { name: "先看 PDF 第 12 页" })).toBeVisible();
  // The head row sits above the label, the tail row below it: each next to the text it continues.
  expect(await top(page, '.canonical-gap[data-pages="3-12"] [data-end="head"]'))
    .toBeLessThan(await top(page, '.canonical-gap[data-pages="3-12"] .canonical-gap-label'));
  expect(await top(page, '.canonical-gap[data-pages="3-12"] [data-end="tail"]'))
    .toBeGreaterThan(await top(page, '.canonical-gap[data-pages="3-12"] .canonical-gap-label'));

  const small = page.locator('.canonical-gap[data-pages="15-16"]');
  await expect(small.locator(".canonical-gap-actions")).toHaveCount(1);
  await expect(small.getByRole("button", { name: "解析第 15–16 页" })).toBeEnabled();
  // Nothing follows the last gap: no claim that it continues anything.
  await expect(small.locator(".canonical-gap-caption")).toHaveCount(0);
});

test("parsed pages arrive where the gap was, and what is on screen does not move or get redrawn", async ({ page }) => {
  await setUp(page, BOOK, 16);
  await open(page);
  // The reader scrolled up from page 13 and met the end of the gap.
  const firstBelow = '[data-source-block="sc-1"]';
  await page.evaluate((selector) => {
    const host = document.querySelector<HTMLElement>("#reader")!;
    host.scrollTop += document.querySelector<HTMLElement>(selector)!.getBoundingClientRect().top - host.getBoundingClientRect().top - 300;
    // Marks the nodes: a redraw would replace them.
    for (const block of document.querySelectorAll<HTMLElement>("[data-source-block]")) (block as any).kept = true;
  }, firstBelow);
  await page.locator(firstBelow).click();
  await expect(page.locator(firstBelow)).toHaveClass(/is-current-block/);
  const before = await top(page, firstBelow);
  expect(Math.abs(before - 300)).toBeLessThanOrEqual(1);

  await page.evaluate(() => (window as any).fixture.hold());
  const big = page.locator('.canonical-gap[data-pages="3-12"]');
  await big.getByRole("button", { name: "解析第 9–12 页" }).click();
  expect(await requests(page)).toEqual([{ canonical_id: CANONICAL, first_page: 9, last_page: 12 }]);
  await expect(big).toHaveAttribute("data-state", "starting");
  await expect(big.locator(".canonical-gap-status")).toHaveText("正在准备解析第 9–12 页…");
  // One parse at a time for this view; the PDF pages stay one click away.
  await expect(big.getByRole("button", { name: "解析第 3–6 页" })).toBeDisabled();
  await expect(page.locator('.canonical-gap[data-pages="15-16"]').getByRole("button", { name: "解析第 15–16 页" })).toBeDisabled();
  await expect(big.getByRole("button", { name: "先看 PDF 第 12 页" })).toBeEnabled();

  // Marker took the lease 65 s ago (the reader came back to the document).
  await page.evaluate(() => {
    const w = window as any;
    w.harness.gapParse.leases = [w.fixture.lease(9, 12, 65_000)];
  });
  await expect(big).toHaveAttribute("data-state", "running", { timeout: 6000 });
  await expect(big.locator(".canonical-gap-status")).toContainText("正在解析第 9–12 页 · 已用时 1:0");
  await expect(big.locator(".canonical-gap-status")).toContainText("通常每页约半分钟");
  const shown = await big.locator(".canonical-gap-elapsed").textContent();
  await expect(big.locator(".canonical-gap-elapsed")).not.toHaveText(shown!, { timeout: 3000 });
  // The reader keeps reading meanwhile.
  expect(await top(page, firstBelow)).toBeCloseTo(before, 0);

  await project(page, [
    { seg: "sa", pages: [1, 2] },
    { gap: [3, 8] },
    { seg: "sn", pages: [9, 12] },
    { seg: "sc", pages: [13, 14] },
    { gap: [15, 16] },
  ]);
  await page.evaluate(() => {
    const w = window as any;
    w.harness.gapParse.leases = [];
    w.fixture.finish(w.fixture.report("expanded", 9, 12));
  });
  const parsed = page.locator('.canonical-segment[data-artifact-id="sn"]');
  await expect(parsed).toBeVisible();
  await expect(parsed.locator(".canonical-segment-header")).toContainText("第 9–12 页");
  await expect(page.locator('.canonical-gap[data-pages="3-12"]')).toHaveCount(0);
  const rest = page.locator('.canonical-gap[data-pages="3-8"]');
  await expect(rest.getByRole("button", { name: "解析第 5–8 页" })).toBeEnabled();
  await expect(page.locator(".reader-controls").first()).toContainText("由 3 个解析快照按页序拼接");
  await expect(page.locator(".reader-controls").first()).toContainText("8 页尚未解析");

  // Page order: 1–2, the rest of the gap, the new pages, then what was being read.
  const order = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>(".canonical-reader > [data-pages]")].map(piece => piece.dataset.pages));
  expect(order).toEqual(["1-2", "3-8", "9-12", "13-14", "15-16"]);
  // The block being read is where it was, selected as before, and is the same node.
  expect(Math.abs(await top(page, firstBelow) - before)).toBeLessThanOrEqual(1);
  await expect(page.locator(firstBelow)).toHaveClass(/is-current-block/);
  const redrawn = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>('[data-segment-artifact-id="sa"], [data-segment-artifact-id="sc"]')]
      .filter(block => !(block as any).kept).length);
  expect(redrawn).toBe(0);
  // The new text sits right above it and works like any other: selectable, numbered in order.
  await expect(page.locator('[data-source-block="sn-27"]')).toBeVisible();
  await page.locator('[data-source-block="sn-27"]').click();
  await expect(page.locator('[data-source-block="sn-27"]')).toHaveClass(/is-current-block/);
  await expect(page.locator(firstBelow)).not.toHaveClass(/is-current-block/);
  await expect(page.locator('[data-source-block="sn-27"]')).toHaveAttribute("aria-label", "分段 2 原文块 28");
  await expect(page.locator(firstBelow)).toHaveAttribute("aria-label", "分段 3 原文块 2");
  const keys = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>("[data-canonical-key]")].map(block => block.dataset.canonicalKey));
  expect(new Set(keys).size).toBe(keys.length);
});

test("pages that continue the text above start where the gap card was", async ({ page }) => {
  await setUp(page, BOOK, 16);
  await open(page);
  const card = '.canonical-gap[data-pages="3-12"]';
  await page.evaluate((selector) => {
    const host = document.querySelector<HTMLElement>("#reader")!;
    host.scrollTop += document.querySelector<HTMLElement>(selector)!.getBoundingClientRect().top - host.getBoundingClientRect().top - 260;
  }, card);
  const lastAbove = '[data-source-block="sa-13"]';
  const aboveBefore = await top(page, lastAbove);
  const cardBefore = await top(page, card);

  await page.evaluate(() => (window as any).fixture.hold());
  await page.locator(card).getByRole("button", { name: "解析第 3–6 页" }).click();
  await expect(page.locator(card)).toHaveAttribute("data-state", "starting");
  expect(Math.abs(await top(page, card) - cardBefore)).toBeLessThanOrEqual(1);

  await project(page, [
    { seg: "sa", pages: [1, 2] },
    { seg: "sh", pages: [3, 6] },
    { gap: [7, 12] },
    { seg: "sc", pages: [13, 14] },
    { gap: [15, 16] },
  ]);
  await page.evaluate(() => {
    const w = window as any;
    w.fixture.finish(w.fixture.report("expanded", 3, 6));
  });
  const parsed = '.canonical-segment[data-artifact-id="sh"]';
  await expect(page.locator(parsed)).toBeVisible();
  expect(Math.abs(await top(page, parsed) - cardBefore)).toBeLessThanOrEqual(1);
  expect(Math.abs(await top(page, lastAbove) - aboveBefore)).toBeLessThanOrEqual(1);
  // The rest of the gap follows the new pages and can be parsed in turn.
  await expect(page.locator('.canonical-gap[data-pages="7-12"]').getByRole("button", { name: "解析第 7–10 页" })).toBeEnabled();
});

test("a failed parse says so on the card, keeps the text, and can be retried or closed", async ({ page }) => {
  await setUp(page, BOOK, 16);
  await open(page);
  await page.evaluate(() => {
    const w = window as any;
    w.harness.gapParse.respond = () => w.fixture.report("failed", 3, 6, {
      message: "Marker run Failed: Warning: You are sending unauthenticated requests to the HF Hub. Please set a HF_TOKEN\nTraceback (most recent call last):",
    });
  });
  const big = page.locator('.canonical-gap[data-pages="3-12"]');
  await big.getByRole("button", { name: "解析第 3–6 页" }).click();
  await expect(big).toHaveAttribute("data-state", "failed");
  await expect(big.locator(".canonical-gap-error")).toHaveText("解析第 3–6 页失败，正文没有变化。可以重试。");
  await expect(big.locator(".canonical-gap-hint")).toContainText("HF_HUB_OFFLINE=1");
  // The raw Marker output is one click away, not in the sentence.
  await expect(big.locator(".canonical-gap-detail pre")).toBeHidden();
  await big.locator(".canonical-gap-detail summary").click();
  await expect(big.locator(".canonical-gap-detail pre")).toContainText("Traceback");
  // Polling must not collapse what the reader opened.
  await page.waitForTimeout(3500);
  await expect(big.locator(".canonical-gap-detail pre")).toBeVisible();
  await expect(page.locator(".canonical-segment")).toHaveCount(2);

  await big.getByRole("button", { name: "重试" }).click();
  expect(await requests(page)).toHaveLength(2);
  await expect(big).toHaveAttribute("data-state", "failed");
  await big.getByRole("button", { name: "关闭" }).click();
  await expect(big).toHaveAttribute("data-state", "idle");
  await expect(big.getByRole("button", { name: "解析第 3–6 页" })).toBeEnabled();

  // No Marker on this machine: said plainly, without the HF hint.
  await page.evaluate(() => { (window as any).harness.gapParse.respond = null; });
  await big.getByRole("button", { name: "解析第 9–12 页" }).click();
  await expect(big.locator(".canonical-gap-error")).toHaveText("没有找到可用的 Marker，这几页没有解析。请确认本机已安装 Marker。");
  await expect(big.locator(".canonical-gap-hint")).toHaveCount(0);
});

test("pages an agent is parsing show as running and arrive without a click", async ({ page }) => {
  await setUp(page, BOOK, 16);
  await page.evaluate(() => {
    const w = window as any;
    w.harness.gapParse.leases = [w.fixture.lease(5, 6, 20_000)];
  });
  await open(page);
  const big = page.locator('.canonical-gap[data-pages="3-12"]');
  await expect(big).toHaveAttribute("data-state", "running");
  await expect(big.locator(".canonical-gap-status")).toContainText("第 5–6 页正在解析（由 agent 或其他窗口发起）");
  await expect(big.getByRole("button", { name: "解析第 3–6 页" })).toBeDisabled();
  // Another gap of the same book can still be asked for; it queues behind.
  const small = page.locator('.canonical-gap[data-pages="15-16"]');
  await page.evaluate(() => (window as any).fixture.hold());
  await small.getByRole("button", { name: "解析第 15–16 页" }).click();
  await expect(small).toHaveAttribute("data-state", "queued");
  await expect(small.locator(".canonical-gap-status")).toHaveText("排队中：这本书的第 5–6 页正在解析，结束后开始解析第 15–16 页");

  await project(page, [
    { seg: "sa", pages: [1, 2] },
    { gap: [3, 4] },
    { seg: "sg", pages: [5, 6] },
    { gap: [7, 12] },
    { seg: "sc", pages: [13, 14] },
    { gap: [15, 16] },
  ]);
  await page.evaluate(() => {
    const w = window as any;
    w.harness.gapParse.leases = [w.fixture.lease(15, 16, 0)];
  });
  await expect(page.locator('.canonical-segment[data-artifact-id="sg"]')).toBeVisible({ timeout: 8000 });
  await expect(page.locator('.canonical-gap[data-pages="3-4"]').getByRole("button", { name: "解析第 3–4 页" })).toBeDisabled();
  await expect(small).toHaveAttribute("data-state", "running");
  await expect(small.locator(".canonical-gap-status")).toContainText("正在解析第 15–16 页 · 已用时");
});

test("the PDF page is one click from the gap, and the way back returns to the gap", async ({ page }) => {
  // The sample PDF has two pages: page 2 is the one not parsed.
  await setUp(page, [{ seg: "sa", pages: [1, 1] }, { gap: [2, 2] }, { seg: "sc", pages: [3, 4] }], 4);
  await open(page);
  const gap = page.locator('.canonical-gap[data-pages="2-2"]');
  await page.evaluate(() => {
    const host = document.querySelector<HTMLElement>("#reader")!;
    host.scrollTop += document.querySelector<HTMLElement>(".canonical-gap")!.getBoundingClientRect().top - host.getBoundingClientRect().top - 240;
  });
  const below = '[data-source-block="sc-0"]';
  const belowBefore = await top(page, below);
  await page.evaluate(() => (window as any).fixture.hold());
  await gap.getByRole("button", { name: "解析第 2 页" }).click();
  await expect(gap).toHaveAttribute("data-state", "starting");
  // The status line makes the card taller upwards: the text below it stays.
  expect(Math.abs(await top(page, below) - belowBefore)).toBeLessThanOrEqual(1);
  const before = await top(page, ".canonical-gap");
  await gap.getByRole("button", { name: "先看 PDF 第 2 页" }).click();
  await expect(page.getByLabel("PDF 页码")).toHaveValue("2");
  await page.getByRole("button", { name: "← 返回规范解析正文" }).click();
  // Back at the gap, at the height it had, with the parse still shown.
  await expect(gap).toHaveAttribute("data-state", "starting");
  await expect.poll(async () => Math.abs(await top(page, ".canonical-gap") - before)).toBeLessThanOrEqual(2);

  // It finishes while the reader is on the PDF again: coming back lands on the parsed page.
  await gap.getByRole("button", { name: "先看 PDF 第 2 页" }).click();
  await expect(page.getByLabel("PDF 页码")).toHaveValue("2");
  await project(page, [{ seg: "sa", pages: [1, 1] }, { seg: "sn", pages: [2, 2] }, { seg: "sc", pages: [3, 4] }]);
  await page.evaluate(() => {
    const w = window as any;
    w.fixture.finish(w.fixture.report("expanded", 2, 2));
  });
  await page.getByRole("button", { name: "← 返回规范解析正文" }).click();
  await expect(page.locator(".canonical-gap")).toHaveCount(0);
  await expect(page.locator('.canonical-segment[data-artifact-id="sn"]')).toBeVisible();
  await expect.poll(async () => Math.abs(await top(page, '.source-page-marker[data-pdf-page-index="1"]') - before)).toBeLessThanOrEqual(2);
});
