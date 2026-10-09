# rustui-layout 模块详细设计

> 所属体系：[架构设计文档](../Architecture_Design_Document.md) · 分层：第 2 层（布局/数据层）
> 源码：`crates/rustui-layout/src/` · 依赖：仅 rustui-core

本模块对应[详细设计文档](../Detailed_Design_Document.md) §15（Layout）、§16（VBox）、
§17（HBox）、§18（Grid），并落实[设计修正](../Design_Corrections.md) C-005、C-014 ~ C-020、
C-043 的布局约定。

## 1. 模块职责

`rustui-layout` 提供 RustUI 的全部**布局原语**：每个类型都是一个普通的
`rustui_core::Widget`，只依赖 `rustui-core`，因此可以被 widget、示例与测试直接使用，
不必引入平台层或渲染层。它承担四件事：

1. **容器算法**：把「父容器能给多少空间」转换成「每个子项的期望尺寸与位置」。
   - 一维：`VBox`（自上而下）/ `HBox`（自左而右），共享 `BoxLayout` 配置结构；
   - 二维：`Grid`（`Auto` / `Px` / `Fraction` 轨道 + 间距 + 跨行跨列）；
   - 叠放：`Stack`（同一盒子内重叠，后绘制者在上）；
   - 流式：`Flow`（水平流动、按容器宽度换行）；
   - 单子项：`Container`（padding + background + border + radius + clip + 对齐）；
   - 辅助：`Spacer`（柔性/固定间隙）、`Separator`（细分隔线）。
2. **两阶段布局**：`measure(available)` 只拿到父容器能提供的空间，返回期望尺寸并写入
   `WidgetBase::measured`；`arrange(rect)` 从不重新测量，只把已测得的尺寸分配到局部
   rect 中，写入 `WidgetBase::bounds` 与 `window_bounds`。
3. **装饰绘制**：`ContainerStyle` 统一提供背景、边框、圆角与裁剪，
   `paint_begin` / `paint_end` 成对出现在每个容器的 `paint` 首尾。
4. **坐标约定**：`arrange` 接收的 rect 是「在父容器内」的矩形，widget 只保存
   **局部** bounds（原点恒为 `(0, 0, width, height)`）；窗口坐标单独存放在
   `window_bounds`，`paint` 用它平移子项（[设计修正](../Design_Corrections.md) C-020
   坐标双空间约定）。

三遍流水线（crate 根文档 `lib.rs` 给出的官方表述）：

```text
measure(available) -> desired size, stored in `WidgetBase::measured`
arrange(rect)      -> local bounds + `window_bounds` for every child
paint(ctx)         -> decoration first, then the children
```

## 2. 模块结构（文件表）

| 文件 | 内容 |
|------|------|
| `crates/rustui-layout/src/lib.rs` | crate 文档（容器一览、三遍流水线、坐标约定）、模块声明、`prelude` |
| `crates/rustui-layout/src/style.rs` | 所有布局 widget 共享的工具：`Axis`、`ContainerStyle`、`content_rect` / `content_available` / `clamp_to` / `apply_fill` / `expands` / `measured_size` / `hint_of` / `visible_children` / `paint_children` |
| `crates/rustui-layout/src/boxes.rs` | `BoxLayout`（measure / arrange / paint 共享算法）、`define_box!` 宏生成的 `VBox` / `HBox`、`Spacer`、`Separator` |
| `crates/rustui-layout/src/container.rs` | `Container`：单子项 + padding + 装饰 + 水平/垂直对齐 |
| `crates/rustui-layout/src/grid.rs` | `Grid`、`Track`（`Auto` / `Px` / `Fraction`）、私有 `Placement`、`plan` / `resolve` / `offsets` 轨道算法 |
| `crates/rustui-layout/src/stack.rs` | `Stack`：子项在同一内容框内叠放，默认 `Stretch` 拉满 |
| `crates/rustui-layout/src/flow.rs` | `Flow`、私有 `Line`：水平流动 + 按宽度换行 |
| `crates/rustui-layout/tests/layout.rs` | 端到端布局测试：自备 `Leaf` / `StubText` / `Tree` 测试骨架，18 个 `#[test]` |

## 3. 核心类型设计

### 3.1 类图

