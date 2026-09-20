# Discovery Repair v1

一次有界修订的系统 Prompt、manifest 与 Rust DTO 生成的 Schema。工作流输入包含 repair
作用域、原始提案和重新构造的原文上下文；不继承上一次聊天历史。

A 允许替换单元提案，B 允许选择当前候选或替换失败任务。所有结果重新验证；工作流不允许
跨阶段替换、跨任务扩大修改范围或无限循环。实现见
[当前架构](../../../docs/architecture.md)。

Prompt revision 7 增加了带基础提案指纹的局部 patch：单元 issue evidence、事实锚点和事实 issue evidence 可以独立修复；完整
replace_units/replace_task 仅用于结构性变化。重复引用的歧义可以作为非阻塞警告保留，仍由确定性验证器决定最终是否接受。
