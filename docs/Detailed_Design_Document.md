# RustUI 详细设计文档

**项目名称：** RustUI
**文档类型：** 详细设计
**版本：** 0.1.0
**状态：** Draft
**对应概要设计：** RustUI 概要设计 0.1

> **实现状态**：本文档为**初版设计**。实现过程中确认的偏离、补充与取舍见
> [`Design_Corrections.md`](Design_Corrections.md)，**两者冲突时以修正记录为准**。
> 阅读本文时请特别注意 C-020（坐标双空间约定）与 C-017（box 刚性/柔性测量顺序）。

---

# 1. 文档目的

本文定义 RustUI Framework 的：

* 模块结构
* 核心数据结构
* Trait
* API
* Object 生命周期
* Event 分发
* Layout
* Rendering
* Property
* Signal
* Model/View
* Window
* GPU Resource
* 线程模型

本文作为 RustUI MVP 的实现依据。

---

# 2. Workspace 设计

```text
rustui/
│
├── Cargo.toml
│
├── crates/
│
│   ├── rustui/
│   │
│   ├── rustui-core/
│   │
│   ├── rustui-platform/
│   │
│   ├── rustui-renderer/
│   │
│   ├── rustui-layout/
│   │
│   ├── rustui-text/
│   │
│   ├── rustui-model/
│   │
│   ├── rustui-animation/
│   │
│   ├── rustui-widgets/
│   │
│   └── rustui-window/
│
└── examples/
```

依赖关系（**实现修正**：与真实 `Cargo.toml` 一致——`rustui-window` **不**依赖
`rustui-platform`，`rustui-widgets` **不**依赖平台 / wgpu，winit 与 wgpu 只出现在
`rustui-platform` / `rustui-renderer` / facade 三处）：

```text
rustui（facade，接线层）
 │ ├── rustui-core
 │ ├── rustui-platform   (+ winit, arboard)
 │ ├── rustui-renderer   (+ wgpu, bytemuck, pollster)
 │ ├── rustui-layout
 │ ├── rustui-text       (+ ab_glyph, fontdb, rustybuzz, unicode-bidi)
 │ ├── rustui-model
 │ ├── rustui-animation
 │ ├── rustui-widgets
 │ └── rustui-window     (+ rustui-renderer, 无 rustui-platform)
 │
rustui-widgets ── core / layout / text / model / animation
rustui-layout / rustui-model / rustui-animation ── core
rustui-renderer ── core（GlyphSource 是 core 的 trait，不认识 rustui-text）
```

---

# 3. Core 数据类型

## 3.1 ObjectId

```rust
#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Hash
)]
pub struct ObjectId {
    index: u32,
    generation: u32,
}
```

**实现修正（C-001）**：id 是**代际句柄**而非裸 `u64`。槽位回收后 `index` 复用而 `generation`
递增，旧句柄自然失效；`ObjectId::INVALID_INDEX`（`u32::MAX`）表示"空对象"。
判断存活用 `WidgetStore::contains(id)` / `ObjectRegistry::contains(id)`，不要缓存"我以为还活着"。

ObjectId 用于：

* Object Tree
* Event
* Focus
* Selection
* Property
* Signal

---

# 4. ObjectRegistry

```rust
pub struct ObjectRegistry {
    slots: Vec<Slot>,
    free: Vec<u32>,
    alive: usize,
    root: Vec<ObjectId>,
}
```

**实现修正（C-002）**：Registry 是**槽位数组**而不是 `HashMap<ObjectId, ObjectState>`，并且只保存
**身份 / 父子索引 / 子数量 / 根标记**；widget 本体（`Box<dyn Widget>`）存放在另一个容器
`WidgetStore`（内部 `RefCell` 池），通过 `store.take(id)` / `store.put(id, widget)` 短借。
"结构关系"与"行为体"分离，遍历结构时不碰 `RefCell`，避免可变借用冲突。

职责：

* 创建 Object
* 删除 Object
* 查找 Object
* Parent/Child 管理
* Object 生命周期管理

---

# 5. Object Tree

```text
Window #1
 │
 └── Panel #2
      │
      ├── Label #3
      │
      ├── TextInput #4
      │
      └── Button #5
```

基本操作：

```rust
pub trait Object {
    fn id(&self) -> ObjectId;

    fn parent(&self) -> Option<ObjectId>;

    fn children(&self) -> &[ObjectId];

    fn add_child(&mut self, child: ObjectId);

    fn remove_child(&mut self, child: ObjectId);
}
```

---

# 6. Widget Trait

实际签名（**实现修正**：`measure` / `paint` 都是 `&mut self`——测量要缓存结果，绘制要从
帧级 `TextArena` 里借 id；`measure` 另外接收 `MeasureContext`，见 §15；所有几何方法都由
`impl_widget_base!` 宏统一提供，宏只实现 `Object`，不实现 `Widget`）：

```rust
pub trait Widget: Object + fmt::Debug {
    fn id(&self) -> ObjectId { ... }
    fn kind(&self) -> &'static str { ... }
    fn children(&self) -> &[ObjectId] { ... }

    /// `true` when the widget can take keyboard focus.
    fn accepts_focus(&self) -> bool { false }
    /// `true` when the widget paints an opaque background (stops hit testing early).
    fn is_opaque(&self) -> bool { false }
    /// Preferred size before `measure` is called.
    fn size_hint(&self) -> SizeHint { SizeHint::default() }
    fn cursor(&self) -> CursorIcon { CursorIcon::Default }
    fn tooltip(&self) -> Option<&str> { None }

    /// Caret rectangle anchoring the input method candidate window, in **window** coordinates;
    /// `None` (the default) switches composition off (C-038).
    fn ime_cursor_area(&self, ui: &UiContext) -> Option<Rect> { None }

    /// Hit test in **window** coordinates.
    fn hit_test(&self, position: Point) -> bool { ... }

    /// Called once per frame before layout: timers, animations, background state.
    fn update(&mut self, ctx: &mut UpdateContext<'_>) {}

    /// Computes and caches the desired size (§15).
    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size;

    /// Assigns a rectangle to this widget and to its children.
    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect);

    /// Handles an event (§10).
    fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult;

    /// Emits paint commands for this widget and its children.
    fn paint(&mut self, ctx: &mut PaintContext<'_>);

    /// Paints the widget's **popup layer** (C-045): content that has to sit above the whole
    /// tree, like an open drop down panel. Ask for it with `ctx.request_popup(self.id())`
    /// while painting; the window runs this once more after the content tree and the overlay
    /// roots, with this widget's window origin as offset and the window as clip. Default: none.
    fn paint_popup(&mut self, _ctx: &mut PaintContext<'_>) {}

    fn background_texture(&self) -> Option<TextureId> { None }
    fn layout_changed(&mut self, old: Size, new: Size) {}
}
```

---

# 7. Geometry

核心类型：

```rust
pub struct Point {
    pub x: f32,
    pub y: f32,
}

pub struct Size {
    pub width: f32,
    pub height: f32,
}

pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
```

辅助：

```rust
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}
```

---

# 8. Event System

## 8.1 UiEvent

**实现修正（C-010）**：实际变体比设计稿多出指针 / IME / 窗口 / 拖放 / 任务几组，
且 `FocusIn` 带原因、`KeyDown` 带 `text` 与 `repeat`、`MouseWheel` 带滚动相位：

```rust
pub enum UiEvent {
    // ---- pointer ----
    MouseMove { position: Point },
    MouseEnter,
    MouseLeave,
    MouseDown { button: MouseButton, position: Point },
    MouseUp { button: MouseButton, position: Point },
    MouseDoubleClick { button: MouseButton, position: Point },
    MouseCapture,            // 指针被本 widget 捕获，直到 MouseRelease
    MouseRelease,
    MouseWheel { delta: Vec2, phase: ScrollPhase },
    Touch { position: Point, phase: ScrollPhase, id: u64 },

    // ---- keyboard ----
    KeyDown { key: Key, modifiers: Modifiers, text: Option<String>, repeat: bool },
    KeyUp { key: Key, modifiers: Modifiers },
    TextInput(String),

    // ---- IME ----
    ImeStart,
    ImeUpdate { text: String, cursor: Option<Range<usize>> },
    ImeCommit(String),
    ImeEnd,

    // ---- focus ----
    FocusIn(FocusReason),
    FocusOut,

    // ---- window ----
    Resize(Size),
    ScaleFactorChanged(f64),
    CloseRequested,
    WindowFocus(bool),
    FrameTick { frame: u64, delta: Duration, elapsed: Duration },

    // ---- data ----
    DragEnter { position: Point, payload: String },
    DragLeave,
    Drop { position: Point, payload: String },
    TaskCompleted { task: TaskId, result: TaskPayload },
    Custom { name: &'static str, payload: TaskPayload },
}

impl UiEvent {
    /// 携带指针位置的事件（参与命中测试）。
    pub fn position(&self) -> Option<Point>;
    pub fn is_pointer(&self) -> bool;
    /// 日志 / 调试用的稳定短名。
    pub fn name(&self) -> &'static str;
}
```

