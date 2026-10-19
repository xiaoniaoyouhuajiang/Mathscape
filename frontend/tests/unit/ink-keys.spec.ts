import { describe, expect, it } from "vitest";
import { isDeleteKey, isTypingTarget } from "../../src/ink/keys";

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
