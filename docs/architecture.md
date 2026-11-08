# 架构

Mathscape 是面向数学学习者和研究者的本地优先桌面应用。当前实现聚焦材料摄取、来源发现、人工审核与来源图；数学语义解释和跨领域关联是目标，不能把当前来源抽取能力等同于该目标已经实现。

## 软件目标

这里回答——使用mathscape的人会希望该软件能带来什么，我认为应该是：
* 能读懂物理，计算机科学，运筹学，金融以及各种理工学科的论文，技术报告中的理论部分——这些学科基本都使用了数学作为建模的基本工具，因此数学是理工学科理论上的基石，利用LLM和各种PDF材料，mathscape让用户能够清楚表达出自己的困惑，让LLM基于这些问题和现实中的材料来让用户切实理解各种现象和设计为何是合理的，举一个例子：一位数学和机器学习的共同爱好者想通过各种论文和数学教材弄明白学术界现阶段如何去解释大语言模型为什么这么强大，在AI的指导下，他得知了主线论文是哪些，要研究的数学理论，比如：正交分解、不变子空间；最小二乘的正交投影，与无限维函数空间中的表示定理；梯度下降算法，与注意力的一次前向计算等等。他去阅读论文，发现很快碰壁了——他并没有足够的数学基础，所以他要扫清障碍——mathscape能帮他梳理他的疑惑（通过具体的问题关联具体的推导过程），通过一些问题了解用户的数学基础。于是LLM告诉他需要学习哪些理论，然后用户将这些理论对应的书籍收集起来了。之后，mathscape内的agent通过扫描这些书籍的目录来评估和规划一条适合读者的learning roadmap，这个roadmap由这些内容组成：
  * 用户应当看哪几本书的哪些章节，阅读的顺序是什么
  * 用户每次阅读完一部分内容后，要完成的习题有哪些
  * 在agent构建这条**具体的道路**过程中，agent会精准地总结出用户需要的**理论基础**是什么，这条道路中的**关键节点**是什么，agent会扫描这条具体道路所引用的markdown材料，同时对**必要的节点**完成**来源层和语义层的构建**
  * 这条路径会被持久化到系统当中，用户完成习题，用户认定对一个Theorem&claim达到理解状态都会更新这条路径的完成进度
* 用户可能对某一个具体的定理，具体的题目的解法感觉到迷惑，或者是想要围绕具体的定理或问题进行**深挖（dive into）**。用户会给出某一份markdown，并选中其中具体的题目，证明过程或者是定理。并告知自己的诉求，很多时候用户的诉求一开始是他在理解或尝试解决问题过程中的一种感觉，因此这里agent需要和用户交流（向他提出一些问题）来引导用户将问题细化，从而识别到agent要做的具体任务。此外，agent可以通过提问或是给出一些具体问题，用户回答或是写一些手稿提供给ai来让其更加清楚当前自己的能力边界。当诉求明确后，mathscape-agent将会利用这本教材的PDF结合markdown解析能力进行**探索**，这个过程和上一个case基本一致，即找到关键的定理，定义，找到可以利用和参考的习题和例子，这个过程会将系统内部的图数据，文本数据，甚至是lean mathlib中的数据，这些知识的索取手段将会被封装为系统内部的一些**tool**提供给agent；同样的，这个过程中，agent会**按需更新语义/来源图**，并且输出一份提供给用户的**解释**，目前考虑会包含的内容有：
  * 对题目的理解：RDTX模型以及各种语义节点和其来源节点，以及它们所构成的子图
  * 用户原本的思路存在的问题（这个可以通过手写输入来识别），为了确保这种问题的识别是准确的，agent可以提供一些问题给用户，用户通过手稿来传达自己当前的理解
  * 总结出有益于用户举一反三的内容，包括相关的理论，其他解法，和相关的题目/例子
  * （规划中）：利用所有的这些数据，agent甚至能利用mathscape内部的一些数学动画制作IR来生成一个数学动画

