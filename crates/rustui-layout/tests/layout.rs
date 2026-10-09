//! End-to-end layout tests: a small tree of test leaves is measured, arranged and inspected.

use rustui_core::impl_widget_base;
use rustui_core::{
    Align, ArrangeContext, Color, CrossAxisAlign, EventContext, EventResult, Interaction,
    MainAxisAlign, MeasureContext, ObjectId, ObjectRegistry, PaintContext, Rect, Size, SizeHint,
    TextLayout, TextMeasurer, UiContext, UiEvent, Widget, WidgetBase, WidgetStore,
};

use rustui_layout::{
    Container, ContainerStyle, Flow, Grid, HBox, Separator, Spacer, Stack, Track, VBox,
};

// ---------------------------------------------------------------------------
// test widgets
// ---------------------------------------------------------------------------

/// A leaf with a fixed natural size.
#[derive(Debug)]
struct Leaf {
    base: WidgetBase,
    size: Size,
    hint: SizeHint,
}

impl Leaf {
    fn new(width: f32, height: f32) -> Self {
        Self {
            base: WidgetBase::new(),
            size: Size::new(width, height),
            hint: SizeHint::default(),
        }
    }
}

impl_widget_base!(Leaf);

impl Widget for Leaf {
    fn kind(&self) -> &'static str {
        "Leaf"
    }

    fn size_hint(&self) -> SizeHint {
        self.hint
    }

    fn measure(&mut self, _ctx: &MeasureContext<'_>, _available: Size) -> Size {
        self.base.set_measured(self.size);
        self.size
    }

    fn arrange(&mut self, _ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
    }

    fn event(&mut self, _ctx: &mut EventContext<'_>, _event: &UiEvent) -> EventResult {
        EventResult::Unhandled
    }

    fn paint(&mut self, ctx: &mut PaintContext<'_>) {
        ctx.fill_rect(self.base.bounds(), Color::BLUE);
    }
}

/// Text measurer reporting a fixed advance per character.
struct StubText;

impl TextMeasurer for StubText {
    fn layout_text(
        &self,
        text: &str,
        _style: &rustui_core::TextStyle,
        _max: Option<f32>,
    ) -> TextLayout {
        TextLayout::single_line(Size::new(text.len() as f32 * 7.0, 14.0), 10.0, 4.0)
    }

    fn line_height(&self, _style: &rustui_core::TextStyle) -> f32 {
        14.0
    }
}

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

struct Tree {
    ui: UiContext,
    store: WidgetStore,
    registry: ObjectRegistry,
    interaction: Interaction,
    root: ObjectId,
}

impl Tree {
    /// A tree without a root; children can be created first so that the root widget can be
    /// built with explicit placements / children.
    fn empty() -> Self {
        Self {
            ui: UiContext::new(Box::new(StubText)),
            store: WidgetStore::new(),
            registry: ObjectRegistry::new(),
            interaction: Interaction::default(),
            root: ObjectId::INVALID,
        }
    }

    fn new(root: Box<dyn Widget>) -> Self {
        let mut tree = Self::empty();
        tree.set_root(root);
        tree
    }

    fn set_root(&mut self, root: Box<dyn Widget>) {
        self.root = self.store.insert(&mut self.registry, root);
    }

    fn insert(&mut self, widget: Box<dyn Widget>) -> ObjectId {
        self.store.insert(&mut self.registry, widget)
    }

    fn add(&mut self, parent: ObjectId, child: ObjectId) {
        let mut widget = self.store.take(parent).expect("parent exists");
        widget.base_mut().add_child(child);
        self.store.put(parent, widget);
    }

    fn measure(&mut self, available: Size) {
        let ctx = MeasureContext::new(&self.ui, &self.store, &self.interaction, 0);
        ctx.measure_child(self.root, available);
    }

    fn arrange(&mut self, rect: Rect) {
        let ctx = ArrangeContext::new(&self.ui, &self.store, &self.interaction, 0);
        ctx.arrange_child(self.root, rect);
    }

    fn layout(&mut self, available: Size, rect: Rect) {
        self.measure(available);
        self.arrange(rect);
    }

    fn bounds(&self, id: ObjectId) -> Rect {
        self.store
            .get(id)
            .expect("widget exists")
            .base()
            .window_bounds()
    }

    fn measured(&self, id: ObjectId) -> Size {
        self.store.get(id).expect("widget exists").base().measured()
    }
}

