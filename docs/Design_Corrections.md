# RustUI 设计修正记录

**项目名称：** RustUI
**文档类型：** 设计修正（Design Corrections）
**版本：** 0.1.0
**状态：** 随实现同步维护
**对应文档：** RustUI 概要设计 0.1 / RustUI 详细设计 0.1

---

两份设计文档描述的是**初版设计**。实现过程中确认的偏离、补充与取舍集中记录在本文档，
**当本文档与设计文档冲突时，以本文档为准**；设计文档中未被本文列出的部分仍然有效。

每条修正包含三部分：

* **设计** —— 设计文档中的原始约定（要点摘录）；
* **实现** —— 代码中确定下来的现状（API / 行为）；
* **理由** —— 为什么偏离，或偏离带来的影响。

条目按主题分组，编号 `C-0xx` 供交叉引用。

---

# 1. 对象模型与 Widget 生命周期

### C-001 `ObjectId` 是代际句柄

* **设计**：`ObjectId` 标识对象树中的一个对象。
* **实现**：`ObjectId { index: u32, generation: u32 }`。槽位回收后 `index` 复用而 `generation`
  递增，旧句柄自然失效；`ObjectId::INVALID_INDEX` / `ObjectId::invalid()` 表示"空对象"。
* **理由**：避免悬挂 id 指向新对象（use-after-free 的类型级形式）。判断存活一律用
  `WidgetStore::contains(id)` / `ObjectRegistry::contains(id)`，不要缓存"我以为还活着"的结论。

### C-002 `ObjectRegistry` 只保存身份信息

* **设计**：Registry 保存对象与它的数据。
* **实现**：`ObjectRegistry { slots, free, alive, root }` 只保存**身份 / 父子索引 / 子数量 / 根标记**；
  widget 本体（`Box<dyn Widget>`）存放在 `WidgetStore`（内部 `RefCell` 池），通过
  `store.take(id)` / `store.put(id, widget)` 短借。
* **理由**：让"结构关系"与"行为体"分离，遍历结构时不碰 `RefCell`，避免可变借用冲突。

### C-003 `impl_widget_base!` 只实现 `Object`

* **设计**：宏一次性生成 widget 的样板实现。
* **实现**：宏只生成 `Object`（`id` / `children` / `bounds` / `measured` 等）与 `WidgetBase`
  的转发；`measure` / `arrange` / `paint` / `event` / `size_hint` 等由各 widget 自己实现
  （trait 有默认实现，未用到的回调不必写）。
* **理由**：宏生成全量 trait 会掩盖每个 widget 的真实语义，也让"忘记实现"变得不可见。

### C-004 不提供 downcast

* **设计**：应用可能需要把 `Box<dyn Widget>` 转回具体类型。
* **实现**：`WidgetStore` 不暴露 `Any`/`as_any`，全链路没有 `downcast`。
  widget 的跨边界数据用 `SharedProperty<T>` / `Shared<T>`（属性系统）传递。
* **理由**：downcast 是 `Rc<RefCell<T>>` 对象模型的入口（§52 明确避免）；
  需要共享状态时共享的是**属性**，不是 widget 指针。

### C-005 容器即 layout widget（原 §15）

* **设计**：§15 定义"容器"概念。
* **实现**：没有独立的容器类型；承担容器职责的都是布局 widget：
  `Container`（包裹单个子项 + 背景/边框/内边距/裁剪）、`VBox` / `HBox` / `Grid` / `Stack` /
  `Flow`、`Separator` / `Spacer`。
* **理由**：避免"容器"与"布局"两套相似 API。

### C-006 overlay 是独立 root

* **设计**：弹层（popup / menu / dialog）浮在内容之上。
* **实现**：`OverlayList`（`Window::add_overlay(id, z)` / `remove_overlay`）维护独立于内容树的
  overlay root 集合，按 `z` 排序绘制；**在内容树内的 overlay 由父级 `paint_child` 绘制，树外的
  由窗口绘制**（`Window::paint` 跳过与 root 同层的 id，避免画两遍）。
* **理由**：popup 的父 widget 通常会被裁剪，必须脱离内容树参与裁剪与命中测试。

### C-007 `set_focus` 的返回值语义

* **设计**：接口给出"设置焦点"。
* **实现**：
  * `EventContext::set_focus(id, reason)` 返回 `()`（焦点变更由 `FocusManager` 记录，
    `FocusIn` / `FocusOut` 随事件队列产生）；
  * `WindowManager::set_focus(window, id) -> bool`，`false` 表示 **id 未知**（窗口不存在或对象不在该窗口）。
* **理由**：把"参数错误"和"焦点被其它机制拒绝"区分开：前者是返回值，后者是事件。

---

# 2. 事件与输入

### C-008 事件经 `EventQueue`，结果只有两态

* **设计**：`EventResult` 若干变体（含 `Stop` / `Continue` 之类命名）。
* **实现**：`EventResult::{Unhandled, Handled}` + `is_handled()`；widget 也可以通过
  `EventContext::accept()` 显式标记消费（`consumed` 标志位，冒泡在该点停止）。
  事件在 `EventQueue` 中排队后按"目标 → 冒泡"投递。
* **理由**：两态覆盖了全部真实场景；"是否继续冒泡"与"是否已处理"是同一个决定。

### C-009 `EventContext` 不携带 clock / modifiers

* **设计**：事件上下文一并提供帧时钟与修饰键。
* **实现**：`EventContext { ui, store, interaction, focus, queue, overlays, depth, consumed }`，
  **没有 `clock`、没有 `modifiers`**：
  * 帧时钟走 `UpdateContext` / `FrameClock`（`update` 回调）；
  * 修饰键在 `UiEvent` 自身（`Modifiers` 字段）里。
* **理由**：避免同一信息存在两个来源；事件是瞬时的，时钟是帧级的。

### C-010 platform → ui 事件经 `EventTranslator`

* **设计**：winit 事件直接映射为 `UiEvent`。
* **实现**：`rustui-platform::event::EventTranslator` 完成转换（跟踪 DPI scale、修饰键状态、
  输入法状态），winit 类型不越过 `rustui-platform` 边界；裸转换函数只用于测试。
* **理由**：`rustui-core` / `rustui-window` 不得依赖 winit（G-001 / P1）。

### C-011 窗口层拦截 Tab / Shift+Tab

* **设计**：焦点遍历由焦点管理器处理。
* **实现**：**窗口 dispatch 层**拦截 `Tab` / `Shift+Tab` 调用 `focus_next` / `focus_previous`；
  同时 `TextInput("\t")` 在文本输入中被**吞掉**（不进入文本缓冲），否则 Tab 会插入制表符。
* **理由**：Tab 的语义由窗口决定（模态对话框内应循环），不能让获得焦点的 widget 单方面定义。

### C-012 窗口自己处理系统事件，返回 `EventOutcome`

* **设计**：平台事件统一进分发管线。
* **实现**：`Window::dispatch` 内部处理 `Resize` / `ScaleFactorChanged` / `CloseRequested`
  （记录尺寸、更新 scale、置位 `close_requested`），返回
  `EventOutcome { handled, close_requested, request_redraw }`；
  `WindowManager` 依据 `outcome.close_requested || wm.take_close_request(id)` 关窗，
  没有窗口时则 `ctx.exit()`。
* **理由**：窗口生命周期事件不是 widget 事件，交给窗口自身最省事且不漏。

### C-013 `UiEvent` 只 `derive(Clone, Debug)`

* **设计**：事件可比较/可序列化。
* **实现**：只 `Clone + Debug`；测试断言用 `matches!(event, UiEvent::PointerMove { .. })`。
* **理由**：多数事件含浮点坐标与平台句柄，`PartialEq` 容易给出误导性的"相等"。

