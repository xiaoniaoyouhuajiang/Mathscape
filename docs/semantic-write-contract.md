# Mathscape Phase2 契约延伸：语义层写路径（`propose_changes` 的语义对象）

> 读者：实现者与审阅者 · 起草：2026-11-13（Asia/Shanghai）· 状态：**第一条语义层 PR 的实现契约**（用户 2026-11-13 已对 §S13 的待决项拍板，见 §S13）
> 定位：`docs/phase2-write-contract.md`（下称 **P2**）的延伸章节，编号用 **S** 前缀，接在 P2 §4.2 之后；P2 未提到的规则一律沿用 P2。本文与 P2 冲突处以本文为准（仅限语义层对象）
> 依据：EMIR v0.2（`docs/rdtx/emir-source-semantics-alignment.md`，下称 **EMIR**）、R/D/T/X 规范 §4.2、`crates/mathscape-domain/src`（`entity.rs`、`trust.rs`、`map_edit.rs`、`source/*`、`semantic/*`）、`skills/mathscape-worknet/references/reader-primer.md`
> 夹具：`fixtures/emir/fibonacci-two-books/`（两本虚构教材的 Fibonacci 小节 + 6 个 change set；Rust 与前端测试都以它为输入）；教程分镜：`fixtures/emir/fibonacci-two-books/tutorial-storyboard.md`
> 标签：**[锁定]** = 用户已拍板；**[已确认]** = 此前讨论中用户同意的第一条 PR 范围；**[实现]** = 本 PR 的具体实现选择，后续 PR 可在不违背锁定项的前提下调整；**[以后]** = 明确不在第一条 PR 内

---

## S0. 一句话 / 范围

**一句话**：`propose_changes` 在来源层之外，新开四种语义层对象：**共享对象**（SemanticEntity）、**对齐线**（Alignment）、**一笔账**（Debt）和**约定卡**（最小 DialectScope）。它们和来源对象一样只进暂存；用户显式接受后才写入地图。接受不提升任何可信状态；数学判断永不阻塞提交，只生成 Debt 或审核项。

**本延伸锁定 / 确认**：
1. [锁定·2026-11-13 用户定 a] 共享对象的写法跟 **mathlib** 保持一致，对象上注明所用约定（如 F₀ = 0），并引用 mathlib 中的对应声明作为参照基线。
2. [锁定·2026-11-13 用户定 b] **Debt 是地图对象**：有身份、有地图节点，挂在对齐线或共享对象上，有状态（open / claimed_paid / paid / waived / obsolete）。
3. [锁定·2026-11-13 用户定 c] 第一条 PR 提前做**最小 DialectScope**，只含第一期必需的字段，让“灯塔从第 1 项起编”这类约定能被机器读到。
4. [已确认] DomainPayload 包裹 SourcePayload（实现：untagged 包裹，存储形状不变，见 S2.1）；SemanticEntity 只开 concept / declaration / theory / problem_schema；Declaration 扩展；Alignment 是有修订的实体，不设 `same_as`；复用 TrustRecord；id 不透明，由 `(change_set_id, local_ref)` 派生；接受 ≠ 提升可信度或对齐状态；R/D/T/X 只作派生视图。
5. [已确认] **只有确定性检查阻塞**；数学判断只生成 Debt 或审核项。
6. [锁定·S13-1] **语义层对象允许 Agent 提议 retire**（选项 B），仍只是提案，须用户接受；来源层 retire 仍禁止 Agent 提议。Debt 的 `paid` / `waived` 只能由用户设置，Agent 最多写 `claimed_paid`。
7. [锁定·S13-3] `TrustRecord.alignment` 只由对齐状态按固定映射派生，任何写入口都不能直接写它。
8. [锁定·S13-8] mathlib 参照基线三态：`unchecked`（Agent 可写）/ `confirmed`（只由服务端在本机 Lean profile 的 mathlib 源码或 Loogle 中查到该名字后写入，并记录 mathlib 版本）/ `not_found`（服务端查过、没有）。没有 profile 时保持 `unchecked`。提供重新核对入口（后端函数 + Desktop 按钮 + CLI 命令）。
9. [锁定·S13-7] 学习视图标签「别的材料里见过」由对齐线自动计算；另有「新的」「新约定」和未还清账目汇总。另已接入用户本地研读进度，独立于地图与对齐推导标签（S9.1）。

**不做**：proof / method_pattern / mathematical_object 三种实体；FormalArtifact；TheoryAdapter；RepresentationBinding、ProofContext；R/D/T/X 对象；Expression AST；跨项目命名空间；自动对齐；自动还清 Debt。

---

## S1. 白话术语表（与教程分镜共用）[实现]

| 领域对象 / 字段 | 白话（地图、审核页、教程统一用） | 一句解释 |
|---|---|---|
| SourceSpan | 原文片段 | 书上这一段在哪儿 |
| SourceSemanticUnit | 来源单元 / 来源卡片 | 书里的一条定义、定理或习题 |
| SourceUnitRelation | 书内关系 | “这段证明那条”“这条提示那题” |
| SemanticEntity · concept | 概念 | 大家都在说的那个东西 |
| SemanticEntity · declaration | 精确说法（铭牌） | 带前提、带变量范围的那一句 |
| SemanticEntity · theory | 理论 | 共享对象的专题书架：“这几个概念和定理大概是哪一块儿的” |
| SemanticEntity · problem_schema | 题型 | 一族题，带参数 |
| `conventions` / `mathlib_baseline` | 约定注记 / 参照基线 | “本对象按 F₀ = 0 写，同 mathlib `Nat.fib`；不表示哪本书更正统” |
| Alignment | 对齐线 | 这段话说的就是那个共享对象，以及哪里不一样 |
| relation = expresses | 说的就是它 | |
| relation = specializes | 特例 | 例：“特例：灯塔的第 n 条 ＝ 共享的第 n+1 条” |
| relation = overlaps_with | 部分重合 | 例：“部分重合：灯塔没有第 0 项” |
| relation = generalizes / instance_of / representation_of / logically_equivalent / definitionally_equivalent / exact_realization / notation_for / incompatible_with | 推广 / 是…的一例 / 换了一种表示 / 互相推出 / 定义展开后一样 / 同一结构的实现 / 只是记号 / 互不相容 | |
| `parameter_map` | 第几条对第几条 | **命题参数**（如 Cassini 的 n）的对应 |
| `term_index_map` | 数列逐项对齐 | 数列**项下标**的对应；`index_shift = 0` 时说“f_k = F_k，数列没有错开” |
| `lost_information` | 对不上的部分 | “灯塔这里没有” |
| Debt（地图对象） | 一笔账 | 还欠一个证明或检查；状态：未还清 / 待确认已还 / 已还清 / 已豁免 / 已失效 |
| DialectScope | 约定卡 | 这本书 / 这一节的编号和叫法约定 |
| TrustRecord | 可信度徽章 | 分好几项，不是一个分数 |
| change set / 接受 | 提案批次 / 接受 | 接受 = 写进地图 ≠ 证明正确 |
| 审核项（proposal diagnostics `warn` / `info`） | 审核项 | 审核页上提醒用户留意的问题，不阻塞 |

