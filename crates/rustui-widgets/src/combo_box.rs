//! Combo box: a closed button that opens a list of options.

use std::fmt;

use rustui_core::{
    impl_widget_base, ArrangeContext, EdgeRadius, EventContext, EventResult, Fill, FocusReason,
    MeasureContext, MouseButton, NamedKey, PaintContext, Point, Rect, Shared, SharedProperty,
    Signal, Size, TextRole, TextStyle, UiContext, UiEvent, Widget, WidgetBase,
};
use rustui_model::{ListModel, StringListModel};

use crate::common::{center_in, control_radius, focus_ring};

/// Rows the popup shows before it scrolls (the control itself never grows).
const MAX_POPUP_ROWS: usize = 8;

/// A drop down list (a "select" control).
///
/// The panel is painted by the control itself — it is *not* a child widget — so opening a combo
/// costs no widget creation and no overlay bookkeeping. It is drawn in the window's popup layer
/// (T-4) instead: after the whole tree, clipped by the window alone. Hit testing is overridden
/// accordingly: [`Widget::hit_test`] also accepts points inside the open popup.
///
/// ```ignore
/// let combo = ComboBox::new(["Small", "Medium", "Large"]);
/// combo.selection_changed().connect(|index| println!("picked {index}"));
/// ```
pub struct ComboBox {
    base: WidgetBase,
    items: Shared<Box<dyn ListModel>>,
    selection: SharedProperty<Option<usize>>,
    open: bool,
    hover: Option<usize>,
    item_height: f32,
    /// Window's drawable area as last seen in [`Widget::arrange`]; the popup uses it to pick a
    /// side (T-4). [`Widget::hit_test`] gets no context, so it has to be cached.
    viewport: Rect,
    selection_changed: Signal<Option<usize>>,
}

impl ComboBox {
    /// A combo box over a fresh [`StringListModel`].
    pub fn new(items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let model = StringListModel::from_values(items);
        Self::with_items(Shared::new(Box::new(model) as Box<dyn ListModel>))
    }

    /// A combo box over `items`. Keep a clone of the handle to change the options later.
    pub fn with_items(items: Shared<Box<dyn ListModel>>) -> Self {
        Self {
            base: WidgetBase::new(),
            items,
            selection: SharedProperty::new(None),
            open: false,
            hover: None,
            item_height: 24.0,
            viewport: Rect::ZERO,
            selection_changed: Signal::new(),
        }
    }

    /// The shared list of options.
    pub fn items(&self) -> Shared<Box<dyn ListModel>> {
        self.items.clone()
    }

    /// Index of the selected option, if any.
    pub fn selected(&self) -> Option<usize> {
        self.selection.get()
    }

    /// Selects an option without notifying observers (use [`ComboBox::select`] for that).
    pub fn set_selected(&mut self, index: Option<usize>) {
        self.selection.set(index);
    }

    /// Selects `index` and emits [`ComboBox::selection_changed`].
    pub fn select(&mut self, index: Option<usize>) {
        self.selection.set(index);
        self.selection_changed.emit(&index);
    }

    /// Shared "selected index" property (clone the handle to observe it).
    pub fn selection_property(&self) -> SharedProperty<Option<usize>> {
        self.selection.clone()
    }

    /// Fires whenever the user (or the code) picks an option.
    pub fn selection_changed(&self) -> Signal<Option<usize>> {
        self.selection_changed.clone()
    }

    /// `true` while the popup is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Text of the selected option (empty when nothing is selected).
    pub fn selected_text(&self) -> String {
        match self.selection.get() {
            Some(index) => self.items.with(|items| items.data(index).as_text()),
            None => String::new(),
        }
    }

    fn row_count(&self) -> usize {
        self.items.with(|items| items.row_count())
    }

    /// Number of rows the popup shows (never more than [`MAX_POPUP_ROWS`]).
    fn popup_rows(&self) -> usize {
        self.row_count().min(MAX_POPUP_ROWS)
    }

    /// Popup rectangle in window coordinates; `None` while closed.
    ///
    /// Opens below the control; when the window has not enough room there it opens above it, and
    /// if neither side fits, the roomier side wins — the window clips whatever still overflows
    /// (T-4). The side is decided against [`Self::viewport`], cached at arrange time, because
    /// neither `hit_test` nor `row_at` has a context to ask.
    fn popup_rect(&self) -> Option<Rect> {
        if !self.open {
            return None;
        }
        let bounds = self.base.window_bounds();
        let rows = self.popup_rows();
        if rows == 0 {
            return None;
        }
        let height = rows as f32 * self.item_height;
        let y = if self.viewport.height > 0.0 {
            let below = self.viewport.bottom() - bounds.bottom();
            let above = bounds.y - self.viewport.y;
            if below >= height || above <= below {
                bounds.bottom()
            } else {
                bounds.y - height
            }
        } else {
            // Unknown viewport (the window never laid out): keep the historical behaviour.
            bounds.bottom()
        };
        Some(Rect::new(bounds.x, y, bounds.width, height))
    }