---

# 3. 布局

### C-014 `MainAxisSize::{Auto, Fill, Fixed}`

* **设计**：主轴尺寸含 `Expand`。
* **实现**：只有 `Auto`（贴内容）、`Fill`（吃满父给的空间）、`Fixed`（显式指定），没有 `Expand`。
  `BoxLayout::with_fill()` 同时设置 `MainAxisSize::Fill` 与 `hint = SizeHint::fill()`。
* **理由**：`Expand` 与 `Fill` 在"父给多少吃多少"上重复；权重问题交给 flex。

### C-015 `CrossAxisAlign` 默认 `Start`

* **设计**：交叉轴默认拉伸（类似 CSS `stretch`）。
* **实现**：`BoxLayout::new()` 的 `cross_align: CrossAxisAlign::Start`，需要拉伸要显式设置。
* **理由**：默认拉伸会把"贴内容高度"的子项（按钮、标签）撑成整行高度，出乎意料。

### C-016 `apply_fill` 是 measure 的统一收尾

* **设计**：（未明确）
* **实现**：`rustui_layout::apply_fill(size, available, hint)`：当某轴 `hint` 为 `∞` 而
  `available` 有限时，取 `available`；否则保留测量值。所有布局 widget 的 `measure` 最后一步
  统一走 `apply_fill(clamp_to(...), available, hint)`。
* **理由**：把"按提示填充"集中到一处，避免每个 widget 各写一遍且不一致。

### C-017 box 测量：刚性先行，柔性按权重分池

* **设计**：每个子项看到"剩余的主轴空间"。
* **实现**：`BoxLayout::measure` 把子项分成两组：
  1. **刚性**（无 flex 权重且该轴 `hint` 有限）按原顺序先测，各自看到剩余空间；
  2. **柔性**（有 flex 权重或该轴 `hint` 为 `∞`）后测，并按权重瓜分"留给柔性组的池"
     （`flex_pool * w_i / weight_left`），互不饿死。

  `BoxLayout::arrange` 里柔性子项的最终主轴尺寸是 `pool * share`，其中
  `pool = content_main - 间隙 - Σ刚性测量值`（**覆盖**测量阶段得到的 main 值；测量值只决定交叉轴）。
* **理由**：原实现中"先测的填充型子项（TextInput / ScrollView…）会拿走全部剩余空间，把后面的
  兄弟测成 0 宽"，按钮因此消失。这是本轮通过端到端测试发现的真实缺陷。
* **影响**：`set_flex_of(id, w)` 覆盖了权重的子项，**即使它自己的 hint 是刚性的**，也参与柔性候选。

### C-018 填充型 widget 必须在 `size_hint` 里声明扩展

* **设计**：（未明确）
* **实现**：`measure` 会吃满 `available` 的 widget，必须让 `size_hint` 与之匹配，否则 C-017 的
  分组会把它们判成刚性：

  | widget | `size_hint()` |
  | --- | --- |
  | `TextInput` / `ProgressBar` | `width: ∞`（横向填充，纵向 hug） |
  | `ScrollView` / `ListView` / `TableView` / `TreeView` | `SizeHint::fill()`（双向填充） |
  | `Label` / `Button` / `Checkbox` / `Menu` / `ComboBox` / `Dialog` | 默认（hug，`min(自然尺寸, available)`） |
* **理由**：hint 是"我想占多少"的声明，`measure` 是它的执行；两者不一致就会饿死兄弟。

### C-019 `Container::child` 是 builder，`with_child` 是关联函数

* **设计**：（命名未明确）
* **实现**：
  * `Container::child(self, id) -> Self` —— builder 方法（同时 `base.add_child(id)`）；
  * `Container::with_child(id) -> Self` —— 关联函数，等价于 `Container::new().child(id)`。
* **理由**：与 `VBox::with_child` 等既有 builder 风格保持一致，同时让
  `Container::new().child(a).with_padding(..)` 这种链式写法成立。

---

# 4. 坐标与绘制

### C-020 坐标双空间约定（最重要的一条）

* **设计**：paint 使用窗口坐标。
* **实现**，明确区分两个空间：

  | 位置 | 空间 | 说明 |
  | --- | --- | --- |
  | `WidgetBase::bounds()` | **局部**（父空间原点） | `arrange` 收到的 rect 也是父空间 |
  | `WidgetBase::window_bounds()` | **窗口** | 由树路径累加得到 |
  | `PaintContext` 的帮助方法（`fill_rect` / `stroke_rect` / `push_clip*` / `paint_child` / `focus_ring` …） | 参数吃**局部**，内部 `to_window_rect` | |
  | `ctx.painter()` 的调用（`rect` / `rounded_rect` / `text` / `line` / `border` …） | 参数吃**窗口**，**没有任何偏移** | |
* **理由**：`Painter` 是帧级命令缓冲，不知道当前 widget 是谁；偏移只能由 `PaintContext` 提供。
* **教训（本轮修复）**：
  * `Label::paint` 曾用 `bounds()`（局部）喂 `painter().text(...)`，文字落在 (0,0) 被自己的裁剪框裁掉；
  * `TextInput::paint` 曾把**已经转换过的窗口 rect** 再传给 `ctx.push_clip_rounded`（局部 API），
    被二次偏移后与当前裁剪交集成为空矩形，占位符整段被裁。
  * 写 paint 的规则：**先 `let bounds = ctx.to_window_rect(self.base.bounds());` 再用 `painter()`；
    传给 `ctx.*` 的一律用 `self.base.bounds()`。**

### C-021 `stroke_rect` 是环形边框，不是"实心 + 透明擦除"

* **设计**：（未明确）
* **实现**：`PaintContext::stroke_rect(rect, radius, color, width)` 现在直接产出
  `PaintCommand::Shape { fill: Fill { color: TRANSPARENT, border: Border { widths: all(width), color } } }`
  （走 `Painter::border`），由渲染器 tessellate 成环；**边框画在 rect 内侧**，与
  `Fill::border`（按钮等的边框）一致。
* **理由**：原实现是"外扩 0.5w 填实心 → push_clip → `rect(_, TRANSPARENT)` 擦除内芯"，
  但 `Painter::rect` 对 `alpha <= 0` 早退，擦除根本不生效 → 整块矩形被涂成边框色，
  **盖住在此之前绘制的子内容**（计算器显示屏的 "0" 就是这样消失的）。
* **影响**：容器 `paint_begin`（背景）→ 子 → `paint_end`（边框）的顺序现在是安全的。

### C-022 `push_clip_rounded` 的圆角是近似的

* **设计**：支持圆角裁剪。
* **实现**：`PaintContext::push_clip_rounded` 只与**矩形**求交得到 `current_clip`，把 `radius`
  记录进 `PaintCommand::PushClip`；裁剪栈本身是矩形的。
* **理由**：scissor 是矩形；圆角由渲染器在着色阶段按半径处理，越界部分是近似（对 2~8px 圆角不可见）。
* **状态（2026-10-09）**：**近似段已消除，见 C-051** —— 矩形部分仍全部由 scissor 执行
  （裁剪栈保持矩形），圆角改由片元 SDF 精确裁出。

### C-023 `Fill` 结构体 + SDF 圆角，`max_side` 取最大圆角

* **设计**：填充用颜色 + 圆角两个参数。
* **实现**：`Fill { color, gradient, border, radius: EdgeRadius }`；渲染器用 SDF 画圆角，
  `EdgeRadius::max_side()` 取四个角最大值作为统一半径。
