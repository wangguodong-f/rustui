//! Contexts passed to [`Widget`](crate::widget::Widget) methods, plus the shared
//! [`UiContext`].
//!
//! All four contexts hold a shared reference to the [`WidgetStore`] and to the [`UiContext`], so a
//! widget can reach its own children without any interior-mutable self references (§52).

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use crate::color::Color;
use crate::event::{CursorIcon, EventPhase, QueuedEvent, UiEvent};
use crate::focus::FocusManager;
use crate::geometry::{EdgeRadius, EdgeWidths, Insets, Point, Rect, Size};
use crate::notify::RedrawFlag;
use crate::object::ObjectId;
use crate::paint::Painter;
use crate::style::{Border, WidgetState};
use crate::text::TextMeasurer;
use crate::theme::Theme;
use crate::widget::SizeHint;
use crate::widget_store::WidgetStore;

/// Clipboard access (§39).
///
/// The trait lives in `rustui-core` so widgets never depend on the platform crate; the concrete
/// `winit` implementation lives in `rustui-platform`.
pub trait Clipboard {
    /// Returns the current text, if any.
    fn get_text(&self) -> Option<String>;
    /// Replaces the clipboard contents.
    fn set_text(&self, text: &str);
    /// `true` when the platform clipboard can currently be used.
    fn is_available(&self) -> bool {
        true
    }
}

/// A clipboard that stores the text in memory; used in tests and headless mode.
#[derive(Debug, Default, Clone)]
pub struct MemoryClipboard {
    inner: std::rc::Rc<RefCell<Option<String>>>,
}

impl Clipboard for MemoryClipboard {
    fn get_text(&self) -> Option<String> {
        self.inner.borrow().clone()
    }

    fn set_text(&self, text: &str) {
        *self.inner.borrow_mut() = Some(text.to_string());
    }
}

/// Overlay (popup) request recorded while painting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayRequest {
    pub widget: ObjectId,
    pub z: f32,
    /// `true` to remove the widget from the overlay list.
    pub remove: bool,
}

/// Immutable, window independent services shared by every widget.
pub struct UiContext {
    theme: Theme,
    redraw: RedrawFlag,
    text: Box<dyn TextMeasurer>,
    clipboard: Option<Box<dyn Clipboard>>,
    /// Cursor requested by the hovered widget during the previous frame.
    cursor: Cell<CursorIcon>,
}

impl std::fmt::Debug for UiContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiContext")
            .field("theme", &self.theme)
            .field("redraw", &self.redraw)
            .finish()
    }
}

impl UiContext {
    pub fn new(text: Box<dyn TextMeasurer>) -> Self {
        Self {
            theme: Theme::light(),
            redraw: RedrawFlag::new(),
            text,
            clipboard: None,
            cursor: Cell::new(CursorIcon::Default),
        }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub fn text(&self) -> &dyn TextMeasurer {
        self.text.as_ref()
    }

    pub fn set_text(&mut self, text: Box<dyn TextMeasurer>) {
        self.text = text;
    }

    pub fn clipboard(&self) -> Option<&dyn Clipboard> {
        self.clipboard.as_deref()
    }

    pub fn set_clipboard(&mut self, clipboard: Option<Box<dyn Clipboard>>) {
        self.clipboard = clipboard;
    }

    pub fn redraw(&self) -> &RedrawFlag {
        &self.redraw
    }

    /// Requests a repaint (§15 of §16: UI state changes come back to the UI thread).
    pub fn request_redraw(&self) {
        self.redraw.request();
        crate::notify::request_redraw();
    }

    pub fn set_cursor(&self, cursor: CursorIcon) {
        self.cursor.set(cursor);
    }

    pub fn cursor(&self) -> CursorIcon {
        self.cursor.get()
    }

    /// Shared reference to the redraw flag (used by `SharedProperty`).
    pub fn redraw_shared(&self) -> Arc<()> {
        // Only used for API symmetry; the flag itself is cloned where required.
        Arc::new(())
    }
}

/// Per-window interaction state, used to derive [`WidgetState`]s.
///
/// Besides pointer / focus it also carries the window's **drawable area**, because that is the
/// one piece of geometry every context already passes around: widgets that paint something
/// outside their own rectangle (an open drop down panel, T-4) ask it where there is room.
#[derive(Clone, Debug, Default)]
pub struct Interaction {
    /// Drawable area of the window, in window coordinates.
    ///
    /// [`Rect::ZERO`] until the window reports a size; widgets must treat that as "unknown".
    pub viewport: Rect,
    pub pointer: Point,
    /// Deepest widget under the pointer (the hit test target).
    pub hovered: Option<ObjectId>,
    /// Full chain under the pointer, from the root down to [`Interaction::hovered`].
    ///
    /// A button that contains a label is hovered whenever the label is, so every widget of the
    /// chain derives `hovered` from this path instead of comparing against the leaf only.
    pub hover_path: Vec<ObjectId>,
    /// Widget that currently holds the pointer capture (press & drag).
    pub captured: Option<ObjectId>,
    pub pressed: Option<ObjectId>,
    pub focused: Option<ObjectId>,
    pub window_focused: bool,
    /// Object currently dragged over (drag & drop).
    pub drag_target: Option<ObjectId>,
}

impl Interaction {
    /// Builds the state of `id`.
    pub fn state_for(&self, id: ObjectId, mut state: WidgetState) -> WidgetState {
        state.visible = state.visible && self.window_focused_or_always(id);
        state.hovered =
            self.hovered == Some(id) || self.hover_path.contains(&id) || self.captured == Some(id);
        state.pressed = self.pressed == Some(id);
        state.focused = self.focused == Some(id);
        state
    }

    fn window_focused_or_always(&self, _id: ObjectId) -> bool {
        true
    }

