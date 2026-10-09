//! End-to-end tests of the window pipeline: layout, hit testing, event delivery, focus,
//! pointer capture, overlays and cursors.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rustui_core::{
    impl_widget_base, ArrangeContext, Color, CursorIcon, EventContext, EventResult, FrameClock,
    Key, MeasureContext, Modifiers, MouseButton, NamedKey, ObjectId, PaintContext, Point, Rect,
    Size, SizeHint, TextLayout, TextMeasurer, UiEvent, Widget, WidgetBase, WidgetState,
};
use rustui_window::{EventOutcome, WindowId, WindowManager, WindowOptions};

// ---------------------------------------------------------------------------
// support
// ---------------------------------------------------------------------------

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

/// Records strings emitted by the widgets under test.
#[derive(Clone, Debug, Default)]
struct Log(Rc<RefCell<Vec<String>>>);

impl Log {
    fn push(&self, entry: impl Into<String>) {
        self.0.borrow_mut().push(entry.into());
    }

    fn entries(&self) -> Vec<String> {
        self.0.borrow().clone()
    }

    fn count(&self, entry: &str) -> usize {
        self.0
            .borrow()
            .iter()
            .filter(|value| value.as_str() == entry)
            .count()
    }

    fn clear(&self) {
        self.0.borrow_mut().clear();
    }
}

/// A leaf widget with configurable behaviour.
#[derive(Debug)]
struct Leaf {
    base: WidgetBase,
    size: Size,
    focusable: bool,
    handles: bool,
    captures: bool,
    cursor: CursorIcon,
    /// Caret rectangle reported for the input method while this leaf owns focus (T-1).
    ime: Option<Rect>,
    /// Draws a popup layer (T-4) instead of staying inside its own rectangle.
    popup: bool,
    log: Option<Log>,
    paint_log: Option<Log>,
    updates: Rc<Cell<u32>>,
}

impl Leaf {
    fn new(width: f32, height: f32) -> Self {
        Self {
            base: WidgetBase::new(),
            size: Size::new(width, height),
            focusable: false,
            handles: false,
            captures: false,
            cursor: CursorIcon::Default,
            ime: None,
            popup: false,
            log: None,
            paint_log: None,
            updates: Rc::new(Cell::new(0)),
        }
    }

    /// Asks for the popup layer while painted (T-4).
    fn popup(mut self) -> Self {
        self.popup = true;
        self
    }

    fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }

    fn handles(mut self) -> Self {
        self.handles = true;
        self
    }

    fn captures(mut self) -> Self {
        self.captures = true;
        self
    }

    fn cursor(mut self, cursor: CursorIcon) -> Self {
        self.cursor = cursor;
        self
    }

    /// Makes this leaf behave like a text field for the input method (T-1).
    fn ime(mut self, rect: Rect) -> Self {
        self.ime = Some(rect);
        self
    }

    fn log(mut self, log: &Log) -> Self {
        self.log = Some(log.clone());
        self
    }

    fn paint_log(mut self, log: &Log) -> Self {
        self.paint_log = Some(log.clone());
        self
    }
}

impl_widget_base!(Leaf);

impl Widget for Leaf {
    fn kind(&self) -> &'static str {
        "Leaf"
    }

    fn accepts_focus(&self) -> bool {
        self.focusable
    }

    fn cursor(&self) -> CursorIcon {
        self.cursor
    }

    fn ime_cursor_area(&self, _ui: &rustui_core::UiContext) -> Option<Rect> {
        self.ime
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }

    fn measure(&mut self, _ctx: &MeasureContext<'_>, _available: Size) -> Size {
        self.base.set_measured(self.size);
        self.size
    }

    fn arrange(&mut self, _ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
    }

    fn update(&mut self, _ctx: &mut rustui_core::UpdateContext<'_>) {
        self.updates.set(self.updates.get() + 1);
    }

    fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult {
        if let Some(log) = &self.log {
            log.push(event.name());
        }
        if self.captures && matches!(event, UiEvent::MouseDown { .. }) {
            ctx.capture_pointer(self.base.id());
        }
        if self.handles {
            EventResult::Handled
        } else {
            EventResult::Unhandled
        }
    }

    fn paint(&mut self, ctx: &mut PaintContext<'_>) {
        if let Some(log) = &self.paint_log {
            log.push("leaf");
        }
        if self.popup {
            ctx.request_popup(self.id());
        }
        ctx.fill_rect(self.base.bounds(), Color::BLUE);
    }

    fn paint_popup(&mut self, ctx: &mut PaintContext<'_>) {
        if self.popup {
            if let Some(log) = &self.paint_log {
                log.push("popup");
            }
            // Local coordinates: the popup context carries this widget's window origin.
            ctx.fill_rect(Rect::new(0.0, 40.0, 40.0, 40.0), Color::RED);
        }
    }
}

