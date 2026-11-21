import { test, expect, type Page } from "@playwright/test";

// Snapshot reader ↔ 「规范解析正文」: a physical snapshot is one parse run's pages;
// reading on past it continues in the stitched view at the same block.
const CANONICAL = "mathscape:canonical-markdown/v1/mathscape.marker.math-ocr.v1/marker-1/pdf";
const TITLE = "母函数 · 规范解析正文";

/**
 * seg-a: PDF pages 1–2. seg-b: parsed as pages 2–4; page 2 overlaps seg-a and
 * is trimmed in the stitched view, which shows seg-b from page 3 on.
 */
async function setUp(page: Page, options: { resolve?: boolean } = {}): Promise<void> {
  await page.setViewportSize({ width: 1200, height: 800 });
  await page.goto("/tests/reader.html");
  await page.waitForFunction(() => "harness" in window);
  await page.evaluate(([canonicalId, title, resolve]) => {
    const h = (window as any).harness;
    const enc = new TextEncoder();
    const dashes = "-".repeat(48);
    const paragraph = (name: string) => `${name}：` + "母函数把数列的性质变成幂级数的运算。".repeat(6);
    const build = (prefix: string, pages: number[], perPage: number) => {
      const parts: { text: string; kind: string; page: number }[] = [];
      for (const pageIndex of pages) {
        parts.push({ text: `{${pageIndex}}${dashes}\n\n`, kind: "page_marker", page: pageIndex });
        for (let i = 0; i < perPage; i++) {
          parts.push({ text: paragraph(`${prefix} 第 ${pageIndex + 1} 页 第 ${i + 1} 段`) + "\n\n", kind: "text", page: pageIndex });
        }
      }
      let offset = 0;
      const blocks = parts.map((part, index) => {
        const length = enc.encode(part.text).length;
        const block = { id: `${prefix}-${index}`, kind: part.kind, range: { start: offset, end: offset + length } };
        offset += length;
        return block;
      });
      return { source: parts.map(part => part.text).join(""), blocks, parts };
    };
    const a = build("sa", [0, 1], 8);
    const b = build("sb", [1, 2, 3], 8);
    const snapshot = (id: string, doc: ReturnType<typeof build>) => ({
      snapshot: { artifact_id: id, conversion_id: `conv-${id}`, content_hash: `hash-${id}`, anchor_scheme: "mathscape-markdown-blocks-1", fingerprint: `fp-${id}` },
      source: doc.source,
      blocks: doc.blocks,
    });
    h.setMarkdownSnapshot("seg-a", snapshot("seg-a", a));
    h.setMarkdownSnapshot("seg-b", snapshot("seg-b", b));
    // seg-b is shown from its page-3 marker on.
    const firstShown = b.parts.findIndex(part => part.page === 2);
    const byteStart = b.blocks[firstShown].range.start;
    h.setCanonicalProjection({
      canonical_id: canonicalId,
      source_pdf_artifact_id: "pdf",
      head_artifact_id: "seg-b",
      total_pages: 6,
      coverage: { covered: [{ from: 1, to: 4 }], missing: [{ from: 5, to: 6 }], head_covered: [{ from: 2, to: 4 }] },
      skipped: [],
      pieces: [
        {
          kind: "segment", conversion_id: "conv-seg-a", artifact_id: "seg-a",
          snapshot_pages: { from: 1, to: 2 }, pages: { from: 1, to: 2 }, overlap: null, overlap_trimmed: false,
          is_head: false, byte_start: 0, content_hash: "hash-seg-a", blocks: a.blocks, markdown: a.source,
        },
        {
          kind: "segment", conversion_id: "conv-seg-b", artifact_id: "seg-b",
          snapshot_pages: { from: 2, to: 4 }, pages: { from: 3, to: 4 }, overlap: { from: 2, to: 2 }, overlap_trimmed: true,
          is_head: true, byte_start: byteStart, content_hash: "hash-seg-b",
          blocks: b.blocks.filter((block: any) => block.range.end > byteStart),
          markdown: new TextDecoder().decode(enc.encode(b.source).subarray(byteStart)),
        },
        { kind: "gap", pages: { from: 5, to: 6 } },
      ],
    });
    // In the app the reader panel is the scroll container.
    const host = document.querySelector<HTMLElement>("#reader")!;
    host.style.cssText = "height: 620px; overflow: auto; position: relative;";
    h.reader.setCanonicalResolver((artifactId: string) =>
      resolve && (artifactId === "seg-a" || artifactId === "seg-b") ? { id: canonicalId, title } : undefined);
    (window as any).fixture = {
      block: (id: string) => {
        const block = [...a.blocks, ...b.blocks].find(item => item.id === id)!;
        return { start: block.range.start, end: block.range.end, blockIds: [id] };
      },
      // Top of the element relative to the top of the reader panel.
      top: (selector: string) =>
        document.querySelector<HTMLElement>(selector)!.getBoundingClientRect().top - host.getBoundingClientRect().top,
    };
  }, [CANONICAL, TITLE, options.resolve ?? true] as const);
}

