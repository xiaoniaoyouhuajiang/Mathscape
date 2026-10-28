# Mathscape Phase1d 契约：手稿可读证据（`ink.page_image`）+ 手稿最小图接线

> 读者：Mob（实现）· 起草：2026-10-28（Asia/Shanghai）· 状态：**§9 六项决策已由用户于 2026-10-28 锁定**；1d-a（图接线）随 Phase1d-a PR 落地，1d-b（渲染工具）另开 PR
> 依据：[Phase0 契约](phase0-contract.md)（`mcp_envelope v0`，下称 **Phase0**）、[Phase1c 图契约](phase1c-graph-contract.md)（下称 **1c**）、[手稿笔记整合提案](../proposals/ink-notebook-integration-v1.md)；缺口来源 `/workspace/bench/phase1c-tool-reeval-report.md` §2.4（note / snapshot 均 `content_ref: null`、`ink_out_of_scope`、邻域 0 边）。
> 版本：沿用 `mcp_envelope v0`，**不新增** `status` 值、**不新增**错误码（`invalid_arg | forbidden | internal`）。标 **[提议]** 为本草案推荐、待用户锁定的条款；标 **[待核]** 为必须由用户拍板的决策点（汇总见 §9）。本文不把任何 [提议] 冒充为已锁定。

---

## 0. 起草时核实的事实（决定了本契约的形状）

以下均在 `main@4642e81` 代码与 bench workspace（`/workspace/bench/workspace`，只读打开）上核实：

1. **快照不是图片。** `ink_snapshots.manifest_json` 是 `InkSnapshotManifest`（`mathscape-ink-snapshot-1/-2`）：`document: InkLabDocument`（pages / capture_segments / operations）+ `visible_stroke_ids` + `point_count`，即**矢量笔迹操作日志**；`content_hash = sha256(manifest_json)`。快照 artifact 本体就是这段 JSON。
2. **服务端没有任何栅格化管线。** Desktop 在前端 TS 渲染：`frontend/src/ink/canvas.ts` 用 `perfect-freehand@1.2.3` `getStroke(points[x,y,pressure??0.5], {size:width, thinning, smoothing, streamline, simulatePressure:false, last})` 得到轮廓多边形后 `Path2D` 填充；`png()` 导出宽 `min(2400, page.width)`、高 > 4800 抛错、白底。Rust workspace 无 image / png / tiny-skia 依赖（`base64 0.22` 已在 `Cargo.lock` 中）。
3. **Rust 侧只算可见性，不算几何。** `InkLabDocument::validate()` 返回 `visible_stroke_ids / undo / redo / point_count`；`transform`（仿射作用于点坐标，不缩放笔宽）、`restyle`（仅改色，`width` 被拒）、`partial_erase`（删源笔迹、加入 fragments 整笔）在 TS `projectInk` 中应用。bench 里 `853d…`「数列递推式&解析式」的快照含 4 个 `transform` + 1 个 `page_add`，必须正确应用。
4. **有链接的手稿没有快照。** bench 9 篇手稿、6 个快照；快照只属于 `bfe6…`「生成函数习题」、`853d…`、`2ac0…`；而有 active NoteLink 的 `b7c9…`「cauchy准则抄写」、`ef74…`「柯西准则变体」、`c374…`「反例尝试」**零快照**。⇒ 只渲染快照的工具无法覆盖最有价值的「笔记 ↔ 教材」路径，工具必须也能渲染**手稿当前 head**（`open_ink_document` 返回完整 `InkLabDocument`，纯 SELECT，只读）。
5. **NoteLink 已在库里，但没进图。** `ink_note_links`（migration 0018）有 17 行；active 的 3 条：`ef74…→source-unit c40e…「正确表述」(study)`、`b7c9…→c40e…(study)`、`c374…→source-unit ecc9…「例 1.2.8 的解答」(question, attempt)`。表注释「Links are note metadata; they never enter the source graph」——本契约只做**读时合成边**，不改此不变量（不持久化、不写 map）。
6. **未命名快照被误分类、也搜不到。** bench 6 个快照只有 1 个有 label（`db9a21fa…`「随便写-1」）。`graph.rs` 的 `snapshots` 来自搜索语料，而 `search.rs::snapshots()` 带 `WHERE s.label IS NOT NULL`；bench 快照 artifact 的 `artifact_revisions.media_type` 是 `application/json`（不是 `SNAPSHOT_MEDIA_TYPE`）。实测 `graph.neighborhood(mathscape:ink-snapshot/94b712de…)` 返回该节点 `kind:"artifact"`、reason `unsupported_media_type`，而非 `snapshot` / `ink_out_of_scope`。
7. **手稿证据 span 已存在。** bench 有 1 个 `mathscape:ink-evidence/c054ed06…`（`SourceSpan`，`ink.snapshot_artifact=…94b712de…`，`page_id=68c241af…`，6 个 `stroke_ids`，lasso），`located_in` → 快照；无 unit 引用它。
8. **MCP 客户端对图片的处理（关键协议约束）。** Codex（本机 `codex-cli 0.163.0`）自 openai/codex#5600 起会把 `CallToolResult.content[]` 的 `{type:"image"}` 块作为 `input_image` 交给模型；但 `FunctionCallOutputPayload::from(&CallToolResult)` **只要 `structuredContent` 非空就只序列化它、丢弃 `content[]`**（openai/codex#10334，维护者称 by design，且 Claude Code 同样行为）；text-only 模型会把图片替换为占位文本（#11346）。当前 `protocol.rs::call_tool` 永远同时输出 `content:[text]` + `structuredContent`。⇒ 图片工具的成功结果**必须不带 `structuredContent`**。