---

# 9. Platform Event

winit 事件不得直接进入 Widget。

设计（**实现修正**：翻译是**有状态**的 `EventTranslator`，因为 DPI / 修饰键 / IME 状态
需要在事件之间保留；一次 winit 事件可能产生**多条** `UiEvent`，所以返回 `Vec`）：

```text
winit::WindowEvent
        ↓
EventTranslator（跟踪 modifiers、scale factor、IME 状态）
        ↓
Vec<UiEvent>
```

```rust
/// 无状态便捷入口。
pub fn translate_event(event: &WindowEvent) -> Vec<UiEvent>;

/// 有状态翻译器（光标位置、DPI、IME 组合态）。
pub struct EventTranslator { … }
impl EventTranslator {
    pub fn translate(&mut self, event: &WindowEvent) -> Vec<UiEvent>;
}
```

这样可以保证：

```text
Widgets
   ↓
RustUI Event
   ↓
Platform
```

而不是：

```text
Widget
   ↓
winit
```

---

# 10. Event Dispatch

流程：

```text
winit
  ↓
UiEvent
  ↓
Window
  ↓
HitTest
  ↓
Target Widget
  ↓
event()
```

事件结果（**实现修正 C-008**：变体命名为 `Unhandled` / `Handled`，不再预留 `StopPropagation`——
"是否继续冒泡"与"是否已处理"是同一个决定）：

```rust
pub enum EventResult {
    /// widget 不感兴趣；事件继续向根冒泡。
    Unhandled,
    /// widget 消费了事件；传播到此为止。
    Handled,
}

impl EventResult {
    pub fn is_handled(&self) -> bool;
}
```

widget 也可以不看返回值，直接在回调里调用 `EventContext::accept()` 标记消费
（内部 `consumed` 标志位，冒泡在该点停止）。

事件在 `EventQueue` 中排队后按"目标 → 冒泡"投递；**实现修正（C-009）**：`EventContext`
只携带 `ui / store / interaction / focus / queue / overlays / depth / consumed`，
**没有 `clock`、没有 `modifiers`**——帧时钟走 `UpdateContext` / `FrameClock`，
修饰键在 `UiEvent` 自身的 `Modifiers` 字段里。

---

# 11. Hit Testing

**实现修正（C-029）**：分成两层——widget 自己回答"这个点在我身上吗"（窗口坐标，返回布尔），
最深目标由窗口层的深度优先遍历挑出：

```rust
// Widget（默认实现：window_bounds 包含该点）
fn hit_test(&self, position: Point) -> bool;

// rustui-window::Window
pub fn hit_test(&self, store: &WidgetStore, position: Point) -> Option<ObjectId>;
// 等价地：WindowManager::hit_test(window, position)
```

容器（`ScrollView` / `ComboBox` 等）覆盖它，把**自己的滚动区域 / 打开的弹层**也算进去；
不透明（`is_opaque()`）的 widget 命中后停止向上遍历。

Tree：

```text
Window
 └── Panel
      ├── Button A
      └── Button B
```

鼠标：

```text
(120, 80)
```

执行：

```text
Window
 ↓
Panel
 ↓
Button B
```

确定最终 Target。

---

# 12. Focus Manager

**实现修正（C-011）**：焦点改动是**延迟生效**的（`pending` 队列，避免在事件回调里
就地切换焦点导致重入），且带 `FocusReason`：

```rust
pub struct FocusManager {
    focused: Option<ObjectId>,
    reason: Cell<FocusReason>,
    pending: RefCell<Vec<(ObjectId, Option<FocusReason>)>>,
}

impl FocusManager {
    pub fn focused(&self) -> Option<ObjectId>;
    pub fn reason(&self) -> FocusReason;
    pub fn set_focus(&self, id: ObjectId, reason: FocusReason);
    pub fn clear_focus(&self);
    /// 取出并清空本轮待处理的焦点变更（在事件派发后统一 flush）。
    pub fn take_pending(&self) -> Vec<(ObjectId, Option<FocusReason>)>;
}
```

`FocusReason`：`Pointer` / `Tab` / `Programmatic` / `Activation` / `Lost`。

Tab 遍历在**窗口层**：`WindowManager::focus_next(window)` / `focus_prev(window)`
按绘制顺序挑下一个 `accepts_focus()` 的 widget，`Shift+Tab` 走 prev（C-011）。

支持：

```text
Mouse Focus
Keyboard Focus
Tab Navigation
Programmatic Focus
```

---

# 13. Property System

定义（**实现修正**：两种——`Property<T>` 是独占的，`SharedProperty<T>` 是 `Rc` 共享句柄，
widget 字段一律用后者）：

```rust
pub struct Property<T> {
    value: T,
    changed: Signal<T>,
}

/// 廉价句柄，多个 widget / 模型共享同一份值。
pub struct SharedProperty<T> {
    inner: Rc<PropertyInner<T>>,
}
```

接口：

```rust
impl<T> Property<T> {
    pub fn get(&self) -> &T;
    pub fn set(&mut self, value: T);
    pub fn changed(&self) -> &Signal<T>;
}

impl<T: Clone> SharedProperty<T> {
    pub fn new(value: T) -> Self;
    pub fn get(&self) -> T;      // 副本
    pub fn set(&self, value: T); // &self，触发 changed + 自动请求重绘
    pub fn changed(&self) -> &Signal<T>;
    pub fn boxed() -> SharedProperty<Box<T>>; // SharedModel 走箱化（§27）
}
```

例如：

```rust
pub struct Button {
    base: WidgetBase,
    text: SharedProperty<String>,
}
let text = button.text_property(); // 取句柄做外部绑定
```

**没有独立的 `bind` API**——绑定 = 把一侧的 `changed` 连到另一侧的 `set`。
`SharedProperty` **不是 `Send`**（**C-042**），不能在 `spawn` 闭包里直接写。

---

# 14. Signal

基本设计：

```rust
pub struct Signal<T> {
    connections: Vec<Connection<T>>,
}
```

Connection：

```rust
pub struct Connection<T> {
    id: ConnectionId,
    callback: Box<dyn Fn(&T)>,
}
```

API（**实现修正**：`connect` 用 `&self` 并**返回 `Connection<T>` 句柄**，句柄 drop 即断开；
回调签名是 `Fn(&T)`）：

```rust
impl<T> Signal<T> {
    pub fn connect<F: Fn(&T) + 'static>(&self, callback: F) -> Connection<T>;
    /// 仅在 `guard` 存活期间有效的连接。
    pub fn connect_scoped<F, G: 'static>(&self, guard: G, callback: F) -> Connection<T>;
    pub fn connection_count(&self) -> usize;
    pub fn disconnect_all(&self);
    pub fn emit(&self, value: &T);
}
```

例如：

```rust
let connection = button.clicked().connect(|_| println!("clicked"));
connection.disconnect();
```

`emit` 是**可重入安全**的：槽位列表在回调执行期间被移出，因此回调里可以再
`connect` / 断开 / 重新 `emit`；信号内部只用 `Rc` + `RefCell`，`Connection` 持**弱引用**，
不会延长信号寿命（避免循环引用，见 §52）。

---

# 15. Layout

统一接口（**实现修正**：布局不是一个独立 `Layout` trait，而是**每个布局 widget 自己实现
`Widget::measure` / `Widget::arrange`**，把公共算法（测量顺序、自由空间分配、交叉轴对齐）
下沉到 `BoxLayout` / `Grid` / `Stack` / `Flow` 这些结构体的方法上）：

```rust
pub trait Widget {
    fn measure(
        &mut self,
        ctx: &MeasureContext<'_>,
        available: Size,
    ) -> Size;

    fn arrange(
        &mut self,
        ctx: &ArrangeContext<'_>,
        rect: Rect, // 父空间（局部坐标）
    );
}
```