* **理由**：一条命令同时表达"渐变 + 边框 + 圆角"，减少命令数；SDF 单半径是性能取舍，
  非等圆角（如上 12 下 0）会被近似成最大值。
* **状态（2026-10-09）**：**单半径近似已消除，见 C-052** —— `Instance` 逐角携带半径，
  着色器按片元所在象限取值，`max_side()` 不再参与绘制路径。

### C-024 窗口背景由框架绘制

* **设计**：（未明确）
* **实现**：`Window::paint` 在 `painter.reset(viewport)` 之后立即
  `painter.fill_all(theme.colors.window_background)`，再画内容树与 overlay。
* **理由**：`window_background` 原本无任何使用者，应用层忘记画就会露出 GPU clear 色（黑）。
  背景属于主题，属于框架。

### C-025 `PaintCommand::Text` 引用帧级 `TextArena`

* **设计**：文本命令带字符串。
* **实现**：`Text { position, run: TextRunId, range: Range<u32>, style, max_width }`；
  字符串 intern 在帧级 `TextArena`（`Painter::arena()`），命令只持 id + 切片范围。
* **理由**：避免每帧每条文本克隆 `String`；`TextRunId` 每帧 reset。

### C-045 弹层通道：widget 在内容树之外画自己的弹层（本轮关键修复）

* **设计**：C-006 的 overlay 只服务**独立 root**；ComboBox 的弹层由控件自己绘制（不是子
  widget），`popup_rect` 固定画在 `bounds.bottom()`（原 TODO T-4）。
* **实现**：新增 `Widget::paint_popup`（默认空实现）+ `PaintContext::request_popup(id)` /
  `PaintContext::paint_popup(id)`，队列挂在 `OverlayList` 上
  （`request_popup` 去重入队，`take_popups` 每帧取走并清空）。widget 在普通 `paint` 里
  `request_popup(self.id())`，`Window::paint` 在**内容树与 overlay root 之后**跑弹层通道；
  弹层 context 的 `offset` = 该 widget 的 `window_bounds().origin()`，`clip` = 窗口矩形
  （`Painter` 侧同步 `push_clip` / `pop_clip`，帧末栈配平）。`popup_rect` 改为按
  `Interaction.viewport`（在 `arrange` 里缓存——`hit_test` 没有 ctx 可问）选择方向：
  下方放不下就翻到上方，两侧都不够则取空间大的一侧，溢出部分由窗口裁剪。
* **理由**：修掉三个真实缺陷（均为"截图才暴露"的渲染问题）：
  1. `paint_child` 把 clip 收窄到 widget 自身矩形，弹层在控件边界以下的部分被**整块裁掉**
     （探针实测：面板 `y=36..144`，生效 clip 是 `(0,0,67,36)`）；
  2. 后绘制的兄弟（示例里的 `TableView`）把弹层整块盖住——弹层必须在整棵树之后画；
  3. 空间不足时不翻转（T-4）。
* **测试**：`an_open_combo_box_panel_escapes_the_clip_of_its_own_bounds`、
  `the_combo_box_popup_flips_above_the_control_when_there_is_no_room_below`、
  `the_combo_box_panel_is_painted_above_later_siblings`（rustui-widgets），
  `the_popup_layer_is_painted_after_the_content_tree`（rustui-window），
  `popup_layer_requests_are_deduplicated_and_drained`（rustui-core）；
  另有真实 Wayland 截图确认下拉面板压在表格之上（此前完全不可见）。

---

# 5. 文本与渲染

### C-026 `glyph_source()` 是渲染拿到字形的唯一通路（本轮关键修复）

* **设计**：文本引擎同时提供测量与字形。
* **实现**：
  * `TextMeasurer::glyph_source(&self) -> Option<&dyn GlyphSource>` —— trait 默认返回 `None`；
  * `impl TextMeasurer for TextEngine` 必须覆写返回 `Some(self)`（`TextEngine` 另有
    `impl GlyphSource for TextEngine`）；
  * `WindowManager::render_input()` 把它放进 `RenderInput::glyphs`，
    注释写明 **`None` 丢弃全部文本**。
* **理由**：默认 `None` 是给"只测量不渲染"的实现（测试里的 `StubText`）用的；真实引擎忘了覆写
  就会静默渲染不出任何文字——这正是最初截图里只有形状没有文字的原因。

### C-027 `GlyphSource` API 形状

* **设计**：（未细化）
* **实现**：
  ```rust
  fn glyphs(&self, text: &str, style: &TextStyle, max_width: Option<f32>, origin: Point)
      -> Vec<PositionedGlyph>;
  fn atlas_info(&self) -> AtlasInfo;            // 含 dirty 标记
  fn copy_atlas_into(&self, dst: &mut Vec<u8>); // 单通道覆盖率位图
  fn mark_atlas_uploaded(&self);                // 渲染器上传后清除 dirty
  fn atlas_texture_index(&self) -> u32;
  ```
* **理由**：atlas 在内部可变性后面，无法长借，故用"拷贝 + 标记已上传"的两步协议。

### C-028 `TextMeasurer::set_scale_factor` 与多窗口限制

* **设计**：DPI 由窗口处理。
* **实现**：
  * `TextMeasurer::set_scale_factor(&self, f32)` 默认空实现，`TextEngine` 转发到内部 state；
  * **布局与 scale 无关**（measure 恒为逻辑像素），scale 只影响字形光栅化；
  * scale 的设置时机与缓存策略见 **C-048**：`WindowManager::render_input` 在组装每帧输入时
    把共享引擎切到**该窗口**的 scale（原 facade 帧首 `sync_engine_scale` 已随之删除）。
* **原限制（C-048 已消除）**：多窗口共享同一个 `TextEngine`，切换 scale 时会**清空整个
  字形缓存与布局缓存**——不同 DPI 的窗口交替绘制时每帧重栅格化全部字形（图集代数也随之
  反复增长）；且 scale 只在 facade 帧首设置，脱离 facade 的路径（直用 `WindowManager` 的
  测试、黄金图）拿不到窗口自己的 scale，2x 窗口会拿 1x 光栅凑合。

### C-029 `measure_text(.., None)` 取自然宽度

* **设计**：（未明确）
* **实现**：`measure_text(text, style, max_width)`：
  * `None` → 按**自然宽度**排版（单行不换行），用于居中、按内容定宽；
  * `Some(w)` → 按 `w` 换行，返回的块宽可能是 `w`。
* **影响**：把 `Some(bounds.width)` 传给居中计算会得到"块宽 = bounds 宽"从而居中失效；
  居中必须用 `None`。

### C-046 atlas 在 tessellate 之后上传 + 离屏渲染通道（T-10 关键修复）

* **缺陷**：`SurfaceRenderer::render` 的顺序是 `sync_atlas → tessellate → draw`，而字形是
  **在 `tessellate` 里**通过 `GlyphSource::glyphs()` 才首次光栅化进 atlas 的 —— 于是首帧
  `sync_atlas` 上传的是空（或陈旧）图集，窗口**第一帧没有文字**。真实窗口里第二帧就补上了，
  所以此前只有单帧的无头渲染把它暴露出来：首版黄金图整幅没有一个文字像素。
