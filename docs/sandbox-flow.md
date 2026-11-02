# Mathscape 沙箱 Flow 契约（探索床 / 可拆卸副作用）

> 读者：Mob（实现）· 起草：2026-10-30（Asia/Shanghai）· 状态：**契约草案，用户决策已锁定（2026-10-30），待用户审阅本文后合并**
> 依据：[Phase0 契约](phase0-contract.md)（**Phase0**）、[Phase2 写路径契约](phase2-write-contract.md)（**Phase2**）、用户 2026-10-30 对话锁定的「沙箱化 flow」方向。
> 标签：**[锁定]** = 用户已拍板；**[本文提议]** = 为闭合实现补的细节，可在不违背 [锁定] 的前提下微调；**[待核]** = 未定。
> 本 PR **只有文档**，不含 Rust / TS 实现。
>
> **命名**：本文「沙箱 Flow / Sandbox session」是 **Mathscape 产品概念**（可回滚的探索会话）。**不要**与 Codex CLI 的 `SandboxMode`（`read-only | workspace-write | …`，见 Phase0 附）混淆；下文凡指后者写 **Codex sandbox**。

---

## 1. 一句话 / 为什么现在做

**一句话**：[锁定] 在正式「定向调查 flow」（`user.ask` → `propose_changes` → Desktop 接受）钉死之前，先做一层 **可拆卸的沙箱对话体验**：用户用自己的**测试工作区 checkpoint** 开一场会话，本机 Codex + 现有 Mathscape MCP tools 自由对话；会话产生的聊天记录与 Agent 构造的语义副作用可以 **物理删除或整包导出**，从而反复从同一 checkpoint 进入，支撑后续迭代与 prompt / 壳体验探索。

动机（用户原话归纳）[锁定]：
1. 近几模块要建立在**个人体感**上，而不是只看 diff。
2. 先**淡化** workflow 名词（调查状态机、propose/accept 仪式），把壳当成「带 Mathscape 工具的对话面」。
3. 在真实 tools 能力下探索引用方式、面板交互、更合适的 prompt，再反哺 Phase2 定向 flow。
4. **可拆卸**是硬要求：副作用不能污染正式库，也不能在丢弃后残留含糊状态。

与 Phase2 的关系 [锁定]：
- Phase2 契约（已合入 main）仍是定向写路径的权威口径；**本文不修改、不削弱** Phase2 的锁定决策。
- 沙箱是 **先行探索床**；定向 flow（Phase2 §9 实现序）在沙箱摸清壳与 prompt 之后再接，顺序上 **S0/S1 可先于** `user.ask` / `propose_changes` 产品化。
- 沙箱 **允许** 暂不实现完整 propose→accept；试写若存在，必须落在隔离层内（§3），不得写用户选定的正式 / 源 checkpoint。

---

## 2. 范围与 Non-goals

### 2.1 In scope（本契约要定的）

| # | 项 | 说明 |
| --- | --- | --- |
| 1 | Checkpoint | 用户个人测试工作区上的可复现起点（§3.1） |
| 2 | Sandbox session | 一次探索会话的生命周期：打开 → 对话 → 丢弃 / 导出（§3.2–3.4） |
| 3 | 隔离与可拆卸 | 副作用存放位置、丢弃语义、导出包最小格式（§3.3–3.4） |
| 4 | 对话壳（淡化 workflow） | Desktop 最小 UI 与 Agent 挂载（§4） |
| 5 | 与 4b / Phase2 的依赖切分 | 实现切片 S0→S1→S2 与后置项（§5） |

### 2.2 Non-goals [锁定]

- 完整定向 flow UI（`user.ask` 待答仪式、propose 接受面板、retire 预览）——属 Phase2，不在沙箱必达。
- Agent 回答里的任意内嵌 JS / 不受限可执行 widget（富交互声明式块可后置）。
- 虚拟形象系统、多调查自动合并、文本流式 token delta 花活（Phase0 §7）。
- Sidecar 便携打包（Phase0 / Phase2 已推迟）。
- 在**源 checkpoint / 正式工作区**上做破坏性试写。
- 把沙箱会话伪装成 Phase2 `investigation` 的完整状态机（可共用底层 round/事件，但产品语义先是「对话会话」）。

---

## 3. 模型：Checkpoint · Session · 隔离 · 拆卸

