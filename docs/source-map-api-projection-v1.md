# 正式来源对象 API 与投影 v1

来源注册完成后，正式 `SourceSpan`、`SourceSemanticUnit` 和 `SourceUnitRelation` 保存在
`source_map_v1` 表中。SQLite 层只返回当前 revision 的稳定对象和关系端点，不把表结构泄露给
Desktop。

`SqliteArtifactStore::source_map_snapshot()` 返回 `SourceMapSnapshot`，其中：

- `objects` 是正式来源对象及其当前 revision、类型、生命周期和 payload；
- `relation_endpoints` 是关系 ID 到角色化端点的映射；
- 关系端点只读取当前 relation revision，避免旧 revision 被误投影。

Query 层的 `SourceGraphProjector` 将快照映射为 `GraphProjection`：来源单元和来源范围成为
来源层节点，`SourceUnitRelation` 成为带 `source`、`target`、`evidence` 角色的超边，所有节点
放入 `source-map` 分组。`source_spans` 字段确定性派生“来源单元包含原文范围”结构边；来源
范围显示为边界节点，便于后续点击回到 Markdown。

`SqliteArtifactStore::landscape_snapshot()` 在同一个 SQLite 读事务中获取 Artifact Catalog 和
Source Map。`LandscapeProjectionInput` 只组合这两个权威快照；`LandscapeProjector` 复用材料图
和来源图，并根据 `SourceSpan.markdown.artifact` 派生“来源于 Markdown”追溯边。因此景观中存在：

```text
SourceSemanticUnit → SourceSpan → Markdown ← ConversionRun ← PDF
```

Desktop 快照同时提供 `graph`、`source_graph` 和 `landscape_graph`。用户可在材料图、来源图和
景观图之间切换；材料图是首次默认值，之后的选择被本地保存。来源注册完成后 Desktop 重新读取
快照，使新节点和追溯链可见。

读取过程是只读和可重建的；注册回执仍是正式提交的幂等凭据。任何来源对象缺少当前关系端点、
引用未知对象或使用不支持的投影视图，投影器都会返回错误而不是生成不完整图。