```mermaid
classDiagram
    direction TB
    class WidgetBase {
        <<rustui-core>>
        +measured Size
        +bounds Rect
        +window_bounds Rect
        +set_measured(size Size)
        +set_bounds(rect Rect)
        +children() ObjectId[]
        +add_child(id ObjectId)
    }
    class Axis {
        <<enum>>
        Vertical
        Horizontal
        +main(size Size) f32
        +cross(size Size) f32
        +size(main f32, cross f32) Size
        +is_vertical() bool
    }
    class ContainerStyle {
        +background Color
        +border_color Color
        +border_width f32
        +corner_radius f32
        +clip bool
        +EMPTY ContainerStyle
        +paint_begin(ctx, bounds)
        +paint_end(ctx, bounds)
        +push_clip(ctx, bounds)
        +pop_clip(ctx)
    }
    class BoxLayout {
        +spacing f32
        +padding Insets
        +main_align MainAxisAlign
        +cross_align CrossAxisAlign
        +main_axis_size MainAxisSize
        +style ContainerStyle
        +hint SizeHint
        -align_self Map
        -flex Map
        +set_align_of(child, align)
        +set_flex_of(child, weight)
        +measure(axis, children, ctx, available) Size
        +arrange(axis, children, ctx, rect)
        +paint(children, ctx, bounds)
    }
    class VBox {
        +base WidgetBase
        +layout BoxLayout
        +with_child(id) VBox
        +with_spacing(spacing) VBox
        +push(id)
    }
    class HBox {
        +base WidgetBase
        +layout BoxLayout
        +with_child(id) HBox
        +with_main_align(align) HBox
        +push(id)
    }
    class Grid {
        +base WidgetBase
        +columns Track[]
        +rows Track[]
        +spacing Size
        +align (Align, Align)
        -placements Map
        +place_child(id, row, column)
        +place_span(id, row, column, row_span, column_span)
        +plan(children) (Track, Track, items)
        +resolve(tracks, spanned, available, spacing) f32[]
        +measure(ctx, available) Size
        +arrange(ctx, rect)
    }
    class Track {
        <<enum>>
        Auto
        Px(f32)
        Fraction(f32)
        +AUTO Track
        +px(size f32) Track
        +fr(fraction f32) Track
    }
    class Stack {
        +base WidgetBase
        +padding Insets
        +align (Align, Align)
        +with_align(h, v) Stack
        +measure(ctx, available) Size
        +arrange(ctx, rect)
    }
    class Flow {
        +base WidgetBase
        +spacing Size
        +align (Align, Align)
        +lines(sizes, max_width) Line[]
        +measure(ctx, available) Size
        +arrange(ctx, rect)
    }
    class Container {
        +base WidgetBase
        +child ObjectId
        +padding Insets
        +align (Align, Align)
        +child(id) Container
        +with_align(h, v) Container
        +with_fill() Container
        +measure(ctx, available) Size
        +arrange(ctx, rect)
    }
    class Spacer {
        +base WidgetBase
        +weight f32
        +fixed Option~Size~
        +new(weight) Spacer
        +fixed(size) Spacer
    }
    class Separator {
        +base WidgetBase
        +axis Axis
        +thickness f32
        +horizontal() Separator
        +vertical() Separator
    }
    VBox *-- WidgetBase : base
    HBox *-- WidgetBase : base
    Grid *-- WidgetBase : base
    Stack *-- WidgetBase : base
    Flow *-- WidgetBase : base
    Container *-- WidgetBase : base
    Spacer *-- WidgetBase : base
    Separator *-- WidgetBase : base
    VBox *-- BoxLayout : layout
    HBox *-- BoxLayout : layout
    Grid --> Track : columns / rows
    Flow *-- Line : lines
    ContainerStyle <.. BoxLayout : style
    ContainerStyle <.. Grid : style
    ContainerStyle <.. Stack : style
    ContainerStyle <.. Flow : style
```

> 说明：`VBox` / `HBox` 由 `boxes.rs` 中的 `define_box!` 宏生成，二者只在 `Axis` 常量上
> 不同；模块中**没有**名为 `FlexBox` 的类型，"弹性盒子"的能力来自 `BoxLayout` 的
> `flex` 权重表与子项 `SizeHint::flex`。

### 3.2 类型说明表

