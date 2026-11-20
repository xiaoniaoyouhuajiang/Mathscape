/**
 * Map Scene Player fourth slice: propose_changes on the main map, the
 * Loogle / Lean / locate Act windows, the running pill, live pacing and
 * 「减少动效」. Every round replays real PR #48 tool events
 * (tests/fixtures/scene-tool-events.json); only the artifact ids are pointed
 * at a PDF the harness map has, so anchors resolve.
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test, type Page } from "@playwright/test";

type Event = { payload: { call_id: string; status?: string; raw: { tool: string; arguments: unknown; result?: unknown } } };
type Pair = { call: Event; result: Event | null };
const fixtures = JSON.parse(readFileSync(resolve("tests/fixtures/scene-tool-events.json"), "utf8")) as Record<string, Pair>;

const CITE_ART = "mathscape:artifact/" + "cd34".repeat(16);
const CITE_UNIT = "mathscape:source-unit/" + "ab12".repeat(16);

/** A harness script step from a fixture pair; real artifact ids → the map's PDF. */
function step(name: string, callAt: number, resultAt?: number, edit?: (text: string) => string) {
  let text = JSON.stringify(fixtures[name]).replace(/mathscape:artifact\/sha256\/[0-9a-f]{64}/g, CITE_ART);
  if (edit) text = edit(text);
  const pair = JSON.parse(text) as Pair;
  return {
    id: `${name}-${callAt}`,
    tool: pair.call.payload.raw.tool,
    args: pair.call.payload.raw.arguments,
    result: pair.result?.payload.raw.result,
    status: pair.result?.payload.status,
    callAt,
    ...(resultAt !== undefined ? { resultAt } : {}),
  };
}

/** 习题 8 of an earlier change set (an existing object in the real session) → the map's unit. */
const EXERCISE8 = "mathscape:source-unit/c47ff212a0812879a40db7f22ebc4f2b430a2b750877456072ab8a1ed4a70cac";
const onMap = (text: string) => text.split(EXERCISE8).join(CITE_UNIT);

const stagedId = (name: string, local: string): string => {
  const raw = fixtures[name].result!.payload.raw.result as { structuredContent: { staged_ids: Record<string, string> } };
  return raw.structuredContent.staged_ids[local];
};
function semanticStep(callAt = 30, resultAt = 220) {
  const staged_ids = {
    span: "mathscape:source-span/" + "11aa".repeat(16),
    theory: "mathscape:semantic-entity/" + "22bb".repeat(16),
    concept: "mathscape:semantic-entity/" + "33cc".repeat(16),
    declaration: "mathscape:semantic-entity/" + "44dd".repeat(16),
    alignment: "mathscape:alignment/" + "55ee".repeat(16),
    debt: "mathscape:debt/" + "66ff".repeat(16),
    scope: "mathscape:dialect-scope/" + "7788".repeat(16),
  };
  const args = {
    request_id: "semantic-scene-playwright",
    rationale: "构建语义对象",
    proposed_objects: [
      { local_ref: "span", kind: "source_span", op: "create", payload: { artifact_id: CITE_ART, md_byte_range: { start: 0, end: 8 }, granularity: "inline", quote: "语义证据" } },
      { local_ref: "theory", kind: "semantic_entity", op: "create", payload: { semantic_kind: "theory", home_theory: null, preferred_labels: [{ lang: "zh", label: "生成函数" }] } },
      { local_ref: "concept", kind: "semantic_entity", op: "create", payload: { semantic_kind: "concept", home_theory: "$new:theory", preferred_labels: [{ lang: "zh", label: "母函数" }], conventions: [{ evidence_spans: ["$new:span"] }] } },
      { local_ref: "declaration", kind: "semantic_entity", op: "create", payload: { semantic_kind: "declaration", home_theory: "$new:theory", plain_label: "定理：形式幂级数的相等", declaration: { statement: { text: "A = B", evidence_spans: ["$new:span"] } } } },
      { local_ref: "alignment", kind: "alignment", op: "create", payload: { source: CITE_UNIT, target: "$new:declaration", relation: "expresses", evidence_spans: ["$new:span"] } },
      { local_ref: "debt", kind: "debt", op: "create", payload: { attached_to: "$new:alignment", obligation: "补充证明", evidence_spans: ["$new:span"] } },
      { local_ref: "scope", kind: "dialect_scope", op: "create", payload: { label: "本节约定", anchor_unit: CITE_UNIT, conventions: [{ text: "记号约定", evidence_spans: ["$new:span"] }] } },
    ],
  };
  return { id: `semantic-${callAt}`, tool: "propose_changes", args, result: { structuredContent: { status: "staged", staged_ids } }, status: "ok", callAt, resultAt };
}


