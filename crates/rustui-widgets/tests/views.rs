//! Integration tests for `TableView` and `TreeView` (design document §29, §30).
//!
//! The harness mirrors `tests/widgets.rs`: widgets are measured, arranged, painted and fed events
//! headlessly, the way `rustui-window` drives them. `deliver` walks the target and then its
//! ancestors until an event is accepted; pointer capture recorded by a widget is mirrored back
//! into the interaction state after every delivery, exactly like the real dispatcher.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rustui_core::{
    ArrangeContext, EventContext, EventQueue, EventResult, FocusManager, FocusReason, Interaction,
    Key, MeasureContext, Modifiers, MouseButton, NamedKey, ObjectId, ObjectRegistry, OverlayList,
    PaintCommand, PaintContext, Painter, Point, Rect, Shared, Size, TextLayout, TextMeasurer,
    TextStyle, UiContext, UiEvent, Widget, WidgetStore,
};
use rustui_model::{NodeId, TableListModel, TableModel, TreeListModel, TreeModel};
use rustui_widgets::{TableView, TreeView};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Text measurer with a fixed advance per character: deterministic and font independent.
struct StubText;

impl TextMeasurer for StubText {
    fn layout_text(&self, text: &str, _style: &TextStyle, max: Option<f32>) -> TextLayout {
        // Mirrors `rustui-text`: a wrapping box is at least as wide as the limit it was given.
        let natural = text.len() as f32 * 7.0;
        let width = match max {
            Some(limit) if limit > 0.0 => limit.max(natural),
            _ => natural,
        };
        TextLayout::single_line(Size::new(width, 14.0), 10.0, 4.0)
    }

    fn line_height(&self, _style: &TextStyle) -> f32 {
        14.0
    }
}

/// Counter a `Fn` signal callback can bump.
fn counter() -> Rc<Cell<u32>> {
    Rc::new(Cell::new(0))
}

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

struct Tree {
    ui: UiContext,
    store: WidgetStore,
    registry: ObjectRegistry,
    interaction: Interaction,
    focus: FocusManager,
    queue: EventQueue,
    overlays: OverlayList,
    consumed: Cell<bool>,
    root: ObjectId,
    viewport: Rect,
}

impl Tree {
    /// A tree whose root is the widget under test, over a stub text engine.
    fn with_root(root: Box<dyn Widget>, width: f32, height: f32) -> Self {
        let ui = UiContext::new(Box::new(StubText));
        let store = WidgetStore::new();
        let mut registry = ObjectRegistry::new();
        let root = store.insert(&mut registry, root);
        Self {
            ui,
            store,
            registry,
            interaction: Interaction::default(),
            focus: FocusManager::new(),
            queue: EventQueue::new(),
            overlays: OverlayList::new(),
            consumed: Cell::new(false),
            root,
            viewport: Rect::new(0.0, 0.0, width, height),
        }
    }

    fn layout(&mut self, available: Size) {
        {
            let ctx = MeasureContext::new(&self.ui, &self.store, &self.interaction, 0);
            ctx.measure_child(self.root, available);
        }
        let rect = self.viewport;
        let ctx = ArrangeContext::new(&self.ui, &self.store, &self.interaction, 0);
        ctx.arrange_child(self.root, rect);
    }

    /// Delivers `event` to `target` and then to its ancestors until it is consumed.
    fn deliver(&mut self, target: ObjectId, event: &UiEvent) -> bool {
        self.consumed.set(false);
        let mut current = Some(target);
        while let Some(id) = current {
            let Some(mut widget) = self.store.take(id) else {
                break;
            };
            let handled = {
                let mut ctx = EventContext::new(
                    &self.ui,
                    &self.store,
                    &self.interaction,
                    &self.focus,
                    &self.queue,
                    &self.overlays,
                    0,
                    &self.consumed,
                );
                widget.event(&mut ctx, event)
            };
            self.store.put(id, widget);
            // Mirror the queue's capture into the interaction, like the window dispatcher does.
            self.interaction.captured = self.queue.captured();
            if matches!(handled, EventResult::Handled) || self.consumed.get() {
                return true;
            }
            current = self.registry.parent(id);
        }
        false
    }

