//! The widget contract (§6) and the base state every widget owns.

use std::fmt;

use crate::context::{
    ArrangeContext, EventContext, MeasureContext, PaintContext, UiContext, UpdateContext,
};
use crate::event::{EventResult, UiEvent};
use crate::geometry::{Point, Rect, Size, Vec2};
use crate::object::ObjectId;
use crate::paint::TextureId;
use crate::style::WidgetState;

/// Preferred size of a widget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeHint {
    pub width: f32,
    pub height: f32,
    pub min_width: f32,
    pub min_height: f32,
    pub max_width: f32,
    pub max_height: f32,
    /// Relative weights used by `HBox` / `VBox` / `Grid` when distributing free space.
    pub flex: f32,
}

impl Default for SizeHint {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            min_width: 0.0,
            min_height: 0.0,
            max_width: f32::INFINITY,
            max_height: f32::INFINITY,
            flex: 0.0,
        }
    }
}

impl SizeHint {
    /// Size that hugs the content.
    pub fn fixed(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            min_width: width,
            min_height: height,
            ..Default::default()
        }
    }

    /// Size that expands in both directions.
    pub fn fill() -> Self {
        Self {
            width: f32::INFINITY,
            height: f32::INFINITY,
            min_width: 0.0,
            min_height: 0.0,
            ..Default::default()
        }
    }

    pub fn with_width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn with_height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn with_flex(mut self, flex: f32) -> Self {
        self.flex = flex;
        self
    }

    pub fn with_min(mut self, min: Size) -> Self {
        self.min_width = self.min_width.max(min.width);
        self.min_height = self.min_height.max(min.height);
        self
    }

    pub fn with_max(mut self, max: Size) -> Self {
        self.max_width = self.max_width.min(max.width);
        self.max_height = self.max_height.min(max.height);
        self
    }

    /// Clamps a measured size into the hint's bounds.
    pub fn apply(&self, size: Size) -> Size {
        Size::new(
            size.width.clamp(self.min_width, self.max_width),
            size.height.clamp(self.min_height, self.max_height),
        )
    }
}

/// Common state shared by every widget: identity, children and cached layout geometry.
#[derive(Debug)]
pub struct WidgetBase {
    id: ObjectId,
    children: Vec<ObjectId>,
    /// Rectangle assigned by `arrange`, in the coordinate space of the parent.
    bounds: Rect,
    /// Rectangle assigned by `arrange`, translated into window coordinates.
    window_bounds: Rect,
    /// Size returned by the last `measure` call.
    measured: Size,
    state: WidgetState,
    kind: &'static str,
}

impl Default for WidgetBase {
    fn default() -> Self {
        Self::new()
    }
}

impl WidgetBase {
    pub fn new() -> Self {
        Self {
            id: ObjectId::INVALID,
            children: Vec::new(),
            bounds: Rect::ZERO,
            window_bounds: Rect::ZERO,
            measured: Size::ZERO,
            state: WidgetState::new(),
            kind: "Widget",
        }
    }

    /// Creates a base for an already registered id.
    pub fn with_id(id: ObjectId) -> Self {
        Self { id, ..Self::new() }
    }

    #[inline]
    pub fn id(&self) -> ObjectId {
        self.id
    }

    /// Overrides the id (used by [`crate::widget_store::WidgetStore::register`], which allocates
    /// the id when the widget is attached).
    #[inline]
    pub fn set_id(&mut self, id: ObjectId) {
        self.id = id;
    }

    #[inline]
    pub fn children(&self) -> &[ObjectId] {
        &self.children
    }

    #[inline]
    pub fn children_mut(&mut self) -> &mut Vec<ObjectId> {
        &mut self.children
    }

    pub fn add_child(&mut self, id: ObjectId) -> &mut Self {
        if !self.children.contains(&id) {
            self.children.push(id);
        }
        self
    }

    pub fn remove_child(&mut self, id: ObjectId) -> Option<ObjectId> {
        let pos = self.children.iter().position(|c| *c == id)?;
        Some(self.children.remove(pos))
    }

    pub fn clear_children(&mut self) {
        self.children.clear();
    }