fn assert_rect(actual: Rect, expected: Rect) {
    let close = (actual.x - expected.x).abs() < 0.01
        && (actual.y - expected.y).abs() < 0.01
        && (actual.width - expected.width).abs() < 0.01
        && (actual.height - expected.height).abs() < 0.01;
    assert!(close, "expected {expected:?}, got {actual:?}");
}

// ---------------------------------------------------------------------------
// VBox / HBox
// ---------------------------------------------------------------------------

#[test]
fn vbox_stacks_children_with_spacing_and_padding() {
    let mut tree = Tree::new(Box::new(
        VBox::new()
            .with_spacing(5.0)
            .with_padding(rustui_core::Insets::all(10.0)),
    ));
    let a = tree.insert(Box::new(Leaf::new(30.0, 10.0)));
    let b = tree.insert(Box::new(Leaf::new(30.0, 20.0)));
    let c = tree.insert(Box::new(Leaf::new(30.0, 30.0)));
    tree.add(tree.root, a);
    tree.add(tree.root, b);
    tree.add(tree.root, c);

    // 10 + 5 + 20 + 5 + 30 = 70 content rows, 30 content columns, 10 px padding around it.
    tree.measure(Size::new(200.0, 200.0));
    assert_eq!(tree.measured(tree.root), Size::new(50.0, 90.0));

    tree.arrange(Rect::new(0.0, 0.0, 200.0, 200.0));
    // Cross alignment defaults to `Start`, so every child keeps its natural width.
    assert_rect(tree.bounds(a), Rect::new(10.0, 10.0, 30.0, 10.0));
    assert_rect(tree.bounds(b), Rect::new(10.0, 25.0, 30.0, 20.0));
    assert_rect(tree.bounds(c), Rect::new(10.0, 50.0, 30.0, 30.0));
}

#[test]
fn vbox_cross_align_stretch_fills_the_content_width() {
    let mut tree = Tree::new(Box::new(
        VBox::new().with_cross_align(CrossAxisAlign::Stretch),
    ));
    let a = tree.insert(Box::new(Leaf::new(30.0, 10.0)));
    tree.add(tree.root, a);

    tree.layout(Size::new(200.0, 200.0), Rect::new(0.0, 0.0, 200.0, 200.0));
    assert_rect(tree.bounds(a), Rect::new(0.0, 0.0, 200.0, 10.0));
}

#[test]
fn hbox_gives_free_space_to_flex_children() {
    let mut tree = Tree::new(Box::new(HBox::new()));
    let a = tree.insert(Box::new(Leaf::new(40.0, 20.0)));
    let gap = tree.insert(Box::new(Spacer::new(1.0)));
    let b = tree.insert(Box::new(Leaf::new(40.0, 20.0)));
    tree.add(tree.root, a);
    tree.add(tree.root, gap);
    tree.add(tree.root, b);

    tree.layout(Size::new(200.0, 50.0), Rect::new(0.0, 0.0, 200.0, 50.0));
    // Cross alignment defaults to `Start`, so the buttons keep their natural height and only the
    // main axis is stretched.
    assert_rect(tree.bounds(a), Rect::new(0.0, 0.0, 40.0, 20.0));
    assert_rect(tree.bounds(gap), Rect::new(40.0, 0.0, 120.0, 0.0));
    assert_rect(tree.bounds(b), Rect::new(160.0, 0.0, 40.0, 20.0));
}

#[test]
fn free_space_is_distributed_by_main_axis_align() {
    let mut tree = Tree::new(Box::new(HBox::new().with_main_align(MainAxisAlign::Center)));
    let a = tree.insert(Box::new(Leaf::new(40.0, 20.0)));
    let b = tree.insert(Box::new(Leaf::new(40.0, 20.0)));
    tree.add(tree.root, a);
    tree.add(tree.root, b);

    tree.layout(Size::new(200.0, 20.0), Rect::new(0.0, 0.0, 200.0, 20.0));
    // 120 px of free space, centred: 60 px before the first child.
    assert_rect(tree.bounds(a), Rect::new(60.0, 0.0, 40.0, 20.0));
    assert_rect(tree.bounds(b), Rect::new(100.0, 0.0, 40.0, 20.0));
}

