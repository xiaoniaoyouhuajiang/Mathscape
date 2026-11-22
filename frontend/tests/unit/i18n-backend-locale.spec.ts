import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const calls: string[] = [];
let release: () => void = () => undefined;
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string, args: { locale: string }) => {
    calls.push(`${cmd}:${args.locale}`);
    return new Promise<void>(resolve => { release = resolve; });
  }),
}));

import { initializeLocale, LOCALE_CHANGED, LOCALE_STORAGE_KEY, setLocale } from "../../src/i18n";

describe("backend display locale", () => {
  let stored: Record<string, string>;
  let win: EventTarget & { __TAURI_INTERNALS__?: object };

  beforeEach(() => {
    calls.length = 0;
    stored = {};
    win = Object.assign(new EventTarget(), { __TAURI_INTERNALS__: {} });
    vi.stubGlobal("window", win);
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => stored[key] ?? null,
      setItem: (key: string, value: string) => { stored[key] = value; },
    });
  });
  afterEach(async () => {
    const reset = setLocale("zh-CN");
    release();
    await reset;
    vi.unstubAllGlobals();
  });

  it("resolves initialization only after the backend has the saved locale", async () => {
    stored[LOCALE_STORAGE_KEY] = "en";
    let ready = false;
    const done = initializeLocale().then(() => { ready = true; });
    await Promise.resolve();
    expect(calls).toEqual(["set_display_locale:en"]);
    expect(ready).toBe(false);
    release();
    await done;
    expect(ready).toBe(true);
  });

  it("announces a locale change once the backend has switched", async () => {
    const seen: string[] = [];
    win.addEventListener(LOCALE_CHANGED, () => seen.push(calls.join(",")));
    const done = setLocale("en");
    await Promise.resolve();
    expect(seen).toEqual([]);
    release();
    await done;
    expect(seen).toEqual(["set_display_locale:en"]);
  });
});
