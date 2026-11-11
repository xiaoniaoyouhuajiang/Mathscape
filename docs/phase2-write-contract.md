# Mathscape Phase2 契约：写路径（`user.ask` / `propose_changes` / `get_change_set` / 接受 / 回滚）

> 读者：Mob（实现）· 起草：2026-10-30（Asia/Shanghai）· 状态：**契约草案，用户决策已锁定（2026-10-30），待用户审阅本文后合并**
> 依据：[Phase0 契约](phase0-contract.md)（下称 **Phase0**）、[Phase1c 图契约](phase1c-graph-contract.md)（**1c**）、[Phase1d 手稿契约](phase1d-ink-snapshot-contract.md)（**1d**）、[Foundations 扩展草案 v0.1](../proposals/core-workflow-foundations-extension-v0.1.md)（**FX**）、[登记撤回提案 v1](../proposals/discovery-registration-reversal-v1.md)（**REV**）、MCP 工具面草案 `/workspace/research/mathscape-tools-mcp-draft.md`（**MCP 草案**，仓库外，见 Phase0 头注）。
> 版本：本文引入 `mcp_envelope v1` 与 `agent_events v1`（增量见 §4.5、§4.1.4）；Phase1 已有 10 个只读工具的语义与描述除 `scope` 一句外不变。Lean 工具契约另见 [`lean-tool-contract.md`](lean-tool-contract.md)。
> 标签：**[锁定]** = 用户 2026-10-30 决策或由其直接推出；**[锁定·2026-11-09 用户改定]** = PR-B 讨论中用户改定的锁定项，原口径以「原口径」保留对照（汇总见 §2.1）；**[本文提议]** = 起草时为闭合契约补的细节，现有草案未写，实现 PR 可在不违背 [锁定] 的前提下微调（汇总见 §11）；**[待核]** = 真正未定（§10）。
> P2-2 来源层暂存切片已在 `mathscape-mcp` / `mathscape-store-sqlite` 实现；PR-B 已接入 SourceSpan、调查级投影、Desktop 接受 / 丢弃 / 批量审核命令和沙箱提案白名单。`user.ask` 与已接受对象撤回仍是后续切片。
> **2026-11-09 修订（PR-B，`feat/pr-b-staged-proposals`）**：新增 SourceSpan 提案、`$new:` 临时引用、最终图依赖校验、跨 change set 引用与连带规则、暂存上图、会话切换 / 退出时的批量审核，以及沙箱接入（§2.1、§4.2.5–§4.2.8、§4.4.1、§5.5、§6.1、§9.1）。用户随后锁定了退役 span、批量未勾选项、多 Markdown 快照分组和依赖丢弃传播规则（§2.2「2026-11-09 用户定」）；investigation 粒度定为**整场沙箱会话**（`sandbox:{session}`），取代早先的线程级写法。Loogle 加速不在本修订范围，推迟到下一个 PR。
> **2026-11-10 定稿（PR-B 实现对齐）**：按实现写入七条修订——会话级调查、quote 先截取再比较、粒度失败码、hash 语义、依赖带版本且替换后转 blocked、批量一个 commit 且未勾选保持 staged、单一投影及其排除项（汇总见 §2.2）；删除旧口径「线程级调查」「全文搜索式 quote 匹配」「替换后依赖自动改指」。

---

## 1. 状态 / 范围 / 不做什么

**一句话**：Phase2 给 Agent 开一个、且只开一个**正式提案写入口** `propose_changes`——它只**暂存**提案（`staged` / `blocked`），从不写正式图；正式写入只发生在用户在 Desktop **显式接受**某个 change_set 时，经现有 plan/commit 管线与短锁协调器提交。文件字节的受控暂存由独立的 `artifact.stage` 负责；它只写当前 round 的 Workspace staging，不创建正式 Artifact、提案对象或图关系。另补 `user.ask`（追问）、`get_change_set`（回读）和 `scope=investigation`（暂存投影）。

本契约锁定 [锁定]：
1. 并发口径、写入口、校验失败处理、接受、两种回滚、侧车推迟、实现顺序（§2 决策表 1–8）。
2. D1 ActorRef / D2 提案生命周期 / D3 Debt 在 Phase2 的**最小**口径（§3）。
3. 四个 Agent 面能力的输入输出形状（§4）与 Desktop 接受路径（§5）。

**Non-goals（本契约明确不做）** [锁定]：
- **sidecar 离线 / 便携打包**（Phase0 §4 末条、§7）：继续推迟，安装脚本（`scripts/install/`）+ BYO harness 仍是唯一渠道，本文**不**解锁 sidecar 双通道。注意区分：本文 §7.4 所说的新增业务表（暂存提案、追问记录）是普通 SQLite 迁移，**不是** sidecar。
- 完整 EMIR 状态机、TrustRecord 规范对齐与产品化 Debt 管理（Phase2-0 只做桩语义，§3）。
- 语义层对象（SemanticEntity / FormalArtifact / Alignment / ProofContext 等）的**可接受**写入：Phase2-0 的 `propose_changes` 只能编译到现有来源图操作（§4.2.3）。
- 内嵌官方 CLI（Codex / Claude Code / Lean / Marker）——沿用 Phase0 §7。
- 多调查提案的自动合并、自动三方合并、自动接受（§7.3）。
- 把通用 Desktop `commit_*` / `plan_*` Tauri 命令开放给 Agent（FX §7.1）。
- 通用 undo/redo（REV §1）；物理删除。

**相关 / 并行（不在本 PR 范围）**：Phase0 清单 4b（登录态真机 Codex `workspace.search → material.read` + Desktop 消费事件）与 Phase1 硬化工作与本文并行推进，**本 PR 不实现、不修改**它们。Phase2 的 `user.ask` 桥（§4.1）依赖 4b 打通的 Desktop 事件消费，见 §9 依赖说明。**沙箱 Flow**（[`sandbox-flow.md`](sandbox-flow.md)）是定向写路径产品化之前的可拆卸探索床，实现顺序上可先于本文 §9；不修改本文锁定决策。

---

## 2. 用户锁定决策（2026-10-30）[锁定]

| # | 决策 | 本文落点 | 与既有草案的关系 |
| --- | --- | --- | --- |
| 1 | **并发**：「不并行」= **单个调查会话内串行**，不是全局单流程锁。同一正式图上可并存多个调查 / 候选。手工 / 平板编辑走同一个短锁协调器。调查进入 `awaiting_user`（如待答 `user.ask`）时**释放该调查的写槽**，会话不被卡死。 | §7 | **覆盖** FX §3 A2「一个 Workspace 同时只有一轮活动调查」与 FX §8.2「启动另一轮调查：拒绝」；保留 FX §3 A3 / §8.1 的「探索期不持写锁、提交期短锁 + OCC」。 |
| 2 | **写入口**：Agent 唯一写入口是 `propose_changes`。服务端校验 → `ProposedChangeSet`，状态 `staged` \| `blocked`。永不向 Agent 暴露通用 Desktop `commit_*` Tauri 命令。 | §4.2、§5.4 | 对齐 MCP 草案 §0.2、§3；FX §4.4、§7.1。 |
| 3 | **校验失败**：留下 **Debt + diagnostics + 提案指纹**；不静默写正式图；不无痕清空整份提案。Agent 可对照指纹修订后重新提议。 | §3.3、§4.2.4 | 对齐 FX §4.4「草稿只能在提案诊断中出现」。 |
| 4 | **接受**：仅用户**显式**接受；**默认关闭自动接受**。接受绑定**提案版本 + 计划指纹**。接受粒度 = **change_set**。接受与调查流程结束**分离**（一次调查内可多次接受）。**2026-11-09 用户改定**：审批时机放宽为「随时可审 + 会话切换 / 退出应用时用一个批量审核界面统一决定」，见 §2.1 #4′、§5.5；显式接受、无自动接受、change_set 整批接受三条不变。 | §5、§5.5 | 对齐 FX §4.4、§12.2「能否自动接受」行；MCP 草案 §3.2「自动接受首版不允许」由此**定案**。 |
| 5 | **两种回滚分开**：未提交暂存 → 丢弃 / 替换提案（廉价）；已接受登记 → 经现有 discovery_registration_reversal 式原语退役 / 级联，**不是**普通 undo，留审计；若会留下非法悬挂节点且未级联则**禁止**。 | §6 | 对齐 REV §1、§3.1、§6。 |
| 6 | **Sidecar**：推迟（Phase0 B）。安装脚本 + BYO harness 保留；便携 sidecar 双通道**不在**本契约解锁。 | §1 Non-goals | 沿用 Phase0 §4、§7。 |
| 7 | **契约锁定后的实现顺序**：`user.ask` → `propose_changes`（仅暂存）+ `get_change_set` → Desktop 接受面板 → 退役 / 回滚；真正的 `scope=investigation` 投影与 propose 同批或紧随其后。 | §9 | 替代 MCP 草案 §5 第 8–9 项的顺序描述。 |
| 8 | **4b / Phase1 硬化**与本文档工作并行，但**不在本 PR 范围**，仅在「相关 / 并行」中提及。 | §1 末 | — |

### 2.1 2026-11-09 用户改定（PR-B）[锁定·2026-11-09 用户改定]

背景：用户在 PR-B 讨论中指出，一轮对话里多次 `propose_changes` 若逐个审批，交互负担过重；Mob 早先的副作用模型是「会话副作用直接显示在地图上、会话切换或退出前统一决定」。用户认可折中方案（2026-11-09）：**暂存对象上图（未审批样式）+ 会话切换 / 退出时批量审核**，正式 head 在接受前不动，接受仍是显式的、按 change_set 整批的。

| # | 改定项 | 原口径（2026-10-30） | 新口径（2026-11-09） | 落点 |
| --- | --- | --- | --- | --- |
| 4′ | 审批时机 | 用户对单个 change_set 显式接受；没有「接受全部调查」快捷方式（§5.1 第 2 步） | 仍只有显式接受、默认无自动接受、change_set 内不拆分。**新增**：会话切换 / 退出应用时弹出**一个批量审核界面**，列出本调查所有活动 version，默认全选、按依赖分组、展示影响预览；取消勾选一项时依赖它的项自动连带取消；用户点一次「接受所选」完成。中途也可随时打开审核。批量范围仅限**本调查**，仍不提供跨调查的「全部接受」 | §5.5 |
| 9 | 跨 change set 引用暂存对象 | 禁止：`local_ref` 只在本 version 内有效，不得只指向其他 change_set 的暂存对象（§4.2.4 第 2 条） | **同一调查内允许**引用其他 change_set 的 `staged` 对象（按其 `staged_ids` 稳定 id）；接受 B 要求它依赖的 A 已接受或同批接受；丢弃 / 替换 A 时依赖 A 的项连带处理（§4.2.8）。跨调查仍不可见、不可引用 | §4.2.4、§4.2.8、§6.1 |
| 10 | SourceSpan 提案 | Phase2-0 不支持新建 SourceSpan（§4.2.3 表末行、§10 待核 1） | 新增 `source_span/create` → `CreateSourceSpan`；Agent 只给定位，服务端校验并补全（§4.2.5） | §4.2.3、§4.2.5 |
| 11 | 暂存上图 | `staged` 只出现在 Agent 的 `scope=investigation` 投影中；Desktop 地图只显示已接受快照 | Desktop 地图也显示本调查的 `staged` 对象（「未审批」样式），Agent 可在自己的暂存对象上继续构建；正式 head 在接受前不动 | §4.4.1 |

不变的锁定项（明确列出，防止误读）：Agent 唯一写入口仍是 `propose_changes`（决策 2）；校验失败留 Debt + 指纹（决策 3）；接受不提升 Trust、接受 ≠ 数学正确（§3.3）；接受时按 OCC 检测冲突、不自动合并（§5.2、§7.3）；撤回走退役 / 级联（决策 5）。

### 2.2 PR-B 定稿：用户定的四项与七条修订 [锁定·2026-11-09 用户定 / 2026-11-10 定稿]

**2026-11-09 用户定**（原 §10 待核 4–6 与丢弃传播）：

| # | 用户定 | 落点 |
| --- | --- | --- |
| U1 | 同 id 的 span 在正式图上**已退役** ⇒ `span_inactive`，version `blocked`；不自动重新激活，重新激活另走显式操作 | §4.2.5 |
| U2 | 一个 change_set 可涉及多个 Markdown 快照；审核面板**按快照分组**展示，每个 SourceSpan 保留自己的 artifact / range | §5.5、§5.6 |
| U3 | 丢弃 A 时，依赖 A 的项**默认转 `blocked`**（`dependency_discarded`，传递依赖者 `dependency_blocked`）；界面提供「连带丢弃」选项，选中后依赖闭包一并 `discarded` | §4.2.8、§6.1 |
| U4 | 混合模型：暂存上图 + 批量审核；批量审核中**未勾选项保持 `staged`**，可稍后再审；沙箱副本整体丢弃时随副本删除 | §5.5 |

**七条修订**（2026-11-10 按 PR-B 实现定稿；原文措辞若与下表冲突，以下表为准）：

