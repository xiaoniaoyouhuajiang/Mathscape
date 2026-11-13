/**
 * 教程「两本书讲的是不是同一个东西」：帮助菜单入口、13 镜、只读（不发任何写命令）、
 * 「减少动效」降级。地图数据来自 fixtures/emir/fibonacci-two-books/tutorial-projection.json。
 */
import { expect, test, type Page } from "@playwright/test";

const TITLE = "教程：两本书讲的是不是同一个东西";

/** 记录教程打开期间的全部 Tauri 命令。 */
async function recordInvokes(page: Page) {
  await page.evaluate(() => {
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, ...rest: unknown[]) => unknown } }).__TAURI_INTERNALS__;
    const original = internals.invoke.bind(internals);
    const log: string[] = [];
    (window as unknown as { __tutorialInvokes: string[] }).__tutorialInvokes = log;
    internals.invoke = (cmd: string, ...rest: unknown[]) => {
      log.push(cmd);
      return original(cmd, ...rest);
    };
  });
}

const invokes = (page: Page) => page.evaluate(() => (window as unknown as { __tutorialInvokes: string[] }).__tutorialInvokes);

async function openTutorial(page: Page) {
  await page.locator("#help-button").click();
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  await menu.getByRole("menuitem", { name: TITLE }).click();
  const dialog = page.getByRole("dialog", { name: TITLE });
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(".tutorial-counter")).toContainText("第 1 / 13 镜");
  return dialog;
}

