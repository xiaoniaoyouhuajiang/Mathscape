# 存储与外部集成

## Workspace 与权威存储

Workspace 包含 `workspace.toml`、`mathscape.sqlite3`、`objects/`、`staging/`、`quarantine/` 和可重建的 `cache/`。SQLite 保存权威元数据和来源对象；对象池保存不可变字节，大型文件不进入 SQLite BLOB。Fixture/导出 JSON 不成为第二套运行时权威来源。

Desktop 启动时通过 `ArtifactPool::create/prepare` 完成数据库迁移、历史兼容修复及完整结构校验，成功后才开放业务请求。运行期 `ArtifactPool::open` / `SqliteArtifactStore::open` 只连接已有数据库、设置连接参数并检查版本，不创建数据库、不执行迁移或完整结构扫描。独立调用方也必须先完成准备；这一区分不表示已支持多个进程同时初始化工作区。

Managed 对象按 SHA-256 寻址，`pool:` URI 相对于显式 Workspace 根解析，不能依赖进程目录、原文件名或历史绝对路径。路径穿越和越界符号链接必须拒绝。普通导入完成后，移动或删除用户原文件不影响材料；原路径仅是导入溯源。

导入先复制到自有 staging、计算摘要、验证并去重，再无覆盖提升到对象池，最后登记 SQLite。文件与数据库不是一个跨系统原子事务：数据库失败可以留下未登记 orphan object，不能留下指向缺失字节的成功记录。重试按内容去重；自动垃圾回收尚未实现。

来源登记以 `map_commit_objects` 保存实际成员与 revision，回执 v2 区分 create/reuse/reactivate/revise，旧 v1 回执继续可读。登记撤回审计与生命周期修订共用一次写事务；即使全部对象因共享而保留，也可只写撤回决定。预览指纹同时绑定地图 head 和有效采纳集合；同 request_id 的成功重试先返回原回执，再检查并发状态。撤回不回滚后续编辑，也不删除池文件或旧 revision。

Workspace 关闭后可以整体搬迁。运行时直接复制目录不构成一致备份，应关闭写入并处理 SQLite WAL，或使用 SQLite backup 机制。完整性与 portability audit 已实现；缺失、损坏或外部依赖未解决时不能宣称自包含。搬迁不包含系统钥匙串里的凭据。

External 是预留的显式模式，不是 Managed 导入失败时的回退。外部引用导入、hydrate、远程对象存储和运行中 Workspace 导出仍未接通。

发现检查点保存请求、模型正文、验证结果和 pending 任务，属于 Workspace 私有材料。它们与正式来源图分开保存；当前只在工作流退出时写入，不提供崩溃前中间状态的耐久保证。

代码：[Artifact Pool](../crates/mathscape-artifact/src/pool.rs)、[Workspace](../crates/mathscape-artifact/src/workspace.rs)、[store](../crates/mathscape-store-sqlite/src/lib.rs)、[发现记录](../crates/mathscape-store-sqlite/src/discovery.rs)。

## 研读进度与 Markdown 校对

migration 25 的 `study_marks` 保存用户本地 `unread` / `studying` / `mastered` 状态与可选笔记，不写地图对象或提升可信度。migration 26 的 `markdown_block_errata` 在不可变物理 Markdown 上保存块 / 连续块范围的校对叠加层，保留原文、校对文、物理字节范围、作者及 manual / discovery_patch 来源；支持历史、替代与撤销。已接受的发现校对也写入该叠加层，不生成替换原快照的新 Artifact。

受影响来源单元的非阻塞审核确认存入 `markdown_errata_unit_reviews`，决定为 `acknowledged` / `sent_to_sandbox`，不创建地图提交。Desktop 提供 `list_markdown_errata` / `list_markdown_errata_history` / `save_markdown_errata` / `revert_markdown_errata` 和单元审核入口。Reader 及 MCP 的 read / grep / locate 使用有效校对正文，定位仍映射到物理快照；详见 [canonical 契约](canonical-markdown.md)。代码：[Desktop 校对服务](../apps/mathscape-desktop/src/markdown_errata.rs)、[发现校对接入](../apps/mathscape-desktop/src/markdown_patch.rs)、[叠加与字节映射](../crates/mathscape-orchestrator/src/markdown_errata.rs)。

## 手稿历史、快照与来源证据

Ink 的文档、会话、操作顺序、写入者 epoch 和 durable ACK 保存在 SQLite（migration 13）；前端 IndexedDB outbox 先保留待提交操作，后台持久确认后才移除。快照（migration 14）把固定历史前缀导入 Artifact Pool，不随着当前手稿编辑而覆盖。

migration 15 增加快照请求绑定和独立的 Ink 来源证据回执。快照请求先在写事务中固定当时的元数据、head 和自包含 manifest，再完成池导入与快照登记；中断后沿用原请求前缀，多个请求可复用同一 Artifact。完成的请求只保留快照引用，未完成的请求保留恢复所需 manifest；历史列表不批量传输完整笔迹。