| # | 修订 | 要点 | 落点 |
| --- | --- | --- | --- |
| R1 | 会话级调查 | 沙箱 investigation = `sandbox:{session}`，整场会话的所有线程与 round 共享；`thread_id` 只是元数据。服务端登记 round↔investigation（`investigation_rounds`，先写者为准），不一致 ⇒ `forbidden` / `investigation_mismatch` | §7.1 |
| R2 | quote 先截取再比较 | 先按 `[start,end)` 截出原文，再与 `quote` 比较：先精确，再两侧同样归一化后比较；**不在**快照全文里搜索 | §4.2.5 |
| R3 | 粒度失败码 | `block` 粒度但范围内没有 canonical block ⇒ `block_locator_unavailable`；`page` 粒度但不能唯一映射到一页 ⇒ `page_mapping_unavailable`；都 blocked，不静默降级 | §4.2.5、§8 |
| R4 | hash 语义 | span 文本 hash 与内容发现同一函数：`source_text_hash(text) = fingerprint(&text)`（JSON 序列化后取 sha256）；span id = `markdown_span_id(artifact, range)` | §4.2.5 |
| R5 | 依赖带版本、替换后转 blocked | 依赖记录 `A@version` 与被引用 id；A 被 `supersedes` 后，直接依赖者转 `blocked`（`dependency_superseded`），更深的转 `blocked`（`dependency_blocked`）；**不自动改指**新 version，由 Agent 重提 | §4.2.8 |
| R6 | 批量一个 commit，未勾选保持 staged | 选中集合（含依赖闭包校验）合成**一个** `UserChangeRequest`、一个计划指纹、一个 `BEGIN IMMEDIATE` 事务、一个 commit；每个 change_set 的接受决定记同一 `commit_id`。未勾选项保持 `staged` | §5.5 |
| R7 | 单一投影及其排除项 | `graph.get`、`graph.neighborhood` 与 Desktop 地图共用同一份服务端投影；暂存对象不进正式 head、正式快照 / landscape、导出、默认搜索、正式编辑入口和母工作区地图 | §4.4、§4.4.1 |

**2026-11-10 沙箱对话 3 验收后的修订**（用户定；与上文冲突时以下表为准）：

| # | 修订 | 要点 | 落点 |
| --- | --- | --- | --- |
| V1 | 只有本提案触及的对象出错才阻塞 | 手稿笔迹 span（`ink.snapshot_artifact` + 非空 `stroke_ids`）算有物理定位，不再报 `invalid_locator`（仅认作合法定位，不进 source payload）。提案、影响预览、批量计划 / 接受与调查投影改用 `plan_map_change_over_head`：本提案及其依赖闭包既未操作、也未改动的对象上，head 里**本来就有**的问题移入 `ImpactPlan.head_issues`，在 `propose_changes` 里是 `warn: head_integrity`（`target_id` = 该对象），不阻塞、不进 Debt；本提案在未触及对象上**新引起**的问题仍阻塞。所有诊断带 `target_id`。手动地图编辑仍用严格的 `plan_map_change`（不做隐式修复） | §4.2.4、§8 |
| V2 | 按页读的字节段 | `material.read` 按 `pdf_pages` 读时，单快照也返回 `details.served_segments`（所读页在快照里的 `md_byte_range`；正文从快照第 0 字节起）。多页快照没有分页标记 ⇒ 诊断 `page_markers_absent`，只能用 `inline` 粒度；用户导入或其他来源的 Markdown 没有分页标记属正常情况，不放宽重解析规则 | §4.2.5 |
| V3 | `material.locate` 与 span 长度上限 | 新只读工具 `material.locate {artifact_id, quote, within?}`：按 R2 同一规则搜索（先精确，否则两侧归一化后比较并映射回原始字节），返回的每个范围与同一 `quote` 必过 R2；唯一命中给 `md_byte_range`、块 id、能唯一确定时的页；多处命中 `ambiguous` 列出候选（上限 20）。单个 span 最长 4096 字节，超过 ⇒ `span_too_long`。R2 的截取后比较不变。`tool_schema_version` = `mathscape-mcp/15` | §4.2.5、§4.6 |
| V4 | 找回并修订本调查的提案（第三轮验收） | 新只读工具 `list_change_sets {pending_only?, limit?}`（§4.3.1）：只列本调查的 change_set，新的在前，每项带活动 version 的状态、阻塞码和可直接放进 `propose_changes` 的 `supersedes`。原因：对话历史只回放文字，Agent 拿不到旧提案 id，重提只能新建 change_set，旧的 blocked 版本留在队列里。`tool_schema_version` = `mathscape-mcp/16`，skill v0.6.4 要求修订前先列出再带 `supersedes` | §4.3.1、§4.2.8 |
| V5 | 页粒度与 `page_markers_absent`（第三轮验收） | `material.locate` 给出唯一页（`page_granularity_available` = true）时，skill 要求用 `page` 粒度，只有拿不到页才用 `inline`，且不得声称未存下的页码；`propose_changes` 对能唯一落到一页的 `inline` span 给 `info: page_granularity_available`（不阻塞）。`material.locate` 在多页且无分页标记的快照上给 `page_markers_absent`（与 `material.read` 同码），其他落不到唯一页的情形给 `page_unresolved`（附原因） | §4.2.5、§4.6 |

---

## 3. D1 / D2 / D3：Phase2 契约级口径

Phase0 §1 把 D1–D3 推迟到「Phase2 写路径前再定」。本节只定写路径**必需**的最小语义；完整 EMIR / TrustRecord 产品仍推迟。

### 3.1 D1 ActorRef [锁定语义 / 本文提议格式]

现状 [事实]：`ActorRef` 是非空文本 newtype（`crates/mathscape-domain/src/id.rs`，`non_empty_text_type!(ActorRef, "actor")`），现有值如 `mathscape-desktop`、`mathscape-desktop-user`（`apps/mathscape-desktop/src/map_edit.rs`、`discovery.rs`、`ink.rs`）。

Phase2 **锁定**的语义：一个 change_set 区分两个身份，**都由服务端/适配器写入，Agent 不可自报**（MCP 草案 §0.4、§3.2「身份字段」行；FX §8.4.2）：

| 角色 | 字段 | 写入者 | 值 [本文提议格式] |
| --- | --- | --- | --- |
| 提议者 | `proposed_by` + `round_id` + `generation` | 适配器注入（来自 `--round` 绑定） | `agent:<backend>`，`<backend>` 取 Phase0 §5 `AgentBackend` 的 snake_case（`codex` \| `claude_code` \| `hmz` \| `custom`） |
| 接受者 | `accepted_by` | Desktop 接受命令 | 沿用现有 `mathscape-desktop-user` |
| 退役 / 丢弃者 | `decided_by` | Desktop | 同上 |

- 正式 revision 的 `created_by` = **接受者**（实际执行提交的 actor，与 `commit_map_change` 中「`patch.author() == actor`」检查一致）；提议来源写入 `map_object_revisions.generation_run`（已有列），值为 `change_set:<change_set_id>@v<version>` [本文提议]，从而 revision → change_set → `proposed_by` / `round_id` 可追溯。
- 不迁移 `ActorRef` 类型、不改已有值；格式只是约定字符串。

### 3.2 D2 提案生命周期（`ProposedChangeSet`）[锁定最小集 + 本文提议补充]

术语：**change_set** 是一个提案的身份（`change_set_id` 稳定）；每次 propose / 重新 propose 产生一个不可变的 **version**（FX §4.4「不可变提案版本」）。状态挂在 version 上。

| 状态 | 含义 | 进入方式 | 是否终态 | 来源 |
| --- | --- | --- | --- | --- |
| `staged` | 校验通过，等待用户决定；可出现在本调查 `scope=investigation` 投影中 | `propose_changes` 校验通过 | 否 | [锁定] |
| `blocked` | 校验未通过；带 Debt + diagnostics + 指纹；**不**进入投影；不可接受 | `propose_changes` 校验失败 | 否（可被新版本替代） | [锁定] |
| `accepted` | 用户已显式接受，已写正式图，带 `CommitReceipt` | Desktop 接受（§5） | 是（除非退役） | [锁定] |
| `discarded` | 用户丢弃未提交暂存；记录保留，不写正式图 | Desktop 丢弃（§6.1） | 是 | [锁定] |
| `superseded` | 被同一 change_set 的更新版本替代 | Agent 带 `supersedes` 重新 propose（§4.2.4） | 是 | [本文提议]（MCP 草案 §3.3 已列 `superseded`） |
| `withdrawn` | 已接受的 change_set 被退役 / 级联撤回，留审计 | Desktop 撤回（§6.2） | 是 | [本文提议]（对应 REV §5.1 `RegistrationState::Withdrawn`） |

转换（其余一律拒绝）：

```
propose ──► staged ──accept──► accepted ──withdraw──► withdrawn
   │          │ └──discard──► discarded
   │          ├──re-propose──► superseded（新 version 为 staged|blocked）
   │          └──dependency_invalidated──► blocked   ［2026-11-09 新增，§4.2.8］
   └──► blocked ──re-propose──► superseded（新 version）
                └──discard──► discarded
```

- [本文提议，2026-11-09 新增] `staged → blocked` 只由服务端在**依赖失效**时执行（所依赖的 change_set 被丢弃、被替换后不再提供被引用对象、或被引用对象在正式图上已退役），并附 `validation` Debt（`dependency_discarded` / `dependency_superseded` / `dependency_withdrawn`）。原内容、指纹保持不变；Agent 用 `supersedes` 重提。原口径中 `blocked` 只能由 `propose_changes` 产生。
- 批量接受（§5.5）对每个被选中的 change_set 分别走 `staged → accepted`，在同一事务内完成；不引入「批」这一新状态。

- 每个 change_set 同一时刻至多一个**活动 version**（`staged` 或 `blocked`）；接受只能针对活动 version。
- MCP 草案 §3.3 的 `rejected` 在本文统一为 `discarded`（用户拒绝 = 丢弃，留痕）。
- 这是**提案存储状态**，不等于对象的 `LifecycleStatus`；不得把它与 `LifecycleStatus::Candidate` / `curation` 混成一个「候选」开关（FX §4.4 末段）。
- 「调查结束」不改变任何 change_set 状态：结束时仍 `staged` 的提案保留，用户之后仍可接受或丢弃（决策 4「接受与流程结束分离」）。

### 3.3 D3 Debt / TrustRecord（桩语义）[锁定最小集]

- **Debt** 是挂在 change_set **version** 上的结构化未决项，不是正式图对象（Phase2-0 不进图、不进投影）。最小字段 [本文提议形状]：
  ```jsonc
  { "debt_id": "…", "kind": "validation|unresolved|agent_declared",
    "code": "dangling_ref|kind_not_supported|stale_revision|quote_mismatch|dangling_new_ref|dangling_staged_ref|unused_span|dependency_discarded|…",
    "local_ref": "c1"|null, "target_id": "…"|null, "message": "…" }
  ```
  - `validation`：服务端校验失败产生；任一 `validation` Debt 存在 ⇒ version 为 `blocked`（决策 3）。
  - `unresolved`：来自输入 `unresolved[]`（MCP 草案 §3.1），不阻塞。
  - `agent_declared`：来自 `proposed_objects[].debts`（MCP 草案 §3.1），不阻塞，接受面板必须显示。
- `staged` version 也可以带非阻塞 Debt；接受时 Debt 原样随 change_set 记录留存（不写入对象 payload）。
- **TrustRecord**：Phase2-0 不新增可信状态。Agent 不得在 payload 中自设 `accepted` / `kernel_verified` / `canonical` 等终态（MCP 草案 §3.2），出现即 `validation` Debt `forbidden_trust_claim`。接受**不**提升数学可信状态（同 REV §6「不提升数学可信状态」）。

---

## 4. Agent 面工具

公共约定 [锁定]：
- `investigation_id` / `round_id` / `generation` 由适配器经 `mathscape mcp --workspace <path> --round <token>` 注入，**不在** Agent 输入 schema 中（Phase0 §2.3 末条；MCP 草案 §0.4、§4）。MCP 草案 §3.1 输入里的这两个字段在本文删除。
- Phase2 起写类工具（`user.ask`、`propose_changes`）**必须**校验 `--round` token：未绑定调查、generation 已失效（取消 / 暂停 / 进入 `awaiting_user` 后）→ 调用错误 `forbidden`（Phase0 §2.2 的 `invalid_arg|forbidden|internal`；FX §8.4.2「迟到结果隔离」）。只读工具继续不校验（Phase0 §2.1）。
- `tools/list` 追加顺序 [本文提议]：在现有 7 个工具之后依次追加 `user.ask`、`propose_changes`、`get_change_set`；已有描述不变，仅 `workspace.search` / `graph.get` / `graph.neighborhood` 中 `scope` 一句随 §4.4 更新（Phase0 §2.1「描述文本固定」——这是一次有版本号的变更）。

### 4.1 `user.ask` [锁定语义]

#### 4.1.1 PendingQuestion

