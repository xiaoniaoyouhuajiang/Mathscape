# 手写输入基础设施开发提案 v1

状态：Ink v0 设计提案；P0 软件原型已实现，真实设备实验待执行，P1–P3 待实施。除已提交的 Ink Lab 契约外，本文中的类型、表名和接口名仍是候选契约。

## 1. 目标与首个验收场景

为 Mathscape 建立可复用的手写输入底座：采集笔迹、保留编辑历史、可靠保存与恢复，并把指定时刻的手稿冻结为可引用的来源证据。桌面数位板、局域网平板浏览器和未来 Android 原生采集端使用同一套 Ink 契约。

首个完整验收场景是：用户从一道已有习题开启书写会话，在 Samsung Tab S8+ 上写推导，期间网络中断，恢复连接后完成同步；用户保存一个快照，退出并重启桌面应用，仍能找到该手稿、打开当时的内容，并准确定位到选定的笔迹。随后继续书写、擦除或撤销，不改变已经保存的快照与来源定位。

Ink v0 的完成条件是这个闭环可靠成立。模型分析和数学语义晋升以此为输入基础，另行开发。

## 2. 设计依据与现有边界

工程事实以[架构](../docs/architecture.md)、[数据模型](../docs/data-model.md)、[存储与集成](../docs/storage-and-integrations.md)和当前代码为准；数学来源与晋升边界参考 [EMIR](../../docs/rdtx/emir-source-semantics-alignment.md)。[手写输入调研](handwriting-input-mode.md)提供生态与交互候选，不作为实现完成度或设备能力的保证。

已有能力可复用：

- Rust 领域层、SQLite 存储、Tauri 命令与 TS 前端之间的职责划分；
- Artifact Pool 的不可变字节、内容寻址、完整性检查与受限读取；
- StableId、RevisionId、来源对象、MapPatch 及登记事务；
- 来源材料阅读、查询与只读图投影。

本次开发需要补足的接口：

- 当前没有统一的 Ink 会话、笔迹操作与可靠同步契约；
- 当前 `SourceSpan` 接受 PDF/Markdown 定位，尚不能直接表达 Ink 定位；
- 阅读与来源详情需要增加 Ink 快照和选区的打开能力；
- 当前 LLM 消息契约没有图片输入。本提案保留分析输入边界，不把多模态调用列入 Ink v0。

保存手稿不要求已有完整书目、DialectScope、共享语义或 Lean 证书。Ink 来源定位也不应通过生成 Markdown 或 PDF 来绕过。

## 3. 本次交付范围

### 3.1 Ink v0 必须交付

1. 有限页面上的普通笔书写、整笔擦除、撤销重做、缩放与平移；
2. 保存采集端报告的样本、笔刷描述和编辑操作，保留取消与中断状态；
3. 桌面本地采集与会话列表、重开、继续书写；
4. 单写入者的 LAN InkPad、二维码配对、写入确认与断线续传；
5. 自动保存、已确认数据的进程崩溃恢复、客户端未确认队列恢复；
6. 后端生成的不可变 Ink 快照，通过 Artifact Pool 管理；
7. 快照上的整笔选区、Ink locator、SourceSpan 登记和来源回链；
8. 协议、持久化、旧数据迁移与真实设备验收材料。

### 3.2 后续扩展

本次不实施：多写入者协作、CRDT、云同步、完整 Android Mathscape、局部擦除、移动旋转笔迹、复杂笔刷、无限画布、OCR/HMER、自动来源单元切分、共享语义对齐、ProofState 或 Lean 工作流。

Android 原生输入是由设备实验触发的扩展分支。若 Web 输入达不到基本书写要求，可提前验证原生采集，但仍遵循同一 Ink 契约，不同时扩展完整移动产品。

## 4. 核心语义与不变量

### 4.1 观察记录与数学解释分开

