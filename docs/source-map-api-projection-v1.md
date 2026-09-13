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
放入 `source-map` 分组。来源范围显示为边界节点，便于后续点击回到 Markdown。

Desktop 快照新增 `source_graph` 字段。它与 Artifact/Conversion 图分开，客户端优先展示正式
来源图；没有来源对象时继续展示原有 Artifact 图。两者的合并展示和节点点击回溯属于后续 UI
迭代，不能把来源对象重新解释成 PDF Artifact。

读取过程是只读和可重建的；注册回执仍是正式提交的幂等凭据。任何来源对象缺少当前关系端点、
引用未知对象或使用不支持的投影视图，投影器都会返回错误而不是生成不完整图。
