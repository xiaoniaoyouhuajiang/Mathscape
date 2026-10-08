问题:
> 我现在正在考虑为mathscape增加手写输入：数学学习者/研究者的学习工作中会产生大量手稿，可能是一些数学思维发散随记，题目或定理的证明尝试，或者是草稿，这些内容可以很大程度折射出其解决思路，对于mathscape来说，一方面这些记录是笔记，另一方面，他们也是llm了解人类思路的极佳输入。并且现在许多工作者习惯无纸化的办公和学习，比如我现在学习理工科相关的内容，假如需要做题，我一般会使用自己的samsung s8 plus来做笔记或写推导过程。当前的mathscape是一款桌面软件，因此我也构思过如何让这些手写内容进入到软件中，再考虑其存储结构前，我认为需要先考虑交互模型，我的大致想法：一个是mathscape软件本身能够支持平板使用（目前只需要考虑android系统）+ 主流对数位板兼容，另一种是电脑开启server端，平板进入局域网，然后通过专门的适配器软件便利地将手写内容传入到我们的软件中，请你调研开源软件生态（rust,ts）来看看如何为这块的开发奠定方向

调研结果:
这个方向和 Mathscape 的目标非常契合，而且我认为它不应该被定义成一个“手写板功能”，而更应该定义成 **Mathscape 的第三类输入模态：Digital Ink / Human Reasoning Trace**。

因为对 Mathscape 来说，真正有价值的不是“把一张手写图片塞进笔记”，而是保留：

- 人写了什么；
- **按什么顺序写的**；
- 哪些东西被划掉、圈出、连接；
- 一次证明尝试是怎样逐步展开的；
- 哪一段是草稿、哪一段是阶段性结论；
- 它当时对应的是哪道题、哪条定理、哪篇论文、哪个 Mathscape/EMIR 节点。

所以我调研开源生态之后，最重要的结论其实是：

> **不要在“做 Android App”和“做局域网手写适配器”之间二选一。**
>
> 应该先设计一个统一的 `InkSession / InkDocument` 层，上面接三种采集端：
>
> **Desktop 数位板 → Web/TS Ink Canvas**
>
> **Android Mathscape → Web Ink 或 AndroidX Ink**
>
> **任意平板浏览器 → LAN InkPad → Desktop Mathscape**
>
> 三者最后产生完全相同的 Mathscape Ink 数据。

---

# 1. 我建议的整体架构

可以先把未来的结构想成这样：

```text
                        Mathscape
                           │
                    ┌──────▼───────┐
                    │ Ink Domain   │
                    │ InkSession   │
                    │ InkStroke    │
                    │ InkRegion    │
                    │ Checkpoint   │
                    └──────┬───────┘
                           │
          ┌────────────────┼─────────────────┐
          │                │                 │
          ▼                ▼                 ▼
   Desktop InkCanvas   Android Ink      Remote InkPad
      TS/WebView        Android App        Browser
          │                │                 │
 PointerEvent         AndroidX Ink       PointerEvent
          │            MotionEvent             │
          │                │              WebSocket
          └────────────────┴─────────────────┘
                           │
                    Canonical Ink
                           │
            ┌──────────────┼──────────────┐
            ▼              ▼              ▼
         Render        LLM/VLM        Mathscape/
        PNG/SVG       Interpretation      EMIR
```

这里最重要的一点是：

**Ink Canvas 不是数据模型。**

Canvas / AndroidX Ink / Excalidraw / perfect-freehand 都只是采集和渲染层。

真正应该属于 Mathscape 的，是自己的：

```text
InkSession
InkPage
InkStroke
InkPoint
InkRegion
InkCheckpoint
InkContextRef
```

这会让以后：

- 换渲染器；
- Android 改成原生；
- 加 iPad；
- 加在线同步；
- 加 OCR；
- 加 LLM；
- 加 handwriting replay；

都不会推翻底层设计。

---

# 2. 一个很好的消息：Tauri + Android 这条路现在已经成熟很多了

如果 Mathscape 本身已经是 Rust + TS 方向，那么 **Tauri 2 是非常自然的技术基础**。

Tauri 2 已经正式支持 Android 开发，可以直接运行：

```text
tauri android dev
tauri android build
```

