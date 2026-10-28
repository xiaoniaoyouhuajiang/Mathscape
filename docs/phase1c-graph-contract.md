# Mathscape Phase1c 契约：图导航（`graph.get` / `graph.neighborhood`）

> 读者：Mob（实现）· 起草：2026-10-28（Asia/Shanghai）· 状态：**草案，待用户锁定**
> 依据：[Phase0 契约](phase0-contract.md)（`mcp_envelope v0`，下称 **Phase0**）、[数据模型](data-model.md)、[opt_solutions Task 4.0 / 4.0b / 4](opt_solutions.md)；咨询回复 `/workspace/bench/phase1c-consult-xiaohuang.md`（小黄，Emir/RDTX 视角）与 `/workspace/bench/phase1c-consult-codex.md`（Codex，Phase1b 测评同 session）；缺口来源 `/workspace/bench/phase1b-tool-eval-report.md` Q11。
> 版本：沿用 `mcp_envelope v0`，**不新增** `status` 值、不新增错误码。标 **[锁定]** 为 Mob 裁决后的契约条款；标 **[待核]** 为起草时发现、需在用户锁定前对照代码确认的点（汇总见 §12）。

---

## 1. 状态 / 范围 / 不做什么

**一句话**：Phase1c 增加两个按 id 的**薄只读**图查询工具，让 Agent 从 `workspace.search` 命中的 `source_unit` / `note` / `material` / `snapshot` 走到「可交给 `material.read` 的定位」或「可引用的邻接」，不复刻 Desktop「聚焦此来源」UI。

本切片锁定 [锁定]：
1. 工具只有 `graph.get` + `graph.neighborhood`（§2）。
2. 默认窄、fail-closed：`direction=both`、`depth=1`、仅结构桥接边；语义边（含 `cites`）显式 opt-in（§5）。
3. 节点**不附正文**，只给 `parent_artifact_id` + `content_ref` 跳板（§6）。
4. 截断、错误、诊断完全沿用 Phase0 信封与 grep/outline 惯例（§3）。

**Non-goals（本切片明确不做）** [锁定]：
- 路径收缩虚拟边（Desktop `FocusContraction`）；`include_virtual` 仅接受 `false`。
- `graph.path`、roots/parents、stats/validate、独立批量 get（`ids[]`）。
- `user.ask`、`ink.snapshot_image`、手稿 PNG / 笔迹正文。
- 任何写操作、`propose_changes`、索引/投影持久化。
- 语义层空壳（SemanticEntity / Alignment / ProofContext / ProofState / ProofTransition / Debt / R·D·T·X 派生边）。
- 正文、snippet、正则、自由查询 DSL、相关性排序、分页 cursor。
- MCP 与 Desktop snapshot 实时同步；CloudAgent 实现。

---

## 2. 工具总览 [锁定]

| 工具 | 目的 | 典型输入 | 典型下一步 |
| --- | --- | --- | --- |
| `graph.get` | 解析**一个** id：它是谁（kind / label / lifecycle）、父 artifact 是谁、怎么读 | search 命中的 `id` | 有 `content_ref` → `material.read`；要看周边 → `graph.neighborhood` |
| `graph.neighborhood` | 从 1 个（至多 8 个）种子做有限 BFS，返回邻接节点与关系边 | `graph.get` 的 `canonical_id` | 节点 `content_ref` → `material.read`；按需打开语义边再查一次 |

- `tools/list` 顺序按 Phase0 §2.1「只追加」：`workspace.search`、`material.read`、`material.grep`、`pdf.outline`、**`graph.get`**、**`graph.neighborhood`**。描述文本固定。
- 两个工具都**只读**：不写 workspace 数据库、objects、投影缓存或手稿。

---

## 3. 共享约定

### 3.1 status / 错误 [锁定]（对齐 Phase0 §2.2）

| 情形 | 返回形 | 说明 |
| --- | --- | --- |
| 正常 | `status: "ok"`，`truncated: false` | — |
| 任一种子 / `node_id` 在投影与目录中都不存在 | 业务 `status: "not_found"` | 不返回部分结果（同 grep 的 all-or-nothing）；`diagnostics` 含 `node_not_found` 并列出缺失 id |
| 结果被 `max_nodes` / `max_edges` / 内部访问上限截断 | `status: "truncated"` + `truncated: true` | `diagnostics` 指明哪个 cap（§3.3） |
| id 存在，但 kind 不在 1c 支持集 | **软失败**：`status: "ok"` | 返回最小节点（`kind: "unsupported"` + `entity_kind`），`diagnostics` 含 `unsupported_node_kind`；neighborhood 对该种子不展开 |
| 参数非法：未知字段、未知枚举值（`direction`/`edge_types`/`node_kinds`）、越过硬顶、`include_virtual: true`、空/重复种子、`node_id` 与 `seed_ids` 同时给或都不给、id 为空串 | **调用错误** `{ "error": { "code": "invalid_arg", "message", "details" } }` | 与 grep `max_hits` 越界一致：越过硬顶报错，不静默钳制 |
| 读取投影 / store 失败 | 调用错误 `internal` | 同 Phase0 |

- `ambiguous`、`capability_unavailable` 本切片不产生。
- `truncated` 字段**始终输出**（`true|false`），与 `pdf.outline` 一致，Agent 不用区分缺省。

### 3.2 diagnostics [锁定]
沿用 `{ "code", "message" }`，可带可选 `details` 对象。1c 定义的 code：

