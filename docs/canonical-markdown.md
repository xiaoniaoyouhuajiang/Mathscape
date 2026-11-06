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
- `complete`：页数已知，并集盖住全书，**并且 head 自己盖住全书**。片段即使拼起来盖住全书，也仍然不是 complete：`material.read` 会按页拼接返回（见下），但注册表里没有一份覆盖全书的快照。
- `partial`：请求区间没有被 `covered` 完全盖住，**或者** `complete` 为 false。因此只读到第 12–40 页时，即使这次请求的页都有文本，`partial` 仍为 true。`status=ok` 不能理解成全文可搜。

## MCP

不新增工具。已有四个工具在命中 canonical 时多带 `canonical_id`、`coverage`、`partial`：

- `material.read`
- `material.grep`
- `workspace.search`
- `pdf.outline`

PDF 的 `pdf_pages` 精读（无论请求页有没有原生文本层，正文都只来自 canonical Marker Markdown）：

- 已有一份快照的页区间 **完全包含** 请求：返回该快照的 Markdown 字节（包含请求的 head 优先，否则取最新的包含快照；会在该 PDF 的所有 lineage 中找），`content_type` 为 `markdown`，`served_artifact_id` 指向物理快照。不按页切开字节；区间以 `coverage` 为准。超出 32KiB 仍是 `truncated`。只与请求相交的单份快照不会冒充整段请求返回，而是走下一条按页拼接。
- 没有单份快照完全包含请求，但该 PDF 全部 lineage 的快照**按页合起来**盖住请求：不跑 Marker，按页拼接返回（`mathscape_domain::plan_page_cover`）。贪心选段：从请求起页开始，在包含当前页的快照里选伸进请求最远的一份；同样远时依次取排名更前的 lineage（与 `all_for_pdf` 同序）、head、`finished_at` 更新者、id。每段在 Marker 页标记 `{N}----------------`（N 为 0 基绝对 PDF 页号）处切到它负责的页；段边界与快照边界重合时不需要标记；需要切却找不到标记时整段保留，`trimmed_to_pages: false` 并给诊断 `stitched_snapshot_untrimmed`（宁可重复，不丢文字）。段间以空行相接，整体仍受 32KiB 上限（`truncated`）。结果没有 `served_artifact_id`，改为 `details.served_segments`：每段 `artifact_id`（物理快照）、`canonical_id`、`pages`（本段负责的请求页）、`snapshot_pages`、`md_byte_range`（该快照内被选中的字节，可原样交给 `material.read` / cite）、`trimmed_to_pages`、`is_head`；诊断 `served_stitched_canonical_markdown`。顶层 `canonical_id`/`coverage`/`partial` 取第一段所在 lineage。
- 有请求页没有任何快照覆盖：进入下面的内部 ensure，**在同一次调用里同步等待** Marker 完成，Marker **只解析缺口页**（见「范围」）。ensure 成功后重新加载视图，按上面两条（单份包含 → 按页拼接）返回（`status: ok`）；否则返回 `capability_unavailable`，`details` 里带 Marker 的 CapabilityProbe 与 `ensure` 结果（`failed` 带明确原因）。没有「已启动，稍后再读」的中间态，Agent 不需要也不应轮询。

精读永不静默回退到 pdf crate 原生文本层：有文本层的页同样走「单份快照包含 → 按页拼接 → 否则同步 ensure 补缺口 → 否则 `capability_unavailable`」。失败路径上只对前 8 个请求页做一次文本层探测，仅用于诊断：有文本的页给出 `pdf_text_layer_not_served`（附 `pages_with_text`），全无文本且 Marker 不可用时给出 `pdf_no_text_layer`。`content_type` 恒为 `markdown`，不再返回 `page_spans`。

用 canonical id 调用 `material.read`（`md_byte_range`）或 `material.grep` 时，解析到 head 快照再读。这不是 artifact 重定向：旧的物理 artifact id 照常可读，并带上同一个 `canonical_id`。