`MeasureContext` 提供 `measure_child(id, available)` 与文本测量器；`ArrangeContext` 提供
`arrange_child(id, rect)` 与上一轮的测量结果（arrange 不复测）。

**测量与分配规则（实现修正 C-017 / C-018 / C-019）**：

1. **刚性子项先测**：无 flex 权重且该轴 `hint` 有限的子项按原顺序先测，各自看到剩余空间；
2. **柔性子项后测**：有 flex 权重（`set_flex_of` 覆盖也算）或该轴 `hint` 为 `∞` 的子项，
   按权重瓜分"留给柔性组的池"，互不饿死；
3. **arrange 时柔性子项的最终主轴尺寸 = `pool * share`**，其中
   `pool = content_main - 间隙 - Σ刚性测量值`（覆盖测量阶段的 main 值，测量值只决定交叉轴）；
4. `measure` 收尾统一走 `apply_fill(clamp_to(size, available), available, hint)`
   （**C-016**）：某轴 `hint` 为 `∞` 而 `available` 有限时取 `available`；
5. **`measure` 会吃满 `available` 的 widget 必须让 `size_hint` 与之匹配**（**C-018**），
   否则第 1 步会把它判成刚性并饿死兄弟：`TextInput` / `ProgressBar` 报 `width: ∞`，
   `ScrollView` / `ListView` / `TableView` / `TreeView` 报 `SizeHint::fill()`；
6. `CrossAxisAlign` 默认 `Start`（**C-015**），`MainAxisSize` 只有
   `Auto` / `Fill` / `Fixed`（**C-014**，无 `Expand`）。

---

# 16. VBox

```rust
pub struct VBox {
    spacing: f32,
    padding: Insets,
}
```

布局：

```text
┌────────────────────┐
│ Label              │
│                    │
│ TextInput          │
│                    │
│ Button             │
└────────────────────┘
```

计算：

```text
height =
    padding.top
  + child1.height
  + spacing
  + child2.height
  + spacing
  + child3.height
  + padding.bottom
```

---

# 17. HBox

```text
┌────────┬──────────┬────────┐
│ Label  │ Input    │ Button │
└────────┴──────────┴────────┘
```

---

# 18. Grid

```text
┌──────┬──────┬──────┐
│ A    │ B    │ C    │
├──────┼──────┼──────┤
│ D    │ E    │ F    │
└──────┴──────┴──────┘
```

---

# 19. Paint API

定义（**实现修正**：分**两层**——`Painter` 是帧级命令缓冲，`PaintContext` 是 widget 看到的门面）：

```rust
pub struct Painter {
    commands: Vec<PaintCommand>,
    arena: TextArena,
}
```

`Painter` 的接口（**坐标：一律窗口坐标，函数内部不做任何偏移**，见 C-020）：

```rust
impl Painter {
    pub fn rect(&mut self, rect: Rect, color: Color);

    pub fn rounded_rect(&mut self, rect: Rect, radius: f32, color: Color);

    /// 填充（可含渐变 / 边框 / 圆角）的统一入口。
    pub fn styled_rect(&mut self, rect: Rect, radius: EdgeRadius, fill: Fill);

    /// 环形边框：透明底色 + border widths，由渲染器 tessellate 成环。
    pub fn border(&mut self, rect: Rect, radius: EdgeRadius, border: &Border);

    pub fn line(&mut self, from: Point, to: Point, width: f32, color: Color);

    /// 文本：字符串 intern 在帧级 `TextArena`，命令只持 `TextRunId + range`（C-025）。
    pub fn text(&mut self, position: Point, text: &str, style: &TextStyle) -> TextRunId;
    pub fn text_wrapped(&mut self, position: Point, text: &str, style: &TextStyle, max_width: f32) -> TextRunId;
    /// 带旋转的文本入口（C-049）：`text` / `text_wrapped` 是它传 `Transform::IDENTITY` 的封装。
    pub fn text_transformed(&mut self, position: Point, text: &str, style: &TextStyle,
                            max_width: Option<f32>, transform: Transform) -> TextRunId;

    pub fn image(&mut self, rect: Rect, texture: TextureId);

    pub fn push_clip(&mut self, rect: Rect);
    pub fn push_clip_rounded(&mut self, rect: Rect, radius: f32);
    pub fn pop_clip(&mut self);

    pub fn fill_all(&mut self, color: Color); // 窗口背景
    pub fn reset(&mut self, viewport: Rect);
    pub fn commands(&self) -> &[PaintCommand];
    pub fn arena(&self) -> &TextArena;
}
```

`PaintContext`（widget 实际持有的引用）提供**局部坐标**帮助方法，内部统一
`to_window_rect` 转换：`fill_rect` / `fill_rounded` / `stroke_rect` / `push_clip` /
`push_clip_rounded` / `paint_child` / `focus_ring` 等；另有 `request_popup(id)` /
`paint_popup(id)` 负责**弹层通道**（C-045），`viewport()` 返回窗口可绘区
（`Interaction::viewport`，popups 用它决定开在哪一侧）。

**坐标双空间约定（C-020，最重要的一条）**：

| 位置 | 空间 |
| --- | --- |
| `WidgetBase::bounds()`、`arrange` 收到的 `rect` | **局部**（父空间原点） |
| `WidgetBase::window_bounds()` | **窗口** |
| `PaintContext` 帮助方法的参数 | **局部**（内部转换） |
| `ctx.painter()` 调用的参数 | **窗口**（无偏移） |

写 `paint` 的规则：先 `let bounds = ctx.to_window_rect(self.base.bounds());` 再用 `painter()`；
传给 `ctx.*` 的一律用 `self.base.bounds()`。把已转换的窗口 rect 再传回 `ctx.*` 会造成
**双重偏移**，与当前裁剪求交后变成空矩形，整块内容被裁掉。

**实现修正（C-021）**：`PaintContext::stroke_rect(rect, radius, color, width)` 产出
"透明底 + `Border { widths: all(width), color }`"的**环形**命令（走 `Painter::border`），
画在 `rect` **内侧**——不要用"实心 + 透明矩形擦除"的做法：`Painter::rect` 对 `alpha <= 0`
会早退，擦除不会生效，边框会变成实心块盖住先绘制的子内容。

---

# 20. PaintCommand

**实现修正（C-023 / C-022 / C-025）**：形状统一为 `Shape`（`Rect` + `RoundedRect` 合并成一条，
颜色 / 渐变 / 边框 / 圆角打包进 `Fill`），裁剪拆成 `PushClip` / `PopClip` 两步，文本不持 `String`：

```rust
pub enum PaintCommand {
    /// 把后续绘制限制在 rect（与当前裁剪求交）；圆角只记录在命令里，裁剪栈本身是矩形的。
    PushClip {
        rect: Rect,
        radius: f32,
    },
    /// 恢复上一层裁剪。
    PopClip,
    /// 填充（可含渐变、边框、圆角）+ 可选旋转。
    Shape {
        rect: Rect,
        fill: Fill,
        transform: Transform,
    },
    Line {
        from: Point,
        to: Point,
        width: f32,
        color: Color,
    },
    /// `range` 选中帧级 `TextArena` 条目的切片，避免每帧克隆 `String`。
    Text {
        position: Point,
        run: TextRunId,
        range: Range<u32>,
        style: TextStyle,
        max_width: Option<f32>,
        /// 绕墨迹框中心的顺时针旋转（C-049；`Shape` 的 transform 绕矩形中心）。
        transform: Transform,
    },
    Image {
        rect: Rect,
        texture: TextureId,
    },
}

/// 一次命令表达"底色 + 渐变 + 边框 + 圆角"。
pub struct Fill {
    pub color: Color,
    pub gradient: Option<Gradient>,
    pub border: Border,
    pub radius: EdgeRadius,
}
```

补充说明：

* `Painter::rect` / `styled_rect` 对**空矩形**与**不可见填充**早退，不产生空命令；
* SDF 圆角逐角取值：`Instance.radii` 按**象限顺序** TL / TR / BL / BR 携带四个半径（物理
  像素），片元在自己的象限里取该角半径——矩形内部的距离与半径无关，四块在中缝无缝拼接，
  四角各自精确；C-023 的 `max_side` 单半径近似已由 **C-052** 消除（见 §23）；
