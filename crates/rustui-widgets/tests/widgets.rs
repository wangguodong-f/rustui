//! Widget level tests.
//!
//! The harness measures, arranges, paints and dispatches events the way `rustui-window` does, so
//! every widget is exercised without a real window: `deliver` walks the target and then its
//! ancestors (bubbling) until an event is accepted, exactly like the window dispatcher.

use std::cell::Cell;
use std::rc::Rc;

use rustui_core::widget::Object;
use rustui_core::{
    impl_widget_base, ArrangeContext, Color, CrossAxisAlign, EventContext, EventQueue, EventResult,
    FocusManager, FocusReason, Interaction, Key, MeasureContext, MemoryClipboard, Modifiers,
    MouseButton, NamedKey, ObjectId, ObjectRegistry, OverlayList, PaintCommand, PaintContext,
    Painter, Point, Rect, ScrollPhase, SharedProperty, Size, TextAlign, TextLayout, TextMeasurer,
    TextRole, TextStyle, Transform, UiContext, UiEvent, Vec2, Widget, WidgetBase, WidgetStore,
};
use rustui_layout::{HBox, VBox};
use rustui_widgets::{
    Button, ButtonVariant, CheckBox, ComboBox, Dialog, Label, Menu, MenuItem, ProgressBar,
    ScrollView, TextInput,
};

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

/// A fixed size leaf used as scrollable / arrangeable content.
#[derive(Debug)]
struct Leaf {
    base: WidgetBase,
    size: Size,
}

impl Leaf {
    fn new(width: f32, height: f32) -> Self {
        Self {
            base: WidgetBase::new(),
            size: Size::new(width, height),
        }
    }
}

impl_widget_base!(Leaf);

impl Widget for Leaf {
    fn kind(&self) -> &'static str {
        "Leaf"
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
    /// A `VBox` root of `width × height` with a stub text engine and an in memory clipboard.
    fn new(width: f32, height: f32) -> Self {
        Self::with_root(Box::new(VBox::new()), width, height)
    }

    /// The same, with an application supplied root widget.
    fn with_root(root: Box<dyn Widget>, width: f32, height: f32) -> Self {
        let mut ui = UiContext::new(Box::new(StubText));
        ui.set_clipboard(Some(Box::new(MemoryClipboard::default())));
        let store = WidgetStore::new();
        let mut registry = ObjectRegistry::new();
        let root = store.insert(&mut registry, root);
        Self {
            ui,
            store,
            registry,
            interaction: Interaction {
                viewport: Rect::new(0.0, 0.0, width, height),
                ..Interaction::default()
            },
            focus: FocusManager::new(),
            queue: EventQueue::new(),
            overlays: OverlayList::new(),
            consumed: Cell::new(false),
            root,
            viewport: Rect::new(0.0, 0.0, width, height),
        }
    }

    fn insert(&mut self, widget: Box<dyn Widget>) -> ObjectId {
        self.store.insert(&mut self.registry, widget)
    }

    fn add(&mut self, parent: ObjectId, child: ObjectId) {
        let mut widget = self.store.take(parent).expect("parent exists");
        widget.base_mut().add_child(child);
        self.store.put(parent, widget);
        self.registry.set_parent(child, Some(parent)).ok();
    }