| code | 出现于 | 含义 |
| --- | --- | --- |
| `node_not_found` | get / nbhd | 与 `status: not_found` 同出，`details.missing_ids` |
| `unsupported_node_kind` | get / nbhd | id 存在但 kind 不在支持集；`details.entity_kind` |
| `content_unavailable` | get / nbhd（节点级汇总） | `content_ref: null` 的原因；`details.reason ∈ ink_out_of_scope \| no_span_locator \| unsupported_media_type \| relation_node` |
| `multiple_spans` | get | source_unit 有多个 span，`content_ref` 只给主 span；`details.span_count` |
| `max_nodes_reached` / `max_edges_reached` | nbhd | 截断由哪个 cap 触发；`details.limit`、`details.returned` |
| `visit_limit_reached` | nbhd | 内部访问上限（§5.3）触发 |
| `depth_frontier` | nbhd | 达到 `depth` 时仍有未展开的边；`details.unexpanded_edges`（只计数，不算截断） |
| `edge_types_filtered` | nbhd | 种子 1 跳内存在被过滤掉的关系；`details.counts`（如 `{ "cites": 3, "proves": 1 }`），提示可 opt-in 重试 |
| `nodes_filtered` | nbhd | `node_kinds` 过滤掉的节点数；被引用它们的边一并省略 |
| `unknown_relation_skipped` | nbhd | 投影中 `relation_kind = unknown` 的关系被 fail-closed 跳过；`details.count` |
| `retired_hidden` | nbhd | `include_trashed=false` 隐藏的节点/关系数 |
| `seed_retired` | get / nbhd | 显式请求的 id 本身已退役/在回收站（仍返回，lifecycle 可见） |
| `relation_seed` | nbhd | 种子是关系 id；不展开（建议用 `graph.get` 看端点） |
| `id_canonicalized` | get / nbhd | 输入 id 被规范化（§3.5）；`details.input_id`、`details.canonical_id` |
| `scope_not_supported` | get / nbhd | 同 Phase0：`scope=investigation` 按 `accepted` 回答 |

### 3.3 截断 [锁定]（对齐 grep / outline）
- 任一 cap 触发：`status: "truncated"`、`truncated: true`，`diagnostics` 至少一条 `max_nodes_reached` / `max_edges_reached` / `visit_limit_reached`。
- 截断是**按 BFS 层、层内确定性顺序**截：先放满较浅层；同层按 `(kind, canonical_id)` 排序。已返回的边，其端点必在 `nodes[]` 中（不返回悬空边）。
- `counts` 回显：`nodes_returned`、`edges_returned`；已知时附 `nodes_seen`（截断前已发现但未返回的数量，下界）。
- `depth` 用完不算截断（`status` 仍 `ok`），只附 `depth_frontier`。

### 3.4 revision / as_of [锁定]
- 节点/边：有 `RevisionId` 来源时输出 `revision`，没有就**省略**（Phase0 §2.3：不填假值）。
- 响应顶层：`as_of`（服务端读取投影快照的时间，RFC3339 带 `+08:00` 之类时区）**Must**；`workspace_revision` 仅在存在全库修订来源时输出，否则省略 [待核：当前代码无此来源，见 §12]。
- 输入不接受 `revision`（Phase0：graph 类输入 revision Phase1 不实现）。
- 与 Desktop snapshot 可短暂不一致；Agent 遇到 `not_found` 应重新 `workspace.search`，不要重试同 id 多次。

### 3.5 id 规范化 [锁定]
- 接受 `workspace.search` 命中的**原样 `id`**，Agent 不必手改 URI。服务端解析为 `canonical_id` 并在响应中回显；之后的调用应使用 `canonical_id`。
- 映射表：

| search `kind` | search `id` 形态 | `canonical_id` | 1c 节点 `kind` |
| --- | --- | --- | --- |
| `material` | `mathscape:artifact/…` | 原样 | `artifact` |
| `source_unit` | `mathscape:source-unit/…`（`SearchTarget::SourceObject.object_id`） | 原样 | `source_unit` |
| `snapshot` | `snapshot_id`（`SearchTarget::InkSnapshot`） | 该快照的 `artifact_id`（landscape 中的 `SourceArtifact`，media `application/vnd.mathscape.ink-snapshot+json`） | `snapshot` |
| `note` | `document_id`（`SearchTarget::InkNote`） | 原样（ink 文档 id） | `note` |

- 图内其他 id（`mathscape:source-span/…`、`mathscape:source-relation/…`、`mathscape:projection-relation/…`、ConversionRun stable id）也接受。
- 规范化发生时附 `id_canonicalized`。
- [待核] snapshot / note id 的实际字符串格式与 artifact id 是否会冲突，见 §12。

---

## 4. `graph.get`

### 4.1 参数 [锁定]

| 参数 | 类型 | 默认 | 约束 |
| --- | --- | --- | --- |
| `node_id` | string | 必填 | 非空；接受 §3.5 任意形态 |
| `scope` | `accepted \| investigation` | `accepted` | 同 Phase0 |

`graph.get` 不展开邻接（Codex：「get 默认不展开直接边」），只返回单节点 + 跳板字段 + 可选边计数提示。`include_trashed` 不适用：显式 id 总是返回，退役状态通过 `lifecycle` 与 `seed_retired` 体现。

### 4.2 Q11 类请求 / 响应示例

请求：
```json
{ "name": "graph.get", "arguments": { "node_id": "mathscape:source-unit/1bdd…" } }
```

响应（source_unit，有 Markdown span）：
```json
{
  "status": "ok",
  "truncated": false,
  "as_of": "2026-10-28T09:12:03+08:00",
  "node": {
    "id": "mathscape:source-unit/1bdd…",
    "canonical_id": "mathscape:source-unit/1bdd…",
    "kind": "source_unit",
    "entity_kind": "SourceSemanticUnit",
    "label": "定理 3.2（特征值的连续性）",
    "unit_kind": "theorem",
    "lifecycle": "active",
    "revision": "mathscape:revision/…",
    "parent_artifact_id": "mathscape:artifact/sha256/a3…",
    "parent_artifact_media_type": "text/markdown",
    "content_ref": {
      "type": "md_byte_range",
      "artifact_id": "mathscape:artifact/sha256/a3…",
      "locator": { "md_byte_range": { "start": 998, "end": 1740 } },
      "media_type": "text/markdown",
      "via": "mathscape:source-span/7c…"
    },
    "edge_summary": { "contains_span": { "out": 2 }, "has_part": { "in": 1 }, "proves": { "in": 1 } }
  },
  "diagnostics": [
    { "code": "multiple_spans", "message": "unit has 2 spans; content_ref is the lowest-ordinal Markdown span", "details": { "span_count": 2 } }
  ]
}
```

