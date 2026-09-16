# 存储与外部集成

## Workspace 与权威存储

Workspace 包含 `workspace.toml`、`mathscape.sqlite3`、`objects/`、`staging/`、`quarantine/` 和可重建的 `cache/`。SQLite 保存权威元数据和来源对象；对象池保存不可变字节，大型文件不进入 SQLite BLOB。Fixture/导出 JSON 不成为第二套运行时权威来源。

Desktop 启动时通过 `ArtifactPool::create/prepare` 完成数据库迁移、历史兼容修复及完整结构校验，成功后才开放业务请求。运行期 `ArtifactPool::open` / `SqliteArtifactStore::open` 只连接已有数据库、设置连接参数并检查版本，不创建数据库、不执行迁移或完整结构扫描。独立调用方也必须先完成准备；这一区分不表示已支持多个进程同时初始化工作区。

Managed 对象按 SHA-256 寻址，`pool:` URI 相对于显式 Workspace 根解析，不能依赖进程目录、原文件名或历史绝对路径。路径穿越和越界符号链接必须拒绝。普通导入完成后，移动或删除用户原文件不影响材料；原路径仅是导入溯源。

导入先复制到自有 staging、计算摘要、验证并去重，再无覆盖提升到对象池，最后登记 SQLite。文件与数据库不是一个跨系统原子事务：数据库失败可以留下未登记 orphan object，不能留下指向缺失字节的成功记录。重试按内容去重；自动垃圾回收尚未实现。

Workspace 关闭后可以整体搬迁。运行时直接复制目录不构成一致备份，应关闭写入并处理 SQLite WAL，或使用 SQLite backup 机制。完整性与 portability audit 已实现；缺失、损坏或外部依赖未解决时不能宣称自包含。搬迁不包含系统钥匙串里的凭据。

External 是预留的显式模式，不是 Managed 导入失败时的回退。外部引用导入、hydrate、远程对象存储和运行中 Workspace 导出仍未接通。

发现检查点保存请求、模型正文、验证结果和 pending 任务，属于 Workspace 私有材料。它们与正式来源图分开保存；当前只在工作流退出时写入，不提供崩溃前中间状态的耐久保证。

代码：[Artifact Pool](../crates/mathscape-artifact/src/pool.rs)、[Workspace](../crates/mathscape-artifact/src/workspace.rs)、[store](../crates/mathscape-store-sqlite/src/lib.rs)、[发现记录](../crates/mathscape-store-sqlite/src/discovery.rs)。

## PDF、Marker 与阅读器

`mathscape-pdf` 做打开检查、页几何、内容分类、OCR 建议和原生定位文本提取；不做数学解释或 OCR。Rust PDF crate 尚无渲染实现；桌面阅读使用本地打包的 PDF.js，两者不要混淆。

Marker 使用显式绝对安装路径及对应 Python 探测版本，当前固定 profiles 面向 Marker 2。请求由独立 argv 构造，不经 shell，不接受任意附加参数；一个请求只处理一个连续物理页范围，非连续选择由 orchestrator 拆成独立转换，不能拼接后伪造同一来源。

固定 profiles 使用 fast/balanced，不启用 `--use_llm` 或 `--redo_inline_math`。未来模型改写应成为独立版本的转换策略与派生产物，不能悄悄改写证据。

Runner 仅拥有本次 staging 子目录，保留日志，区分失败、超时、取消与启动错误；Unix 进程组先请求退出，再按期限终止。工具层可取消不代表 Desktop 已有 Marker 取消按钮。Scanner 拒绝不安全路径并检查产物，只有成功且完整的 inventory 可提升；提升时再次计算摘要。诊断日志不自动作为转换产品登记。

转换的相对产品路径映射随 ConversionRun 保存，用于 Markdown 图片恢复；路径上下文属于转换，不属于 Artifact 内容身份。没有旧映射时可读正文，但不能从哈希文件名猜图片位置。

阅读服务只接受已登记的 Managed Artifact ID，执行有界读取，不开放任意路径。Markdown 原始 HTML 不执行，图片只从转换清单解析，拒绝网络和越界路径。阅读并非每段都重算全文件哈希，完整性检查另有入口；切换文件应释放 PDF 任务和 Blob URL，迟到响应不能覆盖当前阅读状态。

代码：[PDF](../crates/mathscape-pdf/src/lib.rs)、[Marker](../crates/mathscape-tool-marker/src/lib.rs)、[产物提升](../crates/mathscape-orchestrator/src/marker_promotion.rs)、[阅读服务](../apps/mathscape-desktop/src/content.rs)。

## LLM 与 Lean

`LlmBackend` 抽象一次普通或流式调用；`GenaiBackend` 隔离供应商 SDK、协议、endpoint 和错误转换。协议由 profile 显式指定，不根据模型名猜测。能力声明不等于供应商实测保证。

Profile 保存凭据引用；Desktop 使用系统钥匙串，API key 不进入 SQLite、请求日志或错误输出。传输原始响应采集默认关闭，但发现工作流仍会保存用于审计的请求和输出正文，不应误称为“默认不保存模型内容”。

底层调用不自动重试；预算、修订和取消由编排层负责。SSE 未收到终止事件应作为传输错误，即使累积正文恰好能解析。结构化输出只保证可解码契约，供应商 Schema 兼容转换不能放宽本地验证。当前发现流程不调用模型工具，也不自动读取输入外文献。

Prompt、manifest 与 Schema 有独立版本和指纹；模型配置和生成参数也影响可重放性。旧 prompt 包因回放、比较和测试保留，不能因主文档精简而删除。

Lean 当前只有 crate 占位，尚无执行、证书收集或与 LLM 联动的应用实现。未来形式有效性、来源翻译忠实性和人工审核须独立记录，不能把任一工具结果当作全局数学认证。

代码：[LLM](../crates/mathscape-llm/src/lib.rs)、[设置与凭据](../apps/mathscape-desktop/src/llm_settings.rs)、[版本化任务](../prompts/README.md)、[Lean 占位](../crates/mathscape-tool-lean/src/lib.rs)。
