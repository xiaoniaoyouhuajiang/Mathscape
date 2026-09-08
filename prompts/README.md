# Prompts

每项 LLM 工作流应使用独立、可版本化的目录，并至少包含 prompt、输出 schema 与 manifest。Prompt 只生成候选结果，不能直接写入权威地图。

已实现：[Source Discovery v1](source-discovery/v1/README.md)，包含 Schema、Prompt、manifest
及 orchestrator 离线请求构建接口；实际模型 Producer 尚待接入。