Agent 下一步即可 `material.read({ "artifact_id": content_ref.artifact_id, "locator": content_ref.locator })`，**无需改 search**（取舍 9）。

`edge_summary`（Should）：按 relation 与方向（§5.4）计数，只数不展开，帮 Agent 决定是否 opt-in 语义边；不受 `edge_types` 默认集限制，`unknown` 关系不计入。

### 4.3 错误示例

不存在：
```json
{ "status": "not_found", "truncated": false, "node": null,
  "diagnostics": [ { "code": "node_not_found", "message": "no graph node or catalog entry with this id",
                     "details": { "missing_ids": ["mathscape:source-unit/dead…"] } } ] }
```

存在但 kind 不支持（如 `SourceWork`）：
```json
{ "status": "ok", "truncated": false,
  "node": { "id": "mathscape:work/…", "canonical_id": "mathscape:work/…", "kind": "unsupported",
            "entity_kind": "SourceWork", "label": "…", "lifecycle": "active",
            "parent_artifact_id": null, "content_ref": null },
  "diagnostics": [ { "code": "unsupported_node_kind", "message": "SourceWork is not navigable in Phase1c",
                     "details": { "entity_kind": "SourceWork" } } ] }
```

参数错：
```json
{ "error": { "code": "invalid_arg", "message": "invalid arguments: missing field `node_id`", "details": null } }
```

---

## 5. `graph.neighborhood`

### 5.1 参数（默认 / 硬顶）[锁定]

| 参数 | 类型 | 默认 | 硬顶 / 合法值 | 越界 |
| --- | --- | --- | --- | --- |
| `node_id` | string | — | 与 `seed_ids` 二选一（恰好一个） | 都给/都不给 → `invalid_arg` |
| `seed_ids` | string[] | — | 1–**8** 个，不可重复 | 空/重复/>8 → `invalid_arg` |
| `depth` | int | **1** | 0–**3**（0 = 只回种子） | >3 → `invalid_arg` |
| `direction` | enum | **`both`** | `both \| in \| out`（§5.4） | 其他 → `invalid_arg` |
| `edge_types` | enum[] | **`["conversion","located_in","contains_span","has_part"]`** | §7 闭集，不含 `unknown` | 未知值 → `invalid_arg`；`[]` = 不走边，只回种子 |
| `node_kinds` | enum[] | 省略 = 不限制 | `artifact \| source_span \| source_unit \| snapshot \| note` | 未知值 → `invalid_arg` |
| `max_nodes` | int | **40** | 1–**100** | >100 → `invalid_arg` |
| `max_edges` | int | **80** | 1–**200** | >200 → `invalid_arg` |
| `include_trashed` | bool | **false** | — | — |
| `include_virtual` | bool | **false** | 仅 `false` | `true` → `invalid_arg` |
| `scope` | enum | `accepted` | 同 Phase0 | — |

- **省略 `edge_types` ≠ 空数组**：省略 = 结构四件套；`[]` = 不遍历任何边。
- 要语义边必须**显式列出**，且需要结构边时要一起列：`["contains_span","has_part","proves","states"]`。
- `include_trashed=false`：隐藏 lifecycle 为 `superseded | rejected | deprecated` 的节点/关系（与 `workspace.search` 的 `retired()` 一致），以及回收站中的 note / snapshot；种子本身例外（§3.2 `seed_retired`）。[待核] 命名见 §12。
- `node_kinds` **只过滤输出，不切断遍历**：BFS 仍穿过被过滤节点计算深度，只是不返回它们；端点被过滤的边一并省略（附 `nodes_filtered`）。这是为了避开 opt_solutions Task 4 (b) 的「过滤切断链路」问题，同时不引入收缩虚拟边。

### 5.2 请求 / 响应示例

默认结构邻域（从 source_unit 出发）：
```json
{ "name": "graph.neighborhood", "arguments": { "node_id": "mathscape:source-unit/1bdd…" } }
```

```json
{
  "status": "ok",
  "truncated": false,
  "as_of": "2026-10-28T09:12:05+08:00",
  "seed_ids": ["mathscape:source-unit/1bdd…"],
  "applied": {
    "depth": 1, "direction": "both",
    "edge_types": ["conversion", "located_in", "contains_span", "has_part"],
    "node_kinds": null, "max_nodes": 40, "max_edges": 80,
    "include_trashed": false, "include_virtual": false, "scope": "accepted"
  },
  "nodes": [
    { "id": "mathscape:source-unit/1bdd…", "kind": "source_unit", "entity_kind": "SourceSemanticUnit",
      "label": "定理 3.2（特征值的连续性）", "lifecycle": "active", "depth": 0, "via_edge": null,
      "parent_artifact_id": "mathscape:artifact/sha256/a3…",
      "content_ref": { "type": "md_byte_range", "artifact_id": "mathscape:artifact/sha256/a3…",
                       "locator": { "md_byte_range": { "start": 998, "end": 1740 } },
                       "media_type": "text/markdown", "via": "mathscape:source-span/7c…" } },
    { "id": "mathscape:source-unit/09ab…", "kind": "source_unit", "entity_kind": "SourceSemanticUnit",
      "label": "第三章 特征值与标准形", "lifecycle": "active", "depth": 1,
      "via_edge": "mathscape:projection-relation/has-part/5e…",
      "parent_artifact_id": "mathscape:artifact/sha256/a3…",
      "content_ref": { "type": "md_byte_range", "artifact_id": "mathscape:artifact/sha256/a3…",
                       "locator": { "md_byte_range": { "start": 0, "end": 997 } },
                       "media_type": "text/markdown", "via": "mathscape:source-span/11…" } },
    { "id": "mathscape:source-span/7c…", "kind": "source_span", "entity_kind": "SourceSpan",
      "label": "来源范围", "lifecycle": "active", "depth": 1,
      "via_edge": "mathscape:projection-relation/contains-span/2f…",
      "parent_artifact_id": "mathscape:artifact/sha256/a3…",
      "content_ref": { "type": "md_byte_range", "artifact_id": "mathscape:artifact/sha256/a3…",
                       "locator": { "md_byte_range": { "start": 998, "end": 1740 } },
                       "media_type": "text/markdown", "via": null } }
  ],
  "edges": [
    { "id": "mathscape:projection-relation/has-part/5e…", "relation": "has_part",
      "entity_kind": "SourceUnitRelation", "synthetic": true, "label": "包含", "lifecycle": null,
      "from": "mathscape:source-unit/09ab…", "to": "mathscape:source-unit/1bdd…",
      "endpoints": [
        { "node_id": "mathscape:source-unit/09ab…", "role": "parent", "side": "from", "ordinal": null },
        { "node_id": "mathscape:source-unit/1bdd…", "role": "child",  "side": "to",   "ordinal": null } ] },
    { "id": "mathscape:projection-relation/contains-span/2f…", "relation": "contains_span",
      "entity_kind": "SourceUnitRelation", "synthetic": true, "label": "对应原文", "lifecycle": null,
      "from": "mathscape:source-unit/1bdd…", "to": "mathscape:source-span/7c…",
      "endpoints": [
        { "node_id": "mathscape:source-unit/1bdd…", "role": "unit", "side": "from", "ordinal": null },
        { "node_id": "mathscape:source-span/7c…",  "role": "span", "side": "to",   "ordinal": 0 } ] }
  ],
  "counts": { "nodes_returned": 3, "edges_returned": 2 },
  "diagnostics": [
    { "code": "depth_frontier", "message": "1 edge not expanded at depth 1", "details": { "unexpanded_edges": 1 } },
    { "code": "edge_types_filtered", "message": "semantic relations exist around the seed; pass edge_types to include them",
      "details": { "counts": { "proves": 1 } } }
  ]
}
```