| 类型 | 文件 | 说明 |
|------|------|------|
| `Axis` | `style.rs` | `Vertical` / `Horizontal`；`main` / `cross` / `size` 在主轴与交叉轴之间换算尺寸 |
| `ContainerStyle` | `style.rs` | 装饰五元组（背景色、边框色、边框宽、圆角、是否裁剪）；`EMPTY` / `Default` 表示什么都不画；`paint_begin` = 背景 + 推裁剪，`paint_end` = 弹裁剪 + 描边 |
| `BoxLayout` | `boxes.rs` | `VBox` / `HBox` 的共享配置与算法：`spacing`、`padding`、`main_align`、`cross_align`、`main_axis_size`、`hint`，以及私有的 `align_self` / `flex` 两张按子项覆盖表；`set_flex_of(child, 0)` 移除覆盖 |
| `VBox` / `HBox` | `boxes.rs` | 一维容器，`measure` / `arrange` / `paint` 全部委托给 `BoxLayout`，仅传入固定 `Axis` |
| `Spacer` | `boxes.rs` | 柔性间隙：`SizeHint { flex: weight, ..SizeHint::fill() }`，`measure` 返回 `Size::ZERO`（或 `fixed(size)` 的固定值），不绘制任何内容 |
| `Separator` | `boxes.rs` | 分隔线：横线（用于 `VBox`）报 `width: ∞`、`height: thickness`，纵线相反；主轴永远固定为线宽，因此不会吃掉自由空间 |
| `Grid` + `Track` | `grid.rs` | 轨道：`Auto`（取该轨道最大项）、`Px(f32)`（固定）、`Fraction(f32)`（瓜分剩余空间）；`with_spacing` / `with_gaps` 设行列间距；`place_child` / `place_span` 显式定位，未定位子项按行优先自动流布 |
| `Placement`（私有） | `grid.rs` | `(row, column, row_span, column_span, h, v)`，由 `plan` 统一产出，measure 与 arrange 共用同一份几何规划 |
| `Stack` | `stack.rs` | 所有可见子项测量同一个内容框，`arrange` 用同一 `align`（默认 `(Stretch, Stretch)`）摆放，后绘制的子项盖在上面 |
| `Flow` + `Line`（私有） | `flow.rs` | `Line { items, width, height }` 是换行结果；`align.0` 决定行在容器内的水平对齐，`align.1` 决定子项在行内的垂直对齐（默认 `(Start, Center)`） |
| `Container` | `container.rs` | 单子项容器：`child(id)` 是 builder 方法（同时 `add_child`），`with_align(h, v)`、`with_fill()` 设置自身 `SizeHint::fill()`；`is_opaque` 取决于背景 alpha |

## 4. 主要流程

### 4.1 两阶段布局：measure → arrange

```mermaid
flowchart TD
    A["Window::layout 构造 MeasureContext"] --> B["measure_child root 传入 viewport.size"]
    B --> C["容器 Widget::measure 收到 available"]
    C --> D["content_available 扣除 padding 无限分量保持无限"]
    D --> E["visible_children_in 过滤不可见子项"]
    E --> F["逐个 measure_child 子项"]
    F --> G["累计主轴与交叉轴 使用量"]
    G --> H["clamp_to 夹到 available 再 apply_fill 收尾"]
    H --> I["set_measured 写入 WidgetBase"]
    I --> J{"还有未测子项"}
    J -- 是 --> F
    J -- 否 --> K["根尺寸返回给 Window"]
    K --> L["构造 ArrangeContext offset 为 0"]
    L --> M["arrange_child root 传入 viewport"]
    M --> N["set_bounds 写局部 bounds"]
    N --> O["从 store 读回 measured_size 分配内容框"]
    O --> P["align_child 计算子项 rect"]
    P --> Q["arrange_child 子项 递归进入子树"]
    Q --> R["set_window_bounds 平移为窗口坐标"]
    R --> S["measured 与上一轮不同则回调 layout_changed"]
    S --> T["paint 阶段按 window_bounds 平移绘制"]
```

要点：

- `measure` 从来看不到 rect，只拿到父容器「还剩多少」的可用空间；`arrange` 从不重新
  测量（arrange 上下文背后没有文本引擎），子项尺寸一律通过 `style::measured_size`
  从 `WidgetStore` 读回。
- 约束向下（`available` 逐层收紧）与 hint 向上（`SizeHint::flex`、无限尺寸偏好
  `f32::INFINITY` 向上冒泡为「我要更多空间」）在同一遍里完成：`expands(hint, axis)`
  判定 `flex > 0.0 || 偏好无限`，命中者进入柔性组参与分池。