---

## 1. 动机 / 目标

**一句话**：让 Agent 从 `workspace.search` / `graph.*` 命中的 note、snapshot、手稿证据 span 走到**模型可看的页面图像**，并让笔记沿用户已建的 NoteLink 走到教材原文，手稿不再是死胡同。

**可度量的成功标准** [提议]：
- S1：bench 上 note / snapshot / ink span 节点的 `content_ref` 由 `null` 变为 `ink_page`（§4.3），`ink_out_of_scope` 不再出现在这三类节点上（仅残留于渲染失败的诊断）。
- S2：Codex 0.163.0 harness 的 jsonl 中，`ink.page_image` 调用后出现模型侧 `input_image` 项（而非 JSON 文本），模型能复述页面上的可辨认内容（标题/公式片段）。
- S3：路径 P1（§7.2）「搜笔记 → 链接到的 unit → `material.read` + 笔记页图」≤ 5 次工具调用完成；1c 报告中「邻域 0 边」的 `b7c9…` / `ef74…` / `c374…` 默认或 opt-in 后至少 1 条边。
- S4：默认参数下单次渲染 PNG ≤ 512 KiB、bench 单页渲染 p95 < 300 ms（debug 构建 < 1 s）；同输入两次调用 PNG 字节完全一致（`image.sha256` 相同）。
- S5：Phase0/1c 既有测试除「`ink_out_of_scope` 断言」与 `tools/list` 计数外全部不改即过。

---

## 2. 范围

### 2.1 In [提议]
1. 新工具 `ink.page_image`（名称见 §9-Q1），作为 `tools/list` 第 7 个**追加**（Phase0 §2.1 只追加）。
2. 渲染输入三类：手稿 note（当前 head）、快照（不可变 manifest）、手稿证据 span（其快照 + 页 + 选区）。
3. Rust 侧最小渲染栈：笔迹几何投影（transform/restyle/partial_erase）+ perfect-freehand 轮廓 + 填充栅格化 + PNG 编码。
4. 图接线最小集：读时合成边 `has_snapshot`、`note_link`；修复未命名快照误分类；note / snapshot / ink span 的 `content_ref` 改为 `ink_page`。
5. 服务器 `instructions` 的 R6 改写、`material.read` 对快照 artifact 的 `invalid_arg` 增加 hint。
6. `docs/phase0-contract.md` §6 item 7「仍推迟：ink.snapshot_image」随实现 PR 更新。

### 2.2 Out（只记录，不做）
- Marker / PDF OCR、手写识别（HTR/LaTeX 转写）、笔迹转文本。
- `material.grep` 增强、默认全库 grep、真正的 investigation scope、`workspace.search` 的 `parent_artifact_id`（Should）。
- 未命名快照进入 `workspace.search`（`label IS NOT NULL` 过滤保持；它们可经 `has_snapshot` 边到达）。
- `user.ask`、任何写路径（创建快照/链接、改 label）、手稿状态字段（发现靠节点关联 + 模糊搜索 + Agent 数据工具即可）。
- WebP / SVG / PDF 输出、多页拼图、图片缓存落盘、Desktop 渲染器替换。
- CloudAgent 实现（Mob 本地实现）。

