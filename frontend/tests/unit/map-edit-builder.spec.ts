import { describe, expect, it } from "vitest";
import {
  clear,
  keep,
  set,
  type FieldChange,
  type SourceRelationDraft,
  type SourceUnitDraft,
  type UserChangeRequest,
} from "../../src/map-edit/types";

describe("map-edit request builders and protocol types", () => {
  describe("FieldChange signal builders", () => {
    it("creates keep signal correctly", () => {
      const signal: FieldChange<string> = keep();
      expect(signal).toEqual({ action: "keep" });
    });

    it("creates set signal with value correctly", () => {
      const signal: FieldChange<string> = set("New Theorem Title");
      expect(signal).toEqual({ action: "set", value: "New Theorem Title" });
    });

    it("creates clear signal correctly", () => {
      const signal: FieldChange<string> = clear();
      expect(signal).toEqual({ action: "clear" });
    });
  });

  describe("UserChangeRequest operation assembly", () => {
    it("builds a valid edit_source_unit request", () => {
      const request: UserChangeRequest = {
        request_id: "req:unit-edit-1",
        base_commit: "commit:abc",
        reason: "Update theorem title and mark complete",
        operations: [
          {
            operation: "edit_source_unit",
            object_id: "unit:euler-identity",
            expected_revision: "rev:1",
            changes: {
              display_title: set("欧拉恒等式"),
              parent: keep(),
              completeness: "complete",
            },
          },
        ],
      };

      expect(request.operations).toHaveLength(1);
      const op = request.operations[0];
      expect(op.operation).toBe("edit_source_unit");
      if (op.operation === "edit_source_unit") {
        expect(op.changes.display_title).toEqual({
          action: "set",
          value: "欧拉恒等式",
        });
        expect(op.changes.completeness).toBe("complete");
      }
    });

    it("builds a valid retire_object request", () => {
      const request: UserChangeRequest = {
        request_id: "req:retire-1",
        base_commit: null,
        reason: "Retire duplicate unit",
        operations: [
          {
            operation: "retire_object",
            object_id: "unit:duplicate-lemma",
            expected_revision: "rev:2",
          },
        ],
      };

      expect(request.operations).toHaveLength(1);
      const op = request.operations[0];
      expect(op.operation).toBe("retire_object");
      if (op.operation === "retire_object") {
        expect(op.object_id).toBe("unit:duplicate-lemma");
        expect(op.expected_revision).toBe("rev:2");
      }
    });

    it("builds a valid create_source_unit request", () => {
      const draft: SourceUnitDraft = {
        kind: "Theorem",
        display_title: "柯西-施瓦茨不等式",
        parent: "unit:parent-chapter",
        source_spans: ["span:evidence-1", "span:evidence-2"],
        completeness: "complete",
      };

      const request: UserChangeRequest = {
        request_id: "req:create-unit-1",
        base_commit: null,
        reason: "Extract Cauchy-Schwarz Theorem",
        operations: [
          {
            operation: "create_source_unit",
            object_id: "unit:cauchy-schwarz",
            draft,
          },
        ],
      };

      expect(request.operations).toHaveLength(1);
      const op = request.operations[0];
      expect(op.operation).toBe("create_source_unit");
      if (op.operation === "create_source_unit") {
        expect(op.draft.display_title).toBe("柯西-施瓦茨不等式");
        expect(op.draft.source_spans).toHaveLength(2);
      }
    });

    it("builds a valid create_source_relation request", () => {
      const draft: SourceRelationDraft = {
        relation: "proves",
        source: "unit:proof-step-1",
        target: "unit:cauchy-schwarz",
        evidence_spans: ["span:evidence-3"],
      };

      const request: UserChangeRequest = {
        request_id: "req:create-rel-1",
        base_commit: "commit:xyz",
        reason: "Link proof step to theorem",
        operations: [
          {
            operation: "create_source_relation",
            object_id: "rel:proof-link",
            draft,
          },
        ],
      };

      expect(request.operations).toHaveLength(1);
      const op = request.operations[0];
      expect(op.operation).toBe("create_source_relation");
      if (op.operation === "create_source_relation") {
        expect(op.draft.relation).toBe("proves");
        expect(op.draft.source).toBe("unit:proof-step-1");
        expect(op.draft.target).toBe("unit:cauchy-schwarz");
      }
    });
  });
});