/// A container that fills the space it is offered and lays its children out in a row.
#[derive(Debug)]
struct Panel {
    base: WidgetBase,
    focusable: bool,
    handles: bool,
    cursor: CursorIcon,
    log: Option<Log>,
    paint_log: Option<Log>,
    updates: Rc<Cell<u32>>,
    child_sizes: Vec<Size>,
}

impl Panel {
    fn new() -> Self {
        Self {
            base: WidgetBase::new(),
            focusable: false,
            handles: false,
            cursor: CursorIcon::Default,
            log: None,
            paint_log: None,
            updates: Rc::new(Cell::new(0)),
            child_sizes: Vec::new(),
        }
    }

    fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }

    fn handles(mut self) -> Self {
        self.handles = true;
        self
    }

    fn cursor(mut self, cursor: CursorIcon) -> Self {
        self.cursor = cursor;
        self
    }

    fn log(mut self, log: &Log) -> Self {
        self.log = Some(log.clone());
        self
    }

    fn paint_log(mut self, log: &Log) -> Self {
        self.paint_log = Some(log.clone());
        self
    }
}

impl_widget_base!(Panel);

impl Widget for Panel {
    fn kind(&self) -> &'static str {
        "Panel"
    }

    fn accepts_focus(&self) -> bool {
        self.focusable
    }

    fn cursor(&self) -> CursorIcon {
        self.cursor
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let children = self.children().to_vec();
        self.child_sizes = ctx.measure_children(&children, available);

        let mut size = Size::ZERO;
        for child in &self.child_sizes {
            size.width = size.width.max(child.width);
            size.height = size.height.max(child.height);
        }
        // The panel fills whatever it is offered; with no constraint it hugs its children.
        if available.width.is_finite() {
            size.width = available.width;
        }
        if available.height.is_finite() {
            size.height = available.height;
        }
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
        let children = self.children().to_vec();
        let mut x = 0.0;
        for (index, child) in children.iter().enumerate() {
            let natural = self.child_sizes.get(index).copied().unwrap_or(Size::ZERO);
            ctx.arrange_child(
                *child,
                Rect::new(x, 0.0, natural.width.max(0.0), natural.height.max(0.0)),
            );
            x += natural.width;
        }
    }

    fn update(&mut self, _ctx: &mut rustui_core::UpdateContext<'_>) {
        self.updates.set(self.updates.get() + 1);
    }

    fn event(&mut self, _ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult {
        if let Some(log) = &self.log {
            log.push(event.name());
        }
        if self.handles {
            EventResult::Handled
        } else {
            EventResult::Unhandled
        }
    }

    fn paint(&mut self, ctx: &mut PaintContext<'_>) {
        if let Some(log) = &self.paint_log {
            log.push("panel");
        }
        ctx.fill_rect(self.base.bounds(), Color::WHITE);
        for child in self.children() {
            ctx.paint_child(*child);
        }
    }
}

/// A tree wired to one window.
struct Harness {
    wm: WindowManager,
    win: WindowId,
}

impl Harness {
    fn new(width: f32, height: f32) -> Self {
        let mut wm = WindowManager::new(Box::new(StubText));
        let win = wm.create_window(WindowOptions::new().size(width, height));
        Self { wm, win }
    }

    fn insert(&mut self, widget: Box<dyn Widget>) -> ObjectId {
        self.wm.insert(widget)
    }

    fn add(&mut self, parent: ObjectId, child: ObjectId) {
        assert!(self.wm.add_child(parent, child));
    }

    fn render(&mut self) {
        let win = self.win;
        self.wm
            .render(win, FrameClock::default())
            .expect("window exists");
    }

    fn bounds(&self, id: ObjectId) -> Rect {
        self.wm
            .store()
            .get(id)
            .expect("widget exists")
            .base()
            .window_bounds()
    }

    fn mouse_down(&mut self, x: f32, y: f32) -> EventOutcome {
        self.wm.handle_event(
            self.win,
            UiEvent::MouseDown {
                button: MouseButton::Left,
                position: Point::new(x, y),
            },
        )
    }

