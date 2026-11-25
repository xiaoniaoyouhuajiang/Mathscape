---
name: mathscape-worknet
description: 在 Mathscape 工作区中帮助用户搭建可回溯的数学地图（来源锚定、对象卡、表示、证明上下文、失败见证）。用户在 Mathscape 沙箱或调查中请求阅读材料、整理对象或构建工作网时使用。
metadata:
  version: "0.7.5"
  tool_schema_version: "mathscape-mcp/21"
---

# Mathscape 数学地图构建 skill v0.7.5

这份 skill 给在 Mathscape 工作区中帮助用户搭建数学地图的 Agent 使用。它的任务很简单：从用户关心的原文、手稿或问题出发，逐步整理出可回溯的数学理解，并把仍然不确定的地方留下来。

你不是在一次对话里“把数学全部证明完”。你是在建立一份可以继续修改的工作记录：原文在哪里、我们想说明什么、用了哪种表示、哪一步需要什么条件、证据是否完整。

Agent 只能提出候选和草案，不能自行接受、删除、退役或宣布数学结论已经可信。

## 0. 先用四句话理解这张地图

1. **来源层**回答“这句话从哪里来”：PDF、Markdown、页码、字节区间、来源单元或手稿笔迹。
2. **语义层**回答“这句话表达什么”：对象、符号约定、不同来源之间的对应关系和局部证明状态。
3. **工作网**回答“这条路线怎样走”：表示、同一表示内推理、跨表示运输、可展开的宏、未偿还条件和证据。
4. **用户视图**回答“现在给谁看、展开到什么程度”：研究者、学习者、证明检查器可能需要不同的展示。

EMIR 是 Mathscape 用来连接这些层的实验性数学中间表示。R/D/T/X 是工作网里的四种动作：

- **R（表示）**：当前用什么方式看问题，以及这种方式允许哪些对象和算子；
- **D（推理）**：在同一种表示内，从已有前提得到下一步；
- **T（运输）**：换到另一种表示，并说明如何把结果重建回去；
- **X（展开/压缩）**：把一组可以展开的定义、见证或桥接步骤收成一个宏。

D 不是跨表示的万能边，T 不是改名，X 不能隐藏需要证明的关键步骤。`proves` 只表示来源材料中的关系，也不是 Lean 证书。

## 1. 第一次收到任务时怎么开始

先不要创建节点或边，按下面顺序做：

1. 读 `references/reader-primer.md`，确认自己理解 EMIR、R/D/T/X、Debt、guard 和 evidence；
2. 读 `references/reference-index.md`，把本轮会用到的规范、案例和报告写成 `reference_reads`；
3. 查看本轮实际挂载的 mathscape 工具（harness 工具列表中可见的那些；MCP 协议层的 `tools/list` 是协议方法，不是可调用工具），按 `references/runtime-capabilities.md` 标记工具是 `implemented`、`contract_only` 还是 `unavailable`；
4. 建立一个 `worknet_run`，先写临时目标、来源范围和下一步；模板在 `references/run-state-template.md`；
5. 先查工作区已有内容，再决定是否需要创建来源单元；
6. 如果用户目标、来源范围、符号含义、章节指代或升格粒度无法确定，调用 `user.ask` 请用户拍板并立即结束本轮（见 §4.3）；没有该工具时把问题写进 `blocked_reasons`，**绝不要跨章节猜测**。

“先声明目标”指先写一个暂时的目标和范围，不表示还没读原文就可以编造完整对象卡。对象卡应在 S1 读取来源后形成。演示时 S1 和 S2 可以放进同一个提案，但不能省略其中任何一个检查。

## 2. 什么时候可以读到什么

默认的读法是：

```text
workspace.search（可指定 kinds: ["theory"] / ["semantic_entity"]，以及 scope: "investigation" 查本会话暂存）
  → graph.get（支持来源节点、共享对象、对齐线、一笔账、约定卡）
  → graph.neighborhood（默认即启用 preset: "heuristic" 跨书启发邻域：0 步长附着文件溯源/约定卡/挂账，1 步长走书内证明与跨书语义桥 aligns/uses_entity/of_concept/paid_by，并在非种子 Theory 书架与外部材料封面上自动止步防爆炸；推荐 depth: 2 触发跨书联想；若仅看单书结构可传 preset: "structural"，仅看抽象语义网可传 preset: "semantic_only"）
  → material.read / material.grep
  → ink.page_image 或 pdf.page_image（需要看版式或手稿时）
```

命中只告诉你“有这个对象”，原文正文必须通过 `content_ref` 回链读取；共享对象、对齐线、账和约定卡的结构化内容直接由 `graph.get` 的 `semantic` 字段给出。没有可靠 locator 时不能猜字节范围，也不能把模型摘要写进图节点来代替正文。