来源登记前重新验证实际受管字节、媒体类型、manifest 与索引，拒绝缺失/损坏、错误页或不可见笔迹。登记事务复用现有地图对象写入设施，原子写入 SourceSpan、map commit/member 和 `ink_evidence_requests` 回执。失败不产生部分地图对象，相同选区可以复用既有证据；历史 SourceSpan 继续引用原快照。ArtifactCheck 写入先取得 SQLite 写锁，避免并发校验时升级旧 WAL 读快照导致锁冲突。

新快照媒体类型为 `application/vnd.mathscape.ink-snapshot+json`、manifest 为 v2；旧 `application/json` / v1 快照通过登记索引和版本化 Schema 识别。v1 摘要遗漏局部擦除 fragment 的已知情况按原操作历史兼容重建，旧字节保持不变，v2 摘要须完整一致。普通 JSON 和 PDF/Markdown 不自动转换为 Ink。代码：[快照与会话服务](../apps/mathscape-desktop/src/ink.rs)、[证据规划](../crates/mathscape-orchestrator/src/ink.rs)、[原子证据登记](../crates/mathscape-store-sqlite/src/ink_evidence.rs)、[Ink Reader](../frontend/src/ink/snapshot-reader.ts)。

migration 16 保存 LAN 服务设置、配对摘要、设备租约、写入权转移审计和恢复导入回执。配对领取、设备授权校验与操作提交/快照前缀固定分别在同一事务中进行；不同文档、会话或旧 epoch 的请求不能写入。正常退出保留启用状态及凭据，重启重新签发同一 CA 下的叶证书；显式停止服务则撤销凭据和待配对二维码。Workspace 的 `ink-lan/trust/` 包含公开根证书和私有 CA 密钥，搬迁或备份时应一同保护，丢失其中之一不静默更换信任根。

远端 IndexedDB 按 session/epoch 保留草稿头、逐条历史及未确认队列；每笔只追加新的历史操作，避免每次复制整稿。durable ACK 才移除 outbox，历史仍用于离线打开与恢复导出。Service Worker 仅缓存静态画板，不缓存认证或材料 API；Origin 改变后浏览器旧队列不会自动迁移。`#doc=` 恢复入口只在本机 IndexedDB 中选择该手稿已保存的凭据，不携带或签发凭据；设置衔接时平板通过 HTTPS 提交本机凭据供服务端判断是否可恢复，服务端不记录不匹配的凭据。转交和设置衔接的配对状态只在内存和既有 `ink_pairings` 表中，不新增迁移。WebSocket 预览是最多 2048 点、短时失效的内存数据，最终持久笔迹保留原始样本。电脑镜像的快照只涵盖已保存前缀；平板快照先排空 outbox。代码：[LAN 服务](../apps/mathscape-desktop/src/ink_lan.rs)、[租约事务](../crates/mathscape-store-sqlite/src/ink_lan.rs)、[平板存储](../frontend/src/ink/pad-store.ts)。

被收回写入权或会话失效的平板可保持只读 standby WSS 连接，等待电脑「推送到平板」并在平板确认切换；不能借此提交旧 epoch 的笔迹。`ink-lan/standby.key` 另存工作区级待命 token（Unix 写入权限 0600），备份 / 搬迁时应作为敏感材料保护，不能把“设备凭据只存摘要”误用于此文件。正常重启保留该 token；显式停止服务轮换 token，并阻断旧设备凭据的 standby 回退认证。

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

Lean 当前已完成受管 Lean/mathlib profile、环境配置、Lean/Lake 探测、环境指纹、受限进程 runner、声明 exporter、本地 Loogle adapter、隔离 scratch 上的 `lean.check` 核心和 profile-local 的源文件/验收回执存储；回执源文件使用现有 `mathscape:artifact/sha256/<digest>` stable id，但尚未登记到 Workspace ArtifactPool。`mathscape-mcp` 已注册 `mathlib.search` / `lean.check`、绑定 round 的 `artifact.stage` 和 Phase2-0 的 `propose_changes` / `get_change_set`；前两者读取 profile，`artifact.stage` 只写 Workspace staging，Phase2 工具暂存和回读来源及语义层提案（共享对象、对齐线、Debt、最小 DialectScope；语义对象可提议退役），接受仍须用户显式操作。Lean 工具契约见 [`lean-tool-contract.md`](lean-tool-contract.md)，通用文件暂存契约见 [`artifact-staging-contract.md`](artifact-staging-contract.md)，写路径契约见 [`phase2-write-contract.md`](phase2-write-contract.md) 及 [语义扩展](semantic-write-contract.md)。与 LLM 联动的 workflow 应用仍未完成。未来形式有效性、来源翻译忠实性和人工审核须独立记录，不能把任一工具结果当作全局数学认证。

代码：[LLM](../crates/mathscape-llm/src/lib.rs)、[设置与凭据](../apps/mathscape-desktop/src/llm_settings.rs)、[版本化任务](../prompts/README.md)、[Lean 基础切片](../crates/mathscape-tool-lean/src/lib.rs)。