### 3.1 Checkpoint [锁定语义 / 本文提议形状]

**Checkpoint** = 用户指定的、可反复回到的工作区起点。

- **来源** [锁定]：用户自己的**测试工作区**（内含可引用的真实对象），不是仓库夹具替代品；夹具仅可作 CI 回归补充。
- **不变式** [锁定]：开启沙箱会话**不得修改** checkpoint 目录（含其中的 `mathscape.sqlite3` 与 artifacts）。所有可变状态进 session 隔离层。
- **标识** [本文提议]：`checkpoint_id`（稳定字符串）+ `checkpoint_fingerprint`（对工作区根关键文件的内容指纹，至少覆盖 `mathscape.sqlite3` 的大小与 mtime 或内容哈希）。打开会话时记录二者；导出包原样带回。
- **打开方式** [本文提议]：Desktop「从 checkpoint 开沙箱」→ 选定已有工作区路径（或已登记的测试库别名）→ 创建 session。

### 3.2 Sandbox session [锁定]

一场会话绑定：一个 `sandbox_session_id`、一个 checkpoint、一个（副本）工作区根、零或多条 round（Codex / AgentExecutor）。

状态（最小）[本文提议]：

| 状态 | 含义 |
| --- | --- |
| `active` | 可对话；MCP `--workspace` 指向隔离工作区根 |
| `exported` | 已导出至少一次；session 仍可继续或随后丢弃 |
| `discarded` | 隔离层已物理删除；终态 |

不允许第三态「半删残留」。`discarded` 后同一 `sandbox_session_id` 不可复活；要继续探索 → 从同一 checkpoint **新开** session。

### 3.3 隔离层 [锁定目标 / 本文提议机制]

**目标** [锁定]：会话内 Agent / 用户通过 Mathscape 能力产生的**一切副作用**（聊天记录、round trace、若有试写对象 / 暂存）都必须落在可整块拆掉的隔离层；源 checkpoint 只读。

**v0 机制** [本文提议]：**会话级工作区副本**。

1. 打开 session 时，把 checkpoint 工作区根 **完整复制**（或等价快照）到 Desktop 管理的沙箱目录，例如：  
   `<app_data>/sandbox_sessions/<sandbox_session_id>/workspace/`  
   （具体根路径实现自定，契约只要求「独立目录 + 可整目录删除」。）
2. `mathscape mcp --workspace` 与 Desktop 打开的图，均指向该副本根。
3. Codex 侧继续沿用 Phase0：对 harness 文件系统的 **Codex sandbox** 仍为 `read-only`（防任意写宿主机）；Mathscape 工具读写的是**副本库**，不是源库。二者正交。
4. v0 **不要求** copy-on-write / 叠加 sqlite；若实现发现全量复制过大，可在不破坏「丢弃=物理删隔离层、源 checkpoint 只读」的前提下换成 COW，属实现优化，须在实现 PR 写明。

**试写策略（分阶段）** [锁定方向 + 本文提议分期]：

| 阶段 | Agent 写能力 | 说明 |
| --- | --- | --- |
| **S0/S1** | **默认只读 tools**（现有 7 个 MCP） | 探索引用、对话壳、事件面板；副本上亦无正式 propose/accept |
| **S1 可选** | 若需「构造语义节点」体感 | 仅允许写入**隔离副本**；仍**不**走 Phase2 正式接受入源图。具体 API 可先用 Desktop 已有 map_edit 的受限调试入口或后续 `propose` 的沙箱适配——实现 PR 选定一种并写进清单 |
| **定向 flow 接回后** | Phase2 `propose_changes` 等 | 沙箱可演进为「投影在副本上预览接受」，但源 checkpoint 仍只经显式「晋升 / 不在本文」路径；本文不定义晋升 |

### 3.4 拆卸：丢弃 vs 导出 [锁定]

两种互不含糊的出口：

#### 丢弃（默认安全出口）[锁定]

- 动作：物理删除该 session 的隔离目录（含副本工作区、transcript、trace、本地缓存）。
- 结果：源 checkpoint **字节级不变**；无「软删除残留」。
- UI：结束会话或「丢弃本次」；需一次确认（不可撤销）。

#### 导出（保留为样本）[锁定语义 / 本文提议包格式]