所有追问——无论来自 Mathscape MCP 工具 `user.ask` 还是 harness 原生通道（Codex `item/tool/requestUserInput`）——**收敛为同一条 `PendingQuestion` 记录**（MCP 草案 §2.6 实现注意；FX §7.1「用户输入与反馈」行：Agent 不能制造用户回答）。

```jsonc
// PendingQuestion [本文提议字段]
{ "question_id": "…", "investigation_id": "…", "round_id": "…",
  "origin": "mcp_user_ask" | "harness_native",
  "harness_ref": { "backend": "codex", "request_id": "…", "native_question_id": "…" } | null,
  "question": "…", "options": ["A","B"] | null, "ink_requested": false,
  "state": "pending" | "answered" | "cancelled",
  "answer": { "text": "…", "selected_option": "A"|null, "ink_snapshot_ref": "…"|null,
              "answered_by": "mathscape-desktop-user", "answered_at": "…" } | null,
  "created_at": "…" }
```

#### 4.1.2 MCP 工具 `user.ask`

输入（对齐 MCP 草案 §2.6）：`{ "question": "string", "options": ["…"]?, "ink_requested": false }`。

输出 [本文提议，**与 MCP 草案 §2.6 不同**]：**非阻塞**，立即返回

```jsonc
{ "status": "ok", "question_id": "…", "state": "pending",
  "next": "end_turn"   // 固定文本提示：结束本 turn，回答会作为下一 turn 的输入送达
}
```

理由：等待可能持续数小时（FX §8.2 末段），而 MCP `tools/call` 长时间挂起会撞 harness 工具超时，也会让 round 一直占着写槽。MCP 草案 §2.6 的 `answer_text` 输出因此不在 `user.ask` 返回里，改为下一 turn 输入里的回答块（§4.1.3）。

#### 4.1.3 harness 桥与 `awaiting_user`

- **Codex 原生 `requestUserInput`**：当前 `CodexExecutor` 对自由提问用空 `answers` 直接拒绝（`crates/mathscape-agent/src/codex/approval.rs`）。若选项是接受/拒绝且点名了白名单工具，则按接受写回。见 Phase0 附录末条。Phase2-1 改为：适配器创建 `origin: harness_native` 的 PendingQuestion 并**持有**该 server request；用户回答后按 Codex 0.163 形状 `{ "answers": { <questionId>: { "answers": [..] } } }` 写回。若 round 在等待期间被取消 / 进程重启，适配器以空 `answers` 结束该请求，回答改为下一 turn 输入投递（不丢答案）。
- **MCP `user.ask`**：`tool_result` 返回后，Agent 结束 turn；适配器在 `turn_end` 后把调查置为 `awaiting_user`。回答到达时，适配器以新 generation 起下一 turn，输入中带回答块 `{question_id, answer_text, selected_option, ink_snapshot_ref}`。
- **进入 `awaiting_user` 时** [锁定]：撤销该调查当前 generation 的执行授权 = **释放该调查的写槽**（FX §8.2 末段「进入 awaiting_user 前保存检查点并撤销本轮执行授权」）；期间该调查发出的任何 `propose_changes` 返回 `forbidden`。其他调查、手工编辑、接受 / 丢弃操作均不受影响（决策 1）。
- **回答到达** [锁定]：重新取得新 generation（写槽）后继续；不自动抢占其他调查（FX §8.2「回复到达不会自动抢占」）。
- **Desktop 回答 UI** [锁定语义]：调查面板列出 `pending` 问题，支持文字 / 选项 / 选择手稿快照（`ink_requested`，引用快照 id，不做 OCR，同 1d §2.2）。回答写 PendingQuestion，`answered_by` 由 Desktop 填。Agent 不能写答案。

#### 4.1.4 事件（`agent_events v1`）[本文提议]

Phase0 §3 规定新增事件类型须改文档并 +1 版本。Phase2 新增 **一个** 类型：

| type | 关键 payload | 何时发 |
| --- | --- | --- |
| `awaiting_user` | `question_id`, `state: entered\|resolved\|cancelled` | 调查进入 / 离开等待用户 |

其余事件不变；`user.ask` 本身仍按 Phase0 §3 镜像成 `tool_call` / `tool_result`。

### 4.2 `propose_changes` [锁定语义]

#### 4.2.1 输入（对齐 MCP 草案 §3.1，按 Phase2-0 可编译范围收窄）

```jsonc
{
  "request_id": "uuid",                 // 幂等键（MCP 草案 §3.2）
  "supersedes": {                       // 可选；修订既有 change_set [本文提议]
    "change_set_id": "…", "version": 2, "proposal_fingerprint": "sha256:…" },
  "rationale": "string",                // 给接受面板看的说明
  "existing_refs": [ { "id": "…", "revision": "…" } ],   // 读依赖（FX §8.3 第 1 类；服务端另行补全）
  "proposed_objects": [
    { "local_ref": "s1", "kind": "source_span", "op": "create",          // 2026-11-09 新增，§4.2.5
      "payload": { "artifact_id": "<canonical markdown 快照 id>",
                   "md_byte_range": { "start": 1024, "end": 1180 },
                   "granularity": "inline|block|page",
                   "quote": "Let $G$ be a finite group …" } },
    { "local_ref": "u1", "kind": "source_unit", "op": "create",
      "payload": { "unit_kind": "theorem", "display_title": "…", "parent": "<id|$new:local_ref>|null",
                   "source_spans": ["$new:s1", "<已有 span id>"], "completeness": "…" },
      "evidence": [ { "kind": "material_locator", "artifact_id": "…", "locator": { "md_byte_range": {"start":0,"end":10} } } ],
      "debts": [ { "kind": "assumption_debt", "note": "…" } ] },
    { "local_ref": "u2", "kind": "source_unit", "op": "edit",
      "target_id": "…", "expected_revision": "…", "changes": { /* SourceUnitChanges */ } }
  ],
  "proposed_relations": [
    { "local_ref": "r1", "kind": "source_relation", "op": "create",
      "relation": "<DiscoveryRelationKind>", "source": "<id|$new:local_ref>", "target": "<id|$new:local_ref>",
      "evidence_spans": ["<span id|$new:local_ref>"] }
  ],
  "unresolved": ["…"]
}
```

引用写法 [锁定·2026-11-09 用户改定]：凡引用位置（`parent`、`source_spans`、关系 `source` / `target` / `evidence_spans`、`changes` 中的同类字段）可写三种值——正式或暂存的稳定 id（`mathscape:<kind>/<hash>`）、`$new:<local_ref>`（本 version 内新建项），或兼容期的裸 `local_ref`（§4.2.6）。原口径只有 `<id|local_ref>` 两种。

#### 4.2.2 输出

```jsonc
{
  "status": "staged" | "blocked",       // mcp_envelope v1，见 §4.5
  "change_set_id": "…",
  "version": 3,
  "proposal_fingerprint": "sha256:…",
  "base_commit": "…" | null,            // 校验时的地图 head
  "staged_ids": { "u1": "<stable id>", "r1": "<stable id>" },  // 仅 staged 时填；blocked 时省略
  "debts": [ /* §3.3 */ ],
  "diagnostics": [ { "level": "error|warn|info", "code": "…", "message": "…", "local_ref": "u1" } ],
  "supersedes": { "change_set_id": "…", "version": 2 } | null
}
```

- **不 commit**：本工具只写暂存表（短事务），从不调用 `commit_map_change` / 登记提交器（MCP 草案 §3.2「提交」行；决策 2）。
- `staged_ids`：服务端预分配稳定 id，键覆盖本 version 的每一个新建项（`local_ref` → id，id 前缀即对象种类：`source-unit` / `source-relation` / `source-span`），所以它也是本次提案条数的权威来源：skill v0.6.6 要求 Agent 向用户汇报数量时按它计数，不靠记忆 [2026-11-11]。接受前后不变。现行实现（`crates/mathscape-mcp/src/tools/propose.rs` `generated_id`）按 `(change_set_id, local_ref)` 派生，同一 change_set 的各 version 中同名 `local_ref` 得到同一 id——这是 §4.2.8「替换后依赖仍可解析」的前提，本文据此改正原写法 `(request_id, local_ref)` [本文提议，2026-11-09 对齐代码]。`source_span` 的 id 例外，按内容确定（§4.2.5）。接受后**沿用**同一 id 作为正式对象 id（FX §4.4 第 1 条；现有 `UserChangeOperation::CreateSourceUnit { object_id }` 由调用方给 id，正好承接）。Agent 不得自带正式 id（MCP 草案 §3.2）。
- 幂等：同 `request_id` + 同内容 → 返回原结果；同 `request_id` + 不同内容 → 调用错误 `invalid_arg`（`details.reason: idempotency_conflict`，与 `map_change_requests` 行为一致，见 `crates/mathscape-store-sqlite/src/map_edit.rs` `commit_map_change`）。

#### 4.2.3 Phase2-0 可编译范围 [本文提议]

现有正式写管线只有：`UserChangeOperation` 的 `CreateSourceUnit` / `CreateSourceRelation` / `EditSourceUnit` / `EditSourceRelation` / `RetireObject`（`crates/mathscape-domain/src/map_edit.rs`），来源登记（`plan_registration_commit` / `commit_bound_source_registration`），手稿证据登记（`register_ink_evidence`）。为「复用、不另造写引擎」（REV §1；FX §4.4「服务端编制补丁，不能直接执行 Agent 给的 MapPatch」）：

| `kind` / `op` | Phase2-0 | 编译目标 |
| --- | --- | --- |
| `source_unit` / `create` | ✅ | `CreateSourceUnit` |
| `source_unit` / `edit` | ✅ | `EditSourceUnit`（`expected_revision` 必填） |
| `source_relation` / `create` | ✅ | `CreateSourceRelation` |
| `source_relation` / `edit` | ✅ | `EditSourceRelation` |
| 任意 `retire` | ❌ Agent 不可提议退役 | 退役只走 Desktop 手工编辑或 §6.2 |
| `SemanticEntity` / `FormalArtifact` / `Alignment` / 解释 / 证明对象 | ❌ | 后续契约（语义层） |
| `source_span` / `create` | ✅ [锁定·2026-11-09 用户改定；原口径 ❌，见 §10 待核 1] | 新增 `UserChangeOperation::CreateSourceSpan`（§4.2.5） |
| `source_span` / `edit` | ❌ | span 是不可变定位；改定位 = 新建 span + 改引用 |

`source_unit` 的 `payload` 即 `SourceUnitDraft`（单元种类写在 `payload.kind`，与外层项的 `kind` 分属两层；2026-11-10 对齐代码，原写法 `unit_kind` 未实现），`source_relation` 的 `relation` / `source` / `target` / `evidence_spans` 写在项的顶层（对应 `SourceRelationDraft`）；`parent` / `source` / `target` / `source_spans` / `evidence_spans` 可写 `$new:<local_ref>`（或兼容期裸 `local_ref`），编译时替换为 `staged_ids`）；`changes` 即 `SourceUnitChanges` / `SourceRelationChanges`。

不支持的项 ⇒ `validation` Debt `kind_not_supported` / `operation_not_supported` ⇒ 整个 version `blocked`（接受粒度是 change_set，不做部分接受，决策 4）。

#### 4.2.4 校验与 blocked [锁定]

服务端校验（在短只读事务里取快照，**不**持正式写锁）：
1. 结构：kind/op 在 §4.2.3 白名单；字段 `deny_unknown_fields`；单 change_set 项数上限沿用 `UserChangeRequest` 的 1000 项。
2. 引用闭合 [锁定·2026-11-09 用户改定]：所有引用均可解析——`$new:<local_ref>` 必须指向本 version 内的新建项（否则 `dangling_new_ref`）；稳定 id 必须是正式图上的活动对象，或**本调查**某个 `staged` 活动 version 的 `staged_ids`（否则 `dangling_staged_ref`；其他调查的暂存视同不存在）。引用其他 change_set 的暂存对象即形成依赖边，接受时须满足 §4.2.8 的连带规则，从而仍保证 FX §4.4 末条「正式提交不得留下只指向暂存对象的引用」。
   - 原口径：「`local_ref` 只在本 version 内有效；不得只指向**其他** change_set 的暂存对象」。
