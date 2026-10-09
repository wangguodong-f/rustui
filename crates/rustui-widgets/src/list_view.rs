//! Virtualized list of rows (design document §26, §51).

use std::fmt;

use rustui_core::{
    impl_widget_base, scroll_delta_to_offset, ArrangeContext, EventContext, EventResult,
    MeasureContext, MouseButton, NamedKey, PaintContext, Rect, Shared, Signal, Size, SizeHint,
    TextRole, UiEvent, Widget, WidgetBase,
};
use rustui_model::{ListModel, SelectionChange, SelectionModel, StringListModel};

use crate::common::{center_in, control_radius, focus_ring};
use crate::view::{item_at, item_top, paint_thumb, visible_range, Scroll};

/// A scrollable, selectable list of rows.
///
/// The list is *virtualized*: it measures the model in O(1), asks the model for the rows inside
/// the viewport only and paints those, so a million row list costs the same widget tree as a ten
/// row one (§51).
///
/// ```ignore
/// let list = ListView::new(["carrot", "apple", "banana"]);
/// list.selection_changed().connect(|change| println!("{change:?}"));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivateMode {
    /// Double click activates the row.
    DoubleClick,
    /// Double click *and* `Enter`.
    DoubleClickAndEnter,
}

pub struct ListView {
    base: WidgetBase,
    model: Shared<Box<dyn ListModel>>,
    selection: SelectionModel,
    scroll: Scroll,
    item_height: f32,
    activate: ActivateMode,
    activated: Signal<usize>,
    hover_row: Option<usize>,
}

impl ListView {
    /// A list over a fresh [`StringListModel`].
    pub fn new(values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let model = StringListModel::from_values(values);
        Self::with_model(Shared::new(Box::new(model) as Box<dyn ListModel>))
    }

    /// A list over `model`. Keep a clone of the handle to mutate the model later.
    pub fn with_model(model: Shared<Box<dyn ListModel>>) -> Self {
        Self {
            base: WidgetBase::new(),
            model,
            selection: SelectionModel::new(rustui_model::SelectionMode::Single),
            scroll: Scroll::default(),
            item_height: 24.0,
            activate: ActivateMode::DoubleClickAndEnter,
            activated: Signal::new(),
            hover_row: None,
        }
    }

    /// The shared model handle (clone it to mutate the model).
    pub fn model(&self) -> Shared<Box<dyn ListModel>> {
        self.model.clone()
    }

    /// Row count of the model.
    pub fn row_count(&self) -> usize {
        self.model.with(|model| model.row_count())
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

    /// Fires with the row index when the user activates a row.
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

    /// How a row is activated.
    pub fn activate_mode(mut self, mode: ActivateMode) -> Self {
        self.activate = mode;
        self
    }

    /// Scrolls the minimum amount that brings `row` into view.
    pub fn scroll_to_row(&mut self, row: usize) {
        let top = item_top(self.item_height, row);
        self.scroll.ensure_visible(top, self.item_height);
    }

    fn rows(&self) -> usize {
        self.row_count()
    }

    fn clamp_scroll(&mut self) {
        self.scroll.clamp();
    }
}

impl fmt::Debug for ListView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListView")
            .field("base", &self.base)
            .field("rows", &self.row_count())
            .field("selected", &self.selected())
            .finish_non_exhaustive()
    }
}

impl_widget_base!(ListView);