**措辞约束** [实现]：凡涉及 `parameter_map` 的展示一律说“第 n 条 ↔ 第 n+1 条”，不得说“下标平移”“数列错开”；`term_index_map.index_shift = 0` 时必须同时展示“数列逐项对齐”。（来源：小黄 2026-11-13 复核意见）

---

## S2. 领域模型新增 [已确认范围 / 实现字段]

### S2.1 DomainPayload

```rust
// crates/mathscape-domain/src/semantic/mod.rs [实现]
#[serde(untagged)]   // 不加 layer 标签：两边都用 payload_kind 区分，SQLite 中已有的来源对象原样可读
pub enum DomainPayload {
    Source(SourcePayload),        // 现有，原样包裹
    Semantic(SemanticPayload),    // 新增
}
#[serde(tag = "payload_kind", content = "payload", rename_all = "snake_case")]
pub enum SemanticPayload {
    SemanticEntity(SemanticEntity), // payload_kind = "semantic_entity"，EntityKind::SemanticEntity
    Alignment(Alignment),         // EntityKind::Alignment（PayloadShape::Entity，见 S2.4）
    Debt(DebtRecord),             // payload_kind = "debt"，EntityKind::Debt
    DialectScope(DialectScope),   // EntityKind::DialectScope
}
pub type DomainEntity = EntityEnvelope<DomainPayload>;
```

- 四个 `EntityKind` 在 `entity.rs` 中**已存在**，不新增枚举值。
- `EntityEnvelope::new` 的 kind / shape 校验对 `DomainPayload` 同样适用；SQLite 行与 API DTO 不得绕过 checked constructor（domain README）。
- 现有 `SourceEntity = EntityEnvelope<SourcePayload>` 保留为别名，存量代码逐步迁移 [实现]。

### S2.2 SemanticEntity

```jsonc
{
  "semantic_kind": "concept | declaration | theory | problem_schema",
  "home_theory": "<semantic-entity id | $new:…> | null",   // 仅 theory 必须为 null；其余必填，且允许后续 edit 迁移
  "preferred_labels": [ { "lang": "zh", "label": "Cassini 恒等式" },     // 必须同时有 zh 和 en，与材料语言无关
                        { "lang": "en", "label": "Cassini's identity" } ],  // lang 只用 "zh" / "en"
  "aliases": ["Cassini's identity", "Simson identity"],      // 可选：中英文同义检索词（尤其推荐给 theory 填写）
  "plain_label": "精确说法：Cassini 恒等式（对一切 n ≥ 1）",  // 可选，Agent 补充；展示以前端模板为准（S9.1）
  "conventions": [                                           // 决定 a：约定注记
    { "key": "sequence_index_origin", "subject": "F", "value": 0,
      "text": "F₀ = 0，F₁ = 1，从第 0 项起编（同 mathlib Nat.fib）", "basis": "mathlib" } ],
  "mathlib_baseline": {                                      // 决定 a：参照基线；无对应时 decl = null
    "decl": "Int.fib_succ_mul_fib_pred_sub_fib_sq",
    "module": "Mathlib.Data.Int.Fib.Lemmas",
    "relation_hint": "specializes",          // 共享对象相对 mathlib 声明的关系，取 AlignmentRelation 子集
    "note": "mathlib 对一切整数 n 成立；共享对象取 n ≥ 1 的部分",
    "status": "unchecked | confirmed | not_found",
    "checked": null | { "mathlib_version": "…", "profile_id": "…", "method": "mathlib_source | loogle",
                        "module": "Mathlib.Data.Int.Fib.Lemmas", "location": "Mathlib/Data/Int/Fib/Lemmas.lean:28" } },
  "declaration": { /* S2.3，仅 declaration */ },
  "problem_schema": { /* S2.3，仅 problem_schema */ },
  "description": "string | null"            // concept 可用
}
```

- `conventions[].key` 受控词表：`sequence_index_origin`、`notation`、`terminology`、`baseline`、`other` [实现]。
- `mathlib_baseline.status` 三态 [锁定·S13-8]：
  - `unchecked`：Agent 只能写这个值，且不得带 `checked`；共享对象在 mathlib 里本来就没有对应时，`decl = null`、状态仍是 `unchecked`，在 `note` 里说明。
  - `confirmed`：只能由服务端写入（`record_mathlib_baseline` 操作，Agent 无法编译出它）。服务端在本机 Lean profile 的 mathlib 源码里按“命名空间 + 声明名”确定性查找（或 Loogle），查到后写 `confirmed`，并在 `checked` 里记录 mathlib 版本、profile、模块和文件行号。
  - `not_found`：服务端查过这个 profile，没有该名字。
  - 本机没有 Lean profile，或 profile 未就绪时：**不改状态**，保持 `unchecked`，核对入口返回“不可用”的原因。
  - **重新核对入口**：后端函数 `mathscape_mcp::mathlib_baseline::recheck_mathlib_baselines`（`crates/mathscape-mcp/src/mathlib_baseline.rs`；Desktop 命令 `recheck_mathlib_baselines` 调用同一入口）、审核页 / 学习视图上的「重新核对 mathlib 名称」按钮、CLI `mathscape mathlib-baseline recheck --workspace <path>`。核对结果以普通地图提交写入（新 revision、可回溯），作者是 `system:mathlib-baseline`。
  - **基线引用不是证明，也不提升 proof 或 alignment 可信度。**`confirmed` 只说明“这个名字在你这份 mathlib 里存在”。
