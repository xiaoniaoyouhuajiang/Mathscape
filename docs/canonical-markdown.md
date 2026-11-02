# 扫描 PDF 的逻辑 canonical Markdown

本文冻结「一个 PDF 对外一份 canonical markdown」的第一刀。实现是注册表 / 视图，不是新的一等图实体，也不改 Artifact 的内容寻址。

物理 Markdown 字节保持不可变。每次 Marker 解析仍产出新的内容寻址快照。canonical 只是稳定逻辑身份，指向当前 head 快照和覆盖台账。

## 身份

一条 lineage 由三元组决定：

```text
source_pdf_artifact_id + parser_profile + parser_policy_version
```

稳定 id：

```text
mathscape:canonical-markdown/v1/{parser_profile}/{parser_policy_version}/{source_pdf_artifact_id}
```

- 同一三元组永远得到同一个 `canonical_id`。换 profile 或 policy（例如 Marker 行为版本）另开 lineage，不能静默覆盖。
- `parser_profile` 来自转换参数 `marker_profile`（现有 Marker promotion 会写入，例如 `mathscape.marker.math-ocr.v1`）。历史运行没有该字段时用 `mathscape.marker.unspecified`。
- `parser_policy_version` 为 `marker-{tool_version}`；没有工具版本时为 `unversioned`。
- profile 与 policy 不能含空白或 `/`。PDF 的 stable id 原样接在最后，允许自身含 `/` 与 `:`。

这不是 `EntityKind`，也不会变成一张可修订的领域实体。图上的节点只是投影。

## 注册表与台账

SQLite（迁移 `0021_canonical_markdown`）两张表：

| 字段 | 含义 |
| --- | --- |
| `canonical_id` | 上面的逻辑 id |
| `source_pdf_artifact_id` | 源 PDF Artifact |
| `parser_profile` / `parser_policy_version` | 与身份三元组一致，`(pdf, profile, policy)` 唯一 |
| `head_artifact_id` / `head_revision` | 当前对外快照。只是指针，不改快照字节 |
| `total_pages` | 已知的 PDF 页数；未知则为空 |
| 覆盖行 `conversion_id` + `markdown_artifact_id` + `page_from`/`page_to` | 某次成功或 `partial` 转换贡献的闭区间 |

`sync_canonical_markdown_from_catalog` 按当前 `ConversionRun` **整表重建**注册表。它不调用 Marker。手写的台账会被这次同步替换。

`sync_canonical_markdown_for_pdf` 只重建一个 PDF 的注册表行与覆盖行，其他 PDF 的行原样保留。ensure 在 promotion 之后用它，避免一次解析改写全库台账。

只读投影 `project_canonical_markdown` 不写库。MCP 用它回答覆盖，因此没有事先 sync 也能报告 `partial`。地图折叠只看已落库的注册表，避免把尚未登记的历史碎片图一次性改写。

Head 选择：在该 lineage 的成功 / `partial`、输出为 `text/markdown` 的 Marker 转换里，取完成时间最新的快照；同一转换里 ordinal 更小者优先（promotion 里主 Markdown 在前）。失败、超时、取消不记入覆盖。

## 页码

对外覆盖区间是 **1-based、闭区间**，`page_index_base = one`，与 `material.read` 的 `pdf_pages` 一致。

归一化：

- `ConversionRun.page_range` 的 `PageIndexBase::Zero`（Marker `--page_range`）加 1。
- `MarkdownPdfBlockMap.marker_page_ids` 与 `{page}----------------` 也是 0-based PDF 页索引。`pages_from_marker_page_ids` 把它们收成 1-based 区间。
- 本切片的自动投影使用转换记录上的 `page_range`。页图 JSON 在 Artifact 字节里，读取它不属于这条只读 SQL 投影。调用方已经解析出 `marker_page_ids` 时，用上面的纯函数，不要把两套页码直接相加。

没有 `page_range` 的转换可以成为 head，但不贡献 `covered` 页。

## 覆盖语义

```json
{
  "coverage": {
    "total_pages": 300,
    "covered": [{ "from": 12, "to": 40 }],
    "missing": [{ "from": 1, "to": 11 }, { "from": 41, "to": 300 }],
    "requested": { "from": 12, "to": 40 },
    "head_covered": [{ "from": 12, "to": 40 }],
    "partial": true,
    "complete": false,
    "page_index_base": "one"
  }
}
```