`material.grep`、`workspace.search`、`pdf.outline`、`pdf.page_image` 永不触发 ensure。`material.grep` 接受 PDF id，**canonical 优先**：
- 该 PDF 有当前 lineage（与 `CanonicalAliasTable` 同一规则：默认 profile 优先，其次 policy 自然序最新，再次页数最多）的 head，且 head 在 catalog 中、是 markdown、有受管副本：扫描该 lineage 覆盖账本里的**全部** markdown 快照（页并集 = `coverage.covered`，不只是 head）。先选 head，再按 `finished_at` 由新到旧加入还能补上新页的快照；页已被已选快照完全覆盖的快照不扫，避免重叠重解析重复命中；不在 catalog、不是 markdown 或没有受管副本的快照记入 `skipped_snapshots`（附 `reason`），其页不算已搜。命中的 `artifact_id` 为**命中所在的物理快照**，locator 为该快照内的 `md_byte_range`，并附 `canonical_id`/`coverage`/`partial`（与直接 grep 该 markdown id 一致，可原样交给 `material.read`）。其他 lineage 不扫描。
- 并集之外的页（或 `total_pages` 未知时 head 未记录页区间的情况）：用 `mathscape-pdf` 逐页扫描原生文本层（字面子串，不跨页），沿用 1MiB 文本层预算；命中带 `surface: "pdf_text_layer"` 与 `pdf_pages {from,to}`（均为命中页），并附当前 lineage 对该页的 `canonical_id`/`coverage`/`partial`。同一条命中永不同时带两种 locator。文本层读不了（受管 PDF 缺失或解析失败）不丢弃 markdown 命中，只在诊断里给 `text_layer_unavailable`。
- 诊断 `pdf_resolved_to_canonical` 如实给出：`canonical_id`、`head_artifact_id`、`searched_pages`（markdown 快照页并集）、`searched_snapshots`（每项 `artifact_id`/`pages`/`head`）、`text_layer_pages`（实际扫了文本层的页）、`unsearched_pages`（两者都没搜到的页，例如命中上限或 1MiB 上限后）与 `partial`；有文本层命中时另给 `pdf_text_layer_hits`。文本层上限的 `scan_capped` 诊断里 `scanned_pages` 为页区间数组。
- 没有可用 head：逐页扫描整份原生文本层，命中形状同上；若该页已有 canonical 视图则附带 `canonical_id`/`coverage`/`partial`。head 存在但不可读（`head_not_in_catalog`/`head_not_markdown`/`content_unavailable`）时另给诊断 `canonical_head_unavailable`。
- grep 命中只是定位，不是可引用正文：snippet（尤其文本层的）只用于定位；引用或建节点一律用命中的 `artifact_id` + locator 调 `material.read`（`pdf_pages` 精读会按需先解析）。
- 以上路径都不运行 Marker。

## Ensure（`material.read` 内部路径）

ensure 是 `material.read` 的内部步骤，不是 Agent 可调用的工具。它只把缺页解析成 **新的** 不可变快照，从不改写或删除已有 Artifact 字节。实现见 `crates/mathscape-mcp/src/tools/ensure.rs`，台账见迁移 `0022_canonical_markdown_ensure`。

### 范围

- 已解析的页永不重新解析。`details.ensure.missing` 是请求内、所有 lineage 并集都没覆盖的页；Marker **每个缺口跑一次**（按页序，`details.ensure.marker_ranges` 依次列出），每次运行各自加锁、计预算、有幂等键、产出一份不可变快照；缺口之间夹着的已覆盖页不会被重解析。每跑完一段都回到第 1 步重新规划，所以排队期间别的读取补上的页也不会重跑。之后按页拼接返回。
- 为什么不用一次多段：Marker 的 `--page_range` 支持 `0-1,3-4` 这类多段写法（2026-11-06 本地实测：输出只含这几页的页标，用时与分段运行相当），但一次转换运行（`ConversionRun.page_range`）在覆盖模型里是一段连续页；把一份多段输出拆成几份快照需要伪造运行记录，破坏来源链。逐段运行用时相当且不改覆盖模型。
- `ensure-v2`（同日较早版本）用过「缺口外包区间」作为单一 Marker 范围，会重解析缺口之间的已覆盖页，已由 `ensure-v3` 取代。
- 2026-11-06 之前的规则是「请求必须被单份快照完全包含，否则整段请求重跑」，会让跨两份快照的请求（例如已有 1–4、5–10 时读 1–8）在 `missing` 为空时仍整段重跑 Marker（沙箱 `sbx-20261101T235040-bc83e994` 实测）。
- 单次最多新解析 `max_pages_per_request` 页（默认 12），按各缺口页数之和计，不按请求页数或缺口外包区间计。超过时返回 `blocked_budget`，不运行 Marker，提示缩小 `pdf_pages`。已全部覆盖的请求不受这个上限影响（返回仍受 32KiB 上限）。
- profile 固定为 `mathscape.marker.math-ocr.v1`；policy 为 `marker-{探测到的版本}`，所以 Marker 升级会开新 lineage。

### 写入顺序