探索某条定理或共享对象的跨教材对应与引理依赖时，直接使用默认 `preset: "heuristic"`（`depth: 1` 或 `depth: 2`）；需要精确指定底层关系时才显式传 `edge_types`。

## 3. 五个阶段：每一步都用白话说清楚

### S1：来源锚定——先找到“原文在哪”

要做的事：

- 查已有 Artifact、SourceSpan、SourceSemanticUnit、手稿和邻域；
- 读目标定义、定理、证明段落或手稿选区；
- 判断是否已有可复用来源单元；只有缺少时才提议创建。

本阶段产出：

- 每个来源片段的 `content_ref`；
- 来源 revision 和定位；
- 来源单元类型和边界；
- 读不到的内容、重复候选和定位风险。

正式图的写入口只有当前运行时实际提供的 `propose_changes`。如果它不存在，可以用 `artifact.stage` 保存来源登记草案，但不能说来源已经进入正式图。

### S2：对象卡和方言——先说清楚“这次要证明什么”

要做的事：

- 用一句话写目标工具或定理；
- 写完整结论承诺、全局前提、参数范围和排除项；
- 把不同情形拆成分支，并写每个分支的量词域；
- 记录原文如何使用符号、标量域、自然数起点、测度和术语；
- 登记分支 guard，并写清分支是否互斥、是否覆盖全部情况、是否有 overlap。

本阶段不画证明网。对象卡不是每个实体一张，而是围绕一个目标对象的一张承诺卡。

如果分支或量词无法由来源确定，应停下来问用户或标记 blocked；不能为了让路线看起来完整而替对象补条件。

### S3：表示和绑定——决定“用什么方式看”

要做的事：

- 为实际会被核心边消费的表示建立 R；
- 写清底层载体、语义内容、合法算子和量词；
- 把来源方言绑定到共享表示；
- 只有重复结构会遮住主链时才引入 X，并写出完整展开。

如果没有跨表示运输候选，可以把 S3 的最小结果并入 S4；这不代表可以跳过对表示的说明。

### S4：证明网——在局部路线里画“每一步怎么走”

要做的事：

- 建立证明局部上下文和状态；
- 同一表示内使用 D；真正换表示时使用带重建证书的 T；
- 把 Debt、guard 和派生 lemma 分开；
- 每条核心边单独记录 evidence、分支、量词和边界；
- 执行 I1–I5，并记录 pass、fail 或无法判定的理由。

人工判断从这里开始明显增加。删掉某个 T 后如果已有 D/X 可以重建全部结论，就不要保留硬 T。只因为两种写法看起来相似，不能声明运输或对齐已经成立。

### S5：失败见证和第二路线——主动找“哪里会坏”

要做的事：

- 为 Debt 和分支边界找具体、可检查的失败见证；
- 区分 theorem-Debt、statement-Debt 和 route-Debt；
- 如果有独立的第二条路线，使用同一个 `object_ref` 分开建两条 route；
- 两条路线都完成 I1–I7 后，才比较路线对 `(O, ρ)` 是否等价；
- 记录 TrustRecord 的具体轴，不能用模型 confidence 或 Lean 通过替代来源忠实性。

没有失败见证时，不要编造反例；可以写“当前没有见证”，并说明这是否意味着该 Debt 不应保留。

### 合并规则

- 演示时 S1 和 S2 可以合并，但必须同时保留来源锚定和对象承诺；
- 没有 T 候选时，S3 可以并入 S4；
- S2 与 S4 不合并：先确定对象、前提和量词，再画证明网；
- S5 不是每次都必须有第二路线，但如果声称路线等价，就必须完成完整比较。

## 4. 现在的工具边界

不要把文档中的计划能力当成当前 API。每轮先看本轮实际挂载的工具列表。

已实现的读工具通常包括：`workspace.search`、`graph.get`、`graph.neighborhood`、`material.read`、`material.grep`、`material.locate`、`pdf.outline`、`ink.page_image`、`pdf.page_image`、`mathlib.search`、`lean.check`。

`artifact.stage` 是受 round 约束的文件暂存能力。它可以保存对象卡、工作网草案、Lean 文件和检查报告，但不会登记正式 Artifact、地图节点或关系。

`propose_changes`、`get_change_set`、`list_change_sets` 和 `user.ask` 只有在本轮工具列表中实际出现时才能调用。共享对象、对齐线、一笔账和约定卡直接用 `propose_changes` 提（§4.2）；其余语义层的 `emir.upsert_*`、`rdtx.upsert_*`、`rdtx.record_failure` 等目前属于契约草案，必须作为 `propose_changes` 的薄封装，不能另开一条绕过用户接受的写路径。

### 4.1 `propose_changes` 怎么写（`mathscape-mcp/17`）