- `covered`：各不可变快照页区间的并集。
- `head_covered`：当前 head 那一份快照自己的页，不是并集。
- `missing`：在已知 `total_pages` 内、未被并集覆盖的页。页数未知时 `missing` 为空，且不能当成全文。
- `complete`：页数已知，并集盖住全书，**并且 head 自己盖住全书**。片段即使拼起来盖住全书，也仍然不是 complete，因为本切片不拼接字节。
- `partial`：请求区间没有被 `covered` 完全盖住，**或者** `complete` 为 false。因此只读到第 12–40 页时，即使这次请求的页都有文本，`partial` 仍为 true。`status=ok` 不能理解成全文可搜。

## MCP

不新增工具。已有四个工具在命中 canonical 时多带 `canonical_id`、`coverage`、`partial`：

- `material.read`
- `material.grep`
- `workspace.search`
- `pdf.outline`

扫描版 PDF（请求页都没有文本层）：

- 已有一份快照的页区间 **完全包含** 请求：返回该快照的 Markdown 字节（包含请求的 head 优先，否则取最新的包含快照；会在该 PDF 的所有 lineage 中找），`content_type` 为 `markdown`，`served_artifact_id` 指向物理快照。不按页切开字节；区间以 `coverage` 为准。超出 32KiB 仍是 `truncated`。只与请求相交的快照不会被返回。
- 没有任何快照完全包含请求：进入下面的内部 ensure。ensure 成功后重新加载视图，按同一条完全包含规则返回新快照；否则返回 `capability_unavailable`，`details` 里带 Marker 的 CapabilityProbe 与 `ensure` 结果。

部分页有文本层、部分页没有的请求不会触发 ensure，仍返回文本层并带 `pdf_pages_without_text` 诊断；单独请求那些空页才会解析。有文本层的 PDF 仍返回文本层，只附加覆盖字段。

用 canonical id 调用 `material.read`（`md_byte_range`）或 `material.grep` 时，解析到 head 快照再读。这不是 artifact 重定向：旧的物理 artifact id 照常可读，并带上同一个 `canonical_id`。

`material.grep`、`workspace.search`、`pdf.outline` 永不触发 ensure。grep / search 在带明确页范围的入口出现之前都不接入 ensure。

## Ensure（`material.read` 内部路径）

ensure 是 `material.read` 的内部步骤，不是 Agent 可调用的工具。它只把缺页解析成 **新的** 不可变快照，从不改写或删除已有 Artifact 字节。实现见 `crates/mathscape-mcp/src/tools/ensure.rs`，台账见迁移 `0022_canonical_markdown_ensure`。

### 范围

- 请求区间就是 Marker 的 `--page_range`（一段连续区间）。因为不拼接，只补缺口无法让单份快照包含请求，所以缺口只用于报告：`details.ensure.missing` 是请求内、所有 lineage 并集都没覆盖的页。
- 单次请求最多 `max_pages_per_request` 页（默认 12）。超过时返回 `blocked_budget`，不运行 Marker，提示缩小 `pdf_pages`。
- profile 固定为 `mathscape.marker.math-ocr.v1`；policy 为 `marker-{探测到的版本}`，所以 Marker 升级会开新 lineage。

### 写入顺序

1. 规划：已有单份快照完全包含请求 → `reused`，不跑 Marker。
2. 探测 Marker（与 CapabilityProbe 同一候选顺序：`MATHSCAPE_MARKER_EXECUTABLE`、`MATHSCAPE_MARKER_VENV`、默认 venv、`PATH`）。不可用 → `capability_unavailable`。
3. 加锁：SQLite `BEGIN IMMEDIATE` 内，先把租约过期的 `running` 行标为 `abandoned`，再依次检查本 PDF 的活租约（每 PDF 互斥）、全局运行数（Marker 信号量，默认 1）、滚动页预算（默认每小时 96 页，统计窗口内 running / succeeded / failed 的页数）。都通过才插入 `running` 行。
4. 运行 Marker（后台线程，进程超时默认 30 分钟，租约 = 超时 + 5 分钟）。
5. 校验产物清单（`is_promotable`），失败即 `failed`。
6. promotion：写入新的内容寻址 Artifact 与 `ConversionRun`（记录 `page_range`、`marker_profile`）。
7. 只对该 PDF 做 `sync_canonical_markdown_for_pdf`，更新注册表与 head。
8. 写终态回执（`succeeded` / `failed`，含 `conversion_id`、`markdown_artifact_id` 或错误），租约随之释放。

