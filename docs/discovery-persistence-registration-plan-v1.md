# Discovery 持久化与来源登记计划 v1

状态：已实现核心契约。Desktop 已在每次内容发现结束时写入运行检查点；接受决策和
`SourceRegistrationPlan` 已实现为后端能力，审核 UI 与正式来源图提交尚未实现。

## 1. 两类数据边界

`discovery_runs` 及其子表保存可重放的工作流事实：来源身份、模型请求和原始响应、阶段状态、
冻结 UnitSet、已验证 FactTask、验证报告和 pending 任务。读回这些 JSON 不恢复
`ValidatedUnitSet`/`ValidatedFactTask` 的权限；恢复或登记仍须重新绑定原文并验证。

无效模型输出只存在于 attempt 的 `raw_text` 和诊断中。只有验证器构造的单元和事实进入
`discovery_units` 与 `discovery_fact_tasks`，但它们仍不是正式地图节点。

`discovery_acceptance_decisions` 保存不可变接受决定及其来源登记计划。接受表示用户或既定策略
授权该结果进入后续登记，不表示人工证明、Lean 检查或全局数学对齐成立。

## 2. 原子检查点

同一个 run ID 绑定不可变的 Artifact、pipeline、DiscoveryInput 指纹和选区指纹。后续保存可
替换该 run 的当前检查点，但不能把 run ID 重新绑定到另一输入。一次保存原子替换 attempts、
reports、units、facts 和 pending；失败不会留下相互错位的半套数据。

Desktop 返回的 `run.run_id` 可用于查找检查点。completed、partial、needs_attention、failed、
cancelled 均可保存；当前实现是在工作流退出时保存，不承诺进程崩溃前的实时中间恢复。

## 3. 接受与 SourceRegistrationPlan

接受决定绑定 `expected_result_fingerprint`，从而拒绝针对旧结果作出的决定。选择可以是全部已
验证单元，或显式单元集合。显式集合必须：

- 非空且只含当前 UnitSet 的单元；
- 包含每个已选单元的 parent；
- 不让已选 source 的关系指向未选 target。

partial 结果可以接受。计划保存接受范围内的 pending 任务，并把相应单元标为
`facts_pending`，不会把“尚未发现事实”解释为“没有事实”。

计划为 Markdown 范围、来源单元和来源关系生成确定性候选 ID，保存 temp ref 到候选 ID 的
映射、标签、原文标题、引用解析状态和关系证据。候选 ID 只供后续登记查询与幂等计划使用；
正式提交仍须查询当前来源图、处理复用/冲突并构造 Domain 实体及 TrustRecord。

## 4. 尚未包含

- 审核 UI、接受/拒绝操作与提案编辑；
- 从磁盘检查点恢复并继续 LLM 工作流；
- `SourceSpan`/`SourceSemanticUnit`/`SourceUnitRelation` 的 SQLite 提交与读取；
- 来源图 projector 和数学语义解析。

这些能力必须建立在本契约之上，不能从 attempt 原始 JSON 绕过接受和登记计划。