### 4.2 VBox 一次布局的轨道分配（`BoxLayout::measure` + `arrange`）

```mermaid
flowchart TD
    M["BoxLayout::measure axis=Vertical"] --> M1["visible_children_in 过滤不可见子项"]
    M1 --> M2["content_available 扣除 padding"]
    M2 --> M3["按 flex_of 与 expands 分组"]
    M3 --> M4["刚性子项先测 各自看到剩余主轴"]
    M4 --> M5["进入柔性组时记录 flex_pool 为剩余空间"]
    M5 --> M6["柔性子项按权重瓜分 flex_pool 互不饿死"]
    M6 --> M7["main_used 累计 spacing cross_used 取最大值"]
    M7 --> M8{"MainAxisSize 为 Fill 且 available 有限"}
    M8 -- 是 --> M9["主轴改取 available"]
    M8 -- 否 --> M10["保留内容合计"]
    M9 --> M11["expand padding 后 clamp_to 与 apply_fill"]
    M10 --> M11
    M11 --> M12["set_measured 写回期望尺寸"]
    M12 --> A["BoxLayout::arrange 读回 measured_size"]
    A --> A1["used 为各子项主轴合计加 gaps free 为剩余"]
    A1 --> A2{"存在柔性子项"}
    A2 -- 是 --> A3["pool 等于内容主轴减刚性合计减 gaps 后 distribute 按权重覆盖主轴尺寸"]
    A2 -- 否 --> A4["free 保持原值"]
    A3 --> A5["free 置 0"]
    A4 --> A5
    A5 --> A6["按 main_align 计算 start_offset 与 gap_extra"]
    A6 --> A7["cursor 逐个推进 cell 用 align_child 摆放子项"]
    A7 --> A8["arrange_child 写回每个子项的 bounds"]
```

- 第 1 步（measure 内的柔性组）只是「预分配」，让同排两个 flex 子项互相不饿死；
  **权威分配在第 2 步（arrange）**：整行已知后 `distribute(pool, weights)` 覆盖柔性
  子项的主轴尺寸，测量值此时只决定交叉轴（[设计修正](../Design_Corrections.md) C-017）。
- `main_align` 的六种取值（`Start` / `Center` / `End` / `SpaceBetween` /
  `SpaceAround` / `SpaceEvenly`）只在**没有**柔性子项吃掉空间时才产生可见效果。

### 4.3 Grid 轨道尺寸解析（`Grid::resolve`）

```mermaid
flowchart TD
    R["Grid::resolve tracks, spanned, available, spacing"] --> R1["第 1 步 Px 轨道直接取固定值"]
    R1 --> R2["span 为 1 的子项把 Auto 轨道顶到其自然尺寸"]
    R2 --> R3["第 2 步 跨多轨子项计算 deficit 等于自然尺寸减覆盖合计"]
    R3 --> R4{"deficit 大于 0 且覆盖了 Auto 轨道"}
    R4 -- 是 --> R5["缺口均摊到被覆盖的 Auto 轨道"]
    R4 -- 否 --> R6["跳过"]
    R5 --> R6
    R6 --> R7["第 3 步 合计 Fraction 权重"]
    R7 --> R8{"存在 Fraction 轨道"}
    R8 -- 是 且 available 有限 --> R9["free 为 available 减非分数轨道减 gaps 后按权重切分"]
    R8 -- 是 且 available 无限 --> R10["分数轨道取覆盖它的子项自然尺寸最大值"]
    R8 -- 否 --> R11["返回 sizes"]
    R9 --> R11
    R10 --> R11
```

`plan` 在 measure 与 arrange 中被**同样调用**，因此两遍得到的行列规划完全一致：
显式 `place_child` / `place_span` 优先，其余子项按行优先游标自动流布，`rows` 不足时
用 `Track::Auto` 补齐。

## 5. 关键时序

一次 `Window::layout` 从根遍历到叶子（`crates/rustui-window/src/window.rs` 发起，
`MeasureContext::measure_child` / `ArrangeContext::arrange_child` 递归，见
`crates/rustui-core/src/context.rs`）：