---

## 3. 工具：`ink.page_image`

### 3.1 描述（`tools/list` 固定文本草案）
> Render one page of a handwritten note, an ink snapshot, or an ink evidence span as a PNG the model can see. node_id accepts a note document_id, a mathscape:ink-snapshot/… id, its snapshot artifact id, or a mathscape:ink-evidence/… span id (pass graph content_ref.node_id unchanged). Notes render their current head (live, not frozen; cite a snapshot for stable evidence); snapshots and spans render immutable manifests. page_id or page_index (0-based) picks the page; default is the span's page, else page 0. crop defaults to page (span: selection, other strokes faded). max_edge_px defaults to 1024 (256..2048), max_bytes to 524288 (32768..1048576); over a hard cap is invalid_arg. include_image=false returns metadata only. Output: one text block with JSON metadata, then one image/png block; no body text, no OCR. Read-only.

### 3.2 输入 schema [提议]

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["node_id"],
  "properties": {
    "node_id":      { "type": "string", "minLength": 1, "maxLength": 200 },
    "page_id":      { "type": "string", "minLength": 1, "maxLength": 200 },
    "page_index":   { "type": "integer", "minimum": 0, "maximum": 99 },
    "crop":         { "type": "string", "enum": ["page", "strokes", "selection"] },
    "bounds": {
      "type": "object", "additionalProperties": false,
      "required": ["x", "y", "width", "height"],
      "properties": {
        "x": {"type": "number"}, "y": {"type": "number"},
        "width": {"type": "number", "exclusiveMinimum": 0},
        "height": {"type": "number", "exclusiveMinimum": 0}
      }
    },
    "format":        { "type": "string", "enum": ["png"], "default": "png" },
    "max_edge_px":   { "type": "integer", "minimum": 256, "maximum": 2048, "default": 1024 },
    "max_bytes":     { "type": "integer", "minimum": 32768, "maximum": 1048576, "default": 524288 },
    "include_image": { "type": "boolean", "default": true },
    "include_trashed": { "type": "boolean", "default": false }
  }
}
```

| 参数 | 默认 | 硬顶 / 约束 | 越界 / 冲突 | 理由 |
| --- | --- | --- | --- | --- |
| `node_id` | 必填 | ≤200 字符；接受 4 种形态（§3.3），规范化后回报 `id_canonicalized` | 缺失/空/无法识别形态 → `invalid_arg` | 与 `graph.get` 同名同语义，Agent 直接传 `content_ref.node_id` |
| `page_id` / `page_index` | span → span 的 `page_id`；其余 → `0` | 二者互斥；`page_index ≤ 99`（domain 限 100 页） | 同时给 → `invalid_arg`；不存在 → `not_found` | 一次只出一页，保持响应小；多页由 `page_count` 引导逐页调用 |
| `crop` | span → `selection`；其余 → `page` | `selection` 仅对 span 合法 | 非 span 用 `selection` → `invalid_arg` | 默认看整页（与 Desktop 阅读一致）；`strokes` = 可见笔迹包围盒 + 24 单位边距，空白多的页更清晰 |
| `bounds` | 无 | 页坐标；与页求交后面积 > 0；给出时覆盖 `crop` | 与页不相交 / 非有限数 → `invalid_arg` | 让 Agent 放大局部（例如一行公式），参数化而非预置 |
| `format` | `png` | 仅 `png` | 其他值（含 `webp`）→ `invalid_arg`，`details.supported_formats:["png"]` | 线稿无损、`tiny-skia` 自带 PNG 编码；WebP 需新依赖且客户端支持不明（§9-Q6） |
| `max_edge_px` | 1024 | 256..2048 | 越界 → `invalid_arg`（不静默夹取，Phase0） | bench 页 1200×1600 → 默认 768×1024，手写可辨；多数视觉模型在 ~1.5–2K 处再降采样，>2048 只增 token 与字节；Desktop 导出上限 2400 |
| `max_bytes` | 524288 | 32768..1048576（PNG 原始字节，base64 后 ≤ ~1.37 MiB） | 越界 → `invalid_arg` | 白底线稿 1024px 实测量级应在 50–200 KiB；1 MiB 顶防止 stdio 帧过大 |
| `include_image` | `true` | — | — | `false` = 廉价探测（页数、包围盒、笔数），先探后看 |
| `include_trashed` | `false` | — | 已删 note/快照且为 false → `not_found` + `trashed` 诊断 | 与 1c `include_trashed` 一致 |

**输出侧自适应**（不是参数夹取）：若在请求的 `max_edge_px` 下 PNG 超 `max_bytes`，渲染器按 ×0.75 逐级降边长（最多 3 次，不低于 256），成功则 `status:"ok"` + 诊断 `image_downscaled {requested_edge_px, returned_edge_px, reason:"max_bytes"}`；仍超 → `status:"truncated"`、`truncated:true`、**不附图片**、诊断 `image_too_large`，提示改用 `bounds`/`crop:"strokes"`。

### 3.3 `node_id` 解析 [提议]

| 输入形态 | 解析为 | 渲染源 | `revision` |
| --- | --- | --- | --- |
| 32 位十六进制 note `document_id` | note | `open_ink_document` 当前 head | 省略；给 `source.head_seq`，附诊断 `live_note_head` |
| `mathscape:ink-snapshot/<h>` | snapshot | `open_ink_snapshot` 的 `manifest_json` | `sha256:<h>` |
| `mathscape:artifact/sha256/<h>` 且 `ink_snapshot_for_artifact` 命中 | snapshot（`id_canonicalized` → snapshot id） | 同上 | 同上 |
| `mathscape:ink-evidence/<h>`（`SourceSpan` 且 payload 有 `ink` locator） | span | 其 `snapshot_artifact` 的 manifest；页 = `page_id`；选区 = `stroke_ids` | span revision |
| 其他已存在节点（Markdown/PDF artifact、source_unit、非 ink span） | — | — | `invalid_arg`，`details:{node_kind, hint:"use material.read with content_ref"}` |
| 形态合法但不存在 | — | — | `status:"not_found"` |

> 快照 vs head 的取舍：快照是不可变证据（可引用、可复现）；head 是「用户现在写到哪」。bench 事实（§0-4）决定了 head 必须支持；Agent 引用时应优先快照（`has_snapshot` 边给出），描述文本与诊断都这样提示（§9-Q2）。

### 3.4 输出 [提议]

成功（`include_image:true`）的 `CallToolResult`：

```json
{
  "content": [
    { "type": "text", "text": "<下方 metadata JSON>" },
    { "type": "image", "data": "<base64 PNG>", "mimeType": "image/png" }
  ],
  "isError": false
}
```

**不输出 `structuredContent`**（§0-8：Codex / Claude Code 在其存在时丢弃 `content[]`）。这是对 Phase0「structuredContent 镜像」惯例的**唯一一处偏离**，仅限本工具的非错误结果；错误结果（`{error:…}`）与 `include_image:false` 的结果仍走既有 `text + structuredContent` 路径（§9-Q4）。文本块先于图片块（Codex #9815 已修顺序问题，但先文本更稳）。

metadata JSON（字段顺序即序列化顺序）：

```json
{
  "status": "ok",
  "truncated": false,
  "as_of": "2026-10-28T13:30:00+08:00",
  "source": {
    "kind": "snapshot",
    "node_id": "mathscape:ink-snapshot/ddbb3690…",
    "snapshot_id": "mathscape:ink-snapshot/ddbb3690…",
    "artifact_id": "mathscape:artifact/sha256/ddbb3690…",
    "document_id": "853d078ccd709fce32b4499eb0e62cc2",
    "note_title": "数列递推式&解析式",
    "label": null,
    "through_seq": 68,
    "manifest_version": "mathscape-ink-snapshot-1"
  },
  "revision": "sha256:ddbb3690…",
  "page": { "id": "a9469ea4e0977b2fca4f1a4c5ac17bc5", "index": 1, "width": 1200.0, "height": 1600.0 },
  "page_count": 2,
  "crop": { "mode": "page", "bounds": { "x": 0, "y": 0, "width": 1200, "height": 1600 } },
  "strokes": { "visible_on_page": 31, "rendered": 31, "selected": null, "ink_bounds": { "x": 88.5, "y": 120.0, "width": 990.2, "height": 1210.7 } },
  "image": { "format": "png", "width": 768, "height": 1024, "scale": 0.64, "bytes": 143210, "sha256": "…" },
  "renderer": "mathscape-ink-raster/1 (perfect-freehand@1.2.3 port, tiny-skia)",
  "diagnostics": []
}
```

- `diagnostics` 恒输出（1c §13 惯例）；`truncated` 恒输出。
- span 时 `strokes.selected` = 选区内**本页可见**笔数，`source.span_id` 额外给出；选区笔迹若部分已不可见（快照中被擦）→ 诊断 `selection_partially_hidden {missing_stroke_ids≤20}`。
- 无正文、无 OCR、无点坐标数组（避免把几十万点塞回上下文）。

### 3.5 渲染语义 [提议]
- 投影：在 `mathscape-domain` 新增与 TS `projectInk` 等价的几何投影（按 operation 顺序；undo/redo 激活集沿用 `validate()`；`transform` 仿射作用于点、不改笔宽；`restyle` 只改色；`partial_erase` 删源加 fragments）。
- 轮廓：perfect-freehand 1.2.3 `getStroke` 的 Rust 移植，参数与 `canvas.ts::drawInkStroke` 一致（`pressure ?? 0.5`、`simulatePressure:false`、`last:true`）。
- 栅格：`tiny-skia` 非零填充、抗锯齿开、白色不透明背景、笔色取 `#rrggbb`；span 的非选区笔迹以 25% 不透明度绘制（对齐 Desktop 阅读器聚焦观感，具体值可调）。
- 确定性：无时间戳 / 无随机；PNG 不写 tEXt/tIME 块；同输入字节一致。
- 快照 manifest 先校验 `sha256(manifest_json) == content_hash` 与 `validate()`，再渲染。