- 动作：打出一份自描述目录或归档，**不**代替丢弃；导出后 session 仍可继续，直到用户丢弃。
- **最小包内容** [本文提议]：

```
sandbox_export_<session_id>/
  manifest.json          # 见下
  transcript.jsonl       # 用户消息 + Agent 可见回复（按时间）
  events.jsonl           # 可选：AgentExecutor §3 事件原样（若已接通 4b）
  side_effects/          # 可选：本会话在副本上新建/改动的对象快照或 change 列表
```

`manifest.json` 最小字段 [本文提议]：

| 字段 | 含义 |
| --- | --- |
| `schema` | 固定 `"mathscape.sandbox_export.v0"` |
| `sandbox_session_id` | 会话 id |
| `checkpoint_id` | 起点 id |
| `checkpoint_fingerprint` | 打开时指纹 |
| `created_at` / `exported_at` | RFC3339 带时区 |
| `backend` | 如 `codex` |
| `tool_allowlist` | 本会话启用的 MCP 工具名列表 |
| `notes` | 可选，用户一句话 |

不要求导出可再导入为活会话（v0）；导出是**样本与对照**，不是迁移格式。若日后要「从导出恢复」，另开契约。

---

## 4. 对话壳（淡化 workflow）[锁定方向]

### 4.1 产品语义

沙箱壳 = **带 Mathscape 工具的对话面**，不是调查状态机面板。

- 不强调 `investigation` / `awaiting_user` / `change_set` 等 Phase2 名词（实现内部可复用 round_id）。
- 用户感知：选 checkpoint → 开聊 → 看工具调用与回复 → 丢弃或导出。

### 4.2 Desktop 最小 UI（S1）[本文提议]

| 元素 | v0 要求 |
| --- | --- |
| 入口 | 「沙箱」或「探索会话」：选 checkpoint、开 / 列 / 丢弃 / 导出 |
| Flow 面板 | 可折叠；建议贴边 dock + 可拖锚点的圆形入口（点=展开/收起）；快捷键展开 |
| 历史 | User / Agent / System 角色条（颜色+短名即可）；**不做**虚拟形象系统 |
| 事件 | 消费 `events(round_id)`：turn / tool_call·result / error / cancel（依赖 4b，线协议仍是 `agent_events v0` 七类）。每轮回复**之前**用纵向可折叠过程卡展示步骤（进行中自动展开，结束后收成「已完成 N 步 · Xs」）；气泡上有状态头（正在调用 / 等待审批 / 已完成 / 失败）。不做向右生长的状态芯片条。 |
| 待答 / 提案条 | **不**作为 S1 必达；有 `user.ask` 后再加 |

### 4.3 Agent 挂载 [锁定]

- 本机 **Codex**（已有 `CodexExecutor`）+ `mathscape mcp --workspace <sandbox_copy> --round <round_id>`。
- 工具面 v0 = Phase1 已有 **7 个只读工具**；allowlist 可配置，默认全开只读集。
- Codex sandbox `read-only`、`ephemeral: true` 沿用 Phase0 附。`approvalPolicy` 用 `on-request`（本机 Codex 的 serde 枚举是 kebab-case：`untrusted` | `on-request` | `granular` | `never`；`onRequest` 会被拒绝为 unknown variant），**不用** `never`：缺少 `readOnlyHint` 的 MCP 工具会被 Codex 当成破坏性调用，`never` 会直接拒绝并报 `approval policy is never`。Mathscape 7 个只读工具在 `tools/list` 上带注解 `readOnlyHint: true`、`destructiveHint: false`、`openWorldHint: false`；`thread/start` 另设 `mcp_servers.mathscape.default_tools_approval_mode = "approve"` 与 `enabled_tools` 白名单。适配器接受白名单内的只读 MCP；shell、文件写入、额外权限、动态工具与打开外链一律拒绝。未能归类的审批最多等 60 秒，超时记「等待审批已超时」并拒绝。以上不得写入源 checkpoint。

### 4.4 引用与视图信号（S2，薄）[锁定可后置 / 本文记方向]

- `@` / 点选 → 对象 chip（带 `object_id`）；候选来自 search / graph / 最近上下文。
- 工具命中在图上短高亮（读路径回声）；**不做**完整 staged 幽灵层（那是 Phase2 接受路径的事）。
- 富交互声明式块、沙箱 iframe widget：**后置**，本契约不展开。