3. 写前置：`edit` 的 `expected_revision` = 当前 revision；`existing_refs` 当前可用（FX §8.3 第 2 类）。
4. 试编排：把 version 编译成 `UserChangeRequest` 并以**当前 head ⊕ 依赖闭包**（本 version 引用到的其他 `staged` change_set 的操作，按依赖拓扑序前置）运行纯 planner `plan_map_change_over_head`（`crates/mathscape-orchestrator/src/map_edit.rs`），`ImpactPlanStatus::Blocked|Conflicted` 的 `ImpactDiagnostic` 逐条转为 `validation` Debt（带 `target_id`）。[2026-11-10 V1] 请求既未操作也未改动的对象上、head 里本来就有的问题（同 `code` + 同对象在原 head 上已出现）不阻塞：它们在 `ImpactPlan.head_issues` 里，`propose_changes` 映射为 `warn: head_integrity`，消息写明对象 id，不进 Debt；Desktop 批量计划与接受用同一 planner，因此接受也不被它们挡住。planner 先把全部操作应用到 `final_map` 再整体校验（含 `source_integrity::validate_live_references` 的 `parent_cycle`、`invalid_evidence`、`mixed_source_context`、`cross_source_parent`、`invalid_locator`、`invalid_relation`、`inactive_or_invalid_reference`），因此本 version 内各项的书写顺序不影响结果（§4.2.7）。2026-11-09 改定前只以当前 head 为底。
5. 禁止项：`same_as`、自设可信终态（MCP 草案 §3.2）。
6. [2026-11-09 新增] SourceSpan 校验（§4.2.5）与 `unused_span`：本 version 新建、但未被本 version 任何单元 / 关系引用的 span → `unused_span`。
7. [2026-11-09 新增] 依赖图无环：change_set 之间的依赖边不得成环（`change_set_dependency_cycle`）。

结果 [锁定]：
- 全部通过 → `staged`。
- 任一失败 → `blocked`：**保留**完整提案内容、Debt、diagnostics、`proposal_fingerprint`；**不**写正式图；**不**删除提案（决策 3）。
- 修订：Agent 以 `supersedes{change_set_id, version, proposal_fingerprint}` 重新 propose；指纹不等于该 change_set 当前活动 version 的指纹 → 调用错误 `invalid_arg`（`details.reason: supersede_conflict`），防止基于过期版本覆盖。成功后旧 version → `superseded`，新 version 同 `change_set_id`、`version+1`。
- `proposal_fingerprint` [本文提议定义]：对规范化后的提案内容（对象 / 关系 / existing_refs / unresolved / Agent 声明 Debt，集合排序）+ 校验器版本 取 sha256；**不含**时间戳、`request_id`、base_commit。它标识「提案内容」；与 §5 的「计划指纹」分开。

#### 4.2.5 SourceSpan 提案 [锁定·2026-11-09 用户改定 / 本文提议细节]

原口径：Phase2-0 不支持 Agent 新建 SourceSpan，`source_unit/create` 只能引用已存在的 span id（§10 待核 1）。

**Agent 只给定位，服务端校验并补全；Agent 不能填身份、哈希、页码、块等字段**（`deny_unknown_fields`，多写即 `invalid_arg`）。

| Agent 输入 | 说明 |
| --- | --- |
| `artifact_id` | canonical Markdown **快照** Artifact 的 id（`material.read` 的 `details.served_segments[].artifact_id`） |
| `md_byte_range` | `{start, end}`，该快照字节坐标，半开区间，`start < end` |
| `granularity` | `inline` \| `block` \| `page`（`SpanGranularity`） |
| `quote` | **整个范围**的原文（可带空白 / TeX 定界符差异，见下）；不能只写开头或用省略号 |

服务端校验（任一失败 ⇒ `validation` Debt ⇒ version `blocked`）：

| 校验 | 失败码 |
| --- | --- |
| `artifact_id` 是本工作区已登记的**不可变** canonical Markdown 快照，内容哈希与登记一致 | `snapshot_not_found` / `snapshot_not_immutable` |
| 范围在快照字节长度内、`start < end` | `range_out_of_bounds` |
| 范围不超过 4096 字节 [2026-11-10 V3]（一章按题或按段拆成多个 span） | `span_too_long` |
| `start`、`end` 落在 UTF-8 字符边界 | `invalid_utf8_boundary` |
| `quote` 与范围原文一致 [2026-11-10 定稿 R2]：**先按 `[start,end)` 截出范围原文**，再比较——先精确相等；否则两侧都去掉外层数学定界符并做 TeX 感知的空白归一化（`source_locator::quote_matches_range`，归一化与内容发现共用 `normalize_locator_text`）后相等。不在快照全文里搜索 quote（原口径的 `locator_matches` 是全文搜索，只留给内容发现） | `quote_mismatch` |
| `granularity = block` 但范围内没有 canonical `block:*`（页标记不算） [R3] | `block_locator_unavailable` |
| `granularity = page` 但范围不能唯一映射到一个 PDF 页（跨分页标记、多页快照无分页标记、快照无页信息等） [R3] | `page_mapping_unavailable` |
| 同 id span 在正式图上已退役 [U1] | `span_inactive` |
| span 未被本 version 任何单元 / 关系引用 | `unused_span`（§4.2.4 第 6 条） |

服务端补全：
- `markdown = MarkdownLocator(artifact_id, ByteRange(start, end))`，`displayed_lines` 不填。
- `normalized_text_hash` [2026-11-10 定稿 R4] = `source_text_hash(范围原文)` = `fingerprint(&text)`，即对原文字符串做 JSON 序列化后取 sha256，与内容发现登记写 span 文本 hash 的函数相同（`crates/mathscape-orchestrator/src/source_locator.rs`，两条路径都调用它）。字段名里的 `normalized` 是历史命名：hash 前**不做**归一化。PR-B 合并前提案路径曾用原文字节 sha256，但该版本从未进入 main，不存在需要迁移的已有数据。
- `marker_block_ids` = 与范围相交的 canonical `block:*` id（同 `ResolvedDiscoverySelector::canonical_block_ids` 的相交规则，块取自快照的 `AnchoredMarkdown.blocks`）。
- `page`：仅 `granularity = page` 时填 `PageCoordinate`，由快照的分页标记（或单页快照的页范围）确定唯一页；不能唯一确定 ⇒ `page_mapping_unavailable`（R3），不留空降级。`inline` / `block` 粒度不填 `page`（来源登记同样留空；`markdown` 已满足 `invalid_locator` 校验）。
- `layout = Other`、`bounding_boxes = []`、`exact_text_artifact = None`；**`ink` 永不填**——手稿证据只走 `register_ink_evidence`（Desktop 用户写入）。
- lifecycle / trust 同 §3.3：接受者写入，不自设可信终态。

**先定位再提案** [2026-11-10 V2/V3]：Agent 用 `material.read` 读页，取 `served_segments[].artifact_id / md_byte_range`；从正文原样复制一段，调用 `material.locate`（可用 `within` 限定在所读页的字节段），把返回的 `md_byte_range` 与**同一** `quote` 放进 `source_span`。`material.locate` 只是 R2 的搜索形式：它返回的范围都满足 `quote_matches_range(范围原文, quote)`，因此一定通过 R2；R2 本身仍是截取后比较，不在全文里搜。快照没有分页标记时（`page_markers_absent`）只能用 `inline` 粒度。

**身份与去重** [锁定·2026-11-09 用户改定]：span id **复用内容发现的确定性规则**——`markdown_span_id(artifact_id, range) = stable_hash_id("source-span", fingerprint(("markdown-source-span-1", artifact_id, range)))`，定义在共享模块 `crates/mathscape-orchestrator/src/source_locator.rs`，来源登记的 `intern_span` 与 propose 都调用它（有跨路径同 id、同 hash 的测试）。因此：
- 同一快照同一范围得到同一 id，无论来自内容发现登记、哪个 change_set、哪个 version。
- 该 id 在正式图上已有**活动** span ⇒ 视为引用，`CreateSourceSpan` 在编译时降为「引用已有 span」，**不** blocked（诊断 `info: span_already_exists`）；Agent 给的 `granularity` 与已有不同时以已有为准，附 `warn: span_granularity_ignored`。
- 多个 change_set 暂存同一 span ⇒ 同一 id；先接受者创建，后接受者变为引用。span 因此**不产生** change_set 之间的依赖边（每个需要它的 change_set 自带一条 `CreateSourceSpan` 即可）；若 Agent 直接写另一 change_set 暂存的 span id 而不自带 `CreateSourceSpan`，则按 §4.2.8 记为依赖（任一创建它的 change_set 已接受或同批接受即满足）。
- 同 id 的 span 在正式图上存在但**已退役** [2026-11-09 用户定 U1]：`span_inactive` ⇒ blocked，不自动重新激活。

#### 4.2.6 临时引用 `$new:<local_ref>` [锁定·2026-11-09 用户改定]

- 本 version 内的新建项用 `$new:<local_ref>` 引用；`local_ref` 本身（`proposed_objects[].local_ref` / `proposed_relations[].local_ref`）复用内容发现临时 id 规则 `valid_temp_id`（`crates/mathscape-orchestrator/src/discovery_validation.rs`：≤64 字节，首字符 ASCII 字母数字，其余为字母数字或 `_` `-` `.`），不合规 ⇒ `invalid_local_ref`。
- `$new:` 前缀与稳定 id 的 `mathscape:` 前缀互斥，消除「local_ref 恰好像 id」的歧义。
- **兼容**：裸写 `local_ref`（现行实现 `resolve_ref` 先查本 version 的 `local_ref` 表、再按 `StableId` 解析）保留**一个 tool schema 版本**（`tool_schema_version` 的下一版），期间每处裸写附 `warn: bare_local_ref_deprecated`；兼容期内现行的 `local_ref` 长度规则（≤128 字符、无控制字符）对裸写仍适用，`$new:` 写法用 `valid_temp_id`。下一版移除裸写，届时 skill 同步。

#### 4.2.7 最终图校验与写入顺序 [锁定·2026-11-09 用户改定]

- **校验在最终图上统一做**：不按书写顺序逐项检查，而是把 head ⊕ 依赖闭包 ⊕ 本 version 全部操作应用后，对最终图运行同一套校验——`plan_map_change` 内的结构检查 + 共享校验器 `source_integrity::validate_live_references`（已被手工编辑、来源登记、登记撤回共用）。成环（`parent_cycle`）、单元证据为空、跨来源父子、关系端点缺失、引用不可用对象等一律在此报出，不另写一套。
- **应用顺序** [2026-11-10 定稿]：planner 不按书写顺序应用操作，而是先 `CreateSourceSpan`，再按**本请求内**新建单元的父深度应用 `CreateSourceUnit`（父在前），最后其余操作；同层保持原相对顺序。因此打乱书写顺序得到同一计划（有测试）。
- **写入顺序**（接受时生成 patch）：`CreateSourceSpan` → `CreateSourceUnit` / `EditSourceUnit`（按最终图父深度，父在前）→ `CreateSourceRelation` / `EditSourceRelation`，同层按 id 排序；整个 change_set（批量时整批，§5.5）在**一个** `BEGIN IMMEDIATE` 事务里写完，失败整体回滚。
- **批量拼接顺序** [2026-11-10 定稿]：多个 change_set 按依赖拓扑序拼成一个请求，同层以 `(change_set_id, version)` 决定先后，**不看 `created_at`**，所以合成请求与计划指纹只由依赖图决定。共享 span 去重（只保留第一条 `CreateSourceSpan`）。
- planner 需新增对 `CreateSourceSpan` 的处理：把 span 写入 `final_map`；已有活动同 id span 时转为 no-op 引用（§4.2.5）。

#### 4.2.8 跨 change set 依赖与连带规则 [锁定·2026-11-09 用户改定 / 本文提议细节]

依赖边：version B 的某个引用解析到 change_set A 的暂存对象（A ≠ B）⇒ 记 `B → A`（存于暂存表，§7.4）。依赖只在同一调查内产生。

| 事件 | 对依赖者 B 的影响 | 码 |
| --- | --- | --- |
| 接受 B | 要求 B 依赖的每个 A **已接受**或在**同批**接受（§5.5）；否则拒绝，面板指出缺哪个 | `dependency_not_accepted` |
| 用户丢弃 A [2026-11-09 用户定 U3] | 预览中列出所有直接 / 传递依赖者；用户二选一：**连带丢弃**（依赖闭包一并 `discarded`），或**保留为 blocked**（默认：直接依赖者 `dependency_discarded`，更深的 `dependency_blocked`，等 Agent 重提）。`blocked` 的 version 也可丢弃 | `dependency_discarded` / `dependency_blocked` |
| Agent 以 `supersedes` 替换 A [2026-11-10 定稿 R5] | 依赖记录带 `A@version`；A 被替换后，直接依赖者 B → `blocked`（`dependency_superseded`），更深的依赖者 → `blocked`（`dependency_blocked`）。**不自动改指** A 的新 version（原口径「id 仍存在则保持 staged 并改指」作废）：B 是对 A 旧内容的判断，由 Agent 读回后重提。之后新提案若引用已不在任何 staged version 中的 id ⇒ `dangling_staged_ref` | `dependency_superseded` / `dependency_blocked` / `dangling_staged_ref` |
| A 的新 version 为 `blocked` | B → `blocked`（被引用对象不再出现在投影中） | `dependency_blocked` |
| A 已接受、之后被撤回（P2-4） | 已接受的 B：按 §6.2「禁止非法悬挂」，A 的撤回计划为 `Blocked`，除非级联把 B 的对象一并退役；仍暂存的 B：转 `blocked` | `dependency_withdrawn` |
| B 引用 A、A 又引用 B（经替换产生） | 两者中后提交的 version `blocked` | `change_set_dependency_cycle` |

