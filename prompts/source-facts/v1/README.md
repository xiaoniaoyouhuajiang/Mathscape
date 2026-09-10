# Source Facts v1

阶段 B 的版本化 Prompt 包。输入以已冻结 UnitSet 生成任务，owner 由系统指定；模型输出标签、
标题、引用和原文明示关系，不重复填写 source，也不能修改单元结构。

M2 已完成任务规划、位置验证、本地目标解析、批次组装和 Producer。输入版本为
mathscape-source-facts-input-2，Prompt revision 为 2；每个批次提供完整授权原文，任务只限定
提取范围。输出 Schema 保持 mathscape-source-facts-1。详见
[M2 说明](../../../docs/discovery-staged-m2.md)。

重新生成 Schema：

```sh
cargo run -p mathscape-orchestrator --example staged_discovery_schema --quiet -- facts
```
