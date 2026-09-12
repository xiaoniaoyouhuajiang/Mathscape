# 分阶段来源内容发现协议草案

状态：M1、M2 已实现；M3～M4 待实现。尚未替换当前 Desktop 的
DiscoveryProposal v3。日期：2026-09-10。
本文定义下一版工作流的阶段、输入输出语义、交接条件与修订边界；字段名作为实现依据，最终
JSON Schema 由 Rust DTO 生成。开发顺序见 [开发任务](discovery-staged-development-plan.md)。

实施细节见 [M2 说明](discovery-staged-m2.md)。根据后续讨论，每个 B 批次保留完整授权选区
及其连续顺序，任务只约束提取归属。B 输入升级为 mathscape-source-facts-input-2，增加
leading_gap、boundary；首次输出事实契约仍为 mathscape-source-facts-1。

## 1. 目标与范围

从用户选定的 Markdown 中发现可追溯的数学言语单元、来源标签、引用提及及原文明示关系，
为来源登记与后续数学语义解析准备候选。两个 LLM 阶段分别处理单元结构和依赖该结构的事实。
程序负责短句柄、文本定位、范围授权、运行预算与结果组合。

一次工作流不等于两次 API 请求：小输入通常每阶段各一次；第二阶段可以分批，失败可以有界
修订。不承诺两次调用后全部成功，也不把验证成功当作数学正确、人工接受或已经入图。
DialectScope、符号解释、全局数学对齐、Lean 验证和来源登记不属于本协议的两个 LLM 阶段。

## 2. 阶段与交接

| 阶段 | 执行者 | 输入 | 输出与交接条件 |
| --- | --- | --- | --- |
| P0 输入准备 | 程序 | 受管 Markdown、用户选区、有效运行设置 | 可信 DiscoveryInput、请求局部块绑定表 |
| A 单元发现 | LLM + 程序验证 | 可见原文、结构提示、边界信息 | 单元、范围、parent、完整性、issues、coverage；通过检查才形成冻结的 UnitSet |
| P1 任务准备 | 程序 | ValidatedUnitSet | 按明确 owner 分配的事实发现任务；分开标注证据可用范围与只读上下文 |
| B 事实发现 | LLM + 程序验证 | 已固定单元目录、任务原文与 owner | 来源标签、原文标题、引用提及、关系、issues；不能修改 UnitSet |
| P2 组合 | 程序 | 同一 UnitSet 下的有效批次 | 规范化定位、引用解析状态、覆盖与运行记录，组成 DiscoveryResult |
| 后续接受与登记 | 用户/策略 + 程序 | 已验证的完整结果或明确的闭合部分 | AcceptedCandidate、受控来源登记；独立开发 |

A 或 B 的验证失败均可进入同阶段的有界修订。A 的结构错误不会自动转交 B 处理。
B 发现需要重新切分单元时产生反馈，停止受影响任务；不能在 B 响应中重命名单元或改动边界。

## 3. 身份、指纹与权限

复用现有 MarkdownSnapshot、UTF-8 半开字节区间与用户选择协议。用户选区仍是不可扩大的
授权上限；教材中存在的外部引用不赋予自动读取整书、其他文件或地图的权限。

模型只看到短句柄：块片段 `bN`、冻结单元 `uN`、事实任务 `tN`、提及 `mN/rN`、修订候选
`cN`。这些都不是地图 StableId。模型响应只回显请求短句柄 `request_ref`；系统将它绑定到
本次请求及其完整指纹，不再要求模型复制 SHA256。句柄不能跨请求单独使用。

系统记录以下依赖：

- source input fingerprint：原文快照、选区及可见文本；
- unit set fingerprint：输入身份、阶段 A 契约、规范化单元及其范围/parent/完整性；
- task input fingerprint：UnitSet、owner、证据片段、只读上下文、单元目录与任务契约；
- attempt identity：阶段、批次、次数、Prompt 指纹、解析器/验证器修订及模型运行配置。

请求内每个 `bN` 绑定一份确定的可见文本。若同一物理块被裁成多个不连续片段，必须使用不同
句柄，不可沿用只按物理块 ID 去重的绑定规则。每个片段保留内部字节回链和裁切标记。
同一批次只发送一份块片段表，各任务引用它；occurrence 始终相对于这一份实际发送的片段。
重分批或改变可见片段后重建绑定和任务指纹。