    /// Focuses `id` the way the window does when the user tabs / clicks onto it.
    fn focus(&mut self, id: ObjectId) {
        self.interaction.focused = Some(id);
        self.focus.set_focus(id, FocusReason::Programmatic);
        self.deliver(id, &UiEvent::FocusIn(FocusReason::Programmatic));
    }

    /// Paints the widget under test and returns the recorded commands.
    fn paint(&mut self) -> Painter {
        let mut painter = Painter::new();
        painter.reset(self.viewport);
        let root = self.root;
        let mut widget = self.store.take(root).expect("root exists");
        {
            let mut ctx = PaintContext::new(
                &self.ui,
                &self.store,
                &self.interaction,
                &mut painter,
                &self.overlays,
                Point::new(0.0, 0.0),
                self.viewport,
                0,
            );
            widget.paint(&mut ctx);
        }
        self.store.put(root, widget);
        painter
    }

    fn click(&mut self, target: ObjectId, x: f32, y: f32) {
        self.interaction.pointer = Point::new(x, y);
        self.deliver(
            target,
            &UiEvent::MouseDown {
                button: MouseButton::Left,
                position: Point::new(x, y),
            },
        );
        self.deliver(
            target,
            &UiEvent::MouseUp {
                button: MouseButton::Left,
                position: Point::new(x, y),
            },
        );
    }

    fn key(&mut self, target: ObjectId, key: NamedKey, modifiers: Modifiers) {
        self.deliver(
            target,
            &UiEvent::KeyDown {
                key: Key::Named(key),
                modifiers,
                text: None,
                repeat: false,
            },
        );
    }
}

/// Measures `widget` without a store, for tests that need the public API of a live widget.
fn measure_widget(widget: &mut dyn Widget, available: Size) -> Size {
    let ui = UiContext::new(Box::new(StubText));
    let store = WidgetStore::new();
    let interaction = Interaction::default();
    let ctx = MeasureContext::new(&ui, &store, &interaction, 0);
    widget.measure(&ctx, available)
}

/// Every string that reached the painter, in paint order.
fn painted_texts(painter: &Painter) -> Vec<String> {
    painter
        .commands()
        .iter()
        .filter_map(|command| match command {
            PaintCommand::Text { run, .. } => painter.arena().get(*run).map(str::to_string),
            _ => None,
        })
        .collect()
}

/// Every painted string with its window position, in paint order.
fn painted_text_positions(painter: &Painter) -> Vec<(String, Point)> {
    painter
        .commands()
        .iter()
        .filter_map(|command| match command {
            PaintCommand::Text { position, run, .. } => painter
                .arena()
                .get(*run)
                .map(|text| (text.to_string(), *position)),
            _ => None,
        })
        .collect()
}

fn table_model(headers: [&str; 2]) -> Shared<Box<dyn TableModel>> {
    Shared::new(Box::new(TableListModel::from_headers(headers)) as Box<dyn TableModel>)
}

fn table_with_rows(rows: usize) -> Shared<Box<dyn TableModel>> {
    let mut model = TableListModel::from_headers(["Name", "Value"]);
    for index in 0..rows {
        model.push_row([format!("row{index}"), format!("v{index}")]);
    }
    Shared::new(Box::new(model) as Box<dyn TableModel>)
}

// ---------------------------------------------------------------------------
// TableView: columns and geometry
// ---------------------------------------------------------------------------

#[test]
fn table_header_and_column_geometry() {
    // Direct measure: the widget seeds the column widths from the header text.
    let mut table = TableView::with_model(table_model(["Package manager", "Version"]));
    let size = measure_widget(&mut table, Size::new(400.0, 300.0));
    assert_eq!(size, Size::new(400.0, 300.0));
    // StubText measures 7 px per character: "Package manager" = 15 * 7 + 16 padding = 121,
    // "Version" = 7 * 7 + 16 = 65, which stays at the 80 px default.
    assert_eq!(table.columns().to_vec(), vec![121.0, 80.0]);
    assert_eq!(table.column_width(0), 121.0);
    assert_eq!(table.column_width(9), 0.0, "out of range reports 0");
    assert_eq!(table.row_count(), 0);
    assert_eq!(table.column_count(), 2);

    // Painted geometry: both headers sit on one line, left aligned inside their column.
    let mut tree = Tree::with_root(
        Box::new(TableView::with_model(table_model([
            "Package manager",
            "Version",
        ]))),
        400.0,
        300.0,
    );
    tree.layout(Size::new(400.0, 300.0));
    let painter = tree.paint();
    let texts = painted_texts(&painter);
    assert!(texts.contains(&"Package manager".to_string()), "{texts:?}");
    assert!(texts.contains(&"Version".to_string()), "{texts:?}");
    let positions = painted_text_positions(&painter);
    let version = positions
        .iter()
        .find(|(text, _)| text == "Version")
        .expect("the Version header was painted");
    // x = 8 px padding + 121 px first column; y = centred in the 36 px header (36 - 14) / 2.
    assert!((version.1.x - 129.0).abs() < 0.01, "got {:?}", version.1);
    assert!((version.1.y - 11.0).abs() < 0.01, "got {:?}", version.1);
}

