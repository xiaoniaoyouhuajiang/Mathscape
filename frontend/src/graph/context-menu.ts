/**
 * Minimal context menu for the map canvas.
 *
 * Deliberately hand-rolled rather than pulled from a Cytoscape extension: the
 * app has no UI framework and the surrounding panels are plain DOM, so a
 * radial third-party menu would not match either the markup or the palette.
 */

export interface ContextMenuItem {
  label: string;
  disabled?: boolean;
  onSelect: () => void;
}

let host: HTMLElement | null = null;

export function isContextMenuOpen(): boolean {
  return host !== null;
}

export function hideContextMenu(): void {
  host?.remove();
  host = null;
  document.removeEventListener("pointerdown", onOutsidePointerDown, true);
  window.removeEventListener("resize", hideContextMenu);
  window.removeEventListener("blur", hideContextMenu);
}

export function showContextMenu(x: number, y: number, items: ContextMenuItem[]): void {
  hideContextMenu();
  if (items.length === 0) return;

  const menu = document.createElement("div");
  menu.className = "graph-context-menu";
  menu.setAttribute("role", "menu");
  for (const item of items) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "context-menu-item";
    button.setAttribute("role", "menuitem");
    button.textContent = item.label;
    button.disabled = item.disabled === true;
    button.onclick = () => {
      hideContextMenu();
      item.onSelect();
    };
    menu.append(button);
  }

  // Positioned off-screen first so the real size is known before clamping.
  menu.style.left = "-9999px";
  menu.style.top = "-9999px";
  document.body.append(menu);
  host = menu;

  const bounds = menu.getBoundingClientRect();
  const margin = 8;
  menu.style.left = `${Math.min(x, window.innerWidth - bounds.width - margin)}px`;
  menu.style.top = `${Math.min(y, window.innerHeight - bounds.height - margin)}px`;

  document.addEventListener("pointerdown", onOutsidePointerDown, true);
  window.addEventListener("resize", hideContextMenu);
  window.addEventListener("blur", hideContextMenu);
}

function onOutsidePointerDown(event: PointerEvent): void {
  if (host !== null && !host.contains(event.target as Node)) hideContextMenu();
}
