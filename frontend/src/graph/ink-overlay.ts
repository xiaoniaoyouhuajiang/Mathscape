/**
 * N2 map overlay (§6.3, §14.7). The projection schema is unchanged: counts
 * and "我的手稿" nodes come from a separate query and are synthesized here,
 * with projection-local ids that never reach the source graph.
 */
import type { Core, NodeSingular } from "cytoscape";
import type { InkLinkOverlay } from "../ink/links";

/** At most this many note nodes are drawn; the rest stay in the inspector. */
export const INK_NOTE_NODE_CAP = 60;
export const INK_NOTE_NODE_CLASS = "ink-note-node";
export const INK_NOTE_EDGE_CLASS = "ink-note-edge";
export const INK_COUNT_CLASS = "has-ink-notes";

export interface InkOverlayResult { drawn: number; hidden: number }

/** Removes synthesized elements before the projection is synced. */
export function clearInkOverlay(core: Core): void {
  core.remove(`.${INK_NOTE_NODE_CLASS}, .${INK_NOTE_EDGE_CLASS}`);
  core.nodes(`.${INK_COUNT_CLASS}`).forEach(node => {
    const base = node.data("inkBaseLabel");
    if (typeof base === "string") node.data("label", base);
    node.removeData("inkBaseLabel inkCount");
    node.removeClass(INK_COUNT_CLASS);
  });
}

function targetNodes(core: Core): Map<string, NodeSingular> {
  const nodes = new Map<string, NodeSingular>();
  core.nodes().forEach(node => {
    const kind = node.data("domainKind") ?? node.data("nodeType");
    const id = node.data("domainId");
    if (typeof id !== "string" || node.hasClass(INK_NOTE_NODE_CLASS)) return;
    if (node.data("elementKind") && node.data("elementKind") !== "entity") return;
    if (kind === undefined || nodes.has(id)) return;
    nodes.set(id, node);
  });
  return nodes;
}

/**
 * Count markers on source objects and Markdown file nodes. With `showNotes`,
 * note nodes are placed in a small ring around their primary (or first)
 * visible target and joined to every visible target by dashed edges.
 * Markdown-linked notes attach to the Markdown file node; when that node is
 * not on the map they only appear in the reader.
 */
export function applyInkOverlay(core: Core, overlay: InkLinkOverlay | undefined, showNotes: boolean): InkOverlayResult {
  clearInkOverlay(core);
  if (!overlay) return { drawn: 0, hidden: 0 };
  const targets = targetNodes(core);
  let drawn = 0, hidden = 0;
  core.batch(() => {
    for (const count of overlay.counts) {
      const node = targets.get(count.target_id);
      if (!node || count.note_count <= 0) continue;
      const base = String(node.data("label") ?? "");
      node.data("inkBaseLabel", base);
      node.data("inkCount", count.note_count);
      node.data("label", `${base} · ✎${count.note_count}`);
      node.addClass(INK_COUNT_CLASS);
    }
    if (!showNotes) return;
    const notes = new Map<string, { title: string; last: number; links: InkLinkOverlay["links"] }>();
    for (const link of overlay.links) {
      if (!targets.has(link.target_id)) continue;
      const entry = notes.get(link.document_id) ?? { title: link.title, last: link.last_used_at, links: [] };
      entry.links.push(link);
      notes.set(link.document_id, entry);
    }
    const ordered = [...notes.entries()].sort((a, b) => b[1].last - a[1].last || a[0].localeCompare(b[0]));
    const ringIndex = new Map<string, number>();
    for (const [documentId, note] of ordered) {
      if (drawn >= INK_NOTE_NODE_CAP) { hidden++; continue; }
      const anchorLink = note.links.find(link => link.is_primary) ?? note.links[0];
      const anchor = targets.get(anchorLink.target_id)!;
      const slot = ringIndex.get(anchorLink.target_id) ?? 0;
      ringIndex.set(anchorLink.target_id, slot + 1);
      const angle = -Math.PI / 2 + slot * (Math.PI / 4);
      const radius = 90 + Math.floor(slot / 8) * 45;
      const position = anchor.position();
      const id = `ink-note:${documentId}`;
      core.add({
        group: "nodes",
        data: { id, label: note.title, documentId },
        classes: INK_NOTE_NODE_CLASS,
        position: { x: position.x + radius * Math.cos(angle), y: position.y + radius * Math.sin(angle) },
      });
      const seen = new Set<string>();
      for (const link of note.links) {
        const target = targets.get(link.target_id)!;
        if (seen.has(target.id())) continue;
        seen.add(target.id());
        core.add({
          group: "edges",
          data: { id: `ink-link:${documentId}:${target.id()}`, source: id, target: target.id() },
          classes: INK_NOTE_EDGE_CLASS + (link.is_primary ? " is-primary" : ""),
        });
      }
      drawn++;
    }
  });
  return { drawn, hidden };
}
