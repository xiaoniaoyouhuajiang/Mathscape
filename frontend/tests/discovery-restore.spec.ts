import { test,expect } from "@playwright/test";
test.beforeEach(async({page})=>{await page.goto("/tests/reader.html");await page.waitForFunction(()=>"harness" in window);await page.evaluate(async()=>await (window as any).harness.prepareRecovery());});
test("discarded result opens read-only, restores without acceptance, then permits explicit acceptance",async({page})=>{
  await page.getByRole("button",{name:"打开审核结果",exact:true}).click();
  const review=page.locator(".review-workspace");await expect(review).toBeVisible();
  await review.getByRole("tab",{name:"单元 2",exact:true}).click();
  await expect(review.locator(".review-unit-checkbox").first()).toBeDisabled();
  await review.getByRole("button",{name:"关闭",exact:true}).click();
  await page.getByRole("button",{name:"恢复审核",exact:true}).click();await expect(review).toBeVisible();
  await review.getByRole("tab",{name:"单元 2",exact:true}).click();await expect(review.locator(".review-unit-checkbox").first()).toBeEnabled();
  await expect(review.getByRole("button",{name:"接受选中内容",exact:true})).toBeEnabled();
  expect(await page.evaluate(()=>(window as any).harness.acceptanceRequests)).toHaveLength(0);
  await expect(page.locator("#ledger-toast")).toContainText("未调用 LLM");
  await review.getByRole("button",{name:"接受选中内容",exact:true}).click();
  expect(await page.evaluate(()=>(window as any).harness.acceptanceRequests)).toHaveLength(1);
});
test("failed revalidation leaves a discarded result and its restore action intact",async({page})=>{
  await page.evaluate(()=>(window as any).harness.setRestoreFailure(true));
  await page.getByRole("button",{name:"恢复审核",exact:true}).click();
  await expect(page.locator("#ledger-toast")).toContainText("恢复验证失败");
  await expect(page.getByRole("button",{name:"恢复审核",exact:true})).toBeEnabled();
  await expect(page.locator("#discovery-ledger")).toContainText("已放弃");
});
test("retry after a lost restore response reuses the same request",async({page})=>{
  await page.evaluate(()=>(window as any).harness.setRestoreLostResponse(true));
  await page.getByRole("button",{name:"恢复审核",exact:true}).click();await expect(page.locator("#ledger-toast")).toContainText("响应丢失");
  await page.getByRole("button",{name:"恢复审核",exact:true}).click();await expect(page.locator(".review-workspace")).toBeVisible();
  const requests=await page.evaluate(()=>(window as any).harness.dispositionRequests);expect(requests[0].request).toEqual(requests[1].request);
});
