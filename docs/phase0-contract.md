# Mathscape Phase0 契约（实现清单）

> 读者：Mob（实现）· 日期：2026-10-27（Asia/Shanghai）
> 依据：user↔Mob 对齐结论（2026-10-27）；字段级 schema 以 `mathscape-tools-mcp-draft.md`（下称 **MCP 草案**）为准，本文不重复粘贴。
> 外部参考：**MCP 草案**与**调研**（`mathscape-core-workflow-调研.md`）目前位于仓库外的研究目录（`/workspace/research/`），尚未入库；本文引用其章节号（如「MCP 草案 §2.2」「调研 §4.6」）。相关仓库内文档：[humanize-agent 提案](../proposals/humanize-agent.md)、[core-workflow 设计](../proposals/core-workflow-design.md)、[架构](architecture.md)。
> 版本：`agent_events v0` + `mcp_envelope v0`。Rust 实现见 `crates/mathscape-agent`（DTO、`AgentExecutor` 桩、`FakeBackend`、CapabilityProbe、JSONL 与配对校验）；安装脚本见 `scripts/install/`。
> 定位：**薄 Phase0**。本文是实现清单，不是评审提案。标 **[锁定]** 的条目已对齐，可直接编码；标 **[待定]** 的不阻塞 Phase0/Phase1。

---

## 1. Purpose & Non-goals（目的与不做什么）

**Phase0 只冻结"怎么说话"**：工具怎么被调用、事件怎么上报、外部能力怎么探测。

Phase0 锁定 [锁定]：
1. MCP 公共信封：工具命名、`status`/错误码、locator / scope / revision 约定（§2）。
2. AgentExecutor 事件类型表（§3）。
3. CapabilityProbe 返回形状 + 外部包"先安装脚本"的约定（§4）。
4. `AgentBackend` 枚举与适配器边界（§5）。

明确推迟（不在 Phase0 做，也不在 Phase0 设计细节）：
- EMIR 完整状态机（D2）、TrustRecord / Debt 关联（D3）、ActorRef 格式（D1）→ Phase2 写路径前再定。
- `propose_changes` / `get_change_set`（MCP 草案 §3）→ Phase2。
- 侧车表 / `ToolExecutionRecord` 迁移 → 随写路径。
- 调研报告 §5 原 Phase0 "EMIR 枚举/状态选型 1 周" **整体取消**，改为本文的薄契约。

**两个面分开** [锁定]：

| 面 | 方向 | 载体 | 用途 |
| --- | --- | --- | --- |
| MCP tool surface | Agent → Mathscape | `mathscape mcp` stdio JSON-RPC | Agent 调用 Mathscape 读（以后写）能力 |
| Realtime sync | AgentExecutor → Desktop | 进程内 event bus → Round / trace | 桌面实时显示进度、usage、取消 |

MCP **不是**桌面实时同步通道；桌面不通过 MCP 订阅进度。两者唯一交集：MCP 的每次 `tools/call` 由适配器在事件总线上镜像成 `tool_call`/`tool_result` 事件（§3）。

---

## 2. Public MCP Envelope（公共信封）

### 2.1 命名 [锁定]
- 工具名 `<domain>.<verb>`，小写 snake：`workspace.search`、`material.read`、`material.grep`……（清单见 MCP 草案 §2、附录 A）。
- 对 harness 注册时 server 名为 `mathscape`；harness 侧显示为 `mathscape.workspace.search` 等，避免与 LeanExplore / lean-lsp-mcp 冲突（MCP 草案 §4）。
- `tools/list` 顺序固定、描述文本固定，不随任务增减（缓存前缀稳定，调研 §4.5 第 1 层）。
- 启动形态：`mathscape mcp --workspace <path> --round <token>`（MCP 草案 §0.1、§4）。`--round` 在 Phase1 可为占位，只读工具不校验。

### 2.2 结果状态 `status` [锁定]
每个工具的 structured result 顶层带 `status`：