const open = (page: Page, id: string, title: string, options: object) =>
  page.evaluate(([i, t, o]) => (window as any).harness.reader.open(i, t, o), [id, title, options] as const);
const top = (page: Page, selector: string) => page.evaluate(s => (window as any).fixture.top(s), selector);
const focusOf = (page: Page, id: string) => page.evaluate(i => (window as any).fixture.block(i), id);

test("locating a source block opens the stitched view at that block, with a way back to it", async ({ page }) => {
  await setUp(page);
  // sb-20 sits on PDF page 4 of seg-b, which the stitched view shows.
  const focus = { ...(await focusOf(page, "sb-20")), label: "定理 4：三类组合问题的母函数" };
  expect(await open(page, "seg-b", "定理 4：三类组合问题的母函数", { focus, preferCanonical: true })).toBe(true);

  await expect(page.locator("#reader h2")).toHaveText(TITLE);
  const located = page.locator(".canonical-reader [data-source-block='sb-20']");
  await expect(located).toHaveClass(/source-block-focus/);
  await expect(located).toBeInViewport();
  await expect(page.locator(".canonical-reader .source-block-focus")).toHaveCount(1);
  // Earlier pages from the other snapshot are in the same document, above.
  await expect(page.locator(".canonical-reader [data-source-block='sa-1']")).toHaveCount(1);

  const chip = page.locator(".reader-return-point");
  await expect(chip).toBeHidden();
  await page.locator("#reader").evaluate(host => { host.scrollTop = 0; });
  await expect(chip).toBeVisible();
  // Pinned to the bottom of the reader, not left at the end of the document.
  await expect(chip).toBeInViewport();
  await expect(chip).toContainText("回到出发处 · 定理 4：三类组合问题的母函数");
  await expect(located).not.toBeInViewport();
  await chip.locator(".reader-return-btn").click();
  await expect(located).toBeInViewport();
  await expect(chip).toBeHidden();
});

test("discovery review and snapshots outside a stitched view keep the physical snapshot", async ({ page }) => {
  await setUp(page, { resolve: false });
  expect(await open(page, "seg-b", "解析快照", { focus: await focusOf(page, "sb-20"), preferCanonical: true })).toBe(true);
  await expect(page.locator(".canonical-reader")).toHaveCount(0);
  await expect(page.locator("[data-source-block='sb-20']")).toHaveClass(/source-block-focus/);
  await expect(page.locator(".reader-canonical-entry")).toHaveCount(0);
  await expect(page.locator(".reader-boundary-card")).toHaveCount(0);

  // A patch review is a physical-snapshot workflow even when the snapshot is stitched.
  await page.evaluate(([canonicalId, title]) => {
    (window as any).harness.reader.setCanonicalResolver(() => ({ id: canonicalId, title }));
  }, [CANONICAL, TITLE] as const);
  expect(await open(page, "md", "数学笔记.md", {
    patch: { runId: "mathscape:discovery-run/browser-fixture", mode: "workflow" },
    preferCanonical: true,
  })).toBe(true);
  await expect(page.locator(".canonical-reader")).toHaveCount(0);
  await expect(page.locator("[data-source-block='test-block-0']")).toBeVisible();
  await expect(page.locator(".reader-canonical-entry")).toHaveCount(0);
});

