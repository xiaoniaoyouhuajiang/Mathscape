# SQLite migrations

这里存放顺序化、不可修改的 SQLite schema migrations。Artifact Pool v1
从 `0001_artifact_pool.sql` 开始，LLM profile 从 `0002_llm_profiles.sql` 开始；
已经应用的 migration 不得原地修改。

首个 Artifact Pool migration 必须遵循
[`../docs/artifact-pool-boundary.md`](../docs/artifact-pool-boundary.md) 中关于
不可变内容核心、metadata revisions、locations、checks 与 Workspace 可搬迁性的契约。
