# Source Facts v1

阶段 B 的版本化 Prompt 包。输入以已冻结 UnitSet 生成任务，owner 由系统指定；模型输出标签、
标题、引用和原文明示关系，不重复填写 source，也不能修改单元结构。

本包的 DTO、Prompt、manifest、Schema 快照和位置无关 preflight 已纳入 M1。任务规划、位置验证、
本地目标解析、批次组装和 Producer 在 M2 实现。

重新生成 Schema：

```sh
cargo run -p mathscape-orchestrator --example staged_discovery_schema --quiet -- facts
```