用户的审核策略和密钥不进入模型内容。阶段 B 的上下文通过独立任务输入建模，不静默改变
当前 DiscoveryInput v2 的 `context=[]` 契约。

模型侧输入 envelope 统一采用 version、request_ref、blocks，加各阶段专属字段。A 专属字段
为 boundary；B 专属字段为 units（冻结目录）和 tasks。B 每个 task 明确列出 task_ref、
owner_unit_ref、evidence_block_refs、context_block_refs；内部范围由绑定表提供，不让模型
计算字节偏移。任务的 evidence/context 句柄集合互斥，context 同时标明是否位于 owner 内部，
供关系证据检查。任务计划本身参与输入指纹，响应不能修改计划。

A 的 ref 在提案内唯一。B 不再要求模型生成标签、引用和关系的机械 ID；验证器按 task_ref
及各集合的规范顺序生成 l1/r1/rel1，跨批 origin 使用 UnitSet fingerprint + task_ref +
系统 ID。旧响应中的事实 ref 只作回放兼容且没有身份权限。阶段输出使用封闭对象、必需数组
和显式 null；未知字段拒绝。关系目标只能引用冻结目录，不能引用另一模型响应中未交接的
临时对象。标题/提及等自由文本也是数据，不作为指令执行。

## 4. 阶段 A：单元发现

### 4.1 输入与输出

输入包含 `request_ref`、块片段表、原文顺序、Markdown 结构提示、裁切状态及文档边界。
Markdown 块不是数学单元：同一个块可含两个单元，一个单元也可跨多个块。

输出 `UnitDiscoveryProposal`：

| 字段 | 语义 |
| --- | --- |
| version、request_ref | 阶段输出契约和本次请求绑定 |
| units | 每项包含 ref、unit_kind、display_title、selectors、parent、completeness |
| issues | 当前单元的边界、分类、归属、原文缺失或转换疑点；沿用现有 issue 种类和补上下文意图 |
| coverage | 对所有非空白输入的 processed / skipped / unprocessed 处置 |

unit_kind 沿用当前 Discovery 的 section、definition、statement、proof、problem、hint、
solution、example、remark、method、notation、narrative。此阶段不输出 source_labels、
原文标题锚点、references 或 proves/cites 等内容关系。display_title 是简短生成说明。
parent 表示原文组织归属；解答可以属于习题、证明可以属于命题，不强求两者物理嵌套。

selectors 继续支持 block_range 与 SelectorIntent v2 的 quote_range。完整块直接选块；
遇到“命题结尾和证明开头在同一块”等边界必须精确选择，不能靠扩大范围制造相邻单元重叠。

### 4.2 验证与冻结

检查版本/请求绑定、字段和预算、单元 ID 唯一、选择器获授权、范围非空、parent 存在且无环、
完整性与 issue 一致、coverage 无缺口且单元位于 processed 区域。

物理嵌套且有 parent 祖先链的结构允许；没有归属依据的兄弟交叠必须修订。阶段 A 不依赖尚未
生成的 explains 边为交叠放行。真正的多角色交叠暂时标为结构待处理；可保留供审核，但不能
直接作为自动任务分配依据。这个首版限制必须在测试中单独计数，不能让模型编造 parent。

验证后的单元 ID 原样固定到本次 UnitSet，无须二次编号。冻结只表示本轮阶段交接稳定，
不表示永久不可修改或已进入数据库。修改范围、ID、分类或 parent 会产生新的 UnitSet 指纹。

partial A 只有在返回的单元结构有效、剩余区域有明确 unprocessed 标记时才可冻结其有效
部分。首版不从一个结构非法的提案中静默挑选“看似正确”的几个单元；修订后仍非法则停止 A。
用户未来接受部分结构，也必须先重新验证新的闭合子集。

## 5. P1：事实任务分配与范围归属

每个 `FactTask` 明确给出 task_ref、owner_unit_ref、evidence_regions 和 context_regions。
模型在该任务组内输出的标签和引用，其 source 由任务的 owner 提供，不再逐条生成 source。
批次响应仍需回显 task_ref，系统检查任务是否确实属于当前批次。

