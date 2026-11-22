import { expect, test, type Page } from "@playwright/test";

// N2 reader integration (proposal §6.4, §6.5): block-level targets only.
const link = (documentId: string, title: string, blocks: string[], primary = true) => ({
  note: { document_id: documentId, title, title_mode: "auto", title_revision: 0, lifecycle: "active", lifecycle_revision: 0, trashed_at: null, last_used_at: 1, created_at: "unix:1" },
  link: {
    link_id: `link-${documentId}-${blocks.join("-")}`, document_id: documentId, target_kind: "markdown", target_key: "md:x", target_id: "md",
    target: { kind: "markdown", artifact_id: "md", content_hash: "test-hash", first_block: blocks[0], last_block: blocks.at(-1), block_ids: blocks, start: 0, end: 1, block_hash: "h", excerpt: "x", label: "x" },
    target_revision: "test-hash", target_label: "x", role: "attempt", is_primary: primary, state: "active", created_at: 1, removed_at: null, target_status: "active",
  },
});

async function openMarkdown(page: Page, setup?: (ink: any) => void): Promise<void> {
  await page.goto("/tests/reader.html");
  await page.waitForFunction(() => "harness" in window);
  if (setup) await page.evaluate(`(${setup.toString()})(window.harness.ink)`);
  await page.evaluate(async () => (window as any).harness.reader.open("md", "数学笔记"));
  await expect(page.getByLabel("原文块 2", { exact: true })).toBeVisible();
}

test("N2 block markers count notes per block and open the related list", async ({ page }) => {
  await page.goto("/tests/reader.html");
  await page.waitForFunction(() => "harness" in window);
  await page.evaluate(async ([a, b, c]) => {
    (window as any).harness.ink.links = [a, b, c];
    await (window as any).harness.reader.open("md", "数学笔记");
  }, [link("note-1", "尝试 · Inline", ["test-block-1", "test-block-2"]), link("note-2", "验算 · Inline", ["test-block-1"]), link("note-1", "尝试 · Inline", ["test-block-1"], false)]);
  const block2 = page.getByLabel("原文块 2", { exact: true });
  await expect(block2.locator(".ink-block-marker")).toHaveText("✎ 2");
  await expect(page.getByLabel("原文块 3", { exact: true }).locator(".ink-block-marker")).toHaveText("✎ 1");
  await expect(page.getByLabel("原文块 1", { exact: true }).locator(".ink-block-marker")).toHaveCount(0);
  await block2.locator(".ink-block-marker").click();
  const related = page.locator(".ink-related-popover");
  await expect(related.getByRole("listitem")).toHaveCount(2);
  await related.getByRole("button", { name: "验算 · Inline" }).click();
  expect(await page.evaluate(() => (window as any).harness.ink.opened)).toEqual(["note-2"]);
});

