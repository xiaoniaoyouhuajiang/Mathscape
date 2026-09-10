# 分阶段来源发现开发任务

状态：M1（D1～D3）、M2（D4～D6）已完成；D7～D8 待实现。协议依据见
[分阶段发现协议](discovery-staged-protocol-draft.md)。本计划先打通可回放的两阶段发现，再接
Desktop；来源登记、SQLite 图写入、DialectScope 与全局对齐不作为这次改造的前置条件。

## 1. 当前工程可复用的部分

| 位置 | 已有能力 | 这次的处理 |
| --- | --- | --- |
| mathscape-domain/src/discovery.rs | Markdown 快照、用户选区、DiscoveryInput | 保留根输入契约；新增独立的阶段任务输入 |
| mathscape-domain/src/discovery_proposal.rs | v3 提案、类型/关系/完整性枚举 | 保留 v3；新增阶段 DTO，复用公共枚举 |
| mathscape-orchestrator/src/discovery_input.rs、markdown_anchors.rs | 可信选区构建、物理块和 UTF-8 回链 | 复用；额外支持事实任务的片段授权 |
| selector_resolver.rs | bN 绑定、安全规范化、quote 消歧 | 提取可复用定位核心；增加 text_occurrence 与候选列表 |
| discovery_validation.rs | 归属、parent、覆盖、关系端点、引用校验 | 拆成定位、单元、事实验证函数；保留旧 v3 入口 |
| discovery_prompt.rs、discovery_producer.rs | Schema 生成、版本化 Prompt、单次流式调用和运行摘要 | 新增两种具体任务；复用单次响应处理，不把循环放进 Producer |
| mathscape-llm | provider/profile、凭据、流式输出、停止原因 | 继续复用；核实断流分类能支持新工作流 |
| apps/mathscape-desktop/src/discovery.rs | run_discovery、全局活动锁、Tauri 结果映射 | 服务入口改调工作流，锁覆盖所有阶段和修订 |
| frontend/src/reader/selection.ts、discovery-task.ts | 选区、流式浮窗、结果和错误展示 | 增加阶段进度与 partial/needs_attention 展示 |

当前 SQLite 已有 Artifact/Conversion 及配置设施，但没有完整发现运行、提案、接受和来源登记
存储。阶段交接先使用内存检查点与离线回放，不能把现有目录中的 JSON 当成已实现的持久恢复。

## 2. 最小可执行任务与验收

按 D1→D7 顺序完成每个可独立检查的提交；D8 是单独的耐久化里程碑。

| 任务 | 具体产出 | 验收条件 |
| --- | --- | --- |
| D1 契约与实例 | Domain 新增 UnitDiscoveryProposal、SourceFactProposal、TextOccurrence、阶段输入/结果及修订类型；两份 Prompt/manifest/Schema | 示例可往返；B 不能携带单元修改或重复 source；v3 样例仍可走原路径 |
| D2 定位与诊断 | Resolver 支持单次文本、跨块逃生分支、候选 ID；提取共享范围验证；独立 preflight | 精确/安全规范化/重复/越界各有明确结果；只选择候选不扩大权限；不可解析位置不阻止独立自指诊断 |
| D3 单元阶段 | A 输入构建、单次 producer、unit validator、冻结 UnitSet | 整块与块内切分均可验证；parent 与物理包含分别处理；非法重叠不能进入 B；partial 覆盖可见 |
| D4 事实任务构建 | FactTask planner、证据/上下文片段、按预算分批和请求绑定 | 每处直接内容有明确 owner；不凭最小范围猜同层归属；片段与 occurrence 口径稳定；同批文本可复用 |
| D5 事实阶段与组装 | B producer/validator、规范标签/引用、保守本地查找、DiscoveryResult | source 从 task_ref 注入；重复出现保留；自指提示被拒；外部引用可 unresolved；失败任务不冒充空结果 |
| D6 工作流与一次修订 | 两阶段协调器、每批检查点、候选选择/任务替换、总期限和调用上限 | 未修订的 A 不进入 B；B 失败不重跑成功批次；超限结束并保留诊断；修改 UnitSet 后旧 B 失效 |
| D7 Desktop 接入与多案例比较 | 原 run_discovery 调新工作流、活动锁/流式事件/结果适配、阶段卡片 | 两阶段间不允许重复提交；切页任务仍可见；partial、调用失败和验证失败区分；运行结束未隐式入图 |
| D8 运行耐久化 | SQLite 运行/attempt/检查点索引，受管产物保存输入输出和诊断，恢复入口 | 重启可查看/续跑未完成任务；恢复核验指纹/版本/预算；不重复发送已成功请求 |

D1 先按普通具体类型落地，不建立通用 agent SDK 或动态 DAG 框架。D3/D5 可各有一个薄 Producer；
共享响应读取、限额、解码与记录逻辑即可，不需要先抽象任意工作流引擎。

D4 的任务片段绑定不能直接照搬按物理 block ID 去重的现有实现：一个块可能被分割后分给两个
任务。同批句柄应绑定实际片段；原始物理块 ID 和字节坐标在内部保留。

D5 的本地目标解析首版只支持明确的简单编号语法及明确作用域。复杂标签、作用域未知或组合
范围均可 unresolved；先保证结果诚实，不为了闭合图实现一套通用数学引用解释器。