- 所有连带变化都在预览中**列出范围**（change_set、version、受影响 `local_ref` / 对象），不静默发生；连带只改提案状态，不写正式图。
- 未被引用的新 span ⇒ `unused_span`（blocked），防止无主 span 进图。
- 连带转 `blocked` 的 version 保留全文与指纹（决策 3），`get_change_set` 可读回。

### 4.3 `get_change_set` [锁定语义]

输入：`{ "change_set_id": "…", "version": 3? }`（省略 version = 最新 version）。
输出（对齐 MCP 草案 §3.3）：

```jsonc
{ "status": "ok",
  "change_set_id": "…", "investigation_id": "…", "version": 3,
  "versions": [ { "version": 1, "state": "superseded" }, { "version": 3, "state": "staged" } ],
  "state": "staged|blocked|accepted|discarded|superseded|withdrawn",
  "proposal_fingerprint": "…", "proposed_by": "agent:codex", "round_id": "…",
  "proposal": { /* 规范化输入全文 */ }, "staged_ids": { … },
  "debts": [ … ], "diagnostics": [ … ],
  "decision": { "kind": "accepted|discarded|withdrawn", "decided_by": "mathscape-desktop-user",
                "plan_fingerprint": "…", "commit_id": "…", "receipt": { /* CommitReceipt */ } } | null }
```

- 可见性 [锁定]：只能读**本调查**的 change_set；其他调查的 id → `status: not_found`（不泄露存在性）。
- 只读，不校验 `--round` generation（Phase0 §2.1），因此 `awaiting_user` 后的下一 turn 可立即回读。
- 也供 Agent 查看接受后回执，确认哪些已提交（FX §6.2 末段「最终报告哪些已提交、哪些仍为候选」）。

#### 4.3.1 `list_change_sets` [2026-11-10 用户定]

只读。输入 `{ "pending_only": false?, "limit": 1–100? }`（默认列全部状态，上限 100）。调查与 `get_change_set` 同一解析（round 绑定或适配器声明的 investigation）；没有调查时返回空列表 + `investigation_unavailable`。输出：

```jsonc
{ "status": "ok", "investigation_id": "sandbox:…", "total": 2, "truncated": false,
  "change_sets": [ {
    "change_set_id": "…", "request_id": "…(活动 version 的)", "title": "rationale 前 120 字",
    "created_at": "…(v1)",
    "active": { "version": 2, "state": "staged|blocked|accepted|discarded|withdrawn",
                "proposal_fingerprint": "…", "created_at": "…", "round_id": "…",
                "blocking_codes": ["…"], "debt_count": 0 },
    "versions": [ { "version": 1, "state": "superseded", "request_id": "…", "created_at": "…" }, … ],
    "supersedes": { "change_set_id": "…", "version": 2, "proposal_fingerprint": "…" } | null } ] }
```

- 按活动 version 的 `created_at` 倒序；`pending_only` 只留活动 version 为 `staged` / `blocked` 的。
- `supersedes` 只在活动 version 为 `staged` / `blocked` 时给出，原样传给 `propose_changes` 即得到同一 change_set 的下一 version（§4.2 的 `supersedes` 校验不变）；已接受 / 丢弃的为 `null`。
- 可见性同 §4.3：只列本调查；其他调查的 change_set 不出现、不泄露数量。

### 4.4 `scope=investigation`：暂存投影 [锁定语义]

替换今天的桩（Phase0 §2.3：`investigation` 按 `accepted` 回答 + `scope_not_supported`；1c §3.2 同）：

- 适用工具：`graph.get`、`graph.neighborhood` 必做；`workspace.search` 的暂存命中可紧随其后（§9 P2-2b），未做之前它继续返回 `scope_not_supported`。`material.*`、`pdf.outline`、`ink.page_image` 读的是材料 / 手稿，不受 scope 影响（1c / 1d 不改）。
- 内容 = **已接受快照 ∪ 本调查所有 `staged` 活动 version 的对象 / 关系**（FX §4.4 第 4 条）。`blocked`、`discarded`、`superseded`、其他调查的暂存**一律不可见**（FX §4.4「跨调查默认不暴露」「草稿不能伪装成合法图节点」）。
- 节点 / 边加字段：`staged: true|false`、`change_set_id`、`change_set_version`（staged 时必有）；暂存的 `edit` 以 replacement 出现并带 `replaces_revision`，不覆盖已接受 revision（FX §4.4 第 2 条）。1c §6 其他字段不变。
- 同一已接受对象在本调查内有多个 `staged` 的互斥 replacement → 不任取，返回该节点的已接受版本并附诊断 `staged_conflict`（列出 change_set_id）。
- 没有绑定调查（调试 CLI、无 `--round`）时仍按 `accepted` 回答并附 `scope_not_supported`，保持向后兼容。
- **单一投影** [2026-11-10 定稿 R7]：投影只有一份服务端实现（`mathscape_mcp::investigation::investigation_landscape`，内部用 orchestrator 的 `project_investigation`），`graph.get`、`graph.neighborhood` 与 Desktop 地图共用。暂存 `edit` 以 replacement 叠加：投影里该对象显示暂存后的内容并带 `replaces_revision`，已接受 revision 不被改写。scope 标签：暂存节点 `scope: "staged"`、`staged: true`；已接受节点在调查范围下 `scope: "accepted"`、`staged: false`；`graph.neighborhood` 的 `applied.scope` 在投影时为 `investigation`。
- **排除项** [R7]：暂存对象不进正式 head（`get_map_head` / `map_edit_snapshot`）、正式快照 / landscape、沙箱导出包（导出只含对话记录与事件）、默认 `scope=accepted` 的 `graph.*` 与 `workspace.search`、正式编辑入口（`get_map_object_edit_context` 只认 head）和母工作区地图。`workspace.search` 的 `scope=investigation` 暂存命中仍未实现，继续返回 `scope_not_supported`。
- `scope=accepted`（默认）行为完全不变。

#### 4.4.1 暂存上图（Desktop 地图）[锁定·2026-11-09 用户改定 / 本文提议样式]

原口径：`staged` 只进 Agent 的 `scope=investigation` 投影；Desktop 地图只显示已接受快照（沙箱 §4.4「不做完整 staged 幽灵层」）。

- Desktop 地图用**同一份投影**（已接受快照 ∪ 当前调查的 `staged` 活动 version，节点 / 边带 `staged: true`、`change_set_id`、`change_set_version`）渲染当前调查的暂存对象，样式为「未审批」：虚线描边、半透明、角标「未审批」；暂存 `edit` 以 replacement 叠在原对象上并可切换查看原版本；`staged_conflict` 节点显示已接受版本加冲突角标。`blocked` 的不上图，只在审核面板出现。
- 暂存对象可点开（详情显示 change_set、Debt、诊断），但**不能**在地图上手工编辑——手工编辑只针对正式对象，避免两条写路径交叉。
- Agent 可以在自己的暂存对象上继续构建：经 `scope=investigation` 读到暂存对象及其 `staged_ids`，在后续 `propose_changes` 中按稳定 id 引用（§4.2.4 第 2 条、§4.2.8）。
- **正式 head 不动，直到接受**；接受时按 OCC 检测冲突（§5.1 第 3 步、§7.3）。地图 `get_map_head`、正式快照、导出等一律不含暂存对象（排除项见 §4.4）。暂存 replacement 节点的详情可「查看已接受版本」（读 head）。
- 切换到其他调查 / 沙箱会话时，地图只显示该调查自己的暂存（跨调查不可见，§4.4）。

### 4.5 `mcp_envelope v1` 增量 [本文提议]

Phase0 §2.2 已把 `blocked` 预留给 `propose_changes`。v1 增量仅：
- `ToolStatus` 增加 `staged`、`blocked`，**只**用于 `propose_changes` 的顶层 `status`；其他工具继续 5 值。`agent_events` 的 `tool_result.status` 相应可取这两个值。
- 不新增调用错误码；新增的 `details.reason`：`idempotency_conflict`、`supersede_conflict`、`round_not_bound`、`generation_stale`（均挂在 `invalid_arg` / `forbidden` 下）。

---

### 4.6 `material.locate` [2026-11-10 用户定]

只读工具，输入 `{artifact_id, quote, within?: {start, end}}`。`artifact_id` 必须是 text/markdown 快照（与 `source_span.artifact_id` 相同的校验：`propose::load_snapshot`）。`quote` 1–4096 字节，超过 ⇒ `not_found` + `span_too_long`。`within` 是快照字节坐标的半开区间，须在快照内且落在 UTF-8 边界，否则 `invalid_arg`。

匹配：`source_locator::locate_quote`——先找精确出现；没有时两侧做与 R2 相同的归一化后搜索，再把命中映射回原始字节；最后用 `quote_matches_range` 过滤，保证每个候选与同一 `quote` 必过 R2。输出 `status`（`ok` 唯一 / `ambiguous` 多处 / `not_found`）、`match`（`exact` | `normalized`）、`candidate_count`、`truncated`、`candidates[]`（每项 `md_byte_range`、`block_ids`（同 `marker_block_ids` 规则）、`page`（仅能唯一映射到一页时给 `{pdf_page_index, page_number}`）、`page_granularity_available`）；唯一命中时这些字段也放在顶层。候选上限 20（`candidates_truncated`）。不写地图、目录或产物池。

[2026-11-10 第三轮] 有候选落不到唯一页时追加一条诊断：快照覆盖多页（或没有页范围）且没有分页标记 ⇒ `page_markers_absent`（带 `artifact_id`、`snapshot_pages`，与 `material.read` 同码）；其他原因（跨分页标记、快照不属于任何 PDF 的 canonical 视图等）⇒ `page_unresolved`（消息里带原因）。两者都表示只能用 `inline` 粒度、不应声称页码。

## 5. Desktop 接受路径 [锁定语义]

### 5.1 流程

1. **预览**：接受面板对选中的 change_set 活动 version（必须 `staged`）调用 Desktop 专用命令 `plan_change_set_accept(change_set_id, version, proposal_fingerprint)`：服务端把 version 编译为 `UserChangeRequest`（`object_id` 用 `staged_ids`，`base_commit` = 当前 head，`request_id` 由 Desktop 生成），以接受者 actor 运行现有 `plan_map_change` → `ImpactPlan`（含 `plan_fingerprint`、`direct_changes`、`affected_objects`、diagnostics），连同 Debt 一起展示。
2. **显式接受**：用户点击接受 → `accept_change_set(change_set_id, version, proposal_fingerprint, plan_fingerprint, request_id)`。**默认无自动接受**，也没有跨调查的「全部接受」快捷方式（决策 4）。[2026-11-09 用户改定] 本调查内的批量接受见 §5.5；原口径为「没有『接受全部调查』快捷方式」，单个 change_set 逐个接受。
3. **提交**（短锁协调器，§7.2）：
   - `BEGIN IMMEDIATE`；先查 `request_id` 幂等（同内容返回原回执）。
   - 核对：change_set 活动 version 仍为 `version` 且状态 `staged`；`proposal_fingerprint` 相等。
   - 在事务内重新读取快照，重跑同一纯 planner，核对 `base_commit` 与 `plan_fingerprint`（同 `commit_map_change` 的 `base_commit_conflict` / `plan_conflict` 语义）。
   - 用共享写入器 `write_patch`（`crates/mathscape-store-sqlite/src/map_edit.rs`，注释「caller owns the transaction and its business audit/receipt」）写 revision；**同一事务**内写接受决定（`accepted_by`、`plan_fingerprint`、`commit_id`、receipt）并把 version 置 `accepted`。不得先调现有 `commit_map_change` 成功、再开第二个事务记接受（同 REV §6 第 2 段）。
4. **回执**：返回现有 `CommitReceipt`（`commit_id`、`objects[]` 的 revision）；`get_change_set` 的 `decision.receipt` 同此。

### 5.2 绑定规则 [锁定]

- 接受绑定 `(change_set_id, version, proposal_fingerprint, plan_fingerprint)`；任一不符即拒绝，旧决定不能接受新内容（FX §4.4 第 2 条）。
- 预览后 head 变化导致 `plan_fingerprint` 失效 → 返回冲突，面板要求**重新预览**；若重新预览为 `Ready`，用户需再次点击接受（新指纹）；若为 `Blocked|Conflicted`，面板显示诊断，提示由 Agent `supersedes` 重提或用户丢弃。**不自动合并、不自动改写提案**。
- 接受粒度 = 整个 change_set version；不做逐项接受。
- 一次调查内可多次接受不同 change_set；接受不结束调查、不启动新轮、不授权其他待审变更（FX §8.2 末段）。
- [2026-11-09 新增] 接受 B 时其依赖 A 必须已接受或同批接受（§4.2.8）；单个接受 B 而 A 仍暂存 ⇒ 拒绝 `dependency_not_accepted`，面板提示「连同 A 一起审」。

### 5.3 复用而非重新暴露