* 裁剪栈仍是**矩形**的（scissor = 全部矩形交集），`PushClip.radius` 由片元 SDF 执行**最内层**
  圆角裁剪——同时只有一个圆角裁剪活跃时精确，普通矩形压在其上同样精确；C-022 的近似段已由
  **C-051** 消除（见 §23），半径为 0 时整条路径与改动前逐位一致；
* 文本命令的 `TextRunId` 每帧随 `Painter::reset` 清空（**C-025**）；
* 旋转的文本（**C-049**）由 tessellate 绕**墨迹框中心**（该 run 字形矩形的并集中心）转
  每个字形 quad；恒等变换走原路径，顶点逐位不变。注意 `paint_child` 把子节点裁到自身
  bounds，所以 widget 必须在 **measure** 就预留旋转外接框（见 §47 Label）。

---

# 21. Render Tree

Widget Tree：

```text
Window
 └── VBox
      ├── Label
      ├── TextInput
      └── Button
```

**实现修正（C-023）**：没有独立的 `RenderTree` 结构——**扁平的 `Vec<PaintCommand>` 就是
渲染树**（paint 阶段按绘制顺序录制，裁剪用 `PushClip`/`PopClip` 表达成栈），字符串另外
放在帧级 `TextArena` 里：

```text
Painter::commands()  →  [Shape, Text, PushClip, Shape, PopClip, …]
Painter::arena()     →  文本条目池（TextCommand::Text 只持 id + range）
```

扁平化的好处：无中间节点分配、可直接按状态排序 / 合批、单遍即可生成实例流。
Render Tree 不保存业务逻辑。

---

# 22. Renderer

**实现修正（C-030 / C-031）**：没有 `Renderer` trait，只有两个具体类型——
`GpuContext`（adapter / device / queue，**整个进程共享一份**）与每窗口一个的
`SurfaceRenderer`（持有自己的 `Surface` 与管线状态）：

```rust
/// 进程级 GPU 资源（共享 adapter / device / queue）。
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: wgpu::Adapter,
}

/// 一帧所需的全部输入（由 WindowManager::render_input 组装）。
pub struct RenderInput<'a> {
    pub commands: &'a [PaintCommand],
    pub arena: &'a TextArena,
    pub scale: f32,                            // 物理像素 / 逻辑像素
    pub glyphs: Option<&'a dyn GlyphSource>,   // None → 丢弃全部文本！
}

impl SurfaceRenderer {
    pub fn new(ctx: &GpuContext, surface: Surface<'static>) -> Result<Self>;
    /// 离屏目标（T-10 / C-046）：渲进纹理，`read_pixels` 读回，无需窗口。
    pub fn new_offscreen(ctx: &GpuContext, width: u32, height: u32) -> Result<Self>;
    pub fn resize(&mut self, ctx: &GpuContext, width: u32, height: u32);
    pub fn render(&mut self, ctx: &GpuContext, input: &RenderInput<'_>) -> Result<()>;
    /// 读回离屏帧：紧凑 RGBA8 行（已去掉 256B 行对齐 padding）。
    pub fn read_pixels(&self, ctx: &GpuContext) -> Result<Vec<u8>>;
}
```

调用链：`WindowManager::render(window, clock)` 录制命令 → `render_input(window)` →
facade 拿 `RenderInput` 交给 `SurfaceRenderer::render`。

**目标与帧内顺序（C-046）**：内部 `Target::{Surface, Offscreen}` 二选一——窗口目标
`acquire → present`，离屏目标永不 present（`resize` 重建纹理）。`render` 的固定顺序是
**acquire → `tessellate` → `sync_atlas` → `upload_instances` → encode → submit**：
字形是在 `tessellate` 里经 `GlyphSource::glyphs()` 首次光栅化的，所以图集**必须在
tessellation 之后**上传，否则首帧采样空图集、整个窗口第一帧没有文字。

---

# 23. GPU Pipeline

**实现修正（C-031）**：不做逐形状的 vertex / index 生成——先 `tessellate` 成**实例流**
（每个四边形一条 `Instance`），再一次性写进可增长的 instance buffer，用**单条管线 +
实例化绘制**跑完整帧（天然按状态合批，无 index buffer）：

```text
PaintCommand
     ↓
tessellate（scissor = 全部裁剪矩形的交集，最内层圆角随实例携带；裁掉不可见四边形）
     ↓
Vec<Instance>   // corners / uv / colors / inset / mode+clip 圆角半径 / 四角半径 / 圆角裁剪矩形 / texture slot / scissor
     ↓
instance buffer（按需增长，复用）
     ↓
bind group（每纹理一个，批次内只换组）
     ↓
单条 RenderPipeline（WGSL: shader.wgs）→ render pass → wgpu::Queue
```

纹理槽位约定：`SLOT_WHITE = 0`（纯色 / 边框 / 圆角 SDF）、`SLOT_GLYPH_ATLAS = 1`
（覆盖率蒙版），`slot <= SLOT_GLYPH_ATLAS` 走 coverage 模式，其余走 RGBA 模式。

---

# 24. GPU Resource Manager

**实现修正（C-032）**：没有独立 `ResourceManager`——纹理槽位由 `SurfaceRenderer`
自己维护（`HashMap<u32, TextureSlot>`，每个槽位同时持有 `Texture` 与它的 `BindGroup`，
这样合批时只换 bind group），字形图集的上传状态由 `AtlasState { width, height, generation }`
跟踪，`GlyphSource::mark_atlas_uploaded` 清掉 dirty 标记：

```rust
struct TextureSlot {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

struct AtlasState {
    width: u32,
    height: u32,
    generation: u32,
}
```

保留的逻辑概念：

```rust
pub struct Texture {
    pub id: TextureId,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}
```

---

# 25. Font System

**实现修正（C-026 / C-027 / C-028）**：选定 **`fontdb`（字体检索）+ `ab_glyph`（度量 / 光栅化）**，
两者都封装在 `rustui-text` 内，`rustui-widgets` 与 `rustui-renderer` 通过 core 里的两个 trait
访问，谁都不知道 `ab_glyph` 的类型：

```text
Text
 ↓
TextLayoutEngine  （整形 + 换行 + 字形位置，逻辑像素；C-053）
 ↓
GlyphCache        （光栅化结果，按物理尺寸 = 逻辑字号 × scale 缓存，C-048）
 ↓
GlyphAtlas        （单通道 coverage 图集，CPU 侧位图）
 ↓
GlyphSource trait → renderer 上传 wgpu texture
```

两个 trait（定义在 `rustui-core`）：

```rust
/// 供 widget 测量文本。
pub trait TextMeasurer {
    fn layout_text(&self, text: &str, style: &TextStyle, max_width: Option<f32>) -> TextLayout;
    fn measure_text(&self, text: &str, style: &TextStyle, max_width: Option<f32>) -> Size;
    fn line_height(&self, style: &TextStyle) -> f32;
    fn set_scale_factor(&self, scale: f32);
    /// 渲染器拿字形的通道；返回 None 表示"这份测量器画不出文字"。
    fn glyph_source(&self) -> Option<&dyn GlyphSource>;
    fn hit_test_x(&self, text: &str, style: &TextStyle, x: f32) -> usize;
}

/// 供 renderer 取字形与图集。
pub trait GlyphSource {
    fn glyphs(&self, text: &str, style: &TextStyle,
              max_width: Option<f32>, origin: Point) -> Vec<PositionedGlyph>;
    fn atlas_info(&self) -> AtlasInfo;
    fn copy_atlas_into(&self, dst: &mut Vec<u8>);
    fn mark_atlas_uploaded(&self);
    fn atlas_texture_index(&self) -> u32;
}
```

`TextEngine` 同时实现两个 trait，且**必须覆写 `glyph_source()` 返回 `Some(self)`**——
trait 的默认实现是 `None`，而 `RenderInput::glyphs == None` 时渲染器会**丢弃全部文本命令**
（这是踩过的一个真实 bug，见 C-027 与 `crates/rustui/tests/pipeline.rs` 的回归测试）。

**复杂文种 shaping（T-9 / C-053，2026-10-09 关闭）**：`rustui-text` 内部用
**`rustybuzz`（HarfBuzz 的纯 Rust 绑定）整形 + `unicode-bidi` 做双向视觉序换行**，
`ab_glyph` 保留为度量与光栅化后端——两者读同一张 `cmap`，glyph id 一致性由
`shaped_glyph_ids_match_the_rasterizer` 钉住：

