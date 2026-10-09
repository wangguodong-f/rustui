# RustUI 概要设计文档

**项目名称：** RustUI
**文档类型：** 概要设计
**版本：** 0.1.0
**状态：** Draft
**语言：** Rust
**目标平台：** Windows / Linux / macOS
**核心依赖：** winit / wgpu

> **实现状态**：本文档为**初版设计**。实现过程中确认的偏离、补充与取舍见
> [`Design_Corrections.md`](Design_Corrections.md)，**两者冲突时以修正记录为准**。

---

# 1. 项目概述

## 1.1 项目背景

Rust 生态已经具备较成熟的 GUI 基础组件，例如：

* winit：跨平台窗口和输入事件
* wgpu：跨平台 GPU 图形 API
* egui：Immediate Mode GUI
* Slint：声明式 GUI
* Iced：Rust GUI Framework

但是，如果希望构建一套类似 Qt 的：

* Retained Mode UI
* Object Tree
* Widget Tree
* Property
* Signal / Slot
* Event System
* Layout System
* Model / View
* Rendering System
* Animation
* Window System

目前仍然需要较多第三方框架或自行组合。

RustUI 的目标是构建一套：

> **Rust 原生、跨平台、GPU 加速、Retained Mode、模块化、可扩展的 GUI Framework。**

底层仅依赖成熟的跨平台基础设施：

```text
RustUI
 ├── winit
 └── wgpu
```

上层逐步构建类似 Qt 的 GUI Framework 能力。

---

# 2. 设计目标

## 2.1 总体目标

构建一套 Rust 原生 GUI Framework：

```text
Application
    │
    ▼
Object / Widget
    │
    ├── Property
    ├── Signal
    ├── Event
    ├── Layout
    └── Model
    │
    ▼
Render Tree
    │
    ▼
wgpu
    │
    ▼
GPU
```

---

## 2.2 核心目标

### G-001 跨平台

支持：

* Windows
* Linux
* macOS

平台相关代码尽量限制在：

```text
rustui-platform
```

内部。

---

### G-002 Rust 原生

核心框架使用 Rust 实现。

不依赖：

* C++
* Qt
* Electron
* WebView
* Node.js

---

### G-003 GPU 加速

底层使用：

```text
wgpu
```

实现：

* 2D UI
* Text
* Image
* Shape
* Transform
* Clip
* Animation

---

### G-004 Retained Mode

UI 对象长期存在。

例如：

```text
Window
 └── VBox
      ├── Label
      ├── TextInput
      └── Button
```

而不是每一帧重新创建 UI 对象。

---

### G-005 模块化

框架拆分为多个 Cargo Crate：

```text
rustui-core
rustui-platform
rustui-renderer
rustui-layout
rustui-text
rustui-model
rustui-animation
rustui-widgets
rustui-window
rustui
```

---

### G-006 可扩展

第三方可以实现：

```rust
trait Widget
```

创建自定义 Widget。

---

# 3. 非目标

RustUI 0.1 不追求：

* 完全兼容 Qt API
* 完全兼容 QML
* 替代 Qt
* 实现完整 Web 浏览器
* 实现复杂 IDE 编辑器
* 实现完整 Accessibility
* 实现所有平台原生控件

这些能力作为后续版本规划。

---

# 4. 总体架构

```text
┌──────────────────────────────────────────┐
│                Application               │
├──────────────────────────────────────────┤
│                  Widgets                 │
│ Button / Label / TextInput / Table / ...│
├──────────────────────────────────────────┤
│                UI Core                   │
│ Object / Property / Signal / Event       │
├──────────────────────────────────────────┤
│          Layout / Model / Animation      │
├──────────────────────────────────────────┤
│                Render Tree               │
├──────────────────────────────────────────┤
│             Platform Layer               │
│                  winit                   │
├──────────────────────────────────────────┤
│              Rendering Layer             │
│                  wgpu                    │
├──────────────────────────────────────────┤
│          Windows / Linux / macOS         │
└──────────────────────────────────────────┘
```