#[test]
fn table_column_width_setter_clamps() {
    let mut table = TableView::with_model(table_model(["A", "B"]));
    table.set_column_width(0, 1.0);
    assert_eq!(table.column_width(0), 24.0, "clamped to the minimum");
    table.set_column_width(1, 500.0);
    assert_eq!(table.column_width(1), 500.0);
    table.set_column_width(7, 123.0);
    assert_eq!(table.column_width(7), 0.0, "beyond the model is a no-op");
    assert_eq!(table.columns().len(), 2, "the width vector never grows");

    // The first measure synchronises with the model but keeps the explicit widths.
    measure_widget(&mut table, Size::new(400.0, 300.0));
    assert_eq!(table.column_width(0), 24.0);
    assert_eq!(table.column_width(1), 500.0);
}

#[test]
fn table_dragging_a_header_boundary_resizes_the_column() {
    let mut tree = Tree::with_root(
        Box::new(TableView::with_model(table_model([
            "Package manager",
            "Version",
        ]))),
        400.0,
        300.0,
    );
    tree.layout(Size::new(400.0, 300.0));
    let root = tree.root;

    let before = painted_text_positions(&tree.paint());
    let version = before
        .iter()
        .find(|(text, _)| text == "Version")
        .expect("the Version header was painted");
    assert!((version.1.x - 129.0).abs() < 0.01, "got {:?}", version.1);

    // Grab the boundary right after the first column (x = 121) and drag it to x = 300.
    tree.deliver(
        root,
        &UiEvent::MouseDown {
            button: MouseButton::Left,
            position: Point::new(121.0, 10.0),
        },
    );
    tree.deliver(
        root,
        &UiEvent::MouseMove {
            position: Point::new(300.0, 10.0),
        },
    );
    tree.deliver(
        root,
        &UiEvent::MouseUp {
            button: MouseButton::Left,
            position: Point::new(300.0, 10.0),
        },
    );

    let after = painted_text_positions(&tree.paint());
    let version = after
        .iter()
        .find(|(text, _)| text == "Version")
        .expect("the Version header was painted");
    // The second column now starts at 8 px padding + 300 px first column.
    assert!(
        (version.1.x - 308.0).abs() < 0.01,
        "expected x = 308, got {:?}",
        version.1
    );
}

// ---------------------------------------------------------------------------
// TableView: interaction
// ---------------------------------------------------------------------------

#[test]
fn table_click_selects_a_row_and_ignores_header_clicks() {
    let table = TableView::with_model(table_with_rows(5));
    let current = table.current_property();
    let changes = Rc::new(RefCell::new(Vec::new()));
    let sink = changes.clone();
    let _connection = table
        .selection_changed()
        .connect(move |change| sink.borrow_mut().push(change.current));

    let mut tree = Tree::with_root(Box::new(table), 400.0, 300.0);
    tree.layout(Size::new(400.0, 300.0));
    let root = tree.root;

    // A click in the header (the first 36 px) never changes the selection.
    tree.click(root, 50.0, 10.0);
    assert_eq!(current.get(), None, "header clicks must not select rows");
    assert!(changes.borrow().is_empty());

    // The row area starts below the header: 36..72 is row 0, 72..108 is row 1.
    tree.click(root, 50.0, 40.0);
    assert_eq!(current.get(), Some(0));
    tree.click(root, 50.0, 78.0);
    assert_eq!(current.get(), Some(1));
    assert_eq!(*changes.borrow(), vec![Some(0), Some(1)]);
}

