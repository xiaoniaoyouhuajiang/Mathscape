# Mathscape Ink Lab P0 / P0.1

电脑运行实验服务，平板扫码进入浏览器画板；手稿可以完整导出为 Ink JSON，再重新导入继续书写。P0.1 在 P0 闭环上增加了稳定笔迹层、变换/局部擦除预览、控制点命中反馈和局部擦除候选过滤，真实 Tab S8+ / S Pen 和数位板的体验验收仍需实测。

## 启动与使用

在 `mathscape/` 执行：

```sh
npm --prefix frontend install
npm --prefix frontend run ink:lab
```

命令会构建独立画板，并在默认 1440 端口启动 Node HTTP harness。打开终端打印的完整电脑页面地址；其中的凭据仅供本次实验管理，不能用不带凭据的 `/host` 地址替代。服务重启后会生成新的凭据，旧链接需要重新扫码。

1. 电脑和平板连接同一局域网；在电脑页面选择平板能访问的 IPv4 地址。
2. 平板扫码或打开“复制地址”得到的画板链接。地址中的 `#session=...` 会保留在浏览器地址栏，便于诊断和跨设备复制；它属于本次服务的临时凭据。`127.0.0.1` 只供电脑本机试写。
3. 用笔书写，默认触摸用于拖动或双指缩放；需要手指书写时勾选对应选项。鼠标左键书写，中键平移，滚轮缩放。
4. “新增页面”创建新的固定尺寸页面；页面选择器切换页面。当前页面内容仍会保留在同一份 Ink JSON 中。
5. “套索选择”圈选整笔后，拖动方框内部可移动，拖动四角可缩放，拖动上方圆点可旋转。变换会记录为可撤销的操作。
6. “整笔擦除”移除碰到的整笔；“局部擦除”沿笔迹路径切出剩余片段。擦除时会显示圆形擦除器和受影响片段的预览，抬笔后才写入操作。撤销和重做保留原始样本与操作历史。被取消或中断的笔迹也保留状态。
7. “导出 Ink JSON”生成浏览器下载；确认文件已保存。PNG 是当前页的附加预览，不包含编辑历史。
8. “保存到电脑”提交完整 JSON；收到成功回执后，可在电脑页面下载，或在 `.mathscape-dev/ink-lab/exports/` 找到按 SHA-256 命名的文件。
9. “导入 Ink JSON”先验证文件，再重现内容和历史；继续书写会新增采集片段。无效文件不会替换当前画板。

Ctrl+C 停止服务，本次链接随之失效。已写入实验目录的文件仍保留；电脑页面的接收列表仅反映本次运行。画板首次连接遇到短暂网络失败时会自动重试，仍离线时使用“重试连接”；右上角显示“电脑可连接”后再点击“保存到电脑”。刷新画板前请手动导出，P0 不提供自动保存或断线队列。电脑失联时，已经打开的画板仍可书写和本地导出。

需要更换端口或只在本机运行时：

```sh
INK_LAB_PORT=1442 npm --prefix frontend run ink:lab
INK_LAB_BIND=127.0.0.1 npm --prefix frontend run ink:lab
```

只绑定本机时局域网二维码不能供平板连接。默认服务只开放打包后的实验静态资源和带凭据的实验 API，不开放 Vite 开发服务器、仓库文件、Tauri 命令或 Workspace。

## 文件契约与输入记录

当前版本是 `mathscape-ink-lab-1`，Rust DTO 位于 `crates/mathscape-domain/src/ink.rs`，Schema 与共享样本位于 `testdata/ink/p0/`。TS 使用同版本契约和共享有效/无效 fixtures；后续语义变化需升级版本。

JSON 保存文档与固定页面、采集片段、笔刷、完整笔迹样本，以及 stroke/erase/undo/redo 历史。一笔终结后形成一个 stroke command；P0 没有逐批远程提交或 durable operation ACK。发送文件时，服务端重新校验形状、引用、顺序和历史，成功写入文件并同步后才返回文件回执。

