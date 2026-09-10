# Prompts

每项 LLM 工作流应使用独立、可版本化的目录，并至少包含 prompt、输出 schema 与 manifest。Prompt 只生成候选结果，不能直接写入权威地图。

已实现：[Source Discovery v3](source-discovery/v3/README.md)，包含 Schema、Prompt、manifest、
orchestrator 请求构建接口和单次模型 Producer；结果持久化与接受尚待接入。

分阶段发现 M1 已实现 [Unit Discovery v1](unit-discovery/v1/README.md)；
[Source Facts v1](source-facts/v1/README.md) 已提供下一阶段使用的 DTO、Prompt、Schema 与
preflight，完整事实发现编排属于 M2。Desktop 当前仍使用 Source Discovery v3。