    /// `true` when `id` currently owns the pointer.
    pub fn owns_pointer(&self, id: ObjectId) -> bool {
        self.captured == Some(id)
    }
}

/// Frame timing information (§33).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameClock {
    pub frame: u64,
    pub delta: Duration,
    pub elapsed: Duration,
}

impl FrameClock {
    /// Delta in seconds, at least 1 ms to keep animations stable after a pause.
    pub fn delta_seconds(&self) -> f32 {
        (self.delta.as_secs_f32() * 1000.0).max(1.0) / 1000.0
    }

    /// Elapsed time in seconds.
    pub fn elapsed_seconds(&self) -> f32 {
        self.elapsed.as_secs_f32()
    }
}

// ---------------------------------------------------------------------------
// measure
// ---------------------------------------------------------------------------

/// Passed to [`Widget::measure`].
pub struct MeasureContext<'a> {
    pub(crate) ui: &'a UiContext,
    pub(crate) store: &'a WidgetStore,
    pub(crate) interaction: &'a Interaction,
    pub(crate) depth: usize,
}

impl<'a> MeasureContext<'a> {
    /// Creates a measure context.
    pub fn new(
        ui: &'a UiContext,
        store: &'a WidgetStore,
        interaction: &'a Interaction,
        depth: usize,
    ) -> Self {
        Self {
            ui,
            store,
            interaction,
            depth,
        }
    }

    /// Shared services.
    pub fn ui(&self) -> &'a UiContext {
        self.ui
    }

    /// The widget store.
    pub fn store(&self) -> &'a WidgetStore {
        self.store
    }

    /// Measures a child widget, returning its size.
    pub fn measure_child(&self, id: ObjectId, available: Size) -> Size {
        let mut widget = match self.store.take(id) {
            Some(w) => w,
            None => return Size::ZERO,
        };
        let ctx = MeasureContext::new(self.ui, self.store, self.interaction, self.depth + 1);
        let hint = widget.size_hint();
        let size = widget.measure(&ctx, available);
        self.store.put(id, widget);
        hint.apply(size)
    }

    /// Measures several children.
    pub fn measure_children(&self, ids: &[ObjectId], available: Size) -> Vec<Size> {
        ids.iter()
            .map(|id| self.measure_child(*id, available))
            .collect()
    }

    /// Measures text with the shared text engine.
    pub fn measure_text(
        &self,
        text: &str,
        style: &crate::paint::TextStyle,
        max_width: Option<f32>,
    ) -> Size {
        self.ui.text().measure_text(text, style, max_width)
    }

    /// Interaction state of the measuring widget.
    pub fn interaction(&self) -> &Interaction {
        self.interaction
    }

    /// Drawable area of the window, in window coordinates (see [`Interaction::viewport`]).
    pub fn viewport(&self) -> Rect {
        self.interaction.viewport
    }

    /// Current nesting depth.
    pub fn depth(&self) -> usize {
        self.depth
    }
}

// ---------------------------------------------------------------------------
// arrange
// ---------------------------------------------------------------------------

/// Passed to [`Widget::arrange`].
pub struct ArrangeContext<'a> {
    pub(crate) ui: &'a UiContext,
    pub(crate) store: &'a WidgetStore,
    pub(crate) interaction: &'a Interaction,
    pub(crate) depth: usize,
    /// Offset (in window coordinates) that children must add to their own bounds.
    pub(crate) offset: Point,
}

impl<'a> ArrangeContext<'a> {
    pub fn new(
        ui: &'a UiContext,
        store: &'a WidgetStore,
        interaction: &'a Interaction,
        depth: usize,
    ) -> Self {
        Self {
            ui,
            store,
            interaction,
            depth,
            offset: Point::ZERO,
        }
    }

    /// Creates a child context whose descendants resolve coordinates against `offset`.
    pub fn with_offset(&self, offset: Point) -> ArrangeContext<'_> {
        ArrangeContext {
            ui: self.ui,
            store: self.store,
            interaction: self.interaction,
            depth: self.depth + 1,
            offset,
        }
    }

    pub fn ui(&self) -> &'a UiContext {
        self.ui
    }

    pub fn store(&self) -> &'a WidgetStore {
        self.store
    }

    /// Offset (in window coordinates) of **this** widget's origin: children add it to their own
    /// coordinates.
    pub fn offset(&self) -> Point {
        self.offset
    }

    /// Arranges one child inside `rect` (parent relative coordinates).
    pub fn arrange_child(&self, id: ObjectId, rect: Rect) {
        let widget = match self.store.take(id) {
            Some(w) => w,
            None => return,
        };
        let old = widget.base().measured();
        let child_offset = self.offset + rect.origin();
        let mut widget = widget;
        let ctx = ArrangeContext {
            ui: self.ui,
            store: self.store,
            interaction: self.interaction,
            depth: self.depth + 1,
            offset: child_offset,
        };
        widget.arrange(&ctx, rect);
        widget
            .base_mut()
            .set_window_bounds(rect.translate(self.offset.x, self.offset.y));
        let new = widget.base().measured();
        self.store.put(id, widget);
        if old != new {
            if let Some(mut w) = self.store.take(id) {
                w.layout_changed(old, new);
                self.store.put(id, w);
            }
        }
    }
    /// Arranges several children.
    pub fn arrange_children(&self, ids: &[ObjectId], rect: Rect) {
        for id in ids {
            self.arrange_child(*id, rect);
        }
    }

    /// The interaction state of the window.
    pub fn interaction(&self) -> &Interaction {
        self.interaction
    }

    /// Drawable area of the window, in window coordinates (see [`Interaction::viewport`]).
    pub fn viewport(&self) -> Rect {
        self.interaction.viewport
    }
}

// ---------------------------------------------------------------------------
// update
// ---------------------------------------------------------------------------