Ink 保存用户输入和编辑的可观察事实。笔迹顺序、停顿、擦除、圈画和修改能支持后续解释，但不能直接等同于用户的推理状态。一笔不等于一个公式，一次停顿不等于一个证明步骤，撤销也不自动表示放弃一种证明路线。

底层使用 Ink 编辑历史或 `InkTrace`。未来模型提出的转写、思路、证明步骤和分支属于解释候选，引用固定快照和选区，并保留不确定性。输入质量、解析质量、数学正确性与人工接受分别记录。

### 4.2 会话、文档、历史与快照分开

| 对象 | 职责 | 身份与变化 |
| --- | --- | --- |
| `InkDocument` | 页、笔迹及当前可编辑内容 | 稳定 document_id；当前状态由有序历史派生 |
| `InkSession` | 一次书写活动、设备与关联材料 | 稳定 session_id；可以继续已有文档 |
| `InkOperation` | 笔迹、页面与可见性操作 | 唯一 operation_id；只追加，不改写旧操作 |
| `InkStroke` | 一次采集产生的样本序列与笔刷 | 稳定 stroke_id；完成状态显式记录 |
| `InkSnapshot` | 固定历史前缀及其内容 | snapshot_id 与受管 Artifact；内容不可变 |
| `InkSelection` | 固定快照中的选定证据 | 绑定快照及明确笔迹集合 |

连接断开不等于会话结束；会话结束不等于文档封存。历史顺序由后端文档序号决定，不依赖不同设备的墙上时钟。

### 4.3 必须保持的不变量

- 已持久确认的操作在支持的进程崩溃恢复场景下不丢失；未确认操作不得显示为已在桌面持久化。
- 同一操作的相同内容重试返回原回执；不同内容复用同一标识必须冲突。
- 擦除与撤销改变可见状态，不物理删除原始样本或旧历史。
- 快照绑定确切的历史前缀；后续编辑不能改变快照字节、笔迹集合或旧 locator。
- 所有来源定位具有可验证锚点；不能把 Ink 页号填入 PDF 页号字段。
- 缺少压感、倾角或上下文时保留缺失或未知状态，不补造能力和数学环境。
- 预测点只用于临时渲染，不写入实际采集证据。
- 前端和远程客户端不拥有来源登记权限；后端重新绑定受管输入并执行验证。

## 5. 架构与模块落点

```text
Desktop Pointer Adapter          Browser InkPad          Android Adapter（后续）
          │                            │                         │
          └──────────── Canonical Ink DTO / Operations ───────────┘
                         │                    │
                    即时本地渲染         客户端未确认队列
                                              │
                                   Tauri IPC / Ink transport
                                              │
                                      Ink Application Service
                                              │
                              验证 → 有序持久化 → Durable ACK
                                              │
                                     固定历史前缀 / 快照
                                              │
                                        Artifact Pool
                                              │
                              Ink Reader / Locator / SourceSpan
```

候选落点如下；新增独立 crate 以形成长期模块边界为条件，不为原型机械拆包。

| 模块 | 工作 |
| --- | --- |
| `mathscape-domain` 的 Ink 模块 | 值类型、操作 DTO、快照、locator 与确定性不变量；不依赖 UI/数据库 |
| `mathscape-store-sqlite` | 会话、操作、回执、写入者 epoch 和快照引用的事务存储 |
| `mathscape-artifact` | 复用受管字节导入与读取，不增加第二套 Ink 对象池 |
| `mathscape-orchestrator` 的 Ink 模块 | 输入重绑定、快照构建、定位验证与来源登记计划 |
| `apps/mathscape-desktop` | 统一应用服务、Tauri 命令、LAN 服务生命周期与事件 |
| `frontend/src/ink/`（新增） | 采集、即时投影、渲染、队列、会话入口与快照阅读 |
| `mathscape-query` / `mathscape-graph` | 新定位的只读回链和投影；不承载书写工作状态 |

