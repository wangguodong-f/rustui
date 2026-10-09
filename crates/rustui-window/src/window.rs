//! Per window state and the frame pipeline (§30 and §32 of the design documents).
//!
//! A [`Window`] owns everything that belongs to *one* window: the content root, the focus
//! manager, the event queue, the pointer interaction state, the overlay list and the painter
//! holding the paint commands of the last frame. Shared services ([`UiContext`],
//! [`WidgetStore`] and [`ObjectRegistry`]) live in [`crate::WindowManager`] and are handed to
//! the window for the duration of a call: windows stay independent without duplicating the
//! theme and the text engine (§52: no `Rc<RefCell<T>>` object model).
//!
//! The frame pipeline is `update -> layout -> paint`, exactly as §32 requires.

use rustui_core::{
    ArrangeContext, CursorIcon, EventQueue, FocusManager, FrameClock, Interaction, MeasureContext,
    ObjectId, ObjectRegistry, OverlayList, PaintContext, Painter, Point, Rect, Size, TextArena,
    UiContext, UpdateContext, WidgetStore,
};

/// Handle of one window inside a [`crate::WindowManager`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(u64);

impl WindowId {
    /// Id used for "no window".
    pub const INVALID: WindowId = WindowId(u64::MAX);

    pub const fn new(raw: u64) -> Self {
        WindowId(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Debug for WindowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WindowId({})", self.0)
    }
}

impl std::fmt::Display for WindowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Logical configuration used when a window is registered with a [`crate::WindowManager`].
///
/// The real operating system window is created by `rustui-platform`; this struct only carries
/// what the window layer itself has to know.
#[derive(Clone, Debug)]
pub struct WindowOptions {
    pub title: String,
    /// Size in logical pixels.
    pub size: Size,
    /// Physical pixels per logical pixel, as reported by the platform.
    pub scale_factor: f64,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: String::from("RustUI"),
            size: Size::new(800.0, 600.0),
            scale_factor: 1.0,
        }
    }
}

impl WindowOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = Size::new(width, height);
        self
    }

    pub fn scale_factor(mut self, factor: f64) -> Self {
        self.scale_factor = factor;
        self
    }
}

/// What happened while one event was dispatched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventOutcome {
    /// The event reached a widget that consumed it.
    pub handled: bool,
    /// The window should close: the window system asked for it and no widget vetoed the request.
    pub close_requested: bool,
    /// The UI state changed; the caller should schedule a redraw.
    pub request_redraw: bool,
}

/// Direction of tab navigation (§12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDirection {
    Forward,
    Backward,
}

/// One window of a [`crate::WindowManager`].
///
/// Windows are created through [`crate::WindowManager::create_window`]; the struct is public so
/// that applications can read the state produced by the last frame (`painter`, `cursor`,
/// `focused`, ...).
pub struct Window {
    pub(crate) id: WindowId,
    pub(crate) title: String,
    /// Logical size of the drawable area.
    pub(crate) size: Size,
    pub(crate) scale_factor: f64,
    pub(crate) root: ObjectId,
    /// `false` while the tree still has to be measured and arranged.
    pub(crate) laid_out: bool,
    pub(crate) queue: EventQueue,
    pub(crate) focus: FocusManager,
    pub(crate) overlays: OverlayList,
    pub(crate) interaction: Interaction,
    pub(crate) painter: Painter,
    pub(crate) cursor: CursorIcon,
    pub(crate) close_requested: bool,
}

impl std::fmt::Debug for Window {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Window")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("size", &self.size)
            .field("scale_factor", &self.scale_factor)
            .field("root", &self.root)
            .field("focused", &self.focus.focused())
            .finish()
    }
}

impl Window {
    pub(crate) fn new(id: WindowId, options: &WindowOptions) -> Self {
        Self {
            id,
            title: options.title.clone(),
            size: options.size,
            scale_factor: options.scale_factor,
            root: ObjectId::INVALID,
            laid_out: false,
            queue: EventQueue::new(),
            focus: FocusManager::new(),
            overlays: OverlayList::new(),
            interaction: Interaction {
                viewport: Rect::new(
                    0.0,
                    0.0,
                    options.size.width.max(0.0),
                    options.size.height.max(0.0),
                ),
                ..Interaction::default()
            },
            painter: Painter::new(),
            cursor: CursorIcon::Default,
            close_requested: false,
        }
    }