我的这些设想有几项前提：
* 理工科的论文/材料中理论部分可被分解为数学理论部分和数学无关的理论部分，mathscape需要覆盖的是数学理论部分，并且这部分绝对可以被数学学科的知识全覆盖
* mathscape的来源层/语义层节点，关系能够增量式**惰性**更新，agent是根据用户的需求来找到自认为真的有关联的部分，并为他们进行必要的节点生成（对图的更新将会被封装为tool），因此这个计算实质上是惰性的，我们要保证这种更新范式必须长期稳定存在，并且不会与emir本身的规则相冲形成系统性矛盾

## 模块与依赖

| 模块 | 职责与边界 |
| --- | --- |
| `mathscape-domain` | 身份、修订、来源实体、角色化超边、MapPatch 和传输契约；不依赖 UI、数据库或供应商 |
| `mathscape-store-sqlite` | 目录、发现运行、接受决定、来源对象的持久化、事务与类型化读取 |
| `mathscape-artifact` | Workspace、不可变受管字节、导入、完整性检查；调用存储层登记目录 |
| `mathscape-pdf` / `mathscape-tool-marker` | PDF 检查与选区 / 外部转换进程；无地图写入权限 |
| `mathscape-llm` | 供应商中立调用契约、凭据解析和传输适配；不决定任务或可信性 |
| `mathscape-orchestrator` | 输入准备、模型任务、确定性验证、修订、来源登记计划和转换产物提升 |
| `mathscape-query` / `mathscape-graph` | 从权威快照构建只读视图 / 超边展开与运行时索引 |
| `apps/mathscape-desktop` / `frontend` | Tauri 用例与事件 / 阅读、审核、设置和 Cytoscape 渲染 |

依赖从应用指向上述能力模块，领域层不反向依赖应用。前端通过 Tauri 命令操作，不能直接访问 SQLite 或任意主机文件。实际依赖以各 crate 的 Cargo.toml 为准。

`mathscape-tool-lean` 已具备第一切片的项目配置、受管 Lean/mathlib profile、环境探测、环境指纹和 argv-safe runner；并已实现受 fingerprint 约束的声明 exporter 与本地 Loogle adapter。探测在缺少 `lake-manifest.json` 时不会调用可能创建 manifest 的 `lake env`，以保持用户工程只读。当前开发 profile 固定 Lean 4.33.1 与 mathlib v4.33.1，profile metadata 和编译缓存位于 Mathscape 管理目录。`lean.check`、证据回执及 MCP 接入仍未实现。`mathscape-validation`、`mathscape-fixture` 仍是模块占位；校验实际位于 domain、orchestrator 和 store 中。CLI 也是 scaffold，尚非完整操作入口。

## 当前主流程

```text
本地 PDF → Managed Artifact → PDF 选区 → Marker → Managed Markdown 与派生产物
                                                     ↓
Markdown 选区 → 后端重建输入 → A 单元发现 → 确定性验证 → 冻结 UnitSet
                                                     ↓
                         任务规划 → B 来源事实发现 → 验证与有界修订
                                                     ↓
                            检查点落库 → 审核 → 重验证 → 接受计划 → 来源登记
                                                     ↓
                                    材料图 / 来源图 / 景观图
```

A 识别单元、范围、parent、完整性、issues 和 coverage；不产生标签、引用或内容关系。B 在冻结目录上提取原文标签、标题、引用和明示关系，以任务 disposition 表达处理状态。生成标题与原文标题必须区分。

Planner 分配事实 owner 和可提取范围；每批 B 仍可阅读完整授权选区。严格嵌套的后代拥有自己的内容，父任务不能重复占有。B 不能修改 A；边界问题保留为需修订状态。A 到 B 自动交接，不以人工接受为前置条件。

工作流限制调用数、批次和总期限；A 最多一次完整单元修订，B 每个原始批次最多一次有界修订。失败任务不能抹去成功兄弟任务。未知 token usage 不能当作零消耗；token 预算是后续调用门槛，不是远端逐 token 硬限额。流式文本仅供观察，完整响应才进入验证。