- 共享对象不占有页码，也不带 `source_spans`（EMIR 不变量 9）。它和来源的连接**只走对齐线**（不变量 11）。

### S2.3 Declaration 扩展与 problem_schema 扩展

```jsonc
"declaration": {
  "declaration_kind": "definition | axiom | theorem | lemma | proposition | conjecture",
  "binders": [ { "name": "n", "type": "ℕ", "role": "universal | index | parameter" } ],
  "assumptions": [ { "text": "$n\\ge 1$", "format": "markdown+tex", "evidence_spans": ["<span id>"] } ],
  "statement":  { "text": "…", "format": "markdown+tex", "evidence_spans": ["…"], "note": "在 ℤ 中计算" } | null,
  "definiens":  { …同上… } | null,          // definition 必填 definiens，其余必填 statement
  "uses_entities": ["<semantic-entity id>"],
  "concept": "<concept id> | null"          // 可选：这条精确说法属于哪颗概念
}
"problem_schema": {
  "parameters":  [ { "name": "k", "type": "ℕ", "role": "parameter" } ],
  "assumptions": [ …结构化文本… ],
  "task":        { …结构化文本… },
  "known_instances": [ { "k": 6, "label": "8×8 → 5×13" } ],
  "uses_entities": ["…"]
}
```

- statement 是**结构化文本**，没有表达式 AST [已确认]；`evidence_spans` 是**措辞来源证据**，不表示归属（[锁定·S13-4]）。
- **理论（`theory`）的定位**：理论保持扁平（`theory` 本身 `home_theory = null`，不建父子树，也不绑定外部分类编码），定位为共享对象的**专题书架**（尺度类似课本一章/一个专题，如「线性递推数列」「数列极限」），只回答“这几个概念和定理大概是哪一块儿的”。每条精确说法的前提、变量范围和约定由对象自身记录，因此换书架不改变对象本身的数学含义。
- **身份字段**：`semantic_kind`、`declaration_kind`、`binders`、`assumptions`、`statement`、`definiens`、`parameters`、`task`、`conventions[].value`。改变它们就是另一条精确说法，必须**新建**，不能 edit（S4.3）。`home_theory` 与 `aliases` 可 edit，以便随着地图增长把对象平滑迁移到更合适的理论书架。

### S2.4 Alignment（有修订的实体）

```jsonc
{
  "source": "<source-unit id>",                 // 第一期只允许来源单元（EMIR 还允许 LocalSymbol / SemanticEntity，暂不开）
  "target": "<semantic-entity id>",
  "relation": "<AlignmentRelation>",            // EMIR §8.2 的 11 种；不含 same_as
  "source_context": { "dialect_scope": "<dialect-scope id>" } | { "binding": "uncontextualized", "reason": "…" },
  "target_context": "<theory id>",
  "parameter_map": [ { "source": "n", "target": "n", "role": "statement_parameter | problem_parameter",
                       "map": "n_共享 = n_灯塔 + 1", "source_range": "n_灯塔 ≥ 1", "target_range": "n_共享 ≥ 2", "note": "…" } ],
  "term_index_map": [ { "source": "f_k", "target": "F_k", "index_shift": 0, "valid_for": "k ≥ 1", "note": "…" } ],
  "required_assumptions": [ { "text": "f_k = F_k（k ≥ 1）", "debt": "<debt id> | null" } ],
  "lost_information": [ { "short": "缺共享的 n = 1", "claim": "…", "direction": "target_has_source_lacks | source_has_target_lacks", "see_also": "<id> | null" } ],
  "evidence_spans": ["<span id>"],              // 至少 1 条
  "status": "<AlignmentStatus>",
  "map_label": "string | null",                 // 可选；以前端模板为准（S9.1）
  "plain_note": "string | null"
}
```

- `parameter_map` 与 `term_index_map` **分开存**：前者是“第几条对第几条”，后者是“数列第几项对第几项”。这是 EMIR `binder_map` / `symbol_map` 在第一期的具体化 [实现]，用来从结构上避开“把移动命题参数说成数列错开”的误读。
- `lost_information` 先内嵌，不另建 Claim 对象 [锁定·S13-5]。
- `PayloadShape::Entity`：对齐线是**可修订、可单独审查和撤回**的实体，在地图上画成边（EMIR §17 “Alignment 是图边还是实体节点”：第一期定为实体，投影为边）[实现]。
- **status** 分三组 [实现]：
  - Agent 可写：`unmatched`、`retrieved_candidate`、`context_checked`、`partially_aligned`、`ambiguous`、`disputed`、`incompatible`；
  - 需证据、第一期不开放：`formally_related`（须附 Lean 检查产物；Lean 成功 ≠ 忠实）；
  - 只能由用户或系统写：`semantically_reviewed`、`accepted`（用户在审核页显式确认，**不是**接受提案的副作用），`rejected`、`superseded`（系统）。
- TrustRecord 的 `alignment` 轴由 status **固定映射**得出，不单独写 [锁定·S13-3]（实现：`AlignmentReviewStatus::trust_axis()`，每次写对齐线时由服务端重算）：unmatched → unmatched；retrieved_candidate / context_checked / partially_aligned / ambiguous → candidate；semantically_reviewed → reviewed；accepted → accepted；disputed / incompatible → disputed。

### S2.5 Debt（地图对象）[锁定·b / 字段实现]

```jsonc
{
  "debt_kind": "parse_debt | scope_debt | typing_debt | assumption_debt | alignment_debt | formalization_debt | proof_debt",
  "attached_to": "<alignment id | semantic-entity id>",
  "status": "open | claimed_paid | paid | waived | obsolete",
  "obligation": "证明 $f_k = F_k$ 对一切 $k\\ge 1$ 成立",
  "quantifier": "∀ k ≥ 1",                       // primer §4：写明量词
  "failure_mode": "theorem | statement | route",  // primer §4：不还清会坏在哪
  "failure_note": "若不成立，灯塔的三条对齐都不再适用",
  "evidence_spans": ["…"],
  "paid_by": [ { "ref": "<id>", "note": "…" } ],  // claimed_paid / paid 时必填
  "payoff_hint": "string | null"
}
```

