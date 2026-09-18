# Unit Discovery v1

阶段 A 的版本化 Prompt 包。输入是程序从可信 DiscoveryInput 投影出的块片段和请求短句柄，
输出只包含数学言语单元、范围、parent、完整性、issues 与 coverage。

运行时 Schema 由 `UnitDiscoveryProposal` 生成，并与 `output.schema.json` 快照比较。当前入口为
`build_unit_discovery_request` 和 `UnitDiscoveryProducer`；有效结果是只读的
`ValidatedUnitSet`，不会生成 v3 中的标签、引用或关系，也不会写入地图。

Prompt revision 5 根据回归结果补充了默认单元尺度、短引言与混合段落处理、完整性与数学正确性区分、
严格包含、issue 证据所有权和跨单元来源问题的正反例；输出 DTO 与 Schema 版本保持不变。

重新生成 Schema：

```sh
cargo run -p mathscape-orchestrator --example staged_discovery_schema --quiet -- units
```