---

## 4. 图 / `content_ref` 接线（最小必要）

全部是**读时合成**，不写 map、不持久化、不改 `ink_note_links` 「不进 source graph」的存储不变量。

### 4.1 修复：快照节点识别 [提议，属 bug 修复]
`Graph` 构建时快照集合改为直接读 `ink_snapshots` 全表（含 `label IS NULL`），不再依赖搜索语料；`is_snapshot_node` 以「artifact_id ∈ ink_snapshots」为准，`media_type` 只作兜底。bench 中 `94b712de…` 等未命名快照随之变为 `kind:"snapshot"`。

### 4.2 新合成边 [提议]

| `edge_type` | 方向（from → to） | 来源 | 端点 role | attrs | 默认是否遍历 |
| --- | --- | --- | --- | --- | --- |
| `has_snapshot` | note → snapshot artifact | `ink_snapshots.document_id` | `note` / `snapshot` | `through_seq`, `label` | **是**：并入结构默认集（§9-Q3） |
| `note_link` | note → target | `ink_note_links` 中 `state='active'` | `note` / `target` | `role`（attempt\|alt_proof\|check\|study\|question\|reference）、`is_primary`、`target_kind`、`target_status`（active\|retired\|trashed\|missing）、Markdown 目标时 `locator:{md_byte_range:{start,end}}` | **否**：opt-in；但 `graph.get` 的 `edge_summary` 计数（§9-Q3） |

