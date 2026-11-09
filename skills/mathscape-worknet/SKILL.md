---
name: mathscape-worknet
description: 在 Mathscape 工作区中帮助用户搭建可回溯的数学地图（来源锚定、对象卡、表示、证明上下文、失败见证）。用户在 Mathscape 沙箱或调查中请求阅读材料、整理对象或构建工作网时使用。
metadata:
  version: "0.6.2"
  tool_schema_version: "mathscape-mcp/13"
---

# Mathscape 数学地图构建 skill v0.6.2

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
6. 如果用户目标、来源范围、符号含义或分支无法确定，使用 `user.ask`；没有该工具时把问题写进 `blocked_reasons`，不要猜。

“先声明目标”指先写一个暂时的目标和范围，不表示还没读原文就可以编造完整对象卡。对象卡应在 S1 读取来源后形成。演示时 S1 和 S2 可以放进同一个提案，但不能省略其中任何一个检查。

## 2. 什么时候可以读到什么

默认的读法是：

```text
workspace.search
  → graph.get
  → graph.neighborhood（只在需要时扩大）
  → material.read / material.grep
  → ink.page_image 或 pdf.page_image（需要看版式或手稿时）
```

命中只告诉你“有这个对象”，正文必须通过 `content_ref` 回链读取。没有可靠 locator 时不能猜字节范围，也不能把模型摘要写进图节点来代替正文。

先读窄的结构邻域；需要证明链时再显式加入 `proves`、`states` 等语义边。不要一开始就打开所有边和最大深度。

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

已实现的读工具通常包括：`workspace.search`、`graph.get`、`graph.neighborhood`、`material.read`、`material.grep`、`pdf.outline`、`ink.page_image`、`pdf.page_image`、`mathlib.search`、`lean.check`。

`artifact.stage` 是受 round 约束的文件暂存能力。它可以保存对象卡、工作网草案、Lean 文件和检查报告，但不会登记正式 Artifact、地图节点或关系。

`propose_changes`、`get_change_set` 和 `user.ask` 只有在本轮工具列表中实际出现时才能调用。语义层的 `emir.upsert_*`、`rdtx.upsert_*`、`rdtx.record_failure` 等目前属于契约草案，必须作为 `propose_changes` 的薄封装，不能另开一条绕过用户接受的写路径。

如果某阶段所需能力不可用，输出应包含：

```text
stage: S2
status: blocked
reason: capability_unavailable
deferred_capability: emir.upsert_dialect_scope
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
2. 读取 `get_change_set`（如果运行时有它），确认规范化提案、staged ids、Debt 和 diagnostics；
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

`reference-index.md` 中的 workspace 路径不是隐含的全局文件系统访问。宿主必须显式提供 `workspace_root`；Agent 只能在该根目录下读取索引列出的 `docs/` 或 `proposals/` 路径。`docs/` 是当前工程契约和实现事实的候选来源，`proposals/` 是设计背景，不能覆盖运行时工具能力。根目录未提供、文件不存在或版本无法核对时，记录 `reference_unavailable`，不要凭名称或旧记忆补内容。

当前 skill 的详细创建规则见 `references/creation-rules.md`，I1–I7 检查动作见 `references/checks-i1-i7.md`。它们是工作规则，不表示语义工具已经实现。
