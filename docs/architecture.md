# 架构

Mathscape 是面向数学学习者和研究者的本地优先桌面应用。当前实现聚焦材料摄取、来源发现、人工审核与来源图；数学语义解释和跨领域关联是目标，不能把当前来源抽取能力等同于该目标已经实现。

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

`mathscape-tool-lean`、`mathscape-validation`、`mathscape-fixture` 目前只有模块占位；校验实际位于 domain、orchestrator 和 store 中。CLI 也是 scaffold，尚非完整操作入口。

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

`GraphProjection` 保留第一类超边及带角色端点；`IncidenceGraph` 把超边展开为关系节点与连接边。端点方向由 Rust 显式给出，前端不能从角色名称猜测或自行折叠关系。渲染 ID 确定性派生，petgraph 内存索引不能用作持久身份。

分组、边界节点、布局和选择状态属于视图，不改变领域身份。审核投影也是只读 DTO：不包含完整模型请求和原始输出，不授予登记权限。图不是第二套权威数据库。

材料与来源投影在景观图中联结，前端以统一地图及材料/来源等过滤选项浏览；显式证据模式展开 SourceSpan。内容单元详情可预览渲染后的原文并跳到 Markdown 锚点。活动地图默认排除退役对象，显示退役选项查看当前 tombstone，登记快照则读取回执绑定的历史 revision；过滤和隐藏不删除来源记录。

## 代码入口与探索范围

- 调用链：[Desktop discovery](../apps/mathscape-desktop/src/discovery.rs)、[workflow](../crates/mathscape-orchestrator/src/discovery_workflow.rs)、[Planner](../crates/mathscape-orchestrator/src/fact_planner.rs)。
- 数据到视图：[query](../crates/mathscape-query/src/lib.rs)、[graph](../crates/mathscape-graph/src/lib.rs)、[审核投影](../crates/mathscape-orchestrator/src/source_review.rs)。
- 版本化任务：[prompts](../prompts/README.md)。单阶段 v3 留作离线回放与比较，非 Desktop 默认路径。

共享数学实体、DialectScope、证明状态、R/D/T/X 构建及 Lean 验证尚未形成端到端应用能力。新的 Agent 架构仍待验证；当前 A/B 流程是实现现状，不是未来数学理解必须遵循的顺序。
