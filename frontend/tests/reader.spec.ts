import { test, expect } from "@playwright/test";

// The harness exercises actual reader/PDF.js/KaTeX code with a binary IPC double.
// Rust service tests cover the other side (Pool + catalog + persisted manifest).
test.beforeEach(async ({ page }) => {
  await page.goto("/tests/reader.html");
  await page.waitForFunction(() => "harness" in window);
});

test("PDF transport ignores late reads after abort or first failure", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const Transport = (window as any).harness.ArtifactPdfTransport;
    const failures: string[] = [], delivered: number[] = [];
    const pending: { resolve: (bytes: Uint8Array) => void; reject: (error: Error) => void }[] = [];
    const read = () => new Promise<Uint8Array>((resolve, reject) => pending.push({ resolve, reject }));
    const t = new Transport(100, new Uint8Array(), read, (e: Error) => failures.push(e.message));
    t.onDataRange = (begin: number) => delivered.push(begin);
    t.requestDataRange(0, 10);
    t.requestDataRange(10, 20);
    pending[0].reject(new Error("original IPC error"));
    await Promise.resolve();
    pending[1].resolve(new Uint8Array(10));
    await Promise.resolve();
    t.requestDataRange(20, 30);
    const aborted = new Transport(100, new Uint8Array(), read, (e: Error) => failures.push(e.message));
    aborted.onDataRange = (begin: number) => delivered.push(begin);
    aborted.requestDataRange(30, 40);
    aborted.abort();
    pending[2].resolve(new Uint8Array(10));
    await Promise.resolve();
    const broken = new Transport(100, new Uint8Array(), read, (e: Error) => failures.push(e.message));
    broken.onDataRange = () => { throw new Error("missing range reader"); };
    broken.requestDataRange(40, 50);
    pending[3].resolve(new Uint8Array(10));
    await Promise.resolve();
    return { failures, delivered, reads: pending.length };
  });
  expect(result.delivered).toEqual([]);
  expect(result.reads).toBe(4);
  expect(result.failures).toHaveLength(2);
  expect(result.failures[0]).toContain("读取字节，字节 [0, 10)");
  expect(result.failures[0]).toContain("original IPC error");
  expect(result.failures[1]).toContain("投递 PDF.js，字节 [40, 50)");
});

test("PDF IPC failure settles opening, preserves first error, and allows reopening", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.evaluate(async () => {
    const h = (window as any).harness;
    h.behavior.pdfRangeFailure = true;
    await h.reader.open("pdf", "Failure test");
  });
  await expect(page.locator("#reader")).toContainText("test original IPC failure");
  await expect(page.locator("#reader")).not.toContainText("正在读取");
  await page.evaluate(async () => {
    const h = (window as any).harness;
    h.behavior.pdfRangeFailure = false;
    await h.reader.open("pdf", "Retry");
  });
  await expect(page.locator("canvas")).toBeVisible();
  expect(errors).toEqual([]);
});

test("PDF range transport, outline destination, page input and cleanup", async ({ page }) => {
  await page.evaluate(async () => (window as any).harness.reader.open("pdf", "Two pages"));
  await expect(page.locator("canvas")).toBeVisible();
  await expect(page.getByLabel("PDF 页码")).toHaveValue("1");
  await page.getByRole("button", { name: "Second page" }).click();
  await expect(page.getByLabel("PDF 页码")).toHaveValue("2");
  await expect.poll(() => page.locator("canvas").evaluate((canvas: HTMLCanvasElement) => {
    return canvas.getContext("2d")!.getImageData(50, 350, 1, 1).data[2];
  })).toBe(255);
  await page.getByLabel("PDF 页码").fill("1");
  await page.getByLabel("PDF 页码").press("Tab");
  await expect(page.getByRole("button", { name: "上一页" })).toBeDisabled();
  const reads = await page.evaluate(() => (window as any).harness.reads);
  expect(reads.some((r: { offset: number }) => r.offset > 0)).toBeTruthy();
  expect(reads.every((r: { length: number }) => r.length <= 4 * 1024 * 1024)).toBeTruthy();
  await page.evaluate(() => (window as any).harness.reader.close());
  await expect(page.locator("#reader")).toBeEmpty();
});

