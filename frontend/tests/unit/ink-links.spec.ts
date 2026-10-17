import { describe, expect, it } from "vitest";
import cytoscape from "cytoscape";
import { markdownPreviewLabel, overlappingNotes, type InkLinkedNote, type InkLinkOverlay } from "../../src/ink/links";
import { applyInkOverlay, clearInkOverlay, INK_NOTE_NODE_CAP } from "../../src/graph/ink-overlay";

const linked = (documentId: string, blocks: string[], primary = false, linkId = `${documentId}:${blocks.join(",")}`): InkLinkedNote => ({
  note: { document_id: documentId, title: documentId } as InkLinkedNote["note"],
  link: {
    link_id: linkId, document_id: documentId, target_kind: "markdown", target_key: "md", target_id: "md",
    target: { kind: "markdown", artifact_id: "md", content_hash: "h", first_block: blocks[0], last_block: blocks[blocks.length - 1], block_ids: blocks, start: 0, end: 1, block_hash: "b", excerpt: "", label: "" },
    target_revision: "h", target_label: "", role: "attempt", is_primary: primary, state: "active", created_at: 0, removed_at: null, target_status: "active",
  },
});

describe("N2 Markdown block links", () => {
  it("lists one entry per note overlapping the blocks, preferring the primary link", () => {
    const links = [linked("n1", ["b1", "b2"]), linked("n1", ["b2"], true), linked("n2", ["b3"]), linked("n3", ["b0", "b1"])];
    const hit = overlappingNotes(links, ["b2"]);
    expect(hit.map(entry => entry.note.document_id)).toEqual(["n1"]);
    expect(hit[0].link.is_primary).toBe(true);
    expect(overlappingNotes(links, ["b1"]).map(entry => entry.note.document_id)).toEqual(["n1", "n3"]);
    expect(overlappingNotes(links, ["b9"])).toEqual([]);
  });

  it("derives short names like the desktop: heading, numbered statement, else 24 characters", () => {
    expect(markdownPreviewLabel("# 第一章 极限\n")).toBe("第一章 极限");
    expect(markdownPreviewLabel("**定理 3.2** 设 f 连续")).toBe("定理 3.2");
    expect(markdownPreviewLabel("例 4. 求极限")).toBe("例 4");
    expect([...markdownPreviewLabel("这是一段很长很长的文字，没有编号，也没有标题，所以截断到二十四个字符为止。")]).toHaveLength(24);
    expect(markdownPreviewLabel("   ")).toBe("Markdown 片段");
  });
});

describe("N2 map ink overlay", () => {
  const core = () => cytoscape({ headless: true, elements: [
    { group: "nodes", data: { id: "r-a", label: "A", domainId: "a", domainKind: "SourceSemanticUnit", elementKind: "entity" }, position: { x: 0, y: 0 } },
    { group: "nodes", data: { id: "r-f", label: "F", domainId: "f", domainKind: "SourceArtifact", elementKind: "entity" }, position: { x: 200, y: 0 } },
  ] });
  const overlay = (notes: number): InkLinkOverlay => ({
    counts: [{ target_kind: "source_object", target_id: "a", note_count: notes }, { target_kind: "source_object", target_id: "hidden", note_count: 1 }],
    links: Array.from({ length: notes }, (_, i) => ({ document_id: `n${i}`, title: `N${i}`, last_used_at: i, target_kind: "source_object" as const, target_id: "a", role: "attempt" as const, is_primary: true })),
  });

  it("marks counts, restores labels on clear and never draws targets that are not on the map", () => {
    const graph = core();
    expect(applyInkOverlay(graph, overlay(2), false)).toEqual({ drawn: 0, hidden: 0 });
    expect(graph.$id("r-a").data("label")).toBe("A · ✎2");
    applyInkOverlay(graph, overlay(2), false);
    expect(graph.$id("r-a").data("label")).toBe("A · ✎2");
    clearInkOverlay(graph);
    expect(graph.$id("r-a").data("label")).toBe("A");
  });

  it("caps note nodes, newest first, and joins them with dashed edges", () => {
    const graph = core();
    const result = applyInkOverlay(graph, overlay(INK_NOTE_NODE_CAP + 5), true);
    expect(result).toEqual({ drawn: INK_NOTE_NODE_CAP, hidden: 5 });
    expect(graph.nodes(".ink-note-node")).toHaveLength(INK_NOTE_NODE_CAP);
    expect(graph.$id(`ink-note:n${INK_NOTE_NODE_CAP + 4}`).nonempty()).toBe(true);
    expect(graph.$id("ink-note:n0").empty()).toBe(true);
    expect(graph.edges(".ink-note-edge")).toHaveLength(INK_NOTE_NODE_CAP);
    clearInkOverlay(graph);
    expect(graph.nodes(".ink-note-node")).toHaveLength(0);
  });
});
