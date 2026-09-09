# Prompts

每项 LLM 工作流应使用独立、可版本化的目录，并至少包含 prompt、输出 schema 与 manifest。Prompt 只生成候选结果，不能直接写入权威地图。

已实现：[Source Discovery v1](source-discovery/v1/README.md)，包含 Schema、Prompt、manifest、
orchestrator 请求构建接口和单次模型 Producer；结果持久化与接受尚待接入。
