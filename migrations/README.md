# SQLite migrations

这里将存放顺序化、不可修改的 SQLite schema migrations。数据库 schema 尚未在脚手架阶段冻结。

首个 Artifact Pool migration 必须遵循
[`../docs/artifact-pool-boundary.md`](../docs/artifact-pool-boundary.md) 中关于
不可变内容核心、metadata revisions、locations、checks 与 Workspace 可搬迁性的契约。
