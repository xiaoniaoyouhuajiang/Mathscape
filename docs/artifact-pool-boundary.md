# Artifact Pool boundary and persistence contract

本文档规定 Mathscape Artifact Pool 的长期边界，并记录首个 Managed 实现切片。当前实现状态仍以各 crate 文档和测试为准。

## 核心目标

Artifact Pool 为 PDF、Markdown、Marker metadata、提取图片及其他派生产物提供：

- 按内容寻址的不可变存储；
- 可审计的导入、校验与派生来源；
- 与物理路径解耦的稳定引用；
- 默认可搬迁、自包含的 Workspace；
- 面向共享语料和受管存储的显式 External 扩展能力。

SQLite 保存权威元数据，文件系统保存 Artifact 字节。大型二进制不写入 SQLite BLOB；sidecar JSON/YAML 只用于 fixture、导出或恢复，不作为运行时权威来源。

## 不可变性的边界

一个 Artifact 的内容身份对应一串确定字节。首版以带算法前缀的 SHA-256 表示：

```text
sha256:8daeba7deb1fa29fedbaf6c5ebdf0db2e1e6b99e7fb227d6941f489c70ae61ef
```

内容摘要与字节大小在同一 Artifact 的所有 metadata revision 中必须保持不变。任意字节变化都会产生新的 Artifact，并通过 `superseded_by`、`derived_from` 等关系连接旧对象；不得覆盖旧对象。

这一约束保证：

- `SourceSpan` 的页码、字节区间和区域坐标持续指向同一证据；
- `ConversionRun` 的输入输出可以复现；
- 相同内容天然去重；
- 后台读取、缓存和并发任务不受原地修改影响。

媒体类型、原始文件名、页数和版权判断不是内容身份。它们可以通过 `EntityHeader` 的 revision 机制纠正，但纠正不得改变内容摘要和字节大小。

## 聚合边界

职责分离不意味着为每个字段建立独立顶层实体。首个持久化版本保持三个主要 Artifact 聚合。

### SourceArtifact

调用者使用的主要目录对象，包含不可变内容身份和当前被接受的描述性 metadata：

```text
SourceArtifact
  stable_id
  content_hash          immutable invariant
  byte_size             immutable invariant
  media_type            revisioned description
  original_filename     revisioned description
  page_count            revisioned description
  rights                revisioned description
```

Rust API 可以将这些数据作为一个聚合返回；存储层可以将稳定内容核心与 metadata revisions 规范化，而不把内部表结构泄露给调用者。

### ArtifactLocation

描述当前可以从哪里读取内容。位置独立于 Artifact revision，因为移动、镜像或丢失文件不改变内容身份：

```text
ArtifactLocation
  artifact_id
  uri
  storage_kind
  role
  priority
  current_status        operational projection
```

### ArtifactCheck

追加式保存对某个位置的检查，而不是反复覆盖一个全局 `integrity` 字段：

```text
ArtifactCheck
  location_id
  checked_at
  observed_hash
  observed_size
  result
  checker_version
```

Artifact 的 availability 和 integrity 是位置与检查记录的派生状态，可以缓存，但缓存不是历史事实的唯一副本。

`ConversionRun` 继续作为来源层中的输入/输出超边存在，不并入 Artifact 聚合。

## Workspace 物理布局

一个 Workspace 是可独立搬迁的目录，建议布局如下：

```text
workspace/
  workspace.toml
  mathscape.sqlite3
  objects/
    sha256/
      8d/
        ae/
          8daeba7deb1fa29fedbaf6c5ebdf0db2e1e6b99e7fb227d6941f489c70ae61ef
  staging/
  quarantine/
  cache/
```

- `workspace.toml` 保存 Workspace 身份和格式版本；
- `mathscape.sqlite3` 保存地图与 Artifact 目录元数据；
- `objects/` 保存已接受、不可变的 Managed 内容；
- `staging/` 保存尚未完成校验和事务提交的输入；
- `quarantine/` 保存不可信、冲突或校验失败的内容；
- `cache/` 只保存可重建数据，不参与逻辑完整性。

受管理对象使用十六进制摘要构造路径，不使用用户文件名，因此规避大小写、Unicode 和路径字符的平台差异。目录分片避免单目录包含过多对象。

Managed URI 使用逻辑 scheme，例如：

```text
pool:sha256/8daeba7deb1fa29fedbaf6c5ebdf0db2e1e6b99e7fb227d6941f489c70ae61ef
```