（`depth_frontier` 那 1 条即 span →`located_in`→ artifact：按实体跳计，artifact 在 depth 2；Agent 已可直接用 span 的 `content_ref`。）

截断示例（片段）：
```json
{ "status": "truncated", "truncated": true,
  "counts": { "nodes_returned": 40, "edges_returned": 52, "nodes_seen": 67 },
  "diagnostics": [ { "code": "max_nodes_reached", "message": "stopped at max_nodes=40; deeper layers omitted",
                     "details": { "limit": 40, "returned": 40 } } ] }
```

种子缺失（all-or-nothing）：
```json
{ "status": "not_found", "truncated": false, "nodes": [], "edges": [],
  "diagnostics": [ { "code": "node_not_found", "message": "1 of 2 seeds not found",
                     "details": { "missing_ids": ["mathscape:source-unit/dead…"] } } ] }
```

### 5.3 BFS 规则 [锁定]
1. 遍历对象：`landscape` 投影（`mathscape_query::project_landscape` 的 IncidenceGraph 语义），**未过滤**的全图，关系作为超边。
2. **关系节点透明**：entity → relation → entity 记 **1 跳**（实体跳）；关系不占 `depth`。所有关系类型同价（1 跳），**不**沿用 Desktop 的 `conversion`/`located_in` 免深度规则（见 §9、§12）。
3. **关系计入预算**：每个返回的关系 = `edges[]` 中 1 个对象，计 1 个 `max_edges`（无论端点数）；关系不出现在 `nodes[]`，不占 `max_nodes`。`max_nodes` 只数实体节点（含种子）。
4. 只沿 `edge_types` 中的关系扩展；同一关系只要类型在集合内且端点都可见即返回。不在集合内的关系**既不扩展也不返回**（MCP 比 Desktop 更窄：Desktop 会显示范围内两端可见的 `cites`，MCP 不返回，只在 `edge_types_filtered` 计数）。
5. `relation_kind = unknown` 永不遍历、不返回（fail-closed，同 Task 4.0），计入 `unknown_relation_skipped`。
6. visited 去重；节点 `depth` 为到最近种子的最短实体跳数，`via_edge` 为首次到达所经关系 id（种子为 `null`）；多种子时附 `seed_of`（最近种子 id，Should）。
7. 内部访问上限：遍历过程中实际访问（含被 `node_kinds` 过滤、被退役隐藏的）实体数 ≤ **2000**，超出即停并 `visit_limit_reached`（防止 `node_kinds` 过滤后的隐藏爆炸）。
8. 输出排序确定性：`nodes` 按 `(depth, kind, canonical_id)`；`edges` 按 `(min endpoint depth, relation, id)`。
9. 种子是关系 id（如 ConversionRun）：不展开，返回空邻域 + `relation_seed`；种子 kind 不支持：跳过该种子 + `unsupported_node_kind`，其余种子照常；全部种子都不可展开时 `status: ok`、`nodes` 只含可返回的种子最小节点。

### 5.4 方向语义（相对种子）[锁定]
方向取自投影的 incidence 方向，**不是**「上游/下游」的数学语义。每条关系有 `from` 侧（`IncidenceDirection::IntoRelation` 端点）和 `to` 侧（`OutOfRelation` 端点）：

- `out`：当前节点在 `from` 侧 → 走到 `to` 侧端点。
- `in`：当前节点在 `to` 侧 → 走到 `from` 侧端点。
- `both`：两者都走；`Undirected` 端点（如角色 `evidence`）**只在 `both` 下**可达。

| relation | `from`（role） | `to`（role） | 从种子看 |
| --- | --- | --- | --- |
| `conversion` | 输入 artifact（PDF，`input`） | 输出 artifact（Markdown，`output`） | PDF `out` → Markdown |
| `located_in` | SourceSpan（`span`） | artifact（`artifact`：Markdown 或手稿快照） | span `out` → artifact；artifact `in` → spans |
| `contains_span` | SourceSemanticUnit（`unit`） | SourceSpan（`span`，带 `ordinal`） | unit `out` → spans |
| `has_part` | 父 unit（`parent`） | 子 unit（`child`） | 章 `out` → 命题；命题 `in` → 章 |
| 领域关系（`states`/`proves`/…/`cites`） | `source` 角色 | `target` 角色 | 依登记方向；`evidence` 角色为 Undirected |