只在本轮工具列表里有 `propose_changes` 时使用。它只产生 `staged` 或 `blocked` 的提案，从不改正式地图；`staged` 的对象会以「未审批」样式出现在用户地图上，用户之后在 Desktop 统一接受或丢弃。接受只表示写入地图，不代表数学上已验证，也不会提升 Trust。

一次提案 = 一个 change set，用户整批接受或丢弃，不会只接受其中几项。`request_id` 用于幂等：同一 `request_id` 重发同样内容得到原结果，内容不同会报错。`rationale` 首句必须带上显式建图动机标签（如 `【来源锚定】`、`【跨书对齐】`、`【题型提炼】`、`【概念/定理升格】`、`【纠错重连】`、`【补账/销账】`，详见 §4.3），让用户在审核面板一眼看清本次变更的立意。

**来源片段 `source_span`**：只给定位，服务端补齐身份、hash、块和页。

```json
{ "local_ref": "s1", "kind": "source_span", "op": "create",
  "payload": { "artifact_id": "<material.read 返回的 Markdown 快照 id>",
               "md_byte_range": { "start": 120, "end": 188 },
               "granularity": "page",
               "quote": "<这段字节范围里的全部原文>" } }
```

**流程：先定位，再提案。** 不要自己数字节、试读小窗口或写脚本算偏移：

1. `material.read` 读页（`pdf_pages`）。返回的 `served_artifact_id` 或 `details.served_segments[].artifact_id` 是快照 id；`served_segments[].md_byte_range` 是所读页在该快照里的范围。正文从快照第 0 字节开始，单快照时正文里的偏移就是快照字节偏移。
2. 从读到的正文里原样复制一小段（一道题、一个定义、一段证明），调用 `material.locate {artifact_id, quote, within?}`。`within` 可填上一步的 `md_byte_range`，把搜索限制在这几页。
3. `status: ok` 时把返回的 `md_byte_range` 和**同一个** `quote` 原样放进 `source_span`，R2 引文核对一定通过；`ambiguous` 时从 `candidates` 里按上下文选一个，或加长 quote / 缩小 `within` 再查；`not_found` 说明这段文字不在该快照里，重新读原文。
4. 粒度按 locate 结果选：`page_granularity_available` 为 true（返回了唯一的 `page`）时**用 `page` 粒度**，页码才会存进 span；只有拿不到页（`page` 为 null，诊断 `page_markers_absent` 或 `page_unresolved`）时才用 `inline`。
5. **不要声称没有存下来的页码。** `inline` span 不记录页码；如果你用了 `inline`，回复里就不要说「锚定到第 N 页」，可以说「位于快照某字节范围，页码未记录」。提案里出现 `page_granularity_available`（info）说明这个 inline 范围其实能落到唯一一页，下次改用 `page`。

规则：

- `quote` 必须是**整个范围**的原文，不能只写开头或结尾，也不能用「…」省略中间。服务端按范围截出原文再和 `quote` 比较（先精确，再忽略空白、TeX 间距命令和外层 `$…$` 差异），不会在全文里帮你找。
- 单个 span 最长 4096 字节（`span_too_long`）。一章不是一个 span：按题、按定义或按段拆成多个小 span，再分别挂到各自的来源单元上；章节单元可以只引用标题那一小段。
- `granularity` 取 `inline`、`block` 或 `page`。`block` 要求范围内有 canonical block，`page` 要求范围能唯一落到一页，否则 blocked。locate 给出唯一页就用 `page`；locate 没给页时用 `inline`。
- 快照没有分页标记（`material.read` 或 `material.locate` 给出 `page_markers_absent`）时只能用 `inline` 粒度，也不要自己推断页码。用户直接导入的 Markdown、其他来源转换的 Markdown，以及部分早期解析的快照都可能没有分页标记，这是正常情况，不要为此重新解析。
- 同一快照同一范围的 span id 是确定的：已经在地图上就会自动变成引用（`span_already_exists`），不会重复创建。
- 不要填 id、hash、页码、块 id 等字段；多写字段会被拒绝。新建的 span 必须被本次提案里的单元或关系用到，否则 `unused_span`。

**来源单元与关系**：

```json
{ "local_ref": "u1", "kind": "source_unit", "op": "create",
  "payload": { "kind": "statement", "display_title": "定理 2.1",
               "parent": null, "source_spans": ["$new:s1"],
               "completeness": "complete" } }
```

```json
{ "local_ref": "r1", "kind": "source_relation", "op": "create",
  "relation": "proves", "source": "$new:u2", "target": "$new:u1",
  "evidence_spans": ["$new:s2"] }
```