| status | 含义 | 典型场景 |
| --- | --- | --- |
| `ok` | 正常返回 | — |
| `not_found` | 目标 id / artifact 不存在 | `material.read` 给了无效 `artifact_id` |
| `ambiguous` | 输入可解析为多个目标，返回候选 | 以后 `context.resolve_symbol`；search 不用 |
| `truncated` | 结果有效但被硬顶截断 | `material.read` 超 32KiB；search 超 `limit_per_group` 时不算，仅正文截断算 |
| `capability_unavailable` | 依赖的外部能力缺失/版本不符 | PDF 无文本层且 Marker 未装；Lean 未装 |

- 这 5 个是**业务结果**，以 MCP 正常 result 返回（不是 JSON-RPC error），Agent 可据此自行重试/换路。
- `truncated` 时同时保留 MCP 草案 §2.2 的 `truncated: true` 字段（两者一致）。
- `capability_unavailable` 时 `details` 附 CapabilityProbe 结果（§4），含 `capability` 名与 `install_hint`。
- **调用错误**（参数非法、内部异常）沿用 MCP 草案 §2 公共错误形 `{ error: { code, message, details } }`，`code ∈ invalid_arg | forbidden | internal`。MCP 草案里的 `not_found` 归入上表 `status`；`blocked` 仅属 `propose_changes`，Phase2 再用。

### 2.3 locator / scope / revision [锁定，字段见草案]
- **locator**：二选一 `md_byte_range {start,end}`（字节，半开区间）| `pdf_pages {from,to}`（1-based，闭区间）。服务端**回显实际返回范围**。见 MCP 草案 §2.2、§2.3。`expand.before/after` 单位统一为**字节**（草案原为待定，此处锁定）。
- PDF 内部页索引（`pdf.outline` 的 `pdf_page_index`）为 0-based，与 locator 的 1-based 并存，文档里写清（草案 §2.4）。
- search 的 `highlights` 保持现有 UTF-16 `[start,end)`（草案 §2.1 [事实]），不改。
- **scope**：`accepted | investigation`。Phase1 只实现 `accepted`；收到 `investigation` 时按 `accepted` 返回并附 `diagnostics: [{code:"scope_not_supported"}]`，不报错（草案 §1.2）。
- **revision**：输出可选字段；没有 revision 来源时**省略**而非填假值（草案 §1.2）。输入里的 `revision`（graph 类）Phase1 不实现。
- `investigation_id` / `round_id` 由适配器注入，Agent 不可传（草案 §0.4）。

---

## 3. AgentExecutor 事件（稳定契约）[锁定]

AgentExecutor 是 Desktop 唯一面向的执行接口；各 harness 由适配器把原生事件映射到下表。事件写入 Round trace（JSONL Artifact，调研 §4.6），同时推给桌面 event bus。

公共字段：`{ type, round_id, turn_id?, seq, ts, backend }`。`seq` 在 round 内单调递增；`ts` 为 RFC3339 带时区；`backend` 见 §5。

| type | 关键 payload | 何时发 | 备注 |
| --- | --- | --- | --- |
| `turn_start` | `turn_id`, `input_ref`（输入包 hash/引用） | 一轮开始 | 一个 round 可多 turn |
| `turn_end` | `turn_id`, `outcome: completed\|cancelled\|failed`, `final_text_ref?` | 一轮结束 | 与 `turn_start` 必须成对 |
| `tool_call` | `call_id`, `tool`, `server`（`mathscape`\|外部名）, `args_summary` | Agent 发起工具调用 | 外部 MCP 工具也报（能观测到时） |
| `tool_result` | `call_id`, `status`（§2.2 或 `error`）, `duration_ms`, `output_hash?` | 工具返回 | 与 `tool_call` 用 `call_id` 配对 |
| `usage` | `input_tokens`, `output_tokens`, `cached_input_tokens?`, `cost_usd?` | 每 turn 至少一次（可累计） | 字段对齐 `mathscape-llm` 已有 usage 映射 |
| `cancel` | `requested_by: user\|system`, `ack: bool` | 用户取消请求 / 适配器确认 | 先 `ack=false`（请求），后 `ack=true`（已停） |
| `error` | `code: backend_unavailable\|auth\|protocol\|internal`, `message`, `fatal: bool` | 适配器或 harness 出错 | `fatal=true` 后必跟 `turn_end(failed)` |