---

## 5. 实现切片与依赖

### 5.1 建议顺序 [锁定方向]

| 切片 | 内容 | 依赖 | 用户体感 |
| --- | --- | --- | --- |
| **S0** | Checkpoint 登记/选择；副本隔离；丢弃；导出（manifest+transcript 最小集） | 无 | 同一 checkpoint 反复进；丢弃后源库不变 |
| **S1** | 对话壳 + 本机 Codex 挂上 7 tools；最小事件条 | S0；**4b** 事件消费（可与 S1 同批薄做） | 真实 tools 对话 |
| **S2** | `@` 引用 chip + 读路径图上高亮 | S1 | 引用顺手、图有回声 |
| **∥** | Phase1 硬化、Phase2-0 领域模型 | 与沙箱并行 | 多为非壳 |
| **后** | 定向 flow 接回（Phase2 §9） | 沙箱体感 + Phase2 模型 | 仪式化 ask/propose/accept |

### 5.2 与 Phase0 清单 4b

4b（真机 Codex + Desktop 消费事件）是 S1 事件条的前提。允许：
- **同 PR** 做「4b 最小事件条 + S1 壳」，或
- 先合极薄 4b，再合 S1。

不得在无事件流时假装「对话已接通」。

### 5.3 文档义务（每个实现 PR）[本文提议]

PR 描述固定三节（用户体感调试约定，保持简单）：
1. `用户能看见的变化`
2. `如何从 checkpoint 点一遍（≤8 步）`
3. `刻意没做`

不强制 Mock 回放基建、不强制 GIF；导出样本可选附在 issue。

---

## 6. 待核

| # | 问题 | 建议默认 | 为何可后定 |
| --- | --- | --- | --- |
| T1 | 副本策略：全量复制 vs COW | v0 全量复制；过大再 COW | 不改变只读 checkpoint 不变式 |
| T2 | 沙箱内是否首版就开放「副本上试写」 | S1 默认只读；试写另开子切片 | 先稳壳与 tools 体感 |
| T3 | 导出是否需要 `side_effects/` 必选 | v0 可选；有试写后再必选 | 只读阶段 transcript 已够 |
| T4 | checkpoint 是否支持「只读绑定远程/只读卷」 | 首版仅本地目录 | 降低 Desktop 复杂度 |

无项阻塞 S0 文档合并；实现 PR 选定 T1/T2 时在 PR 正文写明。

---

## 7. 本文提议汇总

| 项 | 落点 |
| --- | --- |
| 会话级工作区副本作隔离层 | §3.3 |
| 沙箱目录布局与 `discarded` 终态 | §3.2–3.3 |
| 导出包 `mathscape.sandbox_export.v0` | §3.4 |
| S1 UI：dock + 角色条，无虚拟形象 | §4.2 |
| S0→S1→S2 切片与 4b 同批许可 | §5 |
| 与 Codex `SandboxMode` 的命名区分 | 文首 |

---

## 8. 修订记录