结论（写进工具描述）：从 source_unit 溯回 artifact 是 `out` 方向（unit→span→artifact），从 artifact 找内容单元是 `in` 方向；拿不准就用默认 `both`。Desktop 聚焦是「沿派生方向单向向下游」，与此不同，见 §9。

---

## 6. 节点 / 边字段

### 6.1 节点 [锁定]

| 字段 | 必有 | 说明 |
| --- | --- | --- |
| `id` | Must | 图内 id（= `canonical_id`；get 另回显 `canonical_id`） |
| `kind` | Must | `artifact \| source_span \| source_unit \| snapshot \| note \| relation \| unsupported`（`relation` 仅 `graph.get` 直接查关系 id 时出现） |
| `entity_kind` | Must | 原 `EntityKind`（PascalCase，如 `SourceSemanticUnit`）；note 为 `InkDocument` |
| `label` | Must | 投影 label（中文显示名）；无则用 id 尾部 |
| `lifecycle` | Must | `LifecycleStatus` snake_case；note/snapshot 为 `active \| trashed`；未知为 `null` |
| `parent_artifact_id` | Must（可 `null`） | 见 §6.3 |
| `content_ref` | Must（可 `null`） | 见 §6.2；`null` 时节点级或汇总 `content_unavailable` |
| `depth` / `via_edge` | nbhd Must | §5.3 |
| `revision` | Should | 有来源才出 |
| `media_type` | Should | artifact / snapshot 节点 |
| `unit_kind` | Should | source_unit 的 payload `unit_kind`（如 `theorem`） |
| `summary` | Should | 投影 summary（短元数据，**不是正文**） |
| `boundary` | May | 投影 `boundary` 标记 |
| `edge_summary` | get Should | §4.2 |
| `endpoints` | get，`kind=relation` 时 Must | 同边对象的 `endpoints` |

**不附正文**：不返回 Markdown 文本、snippet、PDF 文本层、笔迹内容。

### 6.2 `content_ref` 判别式 [锁定]
`content_ref` 要么是 `null`，要么是可**直接**喂给 `material.read` 的对象（`artifact_id` + `locator` 原样复用 Phase0 线格式），`type` 为判别字段：

Markdown（span 精确定位；source_unit 取最小 ordinal 的 Markdown span）：
```json
{ "type": "md_byte_range", "artifact_id": "mathscape:artifact/sha256/a3…",
  "locator": { "md_byte_range": { "start": 998, "end": 1740 } },
  "media_type": "text/markdown", "via": "mathscape:source-span/7c…" }
```

PDF（span 只有页定位：`page.pdf_page_index` 0-based → `pdf_pages` 1-based 闭区间）：
```json
{ "type": "pdf_pages", "artifact_id": "mathscape:artifact/sha256/9f…",
  "locator": { "pdf_pages": { "from": 146, "to": 146 } },
  "media_type": "application/pdf", "via": "mathscape:source-span/c4…" }
```
是否真能读出文字仍由 `material.read` 决定（可能 `capability_unavailable` + `pdf_no_text_layer`），图工具不预判。

artifact 节点自身（入口窗口，不是精确证据）：
```json
{ "type": "artifact_head", "artifact_id": "mathscape:artifact/sha256/a3…",
  "locator": { "md_byte_range": { "start": 0, "end": 32768 } }, "media_type": "text/markdown", "via": null }
```
PDF artifact 的 `artifact_head` 为 `pdf_pages {1,1}`；Agent 应优先 `pdf.outline` 再选页。

不可读：
```json
"content_ref": null
```
同时 `diagnostics` 含：
```json
{ "code": "content_unavailable", "message": "handwriting content is not readable in Phase1c",
  "details": { "node_ids": ["…"], "reason": "ink_out_of_scope" } }
```
`reason`：`ink_out_of_scope`（note、snapshot、ink span）｜`no_span_locator`（unit 没有带定位的 span，**不猜** byte range）｜`unsupported_media_type`（material.read 不支持的 media）｜`relation_node`。

优先级：Markdown 定位 > PDF 页定位 > `null`。同一 span 同时有 Markdown 与 PDF 页定位时只给 Markdown，PDF 页可由 Agent 通过 `graph.get(span)` 的 `summary` 或后续版本获取（1c 不做多 ref）。

### 6.3 `parent_artifact_id` 推导 [锁定]

| 节点 | `parent_artifact_id` |
| --- | --- |
| `source_span` | payload `markdown.artifact`；否则 `page.artifact`；否则 `ink.snapshot_artifact` |
| `source_unit` | 其主 span（`contains_span` 最小 ordinal、带定位）的上一行结果；无 span → `null`（不沿 `has_part` 猜祖先） |
| `artifact` | `null`（自身即可读材料；转换来源经 `conversion` 边看） |
| `snapshot` | `null`；所属 ink 文档 id 放 `document_id`（Should） |
| `note` | `null` |
| `relation` / `unsupported` | `null` |

### 6.4 边对象 [锁定]

| 字段 | 必有 | 说明 |
| --- | --- | --- |
| `id` | Must | 关系 stable id；合成关系为确定性 `mathscape:projection-relation/<prefix>/<digest>` |
| `relation` | Must | §7 闭集 snake_case |
| `entity_kind` | Must | `ConversionRun` / `SourceUnitRelation` / `Evidence`（located_in） |
| `synthetic` | Must | 投影合成（`located_in`、`contains_span`、由 `parent_unit` 合成的 `has_part`）为 `true` |
| `endpoints[]` | Must | `{ node_id, role, side: from\|to\|undirected, ordinal }`，**保留全部角色**，多元关系不压成二元（data-model：不丢证据与身份） |
| `from` / `to` | Must（可 `null`） | 恰好 1 个 from 端点与 1 个 to 端点时的便捷字段；否则 `null`，以 `endpoints` 为准 |
| `label` | Should | 投影中文 label |
| `lifecycle` / `revision` | Should | 有来源才出 |