    pub fn insert_child(&mut self, index: usize, id: ObjectId) {
        if index >= self.children.len() {
            self.add_child(id);
        } else {
            self.children.insert(index, id);
        }
    }

    /// Rectangle assigned during the last `arrange`, in the widget's **own** coordinate space.
    ///
    /// The origin is always `(0, 0)`; the position inside the parent lives in
    /// [`WidgetBase::window_bounds`] (and in the rect passed to `arrange`). Keeping bounds local
    /// is what lets a widget paint itself with `ctx.fill_rect(self.base().bounds(), ..)` at any
    /// nesting depth.
    #[inline]
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Rectangle assigned during the last `arrange`, in window coordinates.
    #[inline]
    pub fn window_bounds(&self) -> Rect {
        self.window_bounds
    }

    /// Stores the local bounds; only the size is used, the origin is pinned to `(0, 0)`.
    #[inline]
    pub fn set_bounds(&mut self, rect: Rect) {
        self.bounds = Rect::new(0.0, 0.0, rect.width.max(0.0), rect.height.max(0.0));
    }

    /// Stores the local bounds from a size (`(0, 0, width, height)`).
    #[inline]
    pub fn set_bounds_size(&mut self, size: Size) {
        self.bounds = Rect::new(0.0, 0.0, size.width.max(0.0), size.height.max(0.0));
    }

    #[inline]
    pub fn set_window_bounds(&mut self, rect: Rect) {
        self.window_bounds = rect;
    }

    /// Size returned by the last `measure` call.
    #[inline]
    pub fn measured(&self) -> Size {
        self.measured
    }

    #[inline]
    pub fn set_measured(&mut self, size: Size) {
        self.measured = size;
    }

    #[inline]
    pub fn state(&self) -> WidgetState {
        self.state
    }

    #[inline]
    pub fn set_state(&mut self, state: WidgetState) {
        self.state = state;
    }

    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.state.enabled
    }

    #[inline]
    pub fn is_visible(&self) -> bool {
        self.state.visible
    }

    #[inline]
    pub fn set_enabled(&mut self, enabled: bool) {
        self.state.enabled = enabled;
    }

    #[inline]
    pub fn set_visible(&mut self, visible: bool) {
        self.state.visible = visible;
    }

    /// Widget type name, used for diagnostics and default hit-testing.
    #[inline]
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    #[inline]
    pub fn set_kind(&mut self, kind: &'static str) {
        self.kind = kind;
    }

    /// Top-left corner in window coordinates.
    #[inline]
    pub fn origin(&self) -> Point {
        self.window_bounds.origin()
    }

    /// Cursor requested by this widget, if any.
    pub fn cursor(&self) -> crate::keyboard::CursorIcon {
        crate::keyboard::CursorIcon::Default
    }

    /// Tooltip text for this widget.
    pub fn tooltip(&self) -> Option<&str> {
        None
    }
}

/// A retained-mode UI object.
///
/// Widgets are stored in a [`WidgetStore`](crate::widget_store::WidgetStore) and addressed by
/// their [`ObjectId`]; they never own `Rc<RefCell<..>>` pointers to each other (§52).
///
/// The call order of a frame is
/// `update -> measure -> arrange -> paint`, and events are delivered through
/// [`Widget::event`] (§32).
/// Identity and base state of a UI object (design document §5).
///
/// Implemented by `impl_widget_base!`, which expects a `base: WidgetBase` field. Keeping it a
/// separate trait (rather than part of [`Widget`]) is what allows the macro to provide these
/// three methods without clashing with the hand written `impl Widget`.
pub trait Object: 'static {
    /// Stable identity of this object.
    fn object_id(&self) -> ObjectId;

    /// Shared base state: children, arranged bounds and widget state.
    fn base(&self) -> &WidgetBase;

    /// Mutable base state.
    fn base_mut(&mut self) -> &mut WidgetBase;
}

/// A retained-mode UI object.
///
/// Widgets are stored in a [`WidgetStore`](crate::widget_store::WidgetStore) and addressed by
/// their [`ObjectId`]; they never own `Rc<RefCell<..>>` pointers to each other (§52).
///
/// The call order of a frame is
/// `update -> measure -> arrange -> paint`, and events are delivered through
/// [`Widget::event`] (§32).
pub trait Widget: Object + fmt::Debug {
    /// Convenience alias of [`Object::object_id`].
    fn id(&self) -> ObjectId {
        self.object_id()
    }