#[test]
fn measure_is_clamped_to_the_available_space() {
    let mut tree = Tree::new(Box::new(VBox::new()));
    let a = tree.insert(Box::new(Leaf::new(100.0, 100.0)));
    let b = tree.insert(Box::new(Leaf::new(100.0, 100.0)));
    tree.add(tree.root, a);
    tree.add(tree.root, b);

    tree.measure(Size::new(80.0, 50.0));
    assert_eq!(tree.measured(tree.root), Size::new(80.0, 50.0));
}

#[test]
fn invisible_children_are_skipped() {
    let mut tree = Tree::new(Box::new(VBox::new()));
    let a = tree.insert(Box::new(Leaf::new(30.0, 10.0)));
    let hidden = tree.insert(Box::new(Leaf::new(999.0, 999.0)));
    let b = tree.insert(Box::new(Leaf::new(30.0, 10.0)));
    tree.add(tree.root, a);
    tree.add(tree.root, hidden);
    tree.add(tree.root, b);
    if let Some(mut w) = tree.store.take(hidden) {
        w.base_mut().set_visible(false);
        tree.store.put(hidden, w);
    }

    tree.layout(Size::new(200.0, 200.0), Rect::new(0.0, 0.0, 200.0, 200.0));
    assert_eq!(tree.measured(tree.root), Size::new(30.0, 20.0));
    assert_rect(tree.bounds(a), Rect::new(0.0, 0.0, 30.0, 10.0));
    assert_rect(tree.bounds(b), Rect::new(0.0, 10.0, 30.0, 10.0));
    assert_eq!(tree.bounds(hidden), Rect::ZERO);
}

#[test]
fn separator_is_a_line_and_never_absorbs_free_space() {
    let mut tree = Tree::new(Box::new(VBox::new()));
    let a = tree.insert(Box::new(Leaf::new(30.0, 10.0)));
    let rule = tree.insert(Box::new(Separator::horizontal().with_thickness(2.0)));
    let gap = tree.insert(Box::new(Spacer::new(1.0)));
    tree.add(tree.root, a);
    tree.add(tree.root, rule);
    tree.add(tree.root, gap);

    tree.layout(Size::new(100.0, 100.0), Rect::new(0.0, 0.0, 100.0, 100.0));
    // The rule spans the full content width but stays 2 px tall.
    assert_rect(tree.bounds(rule), Rect::new(0.0, 10.0, 100.0, 2.0));
    // All the remaining height goes to the spacer (which is invisible: zero width, but it does
    // occupy the main axis so that the box knows where its free space went).
    assert_rect(tree.bounds(gap), Rect::new(0.0, 12.0, 0.0, 88.0));
}

// ---------------------------------------------------------------------------
// Grid
// ---------------------------------------------------------------------------

#[test]
fn grid_splits_fixed_and_fractional_tracks() {
    let mut tree = Tree::new(Box::new(
        Grid::new()
            .with_columns(vec![Track::px(50.0), Track::fr(1.0)])
            .with_rows(vec![Track::Auto]),
    ));
    let a = tree.insert(Box::new(Leaf::new(30.0, 10.0)));
    let b = tree.insert(Box::new(Leaf::new(20.0, 10.0)));
    tree.add(tree.root, a);
    tree.add(tree.root, b);

    tree.measure(Size::new(200.0, 40.0));
    // 50 px fixed column + everything that is left.
    assert_eq!(tree.measured(tree.root), Size::new(200.0, 10.0));

    tree.arrange(Rect::new(0.0, 0.0, 200.0, 40.0));
    assert_rect(tree.bounds(a), Rect::new(0.0, 0.0, 50.0, 10.0));
    assert_rect(tree.bounds(b), Rect::new(50.0, 0.0, 150.0, 10.0));
}

#[test]
fn grid_auto_placement_flows_row_by_row() {
    let mut tree = Tree::new(Box::new(
        Grid::new().with_columns(vec![Track::Auto, Track::Auto]),
    ));
    let a = tree.insert(Box::new(Leaf::new(30.0, 10.0)));
    let b = tree.insert(Box::new(Leaf::new(20.0, 10.0)));
    let c = tree.insert(Box::new(Leaf::new(40.0, 10.0)));
    let d = tree.insert(Box::new(Leaf::new(10.0, 10.0)));
    tree.add(tree.root, a);
    tree.add(tree.root, b);
    tree.add(tree.root, c);
    tree.add(tree.root, d);

    tree.layout(Size::new(500.0, 500.0), Rect::new(0.0, 0.0, 500.0, 500.0));
    // Row 0: a | b, row 1: c | d. The first column is as wide as its widest cell (c, 40 px),
    // the second one as wide as its widest cell (b, 20 px).
    assert_rect(tree.bounds(a), Rect::new(0.0, 0.0, 40.0, 10.0));
    assert_rect(tree.bounds(b), Rect::new(40.0, 0.0, 20.0, 10.0));
    assert_rect(tree.bounds(c), Rect::new(0.0, 10.0, 40.0, 10.0));
    assert_rect(tree.bounds(d), Rect::new(40.0, 10.0, 20.0, 10.0));
}