    fn mouse_move(&mut self, x: f32, y: f32) -> EventOutcome {
        self.wm.handle_event(
            self.win,
            UiEvent::MouseMove {
                position: Point::new(x, y),
            },
        )
    }

    fn tab(&mut self, shift: bool) -> EventOutcome {
        self.wm.handle_event(
            self.win,
            UiEvent::KeyDown {
                key: Key::Named(NamedKey::Tab),
                modifiers: Modifiers {
                    shift,
                    control: false,
                    alt: false,
                    super_key: false,
                },
                text: None,
                repeat: false,
            },
        )
    }

    fn focused(&self) -> Option<ObjectId> {
        self.wm.window(self.win).expect("window exists").focused()
    }

    fn cursor(&self) -> CursorIcon {
        self.wm.window(self.win).expect("window exists").cursor()
    }

    /// `(hovered leaf, root to leaf chain)` of the window.
    fn hover(&self) -> (Option<ObjectId>, Vec<ObjectId>) {
        let window = self.wm.window(self.win).expect("window exists");
        let interaction = window.interaction();
        (interaction.hovered, interaction.hover_path.clone())
    }

    /// Pseudo state `id` derives from the current interaction.
    fn state(&self, id: ObjectId) -> WidgetState {
        self.wm
            .window(self.win)
            .expect("window exists")
            .interaction()
            .state_for(id, WidgetState::new())
    }
}

fn assert_rect(actual: Rect, expected: Rect) {
    let close = (actual.x - expected.x).abs() < 0.01
        && (actual.y - expected.y).abs() < 0.01
        && (actual.width - expected.width).abs() < 0.01
        && (actual.height - expected.height).abs() < 0.01;
    assert!(close, "expected {expected:?}, got {actual:?}");
}

/// Builds `Panel[ a, b ]` with the panel as window root.
fn build(harness: &mut Harness) -> (ObjectId, ObjectId, ObjectId) {
    let panel = harness.insert(Box::new(Panel::new()));
    let a = harness.insert(Box::new(Leaf::new(50.0, 20.0)));
    let b = harness.insert(Box::new(Leaf::new(60.0, 30.0)));
    harness.add(panel, a);
    harness.add(panel, b);
    assert!(harness.wm.set_root_id(harness.win, panel));
    (panel, a, b)
}

// ---------------------------------------------------------------------------
// frame pipeline
// ---------------------------------------------------------------------------

#[test]
fn frame_pipeline_arranges_the_content_into_the_window() {
    let mut harness = Harness::new(400.0, 300.0);
    let (panel, a, b) = build(&mut harness);
    harness.render();

    assert_rect(harness.bounds(panel), Rect::new(0.0, 0.0, 400.0, 300.0));
    assert_rect(harness.bounds(a), Rect::new(0.0, 0.0, 50.0, 20.0));
    assert_rect(harness.bounds(b), Rect::new(50.0, 0.0, 60.0, 30.0));

    // Panel background + two leaves.
    let painter = harness.wm.window(harness.win).unwrap().painter();
    assert!(painter.len() >= 3, "commands: {}", painter.len());
    assert!(harness.wm.render_input(harness.win).is_some());
}

#[test]
fn update_runs_exactly_once_per_widget_per_frame() {
    let mut harness = Harness::new(200.0, 100.0);

    let panel_widget = Panel::new();
    let panel_updates = panel_widget.updates.clone();
    let panel = harness.insert(Box::new(panel_widget));

    let a_widget = Leaf::new(50.0, 20.0);
    let a_updates = a_widget.updates.clone();
    let a = harness.insert(Box::new(a_widget));

    harness.add(panel, a);
    assert!(harness.wm.set_root_id(harness.win, panel));

    harness.render();
    assert_eq!(panel_updates.get(), 1, "the container is updated once");
    assert_eq!(a_updates.get(), 1, "the child is updated once, not twice");

    harness.render();
    assert_eq!(panel_updates.get(), 2);
    assert_eq!(a_updates.get(), 2);
}