```mermaid
sequenceDiagram
    participant W as Window
    participant M as MeasureContext
    participant V as VBox
    participant C as Container
    participant L as 叶子 Leaf
    participant A as ArrangeContext
    W->>M: MeasureContext::new ui, store, interaction
    M->>V: measure_child 传入 viewport.size
    V->>V: content_available 扣除 padding
    V->>C: measure_child 传入剩余主轴与全交叉轴
    C->>L: measure_child 传入 content_available
    L-->>C: 返回自然尺寸
    C->>C: expand padding 后 clamp_to 与 apply_fill 并 set_measured
    C-->>V: 返回子项尺寸
    V->>V: 累计主轴并加 spacing 后 set_measured
    V-->>M: 根尺寸返回
    M-->>W: 根 measured 得到
    W->>A: ArrangeContext::new offset 为 0
    A->>V: arrange_child 传入 viewport
    V->>V: set_bounds 写局部 bounds
    V->>A: arrange_child 子项 传入内容框内 rect
    A->>C: arrange 收到父空间内 rect
    C->>A: arrange_child 按 align_child 的结果摆放
    A->>L: arrange 收到 rect
    A->>L: set_window_bounds 平移累加 offset
    A->>L: measured 变化则回调 layout_changed
```

约定：`arrange_child` 里的 `offset` 是「本 widget 原点的窗口坐标」，子项把自己的局部
rect 平移后写入 `window_bounds`；这正是嵌套容器中
`ctx.fill_rect(bounds, color)` 无论嵌套多深都能画对的原因（测试
`nested_paint_uses_window_coordinates` 覆盖该性质）。

## 6. 状态转换

本 crate **没有持久的显式状态机**——所有布局 widget 都是无状态的纯算法容器。下图刻画
的是单帧内一个节点的**几何值生命周期**（`WidgetBase` 字段的写入时序），属于帧内状态
而非跨帧状态机：

```mermaid
stateDiagram-v2
    [*] --> 未布局
    未布局 --> 已测量 : measure 中 set_measured
    已测量 --> 已定位 : arrange 中 set_bounds 加 set_window_bounds
    已定位 --> 已测量 : 下一帧 measure 覆盖 measured
    未布局 --> 隐藏跳过 : set_visible false
    隐藏跳过 --> 已测量 : 恢复可见后重新 measure
```

- 「隐藏跳过」：`visible_children` 在 measure / arrange / paint 三遍里使用同一过滤，
  不可见子项的 bounds 保持 `Rect::ZERO`（测试
  `invisible_children_are_skipped` 断言）。
- arrange 期间若 `measured` 与上一轮不同，`ArrangeContext::arrange_child` 会回调
  `Widget::layout_changed(old, new)`，widget 由此得知自身尺寸变化。

## 7. 关键设计约束与不变式

引用编号来自 [Design_Corrections.md](../Design_Corrections.md) 与
[Detailed_Design_Document.md](../Detailed_Design_Document.md) §15 ~ §18。

- **C-005 容器即 layout widget**：布局不是独立 trait，每个布局 widget 自己实现
  `Widget::measure` / `Widget::arrange`，公共算法下沉到 `BoxLayout` / `Grid` /
  `Stack` / `Flow` 的方法上（`impl rustui_core::Layout for ...` 只是空标记实现）。
- **C-014 `MainAxisSize` 只有 `Auto` / `Fill` / `Fixed`**，无 `Expand`；
  `MainAxisSize::Fill` 仅在 `measure` 中把主轴改取 `available`。
- **C-015 `CrossAxisAlign` 默认 `Start`**：交叉轴默认不拉伸，子项保持自然交叉尺寸。
- **C-016 `apply_fill` 是 measure 的统一收尾**：`clamp_to` 先夹到 `available`，
  再由 `apply_fill` 处理「hint 无限 = 吃满 offered」；`content_available` 保持无限
  分量为无限，让自由贴合内容的容器把这份自由继续下传。
- **C-017 box 测量：刚性先行，柔性按权重分池**；arrange 阶段 `distribute` 的覆盖值
  才是柔性子项的权威主轴尺寸。
- **C-018 填充型 widget 必须在 `size_hint` 里声明扩展**：`measure` 会吃满 `available`
  的 widget 必须让 `size_hint` 与之匹配，否则被第 1 步判成刚性而饿死兄弟
  （对照 `Separator`：它只在交叉轴报 `∞`，主轴固定为线宽，因此永远不参与抢空间）。
- **C-019 `Container::child` 是 builder**，会同时 `add_child`；`with_child` 是关联函数。
- **C-020 坐标双空间约定**：widget 只存局部 bounds，窗口坐标只存 `window_bounds`，
  `paint` 用后者平移子项。
