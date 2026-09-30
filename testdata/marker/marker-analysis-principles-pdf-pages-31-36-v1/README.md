# Marker 真实产物 fixture：PDF 第 31–36 页

这个 fixture 用于验证 `mathscape-tool-marker` 的产物扫描与清单生成。它不是人工拼装的样本，而是由封装后的 Marker 子进程对下列真实教材片段生成，再由 `MarkerProductScanner` 扫描确认：

- 来源：`$MATHSCAPE_CORPUS_ROOT/textbooks/大学数学-数学分析原理与方法_含目录.pdf`
- 用户可见的 PDF 物理页：31–36（从 1 开始计数）
- Mathscape 与 Marker 页索引：30–35（从 0 开始计数）
- Marker profile：`mathscape.marker.math-ocr.v1`（balanced）
- Marker 版本：2.0.0

`run/` 保留 Markdown 与 Marker metadata 两项可移植派生产物。标准输出和错误输出日志没有纳入 fixture，因为它们包含本机暂存路径及非确定性的耗时信息；子进程诊断日志的分类由独立的合成测试覆盖。

`fixture.yaml` 固化来源文件摘要、页码映射、生成参数和产物摘要。回归测试还会读取 metadata 的 `page_stats`，确认实际处理的页索引严格为 `30, 31, 32, 33, 34, 35`。

来源 PDF 不进入 Mathscape 仓库。fixture 通过相对语料路径和 SHA-256 与外部 Corpus 关联；扫描器的默认测试只读取本目录中的轻量产物，不依赖环境变量或原始 PDF。

该 fixture 保留的是历史未分页 Marker 产物，用于验证旧转换兼容性；新的
Mathscape Marker 请求会加入 `--paginate_output`，并在 promotion 阶段额外生成
Markdown-PDF 页映射 sidecar。
