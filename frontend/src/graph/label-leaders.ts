import type { Core } from "cytoscape";
import { labelBox, type LabelEle } from "./label-hover";
import { mapLabelOffset } from "./label-density";

/** Annotation leaders identify displaced captions; never add graph objects. */
export function bindLabelLeaders(core: Core): void {
  const container = core.container();
  if (!container) return;
  const ns = "http://www.w3.org/2000/svg";
  const layer = document.createElementNS(ns, "svg");
  layer.classList.add("map-label-leaders");
  layer.setAttribute("aria-hidden", "true");
  Object.assign(layer.style, { position: "absolute", inset: "0", width: "100%", height: "100%", pointerEvents: "none", zIndex: "0" });
  const path = document.createElementNS(ns, "path");
  path.setAttribute("fill", "none");
  path.setAttribute("stroke", "var(--ms-text-muted)");
  path.setAttribute("stroke-width", "0.8");
  path.setAttribute("stroke-dasharray", "2 3");
  path.setAttribute("opacity", "0.38");
  layer.append(path);
  container.prepend(layer);
  const draw = () => {
    if (core.destroyed()) return;
    const z = core.zoom(), pan = core.pan();
    const segments: string[] = [];
    core.nodes(':visible').forEach(node => {
      const offset = mapLabelOffset(node);
      if (Math.hypot(offset.x, offset.y) < 1 || Number(node.style("text-opacity")) < 0.4 || Number(node.style("opacity")) < 0.4) return;
      const box = labelBox(node as unknown as LabelEle), p = node.renderedPosition();
      const x1 = box.x1 * z + pan.x, x2 = box.x2 * z + pan.x;
      const y1 = box.y1 * z + pan.y, y2 = box.y2 * z + pan.y;
      const end = { x: Math.max(x1, Math.min(p.x, x2)), y: Math.max(y1, Math.min(p.y, y2)) };
      const dx = end.x - p.x, dy = end.y - p.y, distance = Math.hypot(dx, dy);
      if (distance < 24 || !Number.isFinite(distance)) return;
      const inset = Math.max(node.renderedOuterWidth(), node.renderedOuterHeight()) / 2 + 2;
      segments.push(`M${p.x + dx / distance * inset},${p.y + dy / distance * inset}L${end.x - dx / distance * 3},${end.y - dy / distance * 3}`);
    });
    const d = segments.join(" ");
    if (path.getAttribute("d") !== d) path.setAttribute("d", d);
  };
  core.on("render", draw);
  core.one("destroy", () => { layer.remove(); });
}