| 现有写面 | 位置 | Phase2 用法 |
| --- | --- | --- |
| `plan_map_change` / `commit_map_change`（Tauri） | `apps/mathscape-desktop/src/map_edit.rs` | **不**暴露给 Agent；接受路径复用其 orchestrator 纯 planner 与 store `write_patch` |
| `plan_map_change`（orchestrator） | `crates/mathscape-orchestrator/src/map_edit.rs` | 校验（§4.2.4 第 4 步）与接受预览 / 提交共用 |
| `SqliteArtifactStore::commit_map_change` / `write_patch` | `crates/mathscape-store-sqlite/src/map_edit.rs` | 接受提交参照其事务顺序，共用 `write_patch` |
| 来源登记 plan/commit、`plan/commit_discovery_registration_reversal` | `apps/mathscape-desktop/src/registration.rs`、`crates/mathscape-orchestrator/src/registration.rs`、`crates/mathscape-store-sqlite/src/registration.rs` | §6.2 撤回复用其共享 / 依赖分析与原子写入模式 |
| 手稿写命令（`register_ink_evidence`、`create_ink_snapshot` 等） | `apps/mathscape-desktop/src/ink.rs` | 不经 Agent；仍是用户写入，经同一协调器（§7.2） |

### 5.4 不开放给 Agent [锁定]

Agent 面只有 `propose_changes`（写暂存）、`user.ask`（写 PendingQuestion）。接受 / 丢弃 / 撤回命令只注册为 Desktop Tauri 命令（PR-B 实现：`get_change_set_investigation`、`list_pending_change_sets`、`get_change_set_snapshot`、`preview_change_set_impact`、`plan_change_set_batch`、`accept_change_set_batch`、`discard_change_set_batch`；单个接受 = 只含一项的批量，不另设 `accept_change_set`），并校验传入的 `investigation_id` 属于本窗口（本窗口绑定的调查，或仍存活的沙箱会话 `sandbox:{session}`；否则 `investigation_not_in_workspace`），不进 `mathscape mcp` 的 `tools/list`，也不经适配器转发（FX §7.1；决策 2）。

### 5.5 批量审核：会话切换 / 退出时统一决定 [锁定·2026-11-09 用户改定 / 本文提议细节]

原口径：只有针对单个 change_set 的接受面板，且无批量入口（决策 4、§5.1 第 2 步）。

**触发**：
- [2026-11-10 定稿] 用户**切换沙箱会话**、**丢弃会话**、**导出会话**、离开沙箱回母工作区或**退出应用**，且当前调查（整场会话，R1）有 `staged` 或 `blocked` 活动 version ⇒ 弹出批量审核界面，等用户决定后再继续原操作。**切换对话线程（「新对话」）不触发**：线程共享同一调查，暂存继续可见。原口径把「新对话」列为触发点，作废。
- 审核范围 = 本调查的全部待审 version，不限于当前线程或某一 round。
- 任何时候也可手动打开（对话侧栏「审核变更（N）」入口）；中途审过的不再出现。
- 用户关闭界面不做决定 ⇒ 全部保持原状态（不自动接受、不自动丢弃），可之后再审（决策 4「接受与流程结束分离」）。

**界面**（[2026-11-10] 这里只指阻塞式批量对话框——切换 / 丢弃 / 导出 / 退出时弹出；手动审核走 §5.6 的全屏审核工作区）：
- 列出本调查全部活动 version，**按依赖分组**（依赖图的连通分量，组内按拓扑序，同层按 `(change_set_id, version)`；无依赖的单独成组）。`staged` 默认**全选**；`blocked` 不可勾选，只能丢弃或留待 Agent 重提。每项展开后按 **Markdown 快照分组**（U2）。
- 每个 change_set 一行：勾选框、标题（含旧版本数与依赖）、状态、Debt 数与**影响预览**（`ImpactPlan` 的 `direct_changes` / `affected_objects`）；表头与各行共用同一列定义，标题列 `minmax(240px, 1fr)`；顶部显示整批合计影响。[2026-11-10] 对话框不再内嵌详情，看内容请用手动审核的工作区（§5.6）。
- **取消勾选**一项 ⇒ 依赖它的项自动连带取消，**多层传递**（并标注「因依赖 X 被取消」）；重新勾选依赖者 ⇒ 自动勾选其依赖闭包。
- 丢弃前预览受影响的依赖者，并提供「连带丢弃依赖项」选项（U3，默认不勾 ⇒ 依赖者转 `blocked`）。
- 单个 change_set 内部仍整批接受，**不拆开**（决策 4）。
- 按钮：「接受所选」「丢弃所选」，以及继续原操作（「保留未审批并切换 / 导出 / 退出」）或取消原操作。未勾选项保持 `staged`（2026-11-09 用户定 U4）。

**命令与提交**（Desktop 专用，不进 `tools/list`，§5.4）：
1. `plan_change_set_batch(investigation_id, selections[{change_set_id, version, proposal_fingerprint}])`：服务端校验依赖闭合（选中集合对依赖封闭，否则返回缺失项），按依赖拓扑序把所有选中 version 编译为**一个** `UserChangeRequest`（写入顺序同 §4.2.7），对当前 head 运行 `plan_map_change_over_head`（§2.2 V1：请求未触及对象上 head 已有的问题进 `head_issues`，不阻塞）→ 批 `ImpactPlan` 与 `batch_plan_fingerprint`；同时给出每个 change_set 的分项影响。
2. `accept_change_set_batch(investigation_id, selections, batch_plan_fingerprint, request_id)`：同 §5.1 第 3 步——一个 `BEGIN IMMEDIATE` 事务内（`SqliteArtifactStore::accept_change_sets`）用当前 head 重跑 planner、核对 `base_commit` 与指纹、`write_patch`、为每个 change_set 分别写接受决定并置 `accepted`；任一不符整批拒绝，要求重新预览。[R6] 整批 = 一个 `UserChangeRequest`、一个计划指纹、一个事务、一个 commit，每个 change_set 的决定记同一 `commit_id`。
3. `discard_change_set_batch(investigation_id, selections, cascade)`：只改提案状态，按 §6.1 处理连带；`cascade=false`（默认）依赖者转 `blocked`，`cascade=true` 依赖闭包一并丢弃。
4. `preview_change_set_impact`：单个 change_set（连同其 staged 依赖闭包）对当前 head 的影响预览（同样用 `plan_map_change_over_head`），供列表的影响预览栏使用。
5. [2026-11-10] `get_change_set_snapshot(investigation_id, artifact_id)`：审核时读提案引用的 Markdown 快照。用 `investigation_id` 解析工作区（与其他命令同一 `workspace_for`：母工作区窗口审核仍存活的沙箱会话时读的是沙箱副本，所以沙箱里新产生的快照也能读到）；只服务本调查某个 version 锚定到的快照（否则 `snapshot_not_referenced`）；转换来源按 `canonical_markdown_coverage` 取唯一登记的 conversion，选不出时退一步用唯一的转换上下文，仍不唯一则 `conversion_ambiguous`；返回 `AnchoredMarkdown` 与所用 `conversion_id`。前端如实显示错误原因，不吞掉。

**不变**：只能显式接受；不自动提升 Trust；接受 ≠ 数学正确（面板固定提示「接受只表示写入地图，不代表数学上已验证」）；接受绑定提案版本 + 计划指纹；冲突不自动合并（§5.2）。

### 5.6 审核面板复用 [本文提议]

单个 change_set 的详情与批量界面的展开项复用内容发现审核工作区（`proposals/discovery-review-workspace-v1.md`，`frontend/src/review/`），不另写：
- 复用 `DiscoveryReviewWorkspace`（`workspace.ts`）的布局与交互、`anchor-fragment.ts`（`ReviewAnchor`、`checkAnchor`、`renderAnchorFragment`，按字节范围高亮、不做文本重搜）、`ReviewGraph`（`review-graph.ts`，独立 Cytoscape 容器，不并入主图）、`labels.ts`。
- 新增**「提案」数据源**：服务端把 change_set version 投影成 `SourceReviewProjection` 形状（新版本号如 `mathscape-change-set-review-projection-1`）：`units` ← `source_unit` 项（`acceptance` 取 `staged|blocked`），`anchors` ← 其 span 的 Markdown 定位，`relations` / `structure_relations` ← 关系与父子，`diagnostics` ← 诊断，新增 `debts` 与 `impact` 字段。工作区按数据源切换。
- 按钮换成「接受」「丢弃」（批量界面里为勾选），去掉内容发现的逐单元勾选——change_set 不做部分接受。增加 Debt 栏与影响预览栏。
- 详见 §9.1 前端项；多来源 change_set 按快照分组（U2）。
- [2026-11-10 实现定稿，同日第二轮修订] 不另设 `get_change_set_review` 命令。锚点由服务端给：`list_pending_change_sets` 的每个 version 带 `anchors[]`（`span_ref`、`span_id`、`origin` = `new|head|staged`、`artifact`、`conversion`、`range`、`block_ids`、`page{pdf_page_index,page_number}`、`granularity`、`excerpt`、`verified`、`compiled`）：新建 span 的块 id 与页码取自该 version 的 `compiled_operations`（未编译的阻塞 version 退回提案自带的范围，`compiled=false`）；引用的已有 span（`mathscape:source-span/…`）先在 head 里解析，再在本调查其他 version 的编译结果里解析；`excerpt` 从快照按范围读出（`verified=true`），读不到时用 Agent 引文。另带 `history[]`（同一 change_set 的旧 version，新在前）与 `targetTitles`（提案点名的已有对象标题：诊断 `target_id`、关系两端、`parent`、编辑目标；先查 head，再查本调查其他 version 的 `staged_ids`；2026-11-11 起不只诊断）。前端只把这些拼成每个快照一份 `SourceReviewProjection`，不再从原始 payload 推块 id 或页码。[2026-11-11] 审核页总览的单元 / 关系 / Debt 数字取整份提案；当前快照另标「快照 i/N · 本快照 x 单元 / y 关系」；默认打开单元 + 关系最多的快照；总览列出整份提案的全部关系（两端标题、关系种类，跨快照与连接已有对象的关系单独标出）并附关系图；跨快照关系同时出现在两端所在的快照里，局部结构图把另一端画成虚线外部节点。
- 手动审核（对话侧栏「审核变更」）直接打开全屏 `DiscoveryReviewWorkspace` 的**提案模式**：两栏、标签页、按字节范围高亮 + KaTeX、点击定位、`rawAnchorPreview`；快照经 `get_change_set_snapshot` 读取，读不到时显示真实原因，并用带 KaTeX 的渲染显示引文、标注「未与快照核对」。总览列出待审队列（同一 change_set 的旧 version 默认折叠）、依赖与被依赖、结构化 Debt（正文为 summary，kind 作标签，id 以灰色小字；不显示 JSON）、「工作区已有对象的问题」（没有 `local_ref` 的诊断按 `target_id` 分组）和提案级诊断。底栏「接受本提案」走批量接受（只含一项的批量，一次提交），「丢弃本提案」先预览依赖者并可选连带丢弃（默认依赖者转 `blocked`）。无逐单元勾选、无 Markdown 补丁、无登记按钮。

---

## 6. 回滚矩阵 [锁定]

| 场景 | 操作 | 写正式图？ | 留痕 | 允许条件 |
| --- | --- | --- | --- | --- |
| 暂存提案不要了 | **丢弃**（Desktop `discard_change_set`） | 否 | version → `discarded`，保留全文 / Debt / 指纹 / `decided_by` / 原因 | 活动 version 为 `staged` 或 `blocked` |
| 暂存提案要改 | **替换**（Agent `propose_changes` + `supersedes`） | 否 | 旧 version → `superseded` | 指纹匹配当前活动 version；调查持有写槽 |
| 已接受，不再采纳 | **撤回**（Desktop 预览 + 提交） | 是：退役 revision（Deprecated），不删除 | 撤回审计行 + 回执；version → `withdrawn`；原接受回执不改 | 计划 `Ready`；见下 |
| 已接受，想「撤销」 | 不存在普通 undo | — | — | 一律走「撤回」（REV §1） |

### 6.1 丢弃（廉价）

- 只改提案状态；不触碰正式图与 head，不占短锁之外的资源。
- [锁定·2026-11-09 用户定 U3] 其他 change_set 若依赖被丢弃者的暂存对象（§4.2.8），丢弃预览列出全部直接 / 传递依赖者，用户选择连带丢弃或令其转 `blocked`（默认）；不静默保留悬挂依赖。原口径：「§4.2.4 第 2 条已禁止跨 change_set 引用暂存，不受影响」。
- 被丢弃的内容仍可在历史中查看（FX §4.4 末条）。

### 6.2 撤回已接受（退役 / 级联）[锁定语义，策略名为本文提议]