    /// Row under a window `position`, when it is over the popup.
    fn row_at(&self, position: Point) -> Option<usize> {
        let popup = self.popup_rect()?;
        if !popup.contains(position) {
            return None;
        }
        let index = ((position.y - popup.y) / self.item_height).floor();
        if index < 0.0 {
            return None;
        }
        let row = index as usize;
        (row < self.popup_rows()).then_some(row)
    }

    fn text_style(&self, ui: &UiContext, enabled: bool) -> TextStyle {
        let color = if enabled {
            ui.theme().colors.text
        } else {
            ui.theme().colors.text_disabled
        };
        ui.theme().text(TextRole::Body, Some(color))
    }
}

impl fmt::Debug for ComboBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComboBox")
            .field("base", &self.base)
            .field("options", &self.row_count())
            .field("selected", &self.selected())
            .field("open", &self.open)
            .finish()
    }
}

impl_widget_base!(ComboBox);

impl Widget for ComboBox {
    fn kind(&self) -> &'static str {
        "ComboBox"
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn is_opaque(&self) -> bool {
        true
    }

    fn hit_test(&self, position: Point) -> bool {
        self.base.window_bounds().contains(position)
            || self
                .popup_rect()
                .is_some_and(|popup| popup.contains(position))
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let theme = ctx.ui().theme();
        let pad = theme.content_padding();
        let style = self.text_style(ctx.ui(), true);
        let mut width = 0.0f32;
        let rows = self.row_count();
        for row in 0..rows {
            let text = self.items.with(|items| items.data(row).as_text());
            let size = ctx.measure_text(&text, &style, None);
            width = width.max(size.width);
        }
        // Room for the text, the chevron and the paddings.
        width += pad.horizontal() + theme.spacing.lg;
        let height = (ctx.measure_text(" ", &style, None).height + pad.vertical())
            .max(theme.control_height());
        let size = Size::new(
            if available.width.is_finite() {
                width.min(available.width.max(0.0))
            } else {
                width
            },
            if available.height.is_finite() {
                height.min(available.height.max(0.0))
            } else {
                height
            },
        );
        self.item_height = theme.control_height();
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
        self.item_height = ctx.ui().theme().control_height();
        self.viewport = ctx.viewport();
    }

    fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult {
        let id = self.id();
        let enabled = self.base.is_enabled();
        let bounds = self.base.window_bounds();
        match event {
            UiEvent::MouseDown {
                button: MouseButton::Left,
                position,
            } if enabled => {
                if let Some(row) = self.row_at(*position) {
                    self.select(Some(row));
                    self.open = false;
                    self.hover = None;
                    ctx.request_redraw();
                    ctx.accept();
                    return EventResult::Handled;
                }
                if bounds.contains(*position) {
                    self.open = !self.open;
                    self.hover = self.selection.get();
                    if self.open {
                        ctx.set_focus(id, FocusReason::Pointer);
                    }
                    ctx.request_redraw();
                    ctx.accept();
                    return EventResult::Handled;
                }
                EventResult::Unhandled
            }
            UiEvent::MouseDoubleClick { position, .. } if enabled => {
                // A second click on a row behaves like a first one.
                if let Some(row) = self.row_at(*position) {
                    self.select(Some(row));
                    self.open = false;
                    ctx.request_redraw();
                    ctx.accept();
                    return EventResult::Handled;
                }
                EventResult::Unhandled
            }
            UiEvent::MouseMove { position } if enabled => {
                let hover = if self.open {
                    self.row_at(*position)
                } else {
                    None
                };
                if hover != self.hover {
                    self.hover = hover;
                    ctx.request_redraw();
                }
                if self.open || bounds.contains(*position) {
                    EventResult::Handled
                } else {
                    EventResult::Unhandled
                }
            }
            UiEvent::FocusOut => {
                if self.open {
                    self.open = false;
                    self.hover = None;
                    ctx.request_redraw();
                }
                EventResult::Handled
            }
            UiEvent::KeyDown { key, .. } if enabled => {
                let rows = self.row_count();
                if key.is_named(NamedKey::Escape) {
                    if self.open {
                        self.open = false;
                        self.hover = None;
                        ctx.request_redraw();
                        ctx.accept();
                        return EventResult::Handled;
                    }
                    return EventResult::Unhandled;
                }
                if rows == 0 {
                    return EventResult::Unhandled;
                }
                let current = self.hover.or(self.selection.get());
                if key.is_named(NamedKey::ArrowDown) {
                    let next = current.map_or(0, |row| (row + 1).min(rows - 1));
                    if self.open {
                        self.hover = Some(next);
                    } else {
                        self.select(Some(next));
                    }
                } else if key.is_named(NamedKey::ArrowUp) {
                    let next = current.map_or(0, |row| row.saturating_sub(1));
                    if self.open {
                        self.hover = Some(next);
                    } else {
                        self.select(Some(next));
                    }
                } else if key.is_named(NamedKey::Home) {
                    if self.open {
                        self.hover = Some(0);
                    } else {
                        self.select(Some(0));
                    }
                } else if key.is_named(NamedKey::End) {
                    if self.open {
                        self.hover = Some(rows - 1);
                    } else {
                        self.select(Some(rows - 1));
                    }
                } else if key.is_named(NamedKey::Return) || key.is_named(NamedKey::Space) {
                    if self.open {
                        if let Some(row) = self.hover {
                            self.select(Some(row));
                        }
                        self.open = false;
                    } else {
                        self.open = true;
                        self.hover = self.selection.get();
                    }
                } else {
                    return EventResult::Unhandled;
                }
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::MouseLeave if self.open => {
                self.hover = None;
                ctx.request_redraw();
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

        // --- control -------------------------------------------------------
        let bg = if !state.enabled {
            theme.colors.surface_alt
        } else if state.pressed || state.hovered {
            theme.button.background(state)
        } else {
            theme.colors.surface
        };
        let mut fill = Fill::rounded(bg, theme.radius.sm);
        fill.border = theme.input.border(state);
        ctx.painter().styled_rect(bounds, radius, fill);

        let style = self.text_style(ctx.ui(), state.enabled);
        let pad = theme.content_padding();
        let text = self.selected_text();
        let text_area = Rect::new(
            bounds.x + pad.left,
            bounds.y + pad.top,
            (bounds.width - pad.horizontal() - theme.spacing.lg).max(0.0),
            (bounds.height - pad.vertical()).max(0.0),
        );
        let shown = if text.is_empty() && self.selection.get().is_none() {
            String::new()
        } else {
            text
        };
        if !shown.is_empty() {
            let size = ctx.measure_text(&shown, &style, None);
            let position = center_in(text_area, Size::new(size.width, size.height));
            ctx.painter().text(position, &shown, &style);
        }

        // Chevron on the trailing edge.
        let cx = bounds.right() - theme.spacing.md;
        let cy = bounds.y + bounds.height * 0.5;
        let arm = theme.spacing.sm;
        let color = if state.enabled {
            theme.colors.text_muted
        } else {
            theme.colors.text_disabled
        };
        let points = if self.open {
            [
                Point::new(cx - arm, cy + arm * 0.5),
                Point::new(cx + arm, cy + arm * 0.5),
                Point::new(cx, cy - arm * 0.5),
            ]
        } else {
            [
                Point::new(cx - arm, cy - arm * 0.5),
                Point::new(cx + arm, cy - arm * 0.5),
                Point::new(cx, cy + arm * 0.5),
            ]
        };
        let painter = ctx.painter();
        let stroke = theme.borders.thin.max(1.5);
        painter.line(points[0], points[1], stroke, color);
        painter.line(points[1], points[2], stroke, color);
        painter.line(points[2], points[0], stroke, color);

        if state.focused {
            focus_ring(ctx, self.base.bounds(), theme.radius.sm);
        }

        // The panel is drawn in a second pass over the whole tree, so that it is neither cut by
        // an ancestor's clip nor overdrawn by a later sibling (T-4).
        if self.popup_rect().is_some() {
            ctx.request_popup(id);
        }
    }

    /// The open panel, painted after the content tree (T-4); see [`Widget::paint_popup`].
    fn paint_popup(&mut self, ctx: &mut PaintContext<'_>) {
        let Some(popup) = self.popup_rect() else {
            return;
        };
        let theme = ctx.ui().theme();
        let pad = theme.content_padding();
        let rows = self.popup_rows();
        // A soft shadow, then the surface.
        let shadow = Rect::new(
            popup.x + theme.spacing.xs * 0.5,
            popup.y + theme.spacing.xs * 0.5,
            popup.width,
            popup.height,
        );
        ctx.painter()
            .rounded_rect(shadow, theme.radius.sm, theme.colors.shadow);
        let popup_fill =
            Fill::rounded(theme.colors.surface, theme.radius.sm).with_border(rustui_core::Border {
                widths: rustui_core::EdgeWidths::all(theme.borders.thin),
                color: theme.colors.border_strong,
            });
        ctx.painter()
            .styled_rect(popup, EdgeRadius::all(theme.radius.sm), popup_fill);

        let list_style = self.text_style(ctx.ui(), true);
        let line_height = ctx.line_height(&list_style);
        let selected = self.selection.get();
        for row in 0..rows {
            let row_rect = Rect::new(
                popup.x,
                popup.y + row as f32 * self.item_height,
                popup.width,
                self.item_height,
            );
            if Some(row) == self.hover {
                ctx.painter().rect(row_rect, theme.colors.surface_hover);
            }
            if Some(row) == selected {
                ctx.painter().rect(row_rect, theme.colors.selection);
            }
            let text = self.items.with(|items| items.data(row).as_text());
            if !text.is_empty() {
                let size = ctx.measure_text(
                    &text,
                    &list_style,
                    Some((popup.width - pad.horizontal()).max(0.0)),
                );
                let position = center_in(
                    Rect::new(
                        row_rect.x + pad.left,
                        row_rect.y,
                        row_rect.width - pad.horizontal(),
                        row_rect.height,
                    ),
                    Size::new(size.width, line_height),
                );
                ctx.painter().text(position, &text, &list_style);
            }
        }
    }
}