test.describe("teaser tutorial", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 860 });
  });

  test("plays all thirteen shots from the help menu and never writes", async ({ page }) => {
    await page.goto("/tests/app.html");
    await expect(page.locator("#ink-button")).toBeVisible();
    await recordInvokes(page);
    const dialog = await openTutorial(page);
    await expect(dialog.locator(".tutorial-card")).toContainText("地图上的两本书");
    await expect(dialog.locator(".tutorial-card")).toContainText("概念节点：Fibonacci 数列");
    await expect(dialog.locator(".tutorial-note")).toHaveCount(0);
    // 图例：沿用主地图底栏样式，只列画面上出现的类型。
    await expect(dialog.locator(".tutorial-legend")).toContainText("Markdown 材料");
    await expect(dialog.locator(".tutorial-legend")).toContainText("概念 concept");
    await expect(dialog.locator(".tutorial-map canvas").first()).toBeVisible();
    // 默认（不减少动效）：自动播放，可暂停。
    await expect(dialog.locator(".tutorial-play")).toHaveText("暂停");
    await dialog.locator(".tutorial-play").click();
    await expect(dialog.locator(".tutorial-play")).toHaveText("播放");

    const next = dialog.locator(".tutorial-next");
    const seen: string[] = [];
    const card = dialog.locator(".tutorial-card");
    const texts: string[] = [];
    for (let i = 0; i < 13; i++) {
      seen.push((await card.getAttribute("data-shot"))!);
      texts.push(`${await card.innerText()}\n${await dialog.locator(".tutorial-caption").innerText()}\n${await dialog.locator(".tutorial-legend").innerText()}`);
      if (seen[i] === "spans") {
        // 原文片段按 Markdown + TeX 渲染：不露出 ** 和 $。
        await expect(card.locator(".tutorial-material-body strong").first()).toHaveText("定义 3.2.1（Fibonacci 数列）");
        await expect(card.locator(".tutorial-material-body .katex").first()).toBeVisible();
        await expect(card.locator(".tutorial-material-file")).toHaveText(["星港数列讲义.md", "灯塔组合数学初步.md"]);
      }
      if (seen[i] === "units") {
        await expect(dialog.locator(".tutorial-legend")).toContainText("派生（虚线：来自哪份材料）");
        await expect(dialog.locator(".tutorial-legend")).toContainText("书内关系（箭头表示方向）");
      }
      if (seen[i] === "declaration") {
        const link = card.locator("a.tutorial-link").first();
        await expect(link).toHaveAttribute("href", "https://en.wikipedia.org/wiki/Lean_(proof_assistant)#Libraries");
        await expect(card).toContainText("不代表哪本书更正统");
      }
      if (seen[i] === "revision") {
        await expect(card.locator(".tutorial-revision-step.is-v1 code")).toHaveText("definitionally_equivalent");
        await expect(card.locator(".tutorial-revision-step.is-v2 code")).toHaveText("overlaps_with");
        await expect(card.locator(".tutorial-feedback-demo")).toBeDisabled();
      }
      if (seen[i] === "accept") {
        await expect(card.locator(".tutorial-banner")).toHaveText("接受只是认可这条对应，不会把可信度变成“已证明”");
        await expect(card.locator(".tutorial-badge")).toHaveText("可信度：候选");
      }
      if (i < 12) await next.click();
    }
    expect(seen).toEqual(["intro", "spans", "units", "same-concept", "declaration", "index-zero", "cassini", "matrix", "revision", "plain-labels", "debts", "accept", "recap"]);
    await expect(card.locator(".tutorial-learning-label").filter({ hasText: "别的材料里见过" }).first()).toBeVisible();
    await expect(card.locator(".tutorial-learning-summary")).toContainText("未还清的账 4");
    const all = texts.join("\n");
    for (const word of ["平移", "错开", "滑动", "甲", "乙", "虚构", "示例材料", "还没在本机核对", "**"]) expect(all).not.toContain(word);
    expect(all).not.toMatch(/星(?!港)/);

    // 回到第 7 镜：编号牌默认先在第 1 列，随后整排右移一格（只动“第几条”）。
    for (let i = 0; i < 6; i++) await dialog.locator(".tutorial-prev").click();
    await expect(card).toHaveAttribute("data-shot", "cassini");
    await expect(card.locator(".tutorial-sequence-fixed")).toHaveText("数列没有动：f_k = F_k");
    await expect(card.locator(".tutorial-plate-row.is-yi")).toHaveClass(/is-shifted/);
    await expect(card.locator(".tutorial-numbering")).not.toHaveClass(/is-static/);

    await next.click();
    await dialog.locator(".tutorial-close").click();
    await expect(dialog).toBeHidden();
    const writes = (await invokes(page)).filter((cmd) => /commit|plan|accept|propose|write|record|recheck|import|create|append|set_|delete|save/.test(cmd));
    expect(writes).toEqual([]);
  });

  test("reduced motion: instant states, no autoplay, static numbering table", async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem("mathscape:reduce-motion", "on"));
    await page.goto("/tests/app.html");
    await expect(page.locator("#ink-button")).toBeVisible();
    const dialog = await openTutorial(page);
    await expect(dialog.locator(".tutorial-play")).toBeHidden();
    await expect(dialog.locator(".tutorial-next")).toHaveText("下一步 ›");
    // 不自动推进。
    await page.waitForTimeout(1500);
    await expect(dialog.locator(".tutorial-card")).toHaveAttribute("data-shot", "intro");
    for (let i = 0; i < 6; i++) await dialog.locator(".tutorial-next").click();
    const card = dialog.locator(".tutorial-card");
    await expect(card).toHaveAttribute("data-shot", "cassini");
    await expect(card.locator(".tutorial-numbering")).toHaveClass(/is-static/);
    await expect(card.locator(".tutorial-plate-row.is-yi")).toHaveClass(/is-shifted/);
    const transition = await card.locator(".tutorial-plate-row.is-yi .tutorial-plate").first().evaluate((node) => getComputedStyle(node).transitionDuration);
    expect(transition).toBe("0s");
    await dialog.locator(".tutorial-next").click();
    await dialog.locator(".tutorial-next").click();
    await expect(card).toHaveAttribute("data-shot", "revision");
    await expect(card.locator(".tutorial-revision")).toHaveClass(/is-static/);
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
  });

  test("map labels: focused labels never overlap, edge labels are horizontal pills on the shot's lines only", async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem("mathscape:reduce-motion", "on"));
    await page.goto("/tests/app.html");
    await expect(page.locator("#ink-button")).toBeVisible();
    const dialog = await openTutorial(page);
    const card = dialog.locator(".tutorial-card");
    // 读教程画布里的 Cytoscape 实例（容器上的注册表），量标签的包围盒（模型坐标）。
    const measure = () => page.evaluate(() => {
      type Box = { x1: number; x2: number; y1: number; y2: number; w: number; h: number };
      type Ele = {
        boundingBox(o: object): Box; data(k: string): string; visible(): boolean; hasClass(c: string): boolean;
        style(k: string): string; id(): string;
      };
      type Coll = { filter(f: (e: Ele) => boolean): Ele[] };
      const cy = (document.querySelector(".tutorial-map") as unknown as { _cyreg: { cy: { nodes(): Coll; edges(): Coll } } })._cyreg.cy;
      const box = (e: Ele) => e.boundingBox({ includeNodes: false, includeEdges: false, includeLabels: true, includeOverlays: false, includeUnderlays: false });
      const hit = (a: Box, b: Box) => a.x1 < b.x2 && b.x1 < a.x2 && a.y1 < b.y2 && b.y1 < a.y2;
      const nodes = cy.nodes().filter((n) => n.visible() && Boolean(n.data("label")));
      const labeledEdges = cy.edges().filter((e) => e.visible() && e.style("label") !== "");
      const overlaps: string[] = [];
      for (const f of nodes.filter((n) => n.hasClass("tutorial-focus"))) {
        for (const o of nodes) if (o !== f && hit(box(f), box(o))) overlaps.push(`${f.data("label")} × ${o.data("label")}`);
        for (const e of labeledEdges) if (hit(box(f), box(e))) overlaps.push(`${f.data("label")} × ${e.data("label")}`);
      }
      const fib = nodes.filter((n) => n.data("label").includes("〔概念 · concept〕"))[0];
      return {
        overlaps,
        fib: fib ? { label: fib.data("label"), focused: fib.hasClass("tutorial-focus"), h: box(fib).h } : null,
        edges: labeledEdges.map((e) => ({ focus: e.hasClass("tutorial-edge-focus"), rotation: e.style("text-rotation"), label: e.data("label") })),
        raw: nodes.filter((n) => /[$\\]/.test(n.data("label"))).map((n) => n.data("label")),
      };
    });
    for (let i = 0; i < 13; i++) {
      const shot = (await card.getAttribute("data-shot"))!;
      // 换过文字的标签下一帧才重算尺寸。
      await page.waitForTimeout(150);
      const m = await measure();
      expect(m.overlaps, shot).toEqual([]);
      expect(m.raw, shot).toEqual([]);
      // 线上的字：只在这一镜要读的线上，水平放。
      for (const edge of m.edges) {
        expect(edge.focus, `${shot}: ${edge.label}`).toBe(true);
        expect(edge.rotation).toBe("none");
      }
      if (m.fib?.focused) {
        // 焦点的概念节点：〔类型〕和全名两行都在，没有被裁成一行。
        expect(m.fib.label).toBe("〔概念 · concept〕\nFibonacci 数列");
        expect(m.fib.h).toBeGreaterThan(26);
      }
      if (shot === "cassini") {
        expect(m.edges.map((e) => e.label).sort()).toEqual(["expresses\n说的就是它", "specializes\n特例：灯塔的第 n 条 ＝ 共享的第 n+1 条"]);
      }
      if (shot === "same-concept") expect(m.fib?.focused).toBe(true);
      if (i < 12) await dialog.locator(".tutorial-next").click();
    }
  });

  test("map labels: hovering a compact node shows a full uncropped pill that does not overlap other visible labels", async ({ page }) => {
    test.setTimeout(120_000);
    await page.addInitScript(() => localStorage.setItem("mathscape:reduce-motion", "on"));
    await page.goto("/tests/app.html");
    await expect(page.locator("#ink-button")).toBeVisible();
    const dialog = await openTutorial(page);
    const card = dialog.locator(".tutorial-card");
    const dense = new Set(["units", "same-concept", "declaration", "index-zero", "cassini", "matrix", "plain-labels", "debts", "recap"]);

    type NodeHit = { id: string; x: number; y: number };
    const listNodes = () => page.evaluate(() => {
      const host = document.querySelector(".tutorial-map") as HTMLElement & { _cyreg: { cy: Cy } };
      type Cy = { nodes(): NodeEle[] };
      type NodeEle = {
        visible(): boolean; data(k: string): string; id(): string;
        renderedBoundingBox(o: object): { x1: number; x2: number; y1: number; y2: number };
      };
      const cy = host._cyreg.cy;
      const r = host.getBoundingClientRect();
      const hits: NodeHit[] = [];
      for (const n of cy.nodes()) {
        if (!n.visible() || n.data("elementKind") === "junction" || !n.data("compactLabel")) continue;
        const bb = n.renderedBoundingBox({ includeNodes: true, includeLabels: false, includeOverlays: false });
        const x = r.left + (bb.x1 + bb.x2) / 2;
        const y = r.top + (bb.y1 + bb.y2) / 2;
        if (x < r.left + 4 || x > r.right - 4 || y < r.top + 4 || y > r.bottom - 4) continue;
        hits.push({ id: n.id(), x, y });
      }
      return hits;
    });

    const inspect = (id: string) => page.evaluate((nodeId) => {
      type Box = { x1: number; x2: number; y1: number; y2: number; w: number; h: number };
      type Ele = {
        boundingBox(o: object): Box; data(k: string): string; visible(): boolean; hasClass(c: string): boolean;
        style(k: string): string; id(): string; numericStyle(k: string): number;
      };
      const host = document.querySelector(".tutorial-map") as HTMLElement & { _cyreg: { cy: { nodes(): Ele[]; edges(): Ele[]; getElementById(id: string): Ele } } };
      const cy = host._cyreg.cy;
      const boxOf = (e: Ele) => e.boundingBox({ includeNodes: false, includeEdges: false, includeLabels: true, includeOverlays: false, includeUnderlays: false });
      const hit = (a: Box, b: Box) => a.x1 < b.x2 && b.x1 < a.x2 && a.y1 < b.y2 && b.y1 < a.y2;
      const node = cy.getElementById(nodeId);
      const label = String(node.data("label") ?? "");
      const hovered = node.hasClass("label-hover");
      const bg = Number.parseFloat(node.style("text-background-opacity") || "0");
      const others = [...cy.nodes(), ...cy.edges()].filter((e) => {
        if (e.id() === nodeId || !e.visible() || e.hasClass("label-recess")) return false;
        const text = e.style("label") || e.data("label") || "";
        const opacity = Number.parseFloat(e.style("text-opacity") || "1");
        return Boolean(text) && opacity > 0.05;
      });
      const overlaps = others.filter((e) => hit(boxOf(node), boxOf(e))).map((e) => `${label} × ${e.data("label") || e.style("label")}`);
      const ctx = document.createElement("canvas").getContext("2d")!;
      ctx.font = `${node.style("font-size") || "12px"} ${node.style("font-family") || "sans-serif"}`;
      const measure = (s: string) => ctx.measureText(s).width;
      const maxW = Number.parseFloat(node.style("text-max-width") || "220") || 220;
      const lines: string[] = [];
      for (const para of label.split("\n")) {
        if (measure(para) <= maxW) { lines.push(para); continue; }
        const words = para.split(/[ \t]+/);
        let line = "";
        for (const word of words) {
          const trial = line ? `${line} ${word}` : word;
          if (measure(trial) <= maxW) line = trial;
          else { if (line) lines.push(line); line = word; }
        }
        if (line) lines.push(line);
      }
      const textW = Math.max(0, ...lines.map(measure));
      const size = node.numericStyle("font-size") || 12;
      const textH = Math.max(1, lines.length) * size * 1.2;
      const bb = boxOf(node);
      const cropped = bb.w + 4 < textW || bb.h + 4 < textH;
      const full = String(node.data("fullLabel") ?? "");
      return {
        hovered, overlaps, cropped, backgroundOpacity: bg, label, full,
        expanded: !full || label === full, bb: { w: bb.w, h: bb.h }, textW, textH,
      };
    }, id);

    for (let i = 0; i < 13; i++) {
      const shot = (await card.getAttribute("data-shot"))!;
      await page.waitForTimeout(150);
      if (dense.has(shot)) {
        const nodes = await listNodes();
        for (const node of nodes) {
          await page.mouse.move(node.x, node.y);
          await page.waitForTimeout(120);
          const m = await inspect(node.id);
          expect(m.hovered, `${shot} ${node.id}`).toBe(true);
          expect(m.expanded, `${shot}: ${m.label}`).toBe(true);
          expect(m.overlaps, `${shot}: ${m.label}`).toEqual([]);
          expect(m.cropped, `${shot}: ${m.label} box=${JSON.stringify(m.bb)} text=${m.textW}x${m.textH}`).toBe(false);
          expect(m.backgroundOpacity, `${shot}: ${m.label}`).toBeGreaterThan(0.8);
        }
        await page.mouse.move(8, 8);
        await page.waitForTimeout(150);
      }
      if (i < 12) await dialog.locator(".tutorial-next").click();
    }
  });

  test("accept shot shows this alignment's debt; recap merges units and highlights on row hover", async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem("mathscape:reduce-motion", "on"));
    await page.goto("/tests/app.html");
    await expect(page.locator("#ink-button")).toBeVisible();
    const dialog = await openTutorial(page);
    const card = dialog.locator(".tutorial-card");
    for (let i = 0; i < 11; i++) await dialog.locator(".tutorial-next").click();
    await expect(card).toHaveAttribute("data-shot", "accept");
    await expect(card.locator(".tutorial-accept-demo")).toHaveText("接受这条对齐");
    await expect(card.locator(".tutorial-accept-demo")).toBeDisabled();
    await expect(card.locator(".tutorial-trust-axes")).toContainText("这条线上的账：1 笔还欠着（前向证书）");
    await expect(dialog.locator(".tutorial-caption")).toContainText("这条线上的那一笔账也还挂着");
    await expect(dialog.locator(".tutorial-caption")).not.toContainText("那几笔账");

    const inspect = () => page.evaluate(() => {
      type Ele = {
        visible(): boolean; hasClass(c: string): boolean; data(k: string): string;
        boundingBox(o: object): { x1: number; x2: number; y1: number; y2: number };
        classes(): string[];
      };
      type Coll = { filter(f: (e: Ele) => boolean): Ele[] };
      const cy = (document.querySelector(".tutorial-map") as unknown as {
        _cyreg: { cy: { nodes(): Coll; edges(): Coll; extent(): { x1: number; x2: number; y1: number; y2: number } } };
      })._cyreg.cy;
      const extent = cy.extent();
      const inView = (bb: { x1: number; x2: number; y1: number; y2: number }) =>
        bb.x1 < extent.x2 && bb.x2 > extent.x1 && bb.y1 < extent.y2 && bb.y2 > extent.y1;
      const nodes = cy.nodes().filter((n) => n.visible()).map((n) => ({
        label: n.data("label"),
        domainKind: n.data("domainKind"),
        focus: n.hasClass("tutorial-focus"),
        seen: n.hasClass("tutorial-seen"),
        fresh: n.hasClass("tutorial-new"),
        accent: n.hasClass("tutorial-accent"),
        row: n.hasClass("tutorial-row-focus"),
        hidden: n.hasClass("tutorial-hidden"),
        inView: inView(n.boundingBox({ includeLabels: true, includeNodes: true, includeEdges: false, includeOverlays: false, includeUnderlays: false })),
      }));
      const edges = cy.edges().filter((e) => e.visible()).map((e) => ({
        classes: e.classes(),
        focus: e.hasClass("tutorial-edge-focus"),
        row: e.hasClass("tutorial-row-focus"),
        hidden: e.hasClass("tutorial-hidden"),
        labeled: e.hasClass("tutorial-edge-labeled"),
      }));
      return { nodes, edges };
    });

    const accept = await inspect();
    const debt = accept.nodes.find((n) => n.domainKind === "Debt" && n.focus);
    expect(debt, "focused debt on accept shot").toBeTruthy();
    expect(debt!.hidden).toBe(false);
    expect(debt!.inView).toBe(true);
    expect(accept.edges.some((e) => e.classes.includes("debt-edge") && e.focus && !e.hidden)).toBe(true);

    await dialog.locator(".tutorial-next").click();
    await expect(card).toHaveAttribute("data-shot", "recap");
    const rows = card.locator(".tutorial-learning-units li");
    await expect(rows).toHaveCount(4);
    await expect(rows.filter({ hasText: "定义 2.4.1" })).toHaveCount(1);
    await expect(rows.filter({ hasText: "定义 2.4.1" })).toContainText("见过：");
    await expect(rows.filter({ hasText: "习题 2.4.7" })).toContainText("含「习题 2.4.7 的提示」");
    await expect(dialog.locator(".tutorial-legend-note")).toHaveText("绿框＝别的材料里见过　黄框＝新的");

    const recap = await inspect();
    expect(recap.nodes.filter((n) => n.seen).length).toBeGreaterThanOrEqual(3);
    expect(recap.nodes.filter((n) => n.fresh).length).toBeGreaterThanOrEqual(2);
    expect(recap.nodes.some((n) => n.seen && n.label.includes("定义"))).toBe(true);
    expect(recap.nodes.some((n) => n.fresh && n.label.includes("习题 2.4.7") && !n.label.includes("提示"))).toBe(true);
    expect(recap.nodes.some((n) => n.fresh && n.label.includes("提示"))).toBe(true);
    expect(recap.nodes.filter((n) => n.domainKind === "Debt" && n.accent && n.inView)).toHaveLength(4);
    expect(recap.nodes.some((n) => n.accent && n.domainKind === "DialectScope" && n.inView)).toBe(true);

    await rows.filter({ hasText: "定义 2.4.1" }).hover();
    const hovered = await inspect();
    expect(hovered.nodes.filter((n) => n.row).length).toBeGreaterThan(0);
    expect(hovered.nodes.some((n) => n.row && n.label.includes("定义 2.4.1"))).toBe(true);
    expect(card.locator(".tutorial-learning-units li.is-active")).toHaveCount(1);
  });
});