---

# 5. 核心模块

## 5.1 rustui-core

负责框架基础抽象：

```text
Object
ObjectId
Property
Signal
Event
Geometry
Color
Style
```

---

## 5.2 rustui-platform

负责平台相关功能：

```text
Window
Event Loop
Keyboard
Mouse
IME
Monitor
DPI
Clipboard
Cursor
```

底层：

```text
winit
```

---

## 5.3 rustui-renderer

负责 GPU 渲染：

```text
PaintCommand
RenderNode
Texture
FontAtlas
VertexBuffer
IndexBuffer
RenderPipeline
```

底层：

```text
wgpu
```

---

## 5.4 rustui-layout

负责：

```text
HBox
VBox
Grid
Stack
Flow
Absolute
```

以及：

```text
Measure
Arrange
```

---

## 5.5 rustui-text

负责：

```text
Font
Text Layout
Glyph
Shaping
Font Atlas
```

**实现修正（C-026 / C-053）**：`fontdb`（字体检索）+ `ab_glyph`（度量 / 光栅化）+
**`rustybuzz` 整形 + `unicode-bidi` 双向视觉序**（T-9，2026-10-09 关闭）——连字、kerning、
阿拉伯文/希伯来文双向与天城文等复杂文种均正确排布，断行只落在字簇边界（连字不拆开）。
对 core 暴露 `TextMeasurer` 与 `GlyphSource` 两个 trait，widgets 与 renderer 不认识 `ab_glyph`。

---

## 5.6 rustui-model

负责 Model/View：

```text
ListModel
TableModel
TreeModel
SelectionModel
```

---

## 5.7 rustui-animation

负责：

```text
Animation
Transition
Easing
Frame Clock
```

---

## 5.8 rustui-widgets

提供基础控件：

```text
Window
Panel
Label
Button
CheckBox
RadioButton
TextInput
ComboBox
ListView
TableView
TreeView
ScrollView
ProgressBar
Menu
Dialog
```

---

# 6. Object Model

RustUI 采用：

```text
ObjectId + ObjectRegistry
```

而不是 C++ QObject 指针体系。

```text
Object
 │
 ├── ObjectId
 ├── Parent
 ├── Children
 ├── Properties
 ├── Signals
 └── Event Handler
```

示意：

```text
Window #1
 └── Panel #2
      ├── Label #3
      ├── TextInput #4
      └── Button #5
```

---

# 7. Widget Tree

Widget Tree 保存 UI 层次结构：

```text
Window
 └── VBox
      ├── Label
      ├── TextInput
      └── Button
```

Widget Tree 不直接依赖 wgpu。

---

# 8. Event System

事件流程：

```text
winit
   ↓
EventTranslator   （platform 内完成 DPI / 修饰键 / IME 状态跟踪）
   ↓
UiEvent  →  EventQueue  →  HitTest / Focus
   ↓
Widget::event(&mut EventContext, &UiEvent) -> EventResult
```

支持：

```text
Mouse
Keyboard
Touch
Focus
Scroll
TextInput
Window
```

**实现修正**：

* 结果只有两态 `EventResult::{Unhandled, Handled}`，或在回调里 `ctx.accept()`
  （C-008；`EventContext` 不携带 clock / modifiers，见 C-009）；
* `Resize` / `ScaleFactorChanged` / `CloseRequested` 由**窗口自己**处理，返回
  `EventOutcome { handled, close_requested, request_redraw }`（C-012）；
* **Tab / Shift+Tab 由窗口拦截**做焦点遍历，同时 `TextInput("\t")` 被吞掉，
  不会进入文本缓冲（C-011）。

---

# 9. Layout System

采用两阶段布局：

```text
Measure
   ↓
Arrange
```

例如：

```text
VBox
 ├── Label
 ├── TextInput
 └── Button
```

先计算尺寸，再确定位置。

**实现修正（C-017 / C-018 / C-015 / C-014）**——同一行里既有"填充型"子项又有"贴内容"子项时，
测量顺序决定成败：