- **消费者**（哪些对象依赖这笔账）不存在 Debt 里，而是由各对齐线的 `required_assumptions[].debt` **反查**。这样依赖方向始终是“后提交的引用先提交的”，不会让 Debt 反过来依赖后续 change set [实现]。
- **状态迁移**：

  | 从 → 到 | 谁 | 条件 |
  |---|---|---|
  | （新建）→ open | Agent / 用户 | — |
  | open → claimed_paid | Agent（edit） | 必须带 `paid_by`，引用活动对象 |
  | claimed_paid → paid | **只能用户**（审核页“确认已还清”） | — |
  | open / claimed_paid → waived | **只能用户** | 填理由 |
  | 任意 → obsolete | 系统 | `attached_to` 不再活动时自动标记并退役该 Debt，在预览中列出；普通 edit 产生新 revision 不触发此迁移 |
  | claimed_paid → open | Agent / 用户 | 撤回声明 |

- **与 P2 §3.3 提案 Debt 的关系**：P2 §3.3 的 `validation / unresolved / agent_declared` 改称**提案诊断**，仍挂在 change set version 上、不进地图。地图 Debt 是本节的新对象。审核页把两者分开显示：前者叫“审核项”，后者叫“一笔账” [实现]。

### S2.6 最小 DialectScope [锁定·c / 字段实现]

```jsonc
{
  "label": "《灯塔》§2.4 的约定",
  "anchor_unit": "<source-unit id>",           // 通常是 section 单元
  "applies_to": "anchor_subtree",              // 第一期唯一取值：锚点单元及其 parent 链下的全部后代
  "parent_scope": null,                        // 第一期固定 null（不做继承树）
  "introduced_by_unit": "<source-unit id>",    // 约定出现在哪条单元里
  "conventions": [
    { "key": "sequence_index_origin", "subject": "f", "value": 1,
      "text": "本书的下标一律从 1 开始，不使用第 0 项",
      "explicitness": "explicit | inherited | conventional | inferred",
      "evidence_spans": ["<span id>"] } ]
}
```

- 第一期**不**做：LocalSymbol / LocalAssumption 对象、notation_rules、继承树、`effective_context_hash` 的去重语义（服务端仍按 `(anchor_unit, conventions)` 计算 hash，作为驻留键）。
- **不改来源单元**：`SourceSemanticUnit.dialect_binding` 在第一期保持原值；“某单元生效的约定卡”由 `anchor_subtree` **派生查询**得出 [实现；何时回写见 §S13 仍待决的第 1 条]。
- `explicitness = inferred / conventional` 的约定，按 EMIR §6.3 不得在对齐时自动视为已确认。审核页标“推断”。
- 对齐线的 `source_context.dialect_scope` 必须指向一张覆盖 `source` 单元的约定卡；不覆盖时只给 `warn`（S5.2）。

---

## S3. id 规则 [已确认 / 前缀实现]

| 对象 | id |
|---|---|
| change set | `mathscape:change-set/sha256(investigation_id ∖0 request_id)`（现行 `resolve_version`） |
| 新建语义对象 | `mathscape:<kind>/sha256(change_set_id ∖0 local_ref)`（现行 `generated_id`），`<kind>` ∈ `semantic-entity`、`alignment`、`debt`、`dialect-scope` |
| source span | 不变，按内容确定（P2 §4.2.5） |

- id 不透明；可读名称只放在 `preferred_labels` / `plain_label` / `label` 中。
- 同一 change set 各 version 中同名 `local_ref` 得到同一 id，所以修订后依赖仍可解析（P2 §4.2.2）。
- 仓库测试里出现过 `mathscape:semantic/…`（`crates/mathscape-graph/src/projection.rs` 测试），实现时需要统一。

---

## S4. `propose_changes` 扩展

### S4.1 输入（在 P2 §4.2.1 的基础上增加 kind）

```jsonc
{ "request_id": "…", "supersedes": { … } | null, "rationale": "…",
  "target_object": "<local_ref | 已有 id>",                    // 新增，S8
  "existing_refs": [ … ],
  "proposed_objects": [
    { "local_ref": "e.cassini", "kind": "semantic_entity", "op": "create", "payload": { /* S2.2 */ } },
    { "local_ref": "a.yi.thm",  "kind": "alignment",       "op": "create", "payload": { /* S2.4 */ } },
    { "local_ref": "d.yi.thm.forward", "kind": "debt",      "op": "create", "payload": { /* S2.5 */ } },
    { "local_ref": "ds.yi",     "kind": "dialect_scope",   "op": "create", "payload": { /* S2.6 */ } },
    { "local_ref": "a.yi.thm",  "kind": "alignment",       "op": "edit",
      "target_id": "<id>", "expected_revision": "<rev>", "changes": { /* AlignmentChanges */ } }
  ],
  "proposed_relations": [ /* 仍只有 source_relation */ ],
  "unresolved": [] }
```

- 语义对象全部放在 `proposed_objects`（包括画成边的对齐线），`proposed_relations` 仍只有书内关系 [实现]。
- 来源 change set 可以附带 `dialect_scope`（夹具 `cs.src-yi` 就是这样）。

### S4.2 可编译范围（P2 §4.2.3 表的延伸）

| `kind` / `op` | 第一条 PR | 编译目标（新增 `UserChangeOperation`） |
| --- | --- | --- |
| `semantic_entity` / `create` | ✅ | `CreateSemanticEntity { object_id, draft }` |
| `semantic_entity` / `edit` | ✅（仅非身份字段） | `EditSemanticEntity`（`expected_revision` 必填） |
| `alignment` / `create` | ✅ | `CreateAlignment` |
| `alignment` / `edit` | ✅（不得改 `source` / `target`） | `EditAlignment` |
| `debt` / `create` | ✅ | `CreateDebt` |
| `debt` / `edit` | ✅（status 只能按 S2.5 的 Agent 路径） | `EditDebt` |
| `dialect_scope` / `create` | ✅ | `CreateDialectScope` |
| `dialect_scope` / `edit` | ✅（不得改 `anchor_unit`） | `EditDialectScope` |
| 语义对象 / `retire` | ✅（S4.4，选项 B） | `RetireObject`（现有，放宽到语义层） |
| 来源对象 / `retire` | ❌ | `operation_not_supported`（同 P2） |
| 用户专用：确认对齐、Debt 已还清 / 豁免 | Desktop 按钮 | `SetAlignmentReview`、`SetDebtSettlement`（MCP 无法编译） |
| 服务端专用：mathlib 基线核对结果 | 核对入口 | `RecordMathlibBaseline`（MCP 无法编译） |
| 其余 `semantic_kind`（proof、method_pattern、mathematical_object） | ❌ | `kind_not_supported` |