TS 需要即时投影以保持笔尖反馈；Rust 负责权威验证和提交。两端使用同一版本化操作语义和 golden fixtures，不各自定义一套文档规则。领域契约由 Rust DTO 及 Schema 驱动，前端类型的生成方式在 P0 确定。

桌面 IPC 与 LAN 请求经过同一应用服务。网络层只做会话认证、消息边界和传输，不另写一套笔迹存储或来源登记逻辑。

## 6. Ink 数据契约

### 6.1 采样、坐标与能力

`InkPoint` 至少表达页坐标 x/y、采集片段内相对时间，以及可选 pressure、tilt 和设备报告的相关属性。字段单位、精度、范围和缺失语义必须写入版本化契约。

坐标使用固定页面坐标系，原点、轴方向和页面尺寸明确。viewport、缩放比例、devicePixelRatio 和屏幕方向属于采集或显示变换，不改变已经保存的页坐标。第一版页面尺寸创建后固定，新增页使用稳定 page_id，显示页码只是可变阅读顺序的标签。

采集适配器记录实际运行环境、设备类别与能力状态。API 字段存在或值为零不表示硬件支持对应传感器；能力可以为 unknown。归一化方式与数值精度由 P0 fixture 验证，Schema 冻结后才能进入 P1 持久化。

必须保存用于重建笔迹的样本序列。平滑轮廓、降采样预览、Canvas 路径与预测点不替换这些样本。处理 coalesced events 时避免重复加入父事件；同时使用 rawupdate 与 move 时也必须避免重复采样。

一个 `capture_segment_id` 表示连续采集时钟片段。页面刷新、进程重启后生成新片段，不能把新的相对时间接成旧采集时钟；文档操作序号继续保持顺序。

### 6.2 操作与用户命令

候选操作类别：

```text
page.add
stroke.begin
stroke.append_samples
stroke.finish       // completed | cancelled | interrupted
stroke.set_visibility
history.undo
history.redo
```

一笔书写是一个用户 command，可包含多个有界样本批次。整笔擦除是一个独立 command；撤销重做针对用户 command，而非每个采样批次。

每次撤销或重做也追加历史，后端根据明确目标 command 和保存的前态执行。第一版只撤销已经完成的命令；活动笔迹先完成或取消。`stroke.begin/append/finish` 必须构成合法状态机，终结后不得继续向原 stroke_id 加点。

样本不能逐点跨 Tauri 或逐点同步 SQLite。使用有界批次；批次大小和最大缓存时间在 P0 测量后固定，并覆盖抬笔、取消和关闭时的尾批次。

### 6.3 关联材料

`InkContextRef` 保存关联对象、角色、来源范围及所依据的 revision。例如 primary_problem、reference_material；允许没有关联的自由草稿。新关联追加修订或操作，不静默覆盖旧会话的语境。

会话上下文仅说明用户当时关联了哪些材料。它不自动构造 DialectScope、RepresentationBinding 或 ProofContext，也不自动建立 `proves` 等来源关系。旧材料已退役时仍应解析历史引用，并显示其当前状态；不能悄悄换成当前 revision。

### 6.4 快照与选区

候选快照 manifest：

```text
schema_version
document_id / originating_session_id
through_document_seq
parent_snapshot_id（可选）
page specifications
capture segments / brush specifications
ordered operations and samples through the frontier
context references at the frontier
materialized visible state fingerprint
```

v0 优先使用自包含快照，包含重建该历史前缀所需的数据，方便重开与搬迁。manifest 与可见状态摘要必须可由后端重算；确定性序列化规则固定，内容哈希以实际受管字节为准。后续若改为分块 manifest，新版本必须显式引用全部不可变依赖。

整笔选区保存快照 Artifact、page_id 和去重后的 stroke_ids。框选或套索手势转换成显式笔迹集合，规则固定并记录；第一版不承诺笔迹内部样本级裁切。跨页选择拆成多个定位。