#[test]
fn a_resize_event_reflows_the_tree() {
    let mut harness = Harness::new(400.0, 300.0);
    let (panel, a, _b) = build(&mut harness);
    harness.render();

    // The drawable area every context exposes as `viewport` (T-4) follows the window.
    let viewport = |harness: &Harness| {
        harness
            .wm
            .window(harness.win)
            .expect("window exists")
            .interaction()
            .viewport
    };
    assert_rect(viewport(&harness), Rect::new(0.0, 0.0, 400.0, 300.0));

    let outcome = harness
        .wm
        .handle_event(harness.win, UiEvent::Resize(Size::new(200.0, 100.0)));
    assert!(outcome.request_redraw);
    harness.render();

    assert_rect(harness.bounds(panel), Rect::new(0.0, 0.0, 200.0, 100.0));
    assert_rect(harness.bounds(a), Rect::new(0.0, 0.0, 50.0, 20.0));
    assert_rect(viewport(&harness), Rect::new(0.0, 0.0, 200.0, 100.0));
}

// ---------------------------------------------------------------------------
// hit testing and event delivery (§10, §11)
// ---------------------------------------------------------------------------

#[test]
fn hit_testing_returns_the_deepest_widget_and_events_bubble_up() {
    let mut harness = Harness::new(400.0, 300.0);
    let panel_log = Log::default();
    let a_log = Log::default();
    let panel = harness.insert(Box::new(Panel::new().handles().log(&panel_log)));
    let a = harness.insert(Box::new(Leaf::new(50.0, 20.0).log(&a_log)));
    let b = harness.insert(Box::new(Leaf::new(60.0, 30.0)));
    harness.add(panel, a);
    harness.add(panel, b);
    assert!(harness.wm.set_root_id(harness.win, panel));
    harness.render();

    assert_eq!(
        harness.wm.hit_test(harness.win, Point::new(10.0, 5.0)),
        Some(a)
    );
    assert_eq!(
        harness.wm.hit_test(harness.win, Point::new(60.0, 5.0)),
        Some(b)
    );
    assert_eq!(
        harness.wm.hit_test(harness.win, Point::new(300.0, 200.0)),
        Some(panel),
        "the container itself is the target when no child covers the point"
    );
    assert_eq!(
        harness.wm.hit_test(harness.win, Point::new(500.0, 500.0)),
        None,
        "outside the window there is nothing to hit"
    );

    a_log.clear();
    panel_log.clear();
    let outcome = harness.mouse_down(10.0, 5.0);
    assert_eq!(
        a_log.count("MouseDown"),
        1,
        "target first: {:?}",
        a_log.entries()
    );
    assert_eq!(
        panel_log.count("MouseDown"),
        1,
        "the unhandled click bubbles to the parent: {:?}",
        panel_log.entries()
    );
    assert!(outcome.handled, "the panel consumes the click");
}

#[test]
fn an_unhandled_event_keeps_bubbling_to_the_root() {
    let mut harness = Harness::new(400.0, 300.0);
    let panel_log = Log::default();
    let a_log = Log::default();
    let panel = harness.insert(Box::new(Panel::new().log(&panel_log)));
    let a = harness.insert(Box::new(Leaf::new(50.0, 20.0).log(&a_log)));
    harness.add(panel, a);
    assert!(harness.wm.set_root_id(harness.win, panel));
    harness.render();
    a_log.clear();
    panel_log.clear();

    let outcome = harness.mouse_down(10.0, 5.0);
    assert!(!outcome.handled, "nobody consumes the click");
    assert_eq!(a_log.count("MouseDown"), 1);
    assert_eq!(panel_log.count("MouseDown"), 1);
}

#[test]
fn hover_keeps_the_whole_chain_hovered() {
    let mut harness = Harness::new(400.0, 300.0);
    let (panel, a, b) = build(&mut harness);
    harness.render();

    let outcome = harness.mouse_move(10.0, 5.0);
    assert!(outcome.request_redraw);

    assert_eq!(
        harness.hover(),
        (Some(a), vec![panel, a]),
        "the whole chain under the pointer is hovered"
    );
    // A button behind its label has to paint as hovered.
    assert!(harness.state(panel).hovered);
    assert!(harness.state(a).hovered);

    // Moving to the other child leaves exactly one hovered chain.
    harness.mouse_move(60.0, 5.0);
    assert_eq!(harness.hover(), (Some(b), vec![panel, b]));
}

