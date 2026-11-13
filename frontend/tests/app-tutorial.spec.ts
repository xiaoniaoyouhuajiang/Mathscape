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
});