### 幂等

幂等键：

```text
{pdf_artifact_id}|{canonical_id}|{from}-{to}|{parser_profile}|{parser_policy_version}|ensure-v1
```

重复请求先经过第 1 步的包含检查；台账 + head + 注册表决定是否已有结果，所以已覆盖的请求不会再跑 Marker。同一 PDF 并发的第二个请求在第 3 步得到 `blocked_lock`。

### 状态

`details.ensure.status`：

| 状态 | 含义 | 结果 |
| --- | --- | --- |
| `reused` | 已有单份快照包含请求 | 直接返回快照 |
| `expanded` | 本次 Marker 成功，新快照已登记 | `status: ok`，返回新快照 |
| `started` | Marker 已启动，等待期（默认 50 秒）内未完成 | `capability_unavailable`，稍后再读 |
| `blocked_budget` | 请求页数或滚动预算超限 | `capability_unavailable`，不跑 Marker |
| `blocked_lock` | 本 PDF 或全局 Marker 槽被占 | `capability_unavailable`，`lease` 给出占用的页 |
| `capability_unavailable` | Marker 未安装或版本不支持 | `capability_unavailable`，附 probe |
| `failed` | Marker、清单校验或 promotion 失败 | `capability_unavailable`，回执记录错误 |

同时会有 `canonical_ensure_{status}` 诊断；Marker 缺失时仍保留 `pdf_no_text_layer`。`details.ensure` 还带 `requested`、`missing`、`marker_range`、`ensure_id`、`idempotency_key`、`canonical_id`、`conversion_id`、`markdown_artifact_id`。

### 副作用与工具注解

`material.read` 因此不再是只读：可能写入工作区 parse cache（新 Artifact、`ConversionRun`、注册表行、ensure 台账），可能在本机运行 Marker。注解为 `readOnlyHint: false`、`destructiveHint: false`、`idempotentHint: true`、`openWorldHint: false`。其余六个工具仍是只读注解。Codex 侧 `mcp_servers.mathscape.tool_timeout_sec = 120`，并把 `MATHSCAPE_MARKER_EXECUTABLE`、`MATHSCAPE_MARKER_VENV`、`XDG_DATA_HOME` 转发给 MCP 进程，让 ensure 找到与桌面端相同的 Marker。

### 已知残留

MCP 进程在 Marker 运行中退出时，后台线程随之终止，`running` 行保留到租约过期，再被下一次加锁标为 `abandoned`。在此之前同一 PDF 会得到 `blocked_lock`。

## 地图

仅当该 PDF 已有注册表行时，材料图把这条 lineage 的 head 与有页码的 Markdown 快照收成 **一个** 派生节点。节点 id 是 `canonical_id`（仍用 `SourceArtifact` 这个投影种类，避免新的领域类型）。转换边和 `located_in` 改指向该节点。图片、元数据 JSON 仍留在图上。

`graph.get` 既有的 `canonical_id` 字段继续表示「解析后的图节点 id」。对这个视图节点，它就是逻辑 canonical id；`content_ref.artifact_id` 指向 head，以便原样交给 `material.read`。SourceSpan 的字节定位仍指向原来的物理快照，本切片不重挂。

未 sync 的历史转换保持原图：每个物理 Markdown 仍是独立节点。

「解析中」信号：`LandscapeSnapshot.canonical_ensures` 是未过期的 `running` 租约。投影时，有活租约的 PDF 节点摘要追加 ` · 解析中 第a–b页`；租约结束后信号消失。`material.read` 在 `started` 和本 PDF 的 `blocked_lock` 时还返回 `details.map_signal = { kind: "parsing", label: "解析中", node_id, pages }`。前端 Map Scene 的专门样式尚未接入，目前依赖节点摘要文本。

## 非目标

本切片明确不做：

- 把多段快照拼成一份新的全书 Markdown，或原地改 Artifact 字节
- 旧 SourceSpan / discovery 的 supersede、redirect、重挂
- 新的解析工具（ensure 只在 `material.read` 内部）
- grep / search 触发 ensure
- 搜索索引改写（搜索仍可能列出物理快照，但会带上 `canonical_id`）
- 沙箱 composer 的解析 chip、Map Scene 的专门「解析中」样式
