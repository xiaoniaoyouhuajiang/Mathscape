import { expect, test, type Page } from "@playwright/test";

const CANONICAL_ID = "mathscape:canonical-markdown/v1/mathscape.marker.math-ocr.v1/copy-outline/pdf";

async function setupLongCanonical(page: Page): Promise<void> {
  await page.setViewportSize({ width: 1360, height: 820 });
  await page.goto("/tests/reader.html");
  await page.waitForFunction(() => "harness" in window);

  await page.evaluate(async ([cid]) => {
    localStorage.removeItem("mathscape:md-outline");
    const enc = new TextEncoder();
    const paragraphs: string[] = [
      "# 第一章 组合数与母函数",
      "用对数作计算工具，要把两个数 $x$、$y$ 的乘积变为和 $\\lg x + \\lg y$。",
      "$$\\lg(xy) = \\lg x + \\lg y$$",
      "由此求反对数即可得到乘积 $xy$，把乘法运算变成了加法运算。",
      "进一步考虑二项式系数 $(1 + x)^n = \\sum_{k=0}^{n} \\binom{n}{k} x^k$。",
      "## 第二节 递推关系",
      "斐波那契数列满足 $F_{n+2} = F_{n+1} + F_n$，初值 $F_0 = 0, F_1 = 1$。",
      "令母函数 $G(x) = \\sum_{n=0}^{\\infty} F_n x^n$，整理可得有理分式：",
      "$$G(x) = \\frac{x}{1 - x - x^2}$$",
      "展开部分分式后即可给出通项公式与渐近估计。",
      "## 第三节 拆分数",
      "整数拆分把正整数写成若干正整数之和，其母函数为无穷乘积 $\\prod_{k \\ge 1} (1 - x^k)^{-1}$。",
      "五边形数定理给出了欧拉函数展开式的稀疏符号规律。",
      "在组合恒等式证明中，母函数提供了代数化的统一视角。",
      "## 第四节 指数型母函数",
      "排列问题常借助指数型母函数 $\\sum_{n \\ge 0} a_n \\frac{x^n}{n!}$ 处理标号结构。",
      "两个标号结构的笛卡尔积对应其指数型母函数的普通乘积。",
      "连通标号图与一般标号图之间满足指数关系 $G(x) = \\exp C(x)$。",
    ];
    const source = paragraphs.join("\n\n") + "\n";
    let offset = 0;
    const blocks = paragraphs.map((p, idx) => {
      const chunk = idx < paragraphs.length - 1 ? p + "\n\n" : p + "\n";
      const len = enc.encode(chunk).length;
      const blk = {
        id: `cb-${idx}`,
        kind: p.startsWith("#") ? "heading" : "text",
        range: { start: offset, end: offset + len },
      };
      offset += len;
      return blk;
    });

    const h = (window as any).harness;
    h.unitMarkers.push({
      unit_id: "mathscape:source-unit/fib-gf",
      title: "斐波那契母函数",
      unit_kind: "statement",
      artifact_id: "seg-copy",
      byte_range: blocks[8].range,
      block_ids: ["cb-8"],
    });
    h.setCanonicalProjection({
      canonical_id: cid,
      source_pdf_artifact_id: "pdf",
      head_artifact_id: "seg-copy",
      total_pages: 4,
      coverage: { covered: [{ from: 1, to: 4 }], missing: [], head_covered: [{ from: 1, to: 4 }] },
      skipped: [],
      pieces: [
        {
          kind: "segment",
          conversion_id: "conv-copy",
          artifact_id: "seg-copy",
          snapshot_pages: { from: 1, to: 4 },
          pages: { from: 1, to: 4 },
          overlap: null,
          overlap_trimmed: false,
          is_head: true,
          byte_start: 0,
          content_hash: "hash-copy",
          blocks,
          markdown: source,
        },
      ],
    });

    const host = document.querySelector<HTMLElement>("#reader")!;
    host.style.cssText = "height: 440px; overflow: auto; position: relative; overflow-anchor: none;";
    await h.reader.open(cid, "母函数 · 规范解析正文");
  }, [CANONICAL_ID]);
}

