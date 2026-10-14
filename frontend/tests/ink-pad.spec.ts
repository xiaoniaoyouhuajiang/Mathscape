import { expect,test,type Page } from "@playwright/test";
import { readFile,writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const output=fileURLToPath(new URL("../../.test-runs/ink-lan-browser/",import.meta.url));
let serial=Date.now();
async function control(action:string){const id=++serial;await writeFile(output+"control.json",JSON.stringify({id,action}));await expect.poll(async()=>{try{return JSON.parse(await readFile(output+"ready.json","utf8")).action_id;}catch{return 0;}}).toBe(id);return JSON.parse(await readFile(output+"ready.json","utf8"));}
async function writeLine(page:Page,dy=0){const canvas=page.locator("#pad-board .ink-input");await expect(canvas).toBeVisible();const box=(await canvas.boundingBox())!,x=box.x+box.width/2,y=box.y+box.height/2+dy;await page.mouse.move(x-60,y);await page.mouse.down();await page.mouse.move(x+60,y,{steps:10});await page.mouse.up();}
test.beforeEach(async({page})=>{const fixture=await control("fresh");await page.goto(fixture.url);await expect(page.locator("#pad-connection")).toContainText("电脑已连接");});
test.afterEach(async()=>{await control("resume");});

test("HTTPS InkPad automatically persists writing and creates a desktop snapshot",async({page})=>{
  await writeLine(page);await expect(page.locator("#pad-local-status")).toContainText("待同步 0");await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  const state=await control("status");expect(state.document.document.operations).toHaveLength(1);
  await page.getByRole("button",{name:"保存快照",exact:true}).click();await expect(page.locator("#pad-status")).toContainText("电脑已保存快照");
  expect((await control("status")).snapshots).toHaveLength(1);
  await page.reload();await expect(page.locator("#pad-desktop-status")).toContainText("head 1");await expect(page.locator("#pad-connection")).toContainText("电脑已连接");
});

test("the tablet writing surface refits when changing from landscape to portrait",async({page})=>{
  await page.setViewportSize({width:1280,height:800});
  await page.setViewportSize({width:800,height:1280});
  await expect.poll(()=>page.locator("#pad-board .ink-settled").evaluate((element)=>{
    const canvas=element as HTMLCanvasElement;
    return canvas.getContext("2d")!.getImageData(Math.floor(canvas.width/2),Math.floor(canvas.height/2),1,1).data[3];
  })).toBe(255);
  await writeLine(page);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  expect((await control("status")).document.document.operations).toHaveLength(1);
});

test("offline handwriting survives page reload and drains after the computer returns",async({page,context})=>{
  await writeLine(page,-30);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  await expect.poll(()=>page.evaluate(()=>!!navigator.serviceWorker.controller)).toBe(true);
  await control("pause");await context.setOffline(true);await writeLine(page,30);await expect(page.locator("#pad-local-status")).toContainText("待同步 1");
  await page.reload();await expect(page.locator("#pad-local-status")).toContainText("待同步 1");
  await context.setOffline(false);await control("resume");await expect(page.locator("#pad-desktop-status")).toContainText("head 2");await expect(page.locator("#pad-local-status")).toContainText("待同步 0");
  expect((await control("status")).document.document.operations).toHaveLength(2);
});

test("desktop process restart restores the same authenticated writer and operation history",async({page})=>{
  await writeLine(page);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");await control("restart");await expect(page.locator("#pad-connection")).toContainText("电脑已连接");
  await writeLine(page,40);await expect(page.locator("#pad-desktop-status")).toContainText("head 2");expect((await control("status")).document.document.operations).toHaveLength(2);
});

test("cached drafts retain their own writer and history when switching documents",async({page})=>{
  await writeLine(page);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  const original=await page.locator("#pad-drafts").inputValue();
  const next=await control("fresh");await page.goto(next.url);await expect(page.locator("#pad-connection")).toContainText("电脑已连接");
  await expect(page.locator("#pad-desktop-status")).toContainText("head 0");const second=await page.locator("#pad-drafts").inputValue();
  await writeLine(page,30);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  await page.locator("#pad-drafts").selectOption(original);await expect(page.locator("#pad-connection")).toContainText("电脑已连接");await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  await writeLine(page,-30);await expect(page.locator("#pad-desktop-status")).toContainText("head 2");
  await page.locator("#pad-drafts").selectOption(second);await expect(page.locator("#pad-connection")).toContainText("电脑已连接");await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  expect((await control("status")).document.document.operations).toHaveLength(1);
});

test("an old epoch keeps its offline draft after the desktop takes back writing",async({page,context})=>{
  await writeLine(page,-30);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");await context.setOffline(true);await control("pause");await writeLine(page,30);await expect(page.locator("#pad-local-status")).toContainText("待同步 1");
  await control("reclaim");await control("resume");await context.setOffline(false);await expect(page.locator("#pad-connection")).toContainText("会话");
  await expect(page.locator("#pad-local-status")).toContainText("待同步 1");expect((await control("status")).document.document.operations).toHaveLength(1);
  const downloaded=page.waitForEvent("download");await page.getByRole("button",{name:"导出恢复草稿"}).click();const file=await downloaded;const json=JSON.parse(await readFile((await file.path())!,"utf8"));expect(json.operations).toHaveLength(2);
});

test("a failed local transaction preserves the drawing and retries the original operation",async({page})=>{
  await page.evaluate(()=>{
    const original=IDBDatabase.prototype.transaction;
    const state=globalThis as typeof globalThis & {restoreInkStorage?:()=>void};
    IDBDatabase.prototype.transaction=function(...args:Parameters<typeof original>){
      const transaction=original.apply(this,args);
      if(args[1]==="readwrite"&&Array.from(transaction.objectStoreNames).includes("operations"))queueMicrotask(()=>transaction.abort());
      return transaction;
    };
    state.restoreInkStorage=()=>{IDBDatabase.prototype.transaction=original;};
  });
  await writeLine(page);await expect(page.locator("#pad-local-status")).toContainText("未保存");
  await expect(page.locator("#pad-status")).toContainText("本机保存失败");
  await expect(page.getByRole("button",{name:"笔",exact:true})).toBeDisabled();
  expect((await control("status")).document.document.operations).toHaveLength(0);
  const downloaded=page.waitForEvent("download");await page.getByRole("button",{name:"导出恢复草稿"}).click();
  expect(JSON.parse(await readFile((await (await downloaded).path())!,"utf8")).operations).toHaveLength(1);
  await page.evaluate(()=>(globalThis as typeof globalThis & {restoreInkStorage?:()=>void}).restoreInkStorage?.());
  await page.getByRole("button",{name:"立即同步",exact:true}).click();
  await expect(page.locator("#pad-desktop-status")).toContainText("head 1");await expect(page.locator("#pad-local-status")).toContainText("待同步 0");
  await page.reload();await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  expect((await control("status")).document.document.operations).toHaveLength(1);
});