### S4.3 edit 的字段白名单 [实现]

| 对象 | 可 edit | 不可 edit（改了就要新建） |
|---|---|---|
| SemanticEntity | `preferred_labels`、`aliases`、`plain_label`、`home_theory`（仅非 theory 对象）、`conventions`（只能增补说明，不能改 `value`）、`mathlib_baseline`（`status` 只能写 `unchecked`，不得带 `checked`）、`description`、`uses_entities` | S2.3 列出的身份字段；`conventions[].value` |
| Alignment | `relation`、`parameter_map`、`term_index_map`、`required_assumptions`、`lost_information`、`evidence_spans`、`status`（仅 Agent 可写的值）、`map_label`、`plain_note`、`source_context` | `source`、`target` |
| Debt | `status`（open ↔ claimed_paid）、`paid_by`、`obligation` 的措辞、`failure_note`、`payoff_hint` | `attached_to`、`debt_kind` |
| DialectScope | `label`、`conventions` | `anchor_unit` |

edit 一个**已接受**对象会产生新 revision；旧 revision 仍可解析（EMIR §3.2）。

### S4.4 retire [锁定·S13-1：选项 B]

P2 §4.2.3 锁定“任意 `retire` ❌ Agent 不可提议退役”。用户 2026-11-13 改定：**语义层**对象（共享对象、对齐线、Debt、约定卡）允许 Agent 提议 `retire`；来源层仍然禁止。

- 写法：`{ "local_ref": "…", "kind": "<语义 kind>", "op": "retire", "target_id": "<已接受对象 id>", "expected_revision": "<rev>" }`。
- 它仍然只是提案，须用户接受；接受前地图不变。
- 受“禁止非法悬挂”约束（S5.1 第 12 条）：仍有活动对齐线指向的共享对象不能单独退役，要么一并退役那些对齐线，要么先改它们。
- 退役一条对齐线或共享对象时，挂在它上面的 Debt 由系统自动标 `obsolete` 并随之退役，预览的“受影响对象”里逐条列出。

### S4.5 引用规则与依赖闭包

- 引用写法沿用 P2 §4.2.6：`$new:<local_ref>`（本 version 新建项）、正式或暂存的稳定 id；兼容期的裸 `local_ref` **不适用于**语义对象（语义对象一开始就只接受 `$new:`）[实现]。
- 新增的引用位置：`home_theory`、`declaration.uses_entities`、`declaration.concept`、`problem_schema.uses_entities`、各结构化文本的 `evidence_spans`、`source`、`target`、`target_context`、`source_context.dialect_scope`、`required_assumptions[].debt`、`lost_information[].see_also`、`attached_to`、`paid_by[].ref`、`anchor_unit`、`introduced_by_unit`。
- **引用种类约束**（确定性，阻塞）：`source` → source-unit；`target` → semantic-entity；`home_theory` / `target_context` → `semantic_kind = theory`；`dialect_scope` → dialect-scope；`debt` → debt；`attached_to` → alignment 或 semantic-entity；`anchor_unit` → source-unit；`evidence_spans` → source-span。
- **跨 change set 依赖**：完全沿用 P2 §4.2.8。引用另一个 change set 的暂存对象即形成依赖边 `B → A`，接受 B 时 A 须已接受或同批接受；A 被 supersede 后 B 转 `blocked`（`dependency_superseded`），由 Agent 读回后重提；依赖图不得成环。夹具的依赖图：`cs.fib → {src-jia, src-yi}`；`cs.qpow / cs.cassini / cs.puzzle → {src-jia, src-yi, fib}`，无环。
- 语义 change set 引用来源 span 时，可以直接写 span id（形成依赖），也可以自带一条 `CreateSourceSpan`（id 由内容决定，自动去重，不形成依赖）。夹具用的是前者。

---

## S5. 校验：阻塞的只有确定性检查 [已确认原则 / 码实现]

### S5.1 阻塞（`validation` 诊断 → version `blocked`）

| # | 检查 | 失败码 |
|---|---|---|
| 1 | kind / op 在 S4.2 白名单；`deny_unknown_fields`；`semantic_kind` 在四种之内 | `kind_not_supported` / `operation_not_supported` / `invalid_arg` |
| 2 | 引用闭合（P2 §4.2.4 第 2 条） | `dangling_new_ref` / `dangling_staged_ref` |
| 3 | 引用种类约束（S4.5） | `invalid_reference_kind` |
| 4 | 必填字段：每条对齐线至少 1 条 `evidence_spans`；definition 有 `definiens`，其余 declaration 有 `statement`；非 theory 有 `home_theory`；Debt 有 `quantifier` 和 `failure_mode`；约定卡至少 1 条约定且每条有证据 | `missing_required_field` |
| 5 | `relation` ∈ EMIR 11 种；`same_as` 一律禁止 | `invalid_relation` / `same_as_forbidden` |
| 6 | Agent 自设终态：TrustRecord 任何轴；Alignment.status ∈ {semantically_reviewed, accepted, formally_related, rejected, superseded}；Debt.status ∈ {paid, waived, obsolete}；`mathlib_baseline.status` ≠ `unchecked` 或带 `checked` | `forbidden_trust_claim` / `forbidden_alignment_status` / `forbidden_debt_status` / `forbidden_baseline_status` |
| 7 | edit 改了身份字段或禁止字段（S4.3） | `identity_field_edit` |
| 8 | edit / retire 的 `expected_revision` ≠ 当前 revision | `revision_conflict`（沿用 P2 规划器的码） |
| 9 | `claimed_paid` 无 `paid_by`，或 `paid_by` 指向非活动对象 | `missing_required_field` / `dangling_staged_ref` |
| 10 | `term_index_map.index_shift`、`conventions[].value` 等结构化字段类型错误；`key` 不在受控词表 | `invalid_arg` |
| 11 | P2 原有的全部检查（span、`unused_span`、依赖无环、最终图校验、幂等） | 同 P2 |
| 12 | 非法悬挂：最终图里有活动对象指向已退役对象 | `inactive_or_invalid_reference` |