| 日期 | 说明 |
| --- | --- |
| 2026-10-30 | 初稿。用户锁定沙箱探索床、可拆卸副作用、淡化 workflow；同意先写短契约再实现。 |
| 2026-10-30 | S0 实现状态（`mob/sandbox-s0-base`）：新 crate `mathscape-sandbox` + Desktop「沙箱」面板。T1=完整复制、T2=只读。布局 `<app_data>/sandbox/{checkpoints.json, sessions/<id>/, exports/, discarded.jsonl}`（可用 `MATHSCAPE_SANDBOX_ROOT` 覆盖）；默认导出到 `exports/`（会话目录外，丢弃后保留）。契约本身未改。 |
| 2026-10-31 | S1 实现状态（`mob/sandbox-s1-shell`）：对话 dock（User/Agent/System 角色条 + 最小事件条，消费 `agent_events v0` 七类事件，4b 与 S1 同批薄做）；后端为本机 Codex app-server + `mathscape mcp --workspace <会话副本>` 的 7 个只读工具（T2 仍为只读）。每轮单 turn、ephemeral，历史以 transcript 回放进提示词（`turn_start.input_ref` 即完整提示）。会话目录新增 `events.jsonl`，导出包随附。窗口内切换：沙箱「在本窗口打开…」/ 顶栏「返回母工作区」均先确认再切换，沙箱中顶栏常驻琥珀色「沙箱」横幅；切换不持久化（重启回母工作区），沙箱中不自动启动手写局域网。可用 `MATHSCAPE_CODEX_PROGRAM` / `MATHSCAPE_MCP_PROGRAM` 覆盖程序路径。对话 dock 支持经 `AgentExecutor` 适配器选择本机 Codex 模型（`current_model` / `list_models` / `set_model`，读写 `~/.codex/config.toml` 与 `models_cache.json`）；并可选择推理强度 effort（`current_effort` / `list_efforts` / `set_effort`）：可选档位取自所选模型在 `models_cache.json` 的 `supported_reasoning_levels` / `default_reasoning_level`（缓存缺失时回退 app-server `model/list` 的 `supportedReasoningEfforts`），持久化到 config.toml 顶层 `model_reasoning_effort`（与 Codex 同键），每轮经 `turn/start.effort` 下发；切换模型时若当前 effort 不受新模型支持则重置为该模型默认档。契约本身未改。 |
| 2026-10-31 | A-PR1 壳层卫生：`tauri dev`/`tauri build` 先 `cargo build -p mathscape-cli [--release]`，`mathscape` 与 `mathscape-desktop` 同目录（`MATHSCAPE_MCP_PROGRAM` 仍优先）；适配器把 Codex `mcpServer/startupStatus/updated` 与 `mcpServerStatus/list` 折成一对 `mcp.startup` 工具步骤（`raw.mcp`），对话卡显示「MCP 工具 7/7」/「MCP 启动失败：…」；`usage` 不再成步骤，按轮求和进摘要（0/0 隐藏）；`mathscape --help` 列出全部 7 个工具。契约未改。 |
| 2026-11-01 | A-PR2 对话侧栏（A2）：对话 dock 新增两种停靠模式，同一套 UI（日志、过程卡、输入框、模型/推理强度、状态）不分叉。`sidebar`（默认）贴右侧、顶栏以下全高，地图区让出等宽空间（`.shell.has-sandbox-sidebar` + `--sandbox-sidebar-width`），地图不被遮挡；左缘拖动或 ←/→ 调宽（300–900px，保证地图区 ≥780px）。`floating` 即 S1 浮窗，拖动/右下角缩放不变。标题栏「浮动」/「侧栏」一键切换；收起仍为可拖动浮标，展开回到上次模式。窗口宽 <1080px 时侧栏偏好按浮窗渲染（偏好不变）。持久化键：`mathscape:sandbox-dialogue-mode`、`mathscape:sandbox-dialogue-sidebar-width`（原 `-size`/`-pos` 保留给浮窗）。仅前端；引用 chip / 节点跳转 / LaTeX 留给 A-PR3，S2 引用契约未改。 |
| 2026-11-01 | A-PR3 引用 chip / 地图回显 / LaTeX（`mob/sandbox-a-pr3-cite-latex`）：对话中的稳定 id（规范 `mathscape:<kind>/<hash>` 或裸 32/64 位 hex）在用户/Agent 气泡与过程卡参数/结果里统一渲染为 chip，显示标签、悬停看短 id 与完整 id；解析顺序为精确匹配 → 同 hash 前缀变体 → 本轮工具结果索引 → 当前地图静态索引，多候选标「歧义」，地图外但本轮结果中有标签的为虚线标签 chip（不可跳转），查不到的为灰色等宽短 id（title 带完整 id，不可跳转）。点击 chip 经回调 `selectNodeById` 在地图上选中并居中，侧栏保持打开。工具驱动回显：`tool_call` 的参数 id 以琥珀色 `sandbox-echo-seed` 点亮（全不在视口时柔和平移），`tool_result` 返回节点以青色 `sandbox-echo-hit`（多节点时连同其间的边 `sandbox-echo-edge`）点亮，约 2.6 秒淡出，上限 40 个，下一次工具调用清除；不持久化、不造假节点、手写层打开时不回显。Agent/用户气泡改为 markdown-it + KaTeX（复用 `markdownRenderer()`，链接不跳转），过程卡摘要仍为纯文本。共享 `shortStableId` / `parseMathscapeId`（`core/stable-id.ts`）。刻意没做：输入框内 chip 化、过程卡回放回显、手稿 document_id → 地图跳转。仅前端，契约未改。 |
| 2026-11-02 | Map Scene Player 第一刀（`mob/sandbox-map-scene`）：A-PR3 的 seed/hit 回显换成声明式场景播放器。每个 MCP `tool_call`/`tool_result` 编译成一个 `ToolScene`（`frontend/src/sandbox/scene/`：标题、摘要、seeds、按层的 focus 节点、是否在地图上、是否截断），播放器（`frontend/src/graph/scene-player.ts`）独占镜头与焦点：Announce（地图右上 HUD，中文标题如「查看节点 · 引理 1.4」「邻域 · X · 深度 2 · 双向」）→ Isolate（复用聚焦模式的 `is-out-of-focus` 0.12 淡化，不隐藏）→ 镜头 fit 到焦点（不重新布局、不聚团）→ seeds 琥珀、`graph.neighborhood` 按深度逐层青色点亮 → 空闲 1.5 秒后 Settle（清样式、恢复用户原焦点范围与镜头、解锁）。播放中锁住地图交互（侧栏与输入不受影响），HUD 有「停止」；手稿层打开时不播放、不回放。焦点上限 60 个（默认邻域 `max_nodes` 40 + seeds 可容纳；seeds 与浅层优先，摘要注明「上限 60」）。回放：过程卡每步「▶」单步回放，标题栏「▶ 回放全部」整轮连续回放（HUD 显示 n/总数）；场景由已持久化的 `events.jsonl` 重新编译，动画状态不写入景观数据、不造假节点。引用 chip 跳转仍用 A-PR3 的单次脉冲。刻意没做：Act 层叠窗（搜索打字机、`pdf.outline` 小窗、手稿页图窗）、局部物理聚团、phase2 写入形变、输入框 chip、手稿 document_id 映射到地图。仅前端，契约未改。 |
| 2026-11-02 | Map Scene Player 第二刀（`mob/sandbox-map-scene-act`）：在第一刀的 Announce 之后加 Act 层叠窗，只接三个工具。`workspace.search`：HUD 里多一行打字机，逐字打出 query（35–90ms/字，总长约 1.2 秒），打完停 0.45 秒再点亮命中；query 为空就跳过。`pdf.outline`：结果为 ok/truncated 时，在 artifact 节点旁开「PDF 目录」小窗（依次尝试右/左/下/上，放不下就钳在舞台内；节点不在图上时贴舞台左下）。小窗列出 `{title, level, 第 pdf_page_index+1 页}`，最多 60 行，页脚注明「仅显示前 X 条，共 Y 条」；没有目录时写「这份 PDF 没有书签目录」，带 `outline_unreadable` 诊断时写「目录无法读取」。`truncated` 现按成功处理，不再显示为错误。`ink.page_image`：开「手稿页图」小窗，显示 PNG、note_title 和「第 n/m 页」。图像块只收 png/jpeg/webp/gif 的合法 base64，转成 data URL；缺图或加载失败时降级为文字元数据「未取得页图（仅元数据）」。`document_id` 不登记成地图节点，有快照/artifact 节点时贴着它们，没有就贴舞台。小窗在 Settle、停止或下一场景开始时关闭，不写入任何持久数据。回放（单步/整轮）从 `events.jsonl` 重新编译，同样重放 Act。手稿开启策略不变：手稿层打开时整段场景（含 Act）不播放、不回放，与第一刀一致。第一刀的约束保留：0.12 淡化、播放中锁图、动画状态与图数据分离、中文文案。刻意没做：局部物理聚团、phase2 写入形变、输入框 chip、手稿 document_id 映射/跳转到地图。仅前端，契约未改。 |
| 2026-10-31 | S1 审批与过程卡（仍在 `mob/sandbox-s1-shell`）：§4.3 的 `approvalPolicy` 从 `never` 改为 `on-request`，只读 MCP 注解 + `default_tools_approval_mode=approve` + `enabled_tools` 白名单，shell / 写入仍拒绝，未归类审批 60 秒超时。§4.2 事件条改为回复前的纵向可折叠过程卡。线协议仍是七类事件。不引入 ACP。 |
| 2026-10-31 | 修正：`thread/start.approvalPolicy` 的线上值是 kebab-case `on-request`。本机 Codex 报 unknown variant `onRequest`，期望 `untrusted` / `on-request` / `granular` / `never`。 |
