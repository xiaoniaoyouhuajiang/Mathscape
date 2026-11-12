/**
 * 教程「两本教材是不是同一颗星」：帮助菜单入口、11 镜、只读（不发任何写命令）、
 * 「减少动效」降级。地图数据来自 fixtures/emir/fibonacci-two-books/tutorial-projection.json。
 */
import { expect, test, type Page } from "@playwright/test";

const TITLE = "教程：两本教材是不是同一颗星";

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
  await expect(dialog.locator(".tutorial-counter")).toContainText("第 1 / 11 镜");
  return dialog;
}

test.describe("teaser tutorial", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 860 });
  });

  test("plays all eleven shots from the help menu and never writes", async ({ page }) => {
    await page.goto("/tests/app.html");
    await expect(page.locator("#ink-button")).toBeVisible();
    await recordInvokes(page);
    const dialog = await openTutorial(page);
    await expect(dialog.locator(".tutorial-card")).toContainText("两本教材，是不是同一颗星？");
    await expect(dialog.locator(".tutorial-note")).toContainText("不会写入你的工作区");
    await expect(dialog.locator(".tutorial-map canvas").first()).toBeVisible();
    // 默认（不减少动效）：自动播放，可暂停。
    await expect(dialog.locator(".tutorial-play")).toHaveText("暂停");
    await dialog.locator(".tutorial-play").click();
    await expect(dialog.locator(".tutorial-play")).toHaveText("播放");

    const next = dialog.locator(".tutorial-next");
    const seen: string[] = [];
    for (let i = 0; i < 11; i++) {
      seen.push((await dialog.locator(".tutorial-card").getAttribute("data-shot"))!);
      if (i < 10) await next.click();
    }
    expect(seen).toEqual(["teaser", "spans", "units", "same-star", "index-zero", "cassini", "matrix", "no-merge", "plain-labels", "debts", "accept-and-learn"]);
    const card = dialog.locator(".tutorial-card");
    await expect(card.locator(".tutorial-banner")).toHaveText("接受只表示写入地图，不代表数学上已验证");
    await expect(card.locator(".tutorial-badge")).toHaveText("可信度：候选");
    await expect(card.locator(".tutorial-learning-label").filter({ hasText: "别的材料里见过" }).first()).toBeVisible();
    await expect(card.locator(".tutorial-learning-summary")).toContainText("未还清的账 4");
    for (const word of ["平移", "错开", "滑动"]) await expect(dialog).not.toContainText(word);

    // 回到第 6 镜：编号牌默认先在第 1 列，随后整排右移一格（只动“第几条”）。
    for (let i = 0; i < 5; i++) await dialog.locator(".tutorial-prev").click();
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
    await expect(dialog.locator(".tutorial-card")).toHaveAttribute("data-shot", "teaser");
    for (let i = 0; i < 5; i++) await dialog.locator(".tutorial-next").click();
    const card = dialog.locator(".tutorial-card");
    await expect(card).toHaveAttribute("data-shot", "cassini");
    await expect(card.locator(".tutorial-numbering")).toHaveClass(/is-static/);
    await expect(card.locator(".tutorial-plate-row.is-yi")).toHaveClass(/is-shifted/);
    const transition = await card.locator(".tutorial-plate-row.is-yi .tutorial-plate").first().evaluate((node) => getComputedStyle(node).transitionDuration);
    expect(transition).toBe("0s");
    await dialog.locator(".tutorial-next").click();
    await dialog.locator(".tutorial-next").click();
    await expect(card).toHaveAttribute("data-shot", "no-merge");
    await expect(card.locator(".tutorial-merge")).toHaveClass(/is-static/);
    const animation = await card.locator(".tutorial-merge-x").first().evaluate((node) => getComputedStyle(node).animationName);
    expect(animation).toBe("none");
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
  });
});