解析器从显式的 Workspace 根目录解析 URI。不得相对于进程当前目录、可执行文件的原始安装路径或数据库中保存的绝对路径解析 Managed 对象。解析后的路径必须仍在 Workspace 根目录内，并拒绝路径穿越和越界符号链接。

## SQLite 持久化方向

具体 migration 将在实现中冻结，但逻辑上至少需要：

```text
artifacts
artifact_revisions
artifact_locations
artifact_checks
conversion_runs
relation_endpoints
```

- `artifacts` 保存稳定身份、内容摘要和字节大小；
- `artifact_revisions` 保存可修订描述及通用 `EntityHeader` metadata；
- `artifact_locations` 保存物理或逻辑位置；
- `artifact_checks` 保存追加式检查历史；
- `conversion_runs` 与 `relation_endpoints` 保存派生来源。

首版以 SHA-256 为主摘要。数据库和 URI 均显式保留算法名，使未来可以增加其他摘要算法而不重解释旧值。

## 导入来源与运行时位置

用户选择的原文件路径属于一次导入的来源，不等于 Artifact 的长期运行时位置：

```text
IngestionSource
  file:///Users/example/books/analysis.pdf

ArtifactLocation
  pool:sha256/8daeba...61ef
```

默认 Managed 导入流程是：

```text
external input
  -> owned staging copy
  -> streaming hash and validation
  -> deduplication
  -> atomic promotion into objects/
  -> SQLite transaction
  -> Managed ArtifactLocation
```

成功后系统只依赖 Managed Location。原路径可以保留在 ingestion provenance 中用于审计，但用户移动或删除原文件不得影响已导入材料。

Artifact Pool 接受文件时必须重新计算扫描器报告的摘要，不能只信任较早的 Marker inventory；从 staging 到对象目录的提升和 SQLite 登记必须具有明确的失败恢复边界。

## Managed 与 External 策略

Managed 是默认且首版必须实现的模式。正式进入地图、需要长期页面预览的证据材料，默认必须至少有一个可用的 Managed Location。

External 是显式选择的扩展模式，不是普通导入的隐式回退。它用于：

- 超大共享语料；
- NAS、机构只读文档库或外部对象存储；
- 多个 Workspace 共享的中心材料；
- 权限策略不允许另行复制的内容；
- 临时分析或按需 hydrate 的瘦 Workspace。

创建 External Location 前仍必须读取内容并计算摘要。外部文件消失或内容变化时，Artifact 和已有地图关系不删除，但位置变为不可用；只有重新定位到摘要一致的内容才能恢复。

首版业务入口只实现 Managed 导入。Domain 可以保留 `External` 枚举以维护 schema 演进空间，但在实现 reference-only 导入、完整性反馈和 UI 警示前不得对用户开放。

## 自包含性与搬迁

Workspace 关闭后可以整体移动到其他路径。应用从新的 Workspace 根目录打开 `workspace.toml` 和 SQLite，并重新解析所有 `pool:` URI；Managed Artifact 因此继续可用。

运行中的 Workspace 不通过普通目录复制制作一致快照。导出器必须先关闭写入并 checkpoint SQLite WAL，或者使用 SQLite backup API，再复制受管理对象。

系统应能生成 portability audit：

```text
managed artifacts
external artifacts
missing artifacts
corrupted artifacts
unavailable artifacts
bytes required to hydrate
self_contained
```

只有不存在未解决的 External 依赖，以及 Missing、Corrupted 或 Unavailable Managed 对象时，Workspace 才能声明 `self_contained = true`。导出时可选择 hydrate、保留链接、排除依赖或中止。

## 首个实现切片

以下内容已实现：

1. `WorkspaceRoot`、稳定 Workspace ID 和安全的 `pool:` URI 解析；
2. Managed object layout、owned staging、流式哈希、去重、quarantine 和原子无覆盖提升；
3. `SourceArtifact`、`ArtifactLocation` 与 `ArtifactCheck` 的 SQLite v1 migration；
4. 将 `availability` 和 `integrity` 的权威状态从 `SourceArtifact` payload 移到位置/检查层；
5. Marker inventory 提升时二次校验摘要，并在同一工作流中登记 `ConversionRun`；
6. Workspace portability audit、改路径重开和对象完整性验证。

对象文件先于 SQLite 登记完成提升。若数据库事务失败，结果是一个不可见、按内容寻址的 orphan object，而不是指向缺失字节的目录记录；相同导入可以通过摘要去重后安全重试。后续垃圾回收器负责发现和清理超过保护期的 orphan object。

External 内容导入、远程对象存储、垃圾回收与 live workspace export 均在上述边界稳定后实现。