1. 规划：已有单份快照完全包含请求，或已有快照按页合起来覆盖请求 → `reused`，不跑 Marker；否则取第一个缺口为本次 Marker 范围；第 8 步之后回到这里，直到没有缺口（`expanded`）或某一段失败。
2. 探测 Marker（与 CapabilityProbe 同一候选顺序：`MATHSCAPE_MARKER_EXECUTABLE`、`MATHSCAPE_MARKER_VENV`、默认 venv、`PATH`）。不可用 → `capability_unavailable`。
3. 加锁：SQLite `BEGIN IMMEDIATE` 内，先把租约过期的 `running` 行标为 `abandoned`，再依次检查本 PDF 的活租约（每 PDF 互斥）、全局运行数（Marker 信号量，默认 1）、滚动页预算（默认每小时 96 页，统计窗口内 running / succeeded / failed 的页数）。都通过才插入 `running` 行。被本 PDF 或全局槽占住时 **排队**：每 `poll_interval`（默认 500ms）查一次活租约，租约结束后回到第 1 步重新规划（对方已覆盖请求 → `reused`，否则再抢锁）。排队和运行共用同一个同步截止时间。
4. 运行 Marker（工作线程；调用方阻塞等待）。租约很短（`lease_ttl` 默认 60 秒），等待期间每 `heartbeat`（默认 15 秒）续一次；持有者进程消失后续约停止，租约最多一个 TTL 后过期，被下一次加锁标为 `abandoned`。Marker 进程超时 = min(`marker_timeout`, 剩余同步时间)。
5. 校验产物清单（`is_promotable`），失败即 `failed`。
6. promotion：写入新的内容寻址 Artifact 与 `ConversionRun`（记录 `page_range`、`marker_profile`）。
7. 只对该 PDF 做 `sync_canonical_markdown_for_pdf`，更新注册表与 head。
8. 写终态回执（`succeeded` / `failed`，含 `conversion_id`、`markdown_artifact_id` 或错误），租约随之释放。

### 幂等

幂等键：

```text
{pdf_artifact_id}|{canonical_id}|{from}-{to}|{parser_profile}|{parser_policy_version}|ensure-v3
```

`ensure-v3`（2026-11-06）起 `{from}-{to}` 是单个缺口（`ensure-v2` 时是缺口外包区间，更早是整段请求）；一次读取有几个缺口就有几个键。重复请求先经过第 1 步的覆盖检查；台账 + head + 注册表决定是否已有结果，所以已覆盖的请求不会再跑 Marker。同一 PDF 并发的第二个请求在第 3 步排队等第一个跑完，然后在第 1 步得到 `reused`（`waited_for` 记录等过的租约），整个过程只跑一次 Marker。

### 状态

`details.ensure.status`：

| 状态 | 含义 | 结果 |
| --- | --- | --- |
| `reused` | 已有快照覆盖请求（单份包含或按页拼接） | 直接返回快照或拼接结果 |
| `expanded` | 本次 Marker 逐个解析了全部缺口，新快照已登记 | `status: ok`，返回新快照或与旧快照的拼接结果 |
| `blocked_budget` | 请求页数或滚动预算超限 | `capability_unavailable`，不跑 Marker |
| `capability_unavailable` | Marker 未安装或版本不支持 | `capability_unavailable`，附 probe |
| `failed` | Marker、清单校验、promotion 失败，或超过同步截止时间（`timed_out: true`）；多缺口时前面已成功的快照保留 | `capability_unavailable`，回执记录错误，租约已清除 |

同时会有 `canonical_ensure_{status}` 诊断；Marker 缺失时仍保留 `pdf_no_text_layer`。`details.ensure` 还带 `requested`、`missing`、`marker_ranges`（本次各次 Marker 范围）、`runs`（每次运行的 `marker_range` / `ensure_id` / `idempotency_key` / `conversion_id` / `markdown_artifact_id`）、`canonical_id`，以及最近一次运行的 `ensure_id`、`idempotency_key`、`conversion_id`、`markdown_artifact_id`，排过队时带 `waited_for`（等过的租约）与 `waited_ms`。

顶层 `status` 只在调用结束时给出：解析进行中调用仍在阻塞，因此不会出现「解析中却 `capability_unavailable`」。顶层 `ToolStatus` 没有 `failed` 变体（契约枚举不变），失败以 `capability_unavailable` + `details.ensure.status = failed` 表达。

### 同步时限

`sync_timeout` 默认 15 分钟，覆盖排队 + Marker。到点后取消 Marker（`MarkerCancellation`），给 10 秒收尾；仍未结束就把本次 ensure 记为 `failed`（`timed_out: true`）并清除租约，返回 `capability_unavailable`。只在排队阶段超时则不会写台账行。Codex 侧 `tool_timeout_sec = 960`（16 分钟），比同步时限多出收尾余量。