- 单元 `kind`：`section`、`definition`、`statement`、`proof`、`problem`、`hint`、`solution`、`example`、`remark`、`method`、`notation`、`narrative`；`completeness`：`complete`、`left_truncated`、`right_truncated`、`internally_missing`、`uncertain`。关系 `relation`：`proves`、`solves`、`hints`、`explains`、`cites`、`applies_method`。
- 修改已有对象用 `op: "edit"`，带 `target_id`、`expected_revision` 和 `changes`。
- **连错的书内关系可以提议退役**：`{ "local_ref": "x", "kind": "source_relation", "op": "retire", "target_id": "<关系 id>", "expected_revision": "<rev>" }`，只带这两个字段，放 `proposed_relations`（放 `proposed_objects` 也收）。关系连错了就退役它再建对的，不要只用 edit 把几条错关系改指同一个目标。来源单元和原文证据仍不能提议退役，需要时告诉用户在 Desktop 里处理（语义层对象的退役见 4.2）。
- 书内关系的 create / edit 只放 `proposed_relations`；放进 `proposed_objects` 会报 `wrong_slot`。
- 顺序随意：服务端在最终图上统一校验，子单元写在父单元前面也可以。

**引用规则**：

- `$new:<local_ref>` 指本次提案里的新建项。`local_ref` 只用字母、数字、`_`、`-`、`.`，首字符是字母或数字，最长 64 字节。
- `mathscape:` 开头的稳定 id 指正式地图上的对象，或**本调查**里其他仍为 `staged` 的提案对象（用 `graph.get` / `graph.neighborhood` 的 `scope: "investigation"`，或先 `list_change_sets` 再 `get_change_set` 的 `staged_ids` 查到）。引用别的提案就形成依赖：用户必须先接受或同批接受被依赖的提案。
- 裸写 `local_ref`（不带 `$new:`）这一版还能用但会收到 `bare_local_ref_deprecated`，下一版移除。
- 本调查 = 整场沙箱会话：开「新对话」后，之前对话里的暂存提案仍然可见、可引用。

**修订**：同一个沙箱对话通常延续同一条 Codex 线程，之前轮次的工具结果仍在你的上下文里；但线程可能在换机器、长时间闲置或版本升级后被恢复甚至降级为转录重建（届时会有「上下文已降级」提示），而且早先的结果可能已过时。所以不要凭记忆里的旧提案 id 或状态行事。**重提或修改之前的提案前，先调用 `list_change_sets`**（只列本调查 = 本沙箱会话的提案，新的在前；`pending_only: true` 只看 staged/blocked）。按 `title`、`request_id` 和 `active.blocking_codes` 找到要改的那份，把它的 `supersedes` 对象原样放进 `propose_changes`，重新提交**整份**提案；需要看旧内容时再用 `get_change_set` 读回。这样得到的是同一个 change set 的下一个版本（v2、v3…），旧版本被折叠；不带 `supersedes` 就会新建一个独立提案，旧的 blocked 版本还留在用户的审核队列里。`supersedes` 为 null 表示该提案已被接受或丢弃，不能再修订，需要的话新建提案。同名 `local_ref` 得到同一个 id。被替换的提案上，依赖它的其他提案会转为 `blocked`（`dependency_superseded`，更深一层是 `dependency_blocked`），不会自动改指新版本，需要你读回后重新提交。

**汇报数量以返回结果为准**：告诉用户「提了多少单元 / 关系 / 证据」时，只按本次 `propose_changes` 的返回（或 `get_change_set` 读回）计数，不要凭记忆或按你打算写的条目估算。`staged` 时数 `staged_ids` 的值：前缀 `mathscape:source-unit/` 是单元、`mathscape:source-relation/` 是关系、`mathscape:source-span/` 是证据，`mathscape:semantic-entity/` 是共享对象、`mathscape:alignment/` 是对齐线、`mathscape:debt/` 是一笔账、`mathscape:dialect-scope/` 是约定卡（`compiled_operations` 的条数应与之相同，可用来核对）；同时说明 `change_set_id` 与 `version`。`blocked` 时没有 `staged_ids`，只能说「提交了 N 条、全部未暂存」，并列出阻塞码。只说本次提案的数字；与地图里已有对象或其他提案的关系要单独说明，不要加进总数。审核界面按快照分组显示，一份提案可能跨多个快照，用户看到的总数以整份提案为准。

**常见诊断码**（`blocked` 时整份提案保留，改好后用 `supersedes` 重提）：

