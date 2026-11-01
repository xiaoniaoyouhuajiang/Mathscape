/**
 * Read-path echo (A-PR3): while a sandbox round runs, the map briefly
 * pulses the nodes a tool call asked about (`sandbox-echo-seed`) and the
 * nodes its result returned (`sandbox-echo-hit`, plus the edges among them,
 * `sandbox-echo-edge`). Classes only, on nodes already on the map: nothing
 * is written, search hits off the map get no fake node.
 *
 * A pulse lasts ~2.6s (strong, then `sandbox-echo-fade`), is capped at
 * `ECHO_CAP` nodes and is cleared by the next pulse.
 */
import type { Core, CollectionReturnValue, NodeSingular } from "cytoscape";

export const ECHO_CAP = 40;
const STRONG_MS = 1600;
const TOTAL_MS = 2600;
const CLASSES = "sandbox-echo-seed sandbox-echo-hit sandbox-echo-edge sandbox-echo-fade";

export type EchoKind = "seed" | "hit";

export class SandboxEcho {
  private timers: number[] = [];
  private lit: CollectionReturnValue | null = null;

  constructor(private readonly core: () => Core | null) {}

  /** Light `nodeIds` (projection ids). Seeds softly pan into view when none
   * of them is on screen; hits never move the camera. Returns the count lit. */
  pulse(kind: EchoKind, nodeIds: readonly string[]): number {
    this.clear();
    const cy = this.core();
    if (!cy || !nodeIds.length) return 0;
    let nodes = cy.collection();
    for (const id of nodeIds.slice(0, ECHO_CAP)) {
      const node = cy.getElementById(id);
      if (node.nonempty() && node.isNode() && node.visible()) nodes = nodes.union(node);
    }
    if (nodes.empty()) return 0;
    let lit = nodes;
    nodes.addClass(kind === "seed" ? "sandbox-echo-seed" : "sandbox-echo-hit");
    if (kind === "hit" && nodes.length > 1) {
      const edges = nodes.edgesWith(nodes);
      edges.addClass("sandbox-echo-edge");
      lit = lit.union(edges);
    }
    this.lit = lit;
    if (kind === "seed" && !anyOnScreen(cy, nodes)) {
      cy.animate({ center: { eles: nodes } }, { duration: 320, easing: "ease-out-cubic" });
    }
    this.timers.push(window.setTimeout(() => lit.addClass("sandbox-echo-fade"), STRONG_MS));
    this.timers.push(window.setTimeout(() => this.clear(), TOTAL_MS));
    return nodes.length;
  }

  clear(): void {
    for (const timer of this.timers) window.clearTimeout(timer);
    this.timers = [];
    if (this.lit && !this.lit.cy().destroyed()) this.lit.removeClass(CLASSES);
    this.lit = null;
  }
}

function anyOnScreen(cy: Core, nodes: CollectionReturnValue): boolean {
  const ext = cy.extent();
  return nodes.nodes().toArray().some((node) => {
    const { x, y } = (node as NodeSingular).position();
    return x >= ext.x1 && x <= ext.x2 && y >= ext.y1 && y <= ext.y2;
  });
}