/// Passed to [`Widget::update`].
pub struct UpdateContext<'a> {
    pub(crate) ui: &'a UiContext,
    pub(crate) store: &'a WidgetStore,
    pub(crate) interaction: &'a Interaction,
    pub(crate) clock: FrameClock,
    pub(crate) depth: usize,
}

impl<'a> UpdateContext<'a> {
    pub fn new(
        ui: &'a UiContext,
        store: &'a WidgetStore,
        interaction: &'a Interaction,
        clock: FrameClock,
        depth: usize,
    ) -> Self {
        Self {
            ui,
            store,
            interaction,
            clock,
            depth,
        }
    }

    pub fn ui(&self) -> &'a UiContext {
        self.ui
    }

    pub fn store(&self) -> &'a WidgetStore {
        self.store
    }

    /// Frame timing (drives animations and cursor blinking).
    pub fn clock(&self) -> FrameClock {
        self.clock
    }

    /// The interaction state of the window.
    pub fn interaction(&self) -> &Interaction {
        self.interaction
    }

    /// Drawable area of the window, in window coordinates (see [`Interaction::viewport`]).
    pub fn viewport(&self) -> Rect {
        self.interaction.viewport
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Runs [`Widget::update`] on a child.
    pub fn update_child(&self, id: ObjectId) {
        let mut widget = match self.store.take(id) {
            Some(w) => w,
            None => return,
        };
        let mut ctx = UpdateContext::new(
            self.ui,
            self.store,
            self.interaction,
            self.clock,
            self.depth + 1,
        );
        widget.update(&mut ctx);
        self.store.put(id, widget);
    }

    /// Runs [`Widget::update`] on a list of children.
    pub fn update_children(&self, ids: &[ObjectId]) {
        for id in ids {
            self.update_child(*id);
        }
    }

    /// Requests a repaint.
    pub fn request_redraw(&self) {
        self.ui.request_redraw();
    }
}

// ---------------------------------------------------------------------------
// event
// ---------------------------------------------------------------------------

/// Passed to [`Widget::event`].
///
/// Delivering an event to a child is **queued**, never re-entrant: `send_to` appends to a queue
/// that the dispatcher drains after the current widget returns. That is what makes it possible for
/// a widget to mutate itself (`&mut self`) while still reaching arbitrary descendants.
pub struct EventContext<'a> {
    pub(crate) ui: &'a UiContext,
    pub(crate) store: &'a WidgetStore,
    pub(crate) interaction: &'a Interaction,
    pub(crate) focus: &'a FocusManager,
    pub(crate) queue: &'a EventQueue,
    pub(crate) overlays: &'a OverlayList,
    pub(crate) depth: usize,
    /// `true` once an event has been consumed, so bubbling stops.
    pub(crate) consumed: &'a Cell<bool>,
}

impl<'a> EventContext<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ui: &'a UiContext,
        store: &'a WidgetStore,
        interaction: &'a Interaction,
        focus: &'a FocusManager,
        queue: &'a EventQueue,
        overlays: &'a OverlayList,
        depth: usize,
        consumed: &'a Cell<bool>,
    ) -> Self {
        Self {
            ui,
            store,
            interaction,
            focus,
            queue,
            overlays,
            depth,
            consumed,
        }
    }

    pub fn ui(&self) -> &'a UiContext {
        self.ui
    }

    pub fn store(&self) -> &'a WidgetStore {
        self.store
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    /// The interaction state of the window.
    pub fn interaction(&self) -> &Interaction {
        self.interaction
    }

    /// Drawable area of the window, in window coordinates (see [`Interaction::viewport`]).
    pub fn viewport(&self) -> Rect {
        self.interaction.viewport
    }

    /// Current keyboard focus.
    pub fn focused(&self) -> Option<ObjectId> {
        self.focus.focused()
    }

    /// Moves keyboard focus; a [`UiEvent::FocusIn`]/[`UiEvent::FocusOut`] pair is generated.
    pub fn set_focus(&self, id: ObjectId, reason: crate::keyboard::FocusReason) {
        self.focus.set_focus(id, reason);
    }

    pub fn clear_focus(&self) {
        self.focus.clear_focus();
    }

    /// Clipboard access (§39).
    pub fn clipboard(&self) -> Option<&dyn Clipboard> {
        self.ui.clipboard()
    }

    /// Requests a repaint.
    pub fn request_redraw(&self) {
        self.ui.request_redraw();
    }

    /// Marks the event as handled, which stops bubbling.
    pub fn accept(&self) {
        self.consumed.set(true);
    }

    /// `true` when an ancestor already handled this event.
    pub fn is_accepted(&self) -> bool {
        self.consumed.get()
    }

    /// Sends `event` to `id` (target phase).
    pub fn send_to(&self, id: ObjectId, event: &UiEvent) {
        self.queue.push(id, event, EventPhase::Target);
    }

    /// Sends `event` to every child of the widget currently being processed.
    pub fn broadcast(&self, event: &UiEvent) {
        let children = match self.store.get(self.current_id()) {
            Some(widget) => widget.children().to_vec(),
            None => Vec::new(),
        };
        for child in children {
            self.send_to(child, event);
        }
    }

    /// Sends an event to the whole subtree of `id`.
    pub fn send_to_subtree(&self, id: ObjectId, event: &UiEvent) {
        for node in self.store.subtree(id) {
            if node != id {
                self.send_to(node, event);
            }
        }
    }

    /// Grabs the pointer so that every following pointer event is delivered to `id`.
    pub fn capture_pointer(&self, id: ObjectId) {
        self.queue.capture_pointer(id);
    }

    pub fn release_pointer(&self) {
        self.queue.release_pointer();
    }

    /// Asks for `cursor` to be set on the window while the pointer is over this widget.
    pub fn set_cursor(&self, cursor: CursorIcon) {
        self.queue.set_cursor(cursor);
    }

    /// Registers `id` as an overlay (popup) for the next frame.
    pub fn request_overlay(&self, id: ObjectId, z: f32) {
        self.overlays.request(OverlayRequest {
            widget: id,
            z,
            remove: false,
        });
    }

    /// Removes `id` from the overlay list.
    pub fn remove_overlay(&self, id: ObjectId) {
        self.overlays.request(OverlayRequest {
            widget: id,
            z: 0.0,
            remove: true,
        });
    }

    /// `true` when `id` is currently shown as an overlay.
    pub fn is_overlay(&self, id: ObjectId) -> bool {
        self.overlays.contains(id)
    }

    /// Closes the innermost open overlay, if any; returns `true` when one was closed.
    pub fn close_top_overlay(&self) -> bool {
        self.overlays.close_top()
    }

    /// The widget currently being processed; set by the dispatcher.
    fn current_id(&self) -> ObjectId {
        self.queue
            .current()
            .map(|(_, id, _)| id)
            .unwrap_or(ObjectId::INVALID)
    }

    /// Measures text with the shared text engine.
    pub fn measure_text(
        &self,
        text: &str,
        style: &crate::paint::TextStyle,
        max_width: Option<f32>,
    ) -> Size {
        self.ui.text().measure_text(text, style, max_width)
    }
}

