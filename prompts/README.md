# 版本化模型任务

Desktop 默认使用 [Unit Discovery](unit-discovery/v1/README.md)、[Source Facts](source-facts/v1/README.md) 和 [Discovery Repair](discovery-repair/v1/README.md)。[Markdown Patch](markdown-patch/v1/README.md) 是可独立触发的文本校准阶段，不直接替代 A/B。工作流、验证和登记边界见 [架构](../docs/architecture.md) 与 [数据规则](../docs/data-model.md)。

[Source Discovery v3](source-discovery/v3/README.md) 是保留的单阶段回放与比较路径；v1/v2 是历史契约。各包的 system、manifest、Schema 保持独立版本，不因文档清理删除。Schema 生成与回归入口见 [开发指南](../docs/development.md)。