---

## 7. relation 枚举（闭集）[锁定]

与 `mathscape_graph::ProjectionRelationKind`（serde snake_case）逐字一致，即 `SourceUnitRelationKind` 9 值 + 投影合成 3 值：

| relation | 来源 | 层 | 默认 `edge_types` |
| --- | --- | --- | --- |
| `conversion` | `ConversionRun`（PDF → Markdown） | 结构桥接 | ✅ |
| `located_in` | 合成：SourceSpan → artifact（Markdown / 手稿快照） | 结构桥接 | ✅ |
| `contains_span` | 合成：unit → span | 结构桥接 | ✅ |
| `has_part` | 合成（`parent_unit`）或领域关系 | 结构桥接 | ✅ |
| `states` | 领域 | 语义 | opt-in |
| `proves` | 领域（原文证明关系，**不是**内核证书） | 语义 | opt-in |
| `poses` | 领域 | 语义 | opt-in |
| `hints` | 领域 | 语义 | opt-in |
| `solves` | 领域 | 语义 | opt-in |
| `explains` | 领域 | 语义 | opt-in |
| `applies_method` | 领域 | 语义 | opt-in |
| `cites` | 领域（跨文献） | 语义 | opt-in |

- 输出中**不会**出现 `unknown`：投影里的 `unknown` 关系被跳过并计数（`unknown_relation_skipped`）。`edge_types` 传 `unknown` → `invalid_arg`。
- 新增枚举值 = 改本文档 + `tools/list` 描述；内部投影改名不得直接泄露给 Agent（Codex 风险 8）。Agent 侧应把未来出现的陌生 `relation` 当作不透明字符串原样引用，不推断语义。
- 不引入 `parent_of` / `derived_from` / `related_to` 等泛化或重复语义。

---

## 8. 推荐 Agent 配方（写进工具描述摘要）

**R1 · search 命中 source_unit → 读原文（Q11 类，最短路径）**
1. `workspace.search(query, kinds=["source_unit"])` → 取 `id`。
2. `graph.get(node_id=id)` → 有 `content_ref` 就直接 `material.read(content_ref.artifact_id, content_ref.locator)`。
3. `content_ref` 为 `null`：看 `content_unavailable.reason`；`no_span_locator` 时用 R2 看结构邻居，**不要**猜字节区间。

**R2 · 浅结构邻域**
`graph.neighborhood(node_id, depth=1)`（全默认）→ 取相邻 span / 父章节 unit 的 `content_ref` → `material.read`。

**R3 · 证明/推导溯源（opt-in）**
`graph.neighborhood(node_id, depth=2, edge_types=["contains_span","has_part","proves","states"], max_nodes=60)` → 对 `proves`/`states` 端点再 `graph.get` 拿 `content_ref`。先看 `edge_types_filtered` / `edge_summary` 是否真有这类边，再打开。

**R4 · 引用（谨慎）**
只在需要跨文献时加 `cites`，且保持 `depth ≤ 2`、显式 `max_nodes`；遇 `truncated` 先缩小 `edge_types` 或 `depth`，不要只调大 cap。

**R5 · 从材料找内容单元**
`graph.neighborhood(node_id=<markdown artifact>, direction="in", depth=2, node_kinds=["source_unit"])` → 得到该 Markdown 的内容单元（span 被 `node_kinds` 过滤但仍穿过）。从 PDF 出发需 `direction="both"`、`depth=3`（PDF `out`→Markdown `in`→span `in`→unit，方向混合，单向 `in`/`out` 走不通）。

**R6 · note / snapshot**
`graph.get` 可返回元数据（标题、lifecycle、快照的 `document_id`），`content_ref: null` + `ink_out_of_scope`。1c 不能读手稿内容，Agent 应如实说明。

---

## 9. Desktop 与 MCP 默认值差异

MCP 以 **Agent 可组合参数**为准；Desktop「聚焦此来源」的策略只作为可选配方，**不是** MCP 默认。

| 维度 | Desktop 聚焦（opt_solutions Task 4） | MCP 1c 默认 | 在 MCP 中复现 Desktop 的近似参数 |
| --- | --- | --- | --- |
| 方向 | 单向，沿派生方向向下游 | `both` | PDF 种子：`direction="out"` 无法覆盖 span→Markdown（located_in 指向上游），需 `both` + `edge_types` 结构四件套 |
| 深度计量 | `conversion`/`located_in` 免深度；`contains_span`/`has_part` 计深度 | 所有关系 1 实体跳 | 「深度 1 = 这份 PDF 的全部内容单元」≈ MCP `depth=3`（PDF→MD→span→unit） |
| 可穿越集合 | 结构 4 种可穿越，语义 8 种禁止穿越 | 同（默认结构 4 种） | 默认即可 |
| 范围内语义边 | 显示（`cites` 等两端可见即保留） | 不返回，只计数 | 显式加入 `edge_types` |
| 虚拟收缩边 | `FocusContraction` | 无（`include_virtual` 仅 false） | 不可复现；用 `node_kinds` 过滤 + 原始边 |
| 规模 | 全图快照内淡化 | `max_nodes` 40 / `max_edges` 80 | 显式放大到 100 / 200 |

Desktop 配方（文档示例，非默认）：
```json
{ "name": "graph.neighborhood",
  "arguments": { "node_id": "mathscape:artifact/sha256/<pdf>", "direction": "both", "depth": 3,
                 "node_kinds": ["artifact", "source_unit"], "max_nodes": 100, "max_edges": 200 } }
```
注意：`both` 会额外带出 PDF 的上游（若有），这是与 Desktop 单向聚焦的已知差异。

---

## 10. 本切片验收