#[test]
fn table_keyboard_moves_selection_and_ensures_visibility() {
    let mut model = TableListModel::from_headers(["Name"]);
    for index in 0..100 {
        model.push_row([format!("row{index}")]);
    }
    let table = TableView::with_model(Shared::new(Box::new(model) as Box<dyn TableModel>));
    let current = table.current_property();
    let mut tree = Tree::with_root(Box::new(table), 400.0, 300.0);
    tree.layout(Size::new(400.0, 300.0));
    let root = tree.root;
    tree.focus(root);

    tree.key(root, NamedKey::ArrowDown, Modifiers::NONE);
    assert_eq!(
        current.get(),
        Some(0),
        "the first arrow selects the first row"
    );

    tree.key(root, NamedKey::End, Modifiers::NONE);
    assert_eq!(current.get(), Some(99));
    // The viewport scrolled to the end: only the tail of the model is painted.
    let painter = tree.paint();
    let texts = painted_texts(&painter);
    assert!(texts.contains(&"row99".to_string()), "{texts:?}");
    assert!(!texts.iter().any(|text| text == "row0"), "{texts:?}");

    tree.key(root, NamedKey::Home, Modifiers::NONE);
    assert_eq!(current.get(), Some(0));
    let painter = tree.paint();
    let texts = painted_texts(&painter);
    assert!(texts.contains(&"row0".to_string()), "{texts:?}");
    assert!(!texts.iter().any(|text| text == "row99"), "{texts:?}");

    tree.key(root, NamedKey::ArrowDown, Modifiers::NONE);
    assert_eq!(current.get(), Some(1));
    tree.key(root, NamedKey::PageDown, Modifiers::NONE);
    // One page = 90% of the 264 px viewport = 237.6 px = 6 rows, plus one to guarantee progress.
    assert_eq!(current.get(), Some(8));
}

#[test]
fn table_activation_fires_on_double_click_and_enter() {
    let mut model = TableListModel::from_headers(["Name"]);
    model.push_row(["row0"]);
    let table = TableView::with_model(Shared::new(Box::new(model) as Box<dyn TableModel>));
    let activations = counter();
    let sink = activations.clone();
    let _connection = table.activated().connect(move |_| sink.set(sink.get() + 1));

    let mut tree = Tree::with_root(Box::new(table), 400.0, 300.0);
    tree.layout(Size::new(400.0, 300.0));
    let root = tree.root;

    // A plain click selects but never activates.
    tree.click(root, 50.0, 40.0);
    assert_eq!(activations.get(), 0);

    tree.deliver(
        root,
        &UiEvent::MouseDoubleClick {
            button: MouseButton::Left,
            position: Point::new(50.0, 40.0),
        },
    );
    assert_eq!(activations.get(), 1, "double click activates the row");

    tree.focus(root);
    tree.key(root, NamedKey::Return, Modifiers::NONE);
    assert_eq!(activations.get(), 2, "Enter activates the current row");
}

#[test]
fn table_paints_only_the_visible_rows_of_a_huge_model() {
    let mut tree = Tree::with_root(
        Box::new(TableView::with_model(table_with_rows(10_000))),
        400.0,
        300.0,
    );
    tree.layout(Size::new(400.0, 300.0));

    let painter = tree.paint();
    let texts = painted_texts(&painter);
    // 36 px rows in a 300 px window below a 36 px header: eight rows, two cells each, plus the
    // two headers — nowhere near the 10_000 rows of the model.
    assert!(texts.len() < 100, "painted {} texts", texts.len());
    assert!(texts.contains(&"row0".to_string()), "{texts:?}");
    assert!(
        !texts.iter().any(|text| text == "row5000"),
        "row5000 is far outside the viewport: {texts:?}"
    );
}

// ---------------------------------------------------------------------------
// TreeView: flattening
// ---------------------------------------------------------------------------

