# Markdown Patch v1

这是独立的 P 阶段协议：它只消费已验证的 Stage A 转换异常，并提出同一物理 block 内的逐字替换。系统会重新解析 `text_occurrence`、检查 issue evidence、冲突和 Markdown 分块结构；模型输出不具备 artifact、字节范围或来源图写入权限。

运行时 Schema 由 `MarkdownPatchResponse` 生成；提交的 `output.schema.json` 可用下面的命令复核：

```sh
cargo run -p mathscape-orchestrator --example staged_discovery_schema --quiet -- markdown-patch
```

P 的审核和应用不修改原 Markdown，而是通过 Artifact Pool 创建内容寻址的派生 Markdown artifact，并记录 `mathscape-markdown-patch` ConversionRun。全部拒绝时继续使用原快照；需要跨 block 或改变结构的修订必须回到 Stage A。