**功能** [锁定]
1. Q11 类可解：真实 bench workspace（`/workspace/bench/workspace`，只读）上，`workspace.search("特征值", kinds=["source_unit","material"])` 命中的 source_unit 经 **`graph.get` 一次**得到 `parent_artifact_id` 与 `content_ref`，随后 `material.read` 返回 `status: ok` 且文本覆盖该单元原文。**不依赖** search 改动。
2. `tools/list` 恰为 6 个工具，顺序如 §2，前 4 个描述文本不变。
3. 默认 `graph.neighborhood` 不返回任何 `cites` / `proves` 边（Task 4 验收「不经 cites 跨到其他 PDF」的 MCP 版）；显式 `edge_types` 加入后可返回。
4. 截断：构造 >40 节点邻域 → `status: truncated`、`truncated: true`、`max_nodes_reached`；无悬空边；结果确定性（两次调用字节相同，`as_of` 除外）。
5. 错误矩阵单测：缺失 id → `not_found`；未知枚举 / depth=4 / max_nodes=101 / `include_virtual:true` / `seed_ids` 9 个或重复 / `node_id`+`seed_ids` 同给 → `invalid_arg`；不支持 kind → `ok` + `unsupported_node_kind`；note → `content_ref:null` + `content_unavailable`。
6. 方向单测：unit `out` 得 span；Markdown artifact `in` 得 span；`has_part` 父在 `from` 侧（对齐 Task 4.0b 断言）。
7. 只读：调用前后 workspace 数据库与 objects 无变化（mtime / hash 断言或只读打开）。

**流程** [锁定]
- 本地 `mob/phase1c-graph`（命名可调）分支开 PR，按 Phase1b 惯例 review；**不用 CloudAgent**。
- 合入后更新 `phase0-contract.md` §6 清单第 7 项：graph 标为 1c 已落地并链接本文；`ink.snapshot_image`、`user.ask` 仍待排。
- Should（不阻塞 1c 合入）：`workspace.search` 的 source_unit 命中补 `parent_artifact_id`（取舍 9）。

---

## 11. 取舍说明（小黄 vs Codex）

| # | 议题 | 小黄 | Codex | Mob 裁决 |
| --- | --- | --- | --- | --- |
| 1 | 工具切分 | 只做 get + neighborhood，不要第三刀 | 同 | **两刀**（双方一致） |
| 2 | 默认方向/深度 | both、depth 1、硬顶 3、virtual 固定 false | 同（「include_virtual 1c 不开放」） | **both / 1 / 3 / virtual=false，true→invalid_arg** |
| 3 | 默认 edge_types | 结构四件套，语义边全部 opt-in（fail-closed） | 默认只排除 `cites` 与虚拟边（其余语义边未明确排除） | **采纳小黄**：结构四件套；`cites` 及全部语义边 opt-in |
| 4 | caps | max_nodes 40/100，max_edges 80/200 | max_nodes 100/500，max_edges 200/1000 | **采纳小黄**（Agent 上下文友好）；Codex 偏好更大 cap，记入风险 §12，若实测频繁截断再议 |
| 5 | 未知 kind | 软失败 + `unsupported_node_kind` | 格式/未知 kind → `invalid_arg`，不软失败为空邻域 | **采纳小黄**软失败；但参数层的未知**枚举值**（`node_kinds` 等）仍是 `invalid_arg`，吸收 Codex「参数错与节点缺失可区分」的关切 |
| 6 | 截断 | `status=truncated`+`truncated:true`+diag | 同 + `retry_hint` / 计数 | **统一 grep/outline 形**；计数（`counts`）采纳 Codex，`retry_hint` 以 diagnostic message 承载，不新增字段 |
| 7 | 正文 | 不附，给跳板 | 不附，content_ref 判别式 | **不附**；content_ref 判别式结构采纳 Codex |
| 8 | relation 枚举 | ProjectionRelationKind / SourceUnitRelationKind + 合成三值 | 闭集 snake_case；未知 edge_type → invalid_arg | **一致**，§7 |
| 9 | search 补 parent | Should（强烈）；get 必须能单独解 Q11 | **Must**（不能改 search 时 get 必须兜底） | **采纳小黄时序**：1c Should；`graph.get` Must 返回 parent/content_ref |
| 10 | 数据源 | 同源 landscape_graph，只读，可短暂不一致 | landscape 出边、source_map 出 locator、catalog 出身份；返回 revision/as_of + `projection_stale` | **合并**：landscape 投影 + source_map payload 取 locator；返回 `as_of`，`workspace_revision` 有来源才出；`projection_stale` 暂不定义（无检测来源） |
| 11 | note/snapshot | note 若有稳定 id 可做根；ink 不做根 | 接受 note/snapshot，content_ref=null + `content_unavailable` | **采纳 Codex 接受面** + 小黄「ink 内容不做」：可 get，不可读 |
| 12 | 深度计量 | 关系节点透明，但计入 max_* | 返回最短 depth、via_edge | **两者合并**：实体跳；关系计 `max_edges`；节点带 `depth`/`via_edge` |
| 13 | 额外参数 | `scope`；`relation_roles` 不做 | `seed_ids`（默认 1 / 硬顶 20）、`include_trashed` | 保留 `include_trashed`(false)、`seed_ids` **硬顶收紧到 8**；`relation_roles` 不做 |
| 14 | 排序 | — | depth → kind → canonical_id，确定性 | **采纳 Codex** |
| 15 | 可用边提示 | — | `available_edge_types` 供重试 | 以 `edge_types_filtered` 诊断 + get 的 `edge_summary` 承载，不新增顶层字段 |

---

## 12. 开放风险 / 待核