另有两条规划器级的作者检查（不是 propose 预检，而是任何写路径都必经的结构约束）：用户专用操作（`SetAlignmentReview`、`SetDebtSettlement`）的作者是 `agent:*` 时阻塞为 `user_only_operation`；服务端专用操作 `RecordMathlibBaseline` 的作者不是 `system:mathlib-baseline` 时阻塞为 `server_only_operation`，Desktop 的地图编辑命令也直接拒收它。MCP 从不编出这三种操作。

### S5.2 不阻塞：数学判断类 → 地图 Debt 或审核项

“审核项”就是 `warn` / `info` 诊断，在审核页显示，可以“反馈给 agent”（S9.2）。服务端**不判断数学对错**，只做形式上的提醒。

| 情形 | 处理 | 码 |
|---|---|---|
| `specializes` / `generalizes` 没有 `parameter_map` 或 `lost_information` | 审核项 | `warn: relation_detail_missing` |
| `overlaps_with` 没有 `lost_information` | 审核项 | `warn: overlap_scope_missing` |
| 关系需要证书（EMIR §8.2 表：specializes 需前向证书、logically_equivalent 需双向……），但对齐线上没有挂 alignment_debt，`required_assumptions` 也没指向任何 Debt | 审核项（建议 Agent 补一笔账） | `warn: certificate_debt_suggested` |
| `source_context.dialect_scope` 不覆盖 `source` 单元，或缺约定卡 | 审核项 | `warn: dialect_scope_mismatch` / `info: uncontextualized` |
| 约定卡的约定是 `inferred` / `conventional` | 审核项 | `info: inferred_convention` |
| 共享对象写了 `mathlib_baseline.decl`，状态是 `unchecked` | 审核项（提醒接受后可用核对入口确认） | `info: mathlib_baseline_unchecked` |
| 理论对象未写 `aliases` 同义检索词 | 审核项（提醒补几个中英文别称便于后续检索复用） | `info: theory_aliases_missing` |
| 理论名称带有 `=`、`$`、`\` 等具体公式符号 | 审核项（提醒理论应按一类专题命名，不要写具体式子） | `warn: theory_name_too_specific` |
| 同一来源单元对同一共享对象有两条活动对齐线 | 审核项 | `warn: duplicate_alignment` |
| 共享对象的 `preferred_labels` 缺少 `zh` 或 `en`（`lang` 只认这两个值，`zh-CN`、`en-US` 等不算） | 审核项（共享对象一律中英双名，不随材料语言） | `warn: label_lang_missing` |
| theory / concept 的某个名称或别名，折叠全角、大小写、空格和标点后与另一个活动的同类对象相同 | 审核项（提醒可能重复，列出已有对象 id；是否同一个由用户判断） | `warn: possible_duplicate_name` |
| concept / declaration 的 `mathlib_baseline.decl` 与另一个活动的同类对象相同 | 审核项（同上） | `warn: duplicate_mathlib_decl` |
| 共享 declaration 未写 `sequence_index_origin` 之类约定，而文本涉及下标 | 不检测（服务端不解析数学） | — |
| 关系选得对不对、映射对不对、对不上的部分有没有写全 | **用户审核 + Debt**，服务端不检查 | — |

---

## S6. supersedes 修订规则 [沿用 P2 / 本文补充]

1. change set 层：完全沿用 P2 §4.2.4。Agent 以 `supersedes{change_set_id, version, proposal_fingerprint}` 提交新 version；指纹不匹配则 `supersede_conflict`；旧 version → `superseded`；同 `local_ref` 得到同一 id。
2. 对象层：**暂存**对象的修订 = change set 的新 version（对象 id 不变，内容随 version 变）；**已接受**对象的修订 = `edit` 产生新 revision（S4.3），旧 revision 可解析。
3. 对齐线的典型路径（夹具 `revision_history`）：用户在对话里反馈“灯塔没有第 0 项”→ Agent `get_change_set` 读回 → 双方同意 → Agent 以 supersedes 提交 v2，把 `definitionally_equivalent` 改成 `overlaps_with`，补上 `lost_information`。审核页显示 v1 / v2 的差异。
4. 依赖者：A 被 supersede 后，依赖 A 的 B 转 `blocked`（`dependency_superseded`），**不自动改指**（P2 §4.2.8 R5）。夹具里 `cs.fib` v2 先于 `cs.qpow` 等提交，所以没有触发。
5. Debt 跟随：对齐线新 revision 若删除了某个 `required_assumptions[].debt` 引用，被删的 Debt 不自动关闭；若对齐线被退役，挂在它上面的 Debt → `obsolete`（系统，预览中列出）。

---

## S7. 接受语义 ≠ 可信度 [锁定，沿用 P2 §5]

- 接受 = 把 change set 编译成 `UserChangeRequest`，经现有 plan/commit 管线写入；对象 `lifecycle = active`。
- 接受**不**改变：TrustRecord 各轴；`Alignment.status`（仍是 `retrieved_candidate` / `partially_aligned` 等）；`Debt.status`（仍是 `open`）；`mathlib_baseline.status`。
- 提升对齐状态（→ `semantically_reviewed` / `accepted`）、确认还清（→ `paid`）、豁免（→ `waived`）都是**另外的、显式的用户动作**，各自留痕（谁、何时、依据）。不是“接受提案”的副作用。
- 审核面板固定提示：「接受只表示写入地图，不代表数学上已验证」（P2 §5.5）。
- 写入顺序（P2 §4.2.7 的延伸）[实现]：`CreateSourceSpan` → 来源单元（父在前）→ 书内关系 → `CreateDialectScope` → `CreateSemanticEntity`（theory 在前，再按 `home_theory` / `uses_entities` 拓扑序）→ `CreateAlignment` → `CreateDebt`；edit 跟在对应 create 层之后；同层按 id 排序；整批一个事务。

---

## S8. 约定：每个 change set 围绕一个目标对象 [实现]

- 输入新增 `target_object`（本 version 的 `local_ref` 或已有 id），审核页标题以它为准，例如“提案：Cassini 恒等式”。
- 一个语义 change set = **一个目标共享对象** + 它的**必要上下文**（它的理论 theory、它的 concept，第一次出现时）+ **连到它的对齐线** + **挂在这些对齐线或该对象上的 Debt**。
- 来源 change set 以 section 单元为目标，可以附带该节的约定卡。
- 不满足时只给审核项（`info: multi_target_change_set`），不阻塞（确定性可判，但这是组织建议而不是正确性要求）。
- 夹具的 6 个 change set：`jia.u.sec`、`yi.u.sec`、`e.fib`（附 `e.theory`、`e.fib_def`）、`e.qpow`、`e.cassini`、`e.puzzle`。
- 好处：用户可以先接受“Fibonacci 数列”，对 Cassini 那批单独讨论、单独修订，修订不会牵连无关对象；代价是依赖边变多（§S13 第 9 条）。

---

## S9. 展示要求 [实现]

### S9.1 地图

- **节点**：概念（星光 `#e8f1ff`）、精确说法铭牌、题型卡、理论背景；约定卡挂在 section 来源卡片上（书封展开）；Debt 为暗金虚线小标签（`#c9a86a`），挂在对齐线中点或对象角上，状态文字常显（“未还清 / 待确认已还 / 已还清”），已还清改为金色实线框（`#ffd89a`），与动画一致。
- **对齐线**：连线色 `#9eb6ff`。暂存为虚线，已接受为实线；**线型只表示是否已接受，不表示可信度**。可信度看徽章。
- **白话标签**由前端模板生成，Agent 给的 `map_label` 只作备选：
  - expresses →「说的就是它」；有非恒等 `parameter_map` 时加「：灯塔的 n 对应题型的 k = n+1」这类说明
  - specializes / generalizes →「特例 / 推广」；有 `statement_parameter` 映射时加「：〈书〉的第 n 条 ＝ 共享的第 n+1 条」
  - overlaps_with →「部分重合：」+ `lost_information[0].short`
  - 有 `term_index_map.index_shift = 0` 时，悬浮卡加一行「数列逐项对齐：f_k = F_k」
  - 禁用词：平移、错开、滑动（S1 措辞约束）