普通选区只选择快照中可见的笔迹；查看已擦除内容须显式进入历史视图并绑定对应历史位置。不能用最终可见图像解释用户此前看到的全部状态。

## 7. 持久化、恢复与快照提交

### 7.1 权威存储

SQLite 保存会话元数据、操作顺序、幂等回执、写入者状态与快照引用。小型操作及有界的小样本批次可直接保存为版本化 DTO；大批次使用不可变 payload Artifact，SQLite 保存引用。两种承载方式采用相同的样本验证和操作指纹规则，阈值在 P0 测量后确定，避免每个采样批次都产生一个小文件。不能把持续增长的全部笔迹放进一个可变 SQLite BLOB 或另一个未纳入 Workspace 的文件。

候选逻辑表为 ink_documents、ink_sessions、ink_operations、ink_writer_state、ink_snapshots、ink_requests。它们保存写入事实；可重建投影不是第二套权威数据。快照来源与 catalog 仍复用现有 Artifact 结构，不把每笔 stroke 都创建成地图节点。

客户端先把待提交批次写入本地 outbox，再允许该批次显示为“本机已保存”。浏览器使用 IndexedDB，需检测可用性与写入失败；本机保存失败时明确展示，不宣称能在刷新后恢复。桌面本地入口采用相同提交语义，避免绕过验证。

### 7.2 操作提交与确认

后端处理持久操作的顺序：

1. 检查消息上限、Schema、样本数值、状态机和会话授权；
2. 如需 payload Artifact，先通过现有 staging/提升路径登记不可变字节；
3. 在一次写事务中检查写入 epoch、顺序与幂等，登记操作、更新 head、写入回执；
4. 事务成功后发送 durable ACK；客户端收到后才移除对应 outbox 项。

服务端收到消息、转发预览或尚未提交的投影都不是 durable ACK。重复请求先核验原请求指纹并返回原成功回执，不被后来变化的 head 错误拒绝。payload 登记后业务事务失败可留下未引用 Artifact，但不得出现成功操作引用缺失字节；重试复用内容。

浏览器关闭前的网络 flush 是尽力而为，不能作为耐久保证。尚未送到客户端 outbox 的尾部样本也不能承诺恢复；该损失窗口由有界批次控制，并在设备实验中记录。

### 7.3 中断恢复

后端重开以操作历史和已提交 head 为准，重新验证序号、引用、哈希及投影指纹。未终结笔迹显示为待恢复；启动时不立即追加 interrupted 终结，因为原客户端 outbox 可能仍有合法的后续样本和 finish。先核对原写入者的恢复队列，无法继续的笔迹再通过幂等操作标记 interrupted；不能补造缺少的末端点或把 interrupted 自动标成 completed。

客户端恢复 outbox 后，先核对服务端最后确认序号，再按原顺序提交。已撤销的写入权限或旧 epoch 队列必须保留并显示冲突；不得自动重编号并写进当前文档。用户可将其导入独立恢复草稿，这应是显式、可审计的操作。

本提案承诺已确认数据的受支持进程崩溃恢复。文件系统损坏、存储介质失效与浏览器数据清理属于可诊断的材料缺失，不能以网络重试掩盖。延续现有 Workspace 备份与完整性边界。

### 7.4 快照创建

用户“保存快照”携带 request_id 和预期文档 head。客户端先完成活动笔迹并同步待提交操作；仍有 pending 或冲突时不能把快照展示成包含这些内容。允许对明确显示的服务端已保存前缀生成快照，但必须说明其范围。

快照创建流程：

1. 后端在短事务中校验预期 head，固定 through_document_seq，并持久保存幂等请求绑定；
2. 验证该前缀的完整历史与 Artifact，构建确定性 manifest；
3. 通过 Artifact Pool 导入快照字节；
4. 在事务中完成快照引用和回执登记；
5. 返回快照 Artifact 和来源阅读入口。