#[test]
fn grid_spanning_item_uses_the_whole_row() {
    let mut tree = Tree::empty();
    let a = tree.insert(Box::new(Leaf::new(30.0, 10.0)));
    let b = tree.insert(Box::new(Leaf::new(20.0, 10.0)));
    let wide = tree.insert(Box::new(Leaf::new(40.0, 10.0)));

    let mut grid = Grid::new()
        .with_columns(vec![Track::Auto, Track::Auto])
        .with_rows(vec![Track::Auto, Track::Auto]);
    grid.place_child(a, 0, 0);
    grid.place_child(b, 0, 1);
    grid.place_span(wide, 1, 0, 1, 2);
    tree.set_root(Box::new(grid));

    tree.measure(Size::new(500.0, 500.0));
    assert_eq!(tree.measured(tree.root), Size::new(50.0, 20.0));

    tree.arrange(Rect::new(0.0, 0.0, 50.0, 20.0));
    assert_rect(tree.bounds(a), Rect::new(0.0, 0.0, 30.0, 10.0));
    assert_rect(tree.bounds(b), Rect::new(30.0, 0.0, 20.0, 10.0));
    assert_rect(tree.bounds(wide), Rect::new(0.0, 10.0, 50.0, 10.0));
}

// ---------------------------------------------------------------------------
// Stack, Flow, Container
// ---------------------------------------------------------------------------

#[test]
fn stack_overlaps_children_in_the_same_box() {
    let mut tree = Tree::new(Box::new(Stack::new()));
    let a = tree.insert(Box::new(Leaf::new(30.0, 10.0)));
    let b = tree.insert(Box::new(Leaf::new(10.0, 40.0)));
    tree.add(tree.root, a);
    tree.add(tree.root, b);

    tree.layout(Size::new(200.0, 200.0), Rect::new(0.0, 0.0, 200.0, 200.0));
    // Stretch: both children fill the stack and overlap.
    assert_rect(tree.bounds(a), Rect::new(0.0, 0.0, 200.0, 200.0));
    assert_rect(tree.bounds(b), Rect::new(0.0, 0.0, 200.0, 200.0));
}

#[test]
fn stack_alignment_keeps_natural_size() {
    let mut tree = Tree::new(Box::new(Stack::new().with_align(Align::End, Align::End)));
    let a = tree.insert(Box::new(Leaf::new(30.0, 10.0)));
    tree.add(tree.root, a);

    tree.layout(Size::new(200.0, 100.0), Rect::new(0.0, 0.0, 200.0, 100.0));
    assert_rect(tree.bounds(a), Rect::new(170.0, 90.0, 30.0, 10.0));
}

#[test]
fn flow_wraps_into_lines() {
    let mut tree = Tree::new(Box::new(Flow::new().with_gaps(10.0, 10.0)));
    let a = tree.insert(Box::new(Leaf::new(50.0, 10.0)));
    let b = tree.insert(Box::new(Leaf::new(50.0, 10.0)));
    let c = tree.insert(Box::new(Leaf::new(50.0, 10.0)));
    tree.add(tree.root, a);
    tree.add(tree.root, b);
    tree.add(tree.root, c);

    // 120 px wide: two 50 px items plus the 10 px gap fit, the third wraps to the next line.
    tree.layout(Size::new(120.0, 200.0), Rect::new(0.0, 0.0, 120.0, 200.0));
    assert_rect(tree.bounds(a), Rect::new(0.0, 0.0, 50.0, 10.0));
    assert_rect(tree.bounds(b), Rect::new(60.0, 0.0, 50.0, 10.0));
    assert_rect(tree.bounds(c), Rect::new(0.0, 20.0, 50.0, 10.0));
}