分配依据是已验证的原文组织关系，不是无条件选择“包含位置的最小 unit”：

1. 没有物理交叠的单元各自拥有其范围。
2. 在 parent 祖先关系与严格物理包含同时成立时，包含区域优先分配给后代；父单元只处理
   剩余直接内容，其余文本可作只读上下文。多层嵌套递归处理。
3. parent 指向物理分离的习题/证明时，不作范围扣除。
4. 同范围的多个角色、无祖先关系的交叠或无法唯一确定直接归属时，不自动选 owner；生成
   ownership_ambiguous，先修订 A 或由审核明确任务归属。

一个 owner 的剩余证据范围可以不连续。任务必须显式枚举可用片段，引用不得跨越被扣除的
子单元文本。片段切断一个提及、或上下文暗示归属不当时，B 可报告需要修改结构。

每批带冻结单元目录：ref、类型、展示标题、parent 和范围摘要。只提供本次 UnitSet 的候选；
目录并不授予以其他单元原文作证据的权限。关系任务的 source 也来自 owner，target 取自目录。
关系 evidence 必须位于 owner 的已验证单元范围且实际出现在任务中；可以使用显式标为
owner 内部的只读上下文来解释整体单元，但不能把相邻单元的上下文当作 source 证据。

一个批次可包含多个 FactTask，不要求每个单元各调用一次 LLM。按输入字节、输出预算和单元
边界分批；单个巨大证明无法容纳时，标记任务过大并保留未处理状态，不能静默截取后声称完成。

## 6. 阶段 B：来源事实发现

### 6.1 输出

`SourceFactProposal` 包含 version、request_ref 和 tasks。每个任务结果回显 task_ref，包含
disposition、source_labels、source_title、references、relations、issues。所有数组必需；
可空字段显式 null。每个被安排任务必须恰好返回一项，不能发明任务或遗漏而暗示已经完成。

| 项目 | 最小内容与规则 |
| --- | --- |
| source_labels | 每项 anchor；每个原文标签一项，所属单元与临时 ID 由程序确定 |
| source_title | null 或原文标题的 TextOccurrence；与生成的 display_title 分开 |
| references | 每项 anchor、target_hint；hint 为目录中的 unit ref 或 null，仅作候选提示 |
| relations | 每项 relation、target、evidence、basis；source 与临时 ID 由程序注入 |
| issues | 当前 owner 的疑点、原文证据和可空 context_request；可报告需要修改单元结构 |
| disposition | processed / unprocessed(reason)；分批应足够小，首版不接受任务内部隐式半完成 |

processed 表示模型按任务检查过，不保证穷尽所有数学事实。unprocessed 任务的事实数组为空、
source_title 为 null，解释保留在 reason；已经生成的碎片只进入尝试记录，不作为有效事实。
对“没有发现标签/引用”的任务返回 processed 和空数组，不能和未处理混淆。

### 6.2 单次文本出现协议

标签、标题和引用统一使用 `TextOccurrence`：

```json
{
  "kind": "text_occurrence",
  "block": "b24",
  "text": "1.3.3",
  "occurrence": 2
}
```

text 只写一次；不用另写 surface_form、start.quote 和 end.quote。最终原文表述由系统从
绑定位置读取。首次输出不要求 prefix/suffix；重复时提供 occurrence 或等待候选选择修订。

解析顺序：

1. block 必须是当前任务中实际可见且允许该用途的句柄。
2. 枚举该片段内 text 的全部精确匹配，支持重叠匹配。
3. 精确零匹配时才尝试既有的保守排版规范化：外围数学定界符、空白、已列举的 TeX 间距。
   不等号、变量、上下标、否定、括号内容等不作数学等价替换。
4. occurrence 为 null 时要求所用匹配层级中恰好一个候选；非 null 时为该层级从 1 开始
   的序号。0 或越界一律拒绝。不能先按 owner 过滤候选再偷偷改变计数口径。
5. 检查选中位置在任务允许范围内，读取原文精确文本和字节范围，并保留原始 text 及规范化日志。