    fn layout(&mut self, available: Size) {
        self.interaction.viewport = self.viewport;
        {
            let ctx = MeasureContext::new(&self.ui, &self.store, &self.interaction, 0);
            ctx.measure_child(self.root, available);
        }
        let rect = self.viewport;
        let ctx = ArrangeContext::new(&self.ui, &self.store, &self.interaction, 0);
        ctx.arrange_child(self.root, rect);
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

    /// Paints the whole tree and returns the recorded commands.
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
            // The popup layer runs last, exactly like `Window::paint` (T-4).
            for id in self.overlays.take_popups() {
                ctx.paint_popup(id);
            }
        }
        self.store.put(root, widget);
        painter
    }

    fn clipboard(&self) -> Option<String> {
        self.ui
            .clipboard()
            .and_then(|clipboard| clipboard.get_text())
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
        self.interaction.pressed = Some(target);
        self.deliver(
            target,
            &UiEvent::MouseUp {
                button: MouseButton::Left,
                position: Point::new(x, y),
            },
        );
        self.interaction.pressed = None;
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

    fn shortcut(&mut self, target: ObjectId, key: &str, modifiers: Modifiers) {
        self.deliver(
            target,
            &UiEvent::KeyDown {
                key: Key::Character(key.to_string()),
                modifiers,
                text: None,
                repeat: false,
            },
        );
    }

    fn type_text(&mut self, target: ObjectId, text: &str) {
        self.deliver(target, &UiEvent::TextInput(text.to_string()));
    }
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

/// Number of filled / bordered rectangle commands.
fn shape_count(painter: &Painter) -> usize {
    painter
        .commands()
        .iter()
        .filter(|command| matches!(command, PaintCommand::Shape { .. }))
        .count()
}

/// Input method anchor `id` reports, in window coordinates (T-1).
fn ime_area(tree: &Tree, id: ObjectId) -> Option<Rect> {
    tree.store
        .get(id)
        .expect("widget exists")
        .ime_cursor_area(&tree.ui)
}

/// Rectangles that were pushed as clips.
fn clip_rects(painter: &Painter) -> Vec<Rect> {
    painter
        .commands()
        .iter()
        .filter_map(|command| match command {
            PaintCommand::PushClip { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Label
// ---------------------------------------------------------------------------

#[test]
fn a_centred_label_is_painted_in_the_middle_of_its_box() {
    // A stretching root gives the label the full width, so centring is observable.
    let mut tree = Tree::with_root(
        Box::new(VBox::new().with_cross_align(CrossAxisAlign::Stretch)),
        400.0,
        300.0,
    );
    let label = tree.insert(Box::new(Label::new("Hello").align(TextAlign::Center)));
    tree.add(tree.root, label);
    tree.layout(Size::new(400.0, 300.0));

    // The stub engine measures "Hello" as 5 × 7 = 35 px wide, the box is 400 wide.
    assert_eq!(tree.measured(label), Size::new(35.0, 14.0));
    assert_eq!(tree.bounds(label).width, 400.0);
    let painter = tree.paint();
    assert_eq!(painted_texts(&painter), vec!["Hello".to_string()]);
    let position = painter
        .commands()
        .iter()
        .find_map(|command| match command {
            PaintCommand::Text { position, .. } => Some(*position),
            _ => None,
        })
        .expect("the label painted text");
    let expected = (400.0 - 35.0) * 0.5;
    assert!(
        (position.x - expected).abs() < 0.01,
        "expected x={expected}, got {position:?}"
    );
}

#[test]
fn a_label_bounded_by_its_parent_never_overflows_it() {
    let mut tree = Tree::new(40.0, 300.0);
    let label = tree.insert(Box::new(Label::new("a very long label indeed")));
    tree.add(tree.root, label);
    tree.layout(Size::new(40.0, 300.0));
    assert!(
        tree.measured(label).width <= 40.0,
        "measured {:?}",
        tree.measured(label)
    );
}

#[test]
fn a_wrapping_label_fills_the_width_it_is_offered() {
    let mut tree = Tree::new(120.0, 300.0);
    let label = tree.insert(Box::new(Label::new("wrap me please").wrap(true)));
    tree.add(tree.root, label);
    tree.layout(Size::new(120.0, 300.0));
    assert_eq!(tree.measured(label).width, 120.0);
}

/// Rotation rides on the text command itself (T-8 / C-049); an unrotated label must not change.
#[test]
fn a_rotated_label_ships_its_rotation_with_the_text() {
    let transform_of = |painter: &Painter| {
        painter
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::Text { transform, .. } => Some(*transform),
                _ => None,
            })
            .expect("the label painted text")
    };

    let mut tree = Tree::new(200.0, 100.0);
    let label = tree.insert(Box::new(Label::new("Tilted").rotation(30.0)));
    tree.add(tree.root, label);
    tree.layout(Size::new(200.0, 100.0));
    assert_eq!(transform_of(&tree.paint()), Transform::rotated(30.0));

    let mut tree = Tree::new(200.0, 100.0);
    let label = tree.insert(Box::new(Label::new("Flat")));
    tree.add(tree.root, label);
    tree.layout(Size::new(200.0, 100.0));
    assert_eq!(transform_of(&tree.paint()), Transform::IDENTITY);
}

// ---------------------------------------------------------------------------
// Button
// ---------------------------------------------------------------------------

#[test]
fn pressing_and_releasing_a_button_emits_exactly_one_click() {
    let mut tree = Tree::new(400.0, 300.0);
    let clicks = counter();
    let counter = clicks.clone();
    let button = Button::new("OK");
    button
        .clicked()
        .connect(move |_| counter.set(counter.get() + 1));
    let button = tree.insert(Box::new(button));
    tree.add(tree.root, button);
    tree.layout(Size::new(400.0, 300.0));
    let bounds = tree.bounds(button);

    tree.click(button, bounds.x + 5.0, bounds.y + 5.0);
    assert_eq!(clicks.get(), 1, "one press and one release = one click");

    // A release outside must not click.
    tree.deliver(
        button,
        &UiEvent::MouseDown {
            button: MouseButton::Left,
            position: Point::new(bounds.x + 5.0, bounds.y + 5.0),
        },
    );
    tree.deliver(
        button,
        &UiEvent::MouseUp {
            button: MouseButton::Left,
            position: Point::new(390.0, 290.0),
        },
    );
    assert_eq!(clicks.get(), 1, "released outside the button");
}

#[test]
fn a_disabled_button_is_inert() {
    let mut tree = Tree::new(400.0, 300.0);
    let clicks = counter();
    let counter = clicks.clone();
    let mut button = Button::new("OK");
    button
        .clicked()
        .connect(move |_| counter.set(counter.get() + 1));
    let mut state = button.base().state();
    state.enabled = false;
    button.base_mut().set_state(state);
    let button = tree.insert(Box::new(button));
    tree.add(tree.root, button);
    tree.layout(Size::new(400.0, 300.0));

    let bounds = tree.bounds(button);
    let consumed = tree.deliver(
        button,
        &UiEvent::MouseDown {
            button: MouseButton::Left,
            position: Point::new(bounds.x + 5.0, bounds.y + 5.0),
        },
    );
    assert!(!consumed, "a disabled widget must not consume the press");
    assert_eq!(clicks.get(), 0);
}

#[test]
fn enter_activates_a_focused_button() {
    let mut tree = Tree::new(400.0, 300.0);
    let clicks = counter();
    let counter = clicks.clone();
    let button = Button::new("OK");
    button
        .clicked()
        .connect(move |_| counter.set(counter.get() + 1));
    let button = tree.insert(Box::new(button));
    tree.add(tree.root, button);
    tree.layout(Size::new(400.0, 300.0));

    tree.focus(button);
    tree.key(button, NamedKey::Return, Modifiers::NONE);
    assert_eq!(clicks.get(), 1);
}

#[test]
fn an_unhandled_press_bubbles_to_the_parent() {
    let mut tree = Tree::new(400.0, 300.0);
    let leaf = tree.insert(Box::new(Leaf::new(50.0, 20.0)));
    tree.add(tree.root, leaf);
    tree.layout(Size::new(400.0, 300.0));
    let bounds = tree.bounds(leaf);
    // The leaf handles nothing, so the event walks up to the root and is not consumed.
    let consumed = tree.deliver(
        leaf,
        &UiEvent::MouseDown {
            button: MouseButton::Left,
            position: Point::new(bounds.x + 1.0, bounds.y + 1.0),
        },
    );
    assert!(!consumed);
}

// ---------------------------------------------------------------------------
// CheckBox
// ---------------------------------------------------------------------------

#[test]
fn a_check_box_toggles_on_click_and_reports_it() {
    let mut tree = Tree::new(400.0, 300.0);
    let on_events = counter();
    let counter = on_events.clone();
    let check = CheckBox::new("Wrap");
    check.toggled().connect(move |value| {
        if *value {
            counter.set(counter.get() + 1);
        }
    });
    let check = tree.insert(Box::new(check));
    tree.add(tree.root, check);
    tree.layout(Size::new(400.0, 300.0));

    let bounds = tree.bounds(check);
    tree.click(check, bounds.x + 4.0, bounds.y + 4.0);
    assert_eq!(on_events.get(), 1, "checking reports `true` once");

    tree.click(check, bounds.x + 4.0, bounds.y + 4.0);
    assert_eq!(on_events.get(), 1, "unchecking must not report `true`");
}

#[test]
fn space_toggles_a_focused_check_box() {
    let mut tree = Tree::new(400.0, 300.0);
    let toggles = counter();
    let counter = toggles.clone();
    let check = CheckBox::new("Wrap");
    check
        .toggled()
        .connect(move |_| counter.set(counter.get() + 1));
    let check = tree.insert(Box::new(check));
    tree.add(tree.root, check);
    tree.layout(Size::new(400.0, 300.0));

    tree.focus(check);
    tree.key(check, NamedKey::Space, Modifiers::NONE);
    assert_eq!(toggles.get(), 1);
}

// ---------------------------------------------------------------------------
// TextInput
// ---------------------------------------------------------------------------

/// Builds a focused text input bound to a shared text handle.
fn input_tree() -> (Tree, ObjectId, SharedProperty<String>) {
    let mut tree = Tree::new(400.0, 300.0);
    let input = TextInput::new().placeholder("Your name");
    let text = input.text_property();
    let input = tree.insert(Box::new(input));
    tree.add(tree.root, input);
    tree.layout(Size::new(400.0, 300.0));
    tree.focus(input);
    (tree, input, text)
}

#[test]
fn typing_appends_at_the_caret() {
    let (mut tree, input, text) = input_tree();
    tree.type_text(input, "Hi");
    assert_eq!(text.get(), "Hi");

    // Move to the start and insert in front: the caret really moved.
    tree.key(input, NamedKey::Home, Modifiers::NONE);
    tree.type_text(input, "!");
    assert_eq!(text.get(), "!Hi");

    tree.key(input, NamedKey::End, Modifiers::NONE);
    tree.type_text(input, "?");
    assert_eq!(text.get(), "!Hi?");
}

#[test]
fn select_all_plus_backspace_clears_the_field() {
    let (mut tree, input, text) = input_tree();
    tree.type_text(input, "hello");
    tree.shortcut(input, "a", Modifiers::CTRL);
    tree.key(input, NamedKey::Backspace, Modifiers::NONE);
    assert_eq!(text.get(), "", "the whole selection was replaced");
}

#[test]
fn copy_and_paste_round_trip_through_the_clipboard() {
    let (mut tree, input, text) = input_tree();
    tree.type_text(input, "abc");
    tree.shortcut(input, "a", Modifiers::CTRL);
    tree.shortcut(input, "c", Modifiers::CTRL);
    assert_eq!(tree.clipboard().as_deref(), Some("abc"));

    tree.key(input, NamedKey::End, Modifiers::NONE);
    tree.shortcut(input, "v", Modifiers::CTRL);
    assert_eq!(text.get(), "abcabc");
}

#[test]
fn shift_arrow_extends_the_selection_and_replaces_it() {
    let (mut tree, input, text) = input_tree();
    tree.type_text(input, "abcd");
    tree.key(input, NamedKey::Home, Modifiers::NONE);
    for _ in 0..2 {
        tree.deliver(
            input,
            &UiEvent::KeyDown {
                key: Key::Named(NamedKey::ArrowRight),
                modifiers: Modifiers::SHIFT,
                text: None,
                repeat: false,
            },
        );
    }
    tree.type_text(input, "-");
    assert_eq!(text.get(), "-cd", "two shifted arrows selected 0..2");
}

#[test]
fn backspace_deletes_one_character_at_a_time() {
    let (mut tree, input, text) = input_tree();
    tree.type_text(input, "abc");
    tree.key(input, NamedKey::Backspace, Modifiers::NONE);
    assert_eq!(text.get(), "ab");
    tree.key(input, NamedKey::Delete, Modifiers::NONE);
    assert_eq!(
        text.get(),
        "ab",
        "the caret is at the end: nothing to delete"
    );
    tree.key(input, NamedKey::Home, Modifiers::NONE);
    tree.key(input, NamedKey::Delete, Modifiers::NONE);
    assert_eq!(text.get(), "b", "the first character was removed");
}

#[test]
fn an_empty_field_paints_its_placeholder() {
    let mut tree = Tree::new(400.0, 300.0);
    let input = tree.insert(Box::new(TextInput::new().placeholder("Your name")));
    tree.add(tree.root, input);
    tree.layout(Size::new(400.0, 300.0));
    let painter = tree.paint();
    assert_eq!(painted_texts(&painter), vec!["Your name".to_string()]);
}

#[test]
fn ime_composition_is_committed_as_text() {
    let (mut tree, input, text) = input_tree();
    tree.deliver(input, &UiEvent::ImeStart);
    tree.deliver(
        input,
        &UiEvent::ImeUpdate {
            text: "ni".to_string(),
            cursor: None,
        },
    );
    tree.deliver(input, &UiEvent::ImeCommit("ni".to_string()));
    assert_eq!(text.get(), "ni");
}

#[test]
fn a_focused_input_paints_a_caret_and_a_focus_ring() {
    let (mut tree, input, text) = input_tree();
    tree.type_text(input, "value");
    let painter = tree.paint();
    assert!(painted_texts(&painter).contains(&text.get()));
    // Background + caret + focus ring at the very least.
    assert!(
        shape_count(&painter) >= 3,
        "expected background, caret and focus ring, got {} shapes",
        shape_count(&painter)
    );
}

#[test]
fn a_password_field_masks_its_content() {
    let mut tree = Tree::new(400.0, 300.0);
    let input = TextInput::new().password(true);
    let text = input.text_property();
    let input = tree.insert(Box::new(input));
    tree.add(tree.root, input);
    tree.layout(Size::new(400.0, 300.0));
    tree.focus(input);
    tree.type_text(input, "secret");

    assert_eq!(text.get(), "secret", "the value stays readable for the app");
    let painter = tree.paint();
    let painted = painted_texts(&painter);
    assert!(
        !painted.iter().any(|value| value == "secret"),
        "the mask must hide the value: {painted:?}"
    );
}

#[test]
fn the_input_method_follows_the_caret_and_the_focus() {
    let (mut tree, input, _text) = input_tree();

    // A focused field anchors the input method to its caret.
    let at_start = ime_area(&tree, input).expect("a focused field anchors the input method");
    let field = tree.bounds(input);
    assert!(
        field.contains(Point::new(at_start.x, at_start.y)),
        "the caret rectangle must start inside the field: {at_start:?} vs {field:?}"
    );
    assert!(
        at_start.width >= 1.0 && at_start.height > 0.0,
        "a degenerate rectangle is ignored by compositors: {at_start:?}"
    );

    // Typing moves the caret, and the candidate window with it.
    tree.type_text(input, "abc");
    let after_text = ime_area(&tree, input).expect("still focused");
    assert!(
        after_text.x > at_start.x,
        "the candidate window follows the caret: {at_start:?} → {after_text:?}"
    );

    // Leaving the field switches the input method off instead of leaving it somewhere stale.
    tree.deliver(input, &UiEvent::FocusOut);
    assert!(
        ime_area(&tree, input).is_none(),
        "a field without focus wants no input method"
    );
}

// ---------------------------------------------------------------------------
// ScrollView
// ---------------------------------------------------------------------------

#[test]
fn the_wheel_scrolls_the_content_and_clamps_at_the_end() {
    let mut tree = Tree::new(200.0, 100.0);
    let content = tree.insert(Box::new(Leaf::new(200.0, 500.0)));
    let scroll = tree.insert(Box::new(ScrollView::new().child(content)));
    tree.add(tree.root, scroll);
    tree.layout(Size::new(200.0, 100.0));

    assert_eq!(tree.bounds(content), Rect::new(0.0, 0.0, 200.0, 500.0));

    tree.deliver(
        scroll,
        &UiEvent::MouseWheel {
            delta: Vec2::new(0.0, -50.0),
            phase: ScrollPhase::Move,
        },
    );
    // Geometry is recomputed by the next layout pass, exactly like the window does per frame.
    tree.layout(Size::new(200.0, 100.0));
    assert_eq!(tree.bounds(content).y, -50.0, "the content moved up");

    tree.deliver(
        scroll,
        &UiEvent::MouseWheel {
            delta: Vec2::new(0.0, -10_000.0),
            phase: ScrollPhase::Move,
        },
    );
    tree.layout(Size::new(200.0, 100.0));
    assert_eq!(
        tree.bounds(content).y,
        -400.0,
        "clamped to content - viewport"
    );
}

#[test]
fn a_scroll_view_clips_its_content_to_the_viewport() {
    let mut tree = Tree::new(200.0, 100.0);
    let content = tree.insert(Box::new(Leaf::new(200.0, 500.0)));
    let scroll = tree.insert(Box::new(ScrollView::new().child(content)));
    tree.add(tree.root, scroll);
    tree.layout(Size::new(200.0, 100.0));

    let painter = tree.paint();
    let clips = clip_rects(&painter);
    assert!(
        clips
            .iter()
            .any(|rect| rect.height <= 100.0 && rect.width <= 200.0),
        "the content must be clipped to the viewport: {clips:?}"
    );
    assert!(shape_count(&painter) >= 1, "the content still paints");
}

// ---------------------------------------------------------------------------
// ProgressBar
// ---------------------------------------------------------------------------

#[test]
fn the_progress_value_is_clamped_and_painted() {
    let mut tree = Tree::new(400.0, 300.0);
    let bar = ProgressBar::new(0.25);
    let value = bar.value_property();
    let bar = tree.insert(Box::new(bar));
    tree.add(tree.root, bar);
    tree.layout(Size::new(400.0, 300.0));

    assert_eq!(value.get(), 0.25);
    let painter = tree.paint();
    assert!(shape_count(&painter) >= 2, "track and fill are two shapes");

    // The property itself is a plain float; the *widget* clamps what it paints, so an absurd
    // value must never paint outside of its own rectangle.
    value.set(5.0);
    let bounds = tree.bounds(bar);
    let painter = tree.paint();
    assert!(painter.commands().iter().any(|command| match command {
        PaintCommand::Shape { rect, .. } => rect.width > 1.0 && rect.width <= bounds.width + 0.5,
        _ => false,
    }));
    value.set(-1.0);
    let painter = tree.paint();
    assert!(
        !painter.commands().iter().any(|command| match command {
            PaintCommand::Shape { rect, .. } => rect.width > 1.0 && rect.x > bounds.x + 1.0,
            _ => false,
        }),
        "a zero value paints no fill"
    );
}

// ---------------------------------------------------------------------------
// ComboBox
// ---------------------------------------------------------------------------

#[test]
fn a_combo_box_opens_selects_and_closes() {
    let mut tree = Tree::new(400.0, 300.0);
    let picks = counter();
    let counter = picks.clone();
    let combo = ComboBox::new(["Red", "Green", "Blue"]);
    combo.selection_changed().connect(move |index| {
        if index.is_some() {
            counter.set(counter.get() + 1);
        }
    });
    let combo = tree.insert(Box::new(combo));
    tree.add(tree.root, combo);
    tree.layout(Size::new(400.0, 300.0));

    let bounds = tree.bounds(combo);
    let below = Point::new(bounds.x + 4.0, bounds.bottom() + 4.0);
    {
        let widget = tree.store.get(combo).expect("combo exists");
        assert!(
            !widget.hit_test(below),
            "the closed combo must not claim the area below"
        );
    }

    // Open it.
    tree.click(combo, bounds.x + 4.0, bounds.y + bounds.height * 0.5);
    {
        let widget = tree.store.get(combo).expect("combo exists");
        assert!(widget.hit_test(below), "the open popup is hit testable");
    }

    // Click the second row of the popup.
    let row_y = bounds.bottom() + tree.ui.theme().control_height() * 1.5;
    tree.deliver(
        combo,
        &UiEvent::MouseDown {
            button: MouseButton::Left,
            position: Point::new(bounds.x + 4.0, row_y),
        },
    );
    assert_eq!(picks.get(), 1, "one option was chosen");
    {
        let widget = tree.store.get(combo).expect("combo exists");
        assert!(!widget.hit_test(below), "the popup closed again");
    }
}

#[test]
fn a_combo_box_paints_its_options_while_open() {
    let mut tree = Tree::new(400.0, 300.0);
    let combo = tree.insert(Box::new(ComboBox::new(["Red", "Green", "Blue"])));
    tree.add(tree.root, combo);
    tree.layout(Size::new(400.0, 300.0));
    let bounds = tree.bounds(combo);

    let before = {
        let painter = tree.paint();
        painted_texts(&painter)
    };
    assert!(before.is_empty(), "nothing selected, nothing painted");

    tree.click(combo, bounds.x + 4.0, bounds.y + bounds.height * 0.5);
    let after = {
        let painter = tree.paint();
        painted_texts(&painter)
    };
    for expected in ["Red", "Green", "Blue"] {
        assert!(
            after.contains(&expected.to_string()),
            "missing {expected:?} in {after:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Menu
// ---------------------------------------------------------------------------

#[test]
fn a_menu_activates_the_highlighted_item_and_hides_itself() {
    let mut tree = Tree::new(400.0, 300.0);
    let fired = counter();
    let closes = counter();
    let (f_counter, c_counter) = (fired.clone(), closes.clone());
    let menu = Menu::new([
        MenuItem::new("Open"),
        MenuItem::separator(),
        MenuItem::new("Quit").disabled(),
    ]);
    menu.activated()
        .connect(move |_| f_counter.set(f_counter.get() + 1));
    menu.closed()
        .connect(move |_| c_counter.set(c_counter.get() + 1));
    let menu = tree.insert(Box::new(menu));
    tree.add(tree.root, menu);
    tree.layout(Size::new(400.0, 300.0));
    tree.focus(menu);

    tree.key(menu, NamedKey::ArrowDown, Modifiers::NONE);
    assert_eq!(fired.get(), 0, "highlighting is not activating");

    tree.key(menu, NamedKey::Return, Modifiers::NONE);
    assert_eq!(fired.get(), 1, "Enter activates the highlighted item");
    assert_eq!(closes.get(), 1, "the menu hid itself");
}

#[test]
fn a_disabled_menu_item_is_never_highlighted() {
    let mut tree = Tree::new(400.0, 300.0);
    let fired = counter();
    let counter = fired.clone();
    let menu = Menu::new([MenuItem::new("First"), MenuItem::new("Gone").disabled()]);
    menu.activated()
        .connect(move |_| counter.set(counter.get() + 1));
    let menu = tree.insert(Box::new(menu));
    tree.add(tree.root, menu);
    tree.layout(Size::new(400.0, 300.0));
    tree.focus(menu);

    // Walk past the disabled item and activate whatever is highlighted.
    tree.key(menu, NamedKey::ArrowDown, Modifiers::NONE);
    tree.key(menu, NamedKey::ArrowDown, Modifiers::NONE);
    tree.key(menu, NamedKey::Return, Modifiers::NONE);
    assert_eq!(fired.get(), 1, "only the enabled item fired");
}

#[test]
fn escape_closes_a_menu_without_activating_anything() {
    let mut tree = Tree::new(400.0, 300.0);
    let fired = counter();
    let closes = counter();
    let (f_counter, c_counter) = (fired.clone(), closes.clone());
    let menu = Menu::new([MenuItem::new("Open")]);
    menu.activated()
        .connect(move |_| f_counter.set(f_counter.get() + 1));
    menu.closed()
        .connect(move |_| c_counter.set(c_counter.get() + 1));
    let menu = tree.insert(Box::new(menu));
    tree.add(tree.root, menu);
    tree.layout(Size::new(400.0, 300.0));
    tree.focus(menu);

    tree.key(menu, NamedKey::Escape, Modifiers::NONE);
    assert_eq!(fired.get(), 0);
    assert_eq!(closes.get(), 1);
}

#[test]
fn a_menu_paints_its_items_and_separators() {
    let mut tree = Tree::new(400.0, 300.0);
    let menu = tree.insert(Box::new(Menu::new([
        MenuItem::new("Cut"),
        MenuItem::separator(),
        MenuItem::new("Paste").shortcut("Ctrl+V"),
    ])));
    tree.add(tree.root, menu);
    tree.layout(Size::new(400.0, 300.0));

    let painter = tree.paint();
    let texts = painted_texts(&painter);
    assert!(texts.contains(&"Cut".to_string()), "{texts:?}");
    assert!(texts.contains(&"Paste".to_string()), "{texts:?}");
    assert!(texts.contains(&"Ctrl+V".to_string()), "{texts:?}");
    assert!(shape_count(&painter) >= 3, "surface and separators");
}

// ---------------------------------------------------------------------------
// Dialog
// ---------------------------------------------------------------------------

#[test]
fn the_close_button_and_escape_ask_the_dialog_to_close() {
    let mut tree = Tree::new(400.0, 300.0);
    let requests = counter();
    let counter = requests.clone();
    let dialog = Dialog::new("Save changes?");
    dialog
        .close_requested()
        .connect(move |_| counter.set(counter.get() + 1));
    let dialog = tree.insert(Box::new(dialog));
    tree.add(tree.root, dialog);
    tree.layout(Size::new(400.0, 300.0));

    let bounds = tree.bounds(dialog);
    let bar = tree.ui.theme().control_height();
    // A click in the middle of the title bar does nothing.
    tree.deliver(
        dialog,
        &UiEvent::MouseDown {
            button: MouseButton::Left,
            position: Point::new(bounds.x + bar, bounds.y + bar * 0.5),
        },
    );
    assert_eq!(requests.get(), 0);

    // The ✕ on the trailing edge of the title bar closes.
    tree.deliver(
        dialog,
        &UiEvent::MouseDown {
            button: MouseButton::Left,
            position: Point::new(bounds.right() - bar * 0.5, bounds.y + bar * 0.5),
        },
    );
    assert_eq!(requests.get(), 1);

    // Escape asks too.
    tree.focus(dialog);
    tree.key(dialog, NamedKey::Escape, Modifiers::NONE);
    assert_eq!(requests.get(), 2);
}

#[test]
fn a_dialog_sizes_itself_around_its_content() {
    let mut tree = Tree::new(400.0, 300.0);
    let content = tree.insert(Box::new(Leaf::new(120.0, 60.0)));
    let dialog = tree.insert(Box::new(Dialog::new("Title").child(content)));
    tree.add(tree.root, dialog);
    tree.layout(Size::new(400.0, 300.0));

    let theme = tree.ui.theme().clone();
    let pad = theme.content_padding();
    // Width = content + padding, but never narrower than four control heights (a title needs room).
    let expected_width = (120.0 + pad.horizontal()).max(theme.control_height() * 4.0);
    assert_eq!(
        tree.measured(dialog),
        Size::new(
            expected_width,
            60.0 + pad.vertical() + theme.control_height()
        )
    );
    // The content sits below the title bar, at the padding offset.
    let dialog_bounds = tree.bounds(dialog);
    let content_bounds = tree.bounds(content);
    assert!((content_bounds.x - (dialog_bounds.x + pad.left)).abs() < 0.01);
    assert!((content_bounds.y - (dialog_bounds.y + theme.control_height() + pad.top)).abs() < 0.01);
}

#[test]
fn a_dialog_can_override_its_default_minimum_width() {
    // An override above both the natural width and the default floor wins (T-7 / C-050).
    let mut tree = Tree::new(400.0, 300.0);
    let content = tree.insert(Box::new(Leaf::new(120.0, 60.0)));
    let dialog = tree.insert(Box::new(
        Dialog::new("Title").child(content).min_width(260.0),
    ));
    tree.add(tree.root, dialog);
    tree.layout(Size::new(400.0, 300.0));
    let pad = tree.ui.theme().content_padding();
    assert_eq!(tree.measured(dialog).width, 260.0);

    // A floor below the content never shrinks the dialog.
    let mut tree = Tree::new(400.0, 300.0);
    let content = tree.insert(Box::new(Leaf::new(120.0, 60.0)));
    let dialog = tree.insert(Box::new(
        Dialog::new("Title").child(content).min_width(40.0),
    ));
    tree.add(tree.root, dialog);
    tree.layout(Size::new(400.0, 300.0));
    assert_eq!(tree.measured(dialog).width, 120.0 + pad.horizontal());
}

#[test]
fn a_dialog_paints_its_title_and_its_content() {
    let mut tree = Tree::new(400.0, 300.0);
    let content = tree.insert(Box::new(Label::new("Body text")));
    let dialog = tree.insert(Box::new(Dialog::new("Title").child(content)));
    tree.add(tree.root, dialog);
    tree.layout(Size::new(400.0, 300.0));

    let painter = tree.paint();
    let texts = painted_texts(&painter);
    assert!(texts.contains(&"Title".to_string()), "{texts:?}");
    assert!(texts.contains(&"Body text".to_string()), "{texts:?}");
}

// ---------------------------------------------------------------------------
// end to end
// ---------------------------------------------------------------------------

#[test]
fn a_form_paints_every_widget_it_contains() {
    let mut tree = Tree::new(400.0, 300.0);
    let title = tree.insert(Box::new(Label::new("Sign up").role(TextRole::Title)));
    let input = tree.insert(Box::new(TextInput::new().placeholder("Name")));
    let row = tree.insert(Box::new(HBox::new().with_spacing(8.0)));
    let ok = tree.insert(Box::new(Button::new("OK")));
    let cancel = tree.insert(Box::new(
        Button::new("Cancel").variant(ButtonVariant::Secondary),
    ));
    tree.add(row, ok);
    tree.add(row, cancel);
    tree.add(tree.root, title);
    tree.add(tree.root, input);
    tree.add(tree.root, row);
    tree.layout(Size::new(400.0, 300.0));

    let painter = tree.paint();
    let texts = painted_texts(&painter);
    for expected in ["Sign up", "Name", "OK", "Cancel"] {
        assert!(
            texts.contains(&expected.to_string()),
            "missing {expected:?} in {texts:?}"
        );
    }
    // Widgets stack vertically: each one is below the previous.
    let title_bounds = tree.bounds(title);
    let input_bounds = tree.bounds(input);
    let row_bounds = tree.bounds(row);
    assert!(input_bounds.y >= title_bounds.bottom() - 0.01);
    assert!(row_bounds.y >= input_bounds.bottom() - 0.01);
    assert!(row_bounds.width <= 400.0);
}

#[test]
fn measuring_twice_keeps_the_layout_stable() {
    let mut tree = Tree::new(400.0, 300.0);
    let label = tree.insert(Box::new(Label::new("stable")));
    let button = tree.insert(Box::new(Button::new("Go")));
    tree.add(tree.root, label);
    tree.add(tree.root, button);
    tree.layout(Size::new(400.0, 300.0));
    let first_label = tree.measured(label);
    let first_button = tree.measured(button);

    tree.layout(Size::new(400.0, 300.0));
    assert_eq!(tree.measured(label), first_label);
    assert_eq!(tree.measured(button), first_button);
}

/// The open panel lies outside the control's own rectangle, so `paint_child`'s clip must be
/// released while it is drawn — otherwise the renderer would discard everything the panel
/// draws below the control and the drop down would never be visible (T-4).
#[test]
fn an_open_combo_box_panel_escapes_the_clip_of_its_own_bounds() {
    let mut tree = Tree::new(400.0, 300.0);
    let combo = tree.insert(Box::new(ComboBox::new(["Red", "Green", "Blue"])));
    tree.add(tree.root, combo);
    tree.layout(Size::new(400.0, 300.0));
    let bounds = tree.bounds(combo);
    tree.click(combo, bounds.x + 4.0, bounds.y + bounds.height * 0.5);

    let painter = tree.paint();
    // Walk the command stream with our own clip stack, exactly like the renderer does.
    let mut clip = Rect::new(0.0, 0.0, 400.0, 300.0);
    let mut stack: Vec<Rect> = Vec::new();
    let mut panel = None;
    for command in painter.commands() {
        match command {
            PaintCommand::PushClip { rect, .. } => {
                stack.push(clip);
                clip = clip.intersection(*rect);
            }
            PaintCommand::PopClip => {
                if let Some(previous) = stack.pop() {
                    clip = previous;
                }
            }
            PaintCommand::Shape { rect, .. } if rect.y >= bounds.bottom() - 0.5 => {
                panel = Some((*rect, clip));
            }
            _ => {}
        }
    }
    assert!(stack.is_empty(), "the clip stack must be balanced");
    let (panel, active_clip) = panel.expect("the panel was painted");
    assert!(
        panel.bottom() > bounds.bottom() + 0.5,
        "the panel is supposed to overflow the control: {panel:?}"
    );
    assert!(
        panel.y >= active_clip.y - 0.01 && panel.bottom() <= active_clip.bottom() + 0.01,
        "the panel was painted under clip {active_clip:?}, which cuts it: {panel:?}"
    );
}

/// With no room below the control the panel flips above it, and it stays hit testable and
/// clickable there (T-4).
#[test]
fn the_combo_box_popup_flips_above_the_control_when_there_is_no_room_below() {
    let mut tree = Tree::new(400.0, 300.0);
    let control_height = tree.ui.theme().control_height();
    let picks = counter();
    let pick_counter = picks.clone();
    let panel_height = 3.0 * control_height;
    // A spacer that leaves only half a panel of room below — never enough, while the space
    // above stays generous.
    let spacer_height = 300.0 - control_height - panel_height * 0.5;

    let combo = ComboBox::new(["Red", "Green", "Blue"]);
    combo.selection_changed().connect(move |index| {
        if index.is_some() {
            pick_counter.set(pick_counter.get() + 1);
        }
    });
    let spacer = tree.insert(Box::new(Leaf::new(400.0, spacer_height)));
    let combo = tree.insert(Box::new(combo));
    tree.add(tree.root, spacer);
    tree.add(tree.root, combo);
    tree.layout(Size::new(400.0, 300.0));

    let bounds = tree.bounds(combo);
    assert!(
        300.0 - bounds.bottom() < panel_height,
        "the fixture must leave no room below: {bounds:?}"
    );
    tree.click(combo, bounds.x + 4.0, bounds.y + bounds.height * 0.5);

    let painter = tree.paint();
    let panel = painter
        .commands()
        .iter()
        .find_map(|command| match command {
            // The panel itself: same column as the control, three rows tall — the soft shadow
            // behind it shares the size but is offset by a pixel.
            PaintCommand::Shape { rect, .. }
                if (rect.height - panel_height).abs() < 1.0 && (rect.x - bounds.x).abs() < 0.25 =>
            {
                Some(*rect)
            }
            _ => None,
        })
        .expect("the open panel was painted");
    assert!(
        panel.bottom() <= bounds.y + 0.5,
        "the panel must flip above the control: {panel:?}, control {bounds:?}"
    );

    // The flipped panel is the hit target, and clicking it selects the row right above the
    // control.
    let above = Point::new(bounds.x + 4.0, bounds.y - 4.0);
    {
        let widget = tree.store.get(combo).expect("combo exists");
        assert!(
            widget.hit_test(above),
            "the flipped panel is hit testable above the control"
        );
    }
    tree.deliver(
        combo,
        &UiEvent::MouseDown {
            button: MouseButton::Left,
            position: above,
        },
    );
    assert_eq!(picks.get(), 1, "the row above the control was chosen");
    {
        let widget = tree.store.get(combo).expect("combo exists");
        assert!(!widget.hit_test(above), "the panel closed again");
    }
}

/// The panel is drawn above a later sibling whose rectangle covers its area: the popup layer runs
/// after the whole content tree (T-4).
#[test]
fn the_combo_box_panel_is_painted_above_later_siblings() {
    let mut tree = Tree::new(400.0, 300.0);
    let combo = tree.insert(Box::new(ComboBox::new(["Red", "Green", "Blue"])));
    let sibling = tree.insert(Box::new(Leaf::new(400.0, 200.0)));
    tree.add(tree.root, combo);
    tree.add(tree.root, sibling);
    tree.layout(Size::new(400.0, 300.0));

    let bounds = tree.bounds(combo);
    let sibling_bounds = tree.bounds(sibling);
    tree.click(combo, bounds.x + 4.0, bounds.y + bounds.height * 0.5);

    let panel_height = 3.0 * tree.ui.theme().control_height();
    let painter = tree.paint();
    let mut panel_index = None;
    let mut sibling_index = None;
    for (index, command) in painter.commands().iter().enumerate() {
        if let PaintCommand::Shape { rect, .. } = command {
            if (rect.height - panel_height).abs() < 1.0 && (rect.x - bounds.x).abs() < 0.25 {
                panel_index = Some(index);
            }
            if *rect == sibling_bounds {
                sibling_index = Some(index);
            }
        }
    }
    let panel_index = panel_index.expect("the panel was painted");
    let sibling_index = sibling_index.expect("the sibling was painted");
    assert!(
        panel_index > sibling_index,
        "the panel must be drawn after the sibling that covers it (panel #{panel_index}, sibling #{sibling_index})"
    );
}
