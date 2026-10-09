# Ink P0 契约样本

这里的 `trace.json` 是人工构造的合成样本，不来自物理设备。它覆盖两个固定页面、两个采集时钟片段、普通/中断/取消笔迹、擦除及撤销重做。`projection.json` 是预期可见笔迹与历史栈，`invalid-cases.json` 的变更与删除由 Rust、TS 测试共同应用。

`schema.json` 从 `InkLabDocument` 生成；Schema 检查形状，引用、数值边界和历史状态仍由确定性验证检查。nullable 字段必须存在，null 表示没有记录对应值。

不把真实用户手稿、运行日志或设备测量放进默认 fixtures。真实实验导出保留在用户的实验目录或外部材料中，按 `device-report.md` 记录；确认可分发并去除私人内容后，才能增加真实样本。
