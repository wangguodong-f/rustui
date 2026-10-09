//! Multi-column table with a resizable header and virtualized rows (design document §29).

use std::borrow::Cow;
use std::fmt;

use rustui_core::{
    impl_widget_base, scroll_delta_to_offset, ArrangeContext, EventContext, EventResult, Fill,
    MeasureContext, MouseButton, NamedKey, PaintContext, Point, Rect, Shared, Signal, Size,
    SizeHint, TextRole, TextStyle, UiEvent, Widget, WidgetBase,
};
use rustui_model::{SelectionChange, SelectionMode, SelectionModel, TableListModel, TableModel};

use crate::common::{center_in, control_radius, focus_ring};
use crate::view::{item_at, item_top, paint_thumb, visible_range, Scroll};

/// Narrowest a column may become through a resize drag, in logical pixels.
const MIN_COLUMN_WIDTH: f32 = 24.0;
/// Width given to a column whose header has not been measured yet.
const DEFAULT_COLUMN_WIDTH: f32 = 80.0;
/// Half width of the grab area around a header column boundary.
const RESIZE_GRAB: f32 = 4.0;
/// Width assumed for a table measured without a finite width hint.
const PREFERRED_WIDTH: f32 = 320.0;

/// A fixed height table of rows split into resizable columns.
///
/// `TableView` paints a column header above a *virtualized* row area: only the rows intersecting
/// the viewport reach the painter, so a model with a hundred thousand rows costs the same widget
/// tree as a ten row one. Clicking a row selects it, dragging a header boundary resizes the column
/// to its left, the wheel scrolls vertically and shift-style horizontal deltas pan the columns.
///
/// Column widths are owned by the widget (the model only supplies text). They are synchronised
/// with `TableModel::column_count` at measure, event and paint time, so adding or removing model
/// columns never leaves a stale width behind; widths that survive a sync keep their value and new
/// columns default to `max(header width + horizontal padding, 80)`.
///
/// Keyboard: `↑`/`↓`, `Home`/`End` and `PageUp`/`PageDown` move the selection (keeping the current
/// row visible), `Return` activates it.
pub struct TableView {
    base: WidgetBase,
    model: Shared<Box<dyn TableModel>>,
    selection: SelectionModel,
    /// Vertical scroll state over the rows; the viewport excludes the header.
    scroll: Scroll,
    /// Horizontal scroll state over the column widths.
    h_scroll: Scroll,
    /// Per column widths in logical pixels, index aligned with the model.
    columns: Vec<f32>,
    row_height: f32,
    header_height: f32,
    /// Column whose boundary is currently being dragged.
    resize_column: Option<usize>,
    /// Row under the pointer, if any.
    hover_row: Option<usize>,
    /// Fires with the row index when the user activates a row.
    activated: Signal<usize>,
}

impl TableView {
    /// A table over an empty [`TableListModel`]; bind a real model with [`TableView::with_model`].
    pub fn new() -> Self {
        Self::with_model(Shared::new(Box::new(TableListModel::new())))
    }

    /// A table over `model`. Keep a clone of the handle to mutate the model later.
    pub fn with_model(model: Shared<Box<dyn TableModel>>) -> Self {
        Self {
            base: WidgetBase::new(),
            model,
            selection: SelectionModel::new(SelectionMode::Single),
            scroll: Scroll::default(),
            h_scroll: Scroll::default(),
            columns: Vec::new(),
            row_height: 24.0,
            header_height: 24.0,
            resize_column: None,
            hover_row: None,
            activated: Signal::new(),
        }
    }

    /// The shared model handle (clone it to mutate the model).
    pub fn model(&self) -> Shared<Box<dyn TableModel>> {
        self.model.clone()
    }

    /// Row count of the model.
    pub fn row_count(&self) -> usize {
        self.model.with(|model| model.row_count())
    }

    /// Column count of the model.
    pub fn column_count(&self) -> usize {
        self.model.with(|model| model.column_count())
    }

    /// Current column widths, index aligned with the model columns.
    ///
    /// Empty until the widget synchronises with its model (first measure, event or paint).
    pub fn columns(&self) -> &[f32] {
        &self.columns
    }

    /// Width of `column`, or `0.0` when out of range.
    pub fn column_width(&self, column: usize) -> f32 {
        self.columns.get(column).copied().unwrap_or(0.0)
    }