后续书写可以继续追加，不影响步骤 1 固定的前缀。跨文件与数据库步骤不宣称全局原子；未知结果重试必须恢复同一请求所绑定的前缀，不能改用更新后的 head。失败快照不能显示为成功。

PNG/SVG 不是必需权威存储。v0 从受管快照重建阅读画面；若保存预览，则作为独立派生产物，记录输入快照、渲染器版本、笔刷配置和坐标映射。高级笔刷的跨平台像素一致性另行验证。

## 8. LAN 协议与单写入者

### 8.1 通道职责

采用 Axum + WebSocket、二维码配对和版本化 JSON。二进制编码、mDNS 和 CRDT 留待性能与产品需求明确后决定。

通道区分 preview 与 durable operation。preview 可丢弃，只用于桌面实时观察；durable operation 必须有序、幂等并得到持久确认。书写端始终本地渲染，不等待服务端往返。

概念消息：

```text
hello / resume
session.context
preview.stroke
operation.submit
operation.ack
operation.conflict
snapshot.request / snapshot.receipt
session.closed
```

持久操作信封至少包含 protocol_version、document_id、session_id、writer_epoch、operation_id、client_seq、previous_operation_id 和 payload。握手绑定文档 head；随后使用连续序号和前驱标识验证链，保证离线排队不依赖逐条取得服务端新序号。

消息大小、单批样本数、队列容量和单会话总量设定显式上限。超限返回诊断或背压，不静默截断样本。

### 8.2 单写入者与重连

同一文档只允许一个活动写入者；桌面镜像为观察端。申请切换到桌面书写时，先同步或保留旧端 pending，再显式转移写入权并生成新 epoch。断开连接本身不立即转移权限。

同 epoch、相同 operation_id 与内容重试返回原 ACK；客户端序号有缺口时要求补齐；前驱不符、同标识不同内容或旧 epoch 写入返回冲突。重连凭据与文档租约分别校验，不能仅凭 session_id 取得写权限。

应用重启后的恢复认证和租约状态属于 P3 必须验证的协议情形。新的配对不能静默接收旧端尚未确认的历史；服务端应区分原写入者恢复与新的写入者接管。

### 8.3 配对与能力限制

LAN 服务由用户开启，关闭会话或服务后撤销相应凭据。二维码使用短期、单次配对凭据，成功后换取限制到当前会话的连接凭据；日志不记录完整凭据。

只暴露 InkPad 静态资源、当前会话明确授权的上下文和 Ink 操作，不开放任意文件读取、完整 Workspace 查询或通用 Tauri 命令。校验 WebSocket Origin 与会话凭据，设定超时和资源上限；Origin 不能替代认证。

P0 可在可信局域网使用 HTTP 验证，但必须记录 secure-context 能力限制。正式使用方案在 P3 明确：浏览器路径采用设备信任的 HTTPS/WSS，或使用具有受保护传输的 Companion。不能将自签名证书未经设备信任的连接宣称为已解决安全上下文。若 TLS 信任与离线队列在目标设备上无法可靠成立，LAN Browser 仍属实验能力，不标记正式验收完成。

## 9. Ink 定位与 EMIR 来源接入

### 9.1 新定位类型

在 SourceSpan 中新增可独立成立的 Ink locator，保留旧 PDF/Markdown payload 的兼容读取。候选字段：

```text
InkLocator
  snapshot_artifact_id
  page_id
  stroke_ids
  selection_mode       // whole_strokes in v0
```

Ink locator 必须验证快照媒体类型、Schema、实际内容哈希、page_id、stroke_ids 和选择模式。证据显示 bbox 可由笔迹及笔刷几何派生；bbox 不是唯一稳定身份。用户框选矩形不能替代被选中的具体笔迹集合。