- **学习视图**（派生，教程镜 11）[锁定·S13-7]：选定一份材料（来源）后，对其中每个来源单元：
  - 它的对齐目标若还被**其他材料**的来源单元对齐（对齐线均为活动、未退役），标「别的材料里见过」，并列出是哪份材料；`lost_information` 非空时附“只差：…”；
  - 没有其他材料对齐到同一共享对象的，标「新的」；
  - 本材料的约定卡标「新约定」（例如“下标从 1 开始”）；
  - 底部汇总挂在本材料相关对齐线 / 共享对象上的未还清账目数（open + claimed_paid）。
  - 上述标签只计算、不写入地图对象。用户另可设置 `unread` / `studying` / `mastered`（未读 / 在读 / 已掌握），经 Desktop `set_study_marks` 存入 `study_marks`（migration 25）。这是用户本地进度，不产生地图 revision，不替代「别的材料里见过」。

### S9.2 审核页

- 总览按目标对象分组，列出：共享对象（白话名 + 约定注记 + 参照基线状态）、对齐线（白话标签 + 两端标题）、地图 Debt（量词、失败方式、状态）、约定卡，以及**审核项**（S5.2）。不显示 JSON。
- 对齐线详情并排显示：来源原文（按 span 高亮 + KaTeX）| 共享对象的精确说法 | 参数对照表（第几条 ↔ 第几条）| 数列项对照（index_shift）| 对不上的部分。
- **「反馈给 agent」按钮**：每个审核项、每条对齐线、每笔账、每张约定卡都有。点击后向对话输入框插入一段引用（不自动发送），格式：
  ```text
  > [审核项] 提案「Cassini 恒等式」v2 · 对齐线 a.yi.thm（特例：灯塔的第 n 条 ＝ 共享的第 n+1 条）
  > id: mathscape:alignment/d1d5…  · 码: warn:relation_detail_missing
  ```
  用户补一句意见后发送。Agent 据此 `get_change_set` 读回并用 supersedes 修订（S6.3）。按钮只拼引用文本，不改提案状态。
- 底栏保留「接受本提案」「丢弃本提案」，以及固定提示条（S7）。对齐状态提升和 Debt 确认还清是**对象级**按钮，与“接受本提案”分开。

---

## S10. 动画元素 ↔ 领域对象

| 小紫动画中的元素 | 领域对象 | 第一条 PR 后 |
|---|---|---|
| 续集预告的两本书 | SourceArtifact + SourceSemanticUnit | ✅ |
| 预告里的候选虚线 | Alignment（暂存或 `retrieved_candidate`） | ✅ |
| “这道题自己的小网”中的来源锚点 | SourceSemanticUnit | ✅ |
| 一笔 Debt：“对一切 n，差恰为 ±1” | Debt（`d.puzzle.all_k`，挂在题型上，open） | ✅（决定 b） |
| Debt 已偿还（虚线变金色实线） | Debt `paid`（用户确认；Agent 先提 `claimed_paid`，`paid_by = e.cassini`） | ✅ 状态；“由 D 还清”的 D 对象要等 R/D/T/X |
| Cassini 青色六边形徽章 | declaration `e.cassini` | ✅ 铭牌；挂在矩阵 R 星上要等 R/D/T/X |
| 四颗 R 星（拼图、递推、矩阵、特征基） | RepresentationCore（派生视图） | ⏳ |
| T 线和证书、D 箭头、守卫圈、X 招式卡、失败见证 | Transport / Inference / guard / Macro | ⏳ |
| 拼图 8×8 → 5×13 | problem_schema `e.puzzle` 的 `known_instances[k=6]` | ✅ |
| 预告旁白“一本从 F₀ = 0 开始，一本从 F₁ = 1 开始” | DialectScope `ds.jia` / `ds.yi`（`sequence_index_origin` = 0 / 1） | ✅（决定 c） |
| 预告旁白“矩阵的下标也不一样” | `a.yi.lem` 的 `parameter_map`（第 n 条 ↔ 第 n+1 条） | ✅ |
| 第 8 幕交互（2×n 铺砖三选一） | X 应用记录 | ⏳ 另立教程 PR |

---

## S11. 夹具与测试 [实现]

`fixtures/emir/fibonacci-two-books/` 是本契约的**示例与测试夹具**：`materials/` 两份虚构教材原文、`objects.json` 6 个 change set、`tutorial-projection.json`（只读教程投影：Desktop 测试 `emir_fixture_accepts_and_learning_view_marks_seen_material` 把 `objects.json` 走一遍真实的暂存 → 接受 → 地图投影后写出；平时逐字比对，过期时用 `MATHSCAPE_UPDATE_TUTORIAL_PROJECTION=1` 重跑该测试）。`build_objects.py` 生成 `objects.json`，`verify_fixtures.py` 做离线核对。

