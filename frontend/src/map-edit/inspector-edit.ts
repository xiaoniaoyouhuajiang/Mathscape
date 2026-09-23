/**
 * Inspector edit form — renders editable fields from an EditContext
 * and constructs UserChangeRequest from the form state.
 */
import type {
  EditContext,
  EditableField,
  EditOption,
  FieldChange,
  SourceRelationChanges,
  SourceUnitChanges,
  UserChangeOperation,
  UserChangeRequest,
} from "./types";
import { generateRequestId } from "./api";

// ---------------------------------------------------------------------------
// Edit / Retire action buttons (inserted into read-only Inspector)
// ---------------------------------------------------------------------------

/**
 * Append "编辑" and optionally "退役" buttons to the Inspector detail panel.
 * `onEdit` is called when the user wants to enter edit mode.
 * `onRetire` is called when the user clicks the retire button.
 */
export function renderEditActions(
  host: HTMLElement,
  editContext: EditContext,
  onEdit: () => void,
  onRetire: () => void,
): void {
  const actions = document.createElement("div");
  actions.className = "edit-actions";

  if (editContext.editable_fields.length > 0) {
    const editButton = document.createElement("button");
    editButton.type = "button";
    editButton.className = "parse-button edit-button";
    editButton.textContent = "编辑";
    editButton.onclick = onEdit;
    actions.append(editButton);
  }

  if (editContext.can_retire) {
    const retireButton = document.createElement("button");
    retireButton.type = "button";
    retireButton.className = "retire-button";
    retireButton.textContent = "退役";
    retireButton.onclick = onRetire;
    actions.append(retireButton);
  }

  host.append(actions);
}

// ---------------------------------------------------------------------------
// Edit form
// ---------------------------------------------------------------------------

export interface EditFormResult {
  form: HTMLFormElement;
  /** Read current form values as a UserChangeRequest. */
  buildRequest: (reason: string) => UserChangeRequest;
}

/**
 * Build a form from the EditContext. Returns the DOM element and a function
 * to extract the current draft as a UserChangeRequest.
 */
export function renderEditForm(
  editContext: EditContext,
  onPreview: (request: UserChangeRequest) => void,
  onCancel: () => void,
): EditFormResult {
  const form = document.createElement("form");
  form.className = "edit-form";

  const heading = document.createElement("span");
  heading.className = "eyebrow";
  heading.textContent = editContext.kind === "SourceSemanticUnit" ? "编辑内容单元" : "编辑来源关系";
  form.append(heading);

  // Render each editable field
  const fieldElements: Map<string, () => unknown> = new Map();
  for (const field of editContext.editable_fields) {
    const { element, getValue } = createFieldControl(field, editContext);
    form.append(element);
    fieldElements.set(field.name, getValue);
  }

  // Reason input
  const reasonField = document.createElement("label");
  reasonField.className = "form-field";
  const reasonLabel = document.createElement("span");
  reasonLabel.textContent = "变更原因";
  const reasonInput = document.createElement("input");
  reasonInput.type = "text";
  reasonInput.placeholder = "例如：修正标题拼写";
  reasonInput.required = true;
  reasonField.append(reasonLabel, reasonInput);
  form.append(reasonField);

  // Action buttons
  const buttonGroup = document.createElement("div");
  buttonGroup.className = "edit-actions";
  const previewButton = document.createElement("button");
  previewButton.type = "submit";
  previewButton.className = "parse-button";
  previewButton.textContent = "预览变更";
  const cancelButton = document.createElement("button");
  cancelButton.type = "button";
  cancelButton.className = "retire-button cancel-button";
  cancelButton.textContent = "取消";
  cancelButton.onclick = onCancel;
  buttonGroup.append(previewButton, cancelButton);
  form.append(buttonGroup);

  const buildRequest = (reason: string): UserChangeRequest => {
    const operation = buildOperation(editContext, fieldElements);
    return {
      request_id: generateRequestId(),
      base_commit: editContext.base_commit,
      reason,
      operations: [operation],
    };
  };

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const reason = reasonInput.value.trim();
    if (!reason) return;
    onPreview(buildRequest(reason));
  });

  return { form, buildRequest };
}