新 locator 自身足以让 SourceSpan 可定位。page/block/inline 的既有规则不能未经解释直接套到 Ink；P0 fixture 确定新增粒度或类型分支，并同步到序列化、构造器、存储验证、query 和 reader。不得为 Ink 构造空壳 PDF PageCoordinate。

### 9.2 保存与登记状态

必须区分：

```text
本机 outbox 已保存
→ 桌面操作已持久确认
→ 不可变快照已登记到 Artifact Pool
→ 用户指定选区已登记为来源证据
```

这些状态分别有成功条件。保存快照不自动接受数学解释，也不自动登记 SourceSemanticUnit。

v0 提供“登记此选区为来源证据”：后端从受管快照重建选择、构造 SourceSpan 与受控登记计划，按 request_id、预期快照身份和选择指纹原子提交地图对象与回执。来源登记失败不影响已经保存的手稿；重试不能重复生成不同证据对象。

这个用户指定的定位入口不伪造 discovery run，也不绕过现有地图对象不变量。可以共享底层事务和 MapPatch 验证，但需要独立的业务请求和授权边界。SourceSpan 的稳定身份基于固定快照和规范化选择，不能按显示页码或 bbox 猜测。

### 9.3 来源阅读与后续晋升

点击 Ink Artifact 或 SourceSpan 打开对应快照，显示选定笔迹，允许返回相关习题与书写会话。快照阅读为只读；“继续书写”打开当前文档并明确区分当前内容与历史快照。

材料图可展示快照 Artifact，证据模式展示 SourceSpan。高频采样、连接状态和未确认草稿不进入来源图。后续从手稿提取来源单元时，继续遵循候选、验证、用户接受与登记的边界；未完成的尝试不因用户写下或保存而获得已证明状态。

## 10. 前端交互

提供两个会话入口：从已有来源单元“开始手写”，以及独立“新建手稿”。会话列表显示最近文档、关联材料、最后持久保存时间及 pending 状态，重启后仍能找到。

书写页显示关联题目或材料摘要、有限页面、笔工具、整笔擦除、撤销重做、缩放平移、保存快照与连接状态。默认以笔书写、触摸导航；掌触和笔触并发策略在目标设备实测，不能把 Pointer Events 支持等同于已实现掌拒。

状态使用用户可理解的表述：本机已保存、等待桌面同步、桌面已保存、连接中断、同步冲突、快照已保存。关闭或切换时仍有 pending，应展示数量和恢复入口，不能用无条件“完成”按钮抹去未确认数据。

框选或套索首先选择整笔；可预览实际选中笔迹，再登记来源证据。未来“询问 Mathscape”使用固定快照与选区，另由多模态提案定义，不在 v0 接入按钮占位或自动发起模型请求。

## 11. 开发任务与依赖

### P0 设备实验与契约 fixture

P0 软件闭环为：电脑启动独立 Ink Lab → 平板扫码打开浏览器画板 → 本地即时书写与渲染 → 手动导出一个自包含 Ink JSON → 重新导入重现内容与编辑历史。另提供“保存到电脑”，把完整文件放到实验目录。启动和使用说明见 [Ink Lab](../frontend/ink-lab/README.md)。

P0 的 LAN 服务是独立 Node HTTP harness，仅提供打包画板与实验文件接收；正式 Axum 服务、配对租约、自动保存、断线续传及 Workspace 登记仍属于 P1–P3。手动 JSON 导出不是 P2 的受管不可变快照，浏览器刷新前也不承诺自动恢复未导出内容。

当前软件交付：

- [x] 带局域网地址选择与二维码的电脑启动页面；
- [x] Pointer Events + Canvas 画板，普通笔、整笔擦除、撤销重做与缩放平移；
- [x] Ink JSON 下载、校验后导入继续书写、PNG 附加预览；
- [x] 手动发送到电脑、文件完整写入后的回执与下载入口；
- [x] Rust DTO、生成 Schema、TS 投影，以及跨采集片段和取消/中断笔迹的共享 fixtures；
- [x] 协议/文件验证、读写失败、离线导出、导入重现和浏览器交互自动化；
- [ ] Tab S8+ / S Pen 与目标数位板实测，20 分钟书写记录及性能门槛收敛。