* **实现**：
  * `render` 顺序改为 **acquire → `tessellate` → `sync_atlas` → `upload_instances` → encode**，
    注释写明"图集必须在 tessellation 之后上传"；
  * 新增离屏目标（T-10）：内部 `Target::{Surface, Offscreen}` 枚举 +
    `SurfaceRenderer::new_offscreen(ctx, w, h)`（`Rgba8UnormSrgb`，与窗口 surface 优先选的
    sRGB 格式一致，故离屏与上屏同色）+ `SurfaceRenderer::read_pixels()`
    （`copy_texture_to_buffer` → `map_async` + `wait_idle` → 去掉 256B 行对齐 padding，
    返回紧凑 RGBA8 行）；`resize` 对离屏目标重建纹理，`render` 对离屏目标不 present。
    `GpuContext::new()` 本就不要求窗口，故整条链路无需 winit。
* **理由**：修的是真实缺陷（首帧丢字），不是测试技巧；离屏渲染则把"人工截图比对"（§58）
  变成可重复执行的黄金图回归。
* **测试**：`crates/rustui-renderer/tests/offscreen.rs`（读回像素、resize 后尺寸、空帧），
  `crates/rustui/tests/golden.rs` 两条黄金图（`controls` 与 `combo_popup`，后者靠真实
  `MouseDown` 打开下拉）；golden 是 8×8 网格、每格 4bit 三通道，`UPDATE_GOLDEN=1` 重建，
  单格容差 1 级、整幅允许 1% 格漂移；无 GPU 时自动跳过。hello 示例真实 Wayland 截图复核。

---

# 6. 主题与样式

### C-030 `StateColors` helper

* **设计**：状态色写在 widget 里。
* **实现**：`theme.button` / `theme.surface_button` 等提供 `StateColors`，
  以 `background(state)` / `foreground(state)` 之类方法按 `WidgetState` 取色；`WidgetState`
  由 `ctx.state_of(id, base.state())` 合成（hover / pressed / focused / disabled 叠加）。
* **理由**：换主题不需要改 widget 代码（G-006）。

---

# 7. Model / View

### C-031 用 `Shared<T>` 实现模型 trait

* **设计**：为 `Shared<T>`（`Rc<RefCell<T>>` 的受控封装）直接实现 `ListModel` 等。
* **实现**：受 orphan rule 限制，实际以**扩展 trait** 形式提供：
  * `impl ListModel for Shared<StringListModel>` / `impl TableModel for Shared<StringListModel>` /
    `impl TableModel for Shared<TableListModel>` / `impl TreeModel for Shared<TreeListModel>`；
  * `SharedModel::boxed() -> Shared<Box<dyn Model>>`（`shared.rs`，带 doctest）。
* **理由**：应用侧需要在回调里改模型；没有 downcast 的前提下，`Shared<T>` 是唯一共享入口。

### C-032 列表/表格/树自绘并虚拟化

* **设计**：复用通用 item 组件。
* **实现**：`ListView` / `TableView` / `TreeView` / `ComboBox` / `Menu` 全部**自绘**（自己
  tessellate 行、表头、twisty、滚动条），按滚动偏移计算可见区间
  （`view::visible_range` / `item_top` / `paint_thumb`），只画可见行——任意规模的模型只产生
  固定数量的绘制命令。
* **理由**：行级 widget 会把 retained 树撑大；虚拟化必须落在绘制层。

### C-033 运行时数据靠 `SharedProperty` 绑定

* **设计**：widget 持有模型引用。
* **实现**：文本类 widget 接 `SharedProperty<String>`（`Label::with_text` / `TextInput::with_text`），
  运行时改 `property.set(..)` 即触发重绘；模型类 widget 接 `Shared<T>` 模型 + `Signal` 通知。
* **理由**：属性是可见的依赖，不需要 widget 之间互相持有引用（§52）。

---

# 8. 窗口、平台与 facade

### C-034 crate 依赖方向

* **设计**：（分层未完全展开）
* **实现**：
  * `rustui-window` **不依赖** `rustui-platform`（只依赖 core / layout / renderer 接口）；
  * facade `rustui` 同时依赖 `rustui-platform` + `rustui-window` + `winit` + `wgpu`，**接线在 facade**：
    平台事件 → `WindowManager`，`WindowManager` 的请求 → 平台；
  * `rustui-widgets` 不依赖平台与 wgpu（G-005）。
* **理由**：窗口管理可以在无平台的测试里跑（`WindowManager::new(text)` 即可）。

### C-035 facade 的应用骨架

* **设计**：`Application` 承载生命周期。
* **实现**：
  ```rust
  Application::new()?
      .window(WindowSpec::new().title(..).size(..))   // 可多次，多窗口
      .on_task(..).on_custom(..).on_update(..)        // 可选回调
      .run(|app| { ... })                             // setup，拿到 AppContext
  ```
  `Handler<F>` 用于把普通闭包适配成 `EventHandler`；`State` 内部持有
  `wm / gpu / renderers / os_to_ui / ui_to_os / cursors / proxy / on_task /
  on_custom / on_update / clock`。
* **帧管线**（每帧）：`on_update → update → layout → paint → render → sync_cursor`；
  `render` 内的 `WindowManager::render_input` 把共享 TextEngine 切到该窗口的 display scale
  （C-048，原帧首 `sync_engine_scale` 已删除），随后 GPU 侧才 tessellate / 光栅化。
* **GPU**：`InstanceDescriptor::new_with_display_handle(Box::new(SurfaceTarget(WindowHandle)))`，
  facade 内的 `SurfaceTarget` 实现 rwh 的 `HasWindowHandle + HasDisplayHandle + Debug`
  （`create_surface` 需要 `wgpu::DisplayAndWindowHandle`）。
* **重绘**：`notify::set_redraw_sink(wm.ui().redraw().clone())`；`on_idle` = `wm.take_redraw()`，
  为 `true` 时向平台请求所有窗口重绘。

### C-036 `AppContext` 能力与 `on_update`

* **设计**：（未展开）
* **实现**：`AppContext::{proxy, ui_mut, wm, open_window, set_root, close_window, exit, spawn}`；
  `on_update(FnMut(&mut WindowManager))` 在每帧 `update` 之前回调——解决"信号回调拿不到
  `WindowManager`"的问题（信号签名是 `Fn(&T)`，不携带上下文）。

### C-037 `Clipboard` 在 core、实现在 platform

* **设计**：剪贴板是平台能力。
* **实现**：`Clipboard` trait 定义在 `rustui-core`（`EventContext::clipboard() -> Option<&dyn Clipboard>`），
  实现在 `rustui-platform`，MVP 为**进程内缓冲**（不读写系统剪贴板）。
* **状态（2026-10-09）**：**跨应用限制已消除，见 C-054** —— 实现改用 `arboard` 接系统
  剪贴板，打开失败降级回进程内缓冲；trait 位置不变，widgets 零改动。

### C-038 光标与 IME

* **设计**：光标/输入法由窗口设置。
* **实现（光标）**：`to_winit_cursor(CursorIcon)`（按值）完成映射，帧末 `sync_cursor` 统一下发。
* **实现（IME，2026-10-08 补全，原 T-1）**：输入法的"开关 + 候选框位置"按每帧一次的
  `State::sync_ime` 下发，与 `sync_cursor` 同构（带 `(allowed, area)` 双字段缓存，无变化不调用）：
  1. `Widget::ime_cursor_area(&self, ui: &UiContext) -> Option<Rect>`——**默认 `None`**，
     表示"这个 widget 不需要输入法"；`TextInput` 覆写为 caret 的**窗口坐标**矩形
     （与 `paint` 共用同一个 `caret_rect`，候选框不会与画出来的 caret 漂移），
     未聚焦 / 不可见 / 被禁用时返回 `None`；
  2. `WindowManager::ime_area(window) -> Option<Rect>`——向**当前聚焦的 widget** 要这个矩形，
     因此每次布局变化后都是新的，不会残留旧位置；
  3. facade `sync_ime`：`area.is_some()` 决定 `set_ime_allowed`，**先开开关再发区域**
     （Wayland 后端只在 `ime_allowed` 时才接受 cursor area），`None` 时只关开关；
  4. platform `WindowHandle::set_ime_cursor_area(Rect)`——收**逻辑像素**，交给 winit
     （`LogicalPosition`/`LogicalSize`，§41）。