接入的测试：MCP `emir_fixture_change_sets_all_stage_in_order`（6 个 change set 依次全部 staged，地图不变）；Desktop `emir_fixture_accepts_and_learning_view_marks_seen_material`（一次接受、状态与可信度不变、学习视图标签）；vitest `tutorial-projection.spec.ts`（投影里每条对齐线的白话标签与前端模板一致、不出现禁用词）。

| 夹具内容 | 覆盖的契约条款 |
|---|---|
| 6 个 change set、53 项（13 span、13 来源单元、6 书内关系、2 约定卡、6 共享对象、9 对齐线、4 地图 Debt） | S2 全部 payload；S8 一个目标 |
| `a.yi.def`（overlaps_with + lost_information + term_index_map） | S2.4、S9.1 部分重合标签 |
| `a.yi.thm` / `a.yi.lem`（specializes + statement_parameter 映射 + see_also） | S2.4、S1 措辞约束 |
| `a.yi.ex`（problem_parameter 映射 k = n+1） | S2.3、S2.4 |
| `d.yi.f_eq_F` 被三个后续 change set 的 `required_assumptions` 引用 | S2.5 消费者反查；S4.5 跨 change set 依赖 |
| `d.puzzle.all_k` 挂在共享对象上 | S2.5、S10 |
| `ds.yi`（`sequence_index_origin = 1`、`terminology` 为 inferred） | S2.6、S5.2 `inferred_convention` |
| `e.fib_def` / `e.cassini` / `e.qpow` 的 conventions 与 mathlib_baseline（全部 `unchecked`；`confirmed` / `not_found` 只由核对入口写） | S2.2、决定 a |
| `revision_history`：两次 v1 → v2 | S6 |

`verify_fixtures.py`（夹具目录内），覆盖数学核对和 S5.1 中能离线判断的项：引用闭合与种类、id 推算、unused_span、依赖无环、禁止状态、`same_as`、一个目标、白话标签禁用词、index_shift。实现 PR 的单元测试建议直接以 `objects.json` 为输入：编译 → 校验全部 `staged`；再逐项构造反例（改一个字段触发一个失败码），每个 S5.1 码至少一例。

---

## S12. 勘误（本 PR 已在 P2 中一并修正）

| 位置 | 原文 | 应为 | 依据 |
|---|---|---|---|
| P2 §4.2.1 示例，`source_unit` 的 payload | `"unit_kind": "theorem"` | `"kind": "statement"` | `crates/mathscape-domain/src/map_edit.rs` `SourceUnitDraft.kind: DiscoveryUnitKind`；`discovery_proposal.rs` 的 `DiscoveryUnitKind` 没有 `theorem`，定理和命题用 `statement`；P2 §4.2.3 正文已说明字段名是 `kind`（2026-11-10），但示例未同步 |
| P2 §4.2.3 表 | “任意 `retire` ❌” | “来源层 `retire` ❌”，语义层见 S4.4 | 用户 2026-11-13 决定（S13.1 第 1 条） |

---

## S13. 决定记录与仍待决事项

### S13.1 用户 2026-11-13 的决定（已在本 PR 实现）

| # | 事项 | 决定 |
|---|---|---|
| 1 | retire | 选项 B：语义层允许 Agent 提议 retire（S4.4）；Debt `paid` / `waived` 只能用户设，Agent 最多 `claimed_paid` |
| 2 | Debt 还清的确认权 | `claimed_paid → paid` 只能用户确认；还清证据第一期只能引用对象 |
| 3 | 两套对齐状态 | 采纳固定映射；`TrustRecord.alignment` 不可直接写 |
| 4 | statement 回链 span | 是措辞来源证据，不是归属 |
| 5 | lost_information | 内嵌在对齐线里 |
| 7 | 学习视图 | 「别的材料里见过」自动计算；「新的」「新约定」+ 未还清账目汇总；用户研读进度另存本地 sidecar，不替代对齐推导标签 |
| 8 | mathlib 基线 | unchecked / confirmed / not_found 三态，confirmed 只由服务端查到后写入并记 mathlib 版本；无 profile 保持 unchecked；提供重新核对入口 |
| 9 | 一个目标的粒度 | 每个 change set 一个 `target_object`；`cs.fib` 带 theory + concept + definition 可以接受 |
| 10 | “限制”关系 | 不新增，继续用 `overlaps_with` |
| 11 | `formally_related` | 本期不开放 |
| 12 | id 前缀 | 采纳 `semantic-entity` / `alignment` / `debt` / `dialect-scope`；测试里的 `mathscape:semantic/` 改为 `mathscape:semantic-entity/` |
| 13 | 教程投影 | 只读、内存中的教程投影（由夹具生成的 JSON），不写用户工作区 |

### S13.2 本机 mathlib 名称核对（2026-11-13，用户 Mac，profile `lean4-4.33.1-mathlib-4.33.1`，只读 `rg`）

| 夹具对象 | 名称 | 结果 |
|---|---|---|
| `e.fib_def` | `Nat.fib` | 存在：`Mathlib/Data/Nat/Fib/Basic.lean:59`（`namespace Nat`，`fib_zero : fib 0 = 0`） |
| `e.cassini` | `Int.fib_succ_mul_fib_pred_sub_fib_sq` | 存在：`Mathlib/Data/Int/Fib/Lemmas.lean:28`，`(n : ℤ)`，右边是 `(-1) ^ n.natAbs` |
| `e.qpow` | Fibonacci 矩阵幂引理 | 这份 mathlib 中没有找到（搜索 `!![1, 1; 1, 0]` 与含 `fib` 的矩阵命题均无结果），夹具中 `decl = null` |

这只是开发时的人工核对；夹具里仍写 `unchecked`，由核对入口在用户机器上写 `confirmed` / `not_found`。

### S13.3 仍待决

用户研读进度的归属已落实为独立 `study_marks` sidecar，不属于地图对象（S9.1）。

1. **约定卡与来源单元的绑定**：第一期用 `anchor_subtree` 派生，不写 `SourceSemanticUnit.dialect_binding`；什么时候回写？
2. **`formally_related` 开放的时间**（与 Lean 工具契约联动）。