    // ---- identity -----------------------------------------------------

    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    /// Logical size of the drawable area.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Logical size change; the tree is re-laid out before the next frame.
    pub fn set_size(&mut self, size: Size) {
        if self.size != size {
            self.size = size;
            self.interaction.viewport =
                Rect::new(0.0, 0.0, size.width.max(0.0), size.height.max(0.0));
            self.laid_out = false;
        }
    }

    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    pub fn set_scale_factor(&mut self, factor: f64) {
        if (self.scale_factor - factor).abs() > f64::EPSILON {
            self.scale_factor = factor;
            self.laid_out = false;
        }
    }

    /// Size of the drawable area in physical pixels (what the surface expects).
    pub fn physical_size(&self) -> [u32; 2] {
        [
            (self.size.width.max(0.0) * self.scale_factor as f32)
                .round()
                .max(1.0) as u32,
            (self.size.height.max(0.0) * self.scale_factor as f32)
                .round()
                .max(1.0) as u32,
        ]
    }

    /// Content root of this window.
    pub fn root(&self) -> ObjectId {
        self.root
    }

    pub(crate) fn set_root(&mut self, root: ObjectId) {
        self.root = root;
        self.laid_out = false;
    }

    // ---- state exposed to the platform layer ---------------------------

    /// Object owning keyboard focus.
    pub fn focused(&self) -> Option<ObjectId> {
        self.focus.focused()
    }

    /// Cursor the platform should show (updated while events are dispatched).
    pub fn cursor(&self) -> CursorIcon {
        self.cursor
    }

    /// Pointer / focus / hover state used to derive widget pseudo states.
    pub fn interaction(&self) -> &Interaction {
        &self.interaction
    }

    /// Paint commands of the last frame, in painter order.
    pub fn painter(&self) -> &Painter {
        &self.painter
    }

    /// Strings arena referenced by [`rustui_core::PaintCommand::Text`].
    pub fn arena(&self) -> &TextArena {
        self.painter.arena()
    }

    /// `true` once the window system asked to close this window.
    pub fn close_requested(&self) -> bool {
        self.close_requested
    }

    pub(crate) fn take_close_request(&mut self) -> bool {
        std::mem::take(&mut self.close_requested)
    }

    /// The overlay (popup / menu / dialog) roots currently shown above the content.
    pub fn overlays(&self) -> Vec<ObjectId> {
        self.overlays.ordered()
    }

    /// Shows `id` above the window content (§ popups, menus, dialogs).
    ///
    /// Widgets normally call `EventContext::request_overlay` instead; this entry point is for
    /// application code that owns a dialog tree directly.
    pub fn add_overlay(&mut self, id: ObjectId, z: f32) {
        self.overlays.insert(id, z);
    }

    /// Stops showing `id` as an overlay.
    pub fn remove_overlay(&mut self, id: ObjectId) {
        self.overlays.remove(id);
    }

    // ---- frame pipeline (§32) ------------------------------------------

    /// Runs [`rustui_core::Widget::update`] over the whole tree.
    ///
    /// Containers do not propagate the call themselves (their `update` is a no-op by default),
    /// so the window layer walks the subtree: every widget is updated exactly once per frame.
    pub(crate) fn update(
        &mut self,
        ui: &UiContext,
        store: &WidgetStore,
        registry: &ObjectRegistry,
        clock: FrameClock,
    ) {
        self.interaction.focused = self.focus.focused();
        self.interaction.captured = self.queue.captured();

        let mut ids = Vec::new();
        if self.root.is_valid() && store.contains(self.root) {
            ids.extend(store.subtree(self.root));
        }
        // Standalone overlays are roots of their own subtree and are not covered above.
        for id in self.overlays.ordered() {
            if registry.root_of(id) == Some(self.root) {
                continue;
            }
            for node in store.subtree(id) {
                if !ids.contains(&node) {
                    ids.push(node);
                }
            }
        }
        if ids.is_empty() {
            return;
        }
        let ctx = UpdateContext::new(ui, store, &self.interaction, clock, 0);
        for id in ids {
            ctx.update_child(id);
        }
    }