* `Shaper` 每次布局请求构建一次（`rustybuzz::Face` 借用 `FontRegistry` 保存的字体原始
  字节，不能跨请求常驻）；只注册 `FontVec` 老接口的字体没有字节，自动退回逐字形排布
  （即改造前的行为）；
* 排布流程：`\n` 切硬行 → 按**字簇（cluster）**贪心折行——断点只落在字簇边界、空格后
  优先、超长词按字簇断，连字永远不会被拆开 → 每个视觉行经 `unicode_bidi::visual_runs`
  切 run、逐 run 以各自方向整形 → 笔位从左到右排（RTL run 在 shaper 输出中已是视觉序）；
* `LaidOutGlyph` 增 `char_index` / `char_count` / `rtl`：连字覆盖的字符区间对外可见，
  `caret_rect` 在簇内按比例插值（连字里按方向键光标仍会移动），`hit_test_x` 按像素位置
  二分字簇并返回**字符索引**（不再隐含"字形序号 == 字符序号"）；
* 拉丁连字与 kerning、天城文合字重排、阿拉伯语境合字与 lam-alef 连字、RTL/混排视觉序，
  由 `crates/rustui-text/tests/shaping.rs` 10 条集成测试覆盖（DejaVu/Noto 系统字体缺失
  时对应测试自动跳过）。

排版在**逻辑像素**里完成，只有光栅化跟 DPI 走。字形缓存按**物理像素尺寸**键控
（`GlyphKey::at_scale`）：14px @2.0 与 28px @1.0 落到同一张位图，两个 DPI 的光栅并存，
`set_scale_factor` 因此**不再清空任何缓存**（布局缓存本就与 scale 无关）；quad 的逻辑尺寸
由 `glyphs()` 用当前窗口的 scale 反算（**C-028 / C-048**）。scale 由
`WindowManager::render_input` 在组装每帧输入时切到该窗口的值——它同时写出
`RenderInput.scale`，两者天然一致（见 §32）。

---

# 26. TextInput

TextInput 必须维护：

```text
Text
Cursor
Selection
Composition
Focus
Scroll
```

数据：

```rust
pub struct TextState {
    pub text: String,
    pub cursor: usize,
    pub selection: Option<Range<usize>>,
}
```

状态的实际归属（**实现修正**）：字符串在 `SharedProperty<String>` 里，`caret` / `anchor`
用**字符**计数（切片时才转字节），`composing` 存 IME 组合中的文本，`scroll_x` 是水平滚动量。

已实现：

```text
IME        组合下划线 + 候选框跟随 caret（C-038）
Clipboard  Ctrl+C / Ctrl+X / Ctrl+V（经 EventContext::clipboard()）
Password   掩码显示
```

后续增加：

```text
Undo
Redo
```

---

# 27. Model

**实现修正（C-033）**：模型 trait 放在 `rustui-model`，值类型是**自有枚举 `ModelValue`**
（不是 `&Self::Item` 引用，这样惰性 / 远端模型也能返回数据），并且提供三个维度的模型，
`TableModel` 多出 `header_data`，`TreeModel` 用 `NodeId(u32)` 并**把展开状态放在模型里**：

```rust
/// 一维：ListView 用。对象安全，可 `Box<dyn ListModel>`。
pub trait ListModel {
    fn row_count(&self) -> usize;
    /// 越界返回 [`ModelValue::None`]。
    fn data(&self, row: usize) -> ModelValue;
    fn is_empty(&self) -> bool;      // 默认实现
    fn contains(&self, row: usize) -> bool;
    fn get(&self, row: usize) -> Option<ModelValue>;
}

/// 二维：TableView 用；表头也是数据（可排序 / 改列宽而不产生第二真相）。
pub trait TableModel {
    fn row_count(&self) -> usize;
    fn column_count(&self) -> usize;
    fn header_data(&self, column: usize) -> ModelValue;
    fn data(&self, row: usize, column: usize) -> ModelValue;
    fn cell(&self, row: usize, column: usize) -> Option<ModelValue>;
}

/// 层级：TreeView 用。
pub trait TreeModel {
    fn root_count(&self) -> usize;
    fn child_count(&self, node: &NodeId) -> usize;
    fn child(&self, node: &NodeId, index: usize) -> Option<NodeId>;
    fn parent(&self, node: &NodeId) -> Option<NodeId>;
    fn data(&self, node: &NodeId) -> ModelValue;
    fn is_expanded(&self, node: &NodeId) -> bool;   // 默认 false
    fn set_expanded(&mut self, node: &NodeId, expanded: bool);
    fn is_leaf(&self, node: &NodeId) -> bool;
}

/// 跨线程 / 跨句柄共享的模型（`SharedModel::boxed()` 走箱化）。
pub trait SharedModel { … }
```

内置实现：`StringListModel` / `TableListModel` / `TreeListModel` / `TreeBuilder`
与 `SelectionModel`（`SelectionChange` 通过信号广播）。

---

# 28. TableView

结构：

```text
TableView
 ├── Header
 ├── Viewport
 │    ├── Row
 │    ├── Row
 │    └── Row
 └── ScrollBar
```

大型数据必须支持：

```text
Virtualization
```

避免：

```text
100000 rows
 ↓
100000 Widget
```

而采用：

```text
100000 rows
 ↓
Visible rows
 ↓
~30 widgets
```

---

# 29. TreeView

```text
▼ vcpkg
  ▼ packages
    ├── fmt
    ├── boost
    └── openssl
  ▼ triplets
    ├── x64-windows
    └── x64-linux
```

TreeModel：

```rust
pub trait TreeModel {
    fn root_count(&self) -> usize;

    fn child_count(
        &self,
        node: NodeId,
    ) -> usize;

    fn child(
        &self,
        node: NodeId,
        index: usize,
    ) -> NodeId;
}
```

---

# 30. Window

**实现修正（C-034 / C-036）**：窗口有两层——`rustui-window::Window` 是**窗口内状态**
（标题 / 尺寸 / root / 表面状态 / 红脏标记），facade 的 `WindowSpec` 是**创建参数**；
两者都通过 `WindowManager` 访问（`WindowManager` 同时持有 `ObjectRegistry` 与 `WidgetStore`）。

```rust
/// 创建参数（逻辑像素）。
pub struct WindowSpec {
    pub title: String,
    pub width: f32,
    pub height: f32,
    pub min_width: Option<f32>,
    pub min_height: Option<f32>,
    pub resizable: bool,
    pub decorated: bool,
    pub visible: bool,
    pub maximized: bool,
}

impl WindowSpec {
    pub fn new() -> Self; // 1024x768, resizable, decorated
    pub fn title(self, title: impl Into<String>) -> Self;
    pub fn size(self, width: f32, height: f32) -> Self;
    // … min_size / resizable / decorated / visible / maximized
}
```

创建：

```rust
Application::new()?
    .window(WindowSpec::new().title("RustUI").size(1200.0, 800.0))
    .run(|app| {
        let id = app.primary_window().expect("registered");
        app.wm().set_root_id(id, root);
    })?
```

`Resize` / `ScaleFactorChanged` / `CloseRequested` 由窗口自己处理（**C-012**），
返回 `EventOutcome { handled, close_requested, request_redraw }`。

---

# 31. Application

**实现修正（C-035 / C-036）**：`Application` 只是 facade 的外壳——真正的状态都在
`State` 里（`wm / gpu / renderers / os_to_ui / ui_to_os / cursors / proxy /
on_task / on_custom / on_update / clock`），winit 的 `EventLoop` 属于 `rustui-platform`，
`ObjectRegistry` 归 `WindowManager` 管；应用不直接持有 `Renderer`。

```rust
pub struct Application {
    spec: Vec<WindowSpec>,
    state: State,
}

impl Application {
    pub fn new() -> Result<Self, UiError>;
    /// 注册一个窗口，可重复调用（多窗口）。
    pub fn window(mut self, spec: WindowSpec) -> Self;
    pub fn on_task(mut self, cb: impl FnMut(TaskId, TaskPayload) + 'static) -> Self;
    pub fn on_custom(mut self, cb: impl FnMut(&'static str, TaskPayload) + 'static) -> Self;
    pub fn on_update(mut self, cb: impl FnMut(&mut WindowManager) + 'static) -> Self;
    /// 进入事件循环；`setup` 拿到 `AppContext` 构建首屏。
    pub fn run(self, setup: impl FnOnce(&mut AppContext<'_>)) -> Result<(), UiError>;
}
```