// ---------------------------------------------------------------------------
// Field controls
// ---------------------------------------------------------------------------

interface FieldControl {
  element: HTMLElement;
  getValue: () => unknown;
}

function createFieldControl(
  field: EditableField,
  editContext: EditContext,
): FieldControl {
  switch (field.field_type) {
    case "nullable_text":
      return createNullableTextControl(field);
    case "nullable_unit":
      return createSelectControl(
        field,
        editContext.available_parents,
        true,
      );
    case "unit":
      return createSelectControl(
        field,
        editContext.available_parents,
        false,
      );
    case "unit_completeness":
      return createCompletenessControl(field);
    case "source_relation_kind":
      return createRelationKindControl(field);
    case "span_list":
      return createSpanListControl(field, editContext.evidence_spans);
    default:
      return createReadonlyControl(field);
  }
}

function createNullableTextControl(field: EditableField): FieldControl {
  // A wrapping <label> would nest the clear-row label and steal its clicks.
  const wrapper = document.createElement("div");
  wrapper.className = "form-field";
  const label = document.createElement("span");
  label.textContent = fieldLabel(field.name);

  const input = document.createElement("input");
  input.type = "text";
  input.value = typeof field.value === "string" ? field.value : "";
  input.placeholder = "(空)";

  const clearRow = document.createElement("label");
  clearRow.className = "field-clear-row";
  const clearBox = document.createElement("input");
  clearBox.type = "checkbox";
  clearBox.checked = field.value === null;
  const clearLabel = document.createElement("span");
  clearLabel.textContent = "清空";
  clearRow.append(clearBox, clearLabel);

  clearBox.onchange = () => {
    input.disabled = clearBox.checked;
    if (clearBox.checked) input.value = "";
  };
  input.disabled = clearBox.checked;

  wrapper.append(label, input, clearRow);

  return {
    element: wrapper,
    getValue: (): FieldChange<string> => {
      if (clearBox.checked) return { action: "clear" };
      const v = input.value.trim();
      if (v === (typeof field.value === "string" ? field.value : "")) {
        return { action: "keep" };
      }
      return { action: "set", value: v };
    },
  };
}

function createSelectControl(
  field: EditableField,
  options: EditOption[],
  nullable: boolean,
): FieldControl {
  const wrapper = document.createElement("label");
  wrapper.className = "form-field";
  const label = document.createElement("span");
  label.textContent = fieldLabel(field.name);

  const select = document.createElement("select");
  if (nullable) {
    const none = new Option("（无）", "__clear__");
    select.append(none);
  }
  for (const opt of options) {
    select.append(new Option(opt.label, opt.object_id));
  }
  const currentValue = typeof field.value === "string" ? field.value : null;
  if (currentValue !== null) {
    select.value = currentValue;
  } else if (nullable) {
    select.value = "__clear__";
  }

  wrapper.append(label, select);

  return {
    element: wrapper,
    getValue: (): FieldChange<string> | string | null => {
      const v = select.value;
      if (nullable) {
        if (v === "__clear__" && currentValue !== null) {
          return { action: "clear" } as FieldChange<string>;
        }
        if (v === "__clear__") return { action: "keep" } as FieldChange<string>;
        if (v === currentValue) return { action: "keep" } as FieldChange<string>;
        return { action: "set", value: v } as FieldChange<string>;
      }
      return v === currentValue ? null : v;
    },
  };
}