1. **刚性子项（无 flex、该轴 hint 有限）先测**，各自看到剩余空间；
2. **柔性子项（`set_flex_of` 覆盖，或该轴 hint 为 ∞）后测**，按权重瓜分剩余的池，互不饿死；
3. `arrange` 中柔性子项的主轴尺寸 = `pool * share`（`pool = 可用 - 间隙 - Σ刚性`），
   测量值只决定交叉轴；
4. 因此 `measure` 会吃满 available 的 widget **必须**在 `size_hint` 里声明
   （`TextInput` 报 `width: ∞`，`ScrollView/ListView/TableView/TreeView` 报 `SizeHint::fill()`），
   否则它会被当成刚性并把兄弟测成 0 宽；
5. 交叉轴默认 `Start`（不是 Stretch），`MainAxisSize` 只有 `Auto/Fill/Fixed`。

---

# 10. Rendering Architecture

Widget 不直接操作 wgpu。

采用：

```text
Widget
 ↓
Painter
 ↓
PaintCommand
 ↓
RenderTree
 ↓
Renderer
 ↓
wgpu
```

---

# 11. Paint Command

示例（**实现修正 C-023 / C-022 / C-025**：`Rect`/`RoundedRect` 合并为 `Shape`，
裁剪拆成 `PushClip`/`PopClip`，文本引用帧级 `TextArena` 而不是 `String`）：

```rust
pub enum PaintCommand {
    PushClip { rect: Rect, radius: f32 },
    PopClip,
    Shape { rect: Rect, fill: Fill, transform: Transform },
    Line { from: Point, to: Point, width: f32, color: Color },
    Text { position: Point, run: TextRunId, range: Range<u32>,
           style: TextStyle, max_width: Option<f32> },
    Image { rect: Rect, texture: TextureId },
}
```

Renderer 将这些命令转换为 GPU 数据。

**坐标约定（C-020）**：`Painter` 吃**窗口坐标**（内部零偏移），
`PaintContext` 的帮助方法吃**局部坐标**（内部 `to_window_rect`）；
`WidgetBase::bounds()` 是局部、`window_bounds()` 是窗口。
窗口背景由框架用 `theme.colors.window_background` 首先填满（C-024），
容器边框必须是"透明底 + border"的**环**，不能是实心矩形（C-021）。

---

# 12. Model/View

数据层与 UI 分离：

```text
Model
  ↓
View
```

例如：

```text
PackageModel
     ↓
TableView
```

适合：

* 大型列表
* 表格
* Tree
* 虚拟化显示

---

# 13. Property System

提供（**实现修正**：`Property<T>` 与 `SharedProperty<T>`；后者在 `Rc` 里，`get` 返回值副本，
`set` 会触发 `changed` 并**自动请求重绘**）：

```text
get        → T（副本）
set        → 触发 changed
changed    → &Signal<T>
clone      → SharedProperty 是廉价句柄，多个 widget 共享同一份
```

示例：

```rust
let enabled = SharedProperty::new(true);
enabled.set(false);              // 观察者收到 changed(true→false 的新值)
let text = button.text_property();
```

视图与模型的绑定 = 把一边的 `changed` 连到另一边的 `set`（**没有独立的 `bind` API**）：

```rust
model.can_install.changed().connect(move |v| can_install.set(v));
```

注意：`SharedProperty` **不是 `Send`**，不能从 `spawn` 闭包直接改（见 §16）。

---

# 14. Signal System

提供类型安全事件机制（**实现修正**：`connect` 返回 `Connection<T>` 句柄，句柄 drop 即断开；
短生命周期回调用 `connect_scoped`）：

```rust
button.clicked().connect(move |_| {
    println!("clicked");
});
```

支持：

```text
Signal<T>
Connection<T>       // 句柄，drop 即断开
connect / connect_scoped
connection_count
Weak reference       // 回调捕获弱引用避免循环
```

避免循环引用。

---

# 15. Rendering Pipeline