典型启动：

```rust
Application::new()?
    .window(WindowSpec::new().title("Demo").size(560.0, 380.0))
    .run(|app| { /* build ui, set_root … */ })?;
```

`AppContext` 提供 `{ proxy, ui_mut, wm, open_window, set_root, close_window, exit, spawn }`；
`AppContext::on_update` 是**每帧 `update` 之前**的回调，用来解决"信号回调拿不到
`WindowManager`"的问题（信号签名是 `Fn(&T)`，不携带上下文）。

---

# 32. Frame Pipeline

每帧执行：

```text
Event
 ↓
Update State
 ↓
Layout
 ↓
Build Render Tree
 ↓
Paint
 ↓
GPU Render
 ↓
Present
```

**实现修正（C-035 / C-039）**——facade 中一帧的真实顺序（`Handler::on_update` →
`WindowManager::render` → 提交）：

```text
on_update(&mut WindowManager)     // 应用级回调，可改模型 / 属性
 ↓
update(window, clock)             // 按窗口遍历：root 子树 + overlay 子树
 ↓
layout(window)                    // measure + arrange（每窗口独立）
 ↓
paint(window)                     // 先窗口背景，再内容树，再 overlay，最后弹层（C-024 / C-045）
 ↓
render(window)                    // render_input 把引擎切到该窗口 scale（C-048）→
 ↓                                // RenderInput { commands, arena, scale, glyphs } → wgpu
sync_cursor                       // 把目标光标下发给平台
```

`on_idle` 只做一件事：`wm.take_redraw()` 为 `true` 时向平台请求所有窗口重绘。

完整流程：

```text
┌──────────────┐
│ winit Event  │
└──────┬───────┘
       ↓
┌──────────────┐
│ UiEvent      │
└──────┬───────┘
       ↓
┌──────────────┐
│ Widget Tree  │
└──────┬───────┘
       ↓
┌──────────────┐
│ Layout       │
└──────┬───────┘
       ↓
┌──────────────┐
│ Paint        │
└──────┬───────┘
       ↓
┌──────────────┐
│ Render Tree  │
└──────┬───────┘
       ↓
┌──────────────┐
│ WgpuRenderer │
└──────┬───────┘
       ↓
      GPU
```

---

# 33. Frame Clock

定义：

```rust
pub struct FrameClock {
    pub frame: u64,
    pub delta: Duration,
    pub elapsed: Duration,
}
```

用途：

```text
Animation
Transition
Cursor Blink
Tooltip
Timer
```

---

# 34. Animation

```rust
pub struct Animation {
    duration: Duration,
    elapsed: Duration,
    easing: Easing,
}
```

计算：

```text
progress =
    elapsed / duration
```

然后：

```text
Easing
 ↓
Property
 ↓
Render
```

---

# 35. Theme

统一主题：

```rust
pub struct Theme {
    pub colors: ThemeColors,
    pub typography: Typography,
    pub spacing: Spacing,
    pub radius: Radius,
}
```

Widget：

```rust
button.paint(theme);
```

而不是每个 Widget 固定颜色。

---

# 36. Style

建议：

```rust
pub struct Style {
    background: Color,
    foreground: Color,
    border: Border,
    padding: Insets,
    margin: Insets,
    radius: f32,
}
```

后续可以支持：

```text
Selector
Pseudo State
Inheritance
```

例如：

```text
Button
Button:hover
Button:pressed
Button:disabled
```

---

# 37. 线程模型

## UI Thread

负责：

```text
winit
Object Tree
Widget
Layout
Event
Render Command
```

## Worker Thread

负责：

```text
File
Network
Process
Git
Database
```

通信：

```text
Worker
  ↓
Channel
  ↓
UI Thread
```

禁止 Worker Thread 直接修改 Widget。

---

# 38. 异步任务

**实现修正（C-041 / C-042）**：MVP **不引入 async runtime**（不依赖 tokio），
任务是普通的 `FnOnce() + Send + 'static` 闭包，通过 `PlatformProxy::post_task`
投递到事件循环线程执行——不需要额外线程池，UI 更新天然回到 UI 线程：

```rust
// rustui facade
pub fn spawn(task: impl FnOnce() + Send + 'static) -> bool;
// 内部：proxy.post_task(SPAWN_TASK.0, Box::new(task))，SPAWN_TASK = TaskId(0xC0_FF_EE_01)
```

任务完成后以 `UiEvent::TaskCompleted { task, result }` 回到事件队列（**C-041**），
按 `TaskId` 分发：

| 出口 | 用途 |
| --- | --- |
| `AppContext::spawn` / `rustui::spawn` | 预约任务（id 固定 `SPAWN_TASK`），结果交 `on_task` |
| `AppContext::on_task(TaskId, TaskPayload)` | 用户自定义 `TaskId` 的完成回调 |
| `AppContext::on_custom(&'static str, TaskPayload)` | `post(name, payload)` 的命名事件 |
| `AppContext::on_update(&mut WindowManager)` | 每帧 `update` 之前的模型 → UI 同步 |

**线程约束（C-042）**：闭包必须 `Send`；`SharedProperty` 等模型对象**不是 `Send`**，
不能在 `spawn` 闭包里直接改——把结果发回 `on_task` / `on_custom`，或在 `on_update`
里统一同步到视图。

完成：

```text
Worker（或事件循环）
 ↓
post_task / post
 ↓
UiEvent::TaskCompleted
 ↓
on_task / on_custom
 ↓
Model（SharedProperty）
 ↓
on_update → View
```

---

# 39. Clipboard

**实现修正（C-037）+ 系统接入（T-3 / C-054，2026-10-09 关闭）**：trait 定义在
`rustui-core`，实现在 `rustui-platform`；widget 通过
`EventContext::clipboard() -> Option<&dyn Clipboard>` 访问，不直接碰平台 API。
`set_text` 收 `&self`（平台实现内部自持可变状态），并提供 `is_available()` 默认实现：

```rust
pub trait Clipboard {
    fn get_text(&self) -> Option<String>;

    fn set_text(&self, text: &str);

    /// `true` when the platform clipboard can currently be used.
    fn is_available(&self) -> bool {
        true
    }
}
```

`rustui-platform` 的 `PlatformClipboard` 有两个后端：**系统剪贴板**（`arboard`，Linux 走
X11/XWayland，selection 服务线程在进程存活期间响应外部客户端的粘贴）与**进程内缓冲**
（`in_memory()`，测试与无头模式）。`new()` 打开系统剪贴板，打开或写入失败**降级**到进程内
缓冲并打日志——跨应用复制粘贴可用，无显示服务器时行为不劣于改造前。`arboard` 只允许出现在
`rustui-platform`（依赖豁免见 C-054），widgets 与 core 仍只见 trait。降级态与跨进程
粘贴由 `clipboard.rs` 3 条测试覆盖（无 `DISPLAY` 时系统部分自动跳过）。

---

# 40. Cursor

```rust
pub enum Cursor {
    Default,
    Pointer,
    Text,
    ResizeHorizontal,
    ResizeVertical,
    Crosshair,
}
```

通过 Platform 层设置：事件派发时 widget 调 `EventContext::set_cursor`，帧末由
`State::sync_cursor` 调 `to_winit_cursor(CursorIcon)` 下发（同一图标不重复下发）。

## 40.1 输入法区域（C-038）

输入法的候选框位置与开关走同一条"帧末同步"通路，数据来自**当前聚焦的 widget**：

```text
Widget::ime_cursor_area(ui) -> Option<Rect>      默认 None；TextInput 返回 caret 的窗口矩形
        ↓
WindowManager::ime_area(window)                  向 focused widget 要（每帧重新取，不会残留）
        ↓
State::sync_ime(window)                          缓存 (allowed, area)，无变化不调用
        ↓
WindowHandle::set_ime_allowed / set_ime_cursor_area   逻辑像素，交 winit（§41）
```

规则：

* `area.is_some()` ⇒ `set_ime_allowed(true)`，**先开开关再发区域**（Wayland 后端只在
  `ime_allowed` 时才接受 cursor area）；
* `area == None` ⇒ 只 `set_ime_allowed(false)`，非文本控件拥有焦点时输入法不抢按键；
* `TextInput` 与 `paint` 共用 `caret_rect`，候选框不会与画出来的 caret 漂移；
* X11 只使用位置，忽略区域大小（winit 平台注释）。