- 复用 REV 的模式：新增退役策略 `ChangeSetScoped`（与 REV §3.1 `RegistrationScoped` 平行），范围由**接受回执**确定，不接受前端任意对象列表；共享 / 依赖分析、reason code（`created_by_change_set`、`subsequently_revised`、`referenced_by_live_object`、`required_by_preserved_object`、`already_inactive` 等，沿用 REV §5.2 的命名习惯）、`ImpactPlan` 的 `retire|preserve|block`、原子提交顺序（REV §6）全部照搬。
- **禁止非法悬挂** [锁定]：若退役会让仍活动的对象指向已退役对象（parent、关系端点、span 等）且计划未把它们一并级联，计划为 `Blocked`，不得提交。用户可选择 `IncidentRelations` 式级联（REV §3.1）重新预览；级联范围在预览中逐项列出。
- 后续人工修订过的对象默认 `preserve`（REV §4.1 思路），不被撤回覆盖。
- 撤回不回滚整个工作区、不删除 revision / Artifact、不释放磁盘（REV §1）。
- 撤回后不支持「反撤回」；重新需要时由 Agent 重新 propose（新 change_set）。
- [本文提议，2026-11-09 先行记录，P2-4 实现] 跨 change_set 依赖下的撤回：撤回 A 时，已接受且依赖 A 的 B 的对象按「禁止非法悬挂」处理——不级联则计划 `Blocked`，级联则 B 的相关对象一并列入退役范围并标 `required_by_withdrawn_change_set`；仍暂存且依赖 A 的 version 转 `blocked`（`dependency_withdrawn`）。批量接受产生的多个 change_set 各自独立撤回，不因同批而绑定。

---

## 7. 并发 / OCC / 短锁协调器 [锁定]

### 7.1 调查会话与写槽

- **调查内串行**：一个调查同一时刻至多一个活动 round / generation；该调查的 `propose_changes`、`user.ask` 按到达顺序串行处理（以 `investigation_id` 为键的短互斥），不并行。
- **调查间并存**：多个调查可同时 `running` / `awaiting_user`，各自有暂存 change_set，互不可见（§4.4）。这**取代** FX §3 A2 与 §8.2「启动另一轮调查：拒绝」一行。
- **写槽** = 调查当前 generation 的执行授权（FX §8.4.2）。持有写槽才可 `propose_changes` / `user.ask`；进入 `awaiting_user`、取消、暂停、Desktop 重启时写槽释放、旧 generation 失效，迟到请求 → `forbidden`。
- 调查最小状态 [本文提议]：`running | awaiting_user | paused | closed`；只有 `running` 持写槽。
- **调查身份** [2026-11-10 定稿 R1]：沙箱调查 = `sandbox:{session}`（整场会话，所有线程与 round 共享；`thread_id` 只作元数据）。服务端在 `investigation_rounds` 登记 round → investigation，先写者为准；同一 round 之后声明别的调查 ⇒ `forbidden`，`details.reason: investigation_mismatch`。没有适配器声明时，未登记的 round 自成一个调查（调试 CLI）。

### 7.2 短锁协调器

- **所有正式写入**——Agent 提案的接受提交、撤回、手工地图编辑（`commit_map_change`）、来源登记、手稿证据登记、平板同步落库——经同一个服务端协调器，在 SQLite `BEGIN IMMEDIATE` 短事务内串行「检查前置条件 → 写入 → 回执」（FX §8.1；现有 `commit_map_change` 已用 `TransactionBehavior::Immediate`）。
- `propose_changes` 只写暂存表，同样是短事务，但**不改变地图 head**，不与正式写竞争语义（仍受 SQLite 写互斥）。
- 探索期（模型推理、读工具、等待用户）不持任何写锁（FX §3 A3）。

### 7.3 OCC 与多调查冲突

- 读依赖、写前置、集合条件按 FX §8.3 三类收集；Phase2-0 的集合条件由 `plan_map_change` 已有的结构校验覆盖（父链、端点、span），无专门守卫的情形以 `base_commit`（全局地图版本）作保守后备（FX §8.3 第 4 段）。
- 两个调查改同一对象：两者均可 `staged`；先被接受者提交成功；后者在接受预览 / 提交时得到 `Conflicted`（`expected_revision` / `base_commit` 不符），**不自动合并**，按 §5.2 处理。
- 手工编辑与暂存冲突同理：手工编辑立即生效（经协调器），受影响的暂存提案在下次预览时冲突。
- 活动调查运行期间，用户仍可手工编辑、接受 / 丢弃任何调查的提案（FX §8.2 第 4–5 行）。

### 7.4 存储（实现 PR 的方向，非本文锁定 DDL）[本文提议]

追加迁移新增业务表（名称可在实现时定）：`investigations`、`pending_questions`、`proposed_change_sets` / `proposed_change_set_versions`、`change_set_decisions`；2026-11-09 增 `change_set_dependencies`（`dependent_change_set_id`、`dependent_version`、`depends_on_change_set_id`、`depends_on_version`、`referenced_ids`）与 `investigation_rounds`（round → investigation 绑定）；PR-B 实现为 schema 24 的 `proposed_change_set_versions` / `change_set_decisions` / `change_set_dependencies` / `investigation_rounds`，同批接受以决定行上的同一 `commit_id` 关联，不另设 `batch_id`。追加式、不改历史迁移（REV §6 末段）。这些是普通业务表，与「sidecar 打包」无关；Phase0 §1 提到的 `ToolExecutionRecord` 侧车审计表不是 Phase2-0 前置条件（事件 JSONL trace 已覆盖工具调用记录），留待需要时另议。

---

## 8. 错误 / 诊断矩阵（高层）

| 情形 | 工具 / 命令 | 结果 |
| --- | --- | --- |
| `--round` 未绑定调查 | `user.ask` / `propose_changes` | 调用错误 `forbidden`，`details.reason: round_not_bound` |
| generation 失效（取消 / `awaiting_user` / 重启后） | 同上 | `forbidden`，`generation_stale` |
| 同 `request_id` 不同内容 | `propose_changes` / `accept_change_set` | `invalid_arg`，`idempotency_conflict` |
| `supersedes` 指纹 / 版本不是当前活动 version | `propose_changes` | `invalid_arg`，`supersede_conflict` |
| 不支持的 kind/op、引用不闭合、`expected_revision` 过期、planner Blocked/Conflicted、自设可信终态 | `propose_changes` | 业务结果 `status: blocked` + Debt + diagnostics + 指纹 |
| change_set 不存在或属于其他调查 | `get_change_set` | `status: not_found` |
| `scope=investigation` 但无绑定调查 | `graph.*` / `workspace.search` | 按 accepted 回答 + `scope_not_supported` |
| 本调查对同一对象有多个 staged replacement | `graph.*` | 已接受版本 + `staged_conflict` |
| SourceSpan 快照无效 / 越界 / 非 UTF-8 边界 / 原文不符 | `propose_changes` | `blocked`：`snapshot_not_found` / `snapshot_not_immutable` / `range_out_of_bounds` / `invalid_utf8_boundary` / `quote_mismatch` |
| `block` / `page` 粒度无法落到 canonical block / 唯一页 | `propose_changes` | `blocked`：`block_locator_unavailable` / `page_mapping_unavailable` |
| span 范围超过 4096 字节 | `propose_changes` | `blocked`：`span_too_long` |
| 本提案触及的原文证据没有物理定位（无 Markdown 范围、无页、无手稿选区） | `propose_changes` | `blocked`：`invalid_locator`（`target_id` 指出对象） |
| head 里已有、本提案未触及的对象有问题 | `propose_changes` / Desktop 预览与接受 | `warn: head_integrity`（`target_id`），不阻塞 |
| 多页快照没有分页标记 | `material.read` / `material.locate` | 诊断 `page_markers_absent`；span 只能用 `inline` |
| locate 命中但落不到唯一页（跨页、无 PDF 映射） | `material.locate` | 诊断 `page_unresolved`；span 只能用 `inline` |
| `inline` span 其实能唯一落到一页 | `propose_changes` | `info: page_granularity_available`，不阻塞 |
| 列出提案时没有绑定调查 | `list_change_sets` | 空列表 + `investigation_unavailable` |
| locate 无命中 / 多处命中 / quote 超过 4096 字节 | `material.locate` | `not_found` / `ambiguous`（候选上限 20）/ `not_found` + `span_too_long` |
| 同 id span 已退役 | `propose_changes` | `blocked`：`span_inactive` |
| round 已绑定到另一调查 | `propose_changes` / `get_change_set` | `forbidden`，`investigation_mismatch` |
| Desktop 命令的 `investigation_id` 不属于本窗口 | Desktop 提案命令 | 拒绝：`investigation_not_in_workspace` |
| 新建 span 未被引用 | `propose_changes` | `blocked`：`unused_span` |
| `$new:` 未定义；引用的暂存 id 不在本调查活动 staged 中 | `propose_changes` | `blocked`：`dangling_new_ref` / `dangling_staged_ref` |
| change_set 依赖成环 | `propose_changes` | `blocked`：`change_set_dependency_cycle` |
| 所依赖的 change_set 被丢弃 / 替换后缺项 / 变 blocked / 撤回 | 服务端连带 | 依赖者 `staged → blocked`：`dependency_discarded` / `dependency_superseded` / `dependency_blocked` / `dependency_withdrawn` |
| 接受时依赖未接受且不在同批 | Desktop `accept_change_set` / `plan_change_set_batch` | 拒绝：`dependency_not_accepted`，列出缺失项 |
| 接受时 version 非活动 / 非 `staged` / 指纹不符 | Desktop `accept_change_set` | 拒绝，面板刷新 |
| 接受时 `base_commit_conflict` / `plan_conflict` | Desktop | 要求重新预览（§5.2） |
| 撤回会留下非法悬挂 | Desktop 撤回预览 | `Blocked`，不可提交，提示级联 |
| 已撤回再撤回 | Desktop | `already_withdrawn` + 原回执（REV §6 第 3 步） |
| 等待中的 harness 原生请求因取消 / 重启中断 | 适配器 | 空 `answers` 结束请求；答案下一 turn 投递；PendingQuestion 不丢 |

---

## 9. 实现切片与清单

依赖：P2-1 的 Desktop 回答 UI 需要 Desktop 能消费 `events(round_id)`，这与 Phase0 清单 4b 同源；4b 由并行工作完成，本文不实现。

| 切片 | 内容 | 状态 |
| --- | --- | --- |
| **P2-0** | 本契约 `docs/phase2-write-contract.md` + Phase0 §6 指针 | 本 PR（文档） |
| **P2-1** `user.ask` | PendingQuestion 表与调查状态 / 写槽；MCP `user.ask`（非阻塞）；Codex `requestUserInput` 桥（替换现拒绝逻辑）；`agent_events v1` 的 `awaiting_user`；Desktop 待答问题 + 回答 UI；`--round` generation 校验 | 后续代码 PR |
| **P2-2** propose（仅暂存） | 暂存表；`propose_changes` 校验器（§4.2.4）、幂等、`supersedes`、指纹；`get_change_set`；`mcp_envelope v1` `staged/blocked` | 已实现：`crates/mathscape-mcp/src/tools/propose.rs`、`crates/mathscape-store-sqlite/src/proposals.rs` |
| **P2-2b** 投影 | `graph.get` / `graph.neighborhood` 的调查级暂存节点投影；随后 `workspace.search` 暂存命中 | PR-B（§9.1 B5，graph.get 已接入；search 暂存命中后续） |
| **P2-3** 接受面板 | `plan_change_set_accept` / `accept_change_set` / `discard_change_set`（Desktop 专用）；OCC 预览、单项 / 批量接受和依赖传播 | PR-B（§9.1 B6–B7，命令与审核面板已接入） |
| **P2-4** 撤回 | `ChangeSetScoped` 退役计划 / 提交、悬挂检查、级联预览、审计 | 后续代码 PR |

本 PR 锁定：§2 决策、§3 最小口径、§4 工具形状与语义、§5 接受绑定、§6 回滚矩阵、§7 并发规则。各代码 PR 可细化字段名 / 表名（[本文提议] 项），但不得违背 [锁定] 项；如需改变，先改本文。

验收要点（各 PR 自带测试）：
- `propose_changes` 永不改变地图 head（属性测试：任意输入后 `get_map_head` 不变）。
- `blocked` 提案可被 `get_change_set` 读回全文与 Debt；`supersedes` 指纹错误被拒。
- `awaiting_user` 期间该调查 `propose_changes` → `forbidden`；另一调查同时可 propose / 被接受。
- 接受指纹不符、head 变化均拒绝；接受后回执与 `get_change_set.decision.receipt` 一致。
- 撤回会悬挂时计划 Blocked；级联后 Ready；revision 不被删除。
- `tools/list`：前 7 个工具描述除 `scope` 句外字节不变。

### 9.1 PR-B 实现切片与清单（2026-11-09）[本文提议]

本 PR（`feat/pr-b-staged-proposals`）先只提交本契约修订；实现由同分支后续提交完成。**Loogle 加速（常驻进程等）推迟到下一个 PR**，不在 PR-B 范围。