/// FIFO queue of events waiting to be delivered, plus per-dispatch side effects.
#[derive(Debug, Default)]
pub struct EventQueue {
    queue: RefCell<VecDeque<QueuedEvent>>,
    capture: Cell<Option<ObjectId>>,
    cursor: Cell<CursorIcon>,
    current: RefCell<Option<(EventPhase, ObjectId, u64)>>,
}

impl EventQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, target: ObjectId, event: &UiEvent, phase: EventPhase) {
        self.queue.borrow_mut().push_back(QueuedEvent {
            target,
            event: event.clone(),
            phase,
            cursor: CursorIcon::Default,
        });
    }

    /// Pops the next event, marking it as the one currently being delivered.
    pub fn pop(&self) -> Option<QueuedEvent> {
        let next = self.queue.borrow_mut().pop_front()?;
        *self.current.borrow_mut() = Some((next.phase, next.target, 0));
        Some(next)
    }

    /// The widget currently being delivered to.
    pub fn current(&self) -> Option<(EventPhase, ObjectId, u64)> {
        *self.current.borrow()
    }
    /// Clears the "current" marker (called when delivery finishes).
    pub fn finish(&self) {
        *self.current.borrow_mut() = None;
    }

    pub fn push_front(&self, target: ObjectId, event: &UiEvent, phase: EventPhase) {
        self.queue.borrow_mut().push_front(QueuedEvent {
            target,
            event: event.clone(),
            phase,
            cursor: CursorIcon::Default,
        });
    }

    pub fn len(&self) -> usize {
        self.queue.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.borrow().is_empty()
    }

    pub fn clear(&self) {
        self.queue.borrow_mut().clear();
        *self.current.borrow_mut() = None;
    }

    pub fn capture_pointer(&self, id: ObjectId) {
        self.capture.set(Some(id));
    }

    pub fn release_pointer(&self) {
        self.capture.set(None);
    }

    pub fn captured(&self) -> Option<ObjectId> {
        self.capture.get()
    }

    pub fn set_cursor(&self, cursor: CursorIcon) {
        if cursor != CursorIcon::Default {
            self.cursor.set(cursor);
        }
    }

    /// Cursor requested during the current event; `CursorIcon::Default` clears it.
    pub fn take_cursor(&self) -> CursorIcon {
        let cursor = self.cursor.get();
        self.cursor.set(CursorIcon::Default);
        cursor
    }

    pub fn clear_cursor(&self) {
        self.cursor.set(CursorIcon::Default);
    }
}

/// The list of widgets rendered above the window content (popups, menus, tooltips).
#[derive(Debug, Default)]
pub struct OverlayList {
    /// `(widget, z)` sorted by `z` then insertion order.
    items: RefCell<Vec<(ObjectId, f32)>>,
    pending: RefCell<Vec<OverlayRequest>>,
    /// Widgets that asked for a popup layer during this frame's paint (see
    /// [`PaintContext::request_popup`]). Drained by the window once the tree is painted, so a
    /// widget only has to ask for it while its popup is open.
    popups: RefCell<Vec<ObjectId>>,
}

impl OverlayList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(&self, request: OverlayRequest) {
        self.pending.borrow_mut().push(request);
    }

    /// Queues `id` for the popup layer of the current frame (deduplicated).
    pub fn request_popup(&self, id: ObjectId) {
        let mut popups = self.popups.borrow_mut();
        if !popups.contains(&id) {
            popups.push(id);
        }
    }

    /// The ids queued for the popup layer, clearing the queue.
    pub fn take_popups(&self) -> Vec<ObjectId> {
        self.popups.borrow_mut().drain(..).collect()
    }

    /// Applies every request recorded during the frame.
    pub fn apply_pending(&self) {
        let pending: Vec<OverlayRequest> = self.pending.borrow_mut().drain(..).collect();
        let mut items = self.items.borrow_mut();
        for request in pending {
            if request.remove {
                items.retain(|(id, _)| *id != request.widget);
            } else if !items.iter().any(|(id, _)| *id == request.widget) {
                items.push((request.widget, request.z));
                items.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            }
        }
    }

    /// Adds an overlay without going through the request queue (used by the window).
    pub fn insert(&self, id: ObjectId, z: f32) {
        self.request(OverlayRequest {
            widget: id,
            z,
            remove: false,
        });
        self.apply_pending();
    }

    /// Removes an overlay immediately.
    pub fn remove(&self, id: ObjectId) {
        self.items.borrow_mut().retain(|(item, _)| *item != id);
    }

    pub fn contains(&self, id: ObjectId) -> bool {
        self.items.borrow().iter().any(|(item, _)| *item == id)
    }

    /// Overlay ids in paint order (lowest `z` first).
    pub fn ordered(&self) -> Vec<ObjectId> {
        self.items.borrow().iter().map(|(id, _)| *id).collect()
    }

    pub fn len(&self) -> usize {
        self.items.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.borrow().is_empty()
    }

    pub fn clear(&self) {
        self.items.borrow_mut().clear();
    }

    /// Removes the overlay with the highest `z` (top-most); returns `true` when one was closed.
    pub fn close_top(&self) -> bool {
        let closed = self.items.borrow_mut().pop().is_some();
        closed
    }
}

