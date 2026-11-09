# Mathscape 运行能力表

skill 不能假设文档里写出的工具已经存在。每次启动一个工作 round，都先查看本轮实际挂载的 mathscape 工具（MCP 协议层的 `tools/list` 由 harness 调用，不是 Agent 可调用的工具），再把能力分成下面三类。

## 已实现的只读能力

通常包括：

- `workspace.search`
- `graph.get`
- `graph.neighborhood`
- `material.read`
- `material.grep`
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

## 只有契约、当前运行时可能没有的能力

- `propose_changes`
- `get_change_set`
- `user.ask`
- `plan_change_set_accept`
- `accept_change_set`
- `discard_change_set`

如果本轮工具列表中没有它们，Agent 不得假装调用成功。可以把草案放进 `artifact.stage`，并在输出中写 `deferred_capability`。

## 语义层假定工具

下面这些名字只是设计接口，不是当前 API：

- `emir.upsert_dialect_scope`
- `emir.upsert_representation_binding`
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
propose_changes         contract_only
rdtx.upsert_edge        unavailable
artifact.stage          implemented_round_bound
```

“unavailable” 代表本轮不能继续依赖它；不代表工作区永远没有该对象。

## Mathscape 沙箱运行时（当前版本）

在 Mathscape 桌面端的沙箱对话中，本轮的实际情况是：

- **挂载的 mathscape 工具**：`workspace.search`、`material.read`、`material.grep`、`pdf.outline`、`graph.get`、`graph.neighborhood`、`ink.page_image`、`pdf.page_image`、`mathlib.search`、`lean.check`、`propose_changes`、`get_change_set`。提案工具只写当前沙箱副本的 staged/blocked 记录；接受和丢弃仍由 Desktop 审核命令完成。
- **未挂载**：`artifact.stage`。文件草稿仍写入 scratch；需要登记来源时使用 `propose_changes`，不能把文件暂存冒充正式地图写入。
- **harness 自带的文件与 shell 工具**：只能写会话 scratch 目录（路径见本轮 developer instructions）；工作区副本、本 skill 目录和 Lean profile 都是只读；网络关闭。scratch 中的文件是草稿，不是地图变更，也不会提升任何可信状态。
- **Lean 草稿**：写在 scratch 中，用 `lean.check` 检查，不要直接运行 `lake build` 或修改 Lean profile 的依赖。编译通过只说明 Lean 接受了这段代码，不说明它忠实于原文或证明完整。
- **本 skill**：只读，不要尝试修改；需要改动时在回复中提出。