test("Canonical reader copies single & multi-block Markdown (with LaTeX & merged errata) via pill, context menu, and shortcut", async ({ page }) => {
  await setupLongCanonical(page);

  // 1. Single-block copy via floating pill preserves display LaTeX source
  const formulaBlock = page.locator("[data-canonical-key='0:cb-2']");
  await formulaBlock.click();
  const pill = page.locator(".block-action-pill");
  const copyBtn = pill.locator(".block-pill-copy");
  await expect(copyBtn).toHaveText("复制");
  await copyBtn.click();
  await expect(copyBtn).toHaveText("✓ 已复制");
  await expect(page.locator(".canonical-status-hint")).toContainText("已复制 1 块 Markdown 到剪贴板");
  expect(await page.evaluate(() => (window as any).harness.lastCopiedText())).toBe("$$\\lg(xy) = \\lg x + \\lg y$$");

  // 2. Merge-edit cb-6..cb-7 in place, then Shift+click cb-6..cb-8 and copy
  const b6 = page.locator("[data-canonical-key='0:cb-6']");
  const b7 = page.locator("[data-canonical-key='0:cb-7']");
  const b8 = page.locator("[data-canonical-key='0:cb-8']");
  await b6.click();
  await b7.click({ modifiers: ["Shift"] });
  await page.locator(".block-action-pill .block-pill-errata").click();
  const textarea = page.locator(".errata-inline-editor").getByLabel("校对后的 Markdown 源码");
  await textarea.fill("斐波那契母函数定义为 $G(x) = \\sum_{n=0}^{\\infty} F_n x^n$（已合并校对）。");
  await textarea.press("Control+Enter");
  await expect(page.locator(".errata-inline-editor")).toBeHidden();

  await b6.click();
  await b8.click({ modifiers: ["Shift"] });

  // Context menu reflects 2 visible blocks (merged cb-6+cb-7 plus cb-8) and strips UI badges
  await b6.click({ button: "right" });
  const menu = page.locator(".graph-context-menu");
  await expect(menu.locator("button").first()).toHaveText("复制这 2 块 Markdown");
  await page.keyboard.press("Escape");

  // Keyboard shortcut Cmd/Ctrl+C copies clean merged Markdown without duplicating hidden cb-7
  await page.keyboard.press("ControlOrMeta+c");
  expect(await page.evaluate(() => (window as any).harness.lastCopiedText())).toBe(
    "斐波那契母函数定义为 $G(x) = \\sum_{n=0}^{\\infty} F_n x^n$（已合并校对）。\n\n$$G(x) = \\frac{x}{1 - x - x^2}$$",
  );
  await expect(page.locator(".canonical-status-hint")).toContainText("已复制 2 块 Markdown 到剪贴板");
});

