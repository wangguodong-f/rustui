//! Popup menu (context menu, menu bar dropdown).

use rustui_core::{
    impl_widget_base, ArrangeContext, EventContext, EventResult, MeasureContext, MouseButton,
    NamedKey, PaintContext, Point, Rect, Signal, Size, TextRole, UiEvent, Widget, WidgetBase,
};

use crate::common::{center_in, control_radius, focus_ring};

/// One entry of a [`Menu`].
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    /// Display text.
    pub label: String,
    /// Optional accelerator shown on the trailing edge.
    pub shortcut: Option<String>,
    /// `false` items are painted muted and never activate.
    pub enabled: bool,
    /// Separators have no label and no action.
    pub separator: bool,
}

impl MenuItem {
    /// A normal, enabled item.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            shortcut: None,
            enabled: true,
            separator: false,
        }
    }

    /// An accelerator such as `"Ctrl+O"`.
    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Marks the item as unavailable.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// A separator line.
    pub fn separator() -> Self {
        Self {
            label: String::new(),
            shortcut: None,
            enabled: false,
            separator: true,
        }
    }
}

/// A popup menu.
///
/// The menu is an ordinary widget that the application shows as an overlay — that keeps the
/// widget model free of hidden children (§52) while still getting correct stacking and hit
/// testing from the window layer:
///
/// ```ignore
/// let menu = Menu::new([MenuItem::new("Open").shortcut("Ctrl+O"), MenuItem::separator(),
///                       MenuItem::new("Quit")]);
/// let menu_id = wm.insert(Box::new(menu));
/// wm.add_overlay(window, menu_id, 10.0);
/// ```
///
/// Selecting an item emits [`Menu::activated`] with the item index and removes the menu from the
/// overlay list again (when it *is* an overlay).
#[derive(Debug)]
pub struct Menu {
    base: WidgetBase,
    items: Vec<MenuItem>,
    selected: Option<usize>,
    activated: Signal<usize>,
    closed: Signal<()>,
    row_height: f32,
}

impl Menu {
    /// A menu over `items`.
    pub fn new(items: impl IntoIterator<Item = MenuItem>) -> Self {
        Self {
            base: WidgetBase::new(),
            items: items.into_iter().collect(),
            selected: None,
            activated: Signal::new(),
            closed: Signal::new(),
            row_height: 24.0,
        }
    }

    /// The menu entries.
    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    /// Fires with the item index when the user picks an item.
    pub fn activated(&self) -> Signal<usize> {
        self.activated.clone()
    }

    /// Fires when the menu hides itself (selection, `Escape`, focus loss).
    pub fn closed(&self) -> Signal<()> {
        self.closed.clone()
    }

    /// Index of the highlighted item.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Moves the highlight to the next / previous *actionable* item.
    fn move_selection(&mut self, forward: bool) {
        if self.items.is_empty() {
            return;
        }
        let count = self.items.len();
        let start = self.selected.unwrap_or(if forward { count - 1 } else { 0 });
        let mut index = start;
        for _ in 0..count {
            index = if forward {
                (index + 1) % count
            } else {
                (index + count - 1) % count
            };
            if self.items[index].enabled && !self.items[index].separator {
                self.selected = Some(index);
                return;
            }
        }
        self.selected = None;
    }

    fn actionable(&self, index: Option<usize>) -> bool {
        index.is_some_and(|index| {
            self.items
                .get(index)
                .is_some_and(|item| item.enabled && !item.separator)
        })
    }

    /// Hides the menu: emits `closed` and drops the overlay, if any.
    fn hide(&mut self, ctx: &EventContext<'_>) {
        let id = self.id();
        if ctx.is_overlay(id) {
            ctx.remove_overlay(id);
        }
        self.closed.emit(&());
        ctx.request_redraw();
    }
}

impl_widget_base!(Menu);

impl Widget for Menu {
    fn kind(&self) -> &'static str {
        "Menu"
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn is_opaque(&self) -> bool {
        true
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let theme = ctx.ui().theme();
        let pad = theme.content_padding();
        let style = theme.text(TextRole::Body, Some(theme.colors.text));
        let mut width = 0.0f32;
        let mut height = 0.0f32;
        for item in &self.items {
            if item.separator {
                height += theme.spacing.xs * 2.0 + theme.borders.thin;
                continue;
            }
            let label = ctx.measure_text(&item.label, &style, None);
            let shortcut = match &item.shortcut {
                Some(text) => ctx.measure_text(text, &style, None).width,
                None => 0.0,
            };
            width = width.max(label.width + shortcut + theme.spacing.lg);
            height = height.max(label.height + pad.vertical());
        }
        self.row_height = theme.control_height();
        let size = Size::new(
            (width + pad.horizontal()).max(theme.control_height() * 3.0),
            height,
        );
        let size = Size::new(
            if available.width.is_finite() {
                size.width.min(available.width.max(0.0))
            } else {
                size.width
            },
            if available.height.is_finite() {
                size.height.min(available.height.max(0.0))
            } else {
                size.height
            },
        );
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
        self.row_height = ctx.ui().theme().control_height();
    }

    fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult {
        let enabled = self.base.is_enabled();
        let bounds = self.base.window_bounds();
        match event {
            UiEvent::MouseMove { position } if enabled => {
                let index = self.index_at(*position, bounds);
                if index != self.selected && index.is_some() {
                    self.selected = index;
                    ctx.request_redraw();
                } else if index.is_none() && self.selected.is_some() {
                    self.selected = None;
                    ctx.request_redraw();
                }
                EventResult::Handled
            }
            UiEvent::MouseDown {
                button: MouseButton::Left,
                position,
            } if enabled => {
                let index = self.index_at(*position, bounds);
                if let Some(index) = index {
                    if self.actionable(Some(index)) {
                        self.activated.emit(&index);
                        self.hide(ctx);
                        ctx.accept();
                        return EventResult::Handled;
                    }
                }
                // A click outside / on a disabled item keeps the menu open but is consumed so the
                // click does not reach the application behind the popup.
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::FocusOut => {
                self.hide(ctx);
                EventResult::Handled
            }
            UiEvent::KeyDown { key, .. } if enabled => {
                if key.is_named(NamedKey::Escape) {
                    self.hide(ctx);
                    ctx.accept();
                    return EventResult::Handled;
                }
                if key.is_named(NamedKey::ArrowDown) {
                    self.move_selection(true);
                } else if key.is_named(NamedKey::ArrowUp) {
                    self.move_selection(false);
                } else if key.is_named(NamedKey::Home) {
                    self.selected = self
                        .items
                        .iter()
                        .position(|item| item.enabled && !item.separator);
                } else if key.is_named(NamedKey::End) {
                    self.selected = self
                        .items
                        .iter()
                        .rposition(|item| item.enabled && !item.separator);
                } else if key.is_named(NamedKey::Return) || key.is_named(NamedKey::Space) {
                    if let Some(index) = self.selected {
                        if self.actionable(Some(index)) {
                            self.activated.emit(&index);
                            self.hide(ctx);
                            ctx.accept();
                            return EventResult::Handled;
                        }
                    }
                } else {
                    return EventResult::Unhandled;
                }
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::MouseLeave => {
                if self.selected.take().is_some() {
                    ctx.request_redraw();
                }
                EventResult::Unhandled
            }
            _ => EventResult::Unhandled,
        }
    }

    fn paint(&mut self, ctx: &mut PaintContext<'_>) {
        let id = self.id();
        let state = ctx.state_of(id, self.base.state());
        let theme = ctx.ui().theme();
        // Window coordinates: the painter takes no offset.
        let bounds = ctx.to_window_rect(self.base.bounds());
        let radius = control_radius(ctx);

        // Drop shadow, then the surface.
        let shadow = Rect::new(
            bounds.x + theme.spacing.xs * 0.5,
            bounds.y + theme.spacing.xs * 0.5,
            bounds.width,
            bounds.height,
        );
        ctx.painter()
            .rounded_rect(shadow, theme.radius.sm, theme.colors.shadow);
        let surface = rustui_core::Fill::rounded(theme.colors.surface, theme.radius.sm)
            .with_border(rustui_core::Border {
                widths: rustui_core::EdgeWidths::all(theme.borders.thin),
                color: theme.colors.border_strong,
            });
        ctx.painter().styled_rect(bounds, radius, surface);

        let pad = theme.content_padding();
        let style = theme.text(TextRole::Body, Some(theme.colors.text));
        let line_height = ctx.line_height(&style);

        // Rows are painted in their measured order: separators take a thin slice of the height.
        let mut y = bounds.y;
        for (index, item) in self.items.iter().enumerate() {
            if item.separator {
                let h = theme.spacing.xs * 2.0 + theme.borders.thin;
                let line = Rect::new(
                    bounds.x + pad.left,
                    y + (h - theme.borders.thin) * 0.5,
                    (bounds.width - pad.horizontal()).max(0.0),
                    theme.borders.thin,
                );
                ctx.painter().rect(line, theme.colors.border);
                y += h;
                continue;
            }
            let h = self.row_height.max(line_height + pad.vertical());
            let row = Rect::new(bounds.x, y, bounds.width, h);
            if self.selected == Some(index) && item.enabled {
                let highlight = if state.focused {
                    theme.colors.selection
                } else {
                    theme.colors.surface_hover
                };
                ctx.painter().rect(row, highlight);
            }
            let color = if item.enabled {
                theme.colors.text
            } else {
                theme.colors.text_disabled
            };
            let mut row_style = style.clone();
            row_style.color = color;
            let label_position = Point::new(bounds.x + pad.left, y + (h - line_height) * 0.5);
            ctx.painter().text(label_position, &item.label, &row_style);
            if let Some(shortcut) = &item.shortcut {
                let size = ctx.measure_text(shortcut, &row_style, None);
                let right = Rect::new(bounds.x, y, bounds.width - pad.right - theme.spacing.sm, h);
                let position = center_in(right, Size::new(size.width, line_height));
                let position = Point::new(right.right() - size.width, position.y);
                ctx.painter().text(position, shortcut, &row_style);
            }
            y += h;
        }

        if state.focused {
            focus_ring(ctx, self.base.bounds(), theme.radius.sm);
        }
    }
}

impl Menu {
    /// Index of the actionable row under a window `position`, if any.
    ///
    /// The vertical geometry mirrors `paint`: separators are thin, rows are at least one line tall.
    fn index_at(&self, position: Point, bounds: Rect) -> Option<usize> {
        if !bounds.contains(position) {
            return None;
        }
        let y = position.y - bounds.y;
        let mut top = 0.0f32;
        let row_height = self.row_height;
        for (index, item) in self.items.iter().enumerate() {
            let height = if item.separator {
                // The separator height is only known from the theme at paint time; using the row
                // height here keeps hit testing monotonic, which is all selection needs.
                row_height * 0.4
            } else {
                row_height.max(row_height)
            };
            if y >= top && y < top + height {
                return if item.separator { None } else { Some(index) };
            }
            top += height;
        }
        None
    }
}