D6 的前置检查应核实流结束标志与取消传播。没有正常终止事件的 EOF 必须作为传输未完成
处理，不能拼接残缺 JSON 去做语义修订；只需补齐当前 provider 适配器的必要检查。

D7 开发默认路径可切换到新方案，但保留 v3 的离线回放入口。输出界面使用“识别单元”“发现
标签与关系”等任务名称，不向普通用户展示内部 DTO 版本和局部句柄协议。

## 3. 阶段里程碑

**M1（D1～D3）：单元识别可独立运行。** 用 ScriptedLlmBackend 和自编 Markdown 验证边界、
父链、覆盖与冻结。此时不接第二阶段，也不因暂缺 source_labels 而伪造 v3 响应。

实现记录：Domain 已增加阶段 A/B、TextOccurrence、修订与组合 envelope；两个阶段的 Prompt、
manifest 和 Schema 已版本化。公共定位器支持单次出现、安全排版规范化、跨块逃生范围和受
请求/提案/字段约束的候选选择。UnitDiscoveryProducer 可单独流式调用、验证并冻结 UnitSet；
阶段 B 目前只实现位置无关 preflight，符合 M1 边界。

以上为 M1 交付时的范围；M2 已继续完成完整事实发现，见下文。

**M2（D4～D6）：两阶段离线闭环。** 能输入同一 Markdown、回放两份模型输出、组合结果，
并用脚本化错误证明候选修订只调用一次。开发期可将调试产物显式导出到工作区下的临时目录；
产物保留/删除遵循项目既有习惯，测试资产不得依赖开发者电脑的绝对路径。

M2 已完成：完整授权原文进入每个 B 批次、边界分片与 owner 分配、事实 Producer/验证、
标签与本地引用组装、一次有界修订、总期限/调用上限、内存检查点与显式续跑；详见
[M2 实现与使用](discovery-staged-m2.md)。原文自动扩展、持久恢复和 Desktop 接入仍为后续工作。

**M3（D7）：Desktop 真实模型比较。** 复用已有 profile 和 Marker 产物；记录每阶段总调用数、
token、耗时与诊断，再决定默认批次大小和是否调整 Prompt。两阶段交接不要求人工批准，
最终审核策略继续来自 Workspace 设置。

**M4（D8）：运行可恢复。** 在实验协议稳定后增加持久恢复；受管产物路径相对 Workspace，
与当前 Artifact Pool 的可搬迁要求一致。运行保存与来源图登记仍是两项不同事务职责。

## 4. 回归矩阵与比较方法

| 小案例 | 主要验证点 |
| --- | --- |
| 命题与完整证明，分别占块 | A 范围、B 隐含 source、proves 类型 |
| 一个块内含命题结尾与证明开头 | A 必须精确切分，不能两个完整块范围重叠 |
| 同块两次引用定义 2 | null 歧义、两个 occurrence、候选修订 |
| 公式中带 TeX 间距的标签 | 安全规范化、准确原文恢复，数学符号不等价 |
| 同名编号出现在不同章节/不同类型中 | 不仅凭字符串唯一或就近位置绑定目标 |
| 章节含命题、命题含证明或多个同范围角色 | 任务归属、parent 非物理包含、无法分配时的明确阻塞 |
| 外部、组合与同单元内部引用 | unresolved 仍是有效来源事实，禁止 unit 自边 |
| 部分完成、响应截断、第二阶段超时 | partial 检查点、不误判成功、不隐式无限重试 |

主 CI 用自编小材料，避免教材版权资产和机器路径成为运行前提。真实回归继续使用
`data/fixtures/discovery_debug_1` 及其他已有材料；原 proposal/diagnose 保留不覆盖，缺少外部
资产时明确跳过该可选回归。新阶段输出按独立目录和 manifest 记录来源快照、配置和版本。

比较同一材料/同一模型配置的 v3 和新工作流时，记录：

- A 首次通过率、B 各批首次通过率、整个请求首次通过率和有界修订后通过率；
- 标签/引用定位精度与遗漏率、unit 边界质量、归属错误和错误目标绑定数量（抽样人工核对）；
- 两阶段加修订的合计输入/输出 token、API 调用数、耗时；缺失 usage 记为未知；
- 因不支持交叠或跨块提及而暂停的比例、partial 和 unresolved 的原因分布。

相同配置重复运行若干次；不要用某一次 token 下降或错误数减少推断普遍收益。新版本不得通过
少发现引用、扩大单元范围或把所有结果变成 unresolved 来获得虚高的通过率。允许 unresolved
的正确率与可以成功解析的覆盖率分别评估。

## 5. 文档和版本发布

每个任务更新对应阶段 manifest、Schema 快照和验收实例。数据形状改变升级阶段 contract；
Prompt 文案优化只更新 prompt revision/指纹；工作流交接规则改变升级 pipeline version。
当前 v3 文档和 Prompt 作为历史实现保留，不边调试边覆盖旧版本。

本文是开发分解，新增模块名为建议；实现时可合并体积很小的文件，但职责和验证边界保持一致。
M2 提供独立 Rust 工作流及离线回放示例；Desktop 仍使用 v3。数据库和测试教材未修改。
