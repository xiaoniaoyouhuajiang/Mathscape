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