test("N2 right-click menu targets the block under the cursor, or the selection when inside it", async ({ page }) => {
  await openMarkdown(page);
  await page.getByLabel("原文块 3", { exact: true }).click({ button: "right" });
  const menu = page.locator(".graph-context-menu");
  await expect(menu.getByRole("menuitem").or(menu.locator("button"))).toHaveText(["复制此块 Markdown", "校对此块 OCR 正文…", "定位对应 PDF 原页", "新建关联手稿…", "关联到已有手稿…", "查看相关手稿（0）"]);
  await expect(page.getByLabel("原文块 3", { exact: true })).toHaveClass(/is-ink-menu-target/);
  await menu.locator("button", { hasText: "新建关联手稿…" }).click();
  const popover = page.locator(".ink-create-popover");
  await expect(popover).toContainText("关联到");
  await popover.getByRole("button", { name: "在电脑上书写" }).click();
  await expect(popover).toHaveCount(0);
  let creates = await page.evaluate(() => (window as any).harness.ink.creates);
  expect(creates[0].target).toEqual({ kind: "markdown", artifact_id: "md", content_hash: "test-hash", first_block: "test-block-2", last_block: "test-block-2" });
  expect(creates[0].choice).toMatchObject({ role: "attempt", mode: "desktop" });
  await expect(page.locator(".is-ink-menu-target")).toHaveCount(0);

  // A selection of blocks 2..4; right-clicking inside uses the whole selection.
  await page.getByLabel("原文块 2", { exact: true }).click();
  await page.getByRole("button", { name: "设置起点" }).click();
  await page.getByLabel("原文块 4", { exact: true }).click();
  await page.getByRole("button", { name: "设置终点" }).click();
  await page.getByLabel("原文块 3", { exact: true }).click({ button: "right" });
  await expect(page.locator(".is-ink-menu-target")).toHaveCount(3);
  await page.locator(".graph-context-menu button", { hasText: "新建关联手稿…" }).click();
  await page.locator(".ink-create-popover").getByRole("combobox", { name: "用途" }).selectOption({ label: "验算" });
  await page.locator(".ink-create-popover").getByRole("button", { name: "用平板书写" }).click();
  creates = await page.evaluate(() => (window as any).harness.ink.creates);
  expect(creates[1].target).toMatchObject({ first_block: "test-block-1", last_block: "test-block-3" });
  expect(creates[1].choice).toMatchObject({ role: "check", mode: "tablet" });

  // Outside the selection: the block under the cursor.
  await page.getByLabel("原文块 6", { exact: true }).click({ button: "right" });
  await expect(page.locator(".is-ink-menu-target")).toHaveCount(1);
  await expect(page.getByLabel("原文块 6", { exact: true })).toHaveClass(/is-ink-menu-target/);
  await page.keyboard.press("Escape");
});

test("N2 手写这段 uses the selection panel range", async ({ page }) => {
  await openMarkdown(page);
  await page.getByLabel("原文块 2", { exact: true }).click();
  await page.getByRole("button", { name: "设置起点" }).click();
  await page.getByLabel("原文块 3", { exact: true }).click();
  await page.getByRole("button", { name: "设置终点" }).click();
  await page.getByRole("button", { name: "手写这段" }).click();
  await page.locator(".ink-create-popover").getByRole("button", { name: "在电脑上书写" }).click();
  const creates = await page.evaluate(() => (window as any).harness.ink.creates);
  expect(creates[0].target).toMatchObject({ first_block: "test-block-1", last_block: "test-block-2" });
});

test("N2 no ink menu on page-boundary blocks or in raw mode", async ({ page }) => {
  await openMarkdown(page, ink => { ink.pageMarkerBlock = 4; });
  const marker = page.locator(".source-page-marker");
  await expect(marker).toHaveCount(1);
  await marker.click({ button: "right" });
  await expect(page.locator(".graph-context-menu")).toBeHidden();
  await page.getByRole("button", { name: "查看原文" }).click();
  await page.locator(".reader-content pre, article pre").first().click({ button: "right" });
  await expect(page.locator(".graph-context-menu")).toBeHidden();
});

test("N2 link to an existing note lists active notes by last use and sends one request", async ({ page }) => {
  await openMarkdown(page);
  await page.getByLabel("原文块 2", { exact: true }).click({ button: "right" });
  await page.locator(".graph-context-menu button", { hasText: "关联到已有手稿…" }).click();
  const popover = page.locator(".ink-existing-popover");
  await expect(popover.getByRole("listbox", { name: "已有手稿" }).getByRole("option")).toHaveText(["旧手稿"]);
  await popover.getByRole("option", { name: "旧手稿" }).click();
  await expect(popover.getByRole("checkbox", { name: "设为主关联" })).toBeChecked();
  await popover.getByRole("button", { name: "关联", exact: true }).click();
  await expect(popover).toContainText("已关联到「旧手稿」");
  const requests = await page.evaluate(() => (window as any).harness.ink.linkRequests);
  expect(requests).toHaveLength(1);
  expect(requests[0]).toMatchObject({ document_id: "note-old", role: "attempt", primary: true, target: { kind: "markdown", first_block: "test-block-1" } });
});