验证器独立检查正式位置、原文与用途授权。不会再拿模型的近似 text 与修正后的原文作重复
严格相等检查；模型原文和规范化原文同时保留，明确哪个是最终证据。

同一片段中两次引用相同目标应记录两次出现。默认不把 ambiguous 自动解释成 all；如需
完整记录，两处都应由模型选择。完全相同的出现和角色在组装时去重，但保留各次提案来源。

跨块的单次提及使用首版逃生分支 `range_occurrence`，其 payload 只有一个既有 quote_range
selector；规范原文从范围读取，不另写 surface_form。允许它是为了处理真实换页/换段拆分，
不能用它包住整段以掩盖重复引用。可解析的同块 range_occurrence 由程序规范化为精确原文
与 occurrence，不扩大授权；跨块或异常宽的提及产生定位精度警告，供修订/审核。程序不能
仅凭“范围有效”认证其提及边界在语义上正确。常规同块提及仍应使用 text_occurrence。

### 6.3 标签和引用的语义

标签记录原文标识符本体，例如 3.3.1、§ 3.4、(20)，不把 Markdown 的 **、###、类型词和
标题一起当编号。原文逐项打印 (20)、(21)、(22) 时输出三项，不合成为不存在的范围。
原文确实使用整体范围作为声明标记时可原样保留，不能无证据展开其全部成员。

reference 的 anchor 尽量包含足以辨认目标的完整提及，如“§ 2.3(27)”而非单独“27”。
引用中出现的 (29b)~(29d) 不能自动转为当前单元的声明标签。标签是否承担声明作用、提及是否
完整属于 LLM/审核判断，字符存在与范围检查只能提供必要条件。

target_hint 不为 null 时须是本批目录中的其他单元；自指返回 invalid_reference。本单元内部
引用、输入外引用、目标不确定时均可用 null，不为满足图闭合而新建空壳 unit。

relation 沿用当前 proves、solves、hints、explains、cites、applies_method 的端点类型和
方向规则。evidence 可用完整块或 quote_range，并需满足任务可见性及 source 范围约束。
proves 只表示原文提供证明的关系，不认证证明正确。找不到真实 target 时保留 reference 或
issue，不生成悬空的正式关系端点。

## 7. P2：规范化结果与本地目标查找

所有同 UnitSet 的有效批次先收集标签，随后统一查找引用目标，避免批次先后顺序决定解析结果。
一个有效但较早完成的 B 批次不因其他批次失败而重新调用模型。

内部结果包括 UnitSet、每项事实的 owner、规范定位、原始输出与重写关联、阶段覆盖、issues
和每个引用的解析状态。它不是当前单次 v3 DTO 的伪装版本；使用独立 DiscoveryResult 契约。

| 引用状态 | 含义 |
| --- | --- |
| unresolved | 未检索、输入外、缺少声明标签、组合表达暂不支持或同单元内部目标 |
| candidate | 有模型提示或文本候选，但证据不足以确定 |
| resolved_local | 文档/已知编号作用域/标签类型一致，有唯一可核对的声明锚点 |
| ambiguous | 有多个合理目标，保留候选供后续选择 |

查找限于本次 UnitSet 的已验证标签。只有受支持的编号语法、明确的来源作用域和唯一候选
同时成立时才自动绑定；“只有一个字符串相同的标签”不充分。没有编号作用域知识时保持
candidate/unresolved，不能推测邻近章节的编号规则。模型 hint 本身不等于 resolved_local。

组合引用如“式(21)、(22)与(24)”保留一条原文提及，内部可有多个目标匹配。只有所有显式
分项都解析成功才把整体记为 resolved_local。首版不支持的范围表达保持 unresolved，不把
端点匹配成功当作整个范围已解析。

同单元内标签可以作为内部引用候选记录，首版不生成 unit→自身的图边。未来若引入正式标签
端点，可单独晋升内部引用。跨单元 cites 与同一 reference 派生出的重复边合并，保留证据。
ReferenceMention 到原文的绑定可以已验证，同时目标保持 unresolved；这不会使来源事实非法。

## 8. 有界修订与错误报告