    /// Type name for diagnostics.
    fn kind(&self) -> &'static str {
        self.base().kind()
    }

    /// Direct children, in paint / focus order.
    fn children(&self) -> &[ObjectId] {
        self.base().children()
    }

    /// `true` when the widget can take keyboard focus.
    fn accepts_focus(&self) -> bool {
        false
    }

    /// `true` when the widget paints an opaque background, which lets hit testing stop early.
    fn is_opaque(&self) -> bool {
        false
    }

    /// Preferred size before `measure` is called.
    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }

    /// Preferred cursor while the pointer is over this widget.
    fn cursor(&self) -> crate::keyboard::CursorIcon {
        crate::keyboard::CursorIcon::Default
    }

    /// Tooltip text shown after a hover delay.
    fn tooltip(&self) -> Option<&str> {
        None
    }

    /// Rectangle anchoring the input method candidate window, in **window** coordinates (T-1).
    ///
    /// Returns `None` — the default — when this widget needs no input method, which turns IME
    /// composition off while such a widget owns keyboard focus. Text editing widgets override it
    /// with their on-screen caret rectangle; the answer is queried once per frame from the
    /// focused widget, so it never goes stale when the layout moves.
    ///
    /// `ui` carries the theme and the text measurer, the two things a caret rectangle needs
    /// besides the widget's own geometry (`base().window_bounds()` is only valid **after**
    /// [`Widget::arrange`]).
    fn ime_cursor_area(&self, ui: &UiContext) -> Option<Rect> {
        let _ = ui;
        None
    }

    /// Hit test in **window** coordinates. Containers forward to their children and honour the
    /// regions occupied by their own decorations (scroll bars, popups, ...).
    fn hit_test(&self, position: Point) -> bool {
        self.base().window_bounds.contains(position)
    }

    /// Called once per frame before layout: timers, animations, background state.
    fn update(&mut self, _ctx: &mut UpdateContext<'_>) {}

    /// Computes and caches the desired size (§15).
    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size;

    /// Assigns a rectangle to this widget and to its children.
    ///
    /// `rect` is the position and size of this widget **inside its parent** (origin at the
    /// parent's content origin). A widget stores only its *local* bounds —
    /// `self.base_mut().set_bounds_size(rect.size())` — and arranges its children in its own
    /// local space, i.e. with rects whose origin is its top-left corner. `ctx.offset()` carries
    /// the widget's window origin, which is what children need to compute `window_bounds`.
    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect);

    /// Handles an event (§10).
    fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult;

    /// Emits paint commands for this widget and its children.
    fn paint(&mut self, ctx: &mut PaintContext<'_>);

    /// Paints the widget's **popup layer** — content that has to sit above everything else,
    /// like an open drop down panel.
    ///
    /// Widgets that draw above the tree register themselves with [`PaintContext::request_popup`]
    /// while their normal [`Widget::paint`] runs; the window then calls this once more after the
    /// whole tree (and every overlay root) was painted, with a context whose offset is this
    /// widget's window origin and whose clip is the window. That is what keeps a panel from
    /// being cut off by an ancestor's clip or overdrawn by a later sibling (T-4). The default
    /// draws nothing.
    fn paint_popup(&mut self, _ctx: &mut PaintContext<'_>) {}

    /// Optional image drawn as the widget background.
    fn background_texture(&self) -> Option<TextureId> {
        None
    }

    /// Called after the widget has been arranged, with the final size.
    fn layout_changed(&mut self, _old: Size, _new: Size) {}
}

/// Implements [`Object`] for a struct that owns a `base: WidgetBase` field.
///
/// ```ignore
/// struct MyWidget { base: WidgetBase }
///
/// impl_widget_base!(MyWidget);
///
/// impl Widget for MyWidget {
///     fn kind(&self) -> &'static str { "MyWidget" }
///     fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size { ... }
///     fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) { ... }
///     fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult { ... }
///     fn paint(&mut self, ctx: &mut PaintContext<'_>) { ... }
/// }
/// ```
#[macro_export]
macro_rules! impl_widget_base {
    ($ty:ty) => {
        impl $crate::widget::Object for $ty {
            fn object_id(&self) -> $crate::object::ObjectId {
                self.base.id()
            }
            fn base(&self) -> &$crate::widget::WidgetBase {
                &self.base
            }
            fn base_mut(&mut self) -> &mut $crate::widget::WidgetBase {
                &mut self.base
            }
        }
    };
}

/// Measures a child and returns the size clamped by its [`SizeHint`].
pub fn hinted_measure(
    store: &crate::widget_store::WidgetStore,
    ctx: &MeasureContext<'_>,
    id: ObjectId,
    available: Size,
) -> Size {
    let hint = store.get(id).map(|w| w.size_hint()).unwrap_or_default();
    let available = Size::new(
        if available.width.is_finite() {
            available.width
        } else {
            hint.width
        },
        if available.height.is_finite() {
            available.height
        } else {
            hint.height
        },
    );
    let size = ctx.measure_child(id, available);
    hint.apply(size)
}

/// Positions `child` at `origin` inside a rect, honouring alignment.
pub fn align_child(
    rect: Rect,
    child: Size,
    h: crate::geometry::Align,
    v: crate::geometry::Align,
) -> Rect {
    let x = match h {
        crate::geometry::Align::Start | crate::geometry::Align::Stretch => rect.left(),
        crate::geometry::Align::Center => rect.left() + (rect.width - child.width) * 0.5,
        crate::geometry::Align::End => rect.right() - child.width,
        crate::geometry::Align::Baseline => rect.left(),
    };
    let width = match h {
        crate::geometry::Align::Stretch => rect.width,
        _ => child.width,
    };
    let y = match v {
        crate::geometry::Align::Start | crate::geometry::Align::Stretch => rect.top(),
        crate::geometry::Align::Center => rect.top() + (rect.height - child.height) * 0.5,
        crate::geometry::Align::End => rect.bottom() - child.height,
        crate::geometry::Align::Baseline => rect.top(),
    };
    let height = match v {
        crate::geometry::Align::Stretch => rect.height,
        _ => child.height,
    };
    Rect::new(x, y, width, height)
}

/// A widget that only exists to give its children a background, border, padding and clip.
///
/// Used as the window root so that applications never need a container just to fill the window.
#[derive(Debug, Default)]
pub struct RootWidget {
    base: WidgetBase,
    background: crate::color::Color,
    padding: crate::geometry::Insets,
}

impl RootWidget {
    pub fn new() -> Self {
        Self {
            base: WidgetBase::new(),
            background: crate::color::Color::TRANSPARENT,
            padding: crate::geometry::Insets::ZERO,
        }
    }

    pub fn with_background(mut self, color: crate::color::Color) -> Self {
        self.background = color;
        self
    }

    pub fn with_padding(mut self, padding: crate::geometry::Insets) -> Self {
        self.padding = padding;
        self
    }

    pub fn set_background(&mut self, color: crate::color::Color) {
        self.background = color;
    }

    pub fn background(&self) -> crate::color::Color {
        self.background
    }

    pub fn set_padding(&mut self, padding: crate::geometry::Insets) {
        self.padding = padding;
    }
}

impl_widget_base!(RootWidget);

impl Widget for RootWidget {
    fn kind(&self) -> &'static str {
        "Root"
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::fill()
    }

    fn is_opaque(&self) -> bool {
        self.background.a > 0.0
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let pad = self.padding;
        // A root with a finite constraint (the normal case: the window size) simply fills it;
        // otherwise it hugs its content, padding included.
        let size = if available.width.is_finite() && available.height.is_finite() {
            available
        } else {
            let inner = Size::new(
                if available.width.is_finite() {
                    (available.width - pad.horizontal()).max(0.0)
                } else {
                    f32::INFINITY
                },
                if available.height.is_finite() {
                    (available.height - pad.vertical()).max(0.0)
                } else {
                    f32::INFINITY
                },
            );
            let content = ctx.measure_children(self.children(), inner);
            let mut size = Size::ZERO;
            for c in content {
                size = size.max(c);
            }
            size.expand(pad)
        };
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds_size(rect.size());
        let local = Rect::new(0.0, 0.0, rect.width.max(0.0), rect.height.max(0.0));
        ctx.arrange_children(self.children(), local.shrink(self.padding));
        // Recorded here as well so that a window arranging the root directly still gets window
        // bounds; when the root is arranged through `arrange_child` this is simply overwritten
        // with the same value.
        self.base
            .set_window_bounds(rect.translate(ctx.offset().x, ctx.offset().y));
    }

    fn event(
        &mut self,
        _ctx: &mut EventContext<'_>,
        _event: &crate::event::UiEvent,
    ) -> EventResult {
        EventResult::Unhandled
    }

    fn paint(&mut self, ctx: &mut PaintContext<'_>) {
        let bounds = self.base.bounds();
        ctx.fill_rect(bounds, self.background);
        for child in self.children() {
            ctx.paint_child(*child);
        }
    }
}

