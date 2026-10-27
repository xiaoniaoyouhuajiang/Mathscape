/** Input types where Delete/Backspace do not edit text. */
const NON_TEXT_INPUTS = new Set(["button", "checkbox", "color", "file", "image", "radio", "range", "reset", "submit"]);

/**
 * N3.0 keyboard guard: true when a key press on `target` would edit text
 * (inputs, textareas, selects, contenteditable), so canvas shortcuts such as
 * Delete must not fire.
 */
export function isTypingTarget(target: EventTarget | null): boolean {
  const element = target as (EventTarget & { tagName?: string; type?: string; isContentEditable?: boolean; closest?: (selector: string) => unknown }) | null;
  if (!element || typeof element.tagName !== "string") return false;
  if (element.isContentEditable) return true;
  const tag = element.tagName.toUpperCase();
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag === "INPUT") return !NON_TEXT_INPUTS.has(String(element.type ?? "text").toLowerCase());
  return Boolean(element.closest?.("[contenteditable=''],[contenteditable='true']"));
}

/** Delete or Backspace without modifiers. */
export function isDeleteKey(event: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey">): boolean {
  return (event.key === "Delete" || event.key === "Backspace") && !event.ctrlKey && !event.metaKey && !event.altKey;
}

/** M1 canvas shortcuts of the desktop 手稿 workspace. */
export type InkShortcut =
  | "undo" | "redo" | "escape" | "delete"
  | "tool-pen" | "tool-erase" | "tool-partial-erase" | "tool-select" | "tool-pan"
  | "zoom-in" | "zoom-out" | "zoom-fit";

/** Shortcuts that change the document; they never fire on a read-only board. */
export const EDITING_SHORTCUTS: ReadonlySet<InkShortcut> = new Set(["undo", "redo", "delete", "tool-pen", "tool-erase", "tool-partial-erase"]);

type ShortcutKey = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey"> & { code?: string; isComposing?: boolean; repeat?: boolean };

/**
 * Maps a key press to an M1 shortcut. Ctrl and ⌘ are interchangeable so the
 * same build works on Windows, Linux and macOS. Letters are matched through
 * `code` too, so a non-Latin keyboard layout still reaches them.
 */
export function inkShortcut(event: ShortcutKey): InkShortcut | undefined {
  if (event.isComposing || event.altKey) return undefined;
  const mod = event.ctrlKey || event.metaKey;
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  const letter = event.code?.startsWith("Key") ? event.code.slice(3).toLowerCase() : key;
  if (mod) {
    if (letter === "z") return event.shiftKey ? "redo" : "undo";
    if (letter === "y" && !event.shiftKey) return "redo";
    if (key === "+" || key === "=") return "zoom-in";
    if (key === "-" || key === "_") return "zoom-out";
    if (key === "0") return "zoom-fit";
    return undefined;
  }
  if (event.key === "Escape") return "escape";
  if (isDeleteKey(event) && !event.shiftKey) return "delete";
  if (event.repeat) return undefined;
  if (letter === "e") return event.shiftKey ? "tool-partial-erase" : "tool-erase";
  if (event.shiftKey) return undefined;
  if (letter === "p") return "tool-pen";
  if (letter === "l") return "tool-select";
  if (letter === "h") return "tool-pan";
  return undefined;
}