test("the snapshot reader continues in the stitched view without moving the block being read", async ({ page }) => {
  await setUp(page);
  expect(await open(page, "seg-b", "母函数 · 解析快照", {})).toBe(true);
  await expect(page.locator(".canonical-reader")).toHaveCount(0);
  const entry = page.getByRole("button", { name: "在规范解析正文中继续阅读" });
  await expect(entry).toBeVisible();
  await expect(page.locator(".reader-boundary-card.is-start")).toContainText("本快照从 PDF 第 2 页开始");
  await expect(page.locator(".reader-boundary-card.is-end")).toContainText("本快照到 PDF 第 4 页为止");

  // Read down to a block on page 4, leaving it partly above the top edge.
  await page.locator("#reader").evaluate((host) => {
    const block = host.querySelector<HTMLElement>("[data-source-block='sb-21']")!;
    host.scrollTop += block.getBoundingClientRect().top - host.getBoundingClientRect().top + 18;
  });
  const before = await top(page, "[data-source-block='sb-21']");
  expect(Math.abs(before + 18)).toBeLessThanOrEqual(1);
  // The toolbar scrolled away with the page; the block's own menu has the entry too.
  await page.locator("[data-source-block='sb-22']").click({ button: "right" });
  await page.getByText("在规范解析正文中从这里继续阅读").click();
  await expect(page.locator(".canonical-reader [data-source-block='sb-22']")).toHaveClass(/source-block-focus/);
  await expect(page.locator("#reader h2")).toHaveText(TITLE);
  const after = await top(page, ".canonical-reader [data-source-block='sb-21']");
  expect(Math.abs(after - before)).toBeLessThanOrEqual(1);
});

test("the toolbar entry uses the first block on screen", async ({ page }) => {
  await setUp(page);
  await open(page, "seg-b", "母函数 · 解析快照", {});
  await expect(page.locator("[data-source-block='sb-21']")).toBeVisible();
  await page.locator("#reader").evaluate((host) => {
    const block = host.querySelector<HTMLElement>("[data-source-block='sb-21']")!;
    host.scrollTop += block.getBoundingClientRect().top - host.getBoundingClientRect().top - 40;
  });
  const before = await top(page, "[data-source-block='sb-21']");
  expect(Math.abs(before - 40)).toBeLessThanOrEqual(1);
  // The toolbar is above the fold now; trigger it the way a click would.
  await page.locator(".reader-canonical-entry").evaluate((button: HTMLButtonElement) => button.click());
  const located = page.locator(".canonical-reader .source-block-focus");
  await expect(located).toHaveCount(1);
  // sb-20 is the block cut by the top edge; it stays where it was.
  await expect(located).toHaveAttribute("data-source-block", "sb-20");
  const after = await top(page, ".canonical-reader [data-source-block='sb-21']");
  expect(Math.abs(after - before)).toBeLessThanOrEqual(1);
});

test("the boundary card lands on the PDF page when the stitched view shows it from another snapshot", async ({ page }) => {
  await setUp(page);
  await open(page, "seg-b", "母函数 · 解析快照", {});
  const card = page.locator(".reader-boundary-card.is-start");
  await expect(card).toBeVisible();
  // seg-b's first block (sb-1) is on PDF page 2, which the stitched view shows from seg-a.
  const before = await top(page, "[data-source-block='sb-1']");
  await card.getByRole("button", { name: "继续向上阅读" }).click();

  await expect(page.locator(".canonical-reader")).toBeVisible();
  await expect(page.locator(".canonical-reader [data-source-block='sb-1']")).toHaveCount(0);
  await expect(page.locator(".canonical-reader .source-block-focus")).toHaveCount(0);
  const hint = page.locator(".canonical-status-hint");
  await expect(hint).toContainText("PDF 第 2 页在规范解析正文中由另一个解析快照显示");
  const after = await top(page, ".canonical-reader .source-page-marker[data-pdf-page-index='1']");
  expect(Math.abs(after - before)).toBeLessThanOrEqual(1);
  // Earlier pages are above now: the reader can keep scrolling up.
  expect(await page.locator("#reader").evaluate(host => host.scrollTop)).toBeGreaterThan(0);

  await hint.getByRole("button", { name: "在原快照中查看这一块" }).click();
  await expect(page.locator(".canonical-reader")).toHaveCount(0);
  await expect(page.locator("[data-source-block='sb-1']")).toHaveClass(/source-block-focus/);
});

test("a position the stitched view cannot show stays in the snapshot with a notice", async ({ page }) => {
  await setUp(page);
  await page.evaluate(() => { (window as any).harness.behavior.markdownOnlyReason = "旧转换未启用分页输出"; });
  // sb-1 is trimmed from the stitched view and its PDF page cannot be resolved.
  expect(await open(page, "seg-b", "定理 2", { focus: await focusOf(page, "sb-1"), preferCanonical: true })).toBe(true);
  await expect(page.locator(".canonical-reader")).toHaveCount(0);
  await expect(page.locator("#reader h2")).toHaveText("定理 2");
  await expect(page.locator("[data-source-block='sb-1']")).toHaveClass(/source-block-focus/);
  await expect(page.locator(".canonical-status-hint")).toContainText("已保留在解析快照中");
  expect(await page.evaluate(() => (window as any).harness.reader.lastOpened()?.id)).toBe("seg-b");
});