修订由编排器调度，单次 Producer 仍只负责一次调用。默认每个 A 请求、每个 B 批次最多一次
自动修订；总 token 上限、批次上限和工作流期限可以进一步禁止修订。传输失败不自动算作
可修复提案，也不无条件重试。手工重试创建新 attempt，保留旧诊断。

程序按可检查条件汇总错误：先做字段/ID/自指/任务成员等不依赖定位的检查，再解析位置并做
归属/覆盖检查。未解析的位置不会触发依赖它的假阳性错误；依赖项标记 not_checked。
保持成功结果和失败报告的类型隔离，不能为了多展示诊断而构造已验证对象。

| 情况 | 反馈与允许的操作 |
| --- | --- |
| 多个文本候选 | 系统提供 c1/c2、原文上下文和顺序；模型选择候选 |
| 文本零匹配、未知块、序号越界 | 提供对应原文片段；模型替换该项 anchor |
| unit 边界或 parent 错误 | A 返回修订后的完整单元提案并重新验证/冻结 |
| B 的归属、target、自指、关系错误 | 替换指定任务结果；不能改 UnitSet |
| B 发现必须改 UnitSet | 返回 requires_unit_revision，受影响任务 pending；不自动循环返回 A |
| 截断 JSON、无终止事件、超时 | 分别记录响应/传输失败；不猜测补齐 JSON 后接受 |

候选选择请求绑定 base proposal fingerprint、问题路径和候选集；模型只回显短 repair_ref、
issue_ref 和 candidate_ref。候选必须在当前允许范围内。程序将选择转成正式 anchor，再完整
验证该阶段或任务；不能把选择候选视作绕过验证。候选过多时截取展示并明确 has_more，模型
可修订更具体的文本，不得对未展示候选猜序号。

B 修订替换整个受影响任务结果，不引入任意 JSON Patch；未受影响任务不重生成。修订后
仍错误时保留未验证提案供查看，不自动放宽规则。已修订 UnitSet 产生新指纹后，首版使旧 B
结果全部失效并要求重跑；更细的依赖失效优化留待后续。

RepairResponse 的固定 envelope 为 version、repair_ref、actions；动作仅允许
choose_candidate(issue_ref, candidate_ref)、replace_units(完整 A 提案)、
replace_task(task_ref, 完整 B 任务结果)、defer(issue_ref, reason)。repair 输入列出该次允许
的动作/任务集合；A 与 B 的替换动作不混用。对同一问题或任务的冲突动作拒绝，模型不能删除
原本合法的其他任务以逃避验证。defer 保留 needs_attention，不被解释为已修复。

## 9. 完成、预算与 Desktop 行为

工作流状态与内容状态分开：running、completed、partial、needs_attention、failed、cancelled
描述运行；unit completeness、阶段 A coverage、B task disposition、reference resolution
各自保存。completed 要求 A 已验证并覆盖全部请求、B 全部计划任务已验证且 processed；外部
引用 unresolved 不影响这一条件。这个 completed 仅表示发现执行完毕，不保证没有遗漏。

partial 结果明确列出 A 的 unprocessed 区域和 B 的未完成任务。无效批次不能整批按有效事实
加入结果。只有实际验证通过的任务及闭合结构可组合；缺少事实发现的单元可显示为“单元已
发现，标签/关系待发现”，不能显示“无引用”。接受和来源登记仍是下一工作流。

Desktop 保持一次“内容发现”入口，按阶段显示“识别单元 / 验证单元 / 发现来源事实 i/n /
修订定位 / 组合结果”。活动任务锁覆盖整个工作流，包括修订与批次间隙；切换阅读页不释放锁。
流式输出按阶段/批次/attempt 分组，区别未验证文本、检查点和有效结果。

新工作流把用户配置的发现总超时作为整个工作流期限，每次调用只能使用剩余时间；连接超时
仍是单次连接上限。开始时快照预算/配置，明确记录每阶段耗时和 usage（缺失值不是零）。
输出预算按批次设置，不直接假定现有 8192 tokens 足够；到达期限或预算上限后保留检查点。
首版检查点可以在内存中，退出应用不恢复须在 UI 明示；持久恢复作为单独开发项。

## 10. 完整的小例子

自编 Markdown 被分成三块：

