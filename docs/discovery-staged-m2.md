# 分阶段发现 M2 实现

状态：已实现两阶段离线闭环。Desktop 接入为 M3，耐久化恢复为 M4。M2 不写入来源图。

## 运行入口

```rust,ignore
let prepared = build_unit_discovery_request(&document, &selection, input_limits)?;
let mut workflow = DiscoveryWorkflow::new(&backend, DiscoveryWorkflowLimits::default());
let status = workflow.run(prepared).await;
let snapshot = workflow.snapshot();
```

阶段 A 验证成功后直接进入 B，不等待人工确认。也可以独立调用 `plan_source_facts`、
`SourceFactProducer`、`PreparedFactRequest::validate` 和 `assemble_discovery` 做调试。

离线示例使用仓库内自编 Markdown 和已记录的模型提案，通过完整工作流回放，输出 JSON 快照；
它不调用外部模型，也不会创建文件：

```sh
cargo run -p mathscape-orchestrator --example discovery_workflow_demo --quiet
```

## 完整原文与提取权限

每个阶段 B 请求都包含完整授权选区，按源顺序发送一次。leading_gap 和 following_gap 保留
原文间隙，boundary 保留文件边界。增加这两个输入字段后，B 输入版本为
`mathscape-source-facts-input-2`；B 输出仍为 `mathscape-source-facts-1`，Prompt revision 为 2。

任务归属独立于阅读范围。严格物理嵌套并且有 parent 祖先链时，后代拥有其直接内容，父单元
只在剩余内容提取；其余原文是可阅读上下文。父子范围物理分离时不扣除。阶段 A 已拒绝兄弟
交叠、交叉区间和同范围多角色，因此 Planner 不自行猜最小 owner。

在单元边界落于原 Markdown 块内部时，Planner 将块分成可见片段，按顺序赋予新的 bN。
片段保存原 Artifact 的 UTF-8 字节区间；不创建新的 Artifact，不改变原文。B 的目录选择器、
任务范围和引用计数均使用这一套片段绑定。高亮回链依据 Artifact 快照与字节范围，不能把
片段 ID 当成原阅读器的物理块 ID。

默认每批最多 32 个任务，可配置；批次划分目前只限制任务数，所有批次均携带完整原文和
冻结单元目录。A 的输入大小限制仍有效。暂不进行原文摘要、按估算 token 截断上下文或远端
文献读取。输入外引用保留 unresolved，补上下文建议保留为 issue，自动扩展阅读尚未实现。

## 验证、组合和引用状态

B 先检查 envelope 的 request_ref、任务唯一性及任务成员。错误 envelope 整批拒绝；有效
envelope 下按任务独立验证，失败任务不会使已验证的兄弟任务丢失。验证包括事实 ID、任务
处置、隐含 source、目标与自指、精确定位、关系端点类型及 owner 证据范围。

候选提及经 TextOccurrence 定位后保存原文精确文本与字节范围，同时保留原提案和规范化
标记。跨块范围标记 precision_review，程序不会把边界可定位宣称成语义边界已经人工确认。
待修改 UnitSet 的 issue 会使工作流进入 needs_attention，B 不能修改 A。

P2 在全部已返回的有效批次上收集标签，再查询本地目标，避免批次顺序影响结果。首版支持
简单的数字/字母/点号标签、括号式标签，以及“定义/定理/命题/式”的单目标提及。只有显式
章节作用域相同、类型相容、唯一声明且没有冲突 hint 时才形成 resolved_local。仅有模型 hint
或作用域不明确时为 candidate；多候选为 ambiguous；外部、同单元内部或不支持的组合表达
保持 unresolved。这里没有把数学表述相同当作全局对齐。

同一 owner、同一来源位置的重复引用聚合 origins；不同出现位置分别保留。相同来源关系
三元组聚合证据与 origins，已解析 reference 派生的 cites 与已有关系合并。禁止 unit 自边。
ValidatedFactTask 和 DiscoveryResult 不提供 Deserialize 或外部可写字段，快照不能直接提升
为已验证状态。M1 的 StagedDiscoveryResult 仅为未验证回放 envelope；实际规范化结果类型是
orchestrator 中的 DiscoveryResult。

## 修订、暂停与内存续跑

单次请求无隐式网络重试。A 最多一次 replace_units 修订；B 每个原始批次最多一次修订，
支持选择 cN 或替换失败任务。候选绑定原提案指纹、字段路径和对应任务定位器；选择后再次
验证。合法兄弟任务不会重新生成；不允许同时替换一个任务并选择它的候选，也不能用 B 的
repair 修改 UnitSet。defer 保留未完成状态。

`WorkflowAttempt` 保存阶段、批次、修订标记、请求、输出文本、模型运行摘要和调用结果。
事实验证诊断单独保存，因此某次请求返回合法 JSON，不等于对应事实通过了全部验证。
`snapshot()` 导出原始 DiscoveryInput、UnitSet 检查点、有效事实、pending tasks、attempts
和诊断；推理摘要不在此快照中。快照当前在内存中，退出应用后不自动恢复。

对同一个工作流对象再次显式调用 `run`，会复用冻结 A 与已 processed 的 B 任务，只请求
未处理或失败任务；累计调用数和 token 记录保留，每次显式续跑重新开始一个超时时段。
不同来源/选区不能混入同一工作流；UnitSet 需要修改时建立新工作流，不复用旧 B 结果。

默认总期限 600 秒、最多 32 次请求、最多 16 个 B 批次。每次请求受剩余总期限约束，批次
之间也检查期限。可选 total token 预算是依据已知 usage 的后续调用门槛，不能预知单次最终
消耗，因此不是远端逐 token 硬限额；usage 缺失时停止预算受限的后续调用，不把未知视作零。
默认不启用此 token 门槛。配额或期限耗尽会保留检查点和 pending tasks。

取消方式：调用者丢弃正在等待的 `run` future 以取消在途请求，再调用 `cancel()` 标记快照；
无需生成后台线程。已冻结的 A 和已成功的 B 任务留在工作流对象内。M3 应把这一顺序接入
Desktop 的任务取消与活动锁管理。

GenaiBackend 已检查真正的 ChatStreamEvent::End：EOF 没有终止事件时返回 Transport 错误，
即使累积正文恰好是合法 JSON 也不能继续验证。finish_reason 缺失本身不等于断流。

## 验证

自编 fixture 覆盖完整流程、阶段 A 修订、候选消歧、整任务修订、失败后不循环、按批次续跑、
未处理任务续跑、完整原文不丢失、块内边界归属、外部引用、跨批标签查找、失效 UnitSet、
预算、期限、取消和需修改单元结构。传输测试用本地 HTTP 模拟端点验证无终止事件的 EOF。

```sh
cargo test -p mathscape-orchestrator --test discovery_workflow --test staged_discovery
cargo test -p mathscape-llm --test genai_adapter
```

Schema 快照、真实模型效果和运行可信性分别验证。上述 fixture 证明工程流程可执行，不代表
已经验证任意教材上 LLM 的单元识别率、引用完整率或实际 token 成本。
