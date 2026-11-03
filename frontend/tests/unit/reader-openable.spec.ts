import { describe, expect, it } from "vitest";
import type { IncidenceNode } from "../../src/graph/projection";
import {
  findSourceArtifactNode,
  openableArtifactId,
  splitSourceItemsFrom,
  tryOpenableArtifactId,
} from "../../src/reader/openable";
import { buildCiteIndex, resolveCite } from "../../src/sandbox/cite";

const PDF = `mathscape:artifact/${"a".repeat(64)}`;
const HEAD = `mathscape:artifact/${"b".repeat(64)}`;
const MD = `mathscape:artifact/${"c".repeat(64)}`;
const CANONICAL = `mathscape:canonical-markdown/v1/marker-markdown/marker-1.10.2/${PDF}`;

function node(id: string, domainId: string, label: string, extra: Partial<IncidenceNode> = {}): IncidenceNode {
  return {
    id, element_kind: "entity", domain_id: domainId, domain_kind: "SourceArtifact", relation_kind: null,
    layer: "source", label, summary: null, lifecycle: "active", parent: null, boundary: false, ...extra,
  } as IncidenceNode;
}

const canonical = node("n-canon", CANONICAL, "book.pdf · 规范解析正文", { openable_artifact_id: HEAD });
const pdf = node("n-pdf", PDF, "book.pdf");
const md = node("n-md", MD, "notes.md");

describe("openableArtifactId", () => {
  it("opens a folded canonical node through its head snapshot", () => {
    expect(openableArtifactId(canonical)).toBe(HEAD);
  });

  it("opens physical artifacts by domain_id", () => {
    expect(openableArtifactId(pdf)).toBe(PDF);
    expect(openableArtifactId(md)).toBe(MD);
  });

  it("fails clearly for a canonical id without a head instead of passing the logical id on", () => {
    const headless = node("n-x", CANONICAL, "x · 规范解析正文");
    expect(() => openableArtifactId(headless)).toThrow(/head/);
    expect(tryOpenableArtifactId(headless)).toBeUndefined();
  });

  it("rejects non-file nodes", () => {
    expect(() => openableArtifactId(node("u", "mathscape:source-unit/x", "u", { domain_kind: "SourceSemanticUnit" }))).toThrow();
  });
});

describe("findSourceArtifactNode", () => {
  it("finds a node by domain_id, or the canonical node that folded a head snapshot", () => {
    const nodes = [pdf, canonical, md];
    expect(findSourceArtifactNode(nodes, PDF)).toBe(pdf);
    expect(findSourceArtifactNode(nodes, CANONICAL)).toBe(canonical);
    expect(findSourceArtifactNode(nodes, HEAD)).toBe(canonical);
    expect(findSourceArtifactNode(nodes, "mathscape:artifact/missing")).toBeUndefined();
  });
});

describe("split source picker", () => {
  it("lists the head snapshot, never the canonical id, and skips headless canonical nodes", () => {
    const headless = node("n-x", `${CANONICAL}-other`, "other · 规范解析正文");
    const items = splitSourceItemsFrom([{ artifact_id: PDF, filename: "book.pdf" }], [pdf, canonical, headless, md]);
    expect(items).toEqual([
      { artifactId: PDF, label: "book.pdf" },
      { artifactId: HEAD, label: "book.pdf · 规范解析正文" },
      { artifactId: MD, label: "notes.md" },
    ]);
  });
});

describe("cite index", () => {
  it("jumps a folded head snapshot id to its canonical node, canonical id still resolves", () => {
    const index = buildCiteIndex([pdf, canonical]);
    expect(resolveCite(HEAD, undefined, index).target?.nodeId).toBe("n-canon");
    expect(resolveCite(CANONICAL, undefined, index).target?.nodeId).toBe("n-canon");
  });
});
