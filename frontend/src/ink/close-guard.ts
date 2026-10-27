import { getCurrentWindow } from "@tauri-apps/api/window";

export type InkCloseGuardOptions = {
  /** Waits for the save and asks the user; `false` keeps the window open. */
  confirm: () => Promise<boolean>;
  /** Synchronous hint for the browser `beforeunload` fallback. */
  hasUnsavedHint: () => boolean;
};

const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/**
 * H5: guard closing the app while 手稿 operations are unsaved.
 *
 * In Tauri the window's close request is intercepted (`onCloseRequested`):
 * when `confirm` resolves `false` the close is prevented, otherwise Tauri
 * destroys the window (needs `core:window:allow-destroy`). A failing guard
 * never traps the user — errors let the window close. In a plain browser the
 * synchronous `beforeunload` prompt is the only option.
 */
export function installInkCloseGuard(options: InkCloseGuardOptions): void {
  if (isTauri()) {
    try {
      void getCurrentWindow().onCloseRequested(async event => {
        let allowed = true;
        try { allowed = await options.confirm(); } catch (error) { console.error("手稿关闭检查失败", error); }
        if (!allowed) event.preventDefault();
      }).catch(error => console.error("无法监听窗口关闭", error));
    } catch (error) {
      console.error("无法监听窗口关闭", error);
    }
    return;
  }
  window.addEventListener("beforeunload", event => {
    if (!options.hasUnsavedHint()) return;
    event.preventDefault();
    event.returnValue = "";
  });
}
