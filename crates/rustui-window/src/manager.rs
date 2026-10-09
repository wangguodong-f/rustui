//! Shared services and the collection of windows (§30, §31).
//!
//! [`WindowManager`] owns the pieces that every window shares — theme, text engine, clipboard
//! ([`UiContext`]), widget payloads ([`WidgetStore`]) and object identity
//! ([`ObjectRegistry`]) — plus the list of [`Window`]s. Windows are only given those services
//! for the duration of a call, so nothing is shared through `Rc<RefCell<T>>` (§52).

use rustui_core::{
    FocusReason, FrameClock, ObjectId, ObjectRegistry, Painter, Rect, TextMeasurer, UiContext,
    UiEvent, Widget, WidgetStore,
};
use rustui_renderer::RenderInput;

use crate::window::{EventOutcome, FocusDirection, Window, WindowId, WindowOptions};

/// Owns every window of the application together with the services they share.
///
/// ```text
/// WindowManager
/// ├── UiContext      theme, text engine, clipboard, redraw flag
/// ├── WidgetStore    widget payloads
/// ├── ObjectRegistry object identity and parent / child links
/// └── Window[]       per window focus, queue, interaction, overlays, painter
/// ```
pub struct WindowManager {
    ui: UiContext,
    store: WidgetStore,
    registry: ObjectRegistry,
    windows: Vec<Window>,
    next_window: u64,
}

impl std::fmt::Debug for WindowManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowManager")
            .field("windows", &self.windows)
            .field("objects", &self.store.len())
            .finish()
    }
}

impl WindowManager {
    /// Creates a manager whose widgets are measured by `text`.
    pub fn new(text: Box<dyn TextMeasurer>) -> Self {
        Self {
            ui: UiContext::new(text),
            store: WidgetStore::new(),
            registry: ObjectRegistry::new(),
            windows: Vec::new(),
            next_window: 0,
        }
    }

    // ---- shared services ------------------------------------------------

    /// Theme, text engine, clipboard and the redraw flag.
    pub fn ui(&self) -> &UiContext {
        &self.ui
    }

    pub fn ui_mut(&mut self) -> &mut UiContext {
        &mut self.ui
    }

    /// Payload storage of every widget of every window.
    pub fn store(&self) -> &WidgetStore {
        &self.store
    }

    /// Identity and parent / child links.
    pub fn registry(&self) -> &ObjectRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut ObjectRegistry {
        &mut self.registry
    }

    /// `true` while a widget is waiting for a redraw (`UiContext::request_redraw`).
    pub fn redraw_pending(&self) -> bool {
        self.ui.redraw().is_pending()
    }

    /// Consumes the pending redraw request; returns `true` when one was recorded.
    pub fn take_redraw(&mut self) -> bool {
        self.ui.redraw().take()
    }

    // ---- building the tree ---------------------------------------------

    /// Registers a widget as a new object; returns its id.
    pub fn insert(&mut self, widget: Box<dyn Widget>) -> ObjectId {
        self.store.insert(&mut self.registry, widget)
    }

    /// Attaches `child` below `parent`.
    pub fn add_child(&mut self, parent: ObjectId, child: ObjectId) -> bool {
        if !self.store.contains(parent) || !self.store.contains(child) {
            return false;
        }
        if self.registry.set_parent(child, Some(parent)).is_err() {
            return false;
        }
        let Some(mut widget) = self.store.take(parent) else {
            return false;
        };
        widget.base_mut().add_child(child);
        self.store.put(parent, widget);
        true
    }

    /// Detaches `child` from `parent`, keeping the object alive.
    pub fn remove_child(&mut self, parent: ObjectId, child: ObjectId) {
        self.store.remove_child(&mut self.registry, parent, child);
    }

    /// Destroys `id` and everything below it; returns the removed objects.
    pub fn destroy(&mut self, id: ObjectId) -> Vec<ObjectId> {
        self.store.destroy_subtree(&mut self.registry, id)
    }

    /// Makes `widget` the content root of `window`, destroying the previous content.
    pub fn set_root(&mut self, window: WindowId, widget: Box<dyn Widget>) -> ObjectId {
        let id = self.insert(widget);
        self.set_root_id(window, id);
        id
    }