Android 底层仍然是标准 Android Studio 工程。[Tauri](https://v2.tauri.app/develop/?utm_source=chatgpt.com)

更重要的是，Tauri Mobile Plugin 可以：

```text
TS
 ↓
Tauri
 ↓
Rust
 ↓
Kotlin Android Plugin
 ↓
Android native API
```

官方插件体系允许 Android 部分直接写 Kotlin，并从 JS 或 Rust 调用。[Tauri](https://v2.tauri.app/develop/plugins/develop-mobile/?utm_source=chatgpt.com)

这意味着你不需要为了 S Pen：

> “放弃 Rust/TS → 整个 Mathscape Android 重写成 Kotlin。”

完全可以是：

```text
95% Mathscape
Rust + TS

5% platform-specific
Kotlin Ink Adapter
```

这非常适合你现在的技术栈。

---

# 3. 更关键的是：Android 最近已经有了相当完整的官方 Ink Engine

这是我这次调研里觉得最值得关注的东西：

## AndroidX Ink

Google 的 Jetpack `androidx.ink` 已经发布 **1.0.0 Stable**，当前官方页面在 2026 年 9 月 23 日仍继续维护 1.1 alpha。[Android Developers](https://developer.android.com/jetpack/androidx/releases/ink)

它不是简单的：

> “给你一个 MotionEvent，你自己画线。”

而是已经拆出了完整的数字墨迹体系：

```text
ink-authoring
ink-brush
ink-geometry
ink-rendering
ink-storage
ink-strokes
```

官方明确把 Authoring 定义为面向：

> 高性能、低延迟实时 Ink Rendering

并且包含 selection / eraser 所需的几何运算等能力。[Android Developers](https://developer.android.com/jetpack/androidx/releases/ink)

Android 原生 `MotionEvent` 又可以提供：

- x/y
- pressure
- orientation
- tilt
- hover
- stylus / eraser 区分

等信息。[Android Developers](https://developer.android.com/reference/android/view/MotionEvent.html?utm_source=chatgpt.com)

同时 Android 官方还有 motion prediction：

```text
real samples
      │
      ▼
MotionEventPredictor
      │
      ▼
predicted points
      │
      ▼
lower perceived latency
```

而且预测出的 MotionEvent 仍然包含坐标、pressure、orientation、tilt。[Android Developers](https://developer.android.com/develop/ui/compose/touch-input/stylus-input/advanced-stylus-features?utm_source=chatgpt.com)

这其实解决了一个很重要的问题：

> 如果未来 Mathscape Android 真的希望做到 Samsung Notes / OneNote 那种“笔尖跟手”，纯 HTML Canvas 不一定应该承担最后那 10% 的体验要求。

现在可以直接让 AndroidX Ink 承担。

---

# 4. 但我不建议第一天就上 AndroidX Ink

这是我认为比较关键的工程选择。

首先做：

## TS Ink Canvas

浏览器标准 `PointerEvent` 已经原生提供：

```ts
event.pointerType
event.pressure
event.tiltX
event.tiltY
event.width
event.height
```

其中 `pointerType` 可以区别：

```text
mouse
pen
touch
``` :chatgpt-content-reference{index="6"}


因此同一个 TS Ink Canvas 可以跑在：

```text
Chrome Android
        │
Tauri Android WebView
        │
Tauri Windows WebView2
        │
普通桌面浏览器
        │
局域网 Tablet Browser
```

这就是最值得利用的统一层。

---

# 5. TS 手写层，我最推荐 `perfect-freehand`，而不是上来采用一个完整白板

有几个值得看的项目。

## perfect-freehand

这是我认为最值得直接采用的。

它：

- TypeScript；
- MIT；
- 专门处理 pressure-sensitive freehand；
- 接受 `[x, y, pressure]`；
- 支持 thinning / smoothing / streamline；
- 可以生成漂亮的自由笔迹。 [GitHub](https://github.com/steveruizok/perfect-freehand/blob/main/packages/perfect-freehand/src/types.ts?utm_source=chatgpt.com)


它正好对应：

```text
PointerEvent samples
       │
       ▼
[x, y, pressure]
       │
       ▼
perfect-freehand
       │
       ▼
stroke outline
       │
       ▼
Canvas / SVG
```

Mathscape 并不需要先造：

> 钢笔笔刷、路径平滑、速度宽度映射……

这些可以直接复用。

---

# 6. Excalidraw 很值得借鉴，但我不会让 Mathscape Ink 基于 Excalidraw 建模

Excalidraw 当前仍然是 MIT，并支持：

- infinite canvas
- free draw
- eraser
- zoom/pan
- shapes
- image
- undo/redo
- JSON open format
- PWA
- local-first
- collaboration [GitHub](https://github.com/excalidraw/excalidraw?pubDate=20260927)


所以它非常适合作为：

> **Mathscape Ink Workspace 的交互参考实现。**

尤其值得研究：

```text
canvas viewport
selection model
eraser
undo
element model
zoom/pan
mobile gestures
```

但我不建议：

```text
Mathscape InkDocument = ExcalidrawDocument
```

因为以后你的 Ink 数据还要承担：

```text
LLM reasoning input
stroke temporal analysis
proof attempt reconstruction
semantic grouping
EMIR linking
```

这已经超出普通 whiteboard document 的范畴。

---

# 7. tldraw 我反而建议谨慎

tldraw 的 SDK 很优秀，但目前默认 license **并不是普通的 permissive open-source license**。

官方 license 当前说明：

- development 可以使用；
- production 需要 trial / hobby / commercial license；
- SDK 本身不是传统意义上的 Open Source。[GitHub](https://github.com/tldraw/tldraw/blob/main/LICENSE.md?utm_source=chatgpt.com)

所以如果 Mathscape 以后存在商业化可能：

> 可以研究 tldraw 的交互，
> 不建议把核心 Ink 系统绑定到 tldraw SDK。

相比之下：

**perfect-freehand + 自己的 Canvas model**

会干净很多。

---

# 8. Rust 世界有一个非常值得研究的项目：Rnote

这个项目我非常建议你们阅读源码。

Rnote 是：

> Rust + GTK4 的开源手写笔记应用。

它已经实现：

- pressure-sensitive stylus；
- handwritten notes；
- infinite canvas；
- selection；
- PDF/image annotation；
- configurable pen；
- tablet button shortcuts；
- vector strokes。 [GitHub](https://github.com/flxzt/rnote?utm_source=chatgpt.com)


而且它的内部架构很有参考价值：

```text
rnote-compose
    basic geometry / paths / pen paths

rnote-engine
    store
    document
    fileformats
    pens
    strokes

rnote-ui
    GTK UI
```

其中 `rnote-engine`：

- stroke 存在独立 store；
- pen 是 interaction abstraction；
- brush / eraser / shaper 都是不同 pen；
- engine 管 undo；
- UI 与 engine 分离。[GitHub](https://github.com/flxzt/rnote/blob/main/CONTRIBUTING.md)

这个架构理念和我认为 Mathscape 应该采用的非常相似：

```text
Ink Input Adapter
        ↓
Ink Engine
        ↓
Ink Document
        ↓
UI / AI / Storage
```

不过：

**Rnote 是 GPL-3.0-or-later。** [GitHub](https://github.com/flxzt/rnote?utm_source=chatgpt.com)

所以如果 Mathscape 不是 GPL 项目：

> 非常适合研究架构，
> 不适合直接搬代码。

---

# 9. 数位板兼容：我反而建议首先依赖系统 Pointer API

这里容易走一个弯路：

> “为了支持 Wacom / XP-Pen / Huion，是不是要分别接他们的 SDK？”

我的建议是：

**不要。**

优先路线：

```text
Tablet Driver
     ↓
OS Pointer / Windows Ink
     ↓
Chromium / WebView
     ↓
PointerEvent
     ↓
Mathscape
```

这样 Mathscape 根本不需要认识：

```text
Wacom
XP-Pen
Huion
Gaomon
Veikk
...
```

只认识：

```text
pointerType = pen
pressure
tilt
buttons
```

这也是为什么我觉得 **TS/WebView 在 Mathscape 这里反而是优势，而不是妥协**。

---

# 10. OpenTabletDriver 可以作为兼容性参考和 fallback

还有一个很值得纳入测试体系的项目：

**OpenTabletDriver**

它是跨平台 user-mode tablet driver，目前支持：

- Windows
- Linux
- macOS
- pressure
- tilt
- pen buttons
- express keys
- many tablet brands [GitHub](https://github.com/OpenTabletDriver/OpenTabletDriver?utm_source=chatgpt.com)


不过我不会把它作为 Mathscape runtime dependency。

更合理的是：

```text
Primary:
OS driver → PointerEvent

Fallback/debug:
OpenTabletDriver

Device compatibility tests:
Wacom / Huion / XP-Pen / Gaomon ...
```

这样能避免 Mathscape 变成一个“驱动程序维护项目”。

---

# 11. 你提出的“电脑 Server + 平板局域网”其实特别值得优先做

因为这个模式可以非常快地验证：

> 用户到底愿不愿意把 Mathscape 当作手写思维空间使用。

我甚至建议把它设计成 Mathscape 的正式功能：

## `Open Ink Pad`

用户在 Desktop Mathscape 点：

```text
Start handwriting session
```

Desktop 出现：

```text
┌────────────────────────────┐
│ Write on another device    │
│                            │
│       [ QR CODE ]          │
│                            │
│ Galaxy Tab S8+             │
│ Connected                  │
└────────────────────────────┘
```

平板扫码：

```text
http://192.168.x.x:xxxx/ink/<token>
```

然后就是一个极简页面：

```text
┌───────────────────────────────┐
│ Problem 3                     │
│                               │
│ ───────────────────────────── │
│                               │
│        handwriting area       │
│                               │
│                               │
│                        ✓ Done │
└───────────────────────────────┘
```

桌面实时出现笔迹。

---

# 12. Rust server 端这部分非常简单

可以直接：

```text
Mathscape Desktop
      │
      ├── Axum HTTP server
      │
      ├── static InkPad frontend
      │
      └── WebSocket
              │
              ▼
          Tablet
```

`axum` 本身就是 Tokio 生态中的 HTTP framework，并直接支持 WebSocket。[GitHub](https://github.com/tokio-rs/axum?utm_source=chatgpt.com)

所以：

```rust
Mathscape
  └─ ink-server
       ├─ GET /ink/:session
       ├─ WS /ink/:session/ws
       └─ POST /ink/:session/commit
```

已经足够。

甚至早期完全不需要 WebRTC。

---

# 13. 配对我建议“二维码优先，mDNS 辅助”

Rust 已经有非常成熟的：

`mdns-sd`

目前支持：

- service publish；
- service discovery；
- Windows / macOS / Linux；
- IPv4 / IPv6；
- sync / async。 [Docs.rs](https://docs.rs/mdns-sd/latest/mdns_sd/?utm_source=chatgpt.com)


可以广播：

```text
_mathscape-ink._tcp.local
```

Android app 自动发现 Mathscape。

但我不会把 mDNS 作为第一版唯一方案，因为真实企业/校园网络里：

```text
AP isolation
multicast filtering
firewall
multiple NIC
VPN
```

都可能把它搞坏。

因此最可靠的是：

```text
QR = IP + port + random session token
```

mDNS 只是 UX enhancement。

---

# 14. Browser InkPad 有一个值得现在就知道的限制

普通 PointerEvent 没问题。

但浏览器为了性能会合并多个 `pointermove`。

Web API 提供：

```js
event.getCoalescedEvents()
```

让绘画程序把这些原始采样点取回来，这对高质量画线很有价值。[MDN Web Docs](https://developer.mozilla.org/en-US/docs/Web/API/PointerEvent/getCoalescedEvents?utm_source=chatgpt.com)

问题是：

**`getCoalescedEvents()` 属于 secure-context API。**

也就是通常需要 HTTPS。[MDN Web Docs](https://developer.mozilla.org/en-US/docs/Web/API/PointerEvent/getCoalescedEvents?utm_source=chatgpt.com)

于是：

```text
https://mathscape.local
        ✓

http://localhost
        ✓ 特殊可信

http://192.168.1.10
        ↓
LAN tablet
        ⚠ 某些高级 Web API 不可用
```

因此我会把：

### LAN Browser

定位成：

> 零安装、极便利、非常适合第一阶段和临时输入。

而把：

### Android Mathscape Companion

定位成：

> 最终高品质 S Pen 输入方案。

这两者正好互补。

---

# 15. Android 最终版我建议一个“渐进增强”模型

不要：

```text
Day 1:
重做 native Android canvas
```

而是：

## Level 1

```text
Tauri Android
      ↓
shared TS InkCanvas
      ↓
PointerEvent
```

直接测试你的 Tab S8+。

如果体验已经很好：

**结束。**

没有必要为了“native”而 native。

---

## Level 2

如果发现：

- 笔迹滞后；
- palm rejection 不理想；
- S Pen hover 缺失；
- eraser/button 行为不稳定；
- 高频采样丢失；

再添加：

```text
Tauri mobile plugin
       ↓
Kotlin
       ↓
AndroidX Ink
```

---

## Level 3

甚至可以做一个：

```text
InkActivity
```

即：

```text
Mathscape Web UI

     ↓ Open handwriting

Android native full-screen
Ink Workspace
(AndroidX Ink)

     ↓ Done

InkStroke[]
     ↓
Rust
     ↓
Mathscape
```

这里还有一个重要工程建议：

> **不要每一个 S Pen point 都 Kotlin → Rust → JS。**

会把 bridge 变成高频数据管线。

应该是：

```text
native rendering
native buffering

stroke end
    ↓
batch stroke
    ↓
Mathscape Ink engine
```

这样 architecture 会稳很多。

---

# 16. Mathscape 的 Ink 数据一定不要只保存 SVG/PNG

虽然你说存储结构准备之后再讨论，但这里和交互架构直接相关，因此我建议现在就确定一个原则：

> **Raw stroke 是 source of truth。**

例如概念上：

```ts
interface InkPoint {
  x: number
  y: number

  time: number

  pressure?: number

  tiltX?: number
  tiltY?: number

  azimuth?: number
  altitude?: number
}

interface InkStroke {
  id: string

  tool: "pen" | "highlighter" | "eraser"

  points: InkPoint[]

  brush: BrushSpec
}

interface InkSession {
  id: string

  context: MathscapeContext

  strokes: InkStroke[]

  checkpoints: InkCheckpoint[]
}
```

然后：

```text
Raw samples
    ↓
InkStroke
    ├── render SVG
    ├── render PNG
    ├── replay
    ├── handwriting recognition
    ├── LLM input
    └── reasoning timeline
```

而不是：

```text
canvas
 ↓
PNG
 ↓
save
```

后一种会直接丢掉 Mathscape 最有价值的东西：

**书写时间序列。**

---

# 17. MathWriting 数据集也证明了这一点

Google 的 **MathWriting Dataset** 本身就是：

> online handwritten mathematical expressions

而不是单纯的图片。

其中每个 handwriting sample 存成 **InkML**，保留 ink stroke 信息，同时有 normalized math label。[GitHub](https://github.com/google-research/google-research/blob/master/mathwriting/archive_readme.md?utm_source=chatgpt.com)

这恰好说明对于数学手写识别：

```text
stroke sequence
```

本身就是非常重要的数据模态。

Mathscape 如果从第一天就保留它，以后可以做很多 PNG 做不了的东西。

---

# 18. 甚至不需要第一版就解决“手写转 LaTeX”

我认为这是另一个很容易把项目带偏的地方。

第一版真正重要的是：

```text
capture
↓
preserve
↓
contextualize
↓
LLM understand
```

不是：

```text
capture
↓
perfect OCR
↓
LaTeX
```

今天的 multimodal LLM 本身就能直接理解：

```text
handwriting screenshot
+
problem context
+
source material
```

所以可以：

```text
Ink strokes
   │
   ├── raw vector → 保存
   │
   └── render high-res image
                 │
                 ▼
                LLM
```

等以后需要结构化 LaTeX 再加 HMER。

而 HMER 本身现在的开源研究生态也已经很丰富，例如：

- BTTR；
- CoMER；
- CAN；
- MathWriting；
- 2025/2026 的 VLM-based HMER。

例如 Google MathWriting 本身就是很大的 online handwriting corpus；Uni-MuMER 这类近年的工作甚至已经把 VLM 引入 handwritten mathematical expression recognition。[GitHub](https://github.com/google-research/google-research/blob/master/mathwriting/archive_readme.md?utm_source=chatgpt.com)

所以完全没必要让 OCR 阻塞 Ink subsystem。

---

# 19. 更重要的是，我会把 Mathscape 的交互定义成 `Ink Session`，而不是“笔记页”

这可能是我对这个功能最核心的产品建议。

例如你现在在 Mathscape 中学习：

```text
Theorem:
Every compact subset of R^n is closed and bounded.
```

旁边有一道题。

点：

```text
✎ Work on this
```

Tab S8+ 自动打开：

```text
┌──────────────────────────────────┐
│ Exercise 4.7                     │
│                                  │
│ Show that ...                    │
├──────────────────────────────────┤
│                                  │
│                                  │
│          handwriting             │
│                                  │
│                                  │
└──────────────────────────────────┘
```

于是这个 Ink Session 天然有：

```text
context =
{
  source_unit: Exercise 4.7,
  nearby_theorem: Heine-Borel,
  mathscape_node: ...,
  document: Rudin,
}
```

这和普通 Samsung Notes 最大的区别就出现了。

---

# 20. 它还可以保留“思考过程”，而不仅仅是最后答案

例如你写：

```text
suppose xn -> x

        ↓

try Cauchy?
       ×

bounded subsequence
       ↓

Bolzano-Weierstrass
       ↓

...
```

期间有：

```text
t = 0s
stroke...

t = 12s
stroke...

t = 48s
cross-out...

t = 75s
circle...

t = 83s
new branch...
```

Mathscape 实际得到的是：

```text
reasoning trace
```

而不是：

```text
final handwritten image
```

以后甚至可以 replay：

```text
▶ 0:00 ━━━━━●━━━━━━━━ 3:42
```

让模型判断：

> 最初尝试了 Cauchy 思路；
>
> 在某处放弃；
>
> 随后改用 Bolzano–Weierstrass；
>
> 当前困难集中在……

这种数据对于你们正在构思的 Mathscape，比单纯“手写转文本”有价值得多。

---

# 21. LLM 交互上，我不建议默认每一笔都实时分析

我会做成明确的 checkpoint：

```text
[ Ask Mathscape ]
```

或者 lasso 一块：

```text
┌───────────────┐
│ handwritten   │
│ region        │
└───────────────┘

Ask:
○ What am I missing?
○ Check this step
○ Formalize this
○ Find related theorem
○ Continue from here
```

因为学习草稿天然包含大量：

```text
false start
temporary notation
cross-outs
incomplete equations
private shorthand
```

实时追着每一笔解释反而会打扰思考。

理想交互应该是：

> **人自由思考 → Mathscape忠实记录 → 用户主动选定分析节点。**

---

# 22. 同步技术暂时不需要一开始就上 CRDT

Rust/TS 领域当前当然有很强的选择。

例如：

### Yjs + Yrs

`Yrs` 是 Yjs 的 Rust port，目标就是：

> 与 Yjs 行为及 binary protocol 兼容。[GitHub](https://github.com/y-crdt/y-crdt?utm_source=chatgpt.com)

非常适合：

```text
TS
Yjs
  ↕
Rust
Yrs
```

### Automerge

现在核心就是 Rust 实现，再暴露 JS/WASM API，并定位于 local-first collaborative applications。Automerge 3 还显著降低了 memory usage。[GitHub](https://github.com/automerge/automerge?utm_source=chatgpt.com)

不过第一阶段我都不会用。

因为：

```text
Tablet
   ↓
Desktop host
```

本质上还是：

**single-writer / client-server**

最简单可靠的协议就是：

```text
stroke.begin
stroke.points
stroke.end

ack
```

网络断掉：

```text
tablet local queue
        ↓
reconnect
        ↓
resend unacked strokes
```

已经足够。

以后真正出现：

```text
desktop editing
+
tablet editing
+
offline tablet editing
+
multi-device
```

再引入 Yjs/Yrs 或 Automerge。

---

# 23. 数据协议我会倾向 Protobuf，但 POC 可以先 JSON

Rust：

```text
prost
```

TS：

```text
ts-proto
```

`ts-proto` 可以直接从 `.proto` 生成 TypeScript 类型和 encoder/decoder。[GitHub](https://github.com/stephenh/ts-proto?utm_source=chatgpt.com)

于是最终可以有：

```protobuf
message InkPoint {
  float x = 1;
  float y = 2;
  uint32 time_delta = 3;

  optional float pressure = 4;
  optional float tilt_x = 5;
  optional float tilt_y = 6;
}
```

这会比长期维护：

```text
random JSON shapes
```

舒服。

不过 POC 阶段：

```json
{
  "strokeId": "...",
  "points": [...]
}
```

完全足够。

暂时不用优化早了。

---

# 24. 因此我会给 Mathscape 选择下面这一套开源栈

| 层 | 推荐 |
|---|---|
| Desktop / Android shell | **Tauri 2** |
| Shared UI | 现有 TS UI |
| Generic pen API | **PointerEvent** |
| TS stroke renderer | **perfect-freehand** |
| Canvas | 原生 Canvas 2D 起步 |
| Android high-end ink | **AndroidX Ink** |
| Android native bridge | Tauri Kotlin plugin |
| Desktop LAN server | **Axum + Tokio** |
| Realtime transport | WebSocket |
| Discovery | QR first + `mdns-sd` optional |
| Wire schema | JSON → later Protobuf |
| Rust/TS Proto | `prost` + `ts-proto` |
| Collaboration later | Yjs/Yrs 或 Automerge |
| Architecture reference | **Rnote** |
| Whiteboard UX reference | **Excalidraw** |
| Tablet compatibility reference | **OpenTabletDriver** |
| Math handwriting dataset | **MathWriting / InkML** |

---

# 25. 我会避免这几种路线

### 不建议 1：直接把 Ink 做成图片附件

```text
handwriting → PNG
```

信息损失太大。

---

### 不建议 2：Android 完全单独开发一套产品

否则会变成：

```text
Mathscape Desktop
        +

Mathscape Android

两套 UI
两套 Canvas
两套 Document Model
两套 synchronization
```

成本会迅速上升。

---

### 不建议 3：一开始围绕某个 Wacom SDK 构建

会绑死设备。

---

### 不建议 4：一开始把 HMER / OCR 当核心

识别不是这个功能最有价值的地方。

---

### 不建议 5：直接采用一个完整 whiteboard SDK 当 domain model

Mathscape Ink 的需求最终会比普通 whiteboard 深得多。

---

# 26. 如果让我来安排第一个技术验证，我会只做这一个纵向切片

我建议你们下一步实际上可以先实现一个非常小的 prototype：

```text
Mathscape Desktop
        │
        │ Start Ink Session
        ▼
Axum local server
        │
        │ QR code
        ▼
Samsung Tab S8+
Chrome
        │
        ▼
PointerEvent
        │
        ▼
perfect-freehand
        │
        ▼
WebSocket
        │
        ▼
Mathscape Desktop Canvas
```

同时记录：

```text
x
y
timestamp
pressure
tiltX
tiltY
pointerType
buttons
```

然后重点测试你的 **Samsung Tab S8+ + S Pen**：

1. `pointerType === "pen"` 是否稳定；
2. pressure 是否是真实连续值；
3. tilt 是否有数据；
4. hover 是否能得到；
5. S Pen button 行为；
6. 手掌接触时 pointer stream 怎样；
7. `pointercancel` 什么时候触发；
8. 快速写数学公式是否掉点；
9. Chrome LAN 页与 Tauri Android WebView 的延迟差异；
10. 一页持续写十几分钟后的内存和渲染表现。

如果这一关通过，

那么实际上：

> **Mathscape Handwriting v0 的 70% 技术风险就已经被消除了。**

随后再用完全相同的 Ink model：

```text
Desktop Tauri Canvas
        ↓

Android Tauri Canvas
        ↓

AndroidX Ink Adapter
```

逐层增强。

---

## 最后给一个更高层的判断

你最开始描述这个功能时用了：

> “这些内容可以很大程度折射出其解决思路，是 LLM 了解人类思路的极佳输入。”

我认为这句话实际上已经决定了这块功能应该怎么设计。

普通笔记软件的模型是：

```text
Human
  ↓
Ink
  ↓
Document
```

而 Mathscape 更应该是：

```text
               Source material
                     │
                     ▼
Human ──writes──> InkSession
                     │
           ┌─────────┴─────────┐
           ▼                   ▼
     Human-readable       Machine-readable
        notebook          reasoning evidence
                               │
                               ▼
                              LLM
                               │
                               ▼
                        Mathscape semantics
```

也就是说，**手稿本身应该成为 Mathscape knowledge pipeline 的 source material**，地位类似：

- 教材段落；
- theorem；
- proof；
- exercise；
- research paper annotation；

而不是 UI 里的一个“附件”。

这会让这个功能从“Mathscape 支持手写”变成更有价值的一件事：

> **Mathscape 开始能够观察、保存和理解学习者自己产生数学思想的过程。**

这才是我认为值得围绕它设计架构的核心。