    /// Measures and arranges the content root and every standalone overlay.
    pub(crate) fn layout(
        &mut self,
        ui: &UiContext,
        store: &WidgetStore,
        registry: &ObjectRegistry,
    ) {
        self.laid_out = true;
        let viewport = Rect::new(
            0.0,
            0.0,
            self.size.width.max(0.0),
            self.size.height.max(0.0),
        );

        if self.root.is_valid() && store.contains(self.root) {
            let measure = MeasureContext::new(ui, store, &self.interaction, 0);
            measure.measure_child(self.root, viewport.size());
            let arrange = ArrangeContext::new(ui, store, &self.interaction, 0);
            arrange.arrange_child(self.root, viewport);
        }

        // Overlays that live outside the content tree have no parent to arrange them.
        for id in self.overlays.ordered() {
            if registry.root_of(id) == Some(self.root) || !store.contains(id) {
                continue;
            }
            let measure = MeasureContext::new(ui, store, &self.interaction, 0);
            let size = measure.measure_child(id, viewport.size());
            let origin = store
                .get(id)
                .map(|widget| widget.base().window_bounds().origin())
                .unwrap_or(Point::ZERO);
            let arrange = ArrangeContext::new(ui, store, &self.interaction, 0);
            arrange.arrange_child(
                id,
                Rect::new(
                    origin.x,
                    origin.y,
                    size.width.max(0.0),
                    size.height.max(0.0),
                ),
            );
        }
    }

    /// Emits the paint commands of one frame into [`Window::painter`].
    ///
    /// The content root is painted first, then every overlay root sorted by its `z` value, and
    /// finally the popup layers — an open drop down panel belongs above all of them.
    pub(crate) fn paint(&mut self, ui: &UiContext, store: &WidgetStore, registry: &ObjectRegistry) {
        let viewport = Rect::new(
            0.0,
            0.0,
            self.size.width.max(0.0),
            self.size.height.max(0.0),
        );
        self.painter.reset(viewport);
        // The window background belongs to the theme; everything else is painted on top of it.
        self.painter.fill_all(ui.theme().colors.window_background);

        let origin = if self.root.is_valid() {
            store
                .get(self.root)
                .map(|widget| widget.base().window_bounds().origin())
                .unwrap_or(Point::ZERO)
        } else {
            Point::ZERO
        };

        {
            let mut ctx = PaintContext::new(
                ui,
                store,
                &self.interaction,
                &mut self.painter,
                &self.overlays,
                origin,
                viewport,
                0,
            );
            if self.root.is_valid() && store.contains(self.root) {
                if let Some(mut widget) = store.take(self.root) {
                    widget.paint(&mut ctx);
                    store.put(self.root, widget);
                }
            }
            for id in self.overlays.ordered() {
                // Overlays inside the content tree are painted by their parent.
                if id == self.root || registry.root_of(id) == Some(self.root) {
                    continue;
                }
                ctx.paint_overlay(id);
            }
            // Popup layers last: drawn with only the window clipping them, so a panel opened by
            // a widget anywhere in the tree stays visible above later siblings (T-4).
            for id in self.overlays.take_popups() {
                ctx.paint_popup(id);
            }
        }

        // Overlay requests made while painting are applied for the next frame.
        self.overlays.apply_pending();
    }

    // ---- queries used by the manager / tests ----------------------------

    /// Deepest widget under `position` (window coordinates), walking overlays first.
    pub fn hit_test(&self, store: &WidgetStore, position: Point) -> Option<ObjectId> {
        let path = self.hit_path(store, position);
        path.last().copied()
    }

    /// `true` once the tree has been measured and arranged at the current size.
    pub fn is_laid_out(&self) -> bool {
        self.laid_out
    }
}
