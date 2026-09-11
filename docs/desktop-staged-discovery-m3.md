# Desktop 分阶段来源发现 M3

状态：M3 的请求切换、配置映射、结构化事件、活动任务控制、稳定结果投影和来源高亮交互均已
完成。真实多案例比较是下一步验收工作；SQLite 耐久化仍待实现。

## 默认执行链

Desktop 的 `run_discovery` 请求仍只接受后端签发的 Markdown 快照与选区，以及可选 profile ID。
服务端重新读取 Artifact、验证快照和选区，构造 `PreparedUnitDiscoveryRequest`，随后执行：

```text
阶段 A → 单元确定性验证 → UnitSet 检查点
                              ↓
                        FactTask Planner
                              ↓
阶段 B（可分批）→ 事实验证 → 一次有界修订 → 组装结果
```

旧 `DiscoveryProducer`、v3 Prompt 与 validator 仍保留在 orchestrator 中供离线回放和比较，
不再是 Desktop 默认路径。“构建发现输入”仍是无模型调用的选区检查入口。

Workspace 的 Discovery 请求超时现在是整个工作流的总期限；连接超时仍是单次连接上限。
模型 profile 与 admission 在启动时形成快照。admission 不阻断 A 到 B 的自动交接，也不会让
当前运行隐式接受或写图。调用数、批次和每批任务数使用工作流的受限默认值。

## 进度与活动任务

Orchestrator 发出带类型的工作流事件：工作流开始、attempt 开始/结束、阶段验证、UnitSet
检查点、事实计划、事实检查点、组装和最终状态。attempt 事件包含阶段、是否修订和批次位置；
LLM 的 text/reasoning 流通过当前 attempt 上下文发给 Desktop，因此多个模型响应不会再拼成
一个貌似损坏的 JSON。

`get_active_discovery` 返回当前阶段、attempt、批次、单元数、事实任务完成数、总期限和取消
状态。一个内存 permit 覆盖整个 A/B/修订流程，批次间隙也不能重复提交。切换阅读页不会释放
任务，其他阅读页可通过轮询恢复进度。

`cancel_discovery` 通过 watch 通道唤醒正在等待的 Desktop 执行。执行端先丢弃借用工作流的
在途 future，再调用 `workflow.cancel()`；已获得的内存检查点进入返回结果，半截传输不会被
送入确定性验证。运行返回或报错后 permit 自动释放。

## 返回边界与展示投影

M3 返回 `DesktopDiscoveryOutcome`，其 `view` 是版本化的
`mathscape-desktop-discovery-view-1`，见 [展示 DTO v1](desktop-discovery-view-v1.md)。它由内存中
已经验证的 UnitSet 与 DiscoveryResult 投影生成，不能反序列化回可信工作流。DTO 明确排除
完整 Prompt、授权原文和模型原始正文，避免将 M2 审计快照直接通过 IPC 发送。

前端按 attempt 分组显示流式内容和阶段状态；阅读器展示单元、来源标签、引用解析状态、关系
依据、问题、pending tasks 和诊断。每个可定位对象携带 Artifact 内绝对 UTF-8 范围。点击时
阅读器滚动并标记相关渲染块，同时显示带精确 `<mark>` 的 Markdown 原文片段：公式渲染后的
KaTeX DOM 不作为字符定位依据。

所有检查点仍只存在于本次命令内存和返回值中。应用重启后的查看/续跑属于 M4，不应由当前
活动任务控制冒充持久化。

## 验证

Rust 服务测试通过本地 OpenAI-compatible SSE 模拟器执行真实的 A、B 两次调用，验证请求顺序、
总期限、跨阶段排他锁、结果不写图和取消释放。Orchestrator 测试验证结构化事件包含 B 批次
位置与最终状态。浏览器测试验证分 attempt 流式显示、切换阅读页及过渡结果。

```sh
cargo test -p mathscape-orchestrator --test discovery_workflow
cargo test -p mathscape-desktop --test service
npm --prefix frontend run test:reader
```
