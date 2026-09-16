# 开发与维护

## 环境与启动

命令均在 `mathscape/` 根执行。Rust 最低版本由 [Cargo.toml](../Cargo.toml) 指定（当前 1.88，edition 2024）；使用 Node/npm 和 [Tauri](../apps/mathscape-desktop/tauri.conf.json) 所需的平台构建环境。依赖版本以 Cargo.lock、前端 package-lock.json 为准。

```sh
npm --prefix frontend install
npm --prefix frontend run tauri:dev:local
```

该命令使用被忽略的 `.mathscape-dev/workspace/`。普通 `tauri:dev` 使用应用数据目录；可用 `MATHSCAPE_WORKSPACE_ROOT` 显式指定。Marker 为可选外部安装，通过 `MATHSCAPE_MARKER_EXECUTABLE` 指向绝对 `marker_single` 路径；模型连接在桌面设置页配置。

## 验证与测试入口

按改动选择检查，不为纯文档调整运行整套应用：

```sh
cargo check --workspace
cargo test -p mathscape-orchestrator
cargo test -p mathscape-store-sqlite --test schema
cargo test -p mathscape-desktop --test service
npm --prefix frontend run build
npm --prefix frontend run test:reader
```

需要全量 Rust 回归时运行 `cargo test --workspace`。浏览器测试初次需在 `frontend/` 执行 `npx playwright install chromium`，也可指定 `PLAYWRIGHT_CHROMIUM_EXECUTABLE`。脚本与测试配置以 [package.json](../frontend/package.json) 和 [Playwright 配置](../frontend/playwright.config.ts) 为准。

- [Domain 测试](../crates/mathscape-domain/tests)：构造器、不变量和 DTO。
- [Orchestrator 测试](../crates/mathscape-orchestrator/tests)：输入定位、A/B、repair、预算、登记计划和重验证。
- [存储测试](../crates/mathscape-store-sqlite/tests)：升级、重开、事务与幂等；[服务测试](../apps/mathscape-desktop/tests/service.rs)：真实 Pool/SQLite 与模拟外部服务。
- [前端测试](../frontend/tests)：实际 PDF.js/KaTeX/交互，替换 Tauri IPC；不等同于真实 WebView 端到端验证。

默认测试不需要 API key、真实 Marker 模型或私人教材。LLM 使用 scripted backend 或本地 HTTP/SSE 模拟器；真实材料效果、遗漏率、数学质量和费用必须独立评估，工程测试通过不能替代这些实验。

## Fixture 与协议演进

小型可分发固定材料放 [testdata](../testdata/)，保留来源摘要。大型语料由 `MATHSCAPE_CORPUS_ROOT` 定位，默认测试不能依赖它。本地 Markdown 锚点回归可用 `MATHSCAPE_MARKDOWN_FIXTURE` 与对应 ignored test。`mathscape-fixture` crate 尚是占位，不能假设已有通用导入器。

Schema 从 Rust DTO 生成，提交快照用于检测漂移；不要只改快照绕过契约测试：

```sh
cargo run -p mathscape-orchestrator --example staged_discovery_schema --quiet -- units
cargo run -p mathscape-orchestrator --example staged_discovery_schema --quiet -- facts
cargo run -p mathscape-orchestrator --example staged_discovery_schema --quiet -- repair
cargo run -p mathscape-orchestrator --example discovery_workflow_demo --quiet
```

前三条只输出 Schema，最后一条离线回放工作流。旧单阶段生成器 `discovery_schema` 对应当前 v3，不用于重写 v1/v2 历史快照。输入或输出语义不兼容时升级契约；Prompt 指令变化记录 revision。旧包和兼容反序列化的去留由回放与测试依赖决定。

## 数据库升级

[migrations](../migrations/) 顺序追加；已发布 migration 不原地改写。升级在打开 Workspace 时执行，必须覆盖已有数据库，而不是要求用户删库重建。检查旧版本升级、重开和拒绝未知未来版本；历史异常 schema 的修复应有专门回归。

跨文件与 SQLite 的失败边界、接受与提交的幂等不可破坏。修改持久 DTO 或身份规则时同时检查重验证、旧运行回放和登记冲突；不要将反序列化当作恢复可信对象。

## 故障定位

| 症状 | 优先检查 |
| --- | --- |
| 材料缺失或迁移后读不到 | Workspace 根、pool URI、目录位置与完整性检查；不要改内容身份迁就损坏文件 |
| Marker 不可用或转换失败 | 绝对执行路径、对应 Python/Marker 版本、本次 staging 的 stdout/stderr 与 inventory；保留失败材料供诊断 |
| 模型连接失败或断流 | profile 协议、endpoint、钥匙串、连接/总期限、规范化错误与终止事件；不靠隐藏重试掩盖错误 |
| 发现结果被拒绝 | attempt 与阶段诊断、原始 quote、授权范围、owner、父链和 coverage；不放宽数学字符匹配来消除错误 |
| 接受或登记失败 | 结果指纹、原文重绑定、选择闭包、登记计划/回执与事务冲突；计划存在不代表来源已登记 |
| 图缺对象或回链异常 | 权威快照、当前 revision 端点、投影与转换路径映射；先查数据再查布局 |

测试诊断常位于 `.test-runs/`；只清理属于相应测试的临时目录，不删除开发 Workspace 来掩盖升级问题。完整请求和模型输出属于私有材料，排障分享前需去除敏感内容与密钥。

## 文档规则

README 加四份主文档构成新人阅读路径。新增能力优先更新对应主文档；只有形成独立、长期维护的边界才新增文档。字段、函数签名和测试细节由代码、Schema 与测试承担；协议版本放在版本化资源中，主文档文件名不递增 v1/v2/v3。历史开发记录交给 Git，探索方向不能写成已实现规范。