**开放风险**
1. **caps 偏紧**：每个 span 也占节点名额，`max_nodes=40` 在「从 Markdown 找全部单元」（R5）上可能常截断；Codex 建议 100/500。先锁 40/100，跑 Phase1c 测评后再看。
2. **深度计量与 Desktop 不同**：MCP 把 `conversion`/`located_in` 计为 1 跳，unit→artifact 需 depth 2、PDF→unit 需 depth 3（=硬顶）。缓解：`graph.get` 必给 parent/content_ref，span 节点自带 content_ref。若用户希望与 Desktop 一致，需改为「`conversion`/`located_in` 免深度」并重新验证 R5。
3. **Agent 误开 `cites` + depth 3**：靠硬顶 + 默认窄边 + 截断诊断兜底；工具描述必须写 R4。
4. **多元关系**：`from`/`to` 便捷字段在多端点时为 `null`，Agent 若只读 `from`/`to` 会漏；描述里要求以 `endpoints` 为准。
5. **重复 `has_part`**：`parent_unit` 合成的 `has_part` 与登记的 `has_part` 领域关系可能同时存在，邻域里出现两条语义相同的边；需决定去重（建议：同端点对只保留领域关系，或都保留但计数）。
6. **投影整体失败**：`project_landscape` 遇悬空 `located_in` 端点返回 `UnknownEndpoint` 错误，会让所有 graph 调用变 `internal`；需决定是否在 MCP 侧降级（跳过坏关系 + 诊断）。
7. **性能**：每次调用从 `landscape_snapshot()` 全量重建投影（opt_solutions 4.5 删掉后端 `get_neighborhood` 正是因为成本）；1c 可接受，但需在大 workspace 上测时延，必要时按 `as_of` 做进程内缓存（不持久化）。
8. **短暂不一致**：Desktop 刚登记/撤回时 id 可能 `not_found`；Agent 应重新 search。
9. **PDF 可读性误判**：`content_ref.type=pdf_pages` 不保证有文本层（bench 上 P1–P4 全是 `pdf_no_text_layer`）。

**[待核] 起草时发现的文档/代码不一致，用户锁定前 Mob 需确认**
- A. **landscape_graph 不是存储表**：代码中是 `SqliteStore::landscape_snapshot()` + `mathscape_query::project_landscape()` 现场投影；咨询稿与本文说的「读 landscape_graph」指这个投影。MCP 实现应复用该函数，不新增存储层遍历（与 architecture 边界一致）。
- B. **note 不在 landscape 投影里**：note 的 search id 是 ink `document_id`，landscape 节点只有 `SourceArtifact` / `SourceSpan` / `SourceSemanticUnit`（+ 关系）。note 的 `graph.get` 需从 ink store 读元数据，邻域恒为空（只能是种子本身）；是否接受此「孤立节点」语义需用户确认。
- C. **snapshot 的 search id 是 `snapshot_id`，图节点是 `artifact_id`**：需在 MCP 侧做 §3.5 映射；确认两者字符串格式不冲突。
- D. **`include_trashed` 命名**：landscape 实体只有 `LifecycleStatus`（无 trashed），trash 只存在于 ink（`InkLifecycle::Trashed`）。本文把它定义为「隐藏 superseded/rejected/deprecated + 回收站 ink」，与 `workspace.search` 的 `retired()` 过滤一致，但名字可能误导；可考虑改名 `include_retired`（会与 search 参数名不一致）。
- E. **无 workspace 级 revision 来源**：代码中没有 `workspace_revision` / `data_version`；按 Phase0 只能省略，仅给 `as_of`。Codex 的「cursor 绑定 workspace revision」在 1c 不成立（1c 也不做 cursor）。
- F. **PDF 页定位的 span 没有 `located_in` 边**：`located_in` 只从 `markdown.artifact` 或 `ink.snapshot_artifact` 合成；只有 `page` 定位的 span 在图上不连到 PDF artifact。本文让 `parent_artifact_id` / `content_ref` 直接读 span payload 的 `page.artifact`，但邻域遍历仍到不了那个 PDF。
- G. **`Undirected` 端点**：领域关系中非 `source`/`target` 角色（如 `evidence`）在投影里是 `Undirected`；且投影会跳过端点不在节点集里的关系（纯证据 span 的关系不进 landscape）。需确认 bench 数据中 `proves`/`states` 的角色确实是 `source`/`target`。
- H. **`phase0-contract.md` §6 第 7 项**把 graph、`ink.snapshot_image`、`user.ask` 列在一起；1c 只做 graph，合入时需改该清单（§10）。
- I. **Desktop 深度规则**：opt_solutions Task 4.0 中 `conversion`/`located_in` 免深度，与本文 §5.3 规则 2 不同（见风险 2），属有意差异，需用户确认。

---

## 13. 实现备注（Phase1c 落地时与本契约的偏差）

实现见 `crates/mathscape-mcp/src/tools/graph.rs`，测试见 `crates/mathscape-mcp/tests/graph.rs`。与上文的差异（均为「最接近的诚实行为」）：

- **悬空端点降级**：`project_landscape` 遇到指向目录外对象的 span 定位/关系端点会整体报错；实现逐个剔除出错的投影链接后重试，并给出新诊断码 `dangling_endpoint_skipped`（`details.items[]`：`object_id` + `missing_entity`）。被剔除的对象仍可 `graph.get`，只是少了那条边。
- **种子不受 `node_kinds` 过滤**：种子永远返回（depth 0），过滤只作用于非种子节点。
- **`edge_summary`**：每个关系类型都带 `in` / `out`，有无向端点（如 `evidence`）时另加 `undirected`。
- **重复 `has_part`**：投影里既有合成 `has_part`（来自 `parent_unit`）又有存储的 `has_part` 关系时，两条都保留，不去重。
- **不支持的种类**：投影外的 source-map 对象（如 `LocalSymbol`）与未知 relation 类型的 landscape 关系返回 `kind: unsupported`（软失败 `ok`）。
- **PDF 页 span**：投影只为 Markdown / Ink span 合成 `located_in`；PDF 页 span 没有到 PDF 的边，BFS 从 PDF 出发到不了它们（`graph.get` 仍给 `pdf_pages` 的 `content_ref`）。
- **笔记**：笔记来自 ink store 的搜索语料，不在投影里，邻域中是孤立节点。
- **其它**：`as_of` 为本地时区 RFC3339；不输出 `workspace_revision`（无来源，省略）；snapshot 节点额外带 `snapshot_id`；合成关系的 `lifecycle` 为 `null`；`diagnostics` 总是输出（可为空数组）。