### 副作用与工具注解

`material.read` 因此不再是只读：可能写入工作区 parse cache（新 Artifact、`ConversionRun`、注册表行、ensure 台账），可能在本机运行 Marker。注解为 `readOnlyHint: false`、`destructiveHint: false`、`idempotentHint: true`、`openWorldHint: false`。其余六个工具仍是只读注解。Codex 侧 `mcp_servers.mathscape.tool_timeout_sec = 960`，并把 `MATHSCAPE_MARKER_EXECUTABLE`、`MATHSCAPE_MARKER_VENV`、`XDG_DATA_HOME` 转发给 MCP 进程，让 ensure 找到与桌面端相同的 Marker。

### 已知残留

MCP 进程在 Marker 运行中退出时，工作线程随之终止，心跳停止，`running` 行最多保留一个 `lease_ttl`（默认 60 秒）后过期，再被下一次加锁标为 `abandoned`。在此之前同一 PDF 的读取会排队等待（不报错）。被遗弃的 Marker 子进程可能仍在运行到自身超时。

## 地图

仅当该 PDF 已有注册表行时，材料图把这条 lineage 的 head 与有页码的 Markdown 快照收成 **一个** 派生节点。节点 id 是 `canonical_id`（仍用 `SourceArtifact` 这个投影种类，避免新的领域类型）。转换边和 `located_in` 改指向该节点。图片、元数据 JSON 仍留在图上。

`graph.get` 既有的 `canonical_id` 字段继续表示「解析后的图节点 id」。对这个视图节点，它就是逻辑 canonical id；`content_ref.artifact_id` 指向 head，以便原样交给 `material.read`。SourceSpan 的字节定位仍指向原来的物理快照，本切片不重挂。

未 sync 的历史转换保持原图：每个物理 Markdown 仍是独立节点。

**静默同步**：Desktop 每次打开沙箱 / 加载地图（`DesktopService::snapshot`）前调用 `sync_canonical_markdown_if_stale`：比较注册表与目录投影（忽略 `finished_at` 等仅活投影才有的字段），不一致才整表重建；一致则什么都不写。没有按钮，失败只记日志、不阻塞打开。因此 Desktop 自己跑完的解析、以及升级前的历史数据，都会在下次加载时自动折叠。

**物理 → 规范 alias 表**（`mathscape_query::CanonicalAliasTable`，唯一来源）：每条 lineage 的 head 与全部有页码的快照都映射到它的 `canonical_id`；同一快照被两条 lineage 认领时记为冲突：该快照不折叠（仍是独立节点 / 命中），认领方节点带诊断——这是唯一会被看见的规范化问题。地图节点在线上带 `aliased_artifact_ids`（head + 非 head），前端 `aliasIdsOf(node)` 是定位（`findSourceArtifactNode`）、cite 索引（`buildCiteIndex`）、chip 与 @ 共用的同一份 alias；搜索折叠用同一张表。

**旧 lineage 降级**：每个 PDF 只有一条「当前」lineage 叫「规范解析正文」——默认 profile（`mathscape.marker.math-ocr.v1`）优先，再按 policy 版本自然序取最新，再按覆盖页数。其余 lineage 改名为「{pdf} · 解析正文（旧版 · {profile} · {policy}）」，摘要追加「旧版 lineage」，因此地图上不会出现两个同名「规范解析正文」节点。仍可阅读、定位、引用，只是不再冒充当前版本。

「解析中」信号：`LandscapeSnapshot.canonical_ensures` 是未过期的 `running` 租约。投影时，有活租约的 PDF 节点摘要追加 ` · 解析中 第a–b页`；租约结束后信号消失。心跳续约让信号在整个同步等待期间保持。

`material.read` 结束时，若本次调用跑了 Marker 或排队等过本 PDF 的解析，结果带 `details.map_signal = { kind: "parsing", label: "解析中", node_id, pages, phase: "done" | "failed", waited_ms }`，作为事后的场景记录。

Map Scene：前端把进行中的 `material.read`（带 `pdf_pages`）视为可能的解析阻塞。调用开始约 2.5 秒后仍未返回时刷新地图；该 PDF 节点摘要出现「解析中」，就播放一个 `parsing` 幕：Scene HUD 标签显示「解析中」，PDF 节点加 `scene-parsing` 高亮；过程卡里正在运行的步骤标题追加「· 解析中」。工具结果到达后信号随下一幕/收尾清除，结果幕按 `map_signal.phase` 显示「解析完成 第a–b页」或失败原因。