---

# 41. DPI

RustUI 内部统一使用：

```text
Logical Pixels
```

平台：

```text
Physical Pixels
```

转换：

```text
Logical × ScaleFactor
       ↓
Physical
```

winit 提供 Scale Factor。

**多窗口（C-048）**：引擎是每应用一份、被所有窗口共享，因此**谁渲染谁设置 scale**——
`WindowManager::render_input` 在组装 `RenderInput` 时把引擎切到该窗口的 scale，光栅化发生
在这之后；字形缓存按物理尺寸键控，两个 DPI 的光栅并存，交替绘制互不清空。measure 与 layout
恒为逻辑像素，不参与换算。

---

# 42. Window ↔ Surface

初始化：

```text
winit::Window
       ↓
wgpu::Instance
       ↓
Surface
       ↓
Adapter
       ↓
Device
       ↓
Queue
```

Render：

```text
SurfaceTexture
 ↓
TextureView
 ↓
CommandEncoder
 ↓
RenderPass
 ↓
Queue.submit()
```

---

# 43. Renderer 生命周期

```text
Application Start
       ↓
Create Instance
       ↓
Create Surface
       ↓
Request Adapter
       ↓
Request Device
       ↓
Create Pipelines
       ↓
Create Resources
       ↓
Render Loop
```

窗口 Resize：

```text
Resize
 ↓
Surface.configure()
 ↓
Render
```

---

# 44. Error Handling

框架错误（**实现修正**：变体都携带诊断信息，另有 `UnknownWindow` / `UnknownWidget`
用于"窗口 / widget 不存在"的查错路径）：

```rust
pub enum UiError {
    WindowCreation(String),
    EventLoop(String),
    GpuUnavailable(String),
    DeviceCreation(String),
    SurfaceCreation(String),
    ResourceCreation(String),
    UnknownWindow(u32),
    UnknownWidget(ObjectId),
    FontDatabase(String),
    Other(String),
}

pub type Result<T> = std::result::Result<T, UiError>;

impl UiError {
    /// 附加一条上下文信息（非 `Other` 会被包装成 `Other`）。
    pub fn context(self, message: &str) -> UiError;
}
```

应用错误不能直接 panic。

原则：

```text
recoverable error
       ↓
Result
       ↓
Application
```

不可恢复错误才允许终止程序。

---

# 45. Widget 状态

Widget 状态建议：

```rust
pub struct WidgetState {
    pub visible: bool,
    pub enabled: bool,
    pub focused: bool,
    pub hovered: bool,
    pub pressed: bool,
}
```

状态影响：

```text
Style
 ↓
Paint
```

例如：

```text
Button
 ├── Normal
 ├── Hover
 ├── Pressed
 ├── Focused
 └── Disabled
```

---

# 46. Button 设计

```rust
pub struct Button {
    id: ObjectId,

    text: Property<String>,

    enabled: Property<bool>,

    clicked: Signal<()>,
}
```

事件：

```text
MouseDown
 ↓
pressed = true

MouseUp
 ↓
pressed = false
 ↓
clicked.emit()
```

---

# 47. Label 设计

```rust
pub struct Label {
    id: ObjectId,

    text: Property<String>,

    text_style: Property<TextStyle>,

    rotation: f32,            // 顺时针度数（C-049）
}
```

Paint：

```text
Layout
 ↓
Text Layout
 ↓
Glyph
 ↓
Painter::text_transformed()   // 恒等变换即 Painter::text()
```

旋转（**C-049**）—— 子节点被 `paint_child` 裁到自己的 bounds，旋转必须在 measure 就预留：

```text
measure   w' = w·|cosθ| + h·|sinθ|   （h' 同理），再按 available 钳制
paint     未旋转的文本块居中放进盒子 → 墨迹框中心 = 盒子中心
wrap      换行宽度迭代求解（换行越窄块越高、外接框越宽）；无解退回不旋转行为
```

---

# 48. TextInput 设计

```rust
pub struct TextInput {
    id: ObjectId,

    text: Property<String>,

    cursor: usize,

    selection: Option<Range<usize>>,

    focused: bool,
}
```

支持：

```text
Keyboard
TextInput
Cursor
Selection
Copy
Paste
Cut
Undo
Redo
IME
```

---

# 49. 性能设计

目标：

```text
60 FPS
```

优先优化：

```text
Layout
Paint Command
Batching
Text
Texture
Widget virtualization
```

避免：

```text
每帧大量 Heap Allocation
每帧创建 GPU Resource
每帧创建大量 String
```

---

# 50. GPU Batching

例如：

```text
100 Buttons
```

不能简单生成：

```text
100 RenderPass
```

应该：

```text
100 Rect
 ↓
Batch
 ↓
Vertex Buffer
 ↓
1 Render Pass
```

降低：

```text
CPU → GPU
```

通信开销。

---

# 51. Virtualization

ListView/TableView/TreeView 必须支持：

```text
Virtualized Rendering
```

例如：

```text
1,000,000 packages
```

只创建：

```text
Visible Area
+
Cache Area
```

对应 Widget。

---

# 52. 内存管理原则

禁止框架核心依赖大量：

```text
Rc<RefCell<T>>
Arc<Mutex<T>>
```

作为默认对象模型。

优先：

```text
ObjectId
Registry
Generational ID
```

进一步可以设计：

```rust
pub struct GenerationalId {
    index: u32,
    generation: u32,
}
```

避免 ObjectId 复用导致的悬空引用。

---

# 53. Object 生命周期

推荐：

```text
Create
 ↓
Register
 ↓
Attach Parent
 ↓
Active
 ↓
Detach
 ↓
Destroy
 ↓
Remove Registry
```

Parent 删除时：

```text
Parent
 ↓
Children
 ↓
Recursive Destroy
```

---

# 54. Declarative UI

第一阶段不实现。

后续可以设计：

```text
RustUI DSL
```

例如：

```text
Window {
    title: "vcpkg Manager"

    VBox {
        Label {
            text: "Packages"
        }

        Button {
            text: "Install"
        }
    }
}
```

最终可以编译成：

```text
Widget Tree
```

---

# 55. 与 QML 的关系

未来可设计：

```text
RustUI DSL
      ↓
Compiler
      ↓
Rust Widget Tree
```

而不是直接复制 QML。

目标：

```text
Declarative UI
+
Rust Type Safety
```

---

# 56. MVP API 示例

**实现修正（C-035 / C-040）**——与 `examples/hello` 一致的真实写法：
窗口由 `WindowSpec` 描述，widget 先 `wm.insert` 取得 `ObjectId`，再由容器 builder 组装，
`set_root_id` 挂到窗口；模型（`SharedProperty`）在 widget 之外：

```rust
use rustui::prelude::*;

fn main() -> rustui::Result<()> {
    Application::new()?
        .window(WindowSpec::new().title("RustUI Demo").size(560.0, 380.0))
        .run(build)
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let name = SharedProperty::new(String::new());

    let input = TextInput::with_text(name.clone()).placeholder("Your name");
    let button = Button::new("Install");

    let wm = app.wm();
    let input = wm.insert(Box::new(input));
    let button = wm.insert(Box::new(button));

    let root = VBox::new()
        .with_spacing(8.0)
        .with_child(wm.insert(Box::new(Label::new("Package Manager"))))
        .with_child(input)
        .with_child(button);
    let root = wm.insert(Box::new(root));

    wm.set_root_id(window, root);
}
```

要点：builder 方法是 `with_*`（返回 `Self`），与子项相关的是 `with_child` / `push`；
`Container::child(...)` 是"链式加子项"的 builder，`with_child` 是关联函数（**C-013**）。

---

# 57. vcpkg Manager 示例架构

RustUI 最终可以承载：

```text
vcpkg Visual Manager
│
├── RepositorySelector
│
├── PackageSearch
│
├── PackageTable
│
├── DependencyTree
│
├── TripletSelector
│
└── OperationPanel
```

业务层：

```text
VcpkgClient
RepositoryManager
OperationManager
ConfigManager
```

GUI 层：

```text
RustUI
```

两者通过 Model/View 解耦。

---

# 58. 测试策略

## Unit Test

测试：

```text
Geometry
Layout
Property
Signal
Model
Event
```