#[test]
fn tree_flattens_expanded_nodes_in_depth_order() {
    let mut model = TreeListModel::new();
    let root = model.append(None, "root");
    let a = model.append(Some(root), "a");
    let b = model.append(Some(root), "b");
    let second = model.append(None, "second");
    let model = Shared::new(Box::new(model) as Box<dyn TreeModel>);
    let mut view = TreeView::with_model(model.clone());

    // Everything starts collapsed: only the roots are visible.
    measure_widget(&mut view, Size::new(300.0, 300.0));
    assert_eq!(view.root_count(), 2);
    assert_eq!(view.node_count(), 4);
    assert_eq!(view.visible_row_count(), 2);
    assert_eq!(view.visible_rows(), &[(root, 0), (second, 0)][..]);

    // Expanding the first root reveals its children depth first, before the next root.
    model.with_mut(|m| m.set_expanded(&root, true));
    measure_widget(&mut view, Size::new(300.0, 300.0));
    assert_eq!(
        view.visible_rows(),
        &[(root, 0), (a, 1), (b, 1), (second, 0)][..]
    );

    // Selection is addressed by node id, so it survives collapsing the parent.
    view.select(b);
    assert_eq!(view.selected(), Some(b));
    model.with_mut(|m| m.set_expanded(&root, false));
    measure_widget(&mut view, Size::new(300.0, 300.0));
    assert_eq!(view.visible_rows(), &[(root, 0), (second, 0)][..]);
    assert_eq!(
        view.selected(),
        Some(b),
        "the selected node id outlives its row"
    );
    assert_eq!(view.visible_row_count(), 2);
}

#[test]
fn tree_twisty_click_toggles_expansion() {
    let mut model = TreeListModel::new();
    let root = model.append(None, "root");
    model.append(Some(root), "child");
    let model = Shared::new(Box::new(model) as Box<dyn TreeModel>);
    let mut tree = Tree::with_root(Box::new(TreeView::with_model(model.clone())), 300.0, 300.0);
    tree.layout(Size::new(300.0, 300.0));
    let view = tree.root;

    assert!(!model.with(|m| m.is_expanded(&root)));
    assert!(
        !painted_texts(&tree.paint()).contains(&"child".to_string()),
        "collapsed children are never painted"
    );

    // The twisty occupies the first 16 px after the 8 px padding; the row is 36 px tall.
    tree.click(view, 12.0, 18.0);
    assert!(
        model.with(|m| m.is_expanded(&root)),
        "the twisty expanded the branch"
    );
    assert!(painted_texts(&tree.paint()).contains(&"child".to_string()));

    // Clicking the label selects instead of toggling.
    tree.click(view, 80.0, 18.0);
    assert!(
        model.with(|m| m.is_expanded(&root)),
        "a label click must not collapse"
    );

    tree.click(view, 12.0, 18.0);
    assert!(
        !model.with(|m| m.is_expanded(&root)),
        "the second twisty click collapses"
    );
    assert!(
        !painted_texts(&tree.paint()).contains(&"child".to_string()),
        "the child is hidden again"
    );
}

#[test]
fn tree_selection_is_addressed_by_node_id() {
    let mut model = TreeListModel::new();
    let root = model.append(None, "root");
    let first_child = model.append(Some(root), "first");
    let second_child = model.append(Some(root), "second");
    let model = Shared::new(Box::new(model) as Box<dyn TreeModel>);
    model.with_mut(|m| m.set_expanded(&root, true));

    // Programmatic selection on a live widget, observed through the signal and the property.
    let mut free = TreeView::with_model(model.clone());
    let changes = Rc::new(RefCell::new(Vec::new()));
    let sink = changes.clone();
    let _connection = free
        .selection_changed()
        .connect(move |value| sink.borrow_mut().push(*value));
    free.select(first_child);
    assert_eq!(free.selected(), Some(first_child));
    free.select(NodeId::new(4_242));
    assert_eq!(free.selected(), None, "an unknown id clears the selection");
    assert_eq!(*changes.borrow(), vec![Some(first_child), None]);

    let property = free.selection_property();
    free.select(second_child);
    assert_eq!(property.get(), Some(second_child));

    // The pointer drives the boxed widget the same way.
    let widget = TreeView::with_model(model.clone());
    let current = widget.selection_property();
    let mut tree = Tree::with_root(Box::new(widget), 300.0, 300.0);
    tree.layout(Size::new(300.0, 300.0));
    let view = tree.root;

    // Rows: root (0..36), first (36..72), second (72..108); the label starts right of the
    // 8 px padding + 16 px twisty slot + 16 px depth indent.
    tree.click(view, 80.0, 54.0);
    assert_eq!(current.get(), Some(first_child));
    tree.click(view, 80.0, 90.0);
    assert_eq!(current.get(), Some(second_child));
}

