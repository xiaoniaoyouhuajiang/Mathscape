# Source Discovery v1（历史契约）

历史参考，不作为当前规范。此包保留原有 system.md、manifest.json 和 output.schema.json，用于识别历史协议、比较及回放材料；保留资源不保证任意旧响应可被当前验证器直接接受。

当前 Desktop 使用 A/B 分阶段流程，当前单阶段 build_discovery_request 使用 v3。不要用最新 DTO 的 Schema 生成结果覆盖本包；具体历史字段以本包 Schema 与 Git 历史为准。旧响应必须先按对应协议解释，再经当前授权与确定性检查，不能直接恢复可信状态。

当前边界见 [架构](../../../docs/architecture.md)，Schema 维护见 [开发指南](../../../docs/development.md)。
