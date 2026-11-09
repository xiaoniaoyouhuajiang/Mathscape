# worknet_run：可恢复的工作状态模板

每次构图都应有一个可恢复的 run。它可以是 Markdown，也可以是 JSON；关键是字段含义稳定。

```yaml
schema_version: worknet_run v0
run_id: "…"
object_ref: "…"
user_goal: "用一句话写本轮想理解或验证什么"
current_stage: S1
source_scope: []
branch_ids: []
route_ids: []
capabilities: {}
reference_reads: []
user_questions: []
staged_artifacts: []
change_set_id: null
checks:
  I1: pending
  I2: pending
  I3: not_applicable
  I4: pending
  I5: pending
  I6: pending
  I7: pending
blocked_reasons: []
open_debts: []
next_action: "…"
```

状态含义：

- `pending`：还没有做检查；
- `pass` / `fail`：已经做了检查并保留理由；
- `not_applicable`：条款确实不适用，必须写理由；
- `undecidable_under_current_criteria`：当前准则无法判断，不能当作 pass；
- `blocked`：缺少能力、用户回答或必要证据，暂时不能推进。

`current_stage` 只表示下一步工作位置，不表示数学正确。重启或新 round 时先读这个文件，再查工作区现状和 staged 记录。