/// Splits a size along an axis, distributing the remainder over `flex` weights.
pub fn distribute(total: f32, weights: &[f32]) -> Vec<f32> {
    let sum: f32 = weights.iter().sum();
    if weights.is_empty() {
        return Vec::new();
    }
    if sum <= f32::EPSILON {
        let each = total / weights.len() as f32;
        return vec![each; weights.len()];
    }
    weights.iter().map(|w| total * w / sum).collect()
}

/// Normalises scroll offsets into `[0, max]`.
pub fn clamp_scroll(value: f32, content: f32, viewport: f32) -> f32 {
    let max = (content - viewport).max(0.0);
    if max <= 0.0 {
        0.0
    } else {
        value.clamp(0.0, max)
    }
}

/// Small helper converting a logical delta into a scroll offset.
pub fn scroll_delta_to_offset(delta: Vec2) -> Vec2 {
    Vec2::new(-delta.x, -delta.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_hint_clamps() {
        let hint = SizeHint::default()
            .with_min(Size::new(10.0, 10.0))
            .with_max(Size::new(50.0, 20.0));
        assert_eq!(hint.apply(Size::new(5.0, 5.0)), Size::new(10.0, 10.0));
        assert_eq!(hint.apply(Size::new(30.0, 15.0)), Size::new(30.0, 15.0));
        assert_eq!(hint.apply(Size::new(80.0, 80.0)), Size::new(50.0, 20.0));
    }

    #[test]
    fn widget_base_children_are_unique() {
        let mut base = WidgetBase::new();
        let a = ObjectId::new(1, 1);
        base.add_child(a);
        base.add_child(a);
        assert_eq!(base.children(), &[a]);
        assert_eq!(base.remove_child(a), Some(a));
        assert!(base.remove_child(a).is_none());
    }

    #[test]
    fn distribute_weights() {
        let sizes = distribute(100.0, &[1.0, 3.0]);
        assert_eq!(sizes, vec![25.0, 75.0]);
        assert_eq!(distribute(100.0, &[]), Vec::<f32>::new());
        let even = distribute(100.0, &[0.0, 0.0, 0.0]);
        assert_eq!(even, vec![100.0 / 3.0; 3]);
    }

    #[test]
    fn clamp_scroll_offsets() {
        assert_eq!(clamp_scroll(-10.0, 500.0, 100.0), 0.0);
        assert_eq!(clamp_scroll(1000.0, 500.0, 100.0), 400.0);
        assert_eq!(clamp_scroll(10.0, 50.0, 100.0), 0.0);
    }

    #[test]
    fn align_child_positions() {
        let rect = Rect::new(0.0, 0.0, 100.0, 50.0);
        let child = Size::new(20.0, 10.0);
        let centered = align_child(
            rect,
            child,
            crate::geometry::Align::Center,
            crate::geometry::Align::End,
        );
        assert_eq!(centered, Rect::new(40.0, 40.0, 20.0, 10.0));
        let stretched = align_child(
            rect,
            child,
            crate::geometry::Align::Stretch,
            crate::geometry::Align::Stretch,
        );
        assert_eq!(stretched, rect);
    }
}