| 码 | 含义 |
| --- | --- |
| `quote_mismatch` | 范围原文和 `quote` 不一致（常见原因：quote 只写了开头）；用 `material.locate` 重新取范围 |
| `span_too_long` | 范围超过 4096 字节；按题或按段拆成多个 span |
| `range_out_of_bounds` / `invalid_utf8_boundary` | 范围越界或切在字符中间 |
| `snapshot_not_found` / `snapshot_not_immutable` | `artifact_id` 不是已登记的 Markdown 快照 |
| `block_locator_unavailable` / `page_mapping_unavailable` | `block` / `page` 粒度定位不到；改用 `inline` |
| `page_markers_absent`（`material.read` / `material.locate` 的诊断） | 快照覆盖多页但没有分页标记，页映射不可用；用 `inline` 粒度，不要声称页码 |
| `page_unresolved`（`material.locate` 的诊断） | 这段范围落不到唯一一页（跨页，或快照不属于任何 PDF）；用 `inline` 粒度 |
| `invalid_locator` | 本提案里的原文证据缺少物理定位（没有 Markdown 范围、页或手稿选区）；`target_id` 指出是哪个对象 |
| `span_inactive` | 同一范围的 span 已被用户退役；不要自动重建，告诉用户 |
| `unused_span` | 新建的 span 没被引用 |
| `wrong_slot` | 书内关系的 create / edit 写进了 `proposed_objects`；挪到 `proposed_relations`，字段是顶层的 `relation` / `source` / `target` / `evidence_spans`（edit 用 `changes`） |
| `dangling_new_ref` / `dangling_staged_ref` | `$new:` 没定义，或引用的暂存对象不在本调查的 staged 提案里 |
| `invalid_local_ref` | `local_ref` 不合规 |
| `change_set_dependency_cycle` | 提案之间互相依赖 |
| `dependency_superseded` / `dependency_blocked` / `dependency_discarded` | 被依赖的提案被替换、变 blocked 或被用户丢弃 |
| `parent_cycle`、`mixed_source_context`、`cross_source_parent` 等 | 最终图结构不合法 |

`span_already_exists`（info）、`page_granularity_available`（info：这个 inline 范围能落到唯一一页，改用 `page` 才会存页码）、`span_granularity_ignored`（warn）和 `head_integrity`（warn）不阻塞提案。`head_integrity` 表示工作区里**已有**、且本提案没有碰到的对象本身有问题（`target_id` 是该对象）：它与本提案无关，不要为此修改或重提提案，在交接时告诉用户即可。只有本提案及其依赖的提案新增或修改的对象出错，才会让提案 blocked。诊断里 `local_ref` 指本提案的条目，`target_id` 指地图对象 id。

### 4.2 语义层：共享对象、对齐线、一笔账、约定卡（`mathscape-mcp/17`）

契约见 `docs/semantic-write-contract.md`。语义对象和来源对象一样只进暂存，用户接受后才写入地图；**接受不提升任何可信度**。语义对象全部写在 `proposed_objects` 里，引用本次新建项一律写 `$new:<local_ref>`（不接受裸 `local_ref`），引用已有或其他暂存提案的对象写稳定 id。

| `kind` | 白话 | 用途 |
| --- | --- | --- |
| `semantic_entity` | 共享对象（概念 / 精确说法 / 理论 / 题型） | `semantic_kind` 只能是 `concept`、`declaration`、`theory`、`problem_schema`；`theory` 本身 `home_theory` 必须为空，其余三种都要有 `home_theory`（指向一个 theory） |
| `alignment` | 对齐线 | 某条来源单元（`source`）说的就是 / 是特例 / 部分重合于某个共享对象（`target`）；至少 1 条 `evidence_spans` |
| `debt` | 一笔账 | 还欠的证明或检查，挂在对齐线或共享对象上（`attached_to`），写明 `quantifier`（量词）和 `failure_mode`（不还清会坏在哪） |
| `dialect_scope` | 约定卡 | 一本书 / 一节的编号与叫法约定（`anchor_unit` 是 section 单元，`applies_to: "anchor_subtree"`） |

**理论（`theory`）怎么建、怎么用（专题书架三原则）**：

1. **先找书架，没有再搭**：建任何共享对象前，先调用 `workspace.search { query, kinds: ["theory"], scope: "investigation" }` 查地图上已有的和本会话已暂存的理论。有能装下它的专题就直接复用，不要重复新建。
2. **按「一类专题」起人话名字，附上同义检索词**：理论只是一个回答“这几个概念和定理大概是哪一块儿的”专题书架（尺度类似课本一章/一个专题，如「线性递推数列」「矩阵与特征值」「数列极限」）。保持扁平（不建父子理论树），**不要用单个概念名或具体算式给理论起名**（如不要建「卡特兰数理论」或带 `=`、`$` 的绕口令名字），并填好 `aliases`（中英文常见叫法，3 个左右），方便后面再搜到它。
3. **同专题挂一起，以后可搬家**：关于某类对象的定理、恒等式（如关于 Fibonacci 数列的 Cassini 恒等式）直接跟该对象挂在同一个主理论书架下；跨专题引用直接写在 `uses_entities` 里。每条精确说法的前提与约定跟对象自己走，如果以后地图大了想拆分或重组书架，可以用 `edit` 修改非 theory 对象的 `home_theory` 平滑迁过去。

