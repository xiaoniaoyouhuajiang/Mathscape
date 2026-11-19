import { test, expect } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/reader.html");
  await page.waitForFunction(() => "harness" in window);
});

test("physical Markdown inline errata editor: live KaTeX preview, save preserves ink badge, Diff popover & revert, plus multi-block merge", async ({ page }) => {
  await page.evaluate(async () => {
    const h = (window as any).harness;
    h.ink.links = [{
      link: {
        link_id: "lnk-1",
        document_id: "note-1",
        role: "note",
        is_primary: true,
        target_status: "active",
        target: {
          kind: "markdown",
          artifact_id: "md",
          content_hash: "test-hash",
          first_block: "test-block-1",
          last_block: "test-block-1",
          block_ids: ["test-block-1"],
          start: 0,
          end: 20,
          block_hash: "bh",
          excerpt: "Inline",
          label: "Inline",
        },
        created_at: "2026-11-19",
      },
      note: { document_id: "note-1", title: "推导草稿", lifecycle: "active", updated_at: "2026-11-19" },
    }];
    await h.reader.open("md", "Math notes");
  });

  const block1 = page.locator('[data-source-block="test-block-1"]');
  await expect(block1.locator(".ink-block-marker")).toHaveText("✎ 1");

  // 1. Click block 1 and launch inline errata editor from the floating pill
  await block1.click();
  await expect(block1.locator(".block-pill-errata")).toHaveText("✎ 校对");
  await block1.locator(".block-pill-errata").click();

  const editor = page.locator(".errata-inline-editor");
  await expect(editor).toBeVisible();
  const textarea = editor.getByLabel("校对后的 Markdown 源码");
  await expect(textarea).toHaveValue(/Inline \$x\^2\$/);

  // 2. Live KaTeX preview warns on broken LaTeX and clears once corrected
  await textarea.fill("Inline $\\frac{1$ broken");
  await expect(editor.locator(".errata-katex-warning")).toBeVisible();
  await expect(editor.locator(".errata-katex-warning")).toContainText("公式语法异常");

  await textarea.fill("Inline $x^n + y^n$ and $\\alpha_k \\to \\alpha$（已核对原书）。");
  await expect(editor.locator(".errata-katex-warning")).toBeHidden();
  await expect(editor.locator(".errata-editor-preview .katex")).toHaveCount(2);
  await expect(editor.locator(".errata-editor-diff-badge")).toContainText("+1 / −1");
  await editor.getByLabel("勘误备注").fill("核对原书第 2 页");

  // 3. Save with Ctrl/Meta+Enter -> immediate re-render + restrained `已校对` badge + preserved `✎ 1` ink badge
  await textarea.press("Control+Enter");
  await expect(editor).toBeHidden();
  await expect(block1).toHaveClass(/has-errata/);
  await expect(block1).toContainText("已核对原书");
  await expect(block1.locator(".ink-block-marker")).toHaveText("✎ 1");
  await expect(block1.locator(".errata-block-badge")).toHaveText("已校对");

  // 4. Open `已校对` Diff popover -> verify OCR vs corrected diff & revert back to original OCR
  await block1.locator(".errata-block-badge").click();
  const popover = page.locator(".errata-diff-popover");
  await expect(popover).toBeVisible();
  await expect(popover).toContainText("备注：核对原书第 2 页");
  await expect(popover.locator(".errata-diff-line.is-del")).toContainText("x^2");
  await expect(popover.locator(".errata-diff-line.is-add")).toContainText("x^n + y^n");

  await popover.getByRole("button", { name: "恢复原 OCR" }).click();
  await expect(popover).toBeHidden();
  await expect(block1.locator(".errata-block-badge")).toHaveCount(0);
  await expect(block1).not.toContainText("已核对原书");
  await expect(block1.locator(".ink-block-marker")).toHaveText("✎ 1");

  // 5. Shift+Click multi-block merge across blocks 1..2 -> hides block 2 and keeps block 1's ink badge
  await block1.click();
  await page.locator('[data-source-block="test-block-2"]').click({ modifiers: ["Shift"] });
  await expect(block1.locator(".block-pill-errata")).toHaveText("✎ 合并校对 2 块");
  await block1.locator(".block-pill-errata").click();
  await expect(page.locator(".errata-inline-editor")).toContainText("合并校对 2 块");
  await page.locator(".errata-inline-editor").getByLabel("校对后的 Markdown 源码").fill("合并定理：$$x^n = \\sum_{k=0}^n c_k$$");
  await page.locator(".errata-inline-editor").getByRole("button", { name: "保存校对" }).click();

  await expect(block1).toContainText("合并定理");
  await expect(block1.locator(".errata-block-badge")).toHaveText("已校对");
  await expect(block1.locator(".ink-block-marker")).toHaveText("✎ 1");
  await expect(page.locator('[data-source-block="test-block-2"]')).toBeHidden();
});