// ---------------------------------------------------------------------------
// paint
// ---------------------------------------------------------------------------

/// Passed to [`Widget::paint`].
///
/// The context offset is the window origin of the widget being painted, so a widget always paints
/// in its own local coordinates: `ctx.fill_rect(self.base().bounds(), color)` is correct at any
/// nesting depth.
pub struct PaintContext<'a> {
    pub(crate) ui: &'a UiContext,
    pub(crate) store: &'a WidgetStore,
    pub(crate) interaction: &'a Interaction,
    pub(crate) painter: &'a mut Painter,
    pub(crate) overlays: &'a OverlayList,
    pub(crate) offset: Point,
    pub(crate) clip: Rect,
    pub(crate) clip_stack: RefCell<Vec<Rect>>,
    pub(crate) depth: usize,
}

impl<'a> PaintContext<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ui: &'a UiContext,
        store: &'a WidgetStore,
        interaction: &'a Interaction,
        painter: &'a mut Painter,
        overlays: &'a OverlayList,
        offset: Point,
        clip: Rect,
        depth: usize,
    ) -> Self {
        Self {
            ui,
            store,
            interaction,
            painter,
            overlays,
            offset,
            clip,
            clip_stack: RefCell::new(Vec::new()),
            depth,
        }
    }

    pub fn ui(&self) -> &'a UiContext {
        self.ui
    }

    pub fn store(&self) -> &'a WidgetStore {
        self.store
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Mutable access to the painter.
    pub fn painter(&mut self) -> &mut Painter {
        self.painter
    }

    /// Window origin of the widget that owns this context: the value to add to **local**
    /// coordinates to obtain window coordinates.
    pub fn offset(&self) -> Point {
        self.offset
    }

    /// Current clip rectangle, in window coordinates.
    pub fn clip(&self) -> Rect {
        self.clip
    }

    /// Converts a local point into window coordinates.
    pub fn to_window(&self, point: Point) -> Point {
        point + self.offset.to_vec()
    }

    /// Converts a local rect into window coordinates.
    pub fn to_window_rect(&self, rect: Rect) -> Rect {
        rect.offset(self.offset.to_vec())
    }

    /// Widget state of `id`.
    pub fn state_of(&self, id: ObjectId, base: WidgetState) -> WidgetState {
        self.interaction.state_for(id, base)
    }

    /// Current interaction state.
    pub fn interaction(&self) -> &Interaction {
        self.interaction
    }

    /// Drawable area of the window, in window coordinates (see [`Interaction::viewport`]).
    pub fn viewport(&self) -> Rect {
        self.interaction.viewport
    }

    /// Measures text with the shared text engine.
    pub fn measure_text(
        &self,
        text: &str,
        style: &crate::paint::TextStyle,
        max_width: Option<f32>,
    ) -> Size {
        self.ui.text().measure_text(text, style, max_width)
    }

    /// Height of one text line.
    pub fn line_height(&self, style: &crate::paint::TextStyle) -> f32 {
        self.ui.text().line_height(style)
    }

    /// Requests a repaint.
    pub fn request_redraw(&self) {
        self.ui.request_redraw();
    }

    /// Registers `id` as an overlay rendered above the window content.
    pub fn request_overlay(&self, id: ObjectId, z: f32) {
        self.overlays.request(OverlayRequest {
            widget: id,
            z,
            remove: false,
        });
    }

    /// Registers `id` for the popup layer of this frame: its [`crate::Widget::paint_popup`] runs
    /// after the whole tree was painted, above every sibling and outside every ancestor's clip.
    pub fn request_popup(&self, id: ObjectId) {
        self.overlays.request_popup(id);
    }

    pub fn remove_overlay(&self, id: ObjectId) {
        self.overlays.request(OverlayRequest {
            widget: id,
            z: 0.0,
            remove: true,
        });
    }

    /// Adds `padding` around `rect` and fills it — the common "surface" pattern.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let r = self.to_window_rect(rect);
        self.painter.rect(r, color);
    }

    /// Fills a rounded rectangle in local coordinates.
    pub fn fill_rounded(&mut self, rect: Rect, radius: f32, color: Color) {
        let r = self.to_window_rect(rect);
        self.painter.rounded_rect(r, radius, color);
    }

    /// Strokes the outline of `rect` (local coordinates).
    ///
    /// The stroke sits **inside** `rect`, the way `Fill::border` does, so a border drawn after the
    /// children (containers do exactly that) can never cover them.
    pub fn stroke_rect(&mut self, rect: Rect, radius: f32, color: Color, width: f32) {
        if width <= 0.0 || color.a <= 0.0 {
            return;
        }
        let r = self.to_window_rect(rect);
        self.painter.border(
            r,
            EdgeRadius::all(radius),
            &Border {
                widths: EdgeWidths::all(width),
                color,
            },
        );
    }

    /// Pushes a clip rectangle expressed in local coordinates.
    pub fn push_clip(&mut self, rect: Rect) {
        let window = self.to_window_rect(rect);
        self.clip_stack.borrow_mut().push(self.clip);
        self.clip = self.clip.intersection(window);
        self.painter.push_clip(self.clip);
    }

    /// Pushes a rounded clip for `rect` (local coordinates).
    pub fn push_clip_rounded(&mut self, rect: Rect, radius: f32) {
        let window = self.to_window_rect(rect);
        self.clip_stack.borrow_mut().push(self.clip);
        self.clip = self.clip.intersection(window);
        self.painter.push_clip_rounded(self.clip, radius);
    }

    /// Pop the last clip and restore the previous one.
    pub fn pop_clip(&mut self) {
        self.painter.pop_clip();
        if let Some(previous) = self.clip_stack.borrow_mut().pop() {
            self.clip = previous;
        }
    }

    /// `true` while this context has at least one clip of its own.
    pub fn has_local_clip(&self) -> bool {
        !self.clip_stack.borrow().is_empty()
    }

    /// Paints a child widget, translating its coordinates.
    ///
    /// Every widget paints in **local** coordinates: its own `bounds` start at `(0, 0)` and the
    /// context offset is its window origin (the value `arrange` recorded in `window_bounds`).
    /// Passing the child's window origin — rather than the parent's — is what keeps nested trees
    /// correct: a grandchild's local coordinates are relative to its own parent, not to the root.
    pub fn paint_child(&mut self, id: ObjectId) {
        let widget = match self.store.take(id) {
            Some(w) => w,
            None => return,
        };
        let window_rect = widget.base().window_bounds();
        let clip = self.clip.intersection(window_rect);
        self.painter.push_clip(clip);

        let store = self.store;
        let ui = self.ui;
        let interaction = self.interaction;
        let overlays = self.overlays;
        let painter = &mut *self.painter;
        let depth = self.depth + 1;
        let offset = window_rect.origin();

        let mut widget = widget;
        {
            let mut ctx = PaintContext {
                ui,
                store,
                interaction,
                painter,
                overlays,
                offset,
                clip,
                clip_stack: RefCell::new(Vec::new()),
                depth,
            };
            widget.paint(&mut ctx);
        }
        self.store.put(id, widget);
        self.painter.pop_clip();
    }

    /// Paints a child with an extra translation applied to its window position (used by
    /// scrolled / offset content).
    pub fn paint_child_with_offset(&mut self, id: ObjectId, extra_offset: Point) {
        let widget = match self.store.take(id) {
            Some(w) => w,
            None => return,
        };
        let window = widget
            .base()
            .window_bounds()
            .translate(extra_offset.x, extra_offset.y);
        let mut widget = widget;
        widget.base_mut().set_window_bounds(window);
        self.store.put(id, widget);
        self.paint_child(id);
    }

    /// Paints an overlay widget: it is not clipped by the window content.
    ///
    /// Overlays are arranged in window coordinates, so the widget's window origin is used as-is.
    pub fn paint_overlay(&mut self, id: ObjectId) {
        let widget = match self.store.take(id) {
            Some(w) => w,
            None => return,
        };
        let origin = widget.base().window_bounds().origin();
        let ui = self.ui;
        let store = self.store;
        let interaction = self.interaction;
        let overlays = self.overlays;
        let painter = &mut *self.painter;
        let clip = Rect::new(0.0, 0.0, f32::MAX, f32::MAX);
        let mut widget = widget;
        {
            let mut ctx = PaintContext {
                ui,
                store,
                interaction,
                painter,
                overlays,
                offset: origin,
                clip,
                clip_stack: RefCell::new(Vec::new()),
                depth: self.depth + 1,
            };
            widget.paint(&mut ctx);
        }
        self.store.put(id, widget);
    }

    /// Paints the popup layer of `id`: only the window clips it, so an open panel is neither cut
    /// by an ancestor's clip nor overdrawn by a later sibling.
    pub fn paint_popup(&mut self, id: ObjectId) {
        let widget = match self.store.take(id) {
            Some(w) => w,
            None => return,
        };
        let origin = widget.base().window_bounds().origin();
        let ui = self.ui;
        let store = self.store;
        let interaction = self.interaction;
        let overlays = self.overlays;
        let clip = self.clip;
        // Make the painter's clip match the context's, and leave it as it was.
        self.painter.push_clip(clip);
        let painter = &mut *self.painter;
        let mut widget = widget;
        {
            let mut ctx = PaintContext {
                ui,
                store,
                interaction,
                painter,
                overlays,
                offset: origin,
                clip,
                clip_stack: RefCell::new(Vec::new()),
                depth: self.depth + 1,
            };
            widget.paint_popup(&mut ctx);
        }
        self.painter.pop_clip();
        self.store.put(id, widget);
    }

    /// Size hint of a child, for layout code running inside `paint`.
    pub fn size_hint_of(&self, id: ObjectId) -> SizeHint {
        self.store
            .get(id)
            .map(|w| w.size_hint())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// helpers shared by layout code
// ---------------------------------------------------------------------------

/// Padding, margin and radius helper used by several widgets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoxMetrics {
    pub padding: Insets,
    pub margin: Insets,
    pub radius: f32,
}