    /// Sets the width of `column`, clamped to at least [`MIN_COLUMN_WIDTH`].
    ///
    /// Columns at or beyond the model's column count are ignored: the width would be dropped by
    /// the next synchronisation anyway.
    pub fn set_column_width(&mut self, column: usize, width: f32) {
        if column >= self.column_count() {
            return;
        }
        if self.columns.len() <= column {
            self.columns.resize(column + 1, DEFAULT_COLUMN_WIDTH);
        }
        self.columns[column] = width.max(MIN_COLUMN_WIDTH);
    }

    /// The view's selection model (inspect it before the widget is boxed).
    pub fn selection_model(&self) -> &SelectionModel {
        &self.selection
    }

    /// Shared "current row" property: the cheapest way to observe the selection from outside.
    pub fn current_property(&self) -> rustui_core::SharedProperty<Option<usize>> {
        self.selection.current_property().clone()
    }

    /// Fires whenever the selection changes.
    pub fn selection_changed(&self) -> Signal<SelectionChange> {
        self.selection.selection_changed().clone()
    }

    /// Fires with the row index when the user activates a row (double click or `Return`).
    pub fn activated(&self) -> Signal<usize> {
        self.activated.clone()
    }

    /// Programmatic selection (ignored when `row` is out of range).
    pub fn select(&mut self, row: usize) {
        if row < self.row_count() {
            self.selection.select(row);
        }
    }

    /// Currently selected row, if any.
    pub fn selected(&self) -> Option<usize> {
        self.selection.current()
    }

    /// Scrolls the minimum amount that brings `row` into view below the header.
    pub fn scroll_to_row(&mut self, row: usize) {
        let rows = self.row_count();
        if rows == 0 || self.row_height <= 0.0 {
            return;
        }
        let row = row.min(rows - 1);
        self.scroll
            .ensure_visible(item_top(self.row_height, row), self.row_height);
    }

    /// Current horizontal scroll offset in logical pixels.
    pub fn h_scroll_offset(&self) -> f32 {
        self.h_scroll.offset
    }

    /// Left edge of `column` in content coordinates (before the horizontal scroll offset).
    fn column_x(&self, column: usize) -> f32 {
        self.columns
            .iter()
            .take(column)
            .fold(0.0, |acc, &width| acc + width)
    }

    /// Window x of the boundary after `column`.
    fn boundary_window_x(&self, column: usize, origin_x: f32) -> f32 {
        origin_x + self.column_x(column + 1) - self.h_scroll.offset
    }

    /// Column whose right boundary is within [`RESIZE_GRAB`] of `window_x`.
    fn column_boundary_at(&self, window_x: f32, origin_x: f32) -> Option<usize> {
        (0..self.columns.len()).find(|&column| {
            (self.boundary_window_x(column, origin_x) - window_x).abs() <= RESIZE_GRAB
        })
    }

    /// Drag: move the boundary after `column` to the local x coordinate `local_x`.
    fn resize_to(&mut self, column: usize, local_x: f32) {
        let width = local_x + self.h_scroll.offset - self.column_x(column);
        self.set_column_width(column, width);
    }

    /// Rebuilds `columns` when the model's column count changed; surviving columns keep their
    /// width, new ones start from the measured header width.
    fn sync_columns(&mut self, ui: &rustui_core::UiContext) {
        let count = self.column_count();
        if self.columns.len() == count {
            return;
        }
        self.columns.resize(count, DEFAULT_COLUMN_WIDTH);
        let theme = ui.theme();
        let pad = theme.content_padding();
        let style = theme.text(TextRole::Button, None);
        for column in 0..count {
            if (self.columns[column] - DEFAULT_COLUMN_WIDTH).abs() > f32::EPSILON {
                continue;
            }
            let header = self.model.with(|model| model.header_data(column).as_text());
            let text_width = ui.text().measure_text(&header, &style, None).width;
            self.columns[column] = (text_width + pad.horizontal()).max(DEFAULT_COLUMN_WIDTH);
        }
        self.h_scroll.content = self.columns.iter().sum();
    }