#[test]
fn container_pads_and_aligns_its_child() {
    let mut tree = Tree::empty();
    let child = tree.insert(Box::new(Leaf::new(30.0, 10.0)));
    let container = Container::new()
        .with_padding(rustui_core::Insets::all(10.0))
        .with_background(Color::rgba(0.1, 0.1, 0.1, 1.0))
        .with_border(Color::WHITE, 2.0)
        .with_radius(4.0)
        .with_clip(true)
        .with_align(Align::Start, Align::Start)
        .child(child);
    tree.set_root(Box::new(container));

    tree.layout(Size::new(200.0, 100.0), Rect::new(0.0, 0.0, 200.0, 100.0));
    assert_rect(tree.bounds(child), Rect::new(10.0, 10.0, 30.0, 10.0));
}

#[test]
fn container_fill_takes_the_offered_space() {
    let mut tree = Tree::empty();
    let child = tree.insert(Box::new(Leaf::new(30.0, 10.0)));
    tree.set_root(Box::new(Container::new().with_fill().child(child)));

    tree.measure(Size::new(150.0, 60.0));
    assert_eq!(tree.measured(tree.root), Size::new(150.0, 60.0));
}

// ---------------------------------------------------------------------------
// nesting
// ---------------------------------------------------------------------------

#[test]
fn nested_containers_accumulate_window_coordinates() {
    // root VBox (padding 10) > HBox (spacing 5) > two leaves
    let mut tree = Tree::new(Box::new(
        VBox::new().with_padding(rustui_core::Insets::all(10.0)),
    ));
    let row = tree.insert(Box::new(HBox::new().with_spacing(5.0)));
    let a = tree.insert(Box::new(Leaf::new(20.0, 30.0)));
    let b = tree.insert(Box::new(Leaf::new(40.0, 30.0)));
    tree.add(tree.root, row);
    tree.add(row, a);
    tree.add(row, b);

    tree.layout(Size::new(300.0, 200.0), Rect::new(0.0, 0.0, 300.0, 200.0));
    // The row is measured as 20 + 5 + 40 = 65 wide and sits inside the root's padding.
    assert_rect(tree.bounds(row), Rect::new(10.0, 10.0, 65.0, 30.0));
    // The children live in the row's own coordinate system; `window_bounds` resolves that to
    // the same coordinates the root produced for the row.
    assert_rect(tree.bounds(a), Rect::new(10.0, 10.0, 20.0, 30.0));
    assert_rect(tree.bounds(b), Rect::new(35.0, 10.0, 40.0, 30.0));
}

#[test]
fn nested_paint_uses_window_coordinates() {
    use rustui_core::{OverlayList, Painter};

    let mut tree = Tree::new(Box::new(
        VBox::new().with_padding(rustui_core::Insets::all(10.0)),
    ));
    let row = tree.insert(Box::new(HBox::new().with_spacing(5.0)));
    let a = tree.insert(Box::new(Leaf::new(20.0, 30.0)));
    tree.add(tree.root, row);
    tree.add(row, a);

    tree.layout(Size::new(300.0, 200.0), Rect::new(0.0, 0.0, 300.0, 200.0));

    let mut painter = Painter::new();
    painter.reset(Rect::new(0.0, 0.0, 300.0, 200.0));
    let overlays = OverlayList::new();
    {
        let mut ctx = rustui_core::PaintContext::new(
            &tree.ui,
            &tree.store,
            &tree.interaction,
            &mut painter,
            &overlays,
            tree.store
                .get(tree.root)
                .unwrap()
                .base()
                .window_bounds()
                .origin(),
            Rect::new(0.0, 0.0, 300.0, 200.0),
            0,
        );
        ctx.paint_child(tree.root);
    }

    let rects: Vec<Rect> = painter
        .commands()
        .iter()
        .filter_map(|c| match c {
            rustui_core::PaintCommand::Shape { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect();
    assert!(
        rects.contains(&Rect::new(10.0, 10.0, 20.0, 30.0)),
        "leaf background missing, painted: {rects:?}"
    );
}

#[test]
fn container_style_defaults_are_empty() {
    let style = ContainerStyle {
        corner_radius: 6.0,
        clip: true,
        border_width: 1.0,
        ..ContainerStyle::EMPTY
    };
    assert_eq!(style.corner_radius, 6.0);
    assert!(style.clip);
    assert_eq!(ContainerStyle::default(), ContainerStyle::EMPTY);
}
