/**
 * N3.0 shared ink palette for the tablet and the desktop: six preset swatches
 * plus a custom colour. Each side remembers its own last choice; colours are
 * not part of the ink data model beyond the stroke's brush colour, and
 * existing strokes are never recoloured.
 */
export const INK_DEFAULT_COLOR = "#193a66";
export const INK_PRESET_COLORS: ReadonlyArray<{ color: string; name: string }> = [
  { color: INK_DEFAULT_COLOR, name: "深蓝（默认）" },
  { color: "#1f2328", name: "黑" },
  { color: "#c62828", name: "红" },
  { color: "#1565c0", name: "蓝" },
  { color: "#2e7d32", name: "绿" },
  { color: "#ef6c00", name: "橙" },
];
export const PAD_COLOR_KEY = "mathscape:ink-pad-color";
export const DESKTOP_COLOR_KEY = "mathscape:ink-desktop-color";

const HEX = /^#[0-9a-f]{6}$/;

export function normalizeInkColor(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const color = value.trim().toLowerCase();
  return HEX.test(color) ? color : undefined;
}

export function loadInkColor(key: string, storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage): string {
  try { return normalizeInkColor(storage?.getItem(key)) ?? INK_DEFAULT_COLOR; } catch { return INK_DEFAULT_COLOR; }
}

export function saveInkColor(key: string, color: string, storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage): void {
  const value = normalizeInkColor(color);
  if (!value) return;
  try { storage?.setItem(key, value); } catch { /* Optional preference. */ }
}

export function isPresetColor(color: string): boolean {
  return INK_PRESET_COLORS.some(preset => preset.color === normalizeInkColor(color));
}

export interface InkPalette {
  readonly element: HTMLElement;
  value(): string;
  set(color: string): void;
  setDisabled(disabled: boolean): void;
}

/** Swatch buttons (aria-pressed) and a custom `<input type=color>`. */
export function createInkPalette(options: { storageKey: string; idPrefix: string; onChange?: (color: string) => void }): InkPalette {
  const root = document.createElement("div");
  root.className = "ink-palette";
  root.setAttribute("role", "group");
  root.setAttribute("aria-label", "笔迹颜色");
  root.title = "之后创建的笔迹使用此颜色";
  const swatches = INK_PRESET_COLORS.map(preset => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "ink-swatch";
    button.style.setProperty("--swatch", preset.color);
    button.dataset.color = preset.color;
    button.setAttribute("aria-label", preset.name);
    button.title = preset.name;
    root.append(button);
    return button;
  });
  const custom = document.createElement("input");
  custom.type = "color";
  custom.id = `${options.idPrefix}-color`;
  custom.className = "ink-swatch-custom";
  custom.setAttribute("aria-label", "自定义颜色");
  custom.title = "自定义颜色";
  root.append(custom);
  let current = loadInkColor(options.storageKey);
  const render = () => {
    for (const button of swatches) button.setAttribute("aria-pressed", String(button.dataset.color === current));
    custom.value = current;
    custom.classList.toggle("is-active", !isPresetColor(current));
    root.dataset.color = current;
  };
  const set = (color: string, persist = true) => {
    const value = normalizeInkColor(color);
    if (!value) return;
    current = value;
    if (persist) saveInkColor(options.storageKey, value);
    render();
    options.onChange?.(value);
  };
  for (const button of swatches) button.addEventListener("click", () => set(button.dataset.color!));
  custom.addEventListener("input", () => set(custom.value));
  render();
  return {
    element: root,
    value: () => current,
    set: color => set(color),
    setDisabled: disabled => { for (const control of [...swatches, custom]) control.disabled = disabled; },
  };
}