```text
b1: **命题 1** 若对所有 n 都有 a_n=c，则数列收敛于 c。
b2: **证明** 对任意 ε>0，取 N=1；n>N 时，|a_n-c|=0<ε。
b3: 由定义 2 得出结论。这也说明本证明满足定义 2 的全部条件。
```

本片段没有定义 2 的正文。阶段 A 响应示例：

```json
{
  "version": "mathscape-unit-discovery-1",
  "request_ref": "a1",
  "units": [
    {"ref":"u1","unit_kind":"statement","display_title":"常数列收敛",
     "selectors":[{"kind":"block_range","first":"b1","last":"b1"}],
     "parent":null,"completeness":"complete"},
    {"ref":"u2","unit_kind":"proof","display_title":"常数列收敛的证明",
     "selectors":[{"kind":"block_range","first":"b2","last":"b3"}],
     "parent":"u1","completeness":"complete"}
  ],
  "issues": [],
  "coverage": [{"selector":{"kind":"block_range","first":"b1","last":"b3"},
                "disposition":{"status":"processed"}}]
}
```

程序验证并固定 u1/u2，任务 t1 的 owner 为 u1、证据范围 b1；t2 的 owner 为 u2、证据范围
b2～b3。阶段 B 输入包含上述任务表、块表和冻结单元目录。下面的同批示例沿用 b1～b3 便于
阅读；实际必须以该批任务构建器提供的句柄为准。

```json
{
  "version": "mathscape-source-facts-1",
  "request_ref": "b_request1",
  "tasks": [
    {"task_ref":"t1","disposition":{"status":"processed"},
     "source_labels":[{"anchor":
       {"kind":"text_occurrence","block":"b1","text":"1","occurrence":null}}],
     "source_title":null,"references":[],"relations":[],"issues":[]},
    {"task_ref":"t2","disposition":{"status":"processed"},
     "source_labels":[],"source_title":null,
     "references":[
       {"anchor":{"kind":"text_occurrence","block":"b3","text":"定义 2","occurrence":1},"target_hint":null},
       {"anchor":{"kind":"text_occurrence","block":"b3","text":"定义 2","occurrence":2},"target_hint":null}
     ],
     "relations":[{"relation":"proves","target":"u1",
       "evidence":[{"kind":"block_range","first":"b2","last":"b3"}],
       "basis":"原文在命题后明确给出证明并得出其结论。"}],
     "issues":[]}
  ]
}
```

模型不重复生成 source 或事实 ID。程序由 t2 得到 source=u2，并为两个引用生成 r1/r2，定位
其不同字节范围。
定义 2 没有声明目标，两条 reference 都保存为 unresolved；proves(u2,u1) 有真实端点和证据。
若 occurrence 均为空，程序反馈两个候选供修订，不自行选第一个或生成两次引用。

## 11. 版本与兼容

当前 [DiscoveryProposal v3](discovery-proposal-v3.md) 保持已实现契约。新方案作为下一代
管线开发，不能在 v3 原名下替换字段或改变 input fingerprint 的语义。

| 维度 | 拟定标识/规则 |
| --- | --- |
| 管线版本 | mathscape-discovery-pipeline-2，表明阶段及交接语义 |
| A 数据契约 | mathscape-unit-discovery-1；任务输入另有 input version |
| B 数据契约 | mathscape-source-facts-1；任务输入另有 input version |
| 修订契约 | mathscape-discovery-repair-1，明确候选选择和任务替换 |
| 组合结果 | mathscape-discovery-result-1，不冒充旧单次提案 |
| Prompt 修订 | 在各阶段 manifest 单独记录 revision 和内容指纹 |
| 解析/验证修订 | 独立记录；变化后重验缓存，不直接复用旧“通过”状态 |

这些名称在实现前可统一调整；不需要为两个新阶段分别称为 Proposal v4。v4 只在仍沿用
同一种整体 Proposal 家族时才有必要。新阶段采用明确名称和自身版本，可减少编号混淆。

已有 proposal_2/3/4 与 diagnose 文件保留原样作为历史实验。没有丢失语义的迁移器时，不把
旧 JSON 改版本号后当新响应回放。新方案先与 v3 离线对比，再切换 Desktop 默认路径。