* **限制**：X11 只用位置不用区域（winit 平台注释）；widget 未被绘制（滚出视口 / 隐藏）时
  仍按其自身状态返回矩形。

### C-039 update 按窗口层遍历

* **设计**：每帧一次 `update`。
* **实现**：`WindowManager::update(window, clock)` 只跑**该窗口**的 root 子树与 overlay 子树
  （`Window::update`），多窗口各自独立；`render(window, clock)` =
  `update → layout → paint`。
* **理由**：不同窗口的帧时钟（动画）可以独立推进，也避免为不可见窗口付出代价。

---

# 9. 异步与线程

### C-040 `spawn` 走 `post_task`，其余走回调

* **设计**：提供 `spawn`。
* **实现**：
  * `AppContext::spawn(FnOnce() + Send)` → `post_task(SPAWN_TASK = TaskId(0xC0_FF_EE_01), Box<closure>)`；
    `on_posted_event` 里用 `Rc::downcast::<SpawnTask>()` + `Rc::try_unwrap` 在**主线程**执行闭包
    （`TaskPayload = Rc<dyn Any>`，`Rc::try_unwrap` 对 `dyn Any` 不可用，必须先 downcast）；
  * 其余 `TaskCompleted` / `Custom` → `AppContext::on_task` / `on_custom` 回调，
    没有注册回调时 `Custom` 投递给主窗口；
  * `AppContext::proxy() -> PlatformProxy`（按值 clone，`Send`），用于跨线程投递。
* **限制**：`spawn` 的闭包必须 `Send`，而 `SharedProperty` 不是 `Send` —— **不能在 `spawn` 里
  直接改 UI**；更新 UI 的正确方式是 `post_task` 一个自定义消息 + `on_task`/`on_custom`，
  或者用同线程的 `on_update`（示例 `table_view` 即如此）。

### C-041 worker 结果回 UI 的推荐路径

* **设计**：（未展开）
* **实现**：`std::thread::spawn` → 干活 → `proxy.post("topic", payload)` → `on_custom(topic, payload)`
  或 `on_task` → 在回调里改模型/属性 → 请求重绘。

---

# 10. 测试约定

### C-042 信号连接不留句柄

* **设计**：`Signal::connect` 返回可断开的连接。
* **实现**：`connect(cb)` 不返回/不保留句柄；测试用 `Rc<Cell<u32>>`（同线程）或
  `Arc<AtomicU32>`（需要跨线程，如 `spawn` 测试）在闭包里计数。
* **理由**：避免句柄泄漏与"忘记断开"的生命周期噪音。

### C-043 几何断言必须在 layout 之后

* **设计**：（未明确）
* **实现**：`WidgetBase::window_bounds()` 只在 `arrange` 后更新；发事件、改属性之后要先
  `wm.layout(window)` 或 `wm.render(window, clock)`，否则断言的是上一帧的几何。
* **影响**：widget 单测里"点一下看尺寸"必须显式跑一帧。

### C-044 端到端回归测试 `crates/rustui/tests/pipeline.rs`

真实 `TextEngine`（系统字体）+ `WindowManager`，不碰 GPU，覆盖本轮发现的三类静默失效：

1. `a_real_text_engine_reaches_the_renderer` —— `RenderInput::glyphs` 必须是 `Some`，
   且每条 `PaintCommand::Text` 都能解出非空字符串与非空字形；
2. `a_filling_child_does_not_starve_its_sibling` —— 填充型子项（TextInput）不能把兄弟（Button）
   压成 0 宽，行必须首尾填满（C-017）；
3. `every_command_lands_inside_the_clip_that_contains_it` —— 维护裁剪栈：
   不允许往**空裁剪**里画任何东西，文字锚点必须落在当前裁剪内，裁剪栈必须配平，
   首条命令是覆盖 viewport 的窗口背景（C-020 / C-024）；
4. `a_container_border_is_a_ring_and_its_child_stays_visible` —— 容器边框必须是"透明底 + border"
   的环，绝不能是实心矩形（C-021）。

### C-047 基准测试：criterion，只测 CPU 热路径

* **设计**：§59 列出 Widget creation / Layout / Hit Test / Paint generation / Text rendering /
  1000 widgets / 10000 widgets / 100000 table rows 一组基线，另列 GPU 指标
  （Draw Calls、Frame Time、GPU Time、CPU Time、VRAM）。
* **实现**：`crates/rustui/benches/pipeline.rs` —— `criterion 0.8`，crate 里
  `[[bench]] name = "pipeline" harness = false`；全部走真实 `WindowManager` 入口：
  * `text` —— `measure_text` 自然宽度 / 320px 换行（§59 "Text rendering"）；
  * `layout` —— 100 与 1000 个标签行的 `wm.layout`（measure + arrange，含文本测量）；
  * `paint` —— 1000 行的 `wm.paint`；
  * `hit_test` —— 16 个点轮转 `wm.hit_test`（单点会永远只测一条分支）；
  * `frame` —— `wm.render`（update + layout + paint 全帧，1000 widgets）；
  * `table` —— TableView 10k / 100k 行的全帧（虚拟化：只有可见行参与）；
  * `widget` —— 构造 1000 个 `Label`。
* **GPU 侧不测**：本机是软件光栅化（llvmpipe/ZINK），Draw Calls / Frame Time 在这里没有
  代表性；等有真机再补 §59 的 GPU 指标。
* **运行**：`cargo bench -p rustui`（结果落 `target/criterion/`），
  `cargo bench -p rustui -- layout` 只跑一组。

### C-048 多窗口 DPI：字形缓存按物理尺寸键控 + 每窗口渲染时切换 scale（T-2 关闭）

* **问题（C-028 的原限制）**：共享 `TextEngine` 只有一个全局 `scale`，且 `set_scale_factor`
  一变就**清空字形缓存与布局缓存**；缓存键 `GlyphKey` 只有逻辑字号，装不下"同一字号、不同
  DPI"的两份光栅。
* **修复**：
  * **`GlyphKey::at_scale(scale)`** —— 所有缓存查/写先经它换成**物理尺寸键**
    （`size_64 = 逻辑字号 × scale`）。同一物理尺寸来自不同 (字号, scale) 组合时（14px @2.0
    与 28px @1.0）本来就该共用同一张位图；几何反算（`region / scale`）在 `glyphs()` 里按
    当前窗口的 scale 做，所以 quad 永远是逻辑像素。
  * **`set_scale_factor` 不再清缓存** —— 只更新当前 scale；两个 DPI 的光栅并存，切换零成本
    （布局缓存本就与 scale 无关，清它纯属浪费，一并去掉）。
  * **`WindowManager::render_input` 是唯一的 scale 设置点** —— 它组装 `RenderInput.scale`
    的同时把引擎切到同一 scale，`input.scale`（tessellate 的位置倍数）与引擎光栅化 scale
    天然一致；脱离 facade 的用法（测试、黄金图）同样受益。facade 帧首 `sync_engine_scale`
    与 `State::engine_scale` 字段随之删除。
* **收益**：1x/2x 窗口交替时图集不再反复重建、字形不重复光栅化；2x 窗口拿到真正 2x 的光栅
  （锐度正确）。