写法要点：

- **共享对象一律中英双名**：四种共享对象的 `preferred_labels` 都要同时写 `{ "lang": "zh", … }` 和 `{ "lang": "en", … }`（`lang` 只写 `zh` / `en`，不写 `zh-CN`），不管材料和用户用什么语言；英文名用标准术语，有 mathlib 名称时参照它。缺任一种是 `label_lang_missing`。名称或别名和已有的同类 theory / concept 撞上会得到 `possible_duplicate_name`，mathlib 名称和已有的同类 concept / declaration 撞上会得到 `duplicate_mathlib_decl`：先确认是不是同一个，是就复用已有对象。
- **共享对象按 mathlib 的写法**，在 `conventions` 里注明约定（如 `sequence_index_origin`，F₀ = 0），可在 `mathlib_baseline` 里写对应的 mathlib 名称。`mathlib_baseline.status` 只能写 `unchecked`：`confirmed` / `not_found` 只由服务端在用户本机的 mathlib 里查过之后写入。不确定名字时先用 `mathlib.search` 查；查不到就 `decl: null` 并在 `note` 里说明。也可以填 `aliases` 记录中英文常见别称。
- **关系**只有 11 种：`expresses`、`specializes`、`generalizes`、`instance_of`、`representation_of`、`logically_equivalent`、`definitionally_equivalent`、`exact_realization`、`notation_for`、`overlaps_with`、`incompatible_with`。**没有 `same_as`**（`same_as_forbidden`）。两边不完全一样时写 `overlaps_with` 或 `specializes`，并在 `lost_information` 里写清对不上的部分。
- **参数对照与数列逐项对照分开写**：`parameter_map` 是“第几条恒等式 / 第几个参数对第几条”（如灯塔的第 n 条 ＝ 共享的第 n+1 条，`role: statement_parameter`）；`term_index_map` 是数列的项对项（`index_shift: 0` 表示 f_k = F_k，数列没有错开）。向用户解释时不要说“下标平移”“数列错开”。
- **状态只写到候选为止**：对齐线 `status` 只能是 `unmatched`、`retrieved_candidate`、`context_checked`、`partially_aligned`、`ambiguous`、`disputed`、`incompatible`；`semantically_reviewed` / `accepted` 只能由用户在审核页确认。账的 `status` 只能是 `open`，或在你认为已还清时用 edit 改为 `claimed_paid` 并在 `paid_by` 里引用还清它的对象；`paid` / `waived` 只有用户能设。任何地方都不要写 `trust`（`forbidden_trust_claim`）。
- **一个提案围绕一个目标**：填 `target_object`（本次某个 `local_ref` 或已有 id），提案里放这个共享对象、它第一次需要的上下文（理论、概念）、连到它的对齐线和挂在上面的账。
- **edit 只改白名单字段**（`identity_field_edit`）：共享对象可改名称、`aliases`、白话标签、`home_theory`（把非 theory 对象迁到另一个理论书架）、约定的说明文字（不能改约定的值）、mathlib 基线（仍是 unchecked）、`description`、`uses_entities`；对齐线不能改 `source` / `target`；账不能改 `attached_to` / `debt_kind`；约定卡不能改 `anchor_unit`。要改精确说法的陈述、前提、变量或约定值就新建对象。
- **语义对象可以提议退役**：`{ "local_ref": "x", "kind": "alignment", "op": "retire", "target_id": "<id>", "expected_revision": "<rev>" }`。仍需用户接受；挂在被退役对象上的账会自动标为已失效；还有活动对齐线指向的共享对象不能单独退役。

```json
{ "local_ref": "a.yi.thm", "kind": "alignment", "op": "create",
  "payload": { "source": "mathscape:source-unit/…", "target": "$new:e.cassini",
               "relation": "specializes",
               "source_context": { "dialect_scope": "mathscape:dialect-scope/…" },
               "target_context": "mathscape:semantic-entity/…",
               "parameter_map": [ { "source": "n", "target": "n", "role": "statement_parameter",
                                    "map": "n_共享 = n_乙 + 1" } ],
               "term_index_map": [ { "source": "f_k", "target": "F_k", "index_shift": 0 } ],
               "required_assumptions": [ { "text": "f_k = F_k（k ≥ 1）", "debt": "mathscape:debt/…" } ],
               "lost_information": [ { "short": "缺共享的 n = 1", "claim": "…",
                                       "direction": "target_has_source_lacks" } ],
               "evidence_spans": [ "mathscape:source-span/…" ],
               "status": "retrieved_candidate",
               "map_label": "特例：乙的第 n 条 ＝ 共享的第 n+1 条" } }
```

