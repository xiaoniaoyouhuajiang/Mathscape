import { describe, expect, it } from "vitest";
import { DESKTOP_COLOR_KEY, INK_DEFAULT_COLOR, INK_PRESET_COLORS, PAD_COLOR_KEY, isPresetColor, loadInkColor, normalizeInkColor, saveInkColor } from "../../src/ink/palette";

const memory = () => { const values = new Map<string, string>(); return { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); }, values }; };

describe("ink palette", () => {
  it("has six distinct presets with the shared default first", () => {
    expect(INK_DEFAULT_COLOR).toBe("#193a66");
    expect(INK_PRESET_COLORS).toHaveLength(6);
    expect(INK_PRESET_COLORS[0].color).toBe(INK_DEFAULT_COLOR);
    expect(new Set(INK_PRESET_COLORS.map(preset => preset.color)).size).toBe(6);
    for (const preset of INK_PRESET_COLORS) expect(normalizeInkColor(preset.color)).toBe(preset.color);
  });

  it("defaults to #193a66 and remembers each side separately", () => {
    const storage = memory();
    expect(loadInkColor(PAD_COLOR_KEY, storage)).toBe(INK_DEFAULT_COLOR);
    expect(loadInkColor(DESKTOP_COLOR_KEY, storage)).toBe(INK_DEFAULT_COLOR);
    saveInkColor(PAD_COLOR_KEY, "#C62828", storage);
    saveInkColor(DESKTOP_COLOR_KEY, "#123456", storage);
    expect(loadInkColor(PAD_COLOR_KEY, storage)).toBe("#c62828");
    expect(loadInkColor(DESKTOP_COLOR_KEY, storage)).toBe("#123456");
    expect(isPresetColor("#C62828")).toBe(true);
    expect(isPresetColor("#123456")).toBe(false);
  });

  it("ignores invalid stored or requested colours", () => {
    const storage = memory();
    storage.values.set(PAD_COLOR_KEY, "red");
    expect(loadInkColor(PAD_COLOR_KEY, storage)).toBe(INK_DEFAULT_COLOR);
    saveInkColor(PAD_COLOR_KEY, "#12345", storage);
    expect(storage.values.get(PAD_COLOR_KEY)).toBe("red");
    expect(loadInkColor(PAD_COLOR_KEY, { getItem: () => { throw new Error("blocked"); } })).toBe(INK_DEFAULT_COLOR);
  });
});