* **测试**：`crates/rustui/tests/dpi.rs`（两个 `scale_factor` 1.0/2.0 的窗口轮流
  render + 按 tessellate 同样方式光栅化，断言 2x 新增自己的光栅、来回切换 `cached_glyphs`
  与累计光栅化次数不变）；`rustui-text` 单测 `scale_change_keeps_every_dpi_cached`、
  `at_scale_keys_rasters_by_physical_size`。

### C-049 文本变换：`PaintCommand::Text.transform` + `Label::rotation`（T-8 关闭）

* **问题**：`Shape` / `Image` 带 `transform`，`Text` 没有 → 文本永远与窗口轴对齐，不能旋转。
* **实现**：
  * `PaintCommand::Text` 增加 `transform: Transform`（与 `Shape` 同语义：绕中心顺时针）；
    `Painter::text` / `text_wrapped` 变成 `text_transformed(position, text, style,
    max_width, transform)` 的恒等封装；
  * tessellate 把每个字形 quad 绕**墨迹框中心**（该 run 全部字形矩形的并集中心）旋转——
    调用方只声明转多少度，枢轴由渲染器从内容推导；恒等变换走原路径（`rotate_about` 早退，
    未旋转文本的顶点逐位不变）；
  * `Label::rotation(degrees)`：**measure 预留旋转后的轴对齐外接框**
    （`w' = w·|cosθ| + h·|sinθ|`，`h'` 同理），**paint 把未旋转的文本块居中**放进盒子。
* **关键发现（裁剪模型）**：`PaintContext::paint_child` 把子节点裁到**自己的 bounds**——
  滚动与溢出都依赖它，是既有设计。因此旋转只在 paint 阶段做必然被裁：首版实测 17 个字符
  只剩中间 7 个（上下左右四边都缺）。**旋转必须在 measure 就预留外接框**，paint 再居中，
  使墨迹框中心与盒子中心重合。`wrap + rotation` 还需迭代求换行宽度（`fitting_wrap_width`：
  换行越窄 → 块越高 → 外接框越宽，无闭式解）；无解时退回 `limit`（同未旋转行为，接受裁剪）。
* **测试**：core `text_commands_carry_their_transform_and_width`、renderer
  `rotated_text_pivots_on_the_ink_box_centre` 与 `identity_text_keeps_the_glyph_corners`、
  widgets `a_rotated_label_ships_its_rotation_with_the_text`、label 单测 4 条
  （外接框 90° 换轴 / measure 钳制 / 换行宽度迭代收敛 / 无解回退）、黄金图第三场景
  `rotated_label`（真实 GPU 渲染，出图人工复核无裁剪）。

### C-050 `Dialog::min_width`：最小宽度可配置（T-7 关闭）

* **问题**：Dialog 宽度硬下限 `theme.control_height() * 4`（给标题栏与关闭按钮留空间），
  极窄对话框无法自定义，内容再窄也被撑开。
* **实现**：`Dialog::min_width(f32)` builder + `min_width: Option<f32>` 字段；`measure` 用
  `min_width.unwrap_or(控制高 × 4)` 作下限；内容宽于下限时内容优先，默认行为逐位不变。
* **测试**：widgets `a_dialog_can_override_its_default_minimum_width`
  （260 的下限压过内容、40 的下限不压缩 120 宽的内容）。

### C-051 圆角裁剪：矩形走 scissor，最内层圆角走片元 SDF（T-5 关闭）

* **问题**：`push_clip_rounded` 的 `radius` 只随命令携带，tessellate 时被丢弃——裁剪完全由
  矩形 scissor 执行，**圆角外、矩形内的四个角落会漏画内容**：4px 半径下约 1px 级误差，
  半径接近边长一半（圆形裁剪）时角落近 30% 面积漏画。
* **精确性论证**：裁剪栈的真实区域 = ∩平面(各矩形) ∩ ∩圆角(各圆角矩形)。圆角矩形 ⊆ 自己
  的矩形，所以**矩形部分可以全部折叠进 scissor 的交集**，圆角部分逐个作为 SDF 保留。于是
  "全部矩形交集（scissor）+ 一个圆角 SDF" 在**同时只有一个圆角裁剪活跃时是精确的**——普通
  矩形压在圆角之上同样精确（平面只收紧 scissor，不改弧线）。widget 给子节点推的正是普通
  矩形 clip，所以「圆角容器 + 内容」这一最常见的形态整条链路精确。
* **实现**：`Builder` 维护 `(scissor 交集, 最内层圆角的原始矩形, 半径)` 三元组栈
  （`ClipFrame`，`PushClip` 存 / `PopClip` 还原）；每个 `Instance` 携带 `clip: [f32;4]`
  （圆角矩形，物理像素）与 `params[1]`（半径），片元着色器
  `shape *= rounded_coverage(片元位置 - clip.xy, clip.zw, r)`；**半径为 0 时整条路径与改动前
  逐位一致**。多个圆角裁剪同时活跃时只保留**最内层**，外层退化为改动前的矩形近似——只会
  少约束、不会多画，相对旧行为零回归。
* **测试**：tess `a_rounded_clip_travels_with_every_instance`（半径与矩形逐实例携带、纯矩形
  clip 保持 0 半径）、`the_innermost_rounded_clip_wins_and_pop_restores_it`（内层胜出 /
  普通 clip 不清它 / pop 还原外层，scissor 始终是全交集）；离屏像素
  `a_rounded_clip_carves_the_corners`（48×48 半径 24 = 整圆，角上 (9,9) 必须是背景——
  旧的矩形裁剪会画成白色）。**三张黄金图逐像素未变**，既有场景零回归。

### C-052 四角半径：`Instance.radii` + 按象限取值的 SDF（T-6 关闭）

* **问题**：`EdgeRadius::max_side()` 把四个角压成一个半径——"上 12 下 0"的卡片被画成四角 12。
* **实现**：
  * `Instance` 增 `radii: [f32;4]`，**象限顺序 TL / TR / BL / BR**（与着色器取值一致，
    注意 `EdgeRadius` 的声明顺序是 TL/TR/BR/BL，构造数组时交换后两个）；`params` 改为
    `[mode, clip_radius, 保留, 保留]`；布局 **144 → 176 字节**，顶点属性增 `@location(11/12)`；
  * `rounded_coverage` 改收四半径，按片元所在**象限**取该角半径。可证与原单半径公式连续
    一致：矩形内部 `d = max(|dx|-半宽, |dy|-半高)` 与半径无关，半径只在角弧与边带里出现
    ——四块在中缝处无缝拼接，四角各自精确；
  * 边框环的内圈半径同样逐角计算（各角半径减去相交两边的 inset，原先是减全局最大 inset）。
* **影响面**：`shape` / `image` 改传 `corner_radii(...)`；`Instance::radius()` 保留为
  "四角最大值"的旧查询（既有断言不变），新增 `clip_radius()`。
* **测试**：tess `corner_radii_reach_the_instance_in_quadrant_order`（TL1/TR2/BR3/BL4 →
  `[1, 2, 4, 3]`）；离屏像素 `per_corner_radii_shape_the_quad`（右侧两角半径 8、左侧 0：
  左角像素必须是填充色——单半径会把它切掉，右角必须是背景）。

### C-053 复杂文种 shaping：`rustybuzz` + `unicode-bidi`（T-9 关闭）

* **问题**：`ab_glyph` 只能逐字形排布——没有连字（"ffi" 是三个字形）、没有 kerning
  （"AV" 不收紧）、阿拉伯文/希伯来文按逻辑序从左排、天城文等复杂文种不合字也不重排。