先实现独立采集实验，记录 Tab S8+ / S Pen、桌面目标数位板、OS/驱动、Chrome 或实际 WebView 版本。比较 LAN HTTP、安全上下文和实际 WebView；没有测试到的环境明确标为未测。

测量笔尖反馈、真实压力变化、快速细小公式、倾角与按钮、掌触、pointercancel、快速缩放、持续书写性能，以及实际收到的采样序列。至少包含一段 20 分钟书写与包含上下标、根号、分式和擦改的小型数学样本。

输出能力矩阵、原始实验记录，以及完成/取消笔迹、整笔擦除与撤销、跨采集片段重开、快照定位的 fixtures。固定坐标与数值编码、操作规则、批次上限、序列化和 Schema，给出基于测量的性能门槛。输入体验达不到基本要求时先验证 Android 原生适配，不直接扩展所有产品功能。

### P1 本地会话与可靠保存

实现领域值类型、确定性验证、SQLite 迁移、payload 引用、幂等回执和统一应用服务。前端接入即时采集与 outbox，提供最小编辑能力、会话列表和重开。

完成已确认操作恢复、未终结笔迹处理、重复提交、损坏 payload、存储失败与客户端队列恢复。必须在这些条件成立后进入 LAN 集成，不能用网络重试补救本地提交语义。

### P2 快照与来源定位

实现固定历史前缀、确定性快照、Artifact 导入、快照幂等、只读 Ink reader、整笔选区和 Ink locator。完成 SourceSpan 扩展、受控证据登记、历史回链及旧 payload 兼容。

以“保存快照后继续擦改，旧证据仍精确打开”为阶段验收。快照创建与操作写入并发时，也必须保持前缀一致。

### P3 LAN InkPad 与端到端验收

接入受限 Axum 服务、独立 InkPad 页面、WebSocket、配对与单写入者协议。实现预览/持久通道、背压、断线重连、桌面重启认证恢复和写入权转移冲突处理。

在真实 Tab S8+ 完成首个验收场景，并解决受保护传输、客户端持久化与 Web API 能力差异。桌面本地入口与远程入口对相同操作 fixture 生成相同的数学书写内容与选择结果；仅在采集和会话元数据也相同时要求快照字节一致，真实设备的来源元数据不得为追求相同哈希而被删除。

依赖关系为 P0 → P1 → P2 → P3。P0 可以使用小型临时 LAN harness 测量设备，正式网络协议与产品服务仍由 P3 交付。多模态输入和 Android 增强分别以 v0 契约为前提，不改变这个来源闭环。

## 12. 验证与验收

### 12.1 自动化验证

| 层 | 必须覆盖的情形 |
| --- | --- |
| Domain / Schema | 非有限数值、越界、非法状态机、缺失能力、错误页/笔迹引用、未知版本 |
| TS / Rust 一致性 | 同一操作 fixture 的可见状态、撤销重做、快照指纹和选择结果一致 |
| Store | 迁移、重开、并发写入、序号缺口、幂等重试、同标识异内容、事务失败 |
| Artifact / Snapshot | payload 损坏、提升失败、登记失败、快照前缀固定、失败后重试 |
| Transport | ACK 丢失、重复发送、网络中断、旧 epoch、超限、背压、无效凭据 |
| Desktop / Reader | 真实 Pool/SQLite 服务测试、重开、历史快照打开、SourceSpan 回链 |
| 兼容 | 既有 PDF/Markdown locator、旧 Workspace、现有来源图与发现工作流 |