async function enterSandbox(page: Page, reduceMotion = false) {
  await page.locator("#sandbox-button").click();
  page.once("dialog", (dialog) => void dialog.accept());
  if (reduceMotion) await page.evaluate(() => localStorage.setItem("mathscape:reduce-motion", "on"));
  await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
  await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
  const dock = page.getByRole("complementary", { name: "沙箱对话" });
  await expect(dock).toHaveClass(/is-sidebar/);
  return dock;
}

async function run(page: Page, dock: ReturnType<Page["getByRole"]>, script: unknown[]) {
  await page.evaluate((value) => { window.appHarness.sandboxRound.script = value as never; }, script);
  await dock.locator(".sandbox-dock-input").fill("跑一下");
  await dock.getByRole("button", { name: "发送" }).click();
}

const ghosts = (page: Page) => page.evaluate(() => window.appHarness.ghosts());
const ghostEdges = (page: Page) => page.evaluate(() => window.appHarness.ghostEdges());

test.describe("Scene tools (fourth slice)", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 860 });
    await page.goto("/tests/app.html");
    await expect(page.locator("#sandbox-button")).toBeVisible();
  });

  test("propose_changes: ghosts beside the PDF, relations one by one, staged_ids counts; replay reproduces it", async ({ page }) => {
    const dock = await enterSandbox(page);
    await run(page, dock, [step("propose_staged_57", 30, 200, onMap)]);
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud.locator(".scene-hud-title")).toContainText("提案");
    // Ghosts appear one by one (never all at once), then every relation draws in.
    await expect.poll(async () => (await ghosts(page)).length).toBeGreaterThan(0);
    const early = (await ghosts(page)).length;
    expect(early).toBeLessThan(17);
    await expect.poll(async () => (await ghosts(page)).length, { timeout: 8000 }).toBe(17);
    const edgesMid = (await ghostEdges(page)).length;
    expect(edgesMid).toBeLessThan(20);
    await expect.poll(async () => (await ghostEdges(page)).length, { timeout: 10000 }).toBe(20);
    // HUD ends with the staged_ids counts (= review overview totals).
    await expect(hud).toHaveAttribute("data-units", "17");
    await expect(hud).toHaveAttribute("data-relations", "20");
    await expect(hud).toHaveAttribute("data-spans", "20");
    await expect(hud.locator(".scene-hud-summary")).toHaveText("已暂存 · 单元 17 · 关系 20 · 片段 20");
    // Ghosts sit beside the PDF their spans cite; the PDF is the lit anchor.
    const list = await ghosts(page);
    const pdf = await page.evaluate(() => window.appHarness.nodePosition("r-cite-art"));
    const unit = await page.evaluate(() => window.appHarness.nodePosition("r-cite-unit"));
    const near = (ghost: { x: number; y: number }, at: { x: number; y: number }) => Math.hypot(ghost.x - at.x, ghost.y - at.y) < 400;
    expect(list.every((ghost) => near(ghost, pdf!) || near(ghost, { x: (pdf!.x + unit!.x) / 2, y: (pdf!.y + unit!.y) / 2 }))).toBe(true);
    // A relation to an existing object starts at that map node.
    expect((await ghostEdges(page)).filter((edge) => edge.source === "r-cite-unit").map((edge) => edge.label)).toEqual(["应用方法", "应用方法", "应用方法"]);
    expect(list.map((ghost) => ghost.label)).toContain("有限数列的母函数");
    expect(await page.evaluate(() => window.appHarness.nodeClasses("r-cite-art"))).toContain("sandbox-echo-seed");
    const labels = (await ghostEdges(page)).map((edge) => edge.label);
    expect(labels).toContain("解释");
    // Settle: every ghost is gone, nothing persisted.
    await expect(hud).toBeHidden({ timeout: 15000 });
    expect(await ghosts(page)).toHaveLength(0);
    expect(await ghostEdges(page)).toHaveLength(0);
    // Replay from the process card plays the same proposal again.
    const agent = dock.locator(".sandbox-line.role-agent").last();
    await expect(agent.locator(".sandbox-process-replay")).toBeEnabled();
    await agent.locator(".sandbox-process-replay").click();
    await expect(hud.locator(".scene-hud-summary")).toHaveText("已暂存 · 单元 17 · 关系 20 · 片段 20", { timeout: 8000 });
    await expect.poll(async () => (await ghosts(page)).length, { timeout: 8000 }).toBe(17);
    await hud.getByRole("button", { name: "停止" }).click();
    await expect(hud).toBeHidden();
    expect(await ghosts(page)).toHaveLength(0);
  });

  test("propose_changes: semantic objects get differentiated ghosts and relation feedback", async ({ page }) => {
    const dock = await enterSandbox(page);
    await run(page, dock, [semanticStep()]);
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud).toHaveAttribute("data-semantic", "6", { timeout: 8000 });
    await expect.poll(async () => (await ghosts(page)).length, { timeout: 10000 }).toBe(5);
    const semanticGhosts = await ghosts(page);
    expect(semanticGhosts.map((ghost) => ghost.label)).toEqual(expect.arrayContaining(["生成函数", "母函数", "定理：形式幂级数的相等", "补充证明", "本节约定"]));
    expect(semanticGhosts.find((ghost) => ghost.label === "生成函数")?.classes).toContain("scene-ghost-theory");
    expect(semanticGhosts.find((ghost) => ghost.label === "补充证明")?.classes).toContain("scene-ghost-debt");
    await expect.poll(async () => (await ghostEdges(page)).length, { timeout: 12000 }).toBe(5);
    const edgeClasses = (await ghostEdges(page)).map((edge) => edge.classes);
    expect(edgeClasses.some((classes) => classes.includes("scene-ghost-alignment-edge"))).toBe(true);
    expect(edgeClasses.some((classes) => classes.includes("scene-ghost-theory-edge"))).toBe(true);
    expect(edgeClasses.some((classes) => classes.includes("scene-ghost-debt-edge"))).toBe(true);
    expect(edgeClasses.some((classes) => classes.includes("scene-ghost-scope-edge"))).toBe(true);
    await expect(hud).toBeHidden({ timeout: 15000 });
    expect(await ghosts(page)).toHaveLength(0);
    expect(await ghostEdges(page)).toHaveLength(0);
  });

  test("propose_changes: a staged unit already on the map is animated in place (no duplicate ghost)", async ({ page }) => {
    const unit = stagedId("propose_staged_page8_v1", "u-page8");
    const dock = await enterSandbox(page);
    await run(page, dock, [step("propose_staged_page8_v1", 30, 120, (text) => text.split(unit).join(CITE_UNIT))]);
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeClasses("r-cite-unit")), { timeout: 6000 }).toContain("scene-propose-hit");
    expect(await ghosts(page)).toHaveLength(0);
    await expect(hud).toHaveAttribute("data-units", "1");
    await expect(hud).toHaveAttribute("data-spans", "1");
  });

  test("propose_changes: blocked items flash red with their diagnostic code", async ({ page }) => {
    const dock = await enterSandbox(page);
    await run(page, dock, [step("propose_blocked_relations", 30, 120)]);
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud.locator(".scene-hud-summary")).toHaveText("被阻塞 · 2 个问题：mixed_source_context", { timeout: 6000 });
    await expect(hud).not.toHaveClass(/is-error/);
    await expect(hud).toHaveAttribute("data-blocked", "2");
    await expect.poll(async () => (await ghosts(page)).filter((ghost) => ghost.label.startsWith("⚠ mixed_source_context")).length, { timeout: 10000 }).toBe(1);
    const blocked = (await ghosts(page)).find((ghost) => ghost.label.startsWith("⚠"))!;
    expect(blocked.classes).toContain("scene-blocked");
    const win = page.locator(".canvas-panel .scene-act[data-act='propose']");
    await expect(win).toBeVisible();
    await expect(win.locator(".scene-act-diagnostic-code").first()).toHaveText("mixed_source_context");
    await expect(win.locator(".scene-act-diagnostic-message").first()).toContainText("原文证据必须属于同一 Markdown 来源");
  });

  test("propose_changes supersedes: the old version's node fades", async ({ page }) => {
    const v1 = (text: string) => {
      const pair = JSON.parse(text) as Pair;
      (pair.result!.payload.raw.result as { structuredContent: { staged_ids: Record<string, string> } }).structuredContent.staged_ids.gone = CITE_UNIT;
      return JSON.stringify(pair);
    };
    const dock = await enterSandbox(page);
    await run(page, dock, [step("propose_staged_page8_v1", 30, 60, v1), step("propose_staged_page8_v2", 90, 120)]);
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud.locator(".scene-hud-title")).toContainText("v2 · 取代 v1", { timeout: 10000 });
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeClasses("r-cite-unit")), { timeout: 4000 }).toContain("scene-retired");
  });

  test("mathlib.search: running pill counts Loogle cold start, then candidates land one by one", async ({ page }) => {
    const dock = await enterSandbox(page);
    await run(page, dock, [step("mathlib_resident", 30, 3600)]);
    const pill = page.locator(".canvas-panel .scene-running");
    await expect(pill).toBeVisible({ timeout: 4000 });
    await expect(pill).toHaveText(/^Loogle 启动中… · \d+\.\ds$/);
    await expect(pill).toHaveAttribute("data-tool", "mathlib.search");
    const win = page.locator(".canvas-panel .scene-act[data-act='mathlib']");
    await expect(win.locator(".scene-act-query")).toHaveText('"klDiv"');
    // Result: badge, candidates (name + truncated type), the pill goes.
    await expect(win.locator(".scene-act-mode")).toHaveText("常驻", { timeout: 6000 });
    const rows = win.locator(".scene-act-candidate");
    await expect(rows.first().locator(".scene-act-candidate-name")).toHaveText("InformationTheory.klDiv");
    await expect(win.locator(".scene-act-candidate.is-pending").first()).toBeAttached();
    await expect(win.locator(".scene-act-candidate.is-pending")).toHaveCount(0, { timeout: 4000 });
    await expect(win.locator(".scene-act-took")).toHaveText(/^用时 \d/);
    await expect(pill).toBeHidden();
  });

  test("mathlib.search: no matches with one-shot fallback flag; rejected query suggestions", async ({ page }) => {
    const dock = await enterSandbox(page);
    await run(page, dock, [step("mathlib_oneshot_fallback", 30, 80), step("lean_checked", 2600, 2700), step("mathlib_rejected", 5200, 5300)]);
    const win = page.locator(".canvas-panel .scene-act[data-act='mathlib']");
    await expect(win.locator(".scene-act-empty")).toHaveText("没有匹配的声明");
    await expect(win.locator(".scene-act-mode")).toHaveText("一次性");
    await expect(win.locator(".scene-act-fallback")).toHaveText("已回退");
    await expect(win.locator(".scene-act-fallback-note")).toContainText("memory budget");
    await expect(win.locator(".scene-act-suggestion")).toHaveText(["InformationTheory.klDiv", "InformationTheory.klFun"], { timeout: 8000 });
  });

  test("lean.check: ✓ with 编译通过 ≠ 证明忠实; failures list diagnostics", async ({ page }) => {
    const dock = await enterSandbox(page);
    await run(page, dock, [step("lean_checked", 30, 80), step("lean_failed", 2800, 2900)]);
    const win = page.locator(".canvas-panel .scene-act[data-act='lean']");
    await expect(win.locator(".scene-act-verdict")).toHaveText("✓ 编译通过");
    await expect(win.locator(".scene-act-note")).toContainText("编译通过 ≠ 证明忠实");
    await expect(win.locator(".scene-act-code")).toContainText("import");
    await expect(win.locator(".scene-act-verdict[data-verdict='failed']")).toBeVisible({ timeout: 8000 });
    await expect(win.locator(".scene-act-diagnostic").first().locator(".scene-act-diagnostic-where")).toHaveText(/^\d+:\d+$/);
  });

  test("material.locate: quote typed, matched excerpt and page; a miss says why", async ({ page }) => {
    const dock = await enterSandbox(page);
    await run(page, dock, [step("locate_ok", 30, 1800), step("locate_not_found", 4200, 4300)]);
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud.locator(".scene-hud-type")).toBeVisible();
    await expect(hud.locator(".scene-hud-type-text")).toContainText("8. 证明", { timeout: 4000 });
    const win = page.locator(".canvas-panel .scene-act[data-act='locate']");
    await expect(win.locator(".scene-act-mark")).toContainText("8. 证明", { timeout: 4000 });
    await expect(win.locator(".scene-locate-quote strong")).toHaveText("8. 证明");
    await expect(win.locator(".scene-locate-quote .katex")).toHaveCount(1);
    await expect(win.locator(".scene-act-meta")).toHaveText("第 8 页");
    await expect(win.locator(".scene-act-verdict")).toHaveText("精确匹配 · 第 8 页");
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeClasses("r-cite-art"))).toContain("sandbox-echo-seed");
    await expect(win.locator(".scene-act-reason")).toHaveText("引文与原文不一致", { timeout: 8000 });
    await expect(win.locator(".scene-act-quote")).toHaveAttribute("data-phase", "miss");
  });

  for (const theme of ["obsidian", "parchment", "slate"]) {
    test(`locate reading preview and categorized diagnostics, live and replay: ${theme}`, async ({ page }, info) => {
      await page.evaluate(theme => localStorage.setItem("mathscape.theme", theme), theme);
      const dock = await enterSandbox(page);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      const quote = "**定理 7** 设方程\n\n$$p_1 x_1 + p_2 x_2 + \\dots + p_n x_n = r$$\n\n非负整数解的个数为 $b_r$。\n\n"
        + "较长的原文仍保留完整段落，不截断公式。\n\n".repeat(20)
        + "$$E = mc^2$$\n\n引文末尾。\n\n<script>window.locateUnsafe = true</script>\n\n[外部链接](https://example.com)";
      const scripted = step("locate_ok", 30, 80);
      (scripted.args as { quote: string }).quote = quote;
      const payload = (scripted.result as { structuredContent: Record<string, unknown> }).structuredContent;
      payload.page = null;
      payload.page_granularity_available = false;
      payload.match = "normalized";
      payload.diagnostics = [
        { code: "matched_raw_ocr_fallback", message: "quote matched raw OCR bytes" },
        { code: "matched_normalized", message: "matched after normalization" },
        { code: "page_markers_absent", snapshot_pages: { from: 45, to: 49 }, message: `snapshot ${CITE_ART} covers pages 45–49 but has no page markers` },
      ];
      await run(page, dock, [scripted]);
      const win = page.locator(".canvas-panel .scene-act[data-act='locate']");
      await expect(win.locator(".scene-locate-quote strong")).toHaveText("定理 7");
      await expect(win.locator(".katex")).toHaveCount(3);
      await expect(win.locator(".katex-error")).toHaveCount(0);
      await expect(win.locator(".scene-locate-quote")).toContainText("引文末尾");
      await expect(win.locator(".scene-act-verdict")).toHaveText("规范化后匹配 · 具体页码不可确定");
      await expect(win.locator(".scene-act-meta")).toHaveCount(0);
      await expect(win.locator(".scene-locate-category")).toHaveText(["原文", "匹配", "页码"]);
      await expect(win.locator(".scene-locate-message").last()).toContainText("第 45–49 页，但缺少分页标记");
      await expect(win.locator(".scene-locate-original").first()).toBeHidden();
      expect(await win.locator(".scene-locate-quote").evaluate(el => el.scrollHeight > el.clientHeight)).toBe(true);
      expect(await win.locator("script, a").count()).toBe(0);
      await page.screenshot({ path: info.outputPath(`locate-${theme}.png`), animations: "disabled" });
      await win.locator("summary").click();
      await expect(win.locator(".scene-locate-artifact")).toHaveText(CITE_ART);
      await expect(win.locator(".scene-locate-original")).toHaveCount(3);
      expect(await win.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      expect((await win.boundingBox())!.y + (await win.boundingBox())!.height).toBeLessThanOrEqual(860);
      await page.screenshot({ path: info.outputPath(`locate-details-${theme}.png`), animations: "disabled" });
      await win.locator(".scene-locate-quote").evaluate(el => el.scrollTop = el.scrollHeight);
      await expect(win.locator(".katex").last()).toBeInViewport();
      const hud = page.locator(".canvas-panel .scene-hud");
      await hud.getByRole("button", { name: "停止" }).click();
      const agent = dock.locator(".sandbox-line.role-agent").last();
      await agent.locator(".sandbox-process-summary[aria-expanded='false']").click();
      await agent.locator(".sandbox-step-replay").click();
      await expect(win.locator(".katex")).toHaveCount(3);
      await expect(win.locator(".scene-locate-category")).toHaveText(["原文", "匹配", "页码"]);
      await expect(win.locator(".scene-locate-details")).not.toHaveAttribute("open", "");
      await hud.getByRole("button", { name: "停止" }).click();
      await agent.locator(".sandbox-process-replay").click();
      await expect(win.locator(".katex")).toHaveCount(3);
      await expect(win.locator(".scene-act-verdict")).toHaveText("规范化后匹配 · 具体页码不可确定");
      await hud.getByRole("button", { name: "停止" }).click();
      await page.setViewportSize({ width: 900, height: 680 });
      await agent.locator(".sandbox-process-replay").click();
      // At this width the existing dock floats over the map; collapse it to
      // inspect the bounded excerpt instead of measuring an occluded window.
      await dock.getByTitle("收起为浮动入口").click();
      await expect(dock).toBeHidden();
      await expect(win.locator(".katex")).toHaveCount(3);
      expect(await win.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      const box = (await win.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(900);
      expect(box.y + box.height).toBeLessThanOrEqual(680);
      await page.screenshot({ path: info.outputPath(`locate-narrow-${theme}.png`), animations: "disabled" });
      await hud.getByRole("button", { name: "停止" }).click();
    });
  }

  test("change sets: light HUD; consecutive same-tool calls coalesce", async ({ page }) => {
    const dock = await enterSandbox(page);
    await run(page, dock, [
      step("propose_staged_page8_v1", 30, 60),
      step("list_change_sets_one", 100, 110),
      step("list_change_sets_empty", 120, 130),
      step("list_change_sets_one", 140, 150),
      step("get_change_set", 400, 420),
    ]);
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud.locator(".scene-hud-step")).toHaveText(/^合并 3 次$/, { timeout: 10000 });
    await expect(hud.locator(".scene-hud-summary")).toHaveText(/^1 个变更集：/);
    await expect(hud.locator(".scene-hud-title")).toHaveText(/^查看变更集 · change-set\//, { timeout: 8000 });
    await expect(hud.locator(".scene-hud-summary")).toHaveText(/^待审 · 单元 \d+ · 关系 \d+ · 片段 \d+/);
  });

  test("减少动效: proposal states switch instantly, nothing moves or types", async ({ page }) => {
    const dock = await enterSandbox(page, true);
    await expect(page.locator("html")).toHaveClass(/reduce-motion/);
    const before = await page.evaluate(() => window.appHarness.nodePosition("r-cite-art"));
    await run(page, dock, [step("propose_staged_57", 30, 100, onMap), step("locate_ok", 2600, 2700)]);
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud.locator(".scene-hud-title")).toContainText("提案");
    // All ghosts and relations at once (no one-by-one reveal).
    await expect.poll(async () => (await ghosts(page)).length, { timeout: 1500, intervals: [50] }).toBe(17);
    expect((await ghostEdges(page)).length).toBe(20);
    expect(await page.evaluate(() => window.appHarness.nodePosition("r-cite-art"))).toEqual(before);
    await expect(page.locator(".canvas-panel")).toHaveClass(/is-reduced-motion/);
    // The locate quote is shown whole at once (no typing).
    await expect(hud.locator(".scene-hud-type")).toBeVisible({ timeout: 8000 });
    await expect(hud.locator(".scene-hud-type")).not.toHaveClass(/is-typing/);
    await expect(hud.locator(".scene-hud-type-text")).toContainText("8. 证明");
  });

  test("减少动效 setting: follows the system by default, toggles in settings", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.reload();
    await expect(page.locator("html")).toHaveClass(/reduce-motion/);
    await page.locator("#llm-settings-button").click();
    const select = page.locator("select[name='reduce-motion']");
    await expect(select).toHaveValue("system");
    await select.selectOption("off");
    await expect(page.locator("html")).not.toHaveClass(/reduce-motion/);
    expect(await page.evaluate(() => localStorage.getItem("mathscape:reduce-motion"))).toBe("off");
    await select.selectOption("system");
    await expect(page.locator("html")).toHaveClass(/reduce-motion/);
    expect(await page.evaluate(() => localStorage.getItem("mathscape:reduce-motion"))).toBeNull();
  });
});