* **实现**（全部在 `rustui-text` 内部，其他 crate 零改动——正是 C-026 承诺的边界）：
  * 新增 `shape` 模块：`Shaper<'a>` 每次 layout 构建一次（`rustybuzz::Face` 借用字体字节，
    生命周期与布局请求一致，不能常驻引擎）；`FontRegistry` 保存字体原始 bytes
    （`add_font_data`，`load_face` 顺带捕获 collection 的 face index），只注册 `FontVec`
    的旧路径自动退回逐字形排布（`shape_by_char`，即改造前行为）；
  * `layout.rs` 重写为「按 `\\n` 切硬行 → 按 **cluster** 贪心折行（断点只落在字簇边界，
    空格后优先，超长词按字簇断——连字永远不会被拆开）→ 每个视觉行经 `unicode-bidi` 的
    `visual_runs` 分 run、逐 run 以各自方向整形 → 笔位从左到右排」；RTL run 在 shaper
    输出里已经是视觉序（实测 `אבג` 首逻辑字符落在最右），无需二次反转；
  * `LaidOutGlyph` 增 `char_index` / `char_count` / `rtl`：连字覆盖的字符区间对外可见，
    `caret_rect` 在簇内按比例插值（在连字里按方向键光标仍会移动），`hit_test_x` 按像素
    位置二分字簇并返回**字符索引**（不再隐含"字形序号 == 字符序号"）；
  * `ab_glyph` 保留为度量与光栅化后端，shaper 与它读同一张 `cmap` —— glyph id 一致性由
    测试 `shaped_glyph_ids_match_the_rasterizer` 钉住，atlas 管线不变。
* **依赖豁免**：`rustybuzz 0.20` + `unicode-bidi 0.3` 加入 `rustui-text`。G-001「底层仅
  winit + wgpu」约束的是平台与渲染层；文本整形是 `rustui-text` 的内部实现细节，不引入
  任何平台/图形耦合，crate 依赖面（widgets 只依赖 core/layout/text/model/animation）不变。
* **测试**：新增 `crates/rustui-text/tests/shaping.rs` 10 条 —— 连字塌缩与簇内 caret 插值、
  kerning 收紧、RTL 首逻辑字符最右、混排视觉序（拉丁在左、希伯来在右且内部反序）、
  shaper/光栅化 glyph id 一致、换行不拆簇（字形数与不换行一致）、hit-test 簇二分、
  天城文重排（6 字符 < 6 字形 + 零宽 matra）、阿拉伯语境合字与 lam-alef 连字、
  无字节 `FontVec` 回退；DejaVu/Noto 字体缺失时对应测试自动跳过。黄金图三张按新排版
  重建（23/51/16 个格子的文字像素微移，结构不变，已逐张人工比对）。

### C-054 系统剪贴板：`arboard` 接入 platform（T-3 关闭）

* **问题**：C-037 的 MVP 剪贴板是进程内缓冲，跨应用复制粘贴不可用。
* **实现**：`PlatformClipboard` 内部改
  `Backing::{System(arboard::Clipboard), Memory(Option<String>)}` ——
  * `new()` 打开系统剪贴板（Linux 走 X11/XWayland；arboard 自带 selection 服务线程，
    进程存活期间可响应外部客户端的粘贴读取），打开失败（无显示服务器、纯 Wayland 无
    XWayland、被占用）**降级**为进程内缓冲并打日志；系统写失败同样降级、内存接管；
  * `in_memory()` 直接构造降级态，供测试与无头模式；`is_system()` 暴露当前后端；
  * facade 注入点不变（`app.rs` 仍 `PlatformClipboard::new()`），`Clipboard` trait 与
    widgets 零改动——trait 在 core 的设计（C-037）就是为这次替换准备的。
* **依赖豁免**：`arboard 3`（`default-features = false`：只要文本通路，不要 image 特性）
  **只允许出现在 `rustui-platform`** —— 平台边界 crate 本就是 winit/wgpu 之外唯一允许
  接触系统 API 的层；`rustui-core`/widgets 仍只见 `Clipboard` trait。未启用
  `wayland-data-control`（wl-clipboard）：主流 GNOME 的 mutter 不支持该协议，XWayland
  路径覆盖本机场景；纯 Wayland 无 XWayland 时降级为进程内缓冲，不劣于改造前。
* **测试**：`clipboard.rs` 3 条 —— 降级态 round-trip 与克隆共享、`new()` 永不 panic、
  **跨进程**系统 round-trip：测试用 `RUSTUI_CLIPBOARD_CHILD_READ` 环境变量把自己重新执行
  一遍，子进程作为独立 X11 客户端读取父进程写入的文本（真跨进程粘贴），结束后恢复用户
  原剪贴板内容；无 `DISPLAY` 时整段自动跳过（`env -u DISPLAY` 实测走降级分支）。

---

# 11. 已知限制与 TODO

**开放项：无 —— T-1 ~ T-11 全部关闭（2026-10-09）。**

| # | 限制 | 影响 | 计划 |
| --- | --- | --- | --- |
| — | （无） | — | — |

**已关闭**：

* **T-9 复杂文种 shaping —— 2026-10-09 完成**（见 C-053）：`rustybuzz` 整形（连字、
  kerning、语境合字与重排）+ `unicode-bidi` 视觉序换行，断行只在字簇边界；`LaidOutGlyph`
  携带字符区间，`caret_rect` 簇内插值、`hit_test_x` 按簇二分返回字符索引；`ab_glyph` 保留
  为度量与光栅化后端（同一张 cmap）；10 条新集成测试，黄金图三张重建并逐张比对。

* **T-3 系统剪贴板 —— 2026-10-09 完成**（见 C-054）：`PlatformClipboard` 用 `arboard`
  接系统剪贴板（X11/XWayland），打开或写入失败降级回进程内缓冲；含跨进程子客户端粘贴
  测试；依赖豁免（arboard 只进 `rustui-platform`）见 C-054。

* **T-6 四角半径 —— 2026-10-09 完成**（见 C-052）：`Instance.radii` 按象限携带四个角的
  半径，着色器按片元所在象限取值（内部距离与半径无关，四块无缝拼接）；`max_side()` 不再
  参与绘制，边框环内圈半径也逐角计算；布局 144 → 176 字节。

* **T-5 圆角裁剪 —— 2026-10-09 完成**（见 C-051）：矩形部分全部折叠进 scissor，最内层
  圆角逐片元 SDF —— 同时只有一个圆角裁剪活跃时精确（普通矩形压在其上也精确）；多圆角
  同时活跃只保留最内层，不劣于旧行为；半径 0 的路径与改动前逐位一致，三张黄金图未变。

* **T-8 文本变换 —— 2026-10-08 完成**（见 C-049）：`PaintCommand::Text.transform` +
  `Painter::text_transformed` + tessellate 绕墨迹框中心旋转 + `Label::rotation`；旋转在
  measure 预留外接框（`paint_child` 把子节点裁到自身 bounds，只在 paint 转会被裁），
  `wrap + rotation` 迭代求换行宽度；黄金图加第三场景 `rotated_label`。

* **T-7 Dialog 最小宽度 —— 2026-10-08 完成**（见 C-050）：`Dialog::min_width(f32)`
  builder 覆盖默认的 `control_height * 4`，内容更宽时仍以内容为准，缺省行为不变。

* **T-2 多窗口 DPI —— 2026-10-08 完成**（见 C-048）：字形缓存改按**物理像素尺寸**键控
  （`GlyphKey::at_scale`），`set_scale_factor` 不再清缓存，`render_input` 组装输入时把引擎
  切到该窗口的 scale；1x/2x 窗口的光栅并存、切换零成本，facade 的 `sync_engine_scale` 删除。