故障注入至少覆盖：事务成功前断开、事务成功后 ACK 丢失、payload 已登记但操作未提交、快照 Artifact 已生成但快照回执未提交、服务端重启、客户端刷新、未终结笔迹，以及写入权转移后旧端恢复。

默认测试使用轻量可分发 fixtures，不依赖私人教材、真实模型或物理设备。浏览器自动化验证交互与协议，不能替代真实 WebView 和 S Pen 测试。性能门槛在 P0 明确后作为后续验收依据，不用未经测量的风险消除百分比。

### 12.2 Ink v0 完成标准

1. 从已有习题或自由草稿开启会话，在目标设备连续书写达到 P0 约定门槛；
2. 压感与能力缺失按实测记录，预测与渲染样本不污染采集证据；
3. 擦除、撤销和重做不删除历史，可重建相应可见状态；
4. 支持的进程崩溃场景中，所有已 durable ACK 的操作在重开后存在且不重复；
5. 断网期间本地书写不等待网络，成功写入 outbox 的操作在恢复后有序同步；
6. pending、冲突和存储失败均有明确状态与恢复入口；
7. 快照请求绑定固定前缀，重复请求返回同一回执，后续编辑不改变旧内容；
8. 从 Ink SourceSpan 能打开正确快照和选定笔迹，关联材料可回溯到指定修订；
9. 来源登记失败不会被展示为成功，也不破坏已保存手稿；
10. 已有 Workspace 可升级，旧 PDF/Markdown 定位与相关回归通过；
11. LAN 正式使用的传输信任、恢复认证与单写入者边界经真实设备验证；
12. 首个端到端场景、设备矩阵、性能结果和故障恢复证据进入验收记录。

## 13. 技术选择与后续接口

第一版采用 Pointer Events + Canvas 2D + 可替换的 perfect-freehand 渲染适配；真正的存储模型属于 Mathscape。使用实测压感时关闭渲染器的模拟压力，保存笔刷与渲染配置以支持回放。

Android 高质量输入候选为 Tauri Kotlin 插件与 AndroidX Ink。原生端负责本地渲染和缓冲，批量输出统一 Ink 操作；不逐点在 Kotlin、Rust 和 JS 之间往返。原生笔刷或网格不能成为唯一来源格式，基础样本必须仍能被 Mathscape 读取。

未来分析输入包至少绑定 snapshot_artifact_id、selection、context_revisions、user_intent、render_artifact/config 与处理预算。分析结果保存 run、输入指纹、版本和证据，不回写原始 Ink。LLM 图片消息、供应商能力、受管图片解析、成本与取消另行实施。

参考能力来源：

- [Pointer Events 规范](https://www.w3.org/TR/pointerevents3/)：采样、默认属性、coalesced/predicted events 与安全上下文；
- [perfect-freehand](https://github.com/steveruizok/perfect-freehand)：笔迹轮廓与实际/模拟压力配置；
- [Tauri 移动插件](https://v2.tauri.app/develop/plugins/develop-mobile/)：原生 Kotlin 适配边界；
- [AndroidX Ink](https://developer.android.com/jetpack/androidx/releases/ink)：后续原生采集与低延迟渲染候选。

## 14. 实施前需收敛的决定与文档维护

P0 收敛数值编码、批次大小、性能门槛、基础笔刷以及 Ink locator 的粒度规则；P3 收敛局域网传输与设备信任方案。这些决定必须由 fixture 或实测支撑并更新本提案，不能以“后续再说”绕过对应阶段验收。

物理表结构、函数签名、Schema 与协议资源在实现中冻结；数据库迁移顺序追加，不预占当前仓库的迁移序号。实现后按已交付能力更新四份工程主文档，提案继续区分已完成任务和后续扩展。

本次提案不修改手写生态调研、不替换现有来源发现提案，也不要求先实现 EMIR 全部语义层。Ink v0 的长期边界是忠实记录、可靠恢复、不可变快照与可验证来源定位。
