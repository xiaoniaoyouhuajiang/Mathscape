# Mathscape 数学地图 skill v0.6 修订说明

## 本次目标

把原来的研究员操作手册改成 Agent 第一次收到也能读懂的工作协议：先解释 EMIR/RDTX 在 Mathscape 中解决什么问题，再给 S1–S5 的动作、工具边界和停手条件。

## 本次改动

- `SKILL.md` 增加 EMIR、来源层、语义层、R/D/T/X、Debt、guard、lemma 和 evidence 的白话解释；
- 修正“先写完整对象卡再读来源”的顺序冲突：先写临时目标，S1 读现状，随后形成 S2 对象卡；
- 明确 S1/S2 可合并只表示合并提案边界，不表示跳过任一职责；
- 加入 `reference-index.md`，为 EMIR、RDTX、Phase2、T04/T08/T12/T18 和小黄完整报告提供逻辑 id、路径和版本说明；
- 将 `rdtx_try.md` 及 T04/T08/T12/T18 的对象卡、工作网副本纳入 `references/`，skill 单独分发时仍能解析这些案例；
- 加入 `runtime-capabilities.md`，明确已实现、只有契约和假定工具，不允许调用不存在的语义 API；
- 收紧 workspace 引用：必须由宿主提供 `workspace_root`，`docs/` 与其中的 `docs/proposals/` 按索引和层级解释，缺失或版本不明时显式标记 unavailable；
- 加入 `run-state-template.md`，让 Agent 可以在中断后恢复工作，并区分 pending、blocked、partial、待核和不可判定；
- `reader-primer.md` 改为第一次接触也能读的白话入门，并用 T04/T08/T12/T18 解释 R/D/T/X；
- 保留 `creation-rules.md` 和 `checks-i1-i7.md` 作为详细规则，不把它们写成已经实现的接口。

## 这份 skill 的定位

它是数学地图构建的流程 skill，不是 EMIR 数据库 Schema，不是数学认证器，也不是语义层 MCP 工具的实现。它可以指导 Agent 产生来源提案、对象卡、工作网草案和检查报告；正式图写入仍受 Phase2 用户接受管线约束。

## 当前已知边界

- `propose_changes`、`get_change_set`、`user.ask` 是否可用必须以本轮实际挂载的工具列表为准；
- `artifact.stage` 只能保存受控文本快照，不会自动登记地图；
- `emir.upsert_*`、`rdtx.upsert_*` 等仍是契约草案；
- RDTX 规范仍是 `0.1.0-draft`，EMIR 是 `0.2-experimental`；
- criteria v0.5.1 的相对稳定声明不等于所有对象完成验收；T04/T08/T12/T18 只是案例，不是自动证书；
- T12 α/β 正式等价尚未登记，T1 外部端点仍 partial·待核，阅读包中的链接缺口见 `LINK-AUDIT.md`。

## 使用顺序

1. 先读 `SKILL.md`；
2. 再读 `references/reader-primer.md` 和 `references/reference-index.md`；
3. 按本轮目标读取正式规范或案例；
4. 读取 `references/runtime-capabilities.md` 并核对本轮实际挂载的工具列表；
5. 创建 `worknet_run` 后再开始 S1。

## 仓库版变更（v0.6.1）

- 纳入 Mathscape 仓库 `skills/mathscape-worknet/` 做版本管理；frontmatter 增加 `name`、`description`、`metadata.version`、`metadata.tool_schema_version`，供 Codex skill 加载器识别。
- 不再要求“调用 `tools/list`”：那是 MCP 协议方法，Agent 只需查看本轮挂载的工具。
- `references/runtime-capabilities.md` 增加「Mathscape 沙箱运行时」一节，写明沙箱当前挂载的工具、scratch 写入边界和 Lean 草稿检查方式。
- 工具名与仓库中的 MCP 工具表由测试 `skill_pack` 校验，改工具或改 skill 时两边需要同步。

