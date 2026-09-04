# Initial architecture

Mathscape 采用 Rust 单内核、本地优先的桌面架构：

```text
embedded frontend
       |
application commands and events
       |
Rust Domain Kernel ---- Rust orchestration
       |                       |
       +-- SQLite store        +-- LLM provider
       +-- Managed Pool        +-- Marker process
       +-- graph projections   +-- Lean/Lake process
```

## 边界原则

1. `mathscape-domain` 定义稳定身份、revision、实体、关系和 MapPatch 契约，不依赖 UI、数据库或外部工具。
2. 所有规范数据写入都必须经过 Kernel 校验和事务提交。
3. SQLite 是权威持久化存储；图结构是可重建、可缓存的类型化投影。
4. Marker、Lean 和模型供应商通过适配器接入，不能持有领域写入权限。
5. Prompt 和响应 schema 是有版本的资源，不作为散落在业务代码中的字符串存在。
6. 桌面应用和 CLI 复用相同的 Rust crates。
7. SQLite 保存 Artifact 权威元数据，Managed Artifact Pool 保存不可变字节；普通导入完成后不依赖用户选择的原始路径。
8. Workspace 内的 Managed URI 相对于显式 Workspace 根目录解析，不能依赖进程当前目录或保存绝对路径，因此 Workspace 关闭后可整体搬迁。

Artifact Pool 的聚合、目录布局、Managed/External 策略和自包含性契约见
[`artifact-pool-boundary.md`](artifact-pool-boundary.md)。External 只作为未来显式启用的扩展模式，首版业务入口只接受 Managed 导入。

## 初始里程碑

导入 `epsilon-n-cesaro-v1` fixture，保存稳定 ID、来源定位和关系语义，通过统一校验，并能查询来源链与 R/D/T/X 邻域。