- **C-043 几何断言必须在 layout 之后**：测试里先 `layout(available, rect)` 再断言
  `window_bounds`。

模块级不变式（P 原则，本 crate 自身约定）：

- **P-L1 arrange 不复测**：`arrange` 上下文没有文本引擎，子项尺寸一律经
  `measured_size` / `measured_size_in` 从 store 读回并用 `SizeHint::apply` 夹紧。
- **P-L2 三遍一致过滤**：`visible_children` 同时服务 measure / arrange / paint，
  保证三遍看到同一组子项。
- **P-L3 所有 measure 都以 `apply_fill(clamp_to(...), available, hint)` 收尾**，
  返回值写入 `set_measured` 后即为该 widget 的期望尺寸。
- **P-L4 局部坐标原点**：`set_bounds(rect)` 后 `bounds` 恒为
  `(0.0, 0.0, rect.width, rect.height)`，子项几何在父的局部空间计算。
- **P-L5 空间非负**：padding 扣除、free space、跨轨缺口一律 `.max(0.0)`，
  `Grid::place_span` 的 span 一律 `.max(1)`。
- **P-L6 measure 与 arrange 共用同一规划**：`Grid::plan` / `Flow::lines` 在两遍中
  调用同一函数，几何不可能不一致。

## 8. 测试布局

`crates/rustui-layout/tests/layout.rs` 是端到端布局测试（共 18 个 `#[test]`），
自带三层骨架：

- **测试叶子** `Leaf`：固定自然尺寸，`measure` 直接返回 `size` 并 `set_measured`，
  `arrange` 只 `set_bounds`，`paint` 填充蓝色矩形；
- **桩文本** `StubText`：`TextMeasurer` 实现，每字符宽 7.0、行高 14.0，供
  `UiContext::new` 使用；
- **测试树** `Tree`：组合 `UiContext` / `WidgetStore` / `ObjectRegistry` /
  `Interaction`，提供 `empty` / `set_root` / `insert` / `add` / `measure` /
  `arrange` / `layout` / `bounds` / `measured` 快捷方法，断言助手 `assert_rect`
  允许 0.01 的浮点容差。

| 分组 | 测试 | 覆盖要点 |
|------|------|----------|
| VBox / HBox | `vbox_stacks_children_with_spacing_and_padding` | 间距 + padding 下的 measured 与逐子项位置 |
| | `vbox_cross_align_stretch_fills_the_content_width` | `CrossAxisAlign::Stretch` 拉满内容宽度 |
| | `hbox_gives_free_space_to_flex_children` | `Spacer::new(1.0)` 吃掉 120px 剩余 |
| | `free_space_is_distributed_by_main_axis_align` | `MainAxisAlign::Center` 的 60px 偏移 |
| | `measure_is_clamped_to_the_available_space` | 超大子项被 `clamp_to` 夹住 |
| | `invisible_children_are_skipped` | 隐形子项跳过且 bounds 为 `Rect::ZERO` |
| | `separator_is_a_line_and_never_absorbs_free_space` | 分隔线横跨内容宽但不吸收高度 |
| Grid | `grid_splits_fixed_and_fractional_tracks` | `px(50)` + `fr(1)` 的轨道切分 |
| | `grid_auto_placement_flows_row_by_row` | 行优先自动流布与 Auto 列宽取最大 |
| | `grid_spanning_item_uses_the_whole_row` | `place_span` 跨列项撑满整行 |
| Stack / Flow / Container | `stack_overlaps_children_in_the_same_box` | 默认 Stretch 叠放 |
| | `stack_alignment_keeps_natural_size` | `(End, End)` 对齐保持自然尺寸 |
| | `flow_wraps_into_lines` | 120px 宽下第三个子项换行 |
| | `container_pads_and_aligns_its_child` | padding + 背景 + 边框 + 圆角 + 裁剪 + 对齐 |
| | `container_fill_takes_the_offered_space` | `with_fill()` 吃满 offered 空间 |
| | `container_style_defaults_are_empty` | `ContainerStyle::default()` 等于 `EMPTY` |
| 嵌套 | `nested_containers_accumulate_window_coordinates` | VBox > HBox > Leaf 的 `window_bounds` 累加 |
| | `nested_paint_uses_window_coordinates` | `Painter` 命令中出现窗口坐标下的叶子矩形 |