规则：
- 适配器**不得**发明新 type；harness 特有信息放 `payload.raw`（可选，原样保留便于回放）。
- 未知/无法映射的 harness 事件：丢弃或塞进下一条事件的 `raw`，不新增 type。
- 新增 type 需改本文档（版本号 +1）。Phase0 版本：`agent_events v0`。
- 文本流式增量（assistant delta）**Phase0 不定**；需要时作为 v1 加 `message_delta`。

---

## 4. CapabilityProbe + 安装脚本 [锁定]

外部依赖统一用 CapabilityProbe 探测，结果三态：

```jsonc
{ "capability": "marker", "state": "available"|"missing"|"version", 
  "found_version": "1.2.0"|null, "required": ">=1.0", "install_hint": "scripts/install/marker.sh" }
```
- `version` = 找到了但版本不满足 `required`。
- Phase0 探测对象：

| capability | 用途 | 探测方式（建议） |
| --- | --- | --- |
| `marker` | 无文本层 PDF → markdown | `marker --version` / Python 包版本 |
| `lean` | 以后 `lean.check` / lean-lsp-mcp | `elan show` / `lean --version`、`lake --version` |
| `codex` | Codex 后端 | `codex --version` |
| `claude_code` | Claude Code 后端 | `claude --version` |
| `hmz` | 可选适配器 | `hmz --version`；missing 不影响任何默认流程 |

- **先安装脚本**：每个 capability 对应 `scripts/install/<capability>.sh`（Windows 另配 `.ps1` 可后补）。脚本幂等、可重跑、只装到用户目录/venv，不改系统全局。
- Desktop 设置页展示 probe 结果 + 一键运行脚本；MCP 工具遇缺失返回 `capability_unavailable`（§2.2）。
- Probe 结果可缓存，启动时与用户点"重新检测"时刷新。
- **sidecar 打包（给网络受限用户的离线包）推迟**，不在 Phase0/1。

---

## 5. Backend Adapters（harness 无关）[锁定]

```
AgentBackend = Codex | ClaudeCode | Hmz | Custom
```
- **默认优先：humanize 式包装 + Codex / Claude Code**。Phase1 起先接其一，另一个紧随（谁先由账号/图像实测决定，不阻塞编码）。
- **Hmz 是可选适配器，不是必需的 flow 引擎。** 没装 hmz，Mathscape 全部默认路径照常工作。
- **Custom**：自建 flow（含将来基于 `mathscape-orchestrator` 的流程）也实现同一个 AgentExecutor 接口，产出同一套 §3 事件。
- 每个适配器只负责：启动/会话管理、把 `mathscape mcp` 注册给 harness、事件映射（§3）、取消、注入 `round_id`。
- 工作流（skill.md / tools.allow 等，调研 §4.4）是**数据**，与后端无关；**Phase0 不要求任何具体工作流**，hmz `dive-into` 不是 Phase0 要求。

AgentExecutor 最小接口（语义，非具体签名）：
- `start_round(backend, workspace, input_ref, tools_allow) -> round_id`
- `events(round_id) -> stream<AgentEvent>`
- `cancel(round_id)`
- `probe() -> [CapabilityProbe]`（各后端自报可用性）

---

## 6. Deliverables Checklist（交付清单）

### Phase0（文档 / DTO / 桩，无业务逻辑）
- [x] 本文入库 `docs/phase0-contract.md`，作为 `agent_events v0` + `mcp_envelope v0` 的单一来源
- [x] DTO：`ToolStatus`（5 值）、`ToolError`（`invalid_arg|forbidden|internal`）、`Locator`、`Scope`
- [x] DTO：`AgentEvent`（7 type + 公共字段）、`AgentBackend` 枚举、`CapabilityProbe`
- [x] trait/接口桩：`AgentExecutor`（§5 四个方法，返回 `unimplemented`/空实现）
- [x] `FakeBackend`（测试用）：按脚本吐出一串合法事件，供桌面 UI 和 trace 落盘先联调
- [x] CapabilityProbe 实现（只读探测，最简单）+ `scripts/install/` 目录与至少 `marker.sh` 一个脚本
- [x] 事件 JSONL 序列化/反序列化 round-trip 测试；`turn_start/turn_end`、`tool_call/tool_result` 配对校验

