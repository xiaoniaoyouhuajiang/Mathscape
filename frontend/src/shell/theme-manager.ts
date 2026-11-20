export const THEMES = {
  obsidian: "深空星图 · Obsidian",
  parchment: "暖宣手稿 · Parchment",
  slate: "青岩静夜 · Slate",
} as const;
export type Theme = keyof typeof THEMES;
export const THEME_STORAGE_KEY = "mathscape.theme";
export const THEME_CHANGED = "mathscape:themechange";

export function isTheme(value: unknown): value is Theme {
  return typeof value === "string" && Object.hasOwn(THEMES, value);
}

export function getTheme(): Theme {
  const value = document.documentElement.dataset.theme;
  return isTheme(value) ? value : "obsidian";
}

export function setTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
  try { localStorage.setItem(THEME_STORAGE_KEY, theme); } catch { /* Private/restricted storage: keep this session usable. */ }
  window.dispatchEvent(new CustomEvent(THEME_CHANGED, { detail: { theme } }));
}

export function mountThemeToggle(button: HTMLButtonElement): void {
  const sync = () => {
    const label = THEMES[getTheme()];
    button.title = `当前主题：${label}；点击切换`;
    button.setAttribute("aria-label", `切换主题，当前：${label}`);
  };
  sync();
  button.addEventListener("click", () => {
    const keys = Object.keys(THEMES) as Theme[];
    setTheme(keys[(keys.indexOf(getTheme()) + 1) % keys.length]);
  });
  window.addEventListener(THEME_CHANGED, sync);
}

/** Head bootstrap restores before paint; embedded entries can call this too. */
export function initializeTheme(): void {
  try {
    const saved = localStorage.getItem(THEME_STORAGE_KEY);
    document.documentElement.dataset.theme = isTheme(saved) ? saved : "obsidian";
  } catch { document.documentElement.dataset.theme = "obsidian"; }
  window.addEventListener("storage", event => {
    if (event.key !== THEME_STORAGE_KEY && event.key !== null) return;
    const theme = isTheme(event.newValue) ? event.newValue : "obsidian";
    document.documentElement.dataset.theme = theme;
    window.dispatchEvent(new CustomEvent(THEME_CHANGED, { detail: { theme } }));
  });
}