- `note_link` 目标映射：`source_object` → 对应 unit/span/artifact 节点；`markdown` → Markdown artifact 节点（字节区间放边 attrs，Agent 可直接 `material.read(artifact_id, md_byte_range)`）；`ink_note` → 目标 note 节点。目标不在投影中（`missing`/被隐藏的 retired/trashed）→ 沿用 `dangling_endpoint_skipped` 诊断，不产生悬挂边。
- 同一 (note, target) 多个 role 各自成边（`c374…→ecc9…` 有 question 与 attempt 两条）；数量天然很小（bench 全库 3 条 active）。
- 不新增：note↔span、snapshot↔span 以外的推断边，「同标题」「同来源」等启发式边，链接历史（removed）边。
- 硬顶不变（`max_nodes 100 / max_edges 200 / depth 3`）；`DEFAULT_EDGE_TYPES` 变为 5 个（加 `has_snapshot`）。
- `node_kinds` 过滤已有 `note`/`snapshot`，无需新增。

### 4.3 新 `content_ref` 类型 `ink_page` [提议]

```json
{
  "type": "ink_page",
  "tool": "ink.page_image",
  "node_id": "mathscape:ink-evidence/c054ed06…",
  "page_id": "68c241af26e5be530cdde254f1cc7240",
  "page_count": 1,
  "stroke_ids": ["009e141a…", "… ≤ 50"],
  "media_type": "image/png"
}
```
- 适用：note（`node_id`=document_id，`page_id`=首页）、snapshot（`node_id`=snapshot id）、ink span（带 `stroke_ids`，超 50 截断并给 `stroke_ids_truncated:true`）。
- 与既有 `md_byte_range | pdf_pages | artifact_head` 不同，**没有 `artifact_id`/`locator` 交给 `material.read`**；用 `tool` 字段显式指路，防止 Agent 按 R1 习惯误传给 `material.read`。
- `content_unavailable` reason `ink_out_of_scope` 从这三类节点上撤除；`material.read(快照 artifact)` 仍 `invalid_arg`，`details` 增加 `hint:"use ink.page_image"`。
- 服务器 `instructions` R6 改为：「notes, snapshots and ink spans: content_ref.type=ink_page → call ink.page_image(content_ref.node_id, page_id); notes link to sources via note_link (opt-in edge) and to frozen evidence via has_snapshot」。

