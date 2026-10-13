**你的理解基本正确：Codex、Claude Code 这类工具可以被当作具有完整行动能力的“员工”，而不只是一次文本生成调用。Mathscape 没必要重新实现它们已有的内部 Agent 循环。**

但我建议把选择拆成三个问题：

| 层次 | 我的建议 |
|---|---|
| **底层 Agent harness**：模型调用、工具执行、上下文管理、编码与运行环境交互 | 优先复用 Codex／Claude Code 及其官方集成接口 |
| **执行与编排设施**：角色调用、会话、预算、审查、运行记录 | 可以复用新版 Humanize／`hmz`，先验证再决定长期依赖程度 |
| **Mathscape 业务 flow**：什么时候调查、询问用户、建立解释、更新路线、接受图变更 | 由 Mathscape 自己定义，但完全可以用 `hmz` 来执行 |

**所以，我推荐的不是“Humanize 或自研”二选一，而是：自己定义业务流程，复用完整 Agent，把 Humanize 放在一个可以替换的执行层后面。**

你已经提供了闭环所需的反馈：用户的解释、手稿、自我判断、对诊断问题的回答。接下来要解决的是，**如何让这些反馈驱动下一次调查，以及如何让调查结果稳定地进入 Mathscape**，而不是重新论证“能不能形成闭环”。

下面的框架能力来自官方仓库、文档和论文；我没有在你的项目中实际安装、运行这些组合，因此会区分“已有能力”和“建议采用的设计”。

---

# 一、首先要区分：你说的 Humanize，实际有两个值得分别看的形态

## 1. 旧版 Humanize：更接近围绕 Claude Code 的开发、审查闭环