    /// Keeps the scroll geometry in step with the current bounds (arrange may have resized us).
    fn sync_scroll(&mut self) {
        let rows = self.row_count();
        self.scroll.content = rows as f32 * self.row_height;
        self.scroll.viewport = (self.base.bounds().height - self.header_height).max(0.0);
        self.h_scroll.content = self.columns.iter().sum();
        self.h_scroll.viewport = self.base.bounds().width;
        self.scroll.clamp();
        self.h_scroll.clamp();
    }

    /// Row under a **window** position, if it is in the row area (below the header).
    fn row_at(&self, position: Point, bounds: Rect) -> Option<usize> {
        if !bounds.contains(position) {
            return None;
        }
        let local_y = position.y - bounds.y - self.header_height;
        if local_y < 0.0 {
            return None;
        }
        item_at(self.row_height, &self.scroll, self.row_count(), local_y)
    }
}

impl Default for TableView {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for TableView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableView")
            .field("base", &self.base)
            .field("rows", &self.row_count())
            .field("columns", &self.columns)
            .field("selected", &self.selected())
            .finish_non_exhaustive()
    }
}

impl_widget_base!(TableView);

impl Widget for TableView {
    fn kind(&self) -> &'static str {
        "TableView"
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn is_opaque(&self) -> bool {
        true
    }

    /// A view fills whatever space it is offered on both axes.
    fn size_hint(&self) -> SizeHint {
        SizeHint::fill()
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        self.sync_columns(ctx.ui());
        let theme = ctx.ui().theme();
        self.row_height = theme.control_height();
        self.header_height = theme.control_height();
        let rows = self.row_count();
        let content = rows as f32 * self.row_height;
        let size = Size::new(
            if available.width.is_finite() {
                available.width.max(0.0)
            } else {
                PREFERRED_WIDTH.max(self.columns.iter().sum::<f32>())
            },
            if available.height.is_finite() {
                available.height.max(0.0)
            } else {
                content + self.header_height
            },
        );
        self.scroll.content = content;
        self.scroll.viewport = (size.height - self.header_height).max(0.0);
        self.h_scroll.content = self.columns.iter().sum();
        self.h_scroll.viewport = size.width;
        self.scroll.clamp();
        self.h_scroll.clamp();
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
        let theme = ctx.ui().theme();
        self.row_height = theme.control_height();
        self.header_height = theme.control_height();
        self.sync_scroll();
    }

    fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult {
        let id = self.id();
        let enabled = self.base.is_enabled();
        let bounds = self.base.window_bounds();
        self.sync_columns(ctx.ui());
        let theme = ctx.ui().theme();
        self.row_height = theme.control_height();
        self.header_height = theme.control_height();
        self.sync_scroll();

        match event {
            UiEvent::MouseMove { position } if enabled => {
                if self.resize_column.is_some() && ctx.interaction().owns_pointer(id) {
                    self.resize_to(
                        self.resize_column.expect("resize column checked above"),
                        position.x - bounds.x,
                    );
                    ctx.request_redraw();
                    ctx.accept();
                    return EventResult::Handled;
                }
                let row = self.row_at(*position, bounds);
                if row != self.hover_row {
                    self.hover_row = row;
                    ctx.request_redraw();
                }
                EventResult::Unhandled
            }
            UiEvent::MouseLeave => {
                if self.hover_row.take().is_some() {
                    ctx.request_redraw();
                }
                EventResult::Unhandled
            }
            UiEvent::MouseDown {
                button: MouseButton::Left,
                position,
            } if enabled && bounds.contains(*position) => {
                if position.y - bounds.y < self.header_height {
                    // Header: only a column boundary starts a resize, and a header click never
                    // changes the row selection.
                    if let Some(column) = self.column_boundary_at(position.x, bounds.x) {
                        self.resize_column = Some(column);
                        ctx.capture_pointer(id);
                        ctx.request_redraw();
                        ctx.accept();
                        return EventResult::Handled;
                    }
                    ctx.accept();
                    return EventResult::Handled;
                }
                let row = self.row_at(*position, bounds);
                if let Some(row) = row {
                    self.selection.select(row);
                    ctx.capture_pointer(id);
                    ctx.request_redraw();
                    ctx.accept();
                    return EventResult::Handled;
                }
                EventResult::Unhandled
            }
            UiEvent::MouseDoubleClick {
                button: MouseButton::Left,
                position,
            } if enabled && bounds.contains(*position) => {
                if position.y - bounds.y < self.header_height {
                    ctx.accept();
                    return EventResult::Handled;
                }
                if let Some(row) = self.row_at(*position, bounds) {
                    self.selection.select(row);
                    self.activated.emit(&row);
                    ctx.request_redraw();
                    ctx.accept();
                    return EventResult::Handled;
                }
                EventResult::Unhandled
            }
            UiEvent::MouseUp { .. } if ctx.interaction().owns_pointer(id) => {
                self.resize_column = None;
                ctx.release_pointer();
                ctx.request_redraw();
                EventResult::Handled
            }
            UiEvent::MouseWheel { delta, .. } => {
                let offset = scroll_delta_to_offset(*delta);
                if offset.x == 0.0 && offset.y == 0.0 {
                    return EventResult::Unhandled;
                }
                self.scroll.scroll_by(offset.y);
                self.h_scroll.scroll_by(offset.x);
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::KeyDown { key, .. } if enabled => {
                let rows = self.row_count();
                if rows == 0 {
                    return EventResult::Unhandled;
                }
                let current = self.selection.current();
                let step = (self.scroll.page() / self.row_height.max(1.0)) as usize + 1;
                let target = if key.is_named(NamedKey::ArrowDown) {
                    Some(current.map_or(0, |row| (row + 1).min(rows - 1)))
                } else if key.is_named(NamedKey::ArrowUp) {
                    Some(current.map_or(0, |row| row.saturating_sub(1)))
                } else if key.is_named(NamedKey::Home) {
                    Some(0)
                } else if key.is_named(NamedKey::End) {
                    Some(rows - 1)
                } else if key.is_named(NamedKey::PageDown) {
                    Some(current.map_or(0, |row| (row + step).min(rows - 1)))
                } else if key.is_named(NamedKey::PageUp) {
                    Some(current.map_or(0, |row| row.saturating_sub(step)))
                } else if key.is_named(NamedKey::Return) {
                    if let Some(row) = current {
                        self.activated.emit(&row);
                    }
                    ctx.accept();
                    return EventResult::Handled;
                } else {
                    None
                };
                let Some(target) = target else {
                    return EventResult::Unhandled;
                };
                self.selection.select(target);
                self.scroll_to_row(target);
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            _ => EventResult::Unhandled,
        }
    }

    fn paint(&mut self, ctx: &mut PaintContext<'_>) {
        let id = self.id();
        let state = ctx.state_of(id, self.base.state());
        self.sync_columns(ctx.ui());
        let theme = ctx.ui().theme();
        self.row_height = theme.control_height();
        self.header_height = theme.control_height();
        let pad = theme.content_padding();
        let rule = theme.borders.thin.max(1.0);
        self.sync_scroll();

        // Window coordinates for the raw painter, local coordinates for the ctx helpers.
        let bounds = ctx.to_window_rect(self.base.bounds());
        let radius = control_radius(ctx);
        ctx.painter().styled_rect(
            bounds,
            radius,
            Fill::rounded(theme.colors.surface, theme.radius.sm),
        );

        ctx.push_clip_rounded(self.base.bounds(), theme.radius.sm);

        // Header background plus the rule that separates it from the rows.
        ctx.fill_rect(
            Rect::new(0.0, 0.0, bounds.width, self.header_height),
            theme.colors.surface_alt,
        );
        ctx.fill_rect(
            Rect::new(0.0, self.header_height - rule, bounds.width, rule),
            theme.colors.border,
        );

        let header_style = theme.text(TextRole::Button, None);
        let header_line = ctx.line_height(&header_style);
        for column in 0..self.columns.len() {
            let width = self.columns[column];
            let x = pad.left + self.column_x(column) - self.h_scroll.offset;
            if x >= bounds.width || x + width <= 0.0 {
                continue;
            }
            let max_width = (width - pad.horizontal()).max(1.0);
            let header = self.model.with(|model| model.header_data(column).as_text());
            let header = fit_text(ctx, &header, &header_style, max_width);
            let position = center_in(
                Rect::new(x, 0.0, max_width, self.header_height),
                Size::new(max_width, header_line),
            );
            let position = ctx.to_window(position);
            ctx.painter().text(position, &header, &header_style);
        }

        // Visible rows: only the ones intersecting the viewport are painted (§51).
        let body_style = theme.text(
            TextRole::Body,
            Some(if state.enabled {
                theme.colors.text
            } else {
                theme.colors.text_disabled
            }),
        );
        let body_line = ctx.line_height(&body_style);
        let pointer = ctx.interaction().pointer;
        let hovered = if self.hover_row.is_some() {
            self.hover_row
        } else if state.hovered && bounds.contains(pointer) {
            self.row_at(pointer, bounds)
        } else {
            None
        };
        for row in visible_range(self.row_height, &self.scroll, self.row_count()) {
            let top = self.header_height + item_top(self.row_height, row) - self.scroll.offset;
            let row_rect = Rect::new(bounds.x, bounds.y + top, bounds.width, self.row_height);
            if self.selection.is_selected(row) && state.enabled {
                ctx.painter().rect(row_rect, theme.colors.selection);
            } else if Some(row) == hovered {
                ctx.painter().rect(row_rect, theme.colors.surface_hover);
            }
            for column in 0..self.columns.len() {
                let width = self.columns[column];
                let x = pad.left + self.column_x(column) - self.h_scroll.offset;
                if x >= bounds.width || x + width <= 0.0 {
                    continue;
                }
                let text = self.model.with(|model| model.data(row, column).as_text());
                if text.is_empty() {
                    continue;
                }
                let max_width = (width - pad.horizontal()).max(1.0);
                let text = fit_text(ctx, &text, &body_style, max_width);
                let position = center_in(
                    Rect::new(x, top, max_width, self.row_height),
                    Size::new(max_width, body_line),
                );
                let position = ctx.to_window(position);
                ctx.painter().text(position, &text, &body_style);
            }
        }

        ctx.pop_clip();

        // Scroll thumbs: rows over the row area, columns along the bottom edge.
        paint_thumb(
            ctx,
            Rect::new(
                self.base.bounds().x,
                self.base.bounds().y + self.header_height,
                self.base.bounds().width,
                self.scroll.viewport,
            ),
            &self.scroll,
        );
        paint_h_thumb(ctx, self.base.bounds(), &self.h_scroll);

        if state.focused {
            focus_ring(ctx, self.base.bounds(), theme.radius.sm);
        }
    }
}

/// Truncates `text` with an ellipsis so that it fits `max_width` without ever wrapping.
///
/// `pub(crate)` so that the sibling item views (`TreeView`) share one implementation.
pub(crate) fn fit_text<'a>(
    ctx: &PaintContext<'_>,
    text: &'a str,
    style: &TextStyle,
    max_width: f32,
) -> Cow<'a, str> {
    if ctx.measure_text(text, style, None).width <= max_width {
        return Cow::Borrowed(text);
    }
    const ELLIPSIS: &str = "…";
    let budget = (max_width - ctx.measure_text(ELLIPSIS, style, None).width).max(0.0);
    // Single line labels never wrap, so a linear scan over the prefixes is enough.
    let mut best = "";
    for (index, _) in text.char_indices() {
        let prefix = &text[..index];
        if ctx.measure_text(prefix, style, None).width > budget {
            break;
        }
        best = prefix;
    }
    let mut fitted = String::with_capacity(best.len() + ELLIPSIS.len());
    fitted.push_str(best);
    fitted.push_str(ELLIPSIS);
    Cow::Owned(fitted)
}

/// Horizontal scrollbar thumb along the bottom edge of `bounds` (local coordinates).
fn paint_h_thumb(ctx: &mut PaintContext<'_>, bounds: Rect, scroll: &Scroll) {
    if !scroll.overflows() || scroll.content <= 0.0 || bounds.width <= 0.0 {
        return;
    }
    let theme = ctx.ui().theme();
    let height = theme.spacing.sm * 0.5;
    let ratio = (scroll.viewport / scroll.content).clamp(0.1, 1.0);
    let thumb_width = (bounds.width * ratio).max(theme.spacing.sm);
    let travel = (bounds.width - thumb_width) * scroll.offset / scroll.content.max(1.0);
    let thumb = Rect::new(
        bounds.x + travel,
        bounds.y + bounds.height - height,
        thumb_width,
        height,
    );
    ctx.fill_rounded(thumb, height * 0.5, theme.colors.border);
}