impl BoxMetrics {
    pub fn new(padding: Insets) -> Self {
        Self {
            padding,
            ..Default::default()
        }
    }

    /// Content area of `rect`.
    pub fn content(&self, rect: Rect) -> Rect {
        rect.shrink(self.margin).shrink(self.padding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::ObjectRegistry;
    use crate::{impl_widget_base, EventResult, Widget};
    use std::rc::Rc;

    #[derive(Debug, Default)]
    struct Probe {
        base: crate::widget::WidgetBase,
        size: Size,
        painted: Rc<Cell<u32>>,
        events: Rc<Cell<u32>>,
        last_offset: Rc<Cell<Point>>,
    }

    impl_widget_base!(Probe);

    impl Widget for Probe {
        fn kind(&self) -> &'static str {
            "Probe"
        }

        fn measure(&mut self, _ctx: &MeasureContext<'_>, _available: Size) -> Size {
            self.base.set_measured(self.size);
            self.size
        }

        fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
            self.last_offset.set(ctx.offset());
            self.base.set_bounds(rect);
        }

        fn event(&mut self, _ctx: &mut EventContext<'_>, _event: &UiEvent) -> EventResult {
            self.events.set(self.events.get() + 1);
            EventResult::Handled
        }

        fn paint(&mut self, ctx: &mut PaintContext<'_>) {
            self.painted.set(self.painted.get() + 1);
            ctx.fill_rect(self.base.bounds(), Color::BLUE);
            for child in self.children() {
                ctx.paint_child(*child);
            }
        }
    }