当前 Desktop 在工作流返回或取消后保存检查点，未实现每阶段实时落盘、崩溃恢复、历史运行浏览或从磁盘续跑入口。同一内存 workflow 可显式续跑。活动任务锁覆盖 A/B/修订，切换阅读页不释放锁；取消会丢弃在途 future，再保留已有结果。

当前接受入口是人工选择；设置中的 automatic candidate 不等于已经自动登记。已实现部分结果接受与正式来源登记，未实现提案编辑、拒绝意见、自动补上下文和跨文献语义对齐。

## 视图边界

SQLite 的 Artifact Catalog 与 Source Map 通过同一读事务组成景观快照。来源链可表达 `SourceSemanticUnit → SourceSpan → Markdown ← ConversionRun ← PDF`，转换溯源并不提供精确 PDF 位置。

手写采用另一条输入路径：桌面画板或 HTTPS InkPad → IndexedDB outbox → Ink 应用服务 → SQLite 操作历史 → 固定前缀快照 → Artifact Pool → Ink SourceSpan。快照创建先持久固定请求及前缀，来源选区由后端重绑定受管字节，再经 orchestrator 构建 SourceSpan 和原子登记回执；不经过发现模型或自动创建数学解释。景观图的 `located_in` 连接 Ink 证据与快照，专用只读 Reader 支持选区定位和返回当前可编辑文档。

P3 的 Axum HTTPS/WSS 服务属于 DesktopService，静态画板随应用打包；短时单次二维码换取指定手稿的设备租约，持久请求复用本地写入验证及 durable ACK。配对后电脑只读镜像，预览不落历史，SQLite 前缀用于快照。断线保留 epoch，显式接管才转移写入权；平板持有时，转交二维码在新设备领取的同一事务中撤销旧凭据，原平板在此之前可继续同步。首次设置可绑定手稿：HTTP 设置码只用于公开证书与检测，书写衔接码只在 URL fragment 和 HTTPS 请求体中出现，由它换取已有凭据恢复或一次性配对；旧队列可导出并导入独立恢复草稿。网络不开放通用 Tauri 命令或 Workspace 文件读取。服务可恢复不等于设备已信任 TLS，正式端到端验收还需目标平板验证。

`GraphProjection` 保留第一类超边及带角色端点；`IncidenceGraph` 把超边展开为关系节点与连接边。端点方向由 Rust 显式给出，前端不能从角色名称猜测或自行折叠关系。渲染 ID 确定性派生，petgraph 内存索引不能用作持久身份。

分组、边界节点、布局和选择状态属于视图，不改变领域身份。审核投影也是只读 DTO：不包含完整模型请求和原始输出，不授予登记权限。图不是第二套权威数据库。

材料与来源投影在景观图中联结，前端以统一地图及材料/来源等过滤选项浏览；显式证据模式展开 SourceSpan。内容单元详情可预览渲染后的原文并跳到 Markdown 锚点。活动地图默认排除退役对象，显示退役选项查看当前 tombstone，登记快照则读取回执绑定的历史 revision；过滤和隐藏不删除来源记录。

## 代码入口与探索范围

- 调用链：[Desktop discovery](../apps/mathscape-desktop/src/discovery.rs)、[workflow](../crates/mathscape-orchestrator/src/discovery_workflow.rs)、[Planner](../crates/mathscape-orchestrator/src/fact_planner.rs)。
- 数据到视图：[query](../crates/mathscape-query/src/lib.rs)、[graph](../crates/mathscape-graph/src/lib.rs)、[审核投影](../crates/mathscape-orchestrator/src/source_review.rs)。
- 版本化任务：[prompts](../prompts/README.md)。单阶段 v3 留作离线回放与比较，非 Desktop 默认路径。

共享数学实体、DialectScope、证明状态、R/D/T/X 构建及 Lean 验证尚未形成端到端应用能力。新的 Agent 架构仍待验证；当前 A/B 流程是实现现状，不是未来数学理解必须遵循的顺序。