---

## 5. 错误矩阵（对齐既有码）[提议]

| 情形 | 结果 | `details` / 诊断 |
| --- | --- | --- |
| 缺 `node_id`、未知字段、类型错、`page_id`+`page_index` 同给 | `error.invalid_arg` | `field` |
| `max_edge_px` / `max_bytes` / `page_index` 越硬顶 | `error.invalid_arg` | `field, max, min`（不静默夹取） |
| `format` ≠ png | `error.invalid_arg` | `supported_formats:["png"]` |
| 非 span 使用 `crop:"selection"`；`bounds` 与页不相交或非有限 | `error.invalid_arg` | `field` |
| `node_id` 形态无法识别（无 scheme 且非 32-hex） | `error.invalid_arg` | 与 `material.read` 一致的 scheme 提示 |
| 节点存在但非手稿类 | `error.invalid_arg` | `node_kind, hint` |
| 形态合法、节点不存在 | `status:not_found` | — |
| note / 快照所属 note 已删且 `include_trashed:false` | `status:not_found` | 诊断 `trashed` |
| 页不存在 | `status:not_found` | `page_count, page_ids(≤100)` |
| 页无可见笔迹 / 选区笔迹全不可见 | `status:ok`，照常出白图 | 诊断 `page_blank` / `selection_hidden` |
| 超 `max_bytes` 但降采样可容纳 | `status:ok` | 诊断 `image_downscaled` |
| 降到 256 仍超 `max_bytes` | `status:truncated`，`truncated:true`，无图片块 | 诊断 `image_too_large` |
| manifest 哈希不符 / `validate()` 失败 / 渲染器内部错误 | `error.internal` | `reason: integrity_failed \| invalid_manifest \| render_failed`（不泄露路径） |
| workspace / 数据库无法打开 | `error.internal` | 沿用既有 |
| 客户端为 text-only 模型 | 非服务器可见；Codex 用占位文本替换图片块 | metadata 文本块仍可用 |

- 本工具**不产生** `forbidden`（不接受任何路径类输入；未知字段先被 `invalid_arg` 拒绝）；也**不产生** `ambiguous`（id 解析是确定的）与 `capability_unavailable`（渲染器是编译期内置）。若用户希望「手稿渲染关闭」开关，再引入 `capability_unavailable`（不在本切片）。

---

## 6. 安全 / 隔离 [提议]

