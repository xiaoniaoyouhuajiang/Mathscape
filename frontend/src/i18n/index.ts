import { invoke } from "@tauri-apps/api/core";
import { en } from "./en";
import { zhCN, type MessageKey, type Messages } from "./zh-CN";

export type { MessageKey } from "./zh-CN";

export const LOCALES = { "zh-CN": zhCN, en } as const satisfies Record<string, Messages>;
export type Locale = keyof typeof LOCALES;
export const DEFAULT_LOCALE: Locale = "zh-CN";
export const LOCALE_STORAGE_KEY = "mathscape.locale";
export const LOCALE_CHANGED = "mathscape:localechange";

let current: Locale = DEFAULT_LOCALE;

export function isLocale(value: unknown): value is Locale {
  return typeof value === "string" && Object.hasOwn(LOCALES, value);
}

export function getLocale(): Locale {
  return current;
}

export function hasMessage(key: string): key is MessageKey {
  return Object.hasOwn(zhCN, key);
}

/** `{name}` placeholders are replaced from `params`; unknown placeholders stay as written. */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  const template = LOCALES[current][key] || zhCN[key];
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    Object.hasOwn(params, name) ? String(params[name]) : match);
}

export function formatDateTime(value: Date | number, options?: Intl.DateTimeFormatOptions): string {
  return new Date(value).toLocaleString(current, options);
}

function applyDocumentLang(): void {
  if (typeof document !== "undefined") document.documentElement.lang = current;
  // The desktop backend names shared objects in this language too.
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
    invoke("set_display_locale", { locale: current }).catch(() => undefined);
  }
}

export function setLocale(locale: Locale): void {
  current = locale;
  applyDocumentLang();
  try { localStorage.setItem(LOCALE_STORAGE_KEY, locale); } catch { /* Restricted storage: keep this session usable. */ }
  if (typeof window !== "undefined") window.dispatchEvent(new CustomEvent(LOCALE_CHANGED, { detail: { locale } }));
}

/**
 * Restores the saved locale. The default stays zh-CN rather than following the
 * system language until the English dictionary covers the whole UI.
 */
export function initializeLocale(): void {
  try {
    const saved = localStorage.getItem(LOCALE_STORAGE_KEY);
    current = isLocale(saved) ? saved : DEFAULT_LOCALE;
  } catch { current = DEFAULT_LOCALE; }
  applyDocumentLang();
}
