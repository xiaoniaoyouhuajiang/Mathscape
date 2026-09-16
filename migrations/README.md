# SQLite migrations

顺序追加的数据库升级脚本；已应用的 migration 不得原地修改。实际升级入口和支持版本见 [store](../crates/mathscape-store-sqlite/src/lib.rs)，维护约束见 [开发指南](../docs/development.md)，存储边界见 [存储与集成](../docs/storage-and-integrations.md)。