总体流程：

```text
Widget Tree
    ↓
Layout
    ↓
Render Tree
    ↓
Paint Commands
    ↓
Batch
    ↓
GPU Buffer
    ↓
Render Pass
    ↓
wgpu
```

---

# 16. 线程模型

默认：

```text
Main Thread
 │
 ├── winit Event Loop
 ├── UI
 ├── Layout
 └── Rendering
```

后台：

```text
Worker Threads
 │
 ├── IO
 ├── Network
 ├── File
 ├── Process
 └── Application Tasks
```

UI 状态修改通过消息机制回到 UI Thread。

---

# 17. Cargo Workspace

```text
rustui/
│
├── Cargo.toml
│
├── crates/
│   ├── rustui/
│   ├── rustui-core/
│   ├── rustui-platform/
│   ├── rustui-renderer/
│   ├── rustui-layout/
│   ├── rustui-text/
│   ├── rustui-model/
│   ├── rustui-animation/
│   ├── rustui-widgets/
│   └── rustui-window/
│
└── examples/
    ├── hello/
    ├── calculator/
    └── table_view/
```

**实现修正**：第三个示例从 `vcpkg-manager` 换成 `table_view`（工具栏 + 表头 + 选中行 +
滚动条 + 树缩进，专门覆盖虚拟化与自绘控件）；三个示例都已截图验证。

---

# 18. 技术路线

## Phase 1

实现：

```text
winit
wgpu
Application
Window
Object Tree
Widget Tree
Event
Layout
Painter
Renderer
```

基础 Widget：

```text
Label
Button
Panel
TextInput
```

---

## Phase 2

实现：

```text
Property
Signal
Model/View
TableView
TreeView
ScrollView
Dialog
Menu
```

---

## Phase 3

实现：

```text
Animation
Text System
IME
Clipboard
Drag & Drop
Accessibility
```

---

## Phase 4

实现：

```text
Declarative UI
DSL
Hot Reload
Plugin
Theme
Advanced GPU Effects
```

---

# 19. MVP 验收标准

**验证状态**：以下清单已在 Linux（Wayland）上逐项通过——创建窗口、Label / Button /
TextInput、鼠标点击、键盘输入、窗口 Resize、布局、GPU 渲染；
ComboBox 下拉面板（弹层通道，C-045）已截图确认压在表格之上；
`cargo test --workspace` 454 通过、clippy 0 warning、fmt 干净。
视觉回归已自动化（T-10 / C-046）：`rustui-renderer` 的离屏渲染（`new_offscreen` /
`read_pixels`）把真实场景渲到纹理并读回像素，`crates/rustui/tests/golden.rs` 与提交的
`tests/golden/*.grid` 逐格比对（`UPDATE_GOLDEN=1` 重建），覆盖"控件行"、"真实点击打开的
下拉弹层"与"旋转文本"三个场景；无 GPU 的机器自动跳过。
文本可旋转（T-8 / C-049）：`Label::rotation(degrees)` 让 `PaintCommand::Text` 携带变换，
渲染器绕墨迹框中心转字形，标签在 measure 阶段预留旋转外接框，已出图人工复核无裁剪。
圆角裁剪与四角半径已精确（T-5/T-6 / C-051/C-052）：矩形裁剪仍走 scissor，最内层圆角逐片元
SDF 裁出（角落不再漏画）；`EdgeRadius` 四角各自携带半径，"上圆下方"不再被摊成最大半径。
离屏像素测试钉住这两个行为（角落必须是背景、方角必须是填充色），三张黄金图逐像素未变
（T-9 换排版后已重建，见下）。
复杂文种 shaping 已接入（T-9 / C-053）：`rustybuzz` 整形（连字 / kerning / 语境合字与
重排）+ `unicode-bidi` 视觉序换行进入 `rustui-text`，`ab_glyph` 保留为度量与光栅化后端
（同一张 cmap，glyph id 一致性有测试钉住）；`LaidOutGlyph` 携带字符区间，光标插值与
hit-test 按字簇映射。10 条集成测试覆盖拉丁 / 天城文 / 阿拉伯 / 希伯来与混排，三张黄金图
按新排版重建并逐张比对（仅文字像素微移、结构不变）。
系统剪贴板已接入（T-3 / C-054）：`PlatformClipboard` 用 `arboard` 接系统剪贴板
（X11/XWayland，selection 服务线程响应外部客户端），打开或写入失败降级回进程内缓冲；
跨进程子客户端粘贴有测试覆盖，`arboard` 只允许出现在 `rustui-platform`。
多窗口 DPI 已有回归测试（T-2 / C-048）：`crates/rustui/tests/dpi.rs` 让 1.0 / 2.0 两个
display scale 的窗口轮流渲染，字形缓存按物理尺寸键控后两者光栅并存、切换零成本。
CPU 热路径已有可复跑基线（T-11 / C-047）：`cargo bench -p rustui`
（`crates/rustui/benches/pipeline.rs`，criterion，text / layout / paint / hit_test /
frame / table(10k,100k) / widget 七组），数字记录在 `Detailed` §59。
Windows / macOS 未在本环境验证。三个示例（`hello` / `calculator` / `table_view`）
已实际运行并截图确认与上图一致。