    fn probe(size: Size) -> Probe {
        Probe {
            size,
            painted: Rc::new(Cell::new(0)),
            events: Rc::new(Cell::new(0)),
            last_offset: Rc::new(Cell::new(Point::ZERO)),
            ..Default::default()
        }
    }

    struct StubText;
    impl TextMeasurer for StubText {
        fn layout_text(
            &self,
            text: &str,
            _style: &crate::paint::TextStyle,
            _max: Option<f32>,
        ) -> crate::text::TextLayout {
            crate::text::TextLayout::single_line(
                Size::new(text.len() as f32 * 7.0, 14.0),
                10.0,
                4.0,
            )
        }

        fn line_height(&self, _style: &crate::paint::TextStyle) -> f32 {
            14.0
        }
    }

    fn ui() -> UiContext {
        UiContext::new(Box::new(StubText))
    }

    #[test]
    fn measure_context_measures_child() {
        let ui = ui();
        let store = WidgetStore::new();
        let mut registry = ObjectRegistry::new();
        let child = store.insert(&mut registry, Box::new(probe(Size::new(30.0, 20.0))));
        let interaction = Interaction::default();
        let ctx = MeasureContext::new(&ui, &store, &interaction, 0);
        assert_eq!(
            ctx.measure_child(child, Size::new(100.0, 100.0)),
            Size::new(30.0, 20.0)
        );
        assert_eq!(
            ctx.measure_text("abc", &Default::default(), None),
            Size::new(21.0, 14.0)
        );
    }

    #[test]
    fn arrange_context_offsets_children() {
        let ui = ui();
        let store = WidgetStore::new();
        let mut registry = ObjectRegistry::new();
        let child_widget = probe(Size::new(10.0, 10.0));
        let seen = child_widget.last_offset.clone();
        let child = store.insert(&mut registry, Box::new(child_widget));
        let interaction = Interaction::default();
        let ctx = ArrangeContext::new(&ui, &store, &interaction, 0);
        ctx.arrange_child(child, Rect::new(5.0, 7.0, 10.0, 10.0));
        assert_eq!(seen.get(), Point::new(5.0, 7.0));
        assert_eq!(
            store.get(child).unwrap().base().window_bounds(),
            Rect::new(5.0, 7.0, 10.0, 10.0)
        );
    }

    #[test]
    fn paint_child_translates_coordinates() {
        let ui = ui();
        let store = WidgetStore::new();
        let mut registry = ObjectRegistry::new();
        let child_widget = probe(Size::new(10.0, 10.0));
        let painted = child_widget.painted.clone();
        let child = store.insert(&mut registry, Box::new(child_widget));
        let interaction = Interaction::default();
        let overlays = OverlayList::new();
        let mut painter = Painter::new();
        painter.reset(Rect::new(0.0, 0.0, 200.0, 200.0));
        {
            let mut ctx = PaintContext::new(
                &ui,
                &store,
                &interaction,
                &mut painter,
                &overlays,
                Point::new(20.0, 30.0),
                Rect::new(0.0, 0.0, 200.0, 200.0),
                0,
            );
            // The child sits at (5, 7) inside a parent whose window origin is (20, 30): local
            // bounds start at (0, 0) and `window_bounds` carries the position.
            if let Some(mut w) = store.take(child) {
                w.base_mut().set_bounds(Rect::new(5.0, 7.0, 10.0, 10.0));
                w.base_mut()
                    .set_window_bounds(Rect::new(25.0, 37.0, 10.0, 10.0));
                store.put(child, w);
            }
            ctx.paint_child(child);
        }
        assert_eq!(painted.get(), 1);
        // The child paints in local coordinates, translated by its own window origin (25, 37).
        let shape = painter
            .commands()
            .iter()
            .find_map(|c| match c {
                crate::paint::PaintCommand::Shape { rect, .. } => Some(*rect),
                _ => None,
            })
            .expect("shape command");
        assert_eq!(shape, Rect::new(25.0, 37.0, 10.0, 10.0));
    }

