/**
 * Generic DOM helpers for component creation and querying.
 */

export function textElement<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  value: string,
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  if (className) {
    element.className = className;
  }
  element.textContent = value;
  return element;
}

export function definitionList(
  rows: ReadonlyArray<readonly [string, string]>,
): HTMLElement {
  const list = document.createElement("dl");
  list.className = "definition-list";
  for (const [term, value] of rows) {
    list.append(textElement("dt", "", term), textElement("dd", "", value));
  }
  return list;
}

export function requiredElement<T extends Element>(
  selector: string,
  parent: ParentNode = document,
): T {
  const element = parent.querySelector<T>(selector);
  if (element === null) {
    throw new Error(`required element is missing: ${selector}`);
  }
  return element;
}

export function formatDuration(milliseconds: number): string {
  if (milliseconds < 1_000) {
    return `${milliseconds} ms`;
  }
  const seconds = milliseconds / 1_000;
  return seconds < 60 ? `${seconds.toFixed(1)} s` : `${(seconds / 60).toFixed(1)} min`;
}