RustUI MVP 至少能够运行：

```text
Windows
Linux
macOS
```

并完成：

```text
创建窗口
显示 Label
显示 Button
显示 TextInput
鼠标点击
键盘输入
窗口 Resize
布局
GPU 渲染
```

示例：

```text
┌─────────────────────────────────────┐
│ RustUI Demo                     □ X │
├─────────────────────────────────────┤
│                                     │
│  Package                            │
│  ┌─────────────────────────────┐    │
│  │ fmt                         │    │
│  └─────────────────────────────┘    │
│                                     │
│  [ Install ]   [ Remove ]           │
│                                     │
└─────────────────────────────────────┘
```

---

# 20. 设计原则

### P1

Widget 不依赖平台。

### P2

Widget 不直接依赖 wgpu。

### P3

Platform 不依赖 Widget。

### P4

Renderer 不理解业务逻辑。

### P5

Model 与 View 分离。

### P6

UI 状态与 GPU Resource 生命周期分离。

### P7

优先使用 Rust 类型系统保证生命周期安全。

### P8

避免复制 Qt 的 C++ 对象模型。

### P9

优先构建稳定核心，再增加高级功能。

---

# 21. 与 Qt 的对应关系

| Qt                     | RustUI                 |
| ---------------------- | ---------------------- |
| QApplication           | Application            |
| QObject                | Object                 |
| QObject tree           | Object Tree            |
| QWidget                | Widget                 |
| QWindow                | Window                 |
| Signal/Slot            | Signal/Connection      |
| Q_PROPERTY             | Property               |
| QEvent                 | UiEvent                |
| QLayout                | Layout                 |
| QPainter               | Painter                |
| QGraphicsScene         | Render Tree            |
| QAbstractItemModel     | Model                  |
| QListView              | ListView               |
| QTreeView              | TreeView               |
| QTableView             | TableView              |
| Qt Quick Scene Graph   | RustUI Render Tree     |
| QOpenGL/Vulkan backend | wgpu                   |
| QPA                    | Platform Layer / winit |

---

# 22. 总体定位

RustUI 不以：

> “重新实现 Qt”

作为第一阶段目标。

而是：

> **构建 Rust 原生的、类似 Qt 架构思想的跨平台 GUI Framework。**

核心技术路线：

```text
Rust
 │
 ├── Object Model
 ├── Event System
 ├── Property
 ├── Signal
 ├── Layout
 ├── Model/View
 ├── Widget
 ├── Render Tree
 └── Animation
       │
       ▼
     winit
       +
      wgpu
```

最终形成：

```text
RustUI
= Rust UI Framework
+ Retained Mode
+ GPU Rendering
+ Cross Platform
+ Type Safety
+ Modular Architecture
```