test("Markdown four math delimiters, local image mapping, raw toggle and unsafe content", async ({ page }) => {
  const external: string[] = [];
  page.on("request", request => { if (request.url().startsWith("https://example.com")) external.push(request.url()); });
  await page.evaluate(async () => (window as any).harness.reader.open("md", "Math notes"));
  await expect(page.locator(".katex")).toHaveCount(4);
  await expect(page.locator(".katex-error")).toHaveCount(0);
  await expect(page.locator(".katex-display")).toHaveCount(2);
  await expect(page.locator("article img")).toHaveCount(1);
  expect(await page.locator("article img").getAttribute("src")).toMatch(/^blob:/);
  await expect.poll(() => page.locator("article img").evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(1);
  await expect(page.locator("article script")).toHaveCount(0);
  await expect(page.locator("article code")).toHaveText("$code_not_math$\n");
  expect(external).toEqual([]);
  await page.getByRole("button", { name: "查看原文" }).click();
  await expect(page.locator(".katex")).toHaveCount(0);
  await page.getByRole("button", { name: "查看渲染" }).click();
  await expect(page.locator(".katex")).toHaveCount(4);
});

test("virtual path boundary, read errors, JSON and switching during a load", async ({ page }) => {
  const paths = await page.evaluate(() => {
    const resolve = (window as any).harness.resolveProductPath;
    return ["../plot.png", "../../outside.png", "/etc/passwd", "file:///etc/passwd", "%2e%2e/%2e%2e/out", "https://example.com/a", "images/a%20b.png"]
      .map(ref => resolve("book/book.md", ref));
  });
  expect(paths).toEqual(["plot.png", null, null, null, null, null, "book/images/a b.png"]);
  await page.evaluate(async () => {
    const { reader } = (window as any).harness;
    await Promise.all([reader.open("pdf", "Old"), reader.open("json", "Metadata")]);
  });
  await expect(page.locator("canvas")).toHaveCount(0);
  await expect(page.locator("pre")).toContainText('"page": 1');
  await page.evaluate(async () => (window as any).harness.reader.open("bad", "Broken"));
  await expect(page.locator("#reader")).toContainText("Invalid PDF");
  await page.evaluate(async () => (window as any).harness.reader.open("missing", "Missing"));
  await expect(page.locator("#reader")).toContainText("Artifact 不存在");
});

test("failed reads are not content and never replace the last successful document", async ({ page }) => {
  const first = await page.evaluate(async () => {
    const reader = (window as any).harness.reader;
    const opened = await reader.open("missing", "Missing");
    return { opened, hasContent: reader.hasContent(), last: reader.lastOpened() };
  });
  expect(first).toEqual({ opened: false, hasContent: false, last: undefined });
  const retry = await page.evaluate(async () => {
    const reader = (window as any).harness.reader;
    await reader.open("md", "Valid", { conversionId: "conversion", focus: { start: 0, end: 10, blockIds: ["test-block-0"] } });
    const opened = await reader.open("missing", "Missing again");
    return { opened, hasContent: reader.hasContent(), last: reader.lastOpened() };
  });
  expect(retry.opened).toBe(false);
  expect(retry.hasContent).toBe(false);
  expect(retry.last).toEqual({ id: "md", title: "Valid", options: { conversionId: "conversion", focus: { start: 0, end: 10, blockIds: ["test-block-0"] } } });
});

test("Markdown block endpoints, raw source, full selection and review policy", async ({ page }) => {
  await page.evaluate(async () => (window as any).harness.reader.open("md", "选择测试"));
  await expect(page.getByRole("button", { name: "设置起点" })).toBeDisabled();
  await page.getByLabel("原文块 2", { exact: true }).click();
  await page.getByRole("button", { name: "设置起点" }).click();
  // Click the actual rendered KaTeX, not a copy of the source formula.
  await page.getByLabel("原文块 3", { exact: true }).locator(".katex").click();
  await page.getByRole("button", { name: "设置终点" }).click();
  await expect(page.locator(".is-selected-block")).toHaveCount(2);
  await page.getByRole("button", { name: "构建发现输入" }).click();
  await expect(page.locator(".selection-panel [role=status]")).toContainText("发现输入已构建：2 块");
  await expect(page.locator(".selection-source")).toContainText("\\frac{1}{n}");
  const request = await page.evaluate(() => (window as any).harness.selectionRequests.at(-1));
  expect(request.selector).toEqual({ kind: "block_range", first: "test-block-1", last: "test-block-2" });
  expect(request.admission).toBe("review_before_publish");
  await page.getByRole("button", { name: "查看原文" }).click();
  await page.getByRole("button", { name: "查看渲染" }).click();
  await expect(page.locator(".is-selected-block")).toHaveCount(2);
  await page.getByRole("button", { name: "选择全文" }).click();
  await expect(page.getByLabel("后续入图方式")).toHaveCount(0);
  await page.getByRole("button", { name: "构建发现输入" }).click();
  await expect(page.locator(".selection-source")).toContainText("# Math reader 中文😀");
  expect(await page.evaluate(() => (window as any).harness.selectionRequests.at(-1).selector.kind)).toBe("whole_document");
  await page.getByRole("button", { name: "清除选择" }).click();
  await expect(page.getByRole("button", { name: "构建发现输入" })).toBeDisabled();
});

test("Markdown selection runs Discovery and shows an unregistered proposal", async ({ page }) => {
  await page.evaluate(async () => (window as any).harness.reader.open("md", "发现测试"));
  await page.getByLabel("原文块 2", { exact: true }).click();
  await page.getByRole("button", { name: "设置起点" }).click();
  await page.getByLabel("原文块 2", { exact: true }).click();
  await page.getByRole("button", { name: "设置终点" }).click();
  await page.evaluate(() => { (window as any).harness.behavior.discoveryDelay = 150; });
  await page.getByRole("button", { name: "使用 LLM 发现内容" }).click();
  await expect(page.getByRole("button", { name: "使用 LLM 发现内容" })).toBeDisabled();
  await expect(page.locator(".discovery-status")).toContainText("LLM 正在分析");
  const workspace = page.locator(".review-workspace");
  await expect(workspace).toBeVisible();
  await expect(page.locator(".discovery-status")).toContainText("尚未接受或入图");
  await expect(workspace.locator(".review-run")).toContainText("150 tokens");
  // The first unit is locked on open: its whole block is outlined and rendered through KaTeX.
  const preview = workspace.locator(".review-preview");
  await expect(preview.locator(".review-fragment-block.is-anchor-block .katex")).toHaveCount(2);
  await workspace.getByRole("tab", { name: "单元 2" }).click();
  const unit = workspace.locator(".review-unit").first();
  await expect(unit).toContainText("测试命题");
  await expect(unit.locator(".review-excerpt .katex")).toHaveCount(2);
  await expect(workspace.getByLabel("选择来源单元 测试命题")).toBeChecked();
  await workspace.getByRole("button", { name: "清空选择" }).click();
  await expect(workspace.getByRole("button", { name: "接受选中内容" })).toBeDisabled();
  // Selecting the proof pulls in the unit it proves.
  await workspace.getByLabel("选择来源单元 测试命题").check();
  await expect(workspace.getByLabel("选择来源单元 被证命题")).toBeChecked();
  await workspace.getByLabel("选择来源单元 被证命题").uncheck();
  await expect(workspace.getByLabel("选择来源单元 测试命题")).not.toBeChecked();
  await workspace.getByRole("button", { name: "选择全部可接受单元" }).click();
  await expect(workspace.getByLabel("选择来源单元 测试命题")).toBeChecked();

  // A reference anchored on a sub-range highlights exactly that formula.
  await workspace.getByRole("tab", { name: "引用 1" }).click();
  await expect(workspace.locator(".review-reference")).toContainText("定义 2 · 未解析");
  await workspace.locator(".review-reference").click();
  await expect(preview.locator(".review-anchor-highlight.markdown-patch-highlight-formula")).toHaveCount(1);
  await expect(preview.locator(".review-fragment-block.is-anchor-block")).toHaveCount(0);
  await expect(preview.locator(".review-raw-source mark")).toHaveText("$x^2$");

  await workspace.getByRole("tab", { name: "关系 1" }).click();
  await workspace.locator(".review-relation").click();
  await expect(preview).toContainText("测试命题 —证明→ 被证命题");
  await preview.getByRole("button", { name: "证据 2" }).click();
  await expect(preview.locator(".review-raw-source mark")).toHaveText("$x^2$");

  await workspace.getByRole("tab", { name: "诊断 2" }).click();
  await workspace.locator(".review-issue").click();
  await expect(preview.locator(".review-fallback-note")).toHaveCount(0);
  await workspace.locator(".review-diagnostic").click();
  await expect(preview.locator(".review-fallback-note")).toContainText("诊断未提供独立原文范围");
  await expect(preview.locator(".review-fragment-block.is-anchor-block")).toHaveCount(1);

  await workspace.getByRole("button", { name: "接受选中内容" }).click();
  await expect(workspace.locator(".review-action-feedback")).toContainText("审核并注册完成：2 个单元");
  await expect(workspace.locator(".review-selection-state")).toContainText("已保存登记");
  await expect(workspace.getByRole("button", { name: "接受选中内容" })).toBeDisabled();
  const acceptance = await page.evaluate(() => (window as any).harness.acceptanceRequests.at(-1));
  expect(acceptance).toEqual({
    run_id: "mathscape:discovery-run/browser-fixture",
    expected_result_fingerprint: "sha256:" + "1".repeat(64),
    selection: { kind: "explicit_units", unit_refs: ["u1", "u2"] },
  });
  const request = await page.evaluate(() => (window as any).harness.discoveryRequests.at(-1));
  expect(request.selector).toEqual({
    kind: "block_range", first: "test-block-1", last: "test-block-1",
  });
  expect(request.admission).toBe("review_before_publish");
});

test("Discovery paused for Markdown patch review shows a rendered comparison and resumes", async ({ page }) => {
  await page.evaluate(async () => {
    (window as any).harness.behavior.discoveryStatus = "awaiting_patch_review";
    await (window as any).harness.reader.open("md", "修订审核测试");
  });
  await page.getByLabel("原文块 2", { exact: true }).click();
  await page.getByRole("button", { name: "设置起点" }).click();
  await page.getByRole("button", { name: "设置终点" }).click();
  await page.getByRole("button", { name: "使用 LLM 发现内容" }).click();
  const host = page.locator(".markdown-patch-host");
  await expect(host).toContainText("内容发现已暂停");
  await expect(page.locator(".discovery-status")).toContainText("审核 Markdown 修订后继续");
  await expect(page.locator(".review-workspace")).toBeHidden();
  await expect(page.getByRole("button", { name: "生成 Markdown 修订建议" })).toHaveCount(0);
  // Both panes render the whole enclosing block through KaTeX.
  const panes = host.locator(".markdown-patch-pane .markdown-body");
  await expect(panes).toHaveCount(2);
  await expect(panes.nth(0).locator(".katex")).toHaveCount(2);
  await expect(panes.nth(1).locator(".katex")).toHaveCount(2);
  await expect(panes.nth(0).locator(".markdown-patch-highlight-old.markdown-patch-highlight-formula")).toHaveCount(1);
  await expect(panes.nth(1).locator(".markdown-patch-highlight-new.markdown-patch-highlight-formula")).toHaveCount(1);
  await expect(host.locator(".markdown-patch-inline del")).toHaveText("x^2");
  await expect(host.locator(".markdown-patch-inline ins")).toHaveText("x^3");
  await expect(host.locator(".markdown-patch-review-status")).toContainText("可直接衔接 Stage B");
  await host.getByLabel("p1 replacement").fill("x^4");
  await expect(host.locator(".markdown-patch-inline ins")).toHaveText("x^4");
  await expect(panes.nth(1)).toContainText("4");
  await host.getByRole("button", { name: "全部拒绝并继续发现" }).click();
  await expect(page.locator(".review-workspace")).toBeVisible();
  await expect(page.locator(".review-workspace .review-footer")).toContainText("已选择 2 个单元");
  await expect(host).toBeHidden();
  const recorded = await page.evaluate(() => ({
    decision: (window as any).harness.patchDecisions.at(-1),
    resume: (window as any).harness.resumeRequests.at(-1),
  }));
  expect(recorded.decision.accepted_patch_refs).toEqual([]);
  expect(recorded.decision.replacements).toEqual({ p1: "x^4" });
  expect(recorded.resume).toEqual({ run_id: "mathscape:discovery-run/browser-fixture" });
});

test("reopening a reader recovers a paused run and continues a recorded decision", async ({ page }) => {
  await page.evaluate(async () => {
    (window as any).harness.behavior.pendingPatchStatus = "proposed";
    await (window as any).harness.reader.open("md", "恢复审核");
  });
  const host = page.locator(".markdown-patch-host");
  await expect(host).toContainText("正在等待审核 Markdown 修订");
  await expect(host.getByRole("button", { name: "应用选中修订并继续发现" })).toBeVisible();
  // Changing the selection must not discard the paused review.
  await page.getByLabel("原文块 1", { exact: true }).click();
  await page.getByRole("button", { name: "设置起点" }).click();
  await expect(host).toBeVisible();

  await page.evaluate(async () => {
    (window as any).harness.behavior.pendingPatchStatus = "applied";
    await (window as any).harness.reader.open("md", "恢复审核");
  });
  await expect(host).toContainText("修订已应用");
  await host.getByRole("button", { name: "按已记录的决定继续内容发现" }).click();
  await expect(page.locator(".review-workspace")).toBeVisible();
  await expect(page.locator(".discovery-result")).toContainText("打开审核工作区");
  await expect(host).toBeHidden();
});

test("ask-each-run workspace policy requires an explicit admission choice", async ({ page }) => {
  await page.evaluate(() => { (window as any).harness.behavior.admission = "ask_each_run"; });
  await page.evaluate(async () => (window as any).harness.reader.open("md", "审核策略测试"));
  await page.evaluate(() => window.dispatchEvent(new CustomEvent(
    "mathscape:workspace-settings-changed",
    { detail: {
      version: "mathscape-workspace-settings-1",
      discovery_admission: "ask_each_run",
      discovery_timeout_seconds: 600,
      connect_timeout_seconds: 15,
    } },
  )));
  await page.getByRole("button", { name: "选择全文" }).click();
  await expect(page.getByRole("button", { name: "使用 LLM 发现内容" })).toBeEnabled();
  await page.getByRole("button", { name: "使用 LLM 发现内容" }).click();
  await expect(page.locator(".admission-dialog")).toBeVisible();
  expect(await page.evaluate(() => (window as any).harness.discoveryRequests.length)).toBe(0);
  await page.getByRole("button", { name: "人工审核", exact: true }).click();
  await expect(page.locator(".task-status")).toContainText("两阶段验证完成");
});

test("partial staged discovery remains visible without claiming missing facts are absent", async ({ page }) => {
  await page.evaluate(() => { (window as any).harness.behavior.discoveryStatus = "partial"; });
  await page.evaluate(async () => (window as any).harness.reader.open("md", "部分发现测试"));
  await page.getByRole("button", { name: "选择全文" }).click();
  await page.getByRole("button", { name: "使用 LLM 发现内容" }).click();
  await expect(page.locator(".discovery-status")).toContainText("两阶段内容发现部分完成");
  const workspace = page.locator(".review-workspace");
  await expect(workspace.locator(".review-pending")).toContainText("尚有 1 个事实任务未完成");
  await workspace.getByRole("tab", { name: "单元 2" }).click();
  await expect(workspace.locator(".review-unit").first()).toContainText("来源事实待发现");
});

test("review workspace survives the reader, restores its draft and returns to the selection", async ({ page }) => {
  await page.evaluate(async () => (window as any).harness.reader.open("md", "往返测试"));
  await page.getByLabel("原文块 2", { exact: true }).click();
  await page.getByRole("button", { name: "设置起点" }).click();
  await page.getByRole("button", { name: "设置终点" }).click();
  await page.getByRole("button", { name: "使用 LLM 发现内容" }).click();
  const workspace = page.locator(".review-workspace");
  await workspace.getByRole("tab", { name: "单元 2" }).click();
  await workspace.getByRole("button", { name: "清空选择" }).click();
  await workspace.getByRole("tab", { name: "引用 1" }).click();
  await workspace.locator(".review-reference").click();

  await workspace.getByRole("button", { name: "在阅读器中打开" }).click();
  await expect(workspace).toBeHidden();
  await expect(page.getByLabel("原文块 2", { exact: true })).toHaveClass(/source-block-focus/);
  await page.getByRole("button", { name: /返回审核工作区/ }).click();
  await expect(workspace).toBeVisible();
  await expect(workspace.locator(".review-reference")).toHaveClass(/is-current/);

  // Reopening the same run (e.g. from the ledger) restores the unaccepted draft.
  await page.evaluate(async () => {
    const h = (window as any).harness;
    window.dispatchEvent(new CustomEvent("mathscape:open-review", { detail: { outcome: await h.discoveryOutcome(), meta: { title: "恢复" } } }));
  });
  await expect(workspace.getByRole("tab", { name: "引用 1" })).toHaveAttribute("aria-selected", "true");
  await expect(workspace.locator(".review-reference")).toHaveClass(/is-current/);
  await expect(workspace.locator(".review-selection-state")).toContainText("已选择 0 个单元");

  await workspace.getByRole("button", { name: "返回选区" }).click();
  await expect(workspace).toBeHidden();
  await expect(page.getByLabel("原文块 2", { exact: true })).toHaveClass(/source-block-focus/);
  await expect(page.getByLabel("原文块 1", { exact: true })).not.toHaveClass(/source-block-focus/);
});

test("review workspace refuses to locate anchors on a drifted Markdown snapshot", async ({ page }) => {
  await page.evaluate(async () => {
    const h = (window as any).harness;
    h.behavior.documentHash = "other-hash";
    window.dispatchEvent(new CustomEvent("mathscape:open-review", { detail: { outcome: await h.discoveryOutcome() } }));
  });
  const workspace = page.locator(".review-workspace");
  await expect(workspace.locator(".review-state")).toHaveText("锚点失效");
  await expect(workspace.locator(".review-preview .review-stale")).toContainText("不做近似定位");
  await expect(workspace.locator(".review-fragment")).toHaveCount(0);
  await expect(workspace.getByRole("button", { name: "接受选中内容" })).toBeDisabled();
  await expect(workspace.getByRole("button", { name: "返回选区" })).toBeDisabled();
});

test("review graph shares unit refs with the lists and shows evidence markers on demand", async ({ page }) => {
  await page.evaluate(async () => {
    const h = (window as any).harness;
    window.dispatchEvent(new CustomEvent("mathscape:open-review", { detail: { outcome: await h.discoveryOutcome() } }));
  });
  const workspace = page.locator(".review-workspace");
  await workspace.getByRole("tab", { name: "局部结构图" }).click();
  const graph = workspace.locator(".review-graph");
  await expect(graph).toHaveAttribute("data-unit-count", "2");
  await expect(graph).toHaveAttribute("data-current", "u1");
  await workspace.getByLabel(/证据模式/).check();
  await expect(graph).toHaveAttribute("data-evidence-count", "2");
  await workspace.getByRole("button", { name: "切换为分层列表" }).click();
  await expect(workspace.locator(".review-layered-row")).toHaveText(["测试命题", "被证命题"]);
  await expect(workspace.locator(".review-layered-row").first()).toHaveClass(/is-current/);
  await workspace.locator(".review-layered-row").nth(1).click();
  await workspace.getByRole("button", { name: "切换为图模式" }).click();
  await expect(workspace.locator(".review-graph")).toHaveAttribute("data-current", "u2");
});

test("streaming task survives reader changes and minimization", async ({ page }) => {
  await page.evaluate(async () => { (window as any).harness.behavior.discoveryDelay = 900; await (window as any).harness.reader.open("md", "流式测试"); });
  await page.getByRole("button", { name: "选择全文" }).click();
  await page.getByRole("button", { name: "使用 LLM 发现内容" }).click();
  await expect(page.locator(".task-output")).toContainText('{"units":');
  await expect(page.locator(".task-reasoning pre")).toHaveText("供应商公开摘要");
  await page.getByRole("button", { name: "最小化任务" }).click();
  await page.evaluate(async () => (window as any).harness.reader.open("json", "另一份材料"));
  await page.locator(".task-launcher").click();
  await expect(page.locator(".task-status")).toContainText("两阶段验证完成");
  await page.getByText("查看完整结果与验证诊断").click();
  await expect(page.locator(".task-result")).toContainText("测试命题");
});

test("reversed endpoints, stale async validation, context switch and errors", async ({ page }) => {
  await page.evaluate(async () => (window as any).harness.reader.open("md", "选择测试"));
  await page.getByLabel("原文块 3", { exact: true }).focus();
  await page.getByLabel("原文块 3", { exact: true }).press("Enter");
  await page.getByRole("button", { name: "设置起点" }).click();
  await page.getByLabel("原文块 1", { exact: true }).click();
  await page.getByRole("button", { name: "设置终点" }).click();
  await expect(page.getByRole("button", { name: "构建发现输入" })).toBeDisabled();
  await expect(page.locator(".selection-panel [role=status]")).toContainText("终点必须");
  await page.getByRole("button", { name: "选择全文" }).click();
  await page.evaluate(() => { (window as any).harness.behavior.selectionDelay = 300; });
  await page.getByRole("button", { name: "构建发现输入" }).click();
  await page.getByRole("button", { name: "清除选择" }).click();
  await page.waitForTimeout(400);
  await expect(page.locator(".selection-source")).toBeHidden();
  await page.getByRole("button", { name: "选择全文" }).click();
  await page.getByLabel("转换来源").selectOption("1");
  await expect(page.getByRole("button", { name: "构建发现输入" })).toBeDisabled();
  await page.getByRole("button", { name: "选择全文" }).click();
  await page.evaluate(() => { (window as any).harness.behavior.failSelection = true; });
  await page.getByRole("button", { name: "构建发现输入" }).click();
  await expect(page.locator(".selection-panel [role=status]")).toContainText("输入快照已改变");
  expect(await page.evaluate(() => (window as any).harness.selectionRequests.at(-1).snapshot.conversion_id)).toBe("conversion-two");
});

test("anchor fragments keep block syntax and never leak renderer markers", async ({ page }) => {
  const result = await page.evaluate(() => {
    const { fragment } = (window as any).harness;
    return {
      heading: fragment("## 定理 1 与 $x$\n", "heading", "定理 1"),
      list: fragment("- 第一项\n- 第二项 $y$\n", "list", "第一项\n- 第二项"),
      table: fragment("| a | b |\n|---|---|\n| 1 | 2 |\n", "table", "1"),
      code: fragment("```\ncode x\n```\n", "code", "x"),
      inlineCode: fragment("Use `abc` here\n", "text", "bc"),
    };
  });
  expect(result.heading).toMatchObject({ heading: true, marks: 1, whole: false, leaked: false });
  expect(result.list).toMatchObject({ items: 2, marks: 2, whole: false, leaked: false });
  expect(result.table).toMatchObject({ table: true, marks: 0, whole: true, leaked: false });
  expect(result.code).toMatchObject({ marks: 0, whole: true, leaked: false });
  expect(result.inlineCode).toMatchObject({ whole: true, leaked: false });
});