    /// Makes an existing object the content root of `window`.
    ///
    /// The object is detached from its current parent first, so a tree that was built
    /// off-screen can be promoted to a window root without rebuilding it.
    pub fn set_root_id(&mut self, window: WindowId, root: ObjectId) -> bool {
        if !self.store.contains(root) {
            return false;
        }
        let previous = self.window(window).map(Window::root);
        if let Some(parent) = self.registry.parent(root) {
            self.store.remove_child(&mut self.registry, parent, root);
        }
        if let Some(target) = self.window_mut(window) {
            target.set_root(root);
        } else {
            return false;
        }
        if let Some(previous) = previous {
            if previous.is_valid() && previous != root {
                self.store.destroy_subtree(&mut self.registry, previous);
            }
        }
        true
    }

    // ---- windows ---------------------------------------------------------

    /// Registers a window; the platform window itself is created by `rustui-platform`.
    pub fn create_window(&mut self, options: WindowOptions) -> WindowId {
        let id = WindowId::new(self.next_window);
        self.next_window = self.next_window.saturating_add(1);
        self.windows.push(Window::new(id, &options));
        id
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|window| window.id() == id)
    }

    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|window| window.id() == id)
    }

    pub fn windows(&self) -> &[Window] {
        &self.windows
    }

    /// Ids of every open window, in creation order.
    pub fn window_ids(&self) -> Vec<WindowId> {
        self.windows.iter().map(Window::id).collect()
    }

    /// Id of the first window, the usual "main window" of an application.
    pub fn primary_window(&self) -> Option<WindowId> {
        self.windows.first().map(Window::id)
    }

    /// Closes a window and destroys its content tree.
    pub fn destroy_window(&mut self, id: WindowId) -> bool {
        let Some(position) = self.windows.iter().position(|window| window.id() == id) else {
            return false;
        };
        let window = self.windows.remove(position);
        if window.root().is_valid() {
            self.store
                .destroy_subtree(&mut self.registry, window.root());
        }
        true
    }

    /// `true` while the window system asked to close `window` and no widget vetoed it.
    pub fn close_requested(&self, id: WindowId) -> bool {
        self.window(id)
            .map(Window::close_requested)
            .unwrap_or(false)
    }

    /// Reads and clears the pending close request.
    pub fn take_close_request(&mut self, id: WindowId) -> bool {
        self.window_mut(id)
            .map(Window::take_close_request)
            .unwrap_or(false)
    }

    /// Shows `id` above the content of `window`.
    pub fn add_overlay(&mut self, id: WindowId, overlay: ObjectId, z: f32) -> bool {
        match self.window_mut(id) {
            Some(window) => {
                window.add_overlay(overlay, z);
                true
            }
            None => false,
        }
    }

    /// Stops showing `overlay` above the content of `window`.
    pub fn remove_overlay(&mut self, id: WindowId, overlay: ObjectId) -> bool {
        match self.window_mut(id) {
            Some(window) => {
                window.remove_overlay(overlay);
                true
            }
            None => false,
        }
    }

    // ---- input -----------------------------------------------------------

    /// Delivers one event to `window` and reports what happened.
    pub fn handle_event(&mut self, window: WindowId, event: UiEvent) -> EventOutcome {
        let ui = &self.ui;
        let store = &self.store;
        let registry = &self.registry;
        let Some(target) = self
            .windows
            .iter_mut()
            .find(|candidate| candidate.id() == window)
        else {
            return EventOutcome::default();
        };
        target.dispatch(ui, store, registry, &event)
    }

    /// Deepest widget under `position` (window coordinates).
    pub fn hit_test(&self, window: WindowId, position: rustui_core::Point) -> Option<ObjectId> {
        self.window(window)
            .and_then(|window| window.hit_test(&self.store, position))
    }

    /// Rectangle anchoring the input method candidate window of `window`, in **window**
    /// coordinates (T-1).
    ///
    /// It is asked of the widget that owns keyboard focus, so it follows the caret after every
    /// layout change. `None` means "no input method wanted right now" — nothing focused, or the
    /// focused widget does not edit text — which the wiring layer turns into
    /// `set_ime_allowed(false)`.
    pub fn ime_area(&self, window: WindowId) -> Option<Rect> {
        let focused = self.window(window)?.focused()?;
        self.store.get(focused)?.ime_cursor_area(&self.ui)
    }

    /// Moves keyboard focus to the next focusable widget (Tab).
    pub fn focus_next(&mut self, window: WindowId) -> Option<ObjectId> {
        self.focus_move(window, FocusDirection::Forward)
    }

    /// Moves keyboard focus to the previous focusable widget (Shift+Tab).
    pub fn focus_previous(&mut self, window: WindowId) -> Option<ObjectId> {
        self.focus_move(window, FocusDirection::Backward)
    }

    fn focus_move(&mut self, window: WindowId, direction: FocusDirection) -> Option<ObjectId> {
        let ui = &self.ui;
        let store = &self.store;
        let registry = &self.registry;
        let target = self
            .windows
            .iter_mut()
            .find(|candidate| candidate.id() == window)?;
        target.focus_move(ui, store, registry, direction)
    }

    /// Moves focus to `id` and delivers the notifications; `false` when `id` is unknown.
    pub fn set_focus(&mut self, window: WindowId, id: ObjectId) -> bool {
        let ui = &self.ui;
        let store = &self.store;
        let registry = &self.registry;
        let Some(target) = self
            .windows
            .iter_mut()
            .find(|candidate| candidate.id() == window)
        else {
            return false;
        };
        target.focus_object(ui, store, registry, id, FocusReason::Programmatic)
    }

    // ---- frame pipeline (§32) ---------------------------------------------

    /// Runs `update` over the tree of `window`.
    pub fn update(&mut self, window: WindowId, clock: FrameClock) -> bool {
        let ui = &self.ui;
        let store = &self.store;
        let registry = &self.registry;
        let Some(target) = self
            .windows
            .iter_mut()
            .find(|candidate| candidate.id() == window)
        else {
            return false;
        };
        target.update(ui, store, registry, clock);
        true
    }

    /// Measures and arranges `window` at its current size.
    pub fn layout(&mut self, window: WindowId) -> bool {
        let ui = &self.ui;
        let store = &self.store;
        let registry = &self.registry;
        let Some(target) = self
            .windows
            .iter_mut()
            .find(|candidate| candidate.id() == window)
        else {
            return false;
        };
        target.layout(ui, store, registry);
        true
    }

    /// Rebuilds the paint commands of `window`.
    pub fn paint(&mut self, window: WindowId) -> bool {
        let ui = &self.ui;
        let store = &self.store;
        let registry = &self.registry;
        let Some(target) = self
            .windows
            .iter_mut()
            .find(|candidate| candidate.id() == window)
        else {
            return false;
        };
        target.paint(ui, store, registry);
        true
    }

    /// Runs the complete frame pipeline and returns the fresh paint commands.
    pub fn render(&mut self, window: WindowId, clock: FrameClock) -> Option<&Painter> {
        let ui = &self.ui;
        let store = &self.store;
        let registry = &self.registry;
        let target = self
            .windows
            .iter_mut()
            .find(|candidate| candidate.id() == window)?;
        target.update(ui, store, registry, clock);
        target.layout(ui, store, registry);
        target.paint(ui, store, registry);
        Some(target.painter())
    }

    /// Everything the GPU renderer needs for one frame of `window`.
    ///
    /// Returns `None` when the window is unknown or has not been painted yet.
    pub fn render_input(&self, window: WindowId) -> Option<RenderInput<'_>> {
        let target = self.window(window)?;
        // The engine is shared by every window, so the scale of the window being drawn wins:
        // rasterization runs while this input is consumed, and each display scale keeps its own
        // rasters (C-048). Layout and measurement are in logical pixels and ignore the scale.
        self.ui
            .text()
            .set_scale_factor(target.scale_factor() as f32);
        if target.painter().is_empty() {
            return None;
        }
        Some(RenderInput::new(
            target.painter().commands(),
            target.painter().arena(),
            target.scale_factor() as f32,
            self.ui.text().glyph_source(),
        ))
    }
}