#[test]
fn tree_keyboard_navigates_and_expands_with_arrows() {
    let mut model = TreeListModel::new();
    let root = model.append(None, "root");
    model.append(Some(root), "c0");
    model.append(Some(root), "c1");
    let second = model.append(None, "second");
    let model = Shared::new(Box::new(model) as Box<dyn TreeModel>);

    let widget = TreeView::with_model(model.clone());
    let current = widget.selection_property();
    let activations = counter();
    let sink = activations.clone();
    let _connection = widget
        .activated()
        .connect(move |_| sink.set(sink.get() + 1));
    let mut tree = Tree::with_root(Box::new(widget), 300.0, 300.0);
    tree.layout(Size::new(300.0, 300.0));
    let view = tree.root;
    tree.focus(view);

    // Collapsed: two rows. The first arrow selects the first root.
    tree.key(view, NamedKey::ArrowDown, Modifiers::NONE);
    assert_eq!(current.get(), Some(root));

    // ArrowRight on a collapsed branch expands it, revealing c0 and c1.
    tree.key(view, NamedKey::ArrowRight, Modifiers::NONE);
    assert!(model.with(|m| m.is_expanded(&root)), "expanded in place");
    assert_eq!(
        current.get(),
        Some(root),
        "expanding does not move the cursor"
    );

    // ArrowRight again steps into the first child.
    tree.key(view, NamedKey::ArrowRight, Modifiers::NONE);
    assert_eq!(current.get(), model.with(|m| m.child(&root, 0)));

    // ArrowLeft on a leaf steps up to the parent; ArrowLeft on an expanded branch collapses it.
    tree.key(view, NamedKey::ArrowLeft, Modifiers::NONE);
    assert_eq!(current.get(), Some(root));
    tree.key(view, NamedKey::ArrowLeft, Modifiers::NONE);
    assert!(!model.with(|m| m.is_expanded(&root)), "collapsed in place");
    assert_eq!(current.get(), Some(root));

    // Back to two rows: ArrowDown reaches the second root, Enter activates it.
    tree.key(view, NamedKey::ArrowDown, Modifiers::NONE);
    assert_eq!(current.get(), Some(second));
    tree.key(view, NamedKey::Return, Modifiers::NONE);
    assert_eq!(activations.get(), 1, "Enter activates the selected node");
    assert_eq!(current.get(), Some(second));
}

#[test]
fn tree_paints_only_visible_rows_of_a_deep_tree() {
    let mut model = TreeListModel::new();
    for index in 0..5_000 {
        model.append(None, &format!("root{index}"));
    }
    let model = Shared::new(Box::new(model) as Box<dyn TreeModel>);

    // A probe view counts the model without entering the harness. Every root is
    // "visible" in the expansion sense when collapsed; the viewport only culls at paint
    // time, which the text assertions below prove.
    let mut probe = TreeView::with_model(model.clone());
    measure_widget(&mut probe, Size::new(300.0, 300.0));
    assert_eq!(probe.node_count(), 5_000);
    assert_eq!(probe.root_count(), 5_000);
    assert_eq!(probe.visible_row_count(), 5_000);

    let mut tree = Tree::with_root(Box::new(TreeView::with_model(model)), 300.0, 300.0);
    tree.layout(Size::new(300.0, 300.0));
    let view = tree.root;
    tree.focus(view);

    let painter = tree.paint();
    let texts = painted_texts(&painter);
    assert!(texts.len() < 100, "painted {} texts", texts.len());
    assert!(texts.contains(&"root0".to_string()), "{texts:?}");
    assert!(
        !texts.iter().any(|text| text == "root4000"),
        "root4000 is far outside the viewport: {texts:?}"
    );

    // End jumps to the last node and scrolls it into view.
    tree.key(view, NamedKey::End, Modifiers::NONE);
    let painter = tree.paint();
    let texts = painted_texts(&painter);
    assert!(texts.contains(&"root4999".to_string()), "{texts:?}");
    assert!(!texts.iter().any(|text| text == "root0"), "{texts:?}");
}