## 统一阅读投影（Desktop）

地图上的「… · 规范解析正文」节点点「阅读文件」时，阅读器打开 **一个** 连续视图：按页序拼接注册表里该 canonical 的全部覆盖段，而不是只打开 head 快照。这是只读投影：不写新的 Artifact、不写台账、不改任何快照字节；每段文字都从原物理快照现读。

- 后端：Tauri 命令 `get_canonical_reader_projection(canonical_id)`（`apps/mathscape-desktop/src/canonical_reader.rs`）。只读已登记的注册表（`list_canonical_markdown`），未 sync 的 canonical 返回「规范解析正文未登记」。`get_artifact_content_info` / `read_artifact_bytes` 仍只接受物理 id；MCP 契约不变。
- 规划：`mathscape_domain::plan_canonical_reader`（纯函数，有单测）。
- 前端：`readerTargetId(node)` 对折叠节点返回 canonical id，`ArtifactReader` 识别该前缀后走统一视图。手写批注、cite chip、发现记录仍用物理 head（`openableArtifactId`）。

**拼接 / 重叠策略**（确定性）：

1. 覆盖段按 `from` 升序；同 `from` 时更宽的段（`to` 大）优先，再 head 优先，再按 artifact id、conversion id。相同 artifact 且同页的重复段去重。
2. 从第 1 页起扫描游标：段的 `to` 小于游标 → 该段被前文完全覆盖，列入 `skipped`，不显示；段的 `from` 大于游标 → 先插入缺口 `gap {游标..from-1}`；否则该段贡献 `max(from, 游标)..to`，与前文重叠的头部记为 `overlap`。
3. `total_pages` 已知时，末尾未覆盖页也作为缺口；未知时不猜。
4. 有重叠时，在快照中找 Marker 页标记 `{N}----------------`（N 是 0 基绝对 PDF 页号）里 `N = 贡献起页 - 1` 的那一行，从那里开始显示（`overlap_trimmed: true`，`byte_start` 为该字节偏移）。若快照没有页标记、任一页标记落在该段声明页范围外（防止相对页号误裁），或找不到目标标记，就整段显示并标注「与前文重叠（重复显示）」（`overlap_trimmed: false`）——宁可重复，不丢文字。
5. 没有任何带页码的段但有 head 时，head 整段显示（页码未知），其后整本标为缺口。

即「先到先得」：已显示的页不会因为后来补解析的段而改变，新段只追加它带来的新页。缺口显示为明显的占位框「第 x–y 页尚未解析」，不补写任何内容。拼接后正文上限 16 MiB（与单个 Markdown 阅读上限相同）。

统一视图不提供原文块选择、发现记录和手写批注（这些需要物理字节坐标）；每段标题旁的「打开此快照」以物理快照打开，功能照旧。

## 搜索（Desktop / MCP）

Desktop 搜索框与 `workspace.search` 共用 `MaterialProvider`。搜索语料（`read_search_corpus`）在同一读事务里带上注册表行，用上面的 alias 表折叠：

- 同一 lineage 的 N 个「… · 解析正文」物理快照 → **一个** 命中，`id` / `target.artifact_id` 都是 `canonical_id`，标题「{pdf} · 规范解析正文」，副标题「规范解析正文 · 覆盖第 a–b 页 · 合并 N 份快照 · 来自 {pdf}」。被折叠快照的标题、文件名仍作为别名参与匹配。
- PDF 本身仍是独立的一行；旧 lineage 是单独一行，标题带「旧版」。
- 「定位」直接选中地图上的规范节点；「打开」用 canonical id 走统一阅读视图。
- 未注册（未 sync）的快照照旧逐个列出。MCP 对折叠命中用 `by_id` 补上 `canonical_id` / `coverage` / `partial`，对 PDF 与未折叠快照的行为不变。

## 非目标

本切片明确不做：

- 把多段快照拼成一份新的全书 Markdown **Artifact**，或原地改 Artifact 字节（Desktop 的统一阅读视图只是读时投影，见上节）
- 旧 SourceSpan / discovery 的 supersede、redirect、重挂
- 新的解析工具（ensure 只在 `material.read` 内部）
- grep / search 触发 ensure
- ~~搜索索引改写~~：已纳入范围，Desktop 与 MCP 搜索按 lineage 折叠（见「搜索」）
- 把 agent 日常读取从 `material.read pdf_pages` 改成别的工具；删除或改写任何物理快照字节
- 沙箱 composer 的解析 chip、Map Scene 的专门「解析中」样式