#[test]
fn mouse_enter_and_leave_follow_the_pointer() {
    let mut harness = Harness::new(400.0, 300.0);
    let a_log = Log::default();
    let panel = harness.insert(Box::new(Panel::new()));
    let a = harness.insert(Box::new(Leaf::new(50.0, 20.0).log(&a_log)));
    let b = harness.insert(Box::new(Leaf::new(60.0, 30.0)));
    harness.add(panel, a);
    harness.add(panel, b);
    assert!(harness.wm.set_root_id(harness.win, panel));
    harness.render();

    harness.mouse_move(10.0, 5.0);
    assert_eq!(a_log.count("MouseEnter"), 1);
    assert_eq!(a_log.count("MouseLeave"), 0);

    harness.mouse_move(60.0, 5.0);
    assert_eq!(a_log.count("MouseLeave"), 1);
    // Synthetic notifications are addressed directly, never bubbled a second time.
    assert_eq!(a_log.count("MouseEnter"), 1);
}

// ---------------------------------------------------------------------------
// focus (§12)
// ---------------------------------------------------------------------------

#[test]
fn pressing_the_pointer_focuses_the_deepest_focusable_widget() {
    let mut harness = Harness::new(400.0, 300.0);
    let a_log = Log::default();
    let panel = harness.insert(Box::new(Panel::new()));
    let a = harness.insert(Box::new(Leaf::new(50.0, 20.0).focusable().log(&a_log)));
    let b = harness.insert(Box::new(Leaf::new(60.0, 30.0)));
    harness.add(panel, a);
    harness.add(panel, b);
    assert!(harness.wm.set_root_id(harness.win, panel));
    harness.render();

    harness.mouse_down(10.0, 5.0);
    assert_eq!(harness.focused(), Some(a));
    assert_eq!(a_log.count("FocusIn"), 1);

    // Clicking a widget that cannot take focus clears it.
    harness.mouse_down(60.0, 5.0);
    assert_eq!(harness.focused(), None);
    assert_eq!(a_log.count("FocusOut"), 1);
}

#[test]
fn tab_walks_the_focus_order_and_wraps() {
    let mut harness = Harness::new(400.0, 300.0);
    let a_log = Log::default();
    let panel = harness.insert(Box::new(Panel::new()));
    let a = harness.insert(Box::new(Leaf::new(50.0, 20.0).focusable().log(&a_log)));
    let b = harness.insert(Box::new(Leaf::new(60.0, 30.0).focusable()));
    harness.add(panel, a);
    harness.add(panel, b);
    assert!(harness.wm.set_root_id(harness.win, panel));
    harness.render();

    harness.tab(false);
    assert_eq!(harness.focused(), Some(a));
    harness.tab(false);
    assert_eq!(harness.focused(), Some(b));
    harness.tab(false);
    assert_eq!(harness.focused(), Some(a), "the order wraps around");
    harness.tab(true);
    assert_eq!(harness.focused(), Some(b), "shift + tab goes backwards");
    assert_eq!(
        a_log.count("FocusIn"),
        2,
        "tab -> a, then shift+tab back past it"
    );
    assert_eq!(a_log.count("FocusOut"), 2, "tab -> b twice leaves a");
}

#[test]
fn focus_moves_can_also_go_through_the_manager() {
    let mut harness = Harness::new(400.0, 300.0);
    let panel = harness.insert(Box::new(Panel::new()));
    let a = harness.insert(Box::new(Leaf::new(50.0, 20.0).focusable()));
    harness.add(panel, a);
    assert!(harness.wm.set_root_id(harness.win, panel));
    harness.render();

    assert!(harness.wm.set_focus(harness.win, a));
    assert_eq!(harness.focused(), Some(a));
    assert!(harness.wm.focus_next(harness.win).is_some());
    assert_eq!(
        harness.focused(),
        Some(a),
        "only one focusable widget: it wraps onto itself"
    );
}

#[test]
fn clicking_a_focusable_container_takes_focus_and_sets_the_cursor() {
    let mut harness = Harness::new(400.0, 300.0);
    let panel = harness.insert(Box::new(
        Panel::new().focusable().cursor(CursorIcon::Pointer),
    ));
    let a = harness.insert(Box::new(Leaf::new(50.0, 20.0)));
    harness.add(panel, a);
    assert!(harness.wm.set_root_id(harness.win, panel));
    harness.render();

    // Clicking the container itself (no child covers the point) focuses it.
    harness.mouse_down(300.0, 200.0);
    assert_eq!(harness.focused(), Some(panel));

    harness.mouse_move(300.0, 200.0);
    assert_eq!(harness.cursor(), CursorIcon::Pointer);
    assert!(harness.state(panel).hovered);
    assert!(harness.state(panel).focused);
}