**代码复用（挪到共享模块，不复制代码）**：
- 新建共享模块（建议 `crates/mathscape-orchestrator/src/source_locator.rs`），从原处**移动**并改为 `pub`（或 `pub(crate)` 后经 orchestrator 对外导出给 `mathscape-mcp`）：
  - `selector_resolver.rs`：`locator_matches`（内容发现继续用于全文定位；propose 不用，改用截取后比较的 `quote_matches_range`，R2）、`normalize_locator_text` 及其私有辅助（`strip_outer_math_delimiters`、`tex_spacing_prefix`、`push_normalized_space`、`normalized_matches`、`overlapping_matches`、`NormalizedLocatorText`）；`selector_resolver.rs` 改为 `use` 新模块。
  - `discovery_validation.rs`：`valid_temp_id`（现 `pub(crate)`）。
  - `source_registration.rs`：`stable_hash_id`（现私有）与 span 身份计算——抽成 `markdown_span_id(artifact_id, range) -> StableId`，`plan_source_registration` 的 `intern_span` 与 propose 共用，保证两边 id 一致（加一条跨路径同 id 的单元测试）。
  - `unit_discovery.rs`：`fingerprint`（现 `pub(crate)`）随之暴露或在新模块内复用。
- 不复用的部分说明：`discovery_validation` 的 `quote_occurrences` / `quote_position` 依赖内容发现的 `AuthorizedBlock` 与锚点模型，propose 只需「给定范围的原文 = quote」，用 `quote_matches_range` 对截取出的范围原文判断即可，不搬。`is_ancestor` / 成环由 `validate_live_references` 在最终图上覆盖，不另搬。

**切片（同分支按顺序提交）**：

| # | 内容 | 主要位置 |
| --- | --- | --- |
| B1 | 共享模块搬移（上表），纯重构，现有测试不变 | `crates/mathscape-orchestrator/src/` |
| B2 | 领域：`UserChangeOperation::CreateSourceSpan { object_id, draft: SourceSpanDraft }`；planner 处理（含已有同 id span → 引用）；写入顺序 span→unit（父深度）→relation | `crates/mathscape-domain/src/map_edit.rs`、`crates/mathscape-orchestrator/src/map_edit.rs`、`crates/mathscape-store-sqlite/src/map_edit.rs` |
| B3 | `propose_changes`：`source_span` 项与 §4.2.5 校验 / 补全；`$new:` 解析与裸写兼容告警；`unused_span`；`source_spans` / `evidence_spans` 的引用解析（现只解析 `parent` / `source` / `target`） | `crates/mathscape-mcp/src/tools/propose.rs` |
| B4 | 跨 change_set 依赖：解析到本调查 staged id、依赖表、依赖闭包试编排、成环检查、连带转 `blocked` | `propose.rs`、`crates/mathscape-store-sqlite/src/proposals.rs`、新迁移 |
| B5 | 投影（P2-2b）：`graph.get` / `graph.neighborhood` 的 `scope=investigation`，`staged: true` 字段，`staged_conflict` | `crates/mathscape-mcp/src/tools/graph.rs` |
| B6 | Desktop 命令（实现）：`get_change_set_investigation`、`list_pending_change_sets`（带服务端锚点）、`get_change_set_snapshot`、`preview_change_set_impact`、`plan_change_set_batch` / `accept_change_set_batch` / `discard_change_set_batch`；单个接受走只含一项的批量；锚点由服务端解析、审核投影在前端拼装（§5.6）；协调器同事务写 patch + 决定 | `apps/mathscape-desktop/src/`（新 `change_sets.rs`） |
| B7 | 前端：审核工作区「提案」数据源、接受 / 丢弃按钮、Debt 栏、影响预览栏；批量审核对话框（依赖分组、默认全选、连带取消）；地图「未审批」样式（§4.4.1）；会话切换 / 退出钩子 | `frontend/src/review/`、`frontend/src/sandbox/`、`frontend/src/graph/` |
| B8 | 沙箱接入：round 绑定 investigation、白名单开放 `propose_changes` / `get_change_set`（沙箱 §4.3.1）；skill 升 v0.6.2（运行时能力的沙箱工具列表、`$new:`、span 提案写法），`skill_pack` 测试同步 | `apps/mathscape-desktop/src/sandbox_dialogue.rs`、`skills/mathscape-worknet/` |

验收要点（追加）：
- 内容发现登记与 propose 对同一 `(artifact, range)` 得到同一 span id；已有活动 span 时 propose 不 blocked。
- `quote_mismatch`、`invalid_utf8_boundary`、`unused_span`、`dangling_new_ref` 各有用例。
- 书写顺序打乱（子单元写在父之前、关系写在单元之前）结果不变；父链成环被 `parent_cycle` 拦下。
- B 依赖 A：单独接受 B 被拒；同批接受成功且只产生一个 commit；丢弃 A 时预览列出 B，默认 B 转 `blocked`。
- `propose_changes` 与暂存上图都不改变 `get_map_head`（沿用属性测试）。
- 沙箱接受只改副本工作区，源 checkpoint 字节不变。

---

## 10. 开放风险 / 待核

用户锁定项（§2）无待核。以下为实现前需确认的**非锁定**细节：

1. ~~**[待核] 暂存新建 SourceSpan**~~ **已定（2026-11-09 用户改定）**：见 §4.2.5。原文：Phase2-0 的 `source_unit/create` 只能引用已存在的 span id。若首个调查场景需要 Agent 从 `material.read` 的 locator 新建 span，需要另定「span 也作为暂存对象」的规则（涉及 artifact revision 绑定与登记管线）。
2. **[风险] Codex 长时间持有 `requestUserInput`**：harness 是否容忍数小时挂起未实测；§4.1.3 已规定中断时降级为下一 turn 投递，实测后可能把原生通道也改为「立即降级」。
3. **[风险] 投影成本**：`scope=investigation` 每次合并已接受快照与暂存；暂存规模大时需限额，诊断沿用 1c 截断惯例。2026-11-09 起 Desktop 地图也用这份投影（§4.4.1），依赖闭包试编排（§4.2.4 第 4 步）同样随暂存规模增长。
4. ~~**[待核] 同 id span 已退役**~~：**2026-11-09 用户定**（U1），`span_inactive` → `blocked`，不自动重新激活；重新激活必须是独立的显式操作。
5. ~~**[待核] 批量审核中未勾选项**~~：**2026-11-09 用户定**（U4），保持 `staged`，可稍后再审；沙箱副本整体丢弃时随副本物理删除。
6. ~~**[待核] 多来源 change_set 的审核投影**~~：**2026-11-09 用户定**（U2），一个 change_set 可涉及多个 Markdown 快照，审核面板按快照分组展示；每个 SourceSpan 保留自己的 artifact/range。
7. ~~**[待核] 沙箱 investigation 的粒度**~~：已定（R1），**整场沙箱会话**对应一个 investigation（`sandbox:{session}`），所有线程与 round 共享；切换线程不触发审核，切换 / 丢弃 / 导出会话与退出应用时触发（§5.5）。早先「一条对话线程一个 investigation」的写法作废。

---

## 11. 与既有草案的差异 / 本文新增（发明）记录

| 项 | 既有草案 | 本文 | 性质 |
| --- | --- | --- | --- |
| 活动调查数 | FX A2：Workspace 单活动调查；§8.2 拒绝第二轮 | 调查内串行、调查间并存 | 用户锁定（决策 1） |
| 自动接受 | MCP 草案 §3.2 / FX §12.2：建议首版不允许，未批 | 默认关闭，仅显式接受 | 用户锁定（决策 4） |
| `investigation_id`/`round_id` 输入 | MCP 草案 §3.1 在输入里 | 移出输入，适配器注入 | 沿 Phase0 §2.3 |
| `user.ask` 输出 | MCP 草案 §2.6 返回 `answer_text`（阻塞） | 非阻塞，返回 `question_id`；答案进下一 turn | 本文提议 |
| `get_change_set` 状态 | `staged\|accepted\|rejected\|superseded` | `staged\|blocked\|accepted\|discarded\|superseded\|withdrawn` | `rejected`→`discarded`；`withdrawn` 本文提议 |
| `supersedes` 输入 | 无 | 指纹绑定的修订入口 | 本文提议（实现决策 3「对照指纹重提」） |
| `proposal_fingerprint` 定义 | 无 | §4.2.4 | 本文提议 |
| 可提议对象种类 | MCP 草案 §3.1 示例为 SemanticEntity / FormalArtifact / Alignment | Phase2-0 仅 source_unit / source_relation create/edit | 本文提议（受现有管线约束） |
| ActorRef 格式 | D1 未批 | `agent:<backend>`；接受者沿用 `mathscape-desktop-user`；`generation_run = change_set:<id>@v<n>` | 本文提议 |
| 事件 | `agent_events v0` | v1 增 `awaiting_user` | 本文提议 |
| 状态值 | Phase0 预留 `blocked` | v1 增 `staged` / `blocked`（仅 propose_changes） | 本文提议 |
| 撤回策略 | REV `RegistrationScoped` | 平行的 `ChangeSetScoped` | 本文提议（复用 REV 原语） |
| 调查状态 | FX §8.2 提及 `awaiting_user` / `awaiting_review` | `running\|awaiting_user\|paused\|closed`，无 `awaiting_review`（接受与流程分离） | 本文提议 |
| 新建 SourceSpan | 本文 2026-10-30 版：不支持（待核 1） | `source_span/create`，服务端校验补全，id 复用内容发现确定性规则 | 用户改定（2026-11-09，§2.1 #10） |
| 临时引用 | `<id\|local_ref>` 裸写 | `$new:<local_ref>`，裸写兼容一个 schema 版本 | 用户改定（2026-11-09） |
| 跨 change_set 引用暂存 | 本文 2026-10-30 版 §4.2.4 第 2 条：禁止 | 同调查内允许 + 连带规则 | 用户改定（2026-11-09，§2.1 #9） |
| 审批时机 | 逐个 change_set 显式接受 | 另加会话切换 / 退出时的批量审核（仍显式、整 change_set） | 用户改定（2026-11-09，§2.1 #4′） |
| 暂存可见性 | 仅 Agent 投影 | Desktop 地图「未审批」样式上图 | 用户改定（2026-11-09，§2.1 #11） |
| `staged_ids` 派生 | 本文 2026-10-30 版写 `(request_id, local_ref)` | `(change_set_id, local_ref)`，与现行代码一致 | 本文提议（对齐代码） |
| 替换后的依赖者 | 2026-11-09 版：id 仍在则改指新 version | 一律转 `blocked`，不改指 | 2026-11-10 定稿（R5） |
| 沙箱 investigation 粒度 | 2026-11-09 版：线程级 | 会话级 `sandbox:{session}` | 2026-11-10 定稿（R1） |

---

## 12. 修订记录

| 日期 | 说明 |
| --- | --- |
| 2026-10-30 | 初稿；用户锁定决策 1–8。 |
| 2026-11-09 | PR-B 契约修订（`feat/pr-b-staged-proposals`）：用户改定决策 4（审批时机）与 §4.2.4 第 2 条（跨 change_set 引用），新增 SourceSpan 提案、`$new:` 临时引用、最终图依赖校验与写入顺序、连带规则、暂存上图、批量审核、审核面板与校验代码复用、PR-B 实现切片（§2.1、§4.2.5–§4.2.8、§4.4.1、§5.5–§5.6、§6.1–§6.2、§9.1、§10）。Loogle 加速推迟到下一个 PR。 |
| 2026-11-10 | PR-B 实现对齐：§2.2 汇总四项「2026-11-09 用户定」与七条修订（会话级调查、quote 截取后比较、粒度失败码、hash 语义、依赖带版本且替换后转 blocked、批量一个 commit、单一投影与排除项）；删去线程级调查、全文搜索式 quote 匹配与替换后自动改指；补应用 / 拼接顺序、Desktop 命令清单与归属校验、§8 新码。 |
| 2026-11-10 | 第三轮验收修订（§2.2 V4–V5）：新增只读 `list_change_sets`（§4.3.1），Agent 修订旧提案前先列出再带 `supersedes`；`material.locate` 给 `page_markers_absent` / `page_unresolved`；唯一页时用 `page` 粒度、`inline` 能落到唯一页时给 `page_granularity_available`；`tool_schema_version` 升到 `mathscape-mcp/16`，skill v0.6.4。 |
| 2026-11-10 | 沙箱对话 3 验收修订（§2.2 V1–V3）：手稿笔迹定位算物理定位；只有本提案触及的对象出错才阻塞，head 里已有问题降为 `head_integrity` 警告；`material.read` 单快照也返回 `served_segments`，无分页标记给 `page_markers_absent`；新增 `material.locate`（§4.6）与 4096 字节的 `span_too_long`；`tool_schema_version` 升到 `mathscape-mcp/15`。审核面板改由服务端给锚点、手动审核进全屏审核工作区（§5.6）。 |
| 2026-11-11 | PR #48 验收修订：`staged_ids` 明确为本次提案条数的权威来源（§4.2.2），skill v0.6.6 要求 Agent 按它汇报单元 / 关系 / 证据数量；审核页总览改为显示整份 change set 的总数与全部关系（含跨快照、指向已有对象的关系），每个快照的数字另标「本快照 x 单元 / y 关系」。`tool_schema_version` 不变（`mathscape-mcp/16`）。 |
