/**
 * 外部说明链接（如教程里 mathlib 的维基百科链接）：在系统浏览器里打开，
 * 不在应用窗口里跳转。桌面端走 `open_external_link`（后端只放行维基百科的
 * https 链接）；浏览器调试页直接开新标签页。
 */
import { safeInvoke } from "./ipc";

const ALLOWED = /^https:\/\/(en|zh)\.wikipedia\.org\//;

export function isAllowedExternalLink(url: string): boolean {
  return ALLOWED.test(url);
}

export async function openExternalLink(url: string): Promise<void> {
  if (!isAllowedExternalLink(url)) return;
  if ("__TAURI_INTERNALS__" in window) {
    await safeInvoke<void>("open_external_link", { url });
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}
