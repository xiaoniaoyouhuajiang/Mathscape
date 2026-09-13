# Source Review Projection v1

`SourceReviewProjection` 是来源发现审核器的只读输入契约。它位于 Domain 层，因此 Desktop、
未来的 Web 客户端或批处理审核器可以共享同一结构。它不包含原始 LLM 请求和响应，也不授予
写入地图的权限；接受操作必须回到后端重新检查并生成 `SourceRegistrationPlan`。

## 交互所需的数据

`SourceReviewUnit.anchors`、标签、标题、引用和关系 evidence 都使用同一个
`SourceReviewAnchor`：用途、UTF-8 半开字节范围、Markdown block ID 和短摘录。前端可以因此实现：

```text
unit hover       -> anchors 高亮
anchor hover     -> 反向找到 unit/reference/relation
unit click       -> 保持 selected 状态并滚动到首个 anchor
relation click   -> 同时高亮 source、target 和 evidence
```

悬停是临时高亮，点击是持久选择；这两种状态由前端维护，DTO 只提供稳定对象引用和锚点。

## 状态边界

- `fact_status` 表示来源事实是否已经处理，不表示数学正确；
- `acceptance` 表示当前审核选择状态；接受命令成功后，Desktop 会把正式来源注册结果以
  `registered` 状态回显；
- `ReferenceStatus` 区分 unresolved、candidate、resolved_local 和 ambiguous；
- `diagnostics` 和 `pending_tasks` 必须在审核页可见；
- `complete` 只表示内容发现工作流完成，不表示来源节点已登记。

`SourceReviewAcceptance` 描述当前可接受范围、缺失 parent、缺失 relation target 和阻塞原因。
前端据此禁用不闭合的接受操作，或者提示用户扩大选择范围。

投影携带 `run_id` 和 `result_fingerprint`。审核器提交显式单元集合时必须原样带回二者；后端不
接收前端回传的单元内容，而是从运行检查点重新绑定受管 Markdown 并重跑确定性验证。父级和
内容关系目标由前端即时闭合以改善交互，后端注册计划器仍会独立检查同一约束。

`structure_relations` 只表达 parent/child 层级；`relations` 表达 `proves`、`solves`、
`explains` 等原文内容关系。两者分开，树视图不必把内容关系误画成目录层级。

## 非目标

该投影不承载 `DialectScope`、`SemanticEntity`、`Alignment`、R/D/T/X、Lean 证据或
`TrustRecord`。这些信息属于后续语义审核视图，应通过独立投影关联到来源单元。
