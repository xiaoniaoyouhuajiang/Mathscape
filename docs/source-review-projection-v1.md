# Source Review Projection v1

`SourceReviewProjection` 是来源发现审核器的只读输入契约。它位于 Domain 层，因此 Desktop、
未来的 Web 客户端或批处理审核器可以共享同一结构。它不包含原始 LLM 请求和响应，也不授予
写入地图的权限；接受操作必须回到后端重新检查并生成 `SourceRegistrationPlan`。

## 交互所需的数据

`SourceReviewUnit.anchors`、标签、标题、引用和关系 evidence 都使用同一个
`SourceReviewAnchor`：UTF-8 半开字节范围、Markdown block ID 和短摘录。前端可以因此实现：

```text
unit hover       -> anchors 高亮
anchor hover     -> 反向找到 unit/reference/relation
unit click       -> 保持 selected 状态并滚动到首个 anchor
relation click   -> 同时高亮 source、target 和 evidence
```

悬停是临时高亮，点击是持久选择；这两种状态由前端维护，DTO 只提供稳定对象引用和锚点。

## 状态边界

- `fact_status` 表示来源事实是否已经处理，不表示数学正确；
- `acceptance` 表示当前审核选择状态，不表示已写入数据库；
- `ReferenceStatus` 区分 unresolved、candidate、resolved_local 和 ambiguous；
- `diagnostics` 和 `pending_tasks` 必须在审核页可见；
- `complete` 只表示内容发现工作流完成，不表示来源节点已登记。

`SourceReviewAcceptance` 描述当前可接受范围、缺失 parent、缺失 relation target 和阻塞原因。
前端据此禁用不闭合的接受操作，或者提示用户扩大选择范围。

## 非目标

该投影不承载 `DialectScope`、`SemanticEntity`、`Alignment`、R/D/T/X、Lean 证据或
`TrustRecord`。这些信息属于后续语义审核视图，应通过独立投影关联到来源单元。