1. **只读**：沿用 `McpWorkspace` 每次调用新开 `SqliteArtifactStore`；只调用 `open_ink_document`、`open_ink_snapshot`、`ink_snapshot_for_artifact`、`list_ink_snapshots`、`ink_note_links`/`ink_link_overlay`（均为 SELECT）。不创建快照、不写 `ink_*` 表、不写 objects、不落缓存文件、不建临时文件。
2. **无路径输入**：输入只有 id 与数值；快照字节取自数据库 `manifest_json`（或 `ArtifactPool` 按 catalog id），**从不**拼接用户提供的路径，因此不存在路径穿越面；也不会读写 workspace 外文件。
3. **完整性**：渲染前校验 `content_hash` 与 `validate()`；失败 `internal`，不尝试「尽量渲染」。
4. **资源上限（DoS）**：复用 domain 上限（≤100 页、≤20 000 ops、≤500 000 点、页边 ≤20 000）；位图面积 ≤ 2048×2048（RGBA ≤ 16 MiB）；仅渲染所选页的可见笔迹；PNG ≤ 1 MiB。本切片**不做点简化**：验收中实测 500k 点最坏耗时，若 > 2 s 再在后续切片引入每页点预算。
5. **输出卫生**：metadata 不含笔迹点、不含 `capture_segments.user_agent`、不含文件系统路径；标题/label 原样（已是用户可见文本）。
6. **stdout 纪律**：图片仅经 JSON-RPC 帧返回；渲染栈不得打印到 stdout（tiny-skia 无日志，仍加测试守护）。

---

## 7. 验收

### 7.1 单元 / 回归（`cargo test -p mathscape-mcp -p mathscape-domain` 等）
1. 几何投影：stroke/erase/partial_erase/transform/restyle/undo/redo/page_add 各一例 + 组合例，与 TS `projectInk` 结果对拍（fixture 由 vitest 导出 JSON）。
2. 轮廓对拍：同一组 fixture 笔迹，vitest 调 `perfect-freehand@1.2.3 getStroke` 导出轮廓点，Rust 移植结果逐点误差 ≤ 1e-6。
3. 渲染：确定性（两次 sha256 相等）、白底、尺寸计算（1200×1600 @1024 → 768×1024）、`crop:strokes` 包围盒、span 选区裁剪、`bounds` 求交。
4. 参数：每个硬顶的 `invalid_arg`、互斥参数、`format:webp`、非 span `selection`、非手稿节点。
5. 状态：页不存在 `not_found`、已删 `not_found+trashed`、空白页 `page_blank`、超字节降采样与 `truncated`、哈希不符 `internal`。
6. 协议：图片结果 `content` 顺序 text→image、**无 `structuredContent`**、`isError:false`；`include_image:false` 与错误结果仍有 `structuredContent`；`tools/list` 第 7 项、前 6 项顺序与描述不变。
7. 图：`has_snapshot` 默认出现；`note_link` 仅 opt-in 出现、`edge_summary` 计数；Markdown 目标边带 `md_byte_range`；缺失目标 `dangling_endpoint_skipped`；未命名快照 `kind:snapshot`；`tests/graph.rs` 中 `ink_out_of_scope` 断言（约 585、599 行）改为 `ink_page`。

### 7.2 bench 真实路径（在 bench workspace **副本**上跑 `mathscape mcp`，再用 Codex 0.163.0 harness 复测）
- **P1 笔记 → 教材 → 笔记图**：`workspace.search("柯西", kinds=[note])` → `ef74da1f…`「柯西准则变体」→ `graph.get`（`edge_summary.note_link.out=1`，`content_ref.type=ink_page`）→ `graph.neighborhood(edge_types=[note_link])` → `source_unit c40eaf0e…`「正确表述」→ `material.read`（`md_byte_range 2312..2501`）+ `ink.page_image(ef74da1f…)` → PNG；模型对照两者给出差异描述。（`b7c9…`「cauchy准则抄写」用 `query:"cauchy"` 同路径。）
- **P2 快照 / 证据 span → 图**：`workspace.search("随便写", kinds=[snapshot])` → `mathscape:ink-snapshot/db9a21fa…`（「随便写-1」）→ `ink.page_image` → PNG；再 `graph.neighborhood(mathscape:artifact/sha256/94b712de…)` → `mathscape:ink-evidence/c054ed06…` → `ink.page_image`（默认 `crop:selection`，6 笔高亮）。
- **P3 变换与多页**：`graph.get(853d078c…)` → `has_snapshot` → `ddbb3690…`（seq 68，含 4 个 transform）→ `ink.page_image(page_index=1)`；与 Desktop 快照阅读器截图目测一致（人工核对一次，截图存 bench）。
- 记录：调用次数、每次耗时、PNG 字节、jsonl 中是否出现 `input_image`（S2）。

---

## 8. PR 拆分建议

**推荐：两个顺序小 PR**（耦合点只有 `content_ref.ink_page` 依赖工具存在，放第二个 PR 即可解耦）：