test("Native partial drag-selection copy inside reader restores KaTeX TeX source and strips badges without MathML duplication", async ({ page }) => {
  await setupLongCanonical(page);

  // Select across block cb-1 (inline formulas) through cb-2 (display formula) and cb-8 (has ◈ unit badge)
  const copiedSpan = await page.evaluate(() => {
    const startBlock = document.querySelector("[data-canonical-key='0:cb-1']")!;
    const endBlock = document.querySelector("[data-canonical-key='0:cb-3']")!;
    const range = document.createRange();
    range.setStart(startBlock, 0);
    range.setEnd(endBlock, endBlock.childNodes.length);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);

    const dt = new DataTransfer();
    const evt = new ClipboardEvent("copy", { bubbles: true, cancelable: true, clipboardData: dt });
    startBlock.dispatchEvent(evt);
    return { prevented: evt.defaultPrevented, plain: dt.getData("text/plain") };
  });

  expect(copiedSpan.prevented).toBe(true);
  expect(copiedSpan.plain).toContain("要把两个数 $x$、$y$ 的乘积变为和 $\\lg x + \\lg y$。");
  expect(copiedSpan.plain).toContain("$$\\lg(xy) = \\lg x + \\lg y$$");
  expect(copiedSpan.plain).toContain("由此求反对数即可得到乘积 $xy$");
  // Ensure hidden MathML text did not duplicate formula characters
  expect(copiedSpan.plain).not.toContain("lg(xy)=lgx+lgy");

  // Selecting block cb-8 which hosts `.unit-map-badge-row` (`◈ 斐波那契母函数 ↗`) excludes badge text
  const badgeBlockCopy = await page.evaluate(() => {
    const b8 = document.querySelector("[data-canonical-key='0:cb-8']")!;
    const range = document.createRange();
    range.selectNodeContents(b8);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
    const dt = new DataTransfer();
    const evt = new ClipboardEvent("copy", { bubbles: true, cancelable: true, clipboardData: dt });
    b8.dispatchEvent(evt);
    sel.removeAllRanges();
    return dt.getData("text/plain");
  });
  expect(badgeBlockCopy).toBe("$$G(x) = \\frac{x}{1 - x - x^2}$$");
  expect(badgeBlockCopy).not.toContain("斐波那契母函数");
});

test("Multi-block floating action pill stays visible in the scroll viewport when the first selected block scrolls off-screen", async ({ page }) => {
  await setupLongCanonical(page);

  await page.locator("[data-canonical-key='0:cb-1']").click();
  await page.locator("[data-canonical-key='0:cb-12']").click({ modifiers: ["Shift"] });
  const pill = page.locator(".block-action-pill");
  await expect(pill).toBeVisible();

  // Scroll down so cb-1 is well above #reader's top edge while cb-6..cb-10 are visible
  await page.evaluate(() => {
    const host = document.querySelector<HTMLElement>("#reader")!;
    const target = document.querySelector<HTMLElement>("[data-canonical-key='0:cb-8']")!;
    host.scrollTop += target.getBoundingClientRect().top - host.getBoundingClientRect().top - 80;
  });

  await expect.poll(async () => {
    return page.evaluate(() => {
      const host = document.querySelector<HTMLElement>("#reader")!.getBoundingClientRect();
      const firstBlock = document.querySelector<HTMLElement>("[data-canonical-key='0:cb-1']")!.getBoundingClientRect();
      const pillRect = document.querySelector<HTMLElement>(".block-action-pill")!.getBoundingClientRect();
      return {
        firstAboveViewport: firstBlock.bottom < host.top,
        pillInViewport: pillRect.top >= host.top && pillRect.bottom <= host.bottom,
      };
    });
  }).toEqual({ firstAboveViewport: true, pillInViewport: true });

  await pill.locator(".block-pill-copy").click();
  await expect(page.locator(".canonical-status-hint")).toContainText("已复制 12 块 Markdown 到剪贴板");
});