`PolyArch/humanize` 的核心是 RLCR：让执行者完成任务，再由 Codex 等审查者检查结果，结合计划、目标追踪和执行约束反复修正。它的默认流程明显面向代码实现和审查。[GitHub](https://github.com/PolyArch/humanize)

它适合帮助你**开发 Mathscape 本身**。但直接把它的默认循环搬进 Mathscape，让它不断迭代直到“用户理解数学”，并不合适——不是因为你的反馈不成立，而是因为**默认的工作产物、审查对象和结束条件不一样**。

例如，软件开发中的“这一轮没有未解决的代码审查问题”，与学习任务中的“已经针对用户当前困惑提供了足够的解释和练习”，需要不同的判定规则。

## 2. 新版独立框架 `hmz`：更符合你描述的“完整 Agent＋外层 flow”

新版 `humanfia/humanize` 把 flow、运行时、底层 coding agent 和执行环境明确分开。Flow 可以用 Python 编写，决定调用哪些角色、何时新建或复用会话、什么时候审查和停止；底层角色仍然执行完整的自主 Agent 回合，而不是被降成单次 LLM 调用。[HumanFia 文档](https://docs.humanfia.ai/humanize/)

这正适合你的设想：

> Mathscape 提交一项有上下文的数学调查任务，由一个完整 Agent 自行检索、阅读、调用工具、尝试计算或形式化，再交回产物。

而且新版已经提供 SDK、结构化输出、人类角色和可恢复 flow 等机制。因此，不能简单说“它只是几个提示词，必须全部自己重写”。不过，官方仓库仍将其标记为预发布软件，应固定版本并验证你真正需要的接口。[HumanFia 文档](https://docs.humanfia.ai/humanize/reference/sdk)

### “效果很好”应怎样影响你的技术选择？

Humanize 论文报告了实际开发、数学等应用，但作者也明确说明主要证据来自观察性案例，而不是严格控制模型、预算和任务条件的因果比较。它支持“这个方向值得实际尝试”，尚不足以推出“默认流程就是 Mathscape 的最优流程”。[arXiv](https://arxiv.org/html/2610.08900v2)

另外，相关 HOA 数学任务仓库并不只有一份计划文件，还包括固定问题、执行与审查脚本、验证器、环境记录和结果清单。这说明其成功经验更接近：

\[
\text{完整 Agent}
+
\text{任务组织}
+
\text{领域工具}
+
\text{明确验收边界}
\]

而不是单独依靠一份写得很好的 `plan.md`。[GitHub](https://raw.githubusercontent.com/humanfia/hoa/main/README.md)

---

# 二、我推荐的架构：Mathscape 管业务，Humanize 管执行，CLI Agent 管局部自主行动

这里最重要的不是抽象分层，而是**谁拥有哪一种状态，以及谁有权作出哪一种决定**。

## 1. 把 Humanize 视为执行后端，而不是 Mathscape 数据模型的主人

我建议采用以下关系：

```text
Mathscape 交互界面
    阅读选区、手稿、对话、自评、练习
                  │
                  ▼
Mathscape 业务核心
    用户目标、学习证据、路线版本、EMIR/RDTX、图变更验收
                  │
                  ▼
Mathscape 自定义 flow
    决定本轮调查、诊断、解释、审查与暂停
                  │
                  ▼
可替换的 Agent 执行适配器
    初期：Humanize/hmz
    另一种实现：直接接官方 Agent 接口
                  │
                  ▼
Codex / Claude Code 等完整 Agent
                  │
                  ▼
来源检索、图查询、数学计算、Lean、变更提案等工具
```

其中，**“业务 flow 由自己定义”不等于“必须自己开发一个 workflow 框架”**。你完全可以先把它写成 `hmz` 的自定义 flow，或者由 Rust 业务核心调用一个 Python 执行服务。

我会让 Mathscape 保有以下事实的权威版本：

- 用户到底问了什么，以及当前已经澄清到什么程度；
- 哪些反馈确实来自用户，哪些是 Agent 的推断；
- 哪条学习路线、哪个来源版本、哪个解释仍然有效；
- 哪些图变更已经通过相应验收。

Humanize 可以保有运行日志和检查点，但这些日志不能取代上述业务记录。

## 2. 即使将来不用 Humanize，也不必从零开发 harness

这是你这次选型里很重要的一点。

Codex 已有面向应用集成的 App Server，提供双向通信、线程与回合管理、事件流，以及文本和图像输入等接口；Claude Agent SDK 则复用了 Claude Code 的 Agent 循环、上下文管理和工具能力。你不必为了摆脱某个编排框架，就退回“自己写模型—工具—模型循环”。[OpenAI Developers](https://developers.openai.com/codex/app-server/)

所以，真正的替代方案应该是：

> **使用 hmz 编排完整 Agent**，或者**直接编排官方提供的完整 Agent 接口**。

而不是：

> 使用 Humanize，或者从最底层重新造一个 coding agent。

对你现阶段，我更倾向于**先使用 hmz 做一条完整业务纵向切片**，同时让 Mathscape 只依赖自己的任务和事件接口。只有当 hmz 的接口确实妨碍多模态交互、中断恢复或工具控制时，再将某个执行适配器替换为直接接入。

---

# 三、具体怎样将你的两个 workflow 交给这些“员工”？

我不建议把员工划分得太碎。

“识别一个定义”“生成一条边”“总结一个引理”，都可以是工具能力，但通常不值得每项都启动一个完整 CLI Agent。否则你会反复支付环境启动、上下文解释、结果协调的成本。

**适合完整 Agent 的任务粒度，是一个具有明确目标、允许自主选择步骤的调查工作包。**

## 1. Dive into：一个调查员工负责解决一个具体理解障碍

例如，给调查员工的任务不是：

> 解析以下文本，输出所有节点。

而是：

> 用户能跟上这段证明的代数计算，但不知道为什么要转到像空间和核空间。结合选区、用户手稿和已有图，找出这个表示转换的作用，提供一个适合其当前基础的解释，并提出一个能检验理解的变式问题。

这个员工可以自主决定：

读取前文定义，检索另一份解释，检查某个推导，查询 mathlib，构造反例，或者发现必须先问用户一个问题。

这样的任务保留了 coding agent 的优势：**它自行探索、调用工具和修正方案，而不是被外层流程规定每一个微小步骤。**

不过，它交回的应该是几种可区分的产物：

| 产物 | Mathscape 应如何处理 |
|---|---|
| 解释与引用 | 面向用户展示，并记录所依赖的来源与版本 |
| 诊断假设 | 保存为对用户当前障碍的可修订判断 |
| 给用户的问题 | 建立待回答事项，等待真实反馈 |
| 来源／语义图变更 | 经工具契约和相应验收后提交 |
| Lean 结果 | 保存目标、环境、证书与对齐状态 |
| 未解决事项 | 保留为后续调查入口，而不是强行补齐 |

这与你新上传规范要求代理交换 candidate、claim、proof obligation、certificate、review 等结构化对象高度一致；规范明确禁止仅把无来源的自然语言总结当成持久事实。emir-source-semantics-alignment…

## 2. Roadmap：不是一次生成完整路线，而是规划与执行交替

Roadmap 员工可以承担一个稍大的工作包：

> 围绕目标论文中的若干具体障碍，在用户提供的材料中形成近期可执行路线；核验近期阅读选区和习题，保留远期候选分支，并说明各段路线解决什么障碍。

之后，用户进入某个节点时，调用同一套 Dive into 能力。新反馈可能触发局部修订：

> 用户已经能证明这个引理，不必按原计划继续展开；但他不能判断什么时候应该使用它，需要增加迁移任务。

因此两个场景可以共享同一执行体系：

\[
\text{目标与路线}
\rightarrow
\text{局部调查}
\rightarrow
\text{用户反馈}
\rightarrow
\text{解释／练习／路线修订}.
\]

**Roadmap 是长期组织方式，Dive into 是主要的交互工作单元。**

## 3. 审查员工应该按风险调用，而不是每句话都过多 Agent 审查

Humanize 的执行者—审查者模式可以复用，但审查职责应变成 Mathscape 的职责，例如：

> 是否忠实引用材料？是否遗漏条件？是否把类比说成等价？诊断是否有用户反馈支持？图变更是否超出证据？

对于重要的 source-to-formal alignment，可以让审查员工看到原文、形式声明和必要背景，但不看到形式化员工的说服性叙述。你的规范本来就提出了这种独立上下文审查和边界条件攻击。emir-source-semantics-alignment…

普通的低风险解释不必每次都经过几个完整 Agent。**固定的多角色仪式，不应该取代按任务风险分配成本。**

---

# 四、`plan.md` 可以很重要，但需要与任务契约、工具权限配合

我支持使用 `plan.md`。它非常适合保存目标、调查策略、已知障碍和阶段成果。

但最好区分三个东西：

**任务契约**规定这次究竟要做什么、什么结果可以被接受。

**执行计划**规定 Agent 目前打算怎么做，可以在探索中调整。

**调查记录**保存发现、失败、待解决问题和来源。

否则 Agent 可能在遇到困难后，同时改掉自己的计划、目标和验收标准，最后宣布任务完成。

下面是一个**建议的任务契约形状，不是 hmz 的现成 API**：

```yaml
job:
  intent: explain_representation_change
  source_revisions: [...]
  selected_spans: [...]
  learner_evidence_refs: [...]
  interaction_mode: guided

deliverables:
  - explanation_with_source_links
  - diagnosis_hypotheses_with_evidence
  - graph_change_proposals
  - diagnostic_or_transfer_question

constraints:
  preserve_original_sources: true
  allow_partial_formalization: true
  canonical_graph_direct_write: false

possible_outcomes:
  - artifact_ready
  - needs_user_input
  - needs_more_source
  - unresolved

budgets:
  tool_calls: ...
  execution_cost: ...
  review_rounds: ...
```

这里最重要的是：

> **员工完成本轮工作，不等于用户完成学习。**

员工可以成功交付一个解释和一道诊断题。用户尚未回答时，整个学习任务仍然可以处于“待反馈”，而不需要让员工继续循环、自问自答。

### 图更新仍然可以是惰性的、渐进式的

不必等整个调查结束后才保存任何内容。

员工可以在调查过程中调用图提案工具，提交已经有足够依据的来源对象或候选关系；Mathscape 的提交服务根据对象成熟度和证据接受相应变更。高风险的形式对齐和 verified D/T 则需要更严格的门槛。

这保留了你要的：

> 边探索、边积累、按需建图。

只是将“可以提出更新”与“可以自行宣布通过验收”分开。这个边界同样已在 R/D/T/X 规范中明确。emir-source-semantics-alignment…

---

# 五、Humanize 接入中，有几个实际细节不能只靠计划文件解决

这些不是反对使用它，而是决定适配器应怎样设计。

## 1. 人类反馈的身份不能被编排框架混淆

新版 hmz 有 human 角色，但文档说明：人在 away 状态时，文本回答可能为空，带默认值的结构化回答可能直接取默认值。它也允许 flow 代替“用户”回答某些底层 Agent 问题。[HumanFia 文档](https://docs.humanfia.ai/humanize/features/human)

这对自动化开发可能很方便，对 Mathscape 却必须做区别：

> “审查员工认为这个解释清楚”不是“用户回答自己理解了”；  
> “框架提供默认回答”不是“用户完成了诊断题”。

我的建议是让所有学习反馈带有明确来源：

```text
真实用户作答 / 用户自评 / Agent 诊断 / 自动检查 / 审查意见
```

需要真实用户时，产生持久化的 `needs_user_input` 事件，暂停本轮工作。用户稍后上传手稿后，再恢复调查。

## 2. 恢复 flow，不等于恢复原来的所有上下文

hmz 当前的恢复机制会启动新运行，Agent 使用新会话；此前已经消耗的预算也不会自动计入新运行。保留下来的是 flow 明确保存的状态和工作区文件。[HumanFia 文档](https://docs.humanfia.ai/humanize/user/resuming)

因此，Mathscape 应负责保存能够重建任务的上下文：

> 当前问题、已核验来源、用户反馈、已接受图版本、待回答问题和累计成本。

也需要在自己的任务层累计预算，避免“每次恢复都重置预算”。

对于图写入，还应使用任务标识和幂等提交：进程在“提交成功、尚未收到确认”时崩溃，重试不能再创建一份重复实体。

## 3. 自定义工具要按实际接口接入，不能假定所有层都支持同一种方式

当前文档中，底层 agent 运行时提供的 callback tools，与 `hmz.flows` 的接口并不完全相同；flow 指南主要通过 Agent 的询问 hook 让 flow 执行额外动作，不能直接将底层工具示例原样搬过来。[HumanFia 文档](https://docs.humanfia.ai/humanize/reference/agents)

对 Mathscape，我更倾向于把核心能力做成**独立的工具服务**，由执行适配器接到支持的底层 Agent：

> 来源搜索与读取、图查询、候选提交、用户反馈请求、Lean 检查。

这样，工具的业务语义不依赖某一种 flow API。具体采用 MCP、进程接口还是其他本地协议，可以根据所选后端确定。

多模态也一样：Codex 的应用接口支持图像输入，但不能因此假定所有 hmz 后端都以同样方式传递图片。适配器应明确声明图像能力；**在提示中写入一张手稿的文件路径，并不自动证明模型看到了原图。**[OpenAI Developers](https://developers.openai.com/codex/app-server/)

## 4. 沙箱和业务权限是两道不同的门

hmz 默认绕过底层 Agent 的交互审批，但它同时提供基于操作系统的权限限制，例如 Linux Landlock 和 macOS Seatbelt。因此“绕过审批”并不等于“没有沙箱”。[HumanFia 文档](https://docs.humanfia.ai/humanize/user/security)

即便如此，我仍建议工作区只包含必要材料和可写草稿，不直接暴露 Mathscape 数据库及其完整写权限。员工只能通过受控工具提交变更。

还有一个产品化问题：不要从“本地 CLI 能用个人订阅登录”推导出“商业应用可以让所有用户复用这种登录”。Claude Agent SDK 官方文档对第三方产品使用 claude.ai 登录有明确限制，分发前需要采用其支持的认证方式。[Claude](https://code.claude.com/docs/en/agent-sdk/overview)

---

# 六、新上传的 R/D/T/X 规范，比之前看到的材料更直接地支持这套实现

读完这份上位规范，我会修正上一轮回答的一个重心：

**AND/OR 路线、学习者差异、展开层级、增量失效，不是现在才需要补造的新模型；你的规范已经有了。现在主要需要把它们落实为可执行契约。**

尤其是：

\[
R=(R_{\mathrm{core}},P_{\mathrm{operational}})
\]

以及：

\[
P_{\mathrm{operational}}=(\Omega_R,\kappa_R,\mathcal Q,u,L).
\]

这里已经包含任务分布、学习者能力和展开粒度。emir-source-semantics-alignment…

这使用户反馈可以有一个非常具体的用途：

> 用户已经能独立完成某个方法，就可以在其视图中把它折叠为一步；用户在某个隐藏前提上失败，就展开相应 debt；用户不理解表示切换，就展开 T 的 map 和 rebuild。

这不需要为每个用户复制数学实体。变化的是其 `OperationalProfile`、解释视图和学习证据，规范第 19.6 节已经这样规定。emir-source-semantics-alignment…

不过，实际实现有两点必须守住。

**第一，形式依赖不是解释路径。** 规范已经明确，Lean declaration dependency DAG 不等于人类主证明路线、教材解释顺序或最佳证明。因此不能直接导出 mathlib 依赖树，再把它包装成学习路线。emir-source-semantics-alignment…

**第二，标准流水线应当是能力组合，不是每次提问的必经路线。** 上位规范给出了 Lean-native 可信核心优先的实现路线，但也明确不要求全部数学先形式化。面向你现在的产品场景，应明确一个“来源驱动、局部形式化”的执行策略，而不是让每个 Dive into 都跑完文献摄取、形式化、R/T/X 挖掘和经验评估全部阶段。emir-source-semantics-alignment…emir-source-semantics-alignment…

---

# 七、哪些 Lean 工具真的值得用于“理解数学”，而不只是“替用户证明”？

我的筛选标准是：**它能否让某个学习动作变得更可靠、更可观察或更容易交互。**

按这个标准，下面这些工具值得优先看。

## 1. 先给调查员工装上可靠的数学工作台

| 工具 | 已有能力 | 对 Mathscape 的直接用途 |
|---|---|---|
| **mathlib＋Loogle／LeanSearch** | 类型或表达式检索，以及自然语言到形式声明的检索 | 找候选定理、核对前提、发现可用的形式接口；检索命中仍需检查作用域和来源对齐。[GitHub](https://github.com/nomeata/loogle?utm_source=chatgpt.com) |
| **lean-lsp-mcp** | 文件结构、目标状态、诊断、符号信息、运行代码、多种 tactic 尝试 | 让 CLI 员工检查“这里实际还需要证明什么”“这个步骤是否合法”，接入成本相对直接。[GitHub](https://raw.githubusercontent.com/oOo0oOo/lean-lsp-mcp/main/docs/tools.md) |
| **LeanInteract** | 通过 Python 与 Lean REPL 交互、管理命令和证明状态 | 适合实现独立的局部数学检查服务，避免把所有检查都组织成文件编辑任务。[GitHub](https://github.com/augustepoiroux/LeanInteract?utm_source=chatgpt.com) |
| **Pantograph** | 可编程证明状态、分支探索、证明草稿和目标间依赖处理 | 当产品需要“尝试另一条路”“改变局部目标后看后果”时，更值得投入。[arXiv](https://arxiv.org/html/2410.16429v2) |

**第一版不必同时接入这些后端。** 我会先使用 `lean-lsp-mcp`，配合固定版本的 Lean/mathlib 工程；只有当状态分支和大量局部尝试成为明确需求时，再选择 LeanInteract 或 Pantograph 作为更专用的后端。

这里还有一个与 R/D/T/X 直接相关的技术细节：Lean 的多个目标可能共享尚未确定的 metavariable，不能总被当作互不关联的子题分别求解。Pantograph 专门处理这种耦合。因此，抽取 AND 超边时不仅要保存“有哪些子目标”，还要保存它们共享的状态和约束。[arXiv](https://arxiv.org/html/2410.16429v2)

## 2. 最接近你产品理念的，是 Verbose Lean 4

**Verbose Lean 4 特别值得你仔细研究。**

它提供受控自然语言形式的 tactics 和命令，并有点选交互。项目明确说，其目标不是单纯让 Lean 代码更容易写，而是让形式证明更容易迁移到传统纸笔证明；目前展示了英语和法语等材料。[GitHub](https://raw.githubusercontent.com/PatrickMassot/verbose-lean4/master/README.md)

对 Mathscape 的启示不是“要求用户改用这种语言写证明”，而是建立这样的桥梁：

```text
用户的数学动作
    “任取一个向量”
    “按像空间定义引入原像”
    “把目标改写成集合包含”
              ↓
可检查的局部操作
              ↓
新的目标与剩余义务
              ↓
面向用户的解释
```

这很接近你的 D、T、X 操作视图。

当然，受控自然语言并不是任意中文手稿的自动理解器。手稿到这些操作之间，仍然需要 Mathscape 的解释、消歧和用户确认。

**但它给出了一个很有价值的设计方向：让用户学习数学动作，而不是把学习 Lean 语法变成使用 Mathscape 的前置条件。**

## 3. Paperproof 与 ProofWidgets4：让证明过程成为可以操作的界面

**Paperproof** 会读取 Lean 证明过程中假设和目标的变化，并展示其历史与作用域。它当前主要面向以 `by` 开始的 tactic 证明，并以 VS Code 扩展提供使用方式。[GitHub](https://github.com/Paper-Proof/paperproof)

它对你的 Dive into 场景很有价值：用户可以看到某一步使用了哪些假设、拆出了哪些目标，以及哪个分支还没有完成。

不过，它不是开箱即用的 EMIR 可视化器。Mathscape 需要适配其数据提取或展示思想，再补上来源、学习者视图和 R/D/T/X 语义。

**ProofWidgets4** 则是创建 Lean 交互组件的工具集，支持在 Lean 环境中展示更丰富的数学数据和交互界面。它适合探索“与精确数学对象关联的图形”，但并不是自动数学动画生成器。[GitHub](https://github.com/leanprover-community/ProofWidgets4)

我会把二者这样定位：

> Paperproof 帮助展示“证明状态怎样变化”；  
> ProofWidgets4 帮助展示“数学对象和交互怎样连接到形式环境”。

你的动画 IR 可以借鉴后者，但不能把动画画面本身当作证明证据。

## 4. Jixia：很适合惰性构建 Lean 一侧的来源和结构

Jixia 是 Lean 4 静态分析工具，支持单文件分析，可以输出声明、源代码范围、类型、引用关系、elaboration 信息和逐行证明状态等数据。它要求与目标项目使用完全匹配的 Lean 版本。[GitHub](https://github.com/frenzymath/jixia)

这与 Mathscape 的惰性策略非常合拍：

> 不是先把整个 mathlib 转成完整图，而是在某个声明真正参与解释时，按需提取相关文件和结构。

它可以成为 Lean 一侧的来源提取器，为 `LeanArtifact`、声明依赖、证明状态及候选 D 提供数据。

但自动提取出“调用了哪些声明”，还不等于识别出了“人应当看到哪个关键方法”。**提取结果进入 S1/S2；如何形成 S3/S4，仍然需要你的解释和视图策略。**

## 5. Lean Blueprint 与 lean4game：分别补路线说明和练习组织

**Lean Blueprint** 把人类可读的数学说明、依赖关系和 Lean 声明联系起来，很适合研究“解释骨架怎样对应形式化成果”。但其声明检查工具主要检查引用的声明是否存在，不能替代来源忠实性审查。[GitHub](https://raw.githubusercontent.com/PatrickMassot/leanblueprint/master/README.md)

你可以借鉴它构建局部解释包：

> 关键命题、关键构造、依赖和形式化进展。

但不应将其形式化依赖图直接当作个性化 Roadmap。

**lean4game** 则提供关卡、提示、可用定理和交互证明练习的组织方式。它适合借鉴到局部诊断、渐进提示和迁移练习，而不是要求把开放研究全部做成固定关卡。[GitHub](https://github.com/leanprover-community/lean4game?utm_source=chatgpt.com)

### 这些工具还没有替你解决什么？

它们分别提供了检索、状态、核验、可视化和练习机制，但**“某种解释是否真的改善了这个用户的理解”仍然需要用户反馈来判断**。

你的规范对此已有非常合适的指标：能否解压关键宏、识别表示切换、在新问题中迁移 X/T，以及是否在不隐藏关键前提的情况下减少无意义搜索。emir-source-semantics-alignment…

---

# 八、用你规范中的例子，可以看清整个系统怎样工作

你上传的规范已经提供了一个很好的测试案例：平方为零的线性映射，以及通过像空间、核空间得到秩界的路线。emir-source-semantics-alignment…

设 \(A:V\to V\)，\(V\) 有限维，\(\dim V=n\)，且 \(A^2=0\)。目标是理解：

\[
2\operatorname{rank}A\le n.
\]

用户的手稿写道：

\[
A^2=0
\Rightarrow
\operatorname{im}A=\ker A.
\]

并说：

> 我知道应该用秩—零度定理，但不理解这一步为什么成立。

## 第一步：调查员工不急着讲完整证明

它先发现，用户的问题可能不是“不懂秩—零度定理”，而是把包含当成了相等。

于是提出一个有区分力的小问题：

> 在非零空间上取零映射，像空间与核空间相等吗？

用户回答后，Mathscape 记录这次真实反馈，并更新对当前障碍的判断。

## 第二步：按需展开真正需要的数学操作

正确的关键步骤是：

\[
\operatorname{im}A\subseteq\ker A.
\]

证明时，取 \(y\in\operatorname{im}A\)，则存在 \(x\) 使 \(y=Ax\)，所以：

\[
Ay=A(Ax)=A^2x=0.
\]

再结合：

\[
\dim\operatorname{im}A\le\dim\ker A,
\qquad
\dim\operatorname{im}A+\dim\ker A=n,
\]

得到目标。

这里可以交给 Lean 员工检查包含关系和维数推导，交给解释员工说明“为什么要把代数关系转成子空间关系”。

## 第三步：生成与你规范一致的操作视图

| 对象 | 本例中的内容 |
|---|---|
| \(R_{\text{dimension}}\) | 用 rank、nullity、维数不等式工作 |
| \(R_{\text{range/kernel}}\) | 用像、核与子空间包含工作 |
| X/Macro | \(A^2=0\Rightarrow\operatorname{im}A\subseteq\ker A\) |
| D | 在当前表示中调用该宏并完成局部推导 |
| T | 将原来的维数目标归约到包含关系，并用维数定理重建原目标 |

这里尤其要保留：**这个 T 是目标归约，不应为了画图方便就标成两个陈述无条件等价。**

## 第四步：用户反馈真正改变视图，而不是改变数学事实

对于已经理解像空间定义的用户，“取原像 \(x\)”可以是一小步。

对于当前用户，这一步可能正是需要展开的关键宏。后续可以给他一个变式：

> 将条件改成 \(A^3=0\)，原来的包含关系还成立吗？应考虑什么新的包含关系？

用户的回答会影响下一步提示和路线，而不会改写已经验证的数学声明。

**这个例子中，Humanize 负责组织员工；Lean 负责检查局部数学；R/D/T/X 负责保存操作结构；Mathscape 负责把真实用户反馈转化为适合他的下一步。**

这才是几部分结合后的价值。

---

# 九、现阶段，我建议怎样落地

我会先做一个范围明确的纵向实现：

> **一个可中断的 Dive into flow，能够接收选区和手稿，调用来源工具与 Lean 工具，产生解释和图提案，等待真实用户反馈，然后继续。**

具体选择上：

**执行层先用新版 hmz，自定义 Mathscape flow，不直接套用默认的代码审查循环。**

**业务层继续由 Mathscape 管理目标、反馈、来源、图版本和提交权限。** 用一个小的执行适配接口隔离 hmz，不提前实现很多后端，也不急着 fork 整个框架。

**Lean 第一批采用固定环境的 mathlib、检索和 lean-lsp-mcp。** 同时研究 Verbose Lean 的数学动作设计；需要持久提取证明结构时引入 Jixia；需要可视化时再做 Paperproof 风格的状态视图或 ProofWidgets 适配。Pantograph 留给明确需要状态分支探索的阶段。

验证框架是否适合，不应只看“最终解出了多少题”。至少应检查：

| 测试 | 要确认的事情 |
|---|---|
| 用户中途离开后返回 | 不伪造反馈，能从业务记录恢复任务 |
| 提交图变更后进程中断 | 重试不重复写入、不覆盖历史 |
| 来源公式或假设被修正 | 依赖解释被标记需要重新检查 |
| Worker 声称任务完成 | 不自动把用户标记为已理解 |
| 更换执行后端 | 相同任务与图提交契约仍成立 |
| 用户做一个变式问题 | 有证据表明其获得了方法，而不只是看过答案 |

比较 Humanize 与直接调用官方 Agent 接口时，还应把**执行框架**和**流程策略**分开比较：多加一个审查者、多花一倍预算得到更好结果，不能直接归因于框架本身。

---

## 我的最终建议

**可以使用 Humanize，而且值得优先试用；但最值得复用的是它调用完整 Agent、组织角色与运行的能力，不是把它默认的 loop 当作 Mathscape 的业务模型。**

你的实现方向可以概括为：

\[
\boxed{
\text{Mathscape 的业务契约与用户反馈}
+
\text{自定义 flow}
+
\text{可替换的完整 Agent}
+
\text{Lean 与来源工具}
}
\]

在这套组合中，`plan.md` 是员工的重要工作文件，但不是数据库、权限系统和验收器的替代品。

而 Lean 对 Mathscape 最有价值的作用，也不只是“把答案证明出来”，而是：

> **让隐藏前提、局部目标、表示转换、方法展开与尝试后果，变成可以检查、可以展示、可以让用户参与的对象。**

你上传的 R/D/T/X 规范已经为这些对象准备了相当完整的概念位置。现在最有价值的工程推进，是让**一次真实的困惑，经由工具支持的调查和用户反馈，形成一个可以复用的理解过程**，而不是先完成所有图模型与形式化基础设施。