| PR | 内容 | 规模估计 | 价值 / 风险 |
| --- | --- | --- | --- |
| **1d-a 图接线** | §4.1 快照识别修复、§4.2 `has_snapshot` + `note_link`、`edge_summary`、§4.3 `content_ref.ink_page` 与 R6（按用户 2026-10-28 指示前移到 1d-a，见 §9.1）、tests 更新 | ~300–500 行，零新依赖 | 立刻解决「笔记邻域 0 边」，P1 的「笔记 → 教材」半程可测；低风险 |
| **1d-b 渲染工具** | domain 几何投影、perfect-freehand 移植、`tiny-skia` 栅格、`ink.page_image`、`protocol.rs` 图片块、phase0 §6 更新、对拍 fixture | ~1 000–1 500 行，新依赖 `tiny-skia` | 解决「看不到手稿」；保真度与客户端图片支持是主要风险，单独审 |

若用户更偏好一次审完，也可合成**一个** PR（两部分代码区域不重叠，commit 分开即可）；本草案不建议，因为 1d-b 的依赖与保真评审会拖住 1d-a 的低风险收益。两者都由 Mob 本地实现，中文 PR 描述，不合并、不经 CloudAgent。

---

## 9. 决策（**已锁定**，用户 2026-10-28）

1. **Q1 工具名**：`ink.page_image`。
2. **Q2 渲染范围**：渲染 note 当前 head + 快照 + span（head 带 `live_note_head` 诊断）。
3. **Q3 新边默认值**：`has_snapshot` 并入默认结构集（`DEFAULT_EDGE_TYPES` 5 个）；`note_link` opt-in，但计入 `graph.get` 的 `edge_summary`。
4. **Q4 图片结果**：不带 `structuredContent`（仅此工具的信封偏离）。
5. **Q5 渲染实现**：自行移植 perfect-freehand + `tiny-skia`。
6. **Q6 硬顶**：`max_edge_px 1024/2048`、`max_bytes 512 KiB/1 MiB`、仅 PNG、span 非选区 25% 不透明度，作为契约的一部分。

原备选方案不再考虑。

### 9.1 1d-a 范围（已锁定）

- In：§4.1 未命名快照识别修复（`workspace.search` 的 `label IS NOT NULL` 过滤**不放宽**）；§4.2 `has_snapshot`（默认）/`note_link`（opt-in）读时边，缺失/隐藏目标走 `dangling_endpoint_skipped`，同一 (note, target) 每个 role 一条边；§4.3 note / snapshot / ink span 的 `content_ref.ink_page`（`stroke_ids` ≤ 50 + `stroke_ids_truncated`），撤除 `ink_out_of_scope`；R6 改写；`material.read(快照)` 的 `invalid_arg` 增加 `hint:"use ink.page_image"`。
- 1d-a **不**把 `ink.page_image` 加入 `tools/list`；在 1d-b 合入前调用它返回既有的 unknown tool 错误（JSON-RPC `-32602`）。
- Out（留给 1d-b 或以后）：渲染器、`tiny-skia`、perfect-freehand、`ink.page_image` 工具本体；Marker/OCR、grep、investigation、`user.ask`、写路径、search 的 `parent_artifact_id`。

---

## 10. 实现备注（给 Mob，非契约）
- 代码落点：`crates/mathscape-domain/src/ink.rs`（几何投影，与 `validate()` 共用激活集）；新 crate `crates/mathscape-ink-raster`（轮廓 + 栅格 + PNG，供 MCP 使用，日后 Desktop 导出/缩略图可复用）；`crates/mathscape-mcp/src/tools/ink.rs`（工具）；`protocol.rs` 增加「工具可返回原始 `CallToolResult` 内容块」的通道，仅本工具使用。
- `graph.rs`：`SnapshotInfo` 改由 `ink_snapshots` 直读；note 节点合成 `has_snapshot`/`note_link` 时，目标节点 id 规范化复用 1c `id_canonicalized` 逻辑。
- base64 用已在锁文件中的 `base64 0.22`。
- 1c 报告中快照 `media_type` 显示 `application/json`（契约原写 `vnd.mathscape.ink-snapshot+json`）——本契约不改存储，只在识别逻辑上兼容；是否迁移 media_type 另议。