function createCompletenessControl(field: EditableField): FieldControl {
  const wrapper = document.createElement("label");
  wrapper.className = "form-field";
  const label = document.createElement("span");
  label.textContent = fieldLabel(field.name);

  const select = document.createElement("select");
  for (const [value, label_text] of [
    ["complete", "完整"],
    ["left_truncated", "左侧截断"],
    ["right_truncated", "右侧截断"],
    ["internally_missing", "内部缺失"],
    ["uncertain", "边界待确认"],
  ] as const) {
    select.append(new Option(label_text, value));
  }
  // Keep the wire value aligned with UnitCompleteness in mathscape-domain.
  // `partial` and `fragment` were never valid domain values.
  const currentStr = typeof field.value === "string" ? field.value : "uncertain";
  select.value = currentStr;

  wrapper.append(label, select);

  return {
    element: wrapper,
    getValue: (): string | null => {
      return select.value === currentStr ? null : select.value;
    },
  };
}

function createRelationKindControl(field: EditableField): FieldControl {
  const wrapper = document.createElement("label");
  wrapper.className = "form-field";
  const label = document.createElement("span");
  label.textContent = fieldLabel(field.name);

  if (field.constraints.length > 0) {
    const hint = document.createElement("span");
    hint.className = "form-note";
    hint.textContent = field.constraints.join("；");
    label.append(hint);
  }

  const select = document.createElement("select");
  for (const kind of [
    "states", "proves", "has_part", "poses", "hints",
    "solves", "explains", "applies_method", "cites",
  ]) {
    select.append(new Option(kind, kind));
  }
  const current = typeof field.value === "string" ? field.value : "states";
  select.value = current;
  wrapper.append(label, select);

  return {
    element: wrapper,
    getValue: (): string | null => {
      return select.value === current ? null : select.value;
    },
  };
}

function createSpanListControl(
  field: EditableField,
  allSpans: EditOption[],
): FieldControl {
  const wrapper = document.createElement("div");
  wrapper.className = "form-field span-list-field";
  const label = document.createElement("span");
  label.textContent = fieldLabel(field.name);
  wrapper.append(label);

  const currentIds = new Set(
    Array.isArray(field.value)
      ? (field.value as string[])
      : [],
  );

  const checkboxes: Array<{ id: string; checkbox: HTMLInputElement }> = [];
  const list = document.createElement("div");
  list.className = "span-checkbox-list";

  for (const span of allSpans) {
    const row = document.createElement("label");
    row.className = "span-checkbox-item";
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = currentIds.has(span.object_id);
    const text = document.createTextNode(span.label);
    row.append(cb, text);
    list.append(row);
    checkboxes.push({ id: span.object_id, checkbox: cb });
  }
  wrapper.append(list);

  return {
    element: wrapper,
    getValue: (): string[] | null => {
      const selected = checkboxes
        .filter(({ checkbox }) => checkbox.checked)
        .map(({ id }) => id);
      const changed =
        selected.length !== currentIds.size ||
        selected.some((id) => !currentIds.has(id));
      return changed ? selected : null;
    },
  };
}

function createReadonlyControl(field: EditableField): FieldControl {
  const wrapper = document.createElement("div");
  wrapper.className = "form-field";
  const label = document.createElement("span");
  label.textContent = fieldLabel(field.name);
  const value = document.createElement("code");
  value.className = "identifier-value";
  value.textContent = typeof field.value === "string"
    ? field.value
    : JSON.stringify(field.value);
  wrapper.append(label, value);

  return {
    element: wrapper,
    getValue: () => null,
  };
}

// ---------------------------------------------------------------------------
// Build UserChangeOperation from form state
// ---------------------------------------------------------------------------