语义层的阻塞码：`kind_not_supported`、`operation_not_supported`、`invalid_arg`、`invalid_relation`、`same_as_forbidden`、`invalid_reference_kind`、`missing_required_field`、`forbidden_trust_claim`、`forbidden_alignment_status`、`forbidden_debt_status`、`forbidden_baseline_status`、`identity_field_edit`、`revision_conflict`、`inactive_or_invalid_reference`，以及 4.1 的引用与依赖码。`relation_detail_missing`、`overlap_scope_missing`、`certificate_debt_suggested`、`dialect_scope_mismatch`、`duplicate_alignment`、`theory_name_too_specific`、`label_lang_missing`、`possible_duplicate_name`、`duplicate_mathlib_decl`（warn）和 `uncontextualized`、`inferred_convention`、`mathlib_baseline_unchecked`、`multi_target_change_set`、`theory_aliases_missing`（info）是给用户看的审核项，不阻塞；用户可能在对话里引用它们（「反馈给 agent」），那时读回提案再用 `supersedes` 修订。

### 4.3 建图动机判别、三层升格准则与 `user.ask` 追问规范（`mathscape-mcp/20`）

在动手调 `propose_changes` 之前，先按下面三层判别树确定**为什么建、建在哪一层**，杜绝“为单道练习硬造共享实体”与“跨章节瞎猜硬连关系”两类污染：

1. **第一层：留在来源层（`source_unit` + `source_relation`）**
   - 当只是登记单本书内的章节、定义、定理、证明、例题或单道常规习题，或者某道题只是直接引用/应用本书某个已明确的定理或方法单元（`cites` / `applies_method` / `solves` / `proves`），且没有跨书对照或通用题型复用需求时，**只建来源单元与来源关系**。
   - **禁止**为每一道普通练习单独造一个一次性的 `problem_schema` 或 `declaration` 充数。
2. **第二层：升格到语义层（`semantic_entity` + `alignment`）**
   - 满足以下明确建图动机之一时才升格（建前必须先 `workspace.search` 查已有 `theory` 与 `semantic_entity`，能复用绝不重造）：
     - **跨书 / 跨手稿对齐**：两份材料（或教材与手写演算）表达同一概念、定理或恒等式，需要对齐记号约定、下标起点或参数对应；
     - **可复用题型提炼（`problem_schema`）**：至少能覆盖两道及以上具体题目，或用户明确要求把一类反复出现的解题套路（如“有限数列母函数消元求组合和”“特征根法解常系数线性递推”）抽成方法骨架；
     - **形式化基线或精确契约锚定（`concept` / `declaration`）**：需要挂接 mathlib 声明、拆清易混量词与边界条件，或作为多条对齐线与挂账的公共锚点。
3. **第三层：挂账或停下追问（`debt` / `user.ask`）**
   - 原文省略了隐含前提、但数学指向清晰无歧义 → 正常建边并挂 `debt` 记录所缺假设；
   - 原文**指代不明或范围存疑** → **宁停勿猜**，先查上下文邻域，仍不能唯一确定时立即调用 `user.ask` 请用户确认。

**`propose_changes.rationale` 首句动机标签（必填）**：

每次提交提案时，`rationale` 第一句必须以下列标签之一开头，简明交代立意：
- `【来源锚定】`：登记原文片段、书内单元与书内结构关系；
- `【跨书对齐】`：连接不同材料/手稿中的同一数学对象并注明参数或记号映射；
- `【题型提炼】`：提炼跨题复用的 `problem_schema` 并将具体题目对齐上去；
- `【概念/定理升格】`：建立共享 `concept` / `declaration` / `theory` 或补充 mathlib 基线；
- `【纠错重连】`：退役错误书内关系、错误对齐、错题型或错账，并重连到正确目标；
- `【补账/销账】`：补充隐含假设 `debt` 或提交 `claimed_paid` 偿还证据。

**`user.ask` 四类高价值触发场景与非阻塞回合纪律**：

只在工具列表包含 `user.ask`、且属于下列用读工具无法自行消解的场景时调用（能靠 `workspace.search` / `graph.get` / `graph.neighborhood` / `material.read` / `material.locate` 查清的事实，绝不打扰用户）：

1. **指代或章节歧义**：如原文只写“由定理 4 / 前述方法”，而全书存在多个同名编号候选（如第一章母函数定理 vs 后面章节计数定理）；
2. **升格粒度选择**：用户说“整理这道题”，既可只连书内 `applies_method`，也可升格为独立 `problem_schema`——给出 2–3 个清晰互斥选项请用户拍板；
3. **超范围清理确认**：核查邻域时发现用户未点名的相邻旧对象或旧关系也连错了，拟顺手退役或改写前先请用户确认范围；
4. **缺页或手稿符号难辨**：`material.read` 配合 `pdf.page_image` / `ink.page_image` 仍无法看清关键上下标，或需要用户附上某份手稿演算快照（此时传 `ink_requested: true`）。