#[test]
fn a_child_that_handles_the_event_stops_the_bubbling() {
    let mut harness = Harness::new(400.0, 300.0);
    let panel_log = Log::default();
    let a_log = Log::default();
    let panel = harness.insert(Box::new(Panel::new().log(&panel_log)));
    let a = harness.insert(Box::new(Leaf::new(50.0, 20.0).handles().log(&a_log)));
    harness.add(panel, a);
    assert!(harness.wm.set_root_id(harness.win, panel));
    harness.render();
    a_log.clear();
    panel_log.clear();

    let outcome = harness.mouse_down(10.0, 5.0);
    assert!(outcome.handled);
    assert_eq!(a_log.count("MouseDown"), 1);
    assert_eq!(
        panel_log.count("MouseDown"),
        0,
        "the parent must not see an event its child consumed: {:?}",
        panel_log.entries()
    );
}

// ---------------------------------------------------------------------------
// pointer capture
// ---------------------------------------------------------------------------

#[test]
fn capture_routes_follow_up_pointer_events_to_the_grabbing_widget() {
    let mut harness = Harness::new(400.0, 300.0);
    let a_log = Log::default();
    let panel = harness.insert(Box::new(Panel::new()));
    let a = harness.insert(Box::new(Leaf::new(50.0, 20.0).captures().log(&a_log)));
    let b = harness.insert(Box::new(Leaf::new(60.0, 30.0)));
    harness.add(panel, a);
    harness.add(panel, b);
    assert!(harness.wm.set_root_id(harness.win, panel));
    harness.render();

    harness.mouse_down(10.0, 5.0);
    {
        let window = harness.wm.window(harness.win).unwrap();
        assert_eq!(window.interaction().captured, Some(a));
        assert_eq!(window.interaction().pressed, Some(a));
    }

    a_log.clear();
    harness.mouse_move(300.0, 200.0);
    assert_eq!(
        a_log.count("MouseMove"),
        1,
        "the grab keeps events flowing to the press target: {:?}",
        a_log.entries()
    );

    // Release ends the capture.
    harness.wm.handle_event(
        harness.win,
        UiEvent::MouseUp {
            button: MouseButton::Left,
            position: Point::new(300.0, 200.0),
        },
    );
    assert!(harness
        .wm
        .window(harness.win)
        .unwrap()
        .interaction()
        .pressed
        .is_none());
    a_log.clear();
    harness.mouse_move(300.0, 200.0);
    assert_eq!(a_log.count("MouseMove"), 0, "capture is over");
}

// ---------------------------------------------------------------------------
// overlays
// ---------------------------------------------------------------------------

#[test]
fn overlays_are_arranged_painted_and_hit_above_the_content() {
    let mut harness = Harness::new(400.0, 300.0);
    let paint_order = Log::default();
    let panel = harness.insert(Box::new(Panel::new().paint_log(&paint_order)));
    let a = harness.insert(Box::new(Leaf::new(50.0, 20.0)));
    harness.add(panel, a);
    assert!(harness.wm.set_root_id(harness.win, panel));

    // A dialog is a root of its own, outside the content tree.
    let dialog = harness.insert(Box::new(Leaf::new(40.0, 20.0).paint_log(&paint_order)));
    assert!(harness.wm.add_overlay(harness.win, dialog, 1.0));

    harness.render();

    assert_rect(harness.bounds(dialog), Rect::new(0.0, 0.0, 40.0, 20.0));
    assert_eq!(
        paint_order.entries(),
        vec!["panel".to_string(), "leaf".to_string()],
        "the overlay is painted after the content"
    );
    assert_eq!(
        harness.wm.hit_test(harness.win, Point::new(10.0, 5.0)),
        Some(dialog),
        "the overlay is hit before the content"
    );

    assert!(harness.wm.remove_overlay(harness.win, dialog));
    assert_eq!(
        harness.wm.hit_test(harness.win, Point::new(10.0, 5.0)),
        Some(a),
        "without the overlay the content is hit again"
    );
}

