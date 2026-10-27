import { describe, expect, it } from "vitest";
import { EDITING_SHORTCUTS, inkShortcut, isDeleteKey, isTypingTarget } from "../../src/ink/keys";

const el = (tagName: string, extra: Record<string, unknown> = {}) => ({ tagName, closest: () => null, ...extra }) as unknown as EventTarget;

describe("ink keyboard guard", () => {
  it("treats text fields, selects and contenteditable as typing", () => {
    expect(isTypingTarget(el("INPUT", { type: "text" }))).toBe(true);
    expect(isTypingTarget(el("input", { type: "search" }))).toBe(true);
    expect(isTypingTarget(el("INPUT", { type: "number" }))).toBe(true);
    expect(isTypingTarget(el("TEXTAREA"))).toBe(true);
    expect(isTypingTarget(el("SELECT"))).toBe(true);
    expect(isTypingTarget(el("DIV", { isContentEditable: true }))).toBe(true);
    expect(isTypingTarget(el("SPAN", { closest: () => ({}) }))).toBe(true);
  });

  it("lets canvas shortcuts through on non-text controls and plain elements", () => {
    for (const type of ["checkbox", "range", "color", "button"]) expect(isTypingTarget(el("INPUT", { type }))).toBe(false);
    expect(isTypingTarget(el("BUTTON"))).toBe(false);
    expect(isTypingTarget(el("CANVAS"))).toBe(false);
    expect(isTypingTarget(null)).toBe(false);
    expect(isTypingTarget({} as EventTarget)).toBe(false);
  });

  it("recognises Delete and Backspace only without modifiers", () => {
    const key = (key: string, modifiers: Partial<KeyboardEvent> = {}) => ({ key, ctrlKey: false, metaKey: false, altKey: false, ...modifiers });
    expect(isDeleteKey(key("Delete"))).toBe(true);
    expect(isDeleteKey(key("Backspace"))).toBe(true);
    expect(isDeleteKey(key("Backspace", { metaKey: true }))).toBe(false);
    expect(isDeleteKey(key("Delete", { ctrlKey: true }))).toBe(false);
    expect(isDeleteKey(key("d"))).toBe(false);
  });
});

describe("ink shortcuts (M1)", () => {
  const key = (k: string, extra: Partial<KeyboardEvent> & { code?: string } = {}) => ({ key: k, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...extra });
  it("maps undo / redo with Ctrl or ⌘", () => {
    expect(inkShortcut(key("z", { ctrlKey: true }))).toBe("undo");
    expect(inkShortcut(key("z", { metaKey: true }))).toBe("undo");
    expect(inkShortcut(key("Z", { metaKey: true, shiftKey: true }))).toBe("redo");
    expect(inkShortcut(key("y", { ctrlKey: true }))).toBe("redo");
    // Non-Latin layout: the physical Z key still undoes.
    expect(inkShortcut(key("я", { ctrlKey: true, code: "KeyZ" }))).toBe("undo");
  });
  it("maps zoom, escape, delete and tools", () => {
    expect(inkShortcut(key("=", { ctrlKey: true }))).toBe("zoom-in");
    expect(inkShortcut(key("+", { metaKey: true, shiftKey: true }))).toBe("zoom-in");
    expect(inkShortcut(key("-", { ctrlKey: true }))).toBe("zoom-out");
    expect(inkShortcut(key("0", { ctrlKey: true }))).toBe("zoom-fit");
    expect(inkShortcut(key("Escape"))).toBe("escape");
    expect(inkShortcut(key("Delete"))).toBe("delete");
    expect(inkShortcut(key("Backspace"))).toBe("delete");
    expect(inkShortcut(key("p"))).toBe("tool-pen");
    expect(inkShortcut(key("e"))).toBe("tool-erase");
    expect(inkShortcut(key("E", { shiftKey: true }))).toBe("tool-partial-erase");
    expect(inkShortcut(key("l"))).toBe("tool-select");
    expect(inkShortcut(key("h"))).toBe("tool-pan");
  });
  it("ignores IME composition, Alt chords, auto-repeat tool keys and unrelated keys", () => {
    expect(inkShortcut(key("z", { ctrlKey: true, isComposing: true } as never))).toBeUndefined();
    expect(inkShortcut(key("z", { ctrlKey: true, altKey: true }))).toBeUndefined();
    expect(inkShortcut(key("p", { repeat: true }))).toBeUndefined();
    expect(inkShortcut(key("c", { ctrlKey: true }))).toBeUndefined();
    expect(inkShortcut(key("x"))).toBeUndefined();
    expect(inkShortcut(key("P", { shiftKey: true }))).toBeUndefined();
  });
  it("flags editing shortcuts so read-only boards skip them", () => {
    for (const shortcut of ["undo", "redo", "delete", "tool-pen", "tool-erase", "tool-partial-erase"] as const) expect(EDITING_SHORTCUTS.has(shortcut)).toBe(true);
    for (const shortcut of ["escape", "zoom-in", "zoom-out", "zoom-fit", "tool-select", "tool-pan"] as const) expect(EDITING_SHORTCUTS.has(shortcut)).toBe(false);
  });
});
