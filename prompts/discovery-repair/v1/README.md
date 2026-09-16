# Discovery Repair v1

一次有界修订的系统 Prompt、manifest 与 Rust DTO 生成的 Schema。工作流输入包含 repair
作用域、原始提案和重新构造的原文上下文；不继承上一次聊天历史。

A 允许替换单元提案，B 允许选择当前候选或替换失败任务。所有结果重新验证；工作流不允许
跨阶段替换、跨任务扩大修改范围或无限循环。实现见
[当前架构](../../../docs/architecture.md)。

Prompt revision 4 增加按 task 聚合的事实修订与 replace_task 完整示例，并保留相同范围单元、
quote_range 重切和跨单元 issue 所有权的有界修订方法。
