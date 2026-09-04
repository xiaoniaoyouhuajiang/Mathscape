# Testdata policy

`testdata/` 保存进入 Mathscape 版本管理的轻量、确定性测试资产。普通协作者只需克隆本仓库即可运行默认的 `cargo test --workspace`。

## 边界

- 可以提交：小型 Markdown、JSON metadata、必要的缩略图片、fixture manifest。
- 不应提交：教材或论文原始 PDF、模型权重、Marker 缓存、运行日志、临时目录。
- 每个真实来源 fixture 必须记录原始材料的相对语料路径、SHA-256、选择页码、生成工具版本与产物摘要。
- 默认测试只能依赖本目录；不得要求 `MATHSCAPE_CORPUS_ROOT`、Marker、GPU 或网络。

大型材料由仓库外的 Corpus/Artifact Pool 管理，并通过 `MATHSCAPE_CORPUS_ROOT` 定位。需要重生成或端到端验证时，测试应显式启用，并在找不到外部语料或 Marker 配置时给出明确错误；这类测试不属于默认测试集合。

Marker 等模型工具升级后，先在外部语料上重生成候选结果并进行语义检查，再有意更新这里的 golden fixture。不要把不同模型版本造成的字节变化自动视为回归。