```json
{
  "question": "习题 8 提到的「母函数方法」当前可按两种粒度入图，你希望采用哪一种？",
  "options": [
    "仅建立书内应用关系（连向第一章已有的有限数列母函数）",
    "提炼为独立共享题型（problem_schema）再对齐本题"
  ],
  "ink_requested": false
}
```

- **非阻塞立即收尾（`next: "end_turn"`）**：`user.ask` 落库后立即返回 `{ "status": "ok", "question_id": "…", "state": "pending", "next": "end_turn" }`。收到后**立刻结束本轮回复**（简短提示用户在下方确认卡片中选择或补充说明）；同一轮内再次调用不同内容的 `user.ask` 或继续调用 `propose_changes` 都会被服务端以 `forbidden`（`awaiting_user`）拒绝。
- **读取下一轮回答**：用户在沙箱卡片点击发送（或直接在输入框回复）后，下一轮 `<mathscape_turn_context>` 会附带 `mathscape.user_answer` JSON（含 `question_id`、`selected_option`、`answer_text`、`ink_snapshot_ref`）；若附了 `ink_snapshot_ref`，可用 `ink.page_image { node_id: "<ink_snapshot_ref>", page_index: 0 }` 读取手稿页图。

如果某阶段所需能力不可用，输出应包含：

```text
stage: S2
status: blocked
reason: capability_unavailable
deferred_capability: emir.upsert_proof_context
staged_artifact: mathscape:staged-artifact/…（如已暂存）
```

“不可用”表示本轮没有这项能力，不表示工作区永远没有该对象。

## 5. 写入、证据和信任边界

所有 Agent 写入都走下面的生命周期：

```text
Agent 提案
  → 服务端校验
  → staged 或 blocked
  → 用户在 Desktop 显式 accept 或 discard
  → 正式管线提交或保留审计记录
```

Agent 不得：

- 直接写正式图或调用 Desktop commit；
- 自己填写 accepted、kernel_verified、canonical 或 accepted_by；
- 自己退役或物理删除历史；
- 把 `proves` 当成内核证明；
- 把 Lean check 通过当成来源翻译忠实；
- 把没有 locator 的摘要当作证据。

每条核心 D/T 边一行 evidence。证据可以是外部定理、精确来源定位、手稿选区或“非外部·自包含”的逐步证明。`partial·待核` 必须保留，不得自动改成 complete。

## 6. 停手和交接

完成当前阶段或遇到阻塞时：

1. 更新 `worknet_run` 的 `current_stage`、`checks`、`open_debts` 和 `blocked_reasons`；
2. 读取 `get_change_set`（如果运行时有它），确认规范化提案、staged ids、Debt 和 diagnostics；交接里的数量按 `staged_ids` 计（见 §4.1「汇报数量」）；
3. 如果没有 `get_change_set`，读取 `artifact.stage` 的本地记录并明确说明它只是文件暂存；
4. 列出 partial、待核、失败见证、未决路线和需要用户决定的问题；
5. 把草案交给用户 accept/discard，不自行作最终决定。

下次继续时，先读 `worknet_run`，再读当前图和暂存记录。不要用新的对象或新路线覆盖旧版本；修订应产生新的 revision 或 route。

## 7. 常见错误

- 没读原文就先画网；
- 把每个来源短语都做成对象卡；
- 把 guard、Debt 和 lemma 写成同一种东西；
- 用 D 跨表示；
- 用 X 藏住关键保持引理；
- 让 Debt 跨分支悄悄泄漏；
- 用一条 evidence 覆盖整段证明；
- 只凭名字或 embedding 合并不同来源；
- 看到 `T04`、`T08` 却没有读取案例文件；
- 看到文档写了假定工具，就把它当成已实现 API；
- 把 staged 写成 accepted，把检查结果写成认证结果。

## 8. 参考资料的读取规则

规范和案例的逻辑编号、路径和版本见 `references/reference-index.md`。小黄的完整实践报告副本是 `references/rdtx_try.md`；它可以帮助理解 T04、T08、T12、T18 的具体例子，但案例正文和论文链接不能替代逐边证据。

`reference-index.md` 中的 workspace 路径不是隐含的全局文件系统访问。宿主必须显式提供 `workspace_root`；Agent 只能在该根目录下读取索引列出的 `docs/` 或 `docs/proposals/` 路径。`docs/` 中提案目录之外的文档是当前工程契约和实现事实的候选来源，`docs/proposals/` 是设计背景，不能覆盖运行时工具能力。根目录未提供、文件不存在或版本无法核对时，记录 `reference_unavailable`，不要凭名称或旧记忆补内容。

当前 skill 的详细创建规则见 `references/creation-rules.md`，I1–I7 检查动作见 `references/checks-i1-i7.md`。它们是工作规则，不表示语义工具已经实现。