* **T-11 基准测试 —— 2026-10-08 完成**（见 C-047）：`crates/rustui/benches/pipeline.rs`
  用 criterion 覆盖 text / layout / paint / hit_test / frame / table(10k,100k) / widget
  七组 CPU 热路径；GPU 指标因本机为软件光栅化暂不测（§59 未竟部分）。

* **T-10 无头渲染 + 黄金图回归 —— 2026-10-08 完成**（见 C-046）：`SurfaceRenderer`
  新增 `Target::Offscreen` / `new_offscreen` / `read_pixels`，测试直接把真实场景渲到纹理再
  读回像素，与 `tests/golden/*.grid` 比对（`UPDATE_GOLDEN=1` 重建）；顺带修掉"首帧 atlas
  未上传 → 首帧无文字"的真实缺陷。人工截图仅剩跨改动的抽查。

* **T-4 ComboBox 弹层方向与可见性 —— 2026-10-08 完成**（见 C-045）：弹层改走窗口的
  **弹层通道**（`Widget::paint_popup`）在整棵树之后绘制、只受窗口裁剪，并按
  `Interaction.viewport` 在下方放不下时翻到上方。
* **T-1 IME cursor area —— 2026-10-08 完成**（见 C-038）：`Widget::ime_cursor_area` →
  `WindowManager::ime_area` → facade `sync_ime` → `WindowHandle::set_ime_cursor_area`；
  候选框跟随 caret，未聚焦时输入法关闭。回归测试
  `the_input_method_follows_the_caret_and_the_focus`（rustui-widgets）与
  `the_input_method_area_is_asked_of_the_focused_widget`（rustui-window）。

---

# 12. 修订历史

| 日期 | 内容 |
| --- | --- |
| 2026-10-07 | 初版：汇总对象模型、事件、布局、坐标、文本渲染、模型、facade、异步与测试约定的全部修正；新增 C-017 / C-018 / C-020 / C-021 / C-024 / C-026 / C-044（本轮渲染与布局缺陷的根因修复）。 |
| 2026-10-07 | 两份设计文档就地修正：`Detailed` §2/§3/§4/§6/§8/§9/§10/§11/§12/§13/§14/§15/§19/§20/§21/§22/§23/§24/§25/§27/§30/§31/§32/§38/§39/§44/§56/§58/§61，`High-Level` §8/§9/§11/§13/§14/§17/§19 全部改为与实现一致并标注 C 编号；TODO 表新增 T-9（无复杂文种 shaping）/ T-10（截图回归人工）/ T-11（无 benchmark）。 |
| 2026-10-08 | 实现 T-1（IME 候选框跟随 caret）：新增 `Widget::ime_cursor_area` / `WindowManager::ime_area` / `WindowHandle::set_ime_cursor_area` 与 facade `sync_ime`；C-038 由 TODO 改为实现说明，T-1 关闭；`Detailed` §6/§26/§40 同步更新，两条新回归测试。 |
| 2026-10-08 | 实现 T-4（ComboBox 弹层）：新增 C-045 弹层通道（`Widget::paint_popup` + `PaintContext::request_popup` / `paint_popup` + `OverlayList` 弹层队列）与 `Interaction::viewport`（`viewport()` 访问器）；修复弹层被父 clip 裁掉、被后兄弟覆盖、不翻转三个缺陷；`Detailed` §6/§19/§32 同步更新，T-4 关闭，五条新回归测试；`High-Level` 测试计数同步为 419。 |
| 2026-10-08 | 实现 T-10（无头渲染 + 黄金图）：新增 C-046 —— `SurfaceRenderer::Target::{Surface,Offscreen}` / `new_offscreen` / `read_pixels`，`render` 改为 tessellate → sync_atlas（修首帧无文字缺陷）；`Detailed` §22/§58 同步更新，T-10 关闭，五条新测试（3 离屏 + 2 黄金图），`High-Level` 测试计数同步为 424。 |
| 2026-10-08 | 实现 T-11（基准测试）：新增 C-047 —— `crates/rustui/benches/pipeline.rs`（criterion 0.8，`harness = false`）覆盖 text / layout / paint / hit_test / frame / table(10k, 100k) / widget 七组 CPU 热路径并记录基线；T-11 关闭，`Detailed` §59 与 `High-Level` §19 同步更新；GPU 指标因本机软件光栅化暂缓。 |
| 2026-10-08 | 实现 T-2（多窗口 DPI）：新增 C-048 —— `GlyphKey::at_scale` 按物理尺寸键控字形缓存、`set_scale_factor` 不再清缓存、`WindowManager::render_input` 成为唯一 scale 设置点（删除 facade `sync_engine_scale` 与 `State::engine_scale`）；C-028 限制段改记为已消除，T-2 关闭，三条新回归测试（1 集成 + 2 单测），`Detailed` §25/§32/§41/§58/§61 与 `High-Level` §19 同步更新，测试计数 426 / 31 目标。 |
| 2026-10-08 | 实现 T-8（文本变换）与 T-7（Dialog 最小宽度）：新增 C-049 —— `PaintCommand::Text.transform` + `Painter::text_transformed` + tessellate 绕墨迹框中心旋转 + `Label::rotation`（measure 预留旋转外接框、paint 居中；根因：`paint_child` 把子节点裁到自身 bounds，旋转只在 paint 做会被裁，实测 17 字只剩 7 字），新增 C-050 —— `Dialog::min_width`；T-7/T-8 关闭，10 条新测试（1 core + 2 renderer + 2 widgets + 4 label 单测 + 1 黄金图场景 `rotated_label`），`Detailed` §19/§47/§58/§61 与 `High-Level` §19 同步更新，测试计数 436 / 31 目标。 |
| 2026-10-09 | 实现 T-5（圆角裁剪）与 T-6（四角半径）：新增 C-051 —— 裁剪栈的矩形部分全部折叠进 scissor，最内层圆角逐片元 SDF（同时只有一个圆角裁剪活跃时精确，普通矩形压在其上也精确；半径 0 路径与改动前逐位一致），新增 C-052 —— `Instance.radii` 按象限顺序逐角携带半径、`rounded_coverage` 按片元象限取值（内部距离与半径无关，四块无缝拼接），布局 144 → 176 字节；C-022/C-023 限制段改记为已消除，T-5/T-6 关闭，5 条新测试（3 tess + 2 离屏像素断言），三张黄金图逐像素未变，`Detailed` §19/§58/§61 与 `High-Level` §19 同步更新，测试计数 441 / 31 目标。 |
| 2026-10-09 | 实现 T-9（复杂文种 shaping）与 T-3（系统剪贴板），**TODO 表清空**：新增 C-053 —— `rustybuzz` 整形 + `unicode-bidi` 视觉序换行 + 字簇级断行/光标/hit-test（`LaidOutGlyph` 增 `char_index`/`char_count`/`rtl`，`ab_glyph` 保留为度量与光栅化后端、同一张 cmap；rustybuzz/unicode-bidi 进 `rustui-text` 的依赖豁免），新增 C-054 —— `PlatformClipboard` 用 `arboard` 接系统剪贴板、失败降级进程内缓冲（arboard 只进 `rustui-platform` 的依赖豁免，含跨进程子客户端粘贴测试）；C-037 限制段改记为已消除，T-9/T-3 关闭，13 条新测试（10 shaping + 1 布局 + 2 剪贴板），黄金图三张按新排版重建（仅文字像素微移、结构不变，逐张比对），`Detailed` §25/§39/§58/§59/§61 与 `High-Level` §19 同步更新，测试计数 454 / 32 目标。 |