test("Sticky outline collapses & reopens in place mid-scroll, highlights active section, and supports `[` shortcut safely", async ({ page }) => {
  await setupLongCanonical(page);

  const outline = page.getByRole("navigation", { name: "文档大纲目录" });
  const collapseBtn = outline.getByRole("button", { name: "收起目录" });
  const reopenBtn = page.locator(".md-outline-reopen");

  await expect(outline).toBeVisible();
  await expect(collapseBtn).toBeVisible();
  await expect(reopenBtn).toBeHidden();

  // Scroll midway down so top `.reader-controls` bar is scrolled out of view
  await page.evaluate(() => {
    const host = document.querySelector<HTMLElement>("#reader")!;
    const sec3 = document.querySelector<HTMLElement>("[data-canonical-key='0:cb-10']")!;
    host.scrollTop += sec3.getBoundingClientRect().top - host.getBoundingClientRect().top - 40;
  });

  // Top toolbar button is above viewport, while sticky outline's `«` collapse button stays in viewport
  const midState = await page.evaluate(() => {
    const host = document.querySelector<HTMLElement>("#reader")!.getBoundingClientRect();
    const topControls = document.querySelector<HTMLElement>(".reader-controls")!.getBoundingClientRect();
    const collapse = document.querySelector<HTMLElement>(".md-outline-collapse")!.getBoundingClientRect();
    return {
      controlsScrolledAway: topControls.bottom < host.top,
      collapseVisibleInPane: collapse.top >= host.top && collapse.bottom <= host.bottom,
    };
  });
  expect(midState).toEqual({ controlsScrolledAway: true, collapseVisibleInPane: true });

  // Active section highlight tracks current scrolled section ("第三节 拆分数")
  await expect(outline.locator(".md-outline-item.is-active-outline-item")).toContainText("第三节 拆分数");

  // Collapse in place via sticky `«` button: reading position stays steady and sticky `☰ 目录` pill appears
  const topBeforeCollapse = await page.evaluate(() => {
    const host = document.querySelector<HTMLElement>("#reader")!.getBoundingClientRect();
    const block = document.querySelector<HTMLElement>("[data-canonical-key='0:cb-10']")!.getBoundingClientRect();
    return block.top - host.top;
  });
  await collapseBtn.click();
  await expect(outline).toBeHidden();
  await expect(reopenBtn).toBeVisible();

  const afterCollapse = await page.evaluate(() => {
    const host = document.querySelector<HTMLElement>("#reader")!.getBoundingClientRect();
    const block = document.querySelector<HTMLElement>("[data-canonical-key='0:cb-10']")!.getBoundingClientRect();
    const reopen = document.querySelector<HTMLElement>(".md-outline-reopen")!.getBoundingClientRect();
    return {
      blockTop: block.top - host.top,
      reopenVisibleInPane: reopen.top >= host.top && reopen.bottom <= host.bottom,
    };
  });
  expect(Math.abs(afterCollapse.blockTop - topBeforeCollapse)).toBeLessThanOrEqual(2);
  expect(afterCollapse.reopenVisibleInPane).toBe(true);

  // Reopen in place via sticky `☰ 目录` pill without scrolling back up
  await reopenBtn.click();
  await expect(outline).toBeVisible();
  await expect(reopenBtn).toBeHidden();
  await expect(outline.locator(".md-outline-item.is-active-outline-item")).toContainText("第三节 拆分数");

  // `[` keyboard shortcut toggles collapse/expand, except when typing inside an input/textarea
  await page.keyboard.press("[");
  await expect(outline).toBeHidden();
  await page.keyboard.press("[");
  await expect(outline).toBeVisible();

  await page.locator("[data-canonical-key='0:cb-10']").click();
  await page.keyboard.press("e");
  const editorInput = page.locator(".errata-editor-input");
  await expect(editorInput).toBeFocused();
  await page.keyboard.type("[1]");
  await expect(outline).toBeVisible();
  await expect(editorInput).toHaveValue(/\[1\]$/);
});

test("Physical snapshot reader supports block Markdown copy via floating pill and context menu", async ({ page }) => {
  await page.goto("/tests/reader.html");
  await page.waitForFunction(() => "harness" in window);
  await page.evaluate(async () => (window as any).harness.reader.open("md", "数学笔记"));

  const block2 = page.getByLabel("原文块 2", { exact: true });
  await block2.click();
  const copyBtn = page.locator(".block-action-pill .block-pill-copy");
  await expect(copyBtn).toBeVisible();
  await copyBtn.click();
  await expect(copyBtn).toHaveText("✓ 已复制");
  expect(await page.evaluate(() => (window as any).harness.lastCopiedText())).toContain("$");
});
