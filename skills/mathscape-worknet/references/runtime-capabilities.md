# Mathscape 运行能力表

skill 不能假设文档里写出的工具已经存在。每次启动一个工作 round，都先查看本轮实际挂载的 mathscape 工具（MCP 协议层的 `tools/list` 由 harness 调用，不是 Agent 可调用的工具），再把能力分成下面三类。

## 已实现的只读能力

通常包括：

- `workspace.search`（支持 `note`、`source_unit`、`theory`、`semantic_entity`、`representation_core`、`material`、`snapshot`，以及 `scope: "investigation"` 检索本会话暂存）
- `graph.get`（支持来源层、语义层与表示层节点/关系，语义与表示对象在 `semantic` 字段返回结构化内容）
- `graph.neighborhood`（`edge_types` 除结构边和书内关系外，支持按需遍历 `aligns`、`binds_representation`、`in_theory`、`debt_on`、`scope_of`、`uses_entity`、`of_concept`、`paid_by`）
- `material.read`
- `material.grep`
- `material.locate`（把一段原文换成快照里的精确 `md_byte_range`，供 `propose_changes` 的 source_span 使用）
- `pdf.outline`
- `ink.page_image`
- `pdf.page_image`
- `mathlib.search`
- `lean.check`

这些工具用于找现状、读原文、看手稿、查数学库和取得 Lean 回执。它们不能直接改变正式地图。

## 已实现但受限的文件暂存

`artifact.stage` 只接受受控文本内容，要求非空 round token。它是文件暂存入口，不是正式图写入口。它可以保存：

- 对象卡草案；
- 方言和表示草案；
- proof route 草案；
- I1–I7 检查报告；
- Markdown 或 Lean 文件。

它不会创建正式 Artifact、source_unit、语义节点或地图关系。暂存记录的状态仍然是 staged/blocked，不能被 Agent 自行接受。

## 已实现的轮次绑定提案与追问能力

在绑定了 `--round` 的会话（如桌面端沙箱对话）中包括：

- `propose_changes`（校验并写入本调查的 staged/blocked 提案，不直接改正式地图）
- `get_change_set`（按 id 读取本调查提案内容、诊断与 staged_ids）
- `list_change_sets`（列出本调查提案及可用于修订的 supersedes 对象）
- `user.ask`（非阻塞写入待用户确认问题卡片并返回 `next: "end_turn"`；同轮发出后不可再调 `propose_changes`）

## 仅由桌面端审核执行、不在 Agent 工具表中的能力

- `plan_change_set_accept`
- `accept_change_set`
- `discard_change_set`

如果本轮工具列表中没有某项写或追问工具，Agent 不得假装调用成功。可以把草案放进 `artifact.stage`（若已挂载）或 scratch，并在输出中写 `deferred_capability`。

## 语义层假定工具

自 `mathscape-mcp/17` 起，共享对象、对齐线、一笔账和最小约定卡已经由 `propose_changes` 直接支持；自 `mathscape-mcp/21` 起，多分支/守卫/排除（`guards`/`branches`/`exclusions`）、局部符号表（`local_symbols`）、表示核（`representation_core`）与表示绑定（`representation_binding`）也已由 `propose_changes` 原生支持，不需要单独假定工具。

下面这些名字只是设计接口，不是当前 API：

- `emir.upsert_proof_context`
- `emir.upsert_proof_state`
- `rdtx.upsert_node`
- `rdtx.upsert_edge`
- `rdtx.upsert_debt`
- `rdtx.upsert_guard`
- `rdtx.upsert_lemma`
- `rdtx.attach_edge_evidence`
- `rdtx.record_failure`
- `emir.upsert_trust_record`

它们未来只能作为 `propose_changes` 的薄封装，不能形成绕过用户接受的第二条写路径。

## 能力状态的写法

对每个本轮需要的工具写一行：

```text
workspace.search        implemented
propose_changes         implemented_staged_only
rdtx.upsert_edge        unavailable
artifact.stage          implemented_round_bound
```

“unavailable” 代表本轮不能继续依赖它；不代表工作区永远没有该对象。

## Mathscape 沙箱运行时（当前版本）

在 Mathscape 桌面端的沙箱对话中，本轮的实际情况是：

- **挂载的 mathscape 工具**：`workspace.search`、`material.read`、`material.grep`、`pdf.outline`、`graph.get`、`graph.neighborhood`、`ink.page_image`、`pdf.page_image`、`mathlib.search`、`lean.check`、`propose_changes`、`get_change_set`、`material.locate`、`list_change_sets`、`user.ask`。提案工具只写当前沙箱副本的 staged/blocked 记录；接受和丢弃仍由 Desktop 审核命令完成。
- **向用户追问确认**：遇到章节指代歧义、升格粒度选择、超范围清理或手稿字迹不清时，调用非阻塞追问工具并在收到 end_turn 后立即结束本轮；用户的选项、补充说明与手稿快照引用会在下一轮上下文的回答块中返回（详见 SKILL.md §4.3）。
- **对话连续性**：同一沙箱会话的各轮延续同一条 Codex 线程，之前轮次的工具调用与结果都保留在上下文中。每个非首轮开头会有一段「自上轮以来工作区变化」摘要（用户可能在两轮之间接受、丢弃提案或改动地图）；与旧结果冲突时以摘要和本轮新调用的工具结果为准。若出现「上下文已降级」，说明线程无法恢复，此前只剩文字转录；出现「工具/skill 已更新」时，按提示重读变化的部分。
- **汇报提案数量**：以 `propose_changes` / `get_change_set` 返回的 `staged_ids` 为准（按 id 前缀分单元、关系、证据、共享对象、表示核、对齐线、表示绑定、账、约定卡计数），不要凭记忆估算（见 SKILL.md §4.1「汇报数量」）。
- **找回之前的提案**：旧提案的 id 和状态可能已变，修订旧提案前先调用只读的提案列表工具（见 SKILL.md §4.1「修订」），它只列本沙箱会话的提案，并给出可直接用于修订的对象。
- **未挂载**：`artifact.stage`。文件草稿仍写入 scratch；需要登记来源时使用 `propose_changes`，不能把文件暂存冒充正式地图写入。
- **harness 自带的文件与 shell 工具**：只能写会话 scratch 目录（路径见本轮 developer instructions）；工作区副本、本 skill 目录和 Lean profile 都是只读；网络关闭。scratch 中的文件是草稿，不是地图变更，也不会提升任何可信状态。
- **Lean 草稿**：写在 scratch 中，用 `lean.check` 检查，不要直接运行 `lake build` 或修改 Lean profile 的依赖。编译通过只说明 Lean 接受了这段代码，不说明它忠实于原文或证明完整。
- **本 skill**：只读，不要尝试修改；需要改动时在回复中提出。