test("N2 a patch-derived document shows only the original-version hint", async ({ page }) => {
  await openMarkdown(page, ink => { ink.originalNoteCount = 2; });
  await expect(page.locator(".ink-patch-hint")).toHaveText("原版本有 2 份相关手稿");
  await expect(page.locator(".ink-block-marker")).toHaveCount(0);
});

test("Markdown block PDF backlink jumps to exact PDF page and round-trips back to focused block", async ({ page }) => {
  await openMarkdown(page);
  await page.getByLabel("原文块 3", { exact: true }).click({ button: "right" });
  await page.locator(".graph-context-menu button", { hasText: "定位对应 PDF 原页" }).click();

  const pdfPageInput = page.getByLabel("PDF 页码");
  await expect(pdfPageInput).toHaveValue("2");
  const backButton = page.getByRole("button", { name: "← 返回 Markdown 原文" });
  await expect(backButton).toBeVisible();

  await backButton.click();
  await expect(page.getByLabel("原文块 3", { exact: true })).toHaveClass(/source-block-focus/);

  // Top-bar "定位 PDF 原页" stays enabled for the focused block and surfaces markdown-only reasons cleanly
  await page.evaluate(() => { (window as any).harness.behavior.markdownOnlyReason = "旧转换未启用分页输出"; });
  await page.getByRole("button", { name: "定位 PDF 原页" }).click();
  await expect(page.locator(".selection-panel [role='status']")).toContainText("旧转换未启用分页输出");
});

test("Shift+click block range, floating block action pill, Escape clear, and discovery drawer toggle", async ({ page }) => {
  await openMarkdown(page);
  await expect(page.locator(".block-action-pill")).toHaveCount(0);

  // Plain click selects single block and shows floating pill
  await page.getByLabel("原文块 2", { exact: true }).click();
  const pill = page.locator(".block-action-pill");
  await expect(pill).toBeVisible();
  await expect(pill.getByRole("button", { name: "✎ 手写此块" })).toBeVisible();
  await expect(pill.getByRole("button", { name: "＋ 关联已有" })).toBeVisible();
  await expect(pill.getByRole("button", { name: "↗ PDF 原页" })).toBeVisible();

  // Shift+click extends continuous block range without clicking top-bar endpoints
  await page.getByLabel("原文块 4", { exact: true }).click({ modifiers: ["Shift"] });
  await expect(page.locator(".is-selected-block")).toHaveCount(3);
  await expect(pill.getByRole("button", { name: "✎ 手写这 3 块" })).toBeVisible();

  await pill.getByRole("button", { name: "✎ 手写这 3 块" }).click();
  await page.locator(".ink-create-popover").getByRole("button", { name: "在电脑上书写" }).click();
  const creates = await page.evaluate(() => (window as any).harness.ink.creates);
  expect(creates[0].target).toMatchObject({ first_block: "test-block-1", last_block: "test-block-3" });

  // Plain click outside resets Shift range back to single block
  await page.getByLabel("原文块 6", { exact: true }).click();
  await expect(page.locator(".is-selected-block")).toHaveCount(0);
  await expect(pill.getByRole("button", { name: "✎ 手写此块" })).toBeVisible();

  // Escape clears block selection and dismisses the floating pill
  await page.keyboard.press("Escape");
  await expect(page.locator(".block-action-pill")).toHaveCount(0);

  // Discovery drawer toggle collapses/expands the secondary toolbar
  await page.getByRole("button", { name: "收起工具 ▴" }).click();
  await expect(page.locator(".selection-discovery-drawer")).toBeHidden();
  await page.getByRole("button", { name: "选区与发现 ▾" }).click();
  await expect(page.locator(".selection-discovery-drawer")).toBeVisible();
});