页坐标原点为左上，x 向右、y 向下；默认页面为 1200 × 1600 逻辑单位。坐标以有限 JSON number 保存，viewport 与 DPR 不改写笔迹。笔迹开始后移出页面的样本仍可保留，显示与 PNG 只裁剪画面，不裁剪 JSON 样本。

`t_us` 是 PointerEvent.timeStamp 转换成的非负整数微秒，基于该采集片段的 performance time origin；不是墙上时钟，也不能在片段之间直接比较。JavaScript 安全整数是上限。重新导入后的新书写使用新的 capture_segment_id。

pressure 为 API 报告的 [0,1] 值，tilt 单位为度；非笔输入使用显式 null。这些可空字段必须存在。P0 的传感器能力始终记为 unknown；API 有字段或笔压有变化不等于已认证硬件能力。实际数值保留，不为了显示或测试而四舍五入。渲染固定关闭模拟压力，原始样本不被平滑轮廓替换，也不加入预测点。

画板采集 pointermove 及其可用的 coalesced events，不同时采集 pointerrawupdate，以免重复记录。诊断显示 rawupdate API 是否存在，但不能将其理解为已经使用。HTTP LAN 页面通常不是安全上下文，缺少合并采样 API 时退回普通 Pointer Events；必须在设备实验中记录这种差异。P0.1 的诊断还显示局部擦除候选笔迹数和当前预览笔迹数，用于后续真实设备性能收敛。

实验上限为每份文件 64 MiB、500,000 个样本、20,000 个操作、单笔 100,000 个样本，页面和采集片段也有明确上限。达到限制会显示提示并保留已有内容。阈值是原型资源界限，尚不是经过实机确认的产品性能门槛。

这些实验文件不属于 Artifact Pool，不宣称已经完成 EMIR 来源登记或数学验证。P0 Ink Lab 仍是手动导出实验入口；P1 已在 SQLite、桌面命令和 IndexedDB outbox 中建立本地可靠保存骨架，桌面手写页面的自动保存接线、P2 的受管快照与 Ink locator、P3 的正式配对和断线续传另行接入。

桌面 P1 验收使用 `npm --prefix frontend run tauri:dev:local_1` 启动应用，点击顶部“手写”。新建或打开手稿后书写，状态栏应显示本地 Workspace 保存；返回地图后再次点击“手写”，同一会话的笔迹和操作 head 应恢复。P0 Ink Lab 的扫码页面仍用于平板 LAN 输入实验，不代表桌面自动保存已经走 LAN 协议。

## 自动化验证

在 `mathscape/` 执行：

```sh
cargo test -p mathscape-domain --test ink_lab
npm --prefix frontend run test:unit
npm --prefix frontend run test:ink-server
npm --prefix frontend run test:ink-browser
```

首次运行浏览器测试可能需要在 `frontend/` 执行 `npx playwright install chromium`；也可按项目现有约定设置 `PLAYWRIGHT_CHROMIUM_EXECUTABLE`。测试产物位于 `.test-runs/ink-browser/`，使用独立 1451 端口，不写入用户实验目录。

测试覆盖共享契约、取消/中断状态、跨采集片段、擦除与撤销、无效引用和历史、二维码页面、鼠标书写、JSON 下载及导入后的像素重现、文件接收、离线导出、凭据与来源限制，以及写入失败。合成 pen events 检查 API 值的保留，不证明真实 S Pen 的压感、掌拒、按钮或延迟。

Schema 重新生成：

```sh
cargo run -p mathscape-domain --example ink_lab_schema --quiet > testdata/ink/p0/schema.json
```

## 实机实验

每次测试保留完整 Ink JSON，并填写[设备实验记录](../../testdata/ink/p0/device-report.md)。至少记录设备、OS/驱动、浏览器或 WebView 版本、HTTP/安全上下文、压力变化、掌触、按钮和 pointercancel，以及一段 20 分钟数学书写。

“最近前景绘制 ms”只测 JS 前景绘制耗时，不是触笔到屏幕的物理延迟。性能门槛和是否需要 AndroidX Ink，应根据真实输入和持续书写结果决定。