function buildOperation(
  ctx: EditContext,
  fieldValues: Map<string, () => unknown>,
): UserChangeOperation {
  if (ctx.kind === "SourceSemanticUnit") {
    const changes: SourceUnitChanges = {};
    const title = fieldValues.get("display_title")?.();
    if (title !== undefined && title !== null && (title as FieldChange<string>).action !== "keep") {
      changes.display_title = title as FieldChange<string>;
    }
    const parent = fieldValues.get("parent")?.();
    if (parent !== undefined && parent !== null && (parent as FieldChange<string>).action !== "keep") {
      changes.parent = parent as FieldChange<string>;
    }
    const completeness = fieldValues.get("completeness")?.() as string | null;
    if (completeness !== null) {
      changes.completeness = completeness;
    }
    return {
      operation: "edit_source_unit",
      object_id: ctx.object_id,
      expected_revision: ctx.current_revision,
      changes,
    };
  } else {
    const changes: SourceRelationChanges = {};
    const relation = fieldValues.get("relation")?.() as string | null;
    if (relation !== null) changes.relation = relation;
    const source = fieldValues.get("source")?.() as string | null;
    if (source !== null) changes.source = source;
    const target = fieldValues.get("target")?.() as string | null;
    if (target !== null) changes.target = target;
    const evidence = fieldValues.get("evidence_spans")?.() as string[] | null;
    if (evidence !== null) changes.evidence_spans = evidence;
    return {
      operation: "edit_source_relation",
      object_id: ctx.object_id,
      expected_revision: ctx.current_revision,
      changes,
    };
  }
}

// ---------------------------------------------------------------------------
// Retire form — WKWebView does not implement window.prompt
// ---------------------------------------------------------------------------

export function renderRetireForm(
  editContext: EditContext,
  onPreview: (request: UserChangeRequest) => void,
  onCancel: () => void,
): HTMLFormElement {
  const form = document.createElement("form");
  form.className = "edit-form";

  const heading = document.createElement("span");
  heading.className = "eyebrow";
  heading.textContent = "退役对象";
  form.append(heading);

  const note = document.createElement("p");
  note.className = "form-note";
  note.textContent =
    editContext.kind === "SourceSemanticUnit"
      ? "将退役此内容单元；相关关系会一并退役，子单元与原文证据默认保留。"
      : "将退役此来源关系；两端的内容单元与原文证据默认保留。";
  form.append(note);

  const reasonField = document.createElement("label");
  reasonField.className = "form-field";
  const reasonLabel = document.createElement("span");
  reasonLabel.textContent = "退役原因";
  const reasonInput = document.createElement("input");
  reasonInput.type = "text";
  reasonInput.placeholder = "例如：单元划分有误";
  reasonInput.required = true;
  reasonField.append(reasonLabel, reasonInput);
  form.append(reasonField);

  const buttonGroup = document.createElement("div");
  buttonGroup.className = "edit-actions";
  const previewButton = document.createElement("button");
  previewButton.type = "submit";
  previewButton.className = "retire-button";
  previewButton.textContent = "预览退役影响";
  const cancelButton = document.createElement("button");
  cancelButton.type = "button";
  cancelButton.className = "retire-button cancel-button";
  cancelButton.textContent = "取消";
  cancelButton.onclick = onCancel;
  buttonGroup.append(previewButton, cancelButton);
  form.append(buttonGroup);

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const reason = reasonInput.value.trim();
    if (!reason) return;
    onPreview(buildRetireRequest(editContext, reason));
  });

  queueMicrotask(() => reasonInput.focus());
  return form;
}

// ---------------------------------------------------------------------------
// Build retire request
// ---------------------------------------------------------------------------

export function buildRetireRequest(
  editContext: EditContext,
  reason: string,
): UserChangeRequest {
  return {
    request_id: generateRequestId(),
    base_commit: editContext.base_commit,
    reason,
    operations: [
      {
        operation: "retire_object",
        object_id: editContext.object_id,
        expected_revision: editContext.current_revision,
      },
    ],
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const FIELD_LABELS: Record<string, string> = {
  display_title: "显示标题",
  parent: "父单元",
  completeness: "完整性",
  relation: "关系类型",
  source: "源节点",
  target: "目标节点",
  evidence_spans: "证据引用",
};

function fieldLabel(name: string): string {
  return FIELD_LABELS[name] ?? name;
}