test("canonical Markdown inline errata editing works in-place without losing TOC or unit badges", async ({ page }) => {
  await page.evaluate(async () => {
    const h = (window as any).harness;
    const md1 = "# 第二章 形式幂级数\n\n定理 1：$(1-x)^{-a} = \\sum C_n x^n$。\n\n证明：对 $n$ 归纳。\n";
    const enc = new TextEncoder();
    const b0Start = 0;
    const b1Start = enc.encode("# 第二章 形式幂级数\n\n").length;
    const b2Start = enc.encode("# 第二章 形式幂级数\n\n定理 1：$(1-x)^{-a} = \\sum C_n x^n$。\n\n").length;
    const total = enc.encode(md1).length;
    h.unitMarkers.push({
      unit_id: "mathscape:source-unit/thm-1",
      title: "定理 1：负二项展开",
      unit_kind: "theorem",
      artifact_id: "art-seg-1",
      byte_range: { start: b1Start, end: b2Start },
      block_ids: ["seg1-b1"],
    });
    h.setCanonicalProjection({
      canonical_id: "mathscape:canonical-markdown/v1/profile/policy/pdf",
      source_pdf_artifact_id: "pdf",
      head_artifact_id: "art-seg-1",
      total_pages: 2,
      coverage: { covered: [{ from: 1, to: 2 }], missing: [], head_covered: [{ from: 1, to: 2 }] },
      skipped: [],
      pieces: [
        {
          kind: "segment",
          conversion_id: "conv-1",
          artifact_id: "art-seg-1",
          snapshot_pages: { from: 1, to: 2 },
          pages: { from: 1, to: 2 },
          overlap: null,
          overlap_trimmed: false,
          is_head: true,
          byte_start: 0,
          content_hash: "sha256:seg1",
          blocks: [
            { id: "seg1-b0", kind: "heading", range: { start: b0Start, end: b1Start } },
            { id: "seg1-b1", kind: "text", range: { start: b1Start, end: b2Start } },
            { id: "seg1-b2", kind: "text", range: { start: b2Start, end: total } },
          ],
          errata: [],
          markdown: md1,
        },
      ],
    });
    await h.reader.open("mathscape:canonical-markdown/v1/profile/policy/pdf", "母函数 · 规范解析正文");
  });

  const thmBlock = page.locator('[data-source-block="seg1-b1"]');
  await expect(thmBlock.locator(".unit-map-badge")).toContainText("定理 1：负二项展开");

  // Select block and press keyboard shortcut `E` to open inline errata editor
  await thmBlock.click();
  await thmBlock.press("e");
  const editor = page.locator(".errata-inline-editor");
  await expect(editor).toBeVisible();

  await editor.getByLabel("校对后的 Markdown 源码").fill("定理 1：$(1-x)^{-n} = \\sum_{j=0}^{\\infty} C_{n+j-1}^{n-1} x^j$。");
  await expect(editor.locator(".errata-editor-preview .katex-error")).toHaveCount(0);
  await editor.getByRole("button", { name: "保存校对" }).click();

  await expect(editor).toBeHidden();
  await expect(thmBlock).toHaveClass(/has-errata/);
  await expect(thmBlock.locator(".errata-block-badge")).toHaveText("已校对");
  await expect(thmBlock.locator(".unit-map-badge")).toContainText("定理 1：负二项展开");

  // Revert from Diff popover restores original block cleanly
  await thmBlock.locator(".errata-block-badge").click();
  await page.locator(".errata-diff-popover").getByRole("button", { name: "恢复原 OCR" }).click();
  await expect(thmBlock.locator(".errata-block-badge")).toHaveCount(0);
  await expect(thmBlock.locator(".unit-map-badge")).toContainText("定理 1：负二项展开");
});