    /// Regression test: a grandchild must be positioned relative to *its parent*, not to the
    /// root. The paint context handed down carries the child's own window origin.
    #[test]
    fn nested_paint_accumulates_window_origins() {
        let ui = ui();
        let store = WidgetStore::new();
        let mut registry = ObjectRegistry::new();

        let child_widget = probe(Size::new(10.0, 10.0));
        let child_painted = child_widget.painted.clone();
        let child = store.insert(&mut registry, Box::new(child_widget));

        let parent_widget = probe(Size::new(100.0, 100.0));
        let parent_painted = parent_widget.painted.clone();
        let parent = store.insert(&mut registry, Box::new(parent_widget));
        if let Some(mut w) = store.take(parent) {
            w.base_mut().add_child(child);
            store.put(parent, w);
        }

        // Parent at window (50, 60), child at window (60, 80) — i.e. (10, 20) inside the parent.
        if let Some(mut w) = store.take(parent) {
            w.base_mut().set_bounds(Rect::new(0.0, 0.0, 100.0, 100.0));
            w.base_mut()
                .set_window_bounds(Rect::new(50.0, 60.0, 100.0, 100.0));
            store.put(parent, w);
        }
        if let Some(mut w) = store.take(child) {
            w.base_mut().set_bounds(Rect::new(0.0, 0.0, 10.0, 10.0));
            w.base_mut()
                .set_window_bounds(Rect::new(60.0, 80.0, 10.0, 10.0));
            store.put(child, w);
        }

        let interaction = Interaction::default();
        let overlays = OverlayList::new();
        let mut painter = Painter::new();
        painter.reset(Rect::new(0.0, 0.0, 200.0, 200.0));
        {
            let mut ctx = PaintContext::new(
                &ui,
                &store,
                &interaction,
                &mut painter,
                &overlays,
                Point::new(50.0, 60.0), // the parent's window origin
                Rect::new(0.0, 0.0, 200.0, 200.0),
                0,
            );
            ctx.paint_child(parent);
        }
        assert_eq!(parent_painted.get(), 1);
        assert_eq!(child_painted.get(), 1);

        let rects: Vec<Rect> = painter
            .commands()
            .iter()
            .filter_map(|c| match c {
                crate::paint::PaintCommand::Shape { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        assert!(
            rects.contains(&Rect::new(50.0, 60.0, 100.0, 100.0)),
            "parent background misplaced: {rects:?}"
        );
        assert!(
            rects.contains(&Rect::new(60.0, 80.0, 10.0, 10.0)),
            "child background misplaced: {rects:?}"
        );
    }

    #[test]
    fn paint_clip_is_restored_after_child() {
        let ui = ui();
        let store = WidgetStore::new();
        let mut registry = ObjectRegistry::new();
        let child = store.insert(&mut registry, Box::new(probe(Size::new(10.0, 10.0))));
        let interaction = Interaction::default();
        let overlays = OverlayList::new();
        let mut painter = Painter::new();
        let viewport = Rect::new(0.0, 0.0, 200.0, 200.0);
        painter.reset(viewport);
        {
            let mut ctx = PaintContext::new(
                &ui,
                &store,
                &interaction,
                &mut painter,
                &overlays,
                Point::ZERO,
                viewport,
                0,
            );
            ctx.push_clip(Rect::new(0.0, 0.0, 100.0, 100.0));
            assert_eq!(ctx.clip(), Rect::new(0.0, 0.0, 100.0, 100.0));
            ctx.paint_child(child);
            assert_eq!(ctx.clip(), Rect::new(0.0, 0.0, 100.0, 100.0));
            assert!(ctx.has_local_clip());
            ctx.pop_clip();
            assert_eq!(ctx.clip(), viewport);
            assert!(!ctx.has_local_clip());
        }
        // push clip + child's push/pop + pop clip
        assert_eq!(painter.clip_depth(), 0);
    }

    #[test]
    fn popup_layer_requests_are_deduplicated_and_drained() {
        let list = OverlayList::new();
        let id = ObjectId::new(3, 1);
        list.request_popup(id);
        list.request_popup(id);
        assert_eq!(list.take_popups(), vec![id], "one entry per widget");
        assert!(
            list.take_popups().is_empty(),
            "the queue belongs to a single frame"
        );
    }

    #[test]
    fn overlay_list_orders_by_z() {
        let list = OverlayList::new();
        let a = ObjectId::new(1, 1);
        let b = ObjectId::new(2, 1);
        list.insert(b, 5.0);
        list.insert(a, 1.0);
        assert_eq!(list.ordered(), vec![a, b]);
        assert!(list.contains(a));
        list.remove(a);
        assert!(!list.contains(a));
        assert_eq!(list.len(), 1);
        assert!(list.close_top());
        assert!(!list.close_top());
    }

    #[test]
    fn event_queue_is_fifo_and_records_cursor() {
        let queue = EventQueue::new();
        queue.push(ObjectId::new(1, 1), &UiEvent::FocusOut, EventPhase::Target);
        queue.push(
            ObjectId::new(2, 1),
            &UiEvent::FocusIn(crate::keyboard::FocusReason::Pointer),
            EventPhase::Target,
        );
        assert_eq!(queue.len(), 2);
        let first = queue.pop().unwrap();
        assert_eq!(first.target, ObjectId::new(1, 1));
        assert_eq!(queue.current().map(|c| c.1), Some(ObjectId::new(1, 1)));
        queue.set_cursor(CursorIcon::Text);
        assert_eq!(queue.take_cursor(), CursorIcon::Text);
        assert_eq!(queue.take_cursor(), CursorIcon::Default);
        queue.finish();
        assert!(queue.current().is_none());
        queue.clear();
        assert!(queue.is_empty());
    }

    #[test]
    fn frame_clock_delta_is_clamped() {
        let clock = FrameClock {
            frame: 1,
            delta: Duration::from_nanos(1),
            elapsed: Duration::from_secs(2),
        };
        assert!((clock.delta_seconds() - 0.001).abs() < 1e-6);
        assert!((clock.elapsed_seconds() - 2.0).abs() < 1e-6);
    }

    #[test]
    fn interaction_state() {
        let interaction = Interaction {
            hovered: Some(ObjectId::new(1, 1)),
            pressed: Some(ObjectId::new(2, 1)),
            focused: Some(ObjectId::new(3, 1)),
            ..Default::default()
        };
        let id = ObjectId::new(1, 1);
        let state = interaction.state_for(id, WidgetState::new());
        assert!(state.hovered);
        assert!(!state.pressed);
        assert!(!state.focused);
    }
}
