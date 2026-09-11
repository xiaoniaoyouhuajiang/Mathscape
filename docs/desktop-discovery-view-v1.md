# Desktop Discovery View v1

状态：已实现。线协议版本为 `mathscape-desktop-discovery-view-1`。

该 DTO 是已验证分阶段发现结果的只读展示投影，不是 Domain 实体、LLM 输出契约或入图请求。
它只能从当前 `DiscoveryWorkflow` 的可信内存对象构造，不提供 Deserialize。

## 顶层结构

- `status` 与 `complete`：区分运行终态和内容是否完整。
- `source`：Artifact、Conversion、内容哈希和 DiscoveryInput 指纹。
- `units`：单元类型、显示标题、父链、完整性、事实阶段状态、标签、原文标题和来源锚点。
- `references`：原文提及、owner、解析状态、候选、保守解析原因和 origins。
- `relations`：来源/目标、关系类型、证据、原文依据和 origins。
- `issues`：A/B 阶段、owner、问题类型、上下文请求、是否要求修改 UnitSet 及证据。
- `pending_tasks`、`diagnostics`、`attempts`：未完成任务、确定性检查及精简调用审计。

每个 `DesktopSourceAnchorView` 保存绝对 UTF-8 `[start,end)`、相关物理 Markdown block ID 和有界
摘要。字节范围以 `source.content_hash` 对应的 Artifact 为解释上下文；前端必须在当前 Markdown
快照上检查边界，不能依据渲染 DOM 文本猜测位置。

DTO 不包含系统 Prompt、完整授权原文、API 凭据或原始模型响应。流式原文只走任务 Channel；
累计 token 与耗时另在 Desktop 运行摘要中返回。结果仍未被审核、接受或登记到来源图。

## 阅读器行为

单元、标签、引用、关系证据和 issue 证据均可触发定位。阅读器先突出所有相交的来源块并滚动
到首块，再从原始 Markdown 字节构建精确高亮片段。这样即使公式已经由 KaTeX 展开，来源回链
仍以 Marker 产出的 Markdown 为准。