**实现状态**：`cargo test --workspace` 454 个测试 / 32 个目标，覆盖 core / layout /
text / widgets / window / renderer / model / animation 与 facade；其中
`rustui-renderer/tests/offscreen.rs`（离屏读回 + 圆角裁剪 / 四角半径的像素断言，
C-046 / C-051 / C-052）与 `rustui/tests/golden.rs`
（黄金图：控件行 / 打开的下拉弹层 / 旋转文本三场景，C-046 + C-049）需要 GPU，
无适配器的机器自动跳过；`rustui/tests/dpi.rs` 覆盖多窗口 DPI（C-048）；
`rustui-text/tests/shaping.rs`（10 条：连字 / kerning / RTL / 混排 / 天城文 / 阿拉伯 /
hit-test / 字体回退，C-053）与 `rustui-platform` 剪贴板 3 条（降级态 + 跨进程粘贴，C-054）
覆盖本轮 T-9 / T-3。

## Renderer Test

测试：

```text
PaintCommand
Batch
Texture
Font
```

## Integration Test

测试：

```text
Window
Widget
Event
Layout
Rendering
```

**实现状态**：`crates/rustui/tests/pipeline.rs` 是端到端回归（真实 `TextEngine`，无 GPU）：

* 字形源可达渲染器（`glyph_source()` 必须返回 `Some`，否则文字全丢）；
* 填充型子项不饿死兄弟（刚性 / 柔性测量顺序，C-017/C-018）；
* 裁剪栈不变式（禁画入空 clip、文字锚点在 clip 内、栈配平、首条命令为窗口背景）；
* 容器边框是环而不是实心块（C-021）。

## Screenshot Test

```text
UI → Render → 离屏纹理 → readback → 量化网格 → 与 golden 比对
```

**已接入自动对比（T-10 / C-046）**：`crates/rustui/tests/golden.rs` 把真实场景
（`controls` 控件行、`combo_popup` 真实点击打开的下拉）用 `SurfaceRenderer::new_offscreen`
渲到纹理 → `read_pixels` 读回 → 8×8 格、每格三通道 4bit 量化成 `tests/golden/*.grid`
文本 → 与提交的 golden 逐格比对（单格容差 1 级、整幅允许 1% 格漂移，抵抗驱动间抗锯齿差异）。
场景构建器另做结构断言（文本必须进绘制列表），补上网格看不出的"文字缺失"。
重建 golden：`UPDATE_GOLDEN=1 cargo test -p rustui --test golden`；无 GPU 的机器自动跳过。
人工截图（桌面门户）仍用于跨改动抽查。

---

# 59. Benchmark

**实现状态（T-11 / C-047）**：CPU 热路径已接入 criterion
（`crates/rustui/benches/pipeline.rs`，`[[bench]] harness = false`，全部走真实
`WindowManager` 入口）：

```text
cargo bench -p rustui                    # 全部
cargo bench -p rustui --bench pipeline -- -n table   # 单组（-n 跳过绘图）
```

本机基线（release，criterion 0.8 默认 100 样本，Linux / llvmpipe 环境，**2026-10-09
T-9 shaping 接入 `rustybuzz` 后复测**——文本测量因此变贵 20% 上下，属整形的固有成本，
布局缓存仍然命中即零成本）：

| 基准 | 中位数 | 说明 |
| --- | --- | --- |
| `text/measure_natural` | 142.7 ns | 单行自然宽度测量（整形后；原 115.3 ns） |
| `text/measure_wrapping` | 530.7 ns | 320px 宽段落换行测量（整形后；原 429.3 ns） |
| `layout/100` | 25.5 µs | 100 个标签行 measure+arrange（3.91 M 行/s） |
| `layout/1000` | 246.4 µs | 1000 行（4.06 M 行/s，近似线性） |
| `paint/labels_1000` | 290.7 µs | 1000 行的命令录制（3.44 M 行/s） |
| `hit_test/labels_1000` | 15.1 µs | 16 点轮转 ≈ 0.94 µs/点 |
| `frame/labels_1000` | 605.4 µs | update+layout+paint 全帧（≈ layout+paint+68µs） |
| `table/10000` | 9.8 µs | TableView 全帧，虚拟化后只画可见行 |
| `table/100000` | 9.4 µs | 行数 ×10 耗时不变；两者的绘制命令数**完全相等**（bench 内断言） |
| `widget/create_1000_labels` | 106.2 µs | 构造 1000 个 `Label`（9.42 M 个/s） |

三条自检断言防止测到"空帧"：paint/frame 必须画出 >1000 条命令，表格帧必须落在
`(40, 500)` 条且 10k 与 100k 行数相同（虚拟化回归即失败）。

重点 benchmark：

```text
Widget creation
Layout
Hit Test
Paint generation
Text rendering
1000 widgets
10000 widgets
100000 table rows
```

GPU：

```text
Draw Calls
Frame Time
GPU Time
CPU Time
VRAM
```

---

# 60. MVP 开发顺序

建议严格按照：

```text
1. Workspace
      ↓
2. winit Window
      ↓
3. wgpu Renderer
      ↓
4. ObjectId / Registry
      ↓
5. Widget Tree
      ↓
6. Event
      ↓
7. Layout
      ↓
8. Painter
      ↓
9. Label
      ↓
10. Button
      ↓
11. TextInput
      ↓
12. Property
      ↓
13. Signal
      ↓
14. Model
      ↓
15. Table
      ↓
16. Tree
      ↓
17. Theme
      ↓
18. Animation
```

---

# 61. MVP Definition of Done

**实现状态（截至本轮）**——`cargo test --workspace` 454 通过 / 0 失败（32 个测试目标），
`cargo clippy --workspace --all-targets` 0 warning，`cargo fmt --all --check` 干净：

* [x] Linux 可以运行（Wayland/X11，实际运行 + 截图验证）
* [ ] Windows 可以运行（未在本环境验证）
* [ ] macOS 可以运行（未在本环境验证）
* [x] winit Window
* [x] wgpu Renderer
* [x] Object Tree
* [x] Widget Tree
* [x] Event System
* [x] Mouse
* [x] Keyboard
* [x] Focus
* [x] Layout
* [x] Painter
* [x] Label
* [x] Button
* [x] TextInput
* [x] Property
* [x] Signal
* [x] Theme
* [x] 基础错误处理（`UiError` + `Result`）
* [x] 基础测试（单元 + 端到端 `crates/rustui/tests/pipeline.rs` + 黄金图
      `crates/rustui/tests/golden.rs`，T-10 / C-046 + 多窗口 DPI
      `crates/rustui/tests/dpi.rs`，T-2 / C-048）
* [x] 基础 Benchmark（`crates/rustui/benches/pipeline.rs`，criterion 七组，基线见 §59，
      T-11 / C-047；GPU 指标因本机软件光栅化暂缓）

三个示例 `hello` / `calculator` / `table_view` 已实际运行并截图确认渲染正确。
`Design_Corrections.md` §11 的 TODO 表已全部关闭（T-1 ~ T-11，2026-10-09），
无尚未完成项。

---

# 62. 长期路线

```text
RustUI 0.1
│
├── Core
├── Window
├── Event
├── Layout
├── Renderer
└── Basic Widgets
        │
        ▼
RustUI 0.2
│
├── Model/View
├── Table
├── Tree
├── Property
└── Signal
        │
        ▼
RustUI 0.3
│
├── Text
├── IME
├── Clipboard
├── Animation
└── Accessibility
        │
        ▼
RustUI 0.4
│
├── Declarative UI
├── Hot Reload
├── Plugin
└── Advanced GPU
```

---

# 63. 核心架构总结

最终 RustUI 的核心数据流：

```text
                Application
                     │
                     ▼
               ObjectRegistry
                     │
                     ▼
                 Widget Tree
                     │
          ┌──────────┼──────────┐
          ▼          ▼          ▼
        Event       Layout    Property
          │          │          │
          └──────────┼──────────┘
                     ▼
                 Paint Tree
                     │
                     ▼
                Render Tree
                     │
                     ▼
                  Painter
                     │
                     ▼
              Paint Commands
                     │
                     ▼
                WgpuRenderer
                     │
                     ▼
                    wgpu
                     │
                     ▼
                     GPU
```

RustUI 的核心原则：

> **Widget 管理状态，Layout 决定位置，Painter 描述视觉，Renderer 负责 GPU，Platform 负责操作系统。**

这样可以保持：

```text
Widget ≠ Layout
Widget ≠ Renderer
Widget ≠ Platform
Model ≠ View
Application ≠ Window
```

最终形成清晰的职责边界。