impl Widget for ListView {
    fn kind(&self) -> &'static str {
        "ListView"
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
        let theme = ctx.ui().theme();
        self.item_height = theme.control_height();
        let rows = self.rows();
        let content = rows as f32 * self.item_height;
        let viewport = Size::new(
            if available.width.is_finite() {
                available.width.max(0.0)
            } else {
                160.0
            },
            if available.height.is_finite() {
                available.height.max(0.0)
            } else {
                content
            },
        );
        self.scroll.content = content;
        self.scroll.viewport = viewport.height;
        self.clamp_scroll();
        self.base.set_measured(viewport);
        viewport
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
        self.item_height = ctx.ui().theme().control_height();
        self.scroll.viewport = rect.height;
        self.scroll.content = self.rows() as f32 * self.item_height;
        self.clamp_scroll();
    }

    fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult {
        let id = self.id();
        let enabled = self.base.is_enabled();
        let bounds = self.base.window_bounds();
        match event {
            UiEvent::MouseMove { position } if enabled => {
                let row = item_at(
                    self.item_height,
                    &self.scroll,
                    self.rows(),
                    position.y - bounds.y,
                );
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
            } if enabled => {
                let row = item_at(
                    self.item_height,
                    &self.scroll,
                    self.rows(),
                    position.y - bounds.y,
                );
                if let Some(row) = row {
                    self.selection.select(row);
                }
                ctx.capture_pointer(id);
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::MouseDoubleClick {
                button: MouseButton::Left,
                position,
            } if enabled => {
                let row = item_at(
                    self.item_height,
                    &self.scroll,
                    self.rows(),
                    position.y - bounds.y,
                );
                if let Some(row) = row {
                    self.activated.emit(&row);
                    ctx.accept();
                }
                EventResult::Handled
            }
            UiEvent::MouseUp { .. } if ctx.interaction().owns_pointer(id) => {
                ctx.release_pointer();
                EventResult::Handled
            }
            UiEvent::MouseWheel { delta, .. } => {
                self.scroll.scroll_by(scroll_delta_to_offset(*delta).y);
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::KeyDown { key, .. } if enabled => {
                let rows = self.rows();
                if rows == 0 {
                    return EventResult::Unhandled;
                }
                let current = self.selection.current();
                let target = if key.is_named(NamedKey::ArrowDown) {
                    Some(current.map_or(0, |row| (row + 1).min(rows - 1)))
                } else if key.is_named(NamedKey::ArrowUp) {
                    Some(current.map_or(0, |row| row.saturating_sub(1)))
                } else if key.is_named(NamedKey::Home) {
                    Some(0)
                } else if key.is_named(NamedKey::End) {
                    Some(rows - 1)
                } else if key.is_named(NamedKey::PageDown) {
                    let step = (self.scroll.page() / self.item_height) as usize + 1;
                    Some(current.map_or(0, |row| (row + step).min(rows - 1)))
                } else if key.is_named(NamedKey::PageUp) {
                    let step = (self.scroll.page() / self.item_height) as usize + 1;
                    Some(current.map_or(0, |row| row.saturating_sub(step)))
                } else if key.is_named(NamedKey::Return)
                    && self.activate == ActivateMode::DoubleClickAndEnter
                {
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
        let theme = ctx.ui().theme();
        // Window coordinates: the painter takes no offset.
        let bounds = ctx.to_window_rect(self.base.bounds());
        let radius = control_radius(ctx);
        ctx.painter().styled_rect(
            bounds,
            radius,
            rustui_core::Fill::rounded(theme.colors.surface, theme.radius.sm),
        );

        ctx.push_clip_rounded(self.base.bounds(), theme.radius.sm);
        let rows = self.rows();
        let style = theme.text(
            TextRole::Body,
            Some(if state.enabled {
                theme.colors.text
            } else {
                theme.colors.text_disabled
            }),
        );
        let line_height = ctx.line_height(&style);
        let pad = theme.content_padding();
        let pointer = ctx.interaction().pointer;
        let hovered = if self.hover_row.is_some() {
            self.hover_row
        } else if state.hovered && bounds.contains(pointer) {
            item_at(self.item_height, &self.scroll, rows, pointer.y - bounds.y)
        } else {
            None
        };

        for row in visible_range(self.item_height, &self.scroll, rows) {
            let top = item_top(self.item_height, row) - self.scroll.offset;
            let row_rect = Rect::new(bounds.x, bounds.y + top, bounds.width, self.item_height);
            let selected = self.selection.is_selected(row);
            if selected && state.enabled {
                ctx.painter().rect(row_rect, theme.colors.selection);
            } else if Some(row) == hovered {
                ctx.painter().rect(row_rect, theme.colors.surface_hover);
            }
            let text = self.model.with(|model| model.data(row).as_text());
            if !text.is_empty() {
                let text_size = ctx.measure_text(
                    &text,
                    &style,
                    Some((row_rect.width - pad.horizontal()).max(0.0)),
                );
                let position = center_in(
                    Rect::new(
                        row_rect.x + pad.left,
                        row_rect.y,
                        row_rect.width - pad.horizontal(),
                        row_rect.height,
                    ),
                    Size::new(text_size.width, line_height),
                );
                ctx.painter().text(position, &text, &style);
            }
        }
        ctx.pop_clip();

        paint_thumb(ctx, self.base.bounds(), &self.scroll);
        if state.focused {
            focus_ring(ctx, self.base.bounds(), theme.radius.sm);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_range_only_covers_the_viewport() {
        let scroll = Scroll {
            offset: 100.0,
            content: 1000.0,
            viewport: 50.0,
        };
        assert_eq!(visible_range(20.0, &scroll, 50), 5..8);
        assert_eq!(visible_range(20.0, &Scroll::default(), 50), 0..0);
    }

    #[test]
    fn item_at_maps_a_local_y_to_a_row() {
        let scroll = Scroll {
            offset: 30.0,
            content: 400.0,
            viewport: 100.0,
        };
        assert_eq!(item_at(20.0, &scroll, 20, 5.0), Some(1));
        assert_eq!(item_at(20.0, &scroll, 20, -40.0), None);
        assert_eq!(item_at(20.0, &scroll, 20, 500.0), None);
    }

    #[test]
    fn scroll_clamps_and_reveals_items() {
        let mut scroll = Scroll {
            offset: 0.0,
            content: 500.0,
            viewport: 100.0,
        };
        scroll.scroll_by(-9999.0);
        assert_eq!(scroll.offset, 0.0);
        scroll.scroll_by(9999.0);
        assert_eq!(scroll.offset, 400.0);
        scroll.ensure_visible(200.0, 20.0);
        assert_eq!(
            scroll.offset, 200.0,
            "the item is above: bring its top into view"
        );
    }

    #[test]
    fn a_list_view_counts_its_model() {
        let list = ListView::new(["a", "b", "c"]);
        assert_eq!(list.row_count(), 3);
        assert_eq!(list.selected(), None);
        let mut list = list;
        list.select(1);
        assert_eq!(list.selected(), Some(1));
        list.select(99);
        assert_eq!(list.selected(), Some(1), "out of range is ignored");
    }
}