/// A widget asks for the popup layer while it is painted, and the window runs that layer after
/// the whole tree — the guarantee an open drop down panel needs (T-4).
#[test]
fn the_popup_layer_is_painted_after_the_content_tree() {
    let mut harness = Harness::new(400.0, 300.0);
    let paint_order = Log::default();
    let panel = harness.insert(Box::new(Panel::new().paint_log(&paint_order)));
    let leaf = harness.insert(Box::new(
        Leaf::new(50.0, 20.0).paint_log(&paint_order).popup(),
    ));
    harness.add(panel, leaf);
    assert!(harness.wm.set_root_id(harness.win, panel));

    harness.render();
    assert_eq!(
        paint_order.entries(),
        vec!["panel".to_string(), "leaf".to_string(), "popup".to_string()],
        "the popup layer runs after the content tree"
    );

    // The layer is drained every frame: a second frame asks for it once more, not twice.
    harness.render();
    assert_eq!(paint_order.count("popup"), 2, "one popup layer per frame");
}

// ---------------------------------------------------------------------------
// cursor and closing
// ---------------------------------------------------------------------------

#[test]
fn the_cursor_follows_the_hovered_widget() {
    let mut harness = Harness::new(400.0, 300.0);
    let panel = harness.insert(Box::new(Panel::new()));
    let a = harness.insert(Box::new(Leaf::new(50.0, 20.0).cursor(CursorIcon::Text)));
    let b = harness.insert(Box::new(Leaf::new(60.0, 30.0)));
    harness.add(panel, a);
    harness.add(panel, b);
    assert!(harness.wm.set_root_id(harness.win, panel));
    harness.render();

    harness.mouse_move(10.0, 5.0);
    assert_eq!(harness.cursor(), CursorIcon::Text);
    harness.mouse_move(60.0, 5.0);
    assert_eq!(harness.cursor(), CursorIcon::Default);
}

#[test]
fn the_input_method_area_is_asked_of_the_focused_widget() {
    let mut harness = Harness::new(400.0, 300.0);
    let panel = harness.insert(Box::new(Panel::new()));
    // `field` behaves like a text field, `plain` does not edit text at all.
    let caret = Rect::new(12.0, 4.0, 2.0, 16.0);
    let field = harness.insert(Box::new(Leaf::new(80.0, 20.0).focusable().ime(caret)));
    let plain = harness.insert(Box::new(Leaf::new(80.0, 20.0).focusable()));
    harness.add(panel, field);
    harness.add(panel, plain);
    assert!(harness.wm.set_root_id(harness.win, panel));
    harness.render();

    assert!(
        harness.wm.ime_area(harness.win).is_none(),
        "nothing is focused yet: no input method"
    );

    assert!(harness.wm.set_focus(harness.win, field));
    assert_eq!(harness.wm.ime_area(harness.win), Some(caret));

    assert!(harness.wm.set_focus(harness.win, plain));
    assert!(
        harness.wm.ime_area(harness.win).is_none(),
        "a widget that does not edit text wants no input method"
    );

    // An unknown window has no focused widget and no area.
    assert!(harness.wm.ime_area(WindowId::INVALID).is_none());
}

#[test]
fn closing_a_window_is_reported_unless_a_widget_vetoes_it() {
    let mut harness = Harness::new(400.0, 300.0);
    let (panel, _a, _b) = build(&mut harness);
    harness.render();

    let outcome = harness
        .wm
        .handle_event(harness.win, UiEvent::CloseRequested);
    assert!(outcome.close_requested, "nobody vetoed the close");
    assert!(harness.wm.close_requested(harness.win));
    assert!(harness.wm.take_close_request(harness.win));
    assert!(!harness.wm.close_requested(harness.win));

    // A widget (a "save changes?" dialog) may consume the request.
    harness.wm.destroy(panel);
    let panel = harness.insert(Box::new(Panel::new().handles()));
    assert!(harness.wm.set_root_id(harness.win, panel));
    harness.render();
    let outcome = harness
        .wm
        .handle_event(harness.win, UiEvent::CloseRequested);
    assert!(!outcome.close_requested);
    assert!(!harness.wm.close_requested(harness.win));
}

#[test]
fn destroying_a_window_removes_its_tree() {
    let mut harness = Harness::new(400.0, 300.0);
    let (panel, a, b) = build(&mut harness);
    harness.render();
    let objects_before = harness.wm.store().len();

    assert!(harness.wm.destroy_window(harness.win));
    assert!(harness.wm.window(harness.win).is_none());
    assert!(!harness.wm.store().contains(panel));
    assert!(!harness.wm.store().contains(a));
    assert!(!harness.wm.store().contains(b));
    assert!(harness.wm.store().len() < objects_before);
    assert!(!harness.wm.destroy_window(harness.win), "already gone");
}
