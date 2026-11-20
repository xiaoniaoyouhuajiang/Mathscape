import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { errorMessage } from "../../src/core/ipc";
import { DEFAULT_LOCALE, LOCALES, formatDateTime, getLocale, setLocale, t } from "../../src/i18n";
import { en } from "../../src/i18n/en";
import { zhCN } from "../../src/i18n/zh-CN";

const placeholders = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map(match => match[1]).sort();

describe("i18n", () => {
  afterEach(() => setLocale(DEFAULT_LOCALE));

  it("keeps the same keys and placeholders in every dictionary", () => {
    const keys = Object.keys(zhCN).sort();
    for (const messages of Object.values(LOCALES)) {
      expect(Object.keys(messages).sort()).toEqual(keys);
      for (const key of keys) {
        const message = messages[key as keyof typeof zhCN];
        expect(message, key).not.toBe("");
        expect(placeholders(message), key).toEqual(placeholders(zhCN[key as keyof typeof zhCN]));
      }
    }
  });

  it("defaults to zh-CN and switches dictionaries", () => {
    expect(getLocale()).toBe("zh-CN");
    expect(t("theme.toggleTitle", { label: "A" })).toBe("当前主题：A；点击切换");
    setLocale("en");
    expect(t("theme.toggleTitle", { label: "A" })).toBe("Theme: A. Click to switch");
    expect(t("theme.toggleAriaLabel")).toBe(en["theme.toggleAriaLabel"]);
  });

  it("formats dates in the current locale", () => {
    const date = new Date(2026, 0, 2, 3, 4, 5);
    setLocale("en");
    expect(formatDateTime(date)).toBe(date.toLocaleString("en"));
  });

  it("translates errors by stable code and keeps other text", () => {
    expect(errorMessage({ code: "unknown", message: "backend text" })).toBe("发生了未知错误");
    expect(errorMessage("unknown: backend text")).toBe("发生了未知错误");
    expect(errorMessage({ code: "not_in_dictionary", message: "backend text" })).toBe("backend text");
    expect(errorMessage(new Error("proposal_not_found: 提案不存在"))).toBe("proposal_not_found: 提案不存在");
    expect(errorMessage(42)).toBe("发生了未知错误");
  });

  it("does not let Chinese literals in frontend/src grow past the baseline", () => {
    const script = fileURLToPath(new URL("../../scripts/check-i18n-ratchet.mjs", import.meta.url));
    expect(() => execFileSync(process.execPath, [script], { stdio: "pipe" })).not.toThrow();
  });
});
