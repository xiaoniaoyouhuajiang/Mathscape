import { test,expect } from "@playwright/test";
test.beforeEach(async({page})=>{await page.goto("/tests/registration.html");await page.waitForFunction(()=>"registrationHarness" in window);});
test("withdrawal preview explains shared protection, renders highlighted math and requires re-preview after reason edits",async({page})=>{
  await page.evaluate(async()=>await (window as any).registrationHarness.open());
  const workspace=page.locator(".registration-workspace");await expect(workspace).toBeVisible();
  await expect(workspace).toContainText("将退役 2 个对象，保留 1 个");
  await page.getByRole("button",{name:"将退役 · 内容单元 · 定理",exact:true}).click();
  await expect(workspace.locator(".review-fragment .katex")).toHaveCount(2);await expect(workspace.locator(".is-anchor-block")).toBeVisible();
  await page.getByLabel("撤回原因").fill("重新分割这份材料");await expect(page.getByRole("button",{name:"确认撤回",exact:true})).toBeDisabled();
  await page.getByRole("button",{name:"重新计算影响",exact:true}).click();await expect(page.getByRole("button",{name:"确认撤回",exact:true})).toBeEnabled();
  await page.getByRole("button",{name:"确认撤回",exact:true}).click();await expect(workspace).toContainText("已撤回登记：退役 2，保留 1");
  expect(await page.evaluate(()=>(window as any).registrationHarness.commits[0].request.reason)).toBe("重新分割这份材料");
  await page.getByRole("button",{name:"取消 / 关闭",exact:true}).click();await expect(workspace).toHaveCount(0);
});
test("conflict preserves reason and disables stale confirmation until re-preview",async({page})=>{
  await page.evaluate(async()=>{const h=(window as any).registrationHarness;h.behavior.conflict=true;await h.open();});
  await page.getByRole("button",{name:"确认撤回",exact:true}).click();await expect(page.getByRole("status")).toContainText("base_commit_conflict");
  await expect(page.getByLabel("撤回原因")).toHaveValue("重新整理这次发现结果");await expect(page.getByRole("button",{name:"确认撤回",exact:true})).toBeDisabled();
  await page.getByRole("button",{name:"重新计算影响",exact:true}).click();await expect(page.getByRole("button",{name:"确认撤回",exact:true})).toBeEnabled();
});
test("lost response retries the same request and historical previews still work after withdrawal",async({page})=>{
  await page.evaluate(async()=>{const h=(window as any).registrationHarness;h.behavior.lost=true;await h.open();});
  await page.getByRole("button",{name:"确认撤回",exact:true}).click();await expect(page.getByRole("status")).toContainText("connection closed after commit");
  await page.getByRole("button",{name:"确认撤回",exact:true}).click();await expect(page.getByRole("status")).toContainText("提交成功");
  const requests=await page.evaluate(()=>(window as any).registrationHarness.commits);expect(requests[0]).toEqual(requests[1]);
  await page.getByRole("button",{name:"取消 / 关闭",exact:true}).click();await expect(page.locator(".registration-workspace")).toHaveCount(0);await page.evaluate(async()=>await (window as any).registrationHarness.history());
  await expect(page.locator(".registration-workspace")).toContainText("登记当时的 revision");
  await page.getByRole("button",{name:/内容单元 · 定理/}).first().click();await expect(page.locator(".review-fragment .katex")).toHaveCount(2);
});
test("replacement diff renders math, requires explicit approval, then confirmation",async({page})=>{
  await page.evaluate(()=>(window as any).registrationHarness.register());
  await expect(page.getByRole("button",{name:"确认登记",exact:true})).toBeDisabled();
  await page.getByRole("button",{name:"阻断 · 内容单元 · 定理",exact:true}).click();
  await expect(page.locator(".registration-diff .katex")).toHaveCount(2);
  await page.locator(".registration-approval input").check();await expect(page.getByRole("button",{name:"确认登记",exact:true})).toBeEnabled();
  await page.getByRole("button",{name:"确认登记",exact:true}).click();await expect(page.locator(".registration-workspace")).toContainText("重新激活 1");
  expect(await page.evaluate(()=>(window as any).registrationHarness.registered())).toBe(1);
});
test("unavailable original preview never becomes a withdrawal blocker",async({page})=>{
  await page.evaluate(async()=>{const h=(window as any).registrationHarness;h.behavior.missingPreview=true;await h.open();});
  await page.getByRole("button",{name:"将退役 · 内容单元 · 定理",exact:true}).click();await expect(page.locator(".registration-preview")).toContainText("预览不可用");
  await expect(page.getByRole("button",{name:"确认撤回",exact:true})).toBeEnabled();
});
test("relation endpoint changes show real before/after roles and IDs rather than null",async({page})=>{
  await page.evaluate(()=>{const h=(window as any).registrationHarness;h.behavior.relationDifference=true;h.register();});
  await page.getByRole("button",{name:"阻断 · 来源关系 · 引用关系",exact:true}).click();
  await expect(page.locator(".registration-diff")).toContainText("mathscape:test/old-theorem");
  await expect(page.locator(".registration-diff")).toContainText("mathscape:test/new-theorem");
  await expect(page.locator(".registration-diff")).toContainText("target");
});
test("registered history binds the saved conversion even when another source appears first",async({page})=>{
  await page.evaluate(async()=>{const h=(window as any).registrationHarness;h.behavior.multipleContexts=true;await h.history();});
  await page.getByRole("button",{name:/内容单元 · 定理/}).first().click();
  await expect(page.locator(".review-fragment .katex")).toHaveCount(2);
  await expect.poll(()=>page.evaluate(()=>(window as any).registrationHarness.backlinkRequests.length)).toBeGreaterThan(0);
  const result=await page.evaluate(()=>{const h=(window as any).registrationHarness;return {md:h.markdownRequests.at(-1).conversionId,pdf:h.backlinkRequests.at(-1).conversion_id,expected:h.markerConversion};});
  expect(result.md).toBe(result.expected);expect(result.pdf).toBe(result.expected);
});
test("withdrawal preview uses the patched Markdown conversion for both reading and backlink",async({page})=>{
  await page.evaluate(async()=>{const h=(window as any).registrationHarness;h.behavior.patched=true;await h.open();});
  await page.getByRole("button",{name:"将退役 · 内容单元 · 定理",exact:true}).click();
  await expect(page.locator(".review-fragment .katex")).toHaveCount(2);
  await expect.poll(()=>page.evaluate(()=>(window as any).registrationHarness.backlinkRequests.length)).toBeGreaterThan(0);
  const result=await page.evaluate(()=>{const h=(window as any).registrationHarness;return {md:h.markdownRequests.at(-1).conversionId,pdf:h.backlinkRequests.at(-1).conversion_id,expected:h.patchConversion};});
  expect(result.md).toBe(result.expected);expect(result.pdf).toBe(result.expected);
});
test("missing saved hint with multiple sources requires explicit selection, not the first conversion",async({page})=>{
  await page.evaluate(async()=>{const h=(window as any).registrationHarness;h.behavior.noHint=true;h.behavior.multipleContexts=true;await h.history();});
  await page.getByRole("button",{name:/内容单元 · 定理/}).first().click();
  await expect(page.getByLabel("预览转换来源")).toBeVisible();
  expect(await page.evaluate(()=>(window as any).registrationHarness.markdownRequests)).toHaveLength(0);
  const selected=await page.evaluate(()=>(window as any).registrationHarness.markerConversion);
  await page.getByLabel("预览转换来源").selectOption(selected);
  await expect(page.locator(".review-fragment .katex")).toHaveCount(2);
  await expect.poll(()=>page.evaluate(()=>(window as any).registrationHarness.backlinkRequests.at(-1)?.conversion_id)).toBe(selected);
});
test("an outside dependency never inherits a conversion from another Markdown",async({page})=>{
  await page.evaluate(async()=>{const h=(window as any).registrationHarness;h.behavior.otherArtifactHint=true;h.behavior.invalidHint=true;await h.open();});
  await page.getByRole("button",{name:"将退役 · 内容单元 · 定理",exact:true}).click();
  await expect(page.locator(".review-fragment .katex")).toHaveCount(2);
  expect(await page.evaluate(()=>{const h=(window as any).registrationHarness;return h.markdownRequests.at(-1).conversionId===h.markerConversion;})).toBe(true);
});
test("a mismatched saved conversion remains an explicit error, without fallback or a commit blocker",async({page})=>{
  await page.evaluate(async()=>{const h=(window as any).registrationHarness;h.behavior.invalidHint=true;await h.open();});
  await page.getByRole("button",{name:"将退役 · 内容单元 · 定理",exact:true}).click();
  await expect(page.locator(".registration-preview")).toContainText("转换上下文不属于此 Markdown");
  await expect(page.getByRole("button",{name:"确认撤回",exact:true})).toBeEnabled();
});
test("standalone imported Markdown still previews without any conversion context",async({page})=>{
  await page.evaluate(async()=>{const h=(window as any).registrationHarness;h.behavior.standalone=true;await h.history();});
  await page.getByRole("button",{name:/内容单元 · 定理/}).first().click();
  await expect(page.locator(".review-fragment .katex")).toHaveCount(2);
  expect(await page.evaluate(()=>(window as any).registrationHarness.markdownRequests.at(-1).conversionId)).toBeNull();
});