test("Canonical Markdown renders ✎ badges, TOC + ◈ map unit badges, direct block-ink creation, and cross-segment Shift clamp", async ({ page }) => {
  await page.setViewportSize({ width: 1360, height: 860 });
  await page.goto("/tests/reader.html");
  await page.waitForFunction(() => "harness" in window);

  const canonicalId = "mathscape:canonical-markdown/v1/mathscape.marker.math-ocr.v1/marker-1/pdf";
  await page.evaluate(async ([cid]) => {
    const enc = new TextEncoder();
    const segAMd = "# 第一章 母函数\n\n定理 1：$(1-x)^{-n}$ 的展开。\n\n证明：由二项式系数易得。\n";
    const segBMd = "## 第二节 不定方程\n\n定理 6：非负整数解个数。\n";
    const mkBlocks = (src: string, prefix: string) => {
      const parts = src.split("\n\n");
      let offset = 0;
      return parts.map((p, idx) => {
        const text = idx < parts.length - 1 ? p + "\n\n" : p;
        const len = enc.encode(text).length;
        const blk = { id: `${prefix}-b${idx}`, kind: idx === 0 ? "heading" : "text", range: { start: offset, end: offset + len } };
        offset += len;
        return blk;
      });
    };
    const h = (window as any).harness;
    h.ink.linksByArtifact["seg-a"] = [{
      note: { document_id: "note-canon-1", title: "验算 · 定理 1", title_mode: "auto", title_revision: 0, lifecycle: "active", lifecycle_revision: 0, trashed_at: null, last_used_at: 1, created_at: "unix:1" },
      link: {
        link_id: "l-canon-1", document_id: "note-canon-1", target_kind: "markdown", target_key: "md:seg-a", target_id: "seg-a",
        target: { kind: "markdown", artifact_id: "seg-a", content_hash: "hash-a", first_block: "sa-b1", last_block: "sa-b1", block_ids: ["sa-b1"], start: 0, end: 10, block_hash: "bh", excerpt: "定理 1", label: "定理 1" },
        target_revision: "hash-a", target_label: "定理 1", role: "check", is_primary: true, state: "active", created_at: 1, removed_at: null, target_status: "active",
      },
    }];
    h.unitMarkers.push(
      {
        unit_id: "mathscape:source-unit/thm1",
        title: "定理 1：(1-x)^(-n) 的展开",
        unit_kind: "statement",
        artifact_id: "seg-a",
        byte_range: { start: 20, end: 50 },
        block_ids: ["sa-b1"],
      },
      {
        unit_id: "mathscape:source-unit/thm6",
        title: "定理 6：非负整数解个数",
        unit_kind: "statement",
        artifact_id: "seg-b",
        byte_range: { start: 25, end: 55 },
        block_ids: ["sb-b1"],
      },
    );
    // Note linked ONLY to the SourceSemanticUnit (`source_object`) on the map — must still surface `✎ 1` on `sb-b1`!
    h.ink.unitLinksByArtifact["seg-b"] = [{
      note: { document_id: "note-unit-only", title: "无基础尝试 · 定理 6", title_mode: "auto", title_revision: 0, lifecycle: "active", lifecycle_revision: 0, trashed_at: null, last_used_at: 2, created_at: "unix:2" },
      link: {
        link_id: "l-unit-only", document_id: "note-unit-only", target_kind: "source_object", target_key: "src:thm6", target_id: "mathscape:source-unit/thm6",
        target: { kind: "source_object", object_id: "mathscape:source-unit/thm6" },
        target_revision: null, target_label: "定理 6", role: "attempt", is_primary: true, state: "active", created_at: 2, removed_at: null, target_status: "active",
      },
    }];
    h.setCanonicalProjection({
      canonical_id: cid,
      source_pdf_artifact_id: "pdf",
      head_artifact_id: "seg-b",
      total_pages: 4,
      coverage: { covered: [{ from: 1, to: 4 }], missing: [], head_covered: [{ from: 3, to: 4 }] },
      skipped: [],
      pieces: [
        {
          kind: "segment", conversion_id: "conv-a", artifact_id: "seg-a",
          snapshot_pages: { from: 1, to: 2 }, pages: { from: 1, to: 2 }, overlap: null, overlap_trimmed: false,
          is_head: false, byte_start: 0, content_hash: "hash-a", blocks: mkBlocks(segAMd, "sa"), markdown: segAMd,
        },
        {
          kind: "segment", conversion_id: "conv-b", artifact_id: "seg-b",
          snapshot_pages: { from: 3, to: 4 }, pages: { from: 3, to: 4 }, overlap: null, overlap_trimmed: false,
          is_head: true, byte_start: 0, content_hash: "hash-b", blocks: mkBlocks(segBMd, "sb"), markdown: segBMd,
        },
      ],
    });
    await h.reader.open(cid, "母函数 · 规范解析正文");
  }, [canonicalId]);

  // 1. TOC outline opens automatically on multi-item canonical view with headings + ◈ registered units
  const outline = page.getByRole("navigation", { name: "文档大纲目录" });
  await expect(outline).toBeVisible();
  await expect(outline).toContainText("第一章 母函数");
  await expect(outline).toContainText("◈ 定理 1：(1-x)^(-n) 的展开");
  await expect(outline).toContainText("第二节 不定方程");

  // 2. Registered unit badge on block jumps to map via locate-domain-object
  const mapBadge = page.locator(".unit-map-badge", { hasText: "◈ 定理 1" });
  await expect(mapBadge).toBeVisible();
  await mapBadge.click();
  expect(await page.evaluate(() => (window as any).harness.locatedObjects)).toEqual(["mathscape:source-unit/thm1"]);

  // 3. Existing physical-snapshot ink badge `✎ 1` AND map `source_object`-only unit ink badge `✎ 1` both render and open notes
  const thmBlock = page.locator("[data-canonical-key='0:sa-b1']");
  await expect(thmBlock.locator(".ink-block-marker")).toHaveText("✎ 1");
  const thm6Block = page.locator("[data-canonical-key='1:sb-b1']");
  await expect(thm6Block.locator(".ink-block-marker")).toHaveText("✎ 1");
  await thm6Block.locator(".ink-block-marker").click();
  await expect(page.locator(".ink-related-popover")).toContainText("无基础尝试 · 定理 6");
  await page.locator(".ink-related-popover").getByRole("button", { name: "无基础尝试 · 定理 6" }).click();
  expect(await page.evaluate(() => (window as any).harness.ink.opened)).toContain("note-unit-only");

  // 4. Intra-segment Shift+click selects theorem + proof and creates ink note anchored to physical snapshot `seg-a`
  await thmBlock.click();
  await page.locator("[data-canonical-key='0:sa-b2']").click({ modifiers: ["Shift"] });
  const pill = page.locator(".block-action-pill");
  await expect(pill.getByRole("button", { name: "✎ 手写这 2 块" })).toBeVisible();
  await pill.getByRole("button", { name: "✎ 手写这 2 块" }).click();
  await page.locator(".ink-create-popover").getByRole("button", { name: "在电脑上书写" }).click();
  const creates = await page.evaluate(() => (window as any).harness.ink.creates);
  expect(creates.at(-1).target).toEqual({
    kind: "markdown",
    artifact_id: "seg-a",
    content_hash: "hash-a",
    first_block: "sa-b1",
    last_block: "sa-b2",
  });

  // 5. Cross-segment Shift+click clamps to current segment boundary with an explicit status hint
  await thmBlock.click();
  await page.locator("[data-canonical-key='1:sb-b1']").click({ modifiers: ["Shift"] });
  await expect(page.locator(".canonical-status-hint")).toContainText("自动截断至当前分段边界");
  await expect(page.locator("[data-canonical-key='1:sb-b1']")).not.toHaveClass(/is-selected-block/);
  await expect(page.locator("[data-canonical-key='0:sa-b2']")).toHaveClass(/is-selected-block/);
});