### Phase1 首批代码（垂直切片：只读）
1. [ ] `mathscape mcp` 空壳：`initialize` / `tools/list` / `tools/call`（MCP 草案 §4）
2. [ ] `workspace.search` 包装现有 `SearchRegistry`，不另写匹配（草案 §1.2、§2.1）
3. [ ] `material.read`（md_byte_range + pdf_pages，32KiB 硬顶 → `truncated`）（草案 §2.2）
4. [ ] 接一个真实后端（Codex 或 Claude Code）跑通：Agent 调 search → read，桌面看到 §3 事件
5. [ ] 然后 `material.grep`（线性扫描，草案 §2.3）
6. [ ] 之后再排：`pdf.outline`、`graph.get/neighborhood`、`ink.snapshot_image`、`user.ask`
- 写路径（`propose_changes`）不进 Phase1。

---

## 7. Out of Scope（明确不做）
- `propose_changes` / `get_change_set` / 暂存投影 / 接受面板
- 完整 EMIR 状态机、TrustRecord 规范对齐、Debt、ActorRef（D1–D3）
- sidecar 离线打包
- 在 Mathscape 内嵌官方 CLI（Codex / Claude Code / Lean / Marker 由用户环境或安装脚本提供，Mathscape 只探测和调用）
- 把 hmz 或任一工作流（如 dive-into）作为默认/必需流程
- 文本流式 delta 事件、FTS5、向量检索

---

## 附：与 MCP 草案的差异记录
| 项 | MCP 草案 | 本文 |
| --- | --- | --- |
| 错误码 | `not_found\|invalid_arg\|forbidden\|blocked\|internal` 统一为 error | 业务结果拆为 `status`（5 值）；调用错误保留 `invalid_arg\|forbidden\|internal`；`blocked` 留给 Phase2 |
| `expand` 单位 | 字节或字符待定 | 锁定字节 |
| 第三方/hmz 地位 | hmz 用于实验 flow（调研 D5） | hmz 为可选适配器，非必需 |
| 实现顺序 | search → read → outline → grep | search → read → 接真实后端 → grep；outline 后移 |

---

## 附：Phase0 实现说明（`crates/mathscape-agent`）

以下为落地时的线格式细化，不改变上文语义：

- **事件线格式**：每行一个 JSON 对象，公共字段平铺，类型相关字段放在 `payload` 下，可选 `payload.raw` 原样保留 harness 原生数据：
  `{"round_id":"r1","turn_id":"t1","seq":3,"ts":"2026-10-27T15:00:00+08:00","backend":"codex","type":"tool_call","payload":{"call_id":"c1","tool":"workspace.search","server":"mathscape","args_summary":"…"}}`
- **`turn_id`** 只作为公共字段出现（不在 payload 内重复）；`turn_start` / `turn_end` 必须带。
- 未知 `type` 与 payload 未知字段在反序列化时拒绝（防止适配器发明新 type）。
- `tool_result.status` 取 §2.2 五值或 `error`；`usage` 字段名对齐 `mathscape-llm::LlmUsage`。
- 配对校验 `validate_round`：单一 `round_id`、`seq` 严格递增、`turn_start/turn_end` 成对、`tool_call/tool_result` 按 `call_id` 成对、`fatal` error 后下一个 `turn_end` 为 `failed`、`cancel(ack=true)` 前须有 `cancel(ack=false)`。
- **locator** 线格式为外部标签：`{"md_byte_range":{"start":0,"end":10}}` 或 `{"pdf_pages":{"from":1,"to":2}}`；调用错误包装为 `{"error":{"code","message","details"}}`。
- **CapabilityProbe**：`found_version` / `required` / `install_hint` 恒输出（无值为 `null`）；Marker 要求 `>=2.0, <3`（与 `mathscape-tool-marker` 支持的 2.x 一致），按 `$MATHSCAPE_MARKER_EXECUTABLE`、`$MATHSCAPE_MARKER_VENV`、`scripts/install/marker.sh` 默认 venv、`PATH` 顺序查找 `marker_single` 并读取同目录 Python 的 `marker-pdf` 版本；找到但读不到版本记为 `version`。其余能力用 `<cmd> --version`。
- `AgentExecutor` 为对象安全 trait（`Box<dyn AgentExecutor>`）；真实适配器 Phase1 起实现，Phase0 仅 `FakeBackend` 与 `UnimplementedExecutor`。
