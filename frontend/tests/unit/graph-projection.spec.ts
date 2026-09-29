import { describe, expect, it } from "vitest";
import type { IncidenceNode } from "../../src/graph/projection";
import {
  domainKindLabel,
  EDGE_TYPES,
  NODE_TYPES,
  nodeTypeKey,
  nodeTypeLabel,
} from "../../src/graph/taxonomy";

describe("graph projection and taxonomy", () => {
  function makeNode(overrides: Partial<IncidenceNode>): IncidenceNode {
    return {
      id: "test_node",
      element_kind: "entity",
      domain_id: "domain:test",
      domain_kind: "SourceSemanticUnit",
      relation_kind: null,
      layer: "source",
      label: "Test Node",
      summary: null,
      lifecycle: "active",
      parent: null,
      boundary: false,
      ...overrides,
    };
  }

  describe("nodeTypeKey and nodeTypeLabel classification", () => {
    it("classifies SourceSemanticUnit as semantic_unit", () => {
      const node = makeNode({ domain_kind: "SourceSemanticUnit" });
      expect(nodeTypeKey(node)).toBe("semantic_unit");
      expect(nodeTypeLabel(node)).toBe("内容单元");
    });

    it("classifies SourceSpan as source_span", () => {
      const node = makeNode({ domain_kind: "SourceSpan" });
      expect(nodeTypeKey(node)).toBe("source_span");
      expect(nodeTypeLabel(node)).toBe("原文证据");
    });

    it("classifies PDF Artifact via summary media type", () => {
      const node = makeNode({
        domain_kind: "SourceArtifact",
        summary: "application/pdf · 1048576 bytes · available",
      });
      expect(nodeTypeKey(node)).toBe("pdf");
      expect(nodeTypeLabel(node)).toBe("PDF");
    });

    it("classifies Markdown Artifact via summary media type", () => {
      const node = makeNode({
        domain_kind: "SourceArtifact",
        summary: "text/markdown · 4096 bytes · available",
      });
      expect(nodeTypeKey(node)).toBe("markdown");
      expect(nodeTypeLabel(node)).toBe("Markdown 正文");
    });

    it("classifies image attachment via summary media type", () => {
      const node = makeNode({
        domain_kind: "SourceArtifact",
        summary: "image/png · 65536 bytes · available",
      });
      expect(nodeTypeKey(node)).toBe("parse_image");
      expect(nodeTypeLabel(node)).toBe("解析图片");
    });

    it("classifies relation nodes correctly", () => {
      const convNode = makeNode({
        element_kind: "relation",
        domain_kind: "ConversionRun",
      });
      expect(nodeTypeKey(convNode)).toBe("conversion_run");
      expect(nodeTypeLabel(convNode)).toBe("转换任务");

      const relNode = makeNode({
        element_kind: "relation",
        domain_kind: "SourceUnitRelation",
      });
      expect(nodeTypeKey(relNode)).toBe("source_relation");
      expect(nodeTypeLabel(relNode)).toBe("来源关系");

      const unknownRel = makeNode({
        element_kind: "relation",
        domain_kind: "CustomRelation",
      });
      expect(nodeTypeKey(unknownRel)).toBe("unknown_relation");
      expect(nodeTypeLabel(unknownRel)).toBe("未分类关系");
    });

    it("falls back to unknown_entity for unmapped domain kinds", () => {
      const unknownEntity = makeNode({
        element_kind: "entity",
        domain_kind: "FutureEntityKind",
      });
      expect(nodeTypeKey(unknownEntity)).toBe("unknown_entity");
      expect(nodeTypeLabel(unknownEntity)).toBe("未分类对象");
    });
  });

  describe("domainKindLabel mapping", () => {
    it("maps known kinds to friendly Chinese labels", () => {
      expect(domainKindLabel("SourceArtifact")).toBe("材料文件");
      expect(domainKindLabel("ConversionRun")).toBe("转换任务");
      expect(domainKindLabel("SourceSemanticUnit")).toBe("内容单元");
      expect(domainKindLabel("SourceSpan")).toBe("原文证据");
      expect(domainKindLabel("SourceUnitRelation")).toBe("来源关系");
    });

    it("handles null and unknown domain kinds gracefully", () => {
      expect(domainKindLabel(null)).toBe("—");
      expect(domainKindLabel("CustomUnregisteredKind")).toBe("CustomUnregisteredKind");
    });
  });

  describe("visual configuration consistency", () => {
    it("ensures all node types have complete visual styles defined", () => {
      for (const [key, visual] of Object.entries(NODE_TYPES)) {
        expect(visual.label).toBeTruthy();
        expect(visual.background).toMatch(/^#[0-9a-f]{6}$/i);
        expect(visual.border).toMatch(/^#[0-9a-f]{6}$/i);
        expect(["solid", "dashed"]).toContain(visual.borderStyle);
        expect(visual.width).toBeGreaterThan(0);
        expect(visual.height).toBeGreaterThan(0);
      }
    });

    it("ensures all edge types have complete visual styles defined", () => {
      for (const [key, visual] of Object.entries(EDGE_TYPES)) {
        expect(visual.label).toBeTruthy();
        expect(visual.color).toMatch(/^#[0-9a-f]{6}$/i);
        expect(typeof visual.dashed).toBe("boolean");
      }
    });
  });
});
