//! Hierarchical list with disclosure twisties and virtualized rows (design document §30).

use std::fmt;

use rustui_core::{
    impl_widget_base, scroll_delta_to_offset, ArrangeContext, EventContext, EventResult, Fill,
    MeasureContext, MouseButton, NamedKey, PaintContext, Point, Rect, Shared, SharedProperty,
    Signal, Size, SizeHint, TextRole, UiEvent, Widget, WidgetBase,
};
use rustui_model::{NodeId, TreeListModel, TreeModel};

use crate::common::{center_in, control_radius, focus_ring};
use crate::table_view::fit_text;
use crate::view::{item_at, item_top, paint_thumb, visible_range, Scroll};

/// Width of the twisty (disclosure triangle) hit area, in logical pixels.
const TWISTY_WIDTH: f32 = 16.0;
/// Extra indent per nesting level on top of the theme spacing.
const INDENT_GAP: f32 = 8.0;
/// Consecutive unknown ids tolerated while discovering the roots of a model.
///
/// See [`roots_of`]: the trait only reports *how many* roots exist, so their ids are found by
/// probing the id space. Arena models assign ids densely and never reuse them, so gaps only
/// appear for deleted nodes; a run this long means the remaining roots cannot be found.
const MAX_EMPTY_PROBES: u32 = 4096;
/// Width assumed for a tree measured without a finite width hint.
const PREFERRED_WIDTH: f32 = 280.0;

/// A collapsible, virtualized tree of nodes addressed by [`NodeId`].
///
/// `TreeView` flattens the model's visible nodes (depth first, descending into expanded nodes
/// only) into the row list it paints, so only the rows inside the viewport reach the painter.
/// The twisty left of a branch label expands or collapses it; clicking a row selects the node,
/// double clicking a leaf activates it, and the arrow keys navigate the way a file tree does:
/// `→` expands or steps into the first child, `←` collapses or steps up to the parent.
///
/// Selection is published as [`TreeView::selection_property`] (a `SharedProperty<Option<NodeId>>`)
/// and [`TreeView::selection_changed`] (a `Signal<Option<NodeId>>`), because node ids — not row
/// indices — are the stable handle an application binds to.
///
/// Note on root discovery: [`TreeModel`] only reports `root_count()`, not the root ids themselves,
/// so the view probes the model's id space (see [`roots_of`]). Models with dense, never-reused
/// ids — such as `TreeListModel` — are enumerated exactly; exotic id layouts may report fewer
/// roots than exist.
pub struct TreeView {
    base: WidgetBase,
    model: Shared<Box<dyn TreeModel>>,
    /// Flattened `(node, depth)` rows currently visible (expanded ancestors only).
    rows: Vec<(NodeId, usize)>,
    row_height: f32,
    scroll: Scroll,
    selected: Option<NodeId>,
    selection: SharedProperty<Option<NodeId>>,
    selection_changed: Signal<Option<NodeId>>,
    /// Fires with the node when the user activates it (double click on a leaf, or `Return`).
    activated: Signal<NodeId>,
    /// Row under the pointer, if any.
    hover_row: Option<usize>,
}

impl TreeView {
    /// A tree over an empty [`TreeListModel`]; bind a real model with [`TreeView::with_model`].
    pub fn new() -> Self {
        Self::with_model(Shared::new(Box::new(TreeListModel::new())))
    }

    /// A tree over `model`. Keep a clone of the handle to mutate the model later.
    pub fn with_model(model: Shared<Box<dyn TreeModel>>) -> Self {
        Self {
            base: WidgetBase::new(),
            model,
            rows: Vec::new(),
            row_height: 24.0,
            scroll: Scroll::default(),
            selected: None,
            selection: SharedProperty::new(None),
            selection_changed: Signal::new(),
            activated: Signal::new(),
            hover_row: None,
        }
    }

    /// The shared model handle (clone it to mutate the model).
    pub fn model(&self) -> Shared<Box<dyn TreeModel>> {
        self.model.clone()
    }

    /// Number of top level nodes in the model.
    pub fn root_count(&self) -> usize {
        self.model.with(|model| model.root_count())
    }

    /// Number of nodes reachable from the roots (regardless of expansion).
    pub fn node_count(&self) -> usize {
        self.model.with(|model| count_nodes(&**model))
    }

    /// Number of rows currently visible (the length of [`TreeView::visible_rows`]).
    pub fn visible_row_count(&self) -> usize {
        self.rows.len()
    }

    /// The flattened `(node, depth)` rows, in paint order.
    ///
    /// Recomputed at measure, event and paint time; keep a clone if it must outlive a frame.
    pub fn visible_rows(&self) -> &[(NodeId, usize)] {
        &self.rows
    }

    /// Currently selected node, if any.
    pub fn selected(&self) -> Option<NodeId> {
        self.selected
    }

    /// Selects `node`; an unknown id clears the selection instead.
    pub fn select(&mut self, node: NodeId) {
        let live = self.model.with(|model| is_live(&**model, &node));
        self.set_selected(if live { Some(node) } else { None });
    }

    /// Shared selection property: bind cursors, inspectors or other views to it.
    pub fn selection_property(&self) -> SharedProperty<Option<NodeId>> {
        self.selection.clone()
    }

    /// Fires with the new selection (`None` when the selection was cleared).
    pub fn selection_changed(&self) -> Signal<Option<NodeId>> {
        self.selection_changed.clone()
    }

    /// Fires with the node when the user activates it.
    pub fn activated(&self) -> Signal<NodeId> {
        self.activated.clone()
    }

    /// Scrolls the minimum amount that brings `node` into view.
    pub fn scroll_to_node(&mut self, node: NodeId) {
        self.refresh_rows();
        if let Some(index) = self.index_of(node) {
            if self.row_height > 0.0 {
                self.scroll
                    .ensure_visible(item_top(self.row_height, index), self.row_height);
            }
        }
    }

    /// Re-flattens the model into `rows` (call after expansion or selection changes).
    fn refresh_rows(&mut self) {
        self.rows = self.model.with(|model| flatten(&**model));
    }

    /// Row index of `node` in the current flattening.
    fn index_of(&self, node: NodeId) -> Option<usize> {
        self.rows.iter().position(|(id, _)| *id == node)
    }

    /// Sets the selection, publishing it through the property and the signal.
    fn set_selected(&mut self, node: Option<NodeId>) {
        if self.selected == node {
            return;
        }
        self.selected = node;
        self.selection.set(node);
        self.selection_changed.emit(&node);
    }

    /// Expands a collapsed branch, collapses an expanded one.
    fn toggle(&mut self, node: NodeId) {
        let expanded = self.model.with(|model| model.is_expanded(&node));
        self.model
            .with_mut(|model| model.set_expanded(&node, !expanded));
        self.refresh_rows();
    }

    /// Row index of the current selection, if it is visible.
    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected?;
        self.index_of(selected)
    }

    /// Scrolls so that `index` is inside the viewport.
    fn ensure_visible(&mut self, index: usize) {
        if self.row_height > 0.0 {
            self.scroll
                .ensure_visible(item_top(self.row_height, index), self.row_height);
        }
    }

    /// Selects `node` (when it exists) and reveals its row.
    fn select_index(&mut self, node: NodeId) {
        self.set_selected(Some(node));
        if let Some(index) = self.index_of(node) {
            self.ensure_visible(index);
        }
    }
}

impl Default for TreeView {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for TreeView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeView")
            .field("base", &self.base)
            .field("roots", &self.root_count())
            .field("rows", &self.rows.len())
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl_widget_base!(TreeView);

impl Widget for TreeView {
    fn kind(&self) -> &'static str {
        "TreeView"
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
        self.refresh_rows();
        self.row_height = ctx.ui().theme().control_height();
        self.scroll.content = self.rows.len() as f32 * self.row_height;
        let size = Size::new(
            if available.width.is_finite() {
                available.width.max(0.0)
            } else {
                PREFERRED_WIDTH
            },
            if available.height.is_finite() {
                available.height.max(0.0)
            } else {
                self.scroll.content
            },
        );
        self.scroll.viewport = size.height;
        self.scroll.clamp();
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
        self.refresh_rows();
        self.row_height = ctx.ui().theme().control_height();
        self.scroll.content = self.rows.len() as f32 * self.row_height;
        self.scroll.viewport = rect.height;
        self.scroll.clamp();
    }

    fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult {
        let id = self.id();
        let enabled = self.base.is_enabled();
        let bounds = self.base.window_bounds();
        self.refresh_rows();
        let theme = ctx.ui().theme();
        self.row_height = theme.control_height();
        self.scroll.content = self.rows.len() as f32 * self.row_height;
        self.scroll.viewport = bounds.height;
        self.scroll.clamp();
        let pad = theme.content_padding();
        let indent_step = theme.spacing.md + INDENT_GAP;

        match event {
            UiEvent::MouseMove { position } if enabled => {
                let row = if bounds.contains(*position) {
                    item_at(
                        self.row_height,
                        &self.scroll,
                        self.rows.len(),
                        position.y - bounds.y,
                    )
                } else {
                    None
                };
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
                let local_y = position.y - bounds.y;
                let Some(index) = item_at(self.row_height, &self.scroll, self.rows.len(), local_y)
                else {
                    return EventResult::Unhandled;
                };
                let (node, depth) = self.rows[index];
                let indent = depth as f32 * indent_step;
                let twisty_left = pad.left + indent;
                let local_x = position.x - bounds.x;
                let over_twisty = (twisty_left..twisty_left + TWISTY_WIDTH).contains(&local_x);
                if over_twisty && self.model.with(|m| m.child_count(&node)) > 0 {
                    self.toggle(node);
                    ctx.request_redraw();
                    ctx.accept();
                    return EventResult::Handled;
                }
                self.set_selected(Some(node));
                ctx.capture_pointer(id);
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::MouseDoubleClick {
                button: MouseButton::Left,
                position,
            } if enabled && bounds.contains(*position) => {
                let local_y = position.y - bounds.y;
                let Some(index) = item_at(self.row_height, &self.scroll, self.rows.len(), local_y)
                else {
                    return EventResult::Unhandled;
                };
                let (node, _) = self.rows[index];
                if self.model.with(|m| m.is_leaf(&node)) {
                    self.set_selected(Some(node));
                    self.activated.emit(&node);
                } else {
                    self.toggle(node);
                }
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::MouseUp { .. } if ctx.interaction().owns_pointer(id) => {
                ctx.release_pointer();
                EventResult::Handled
            }
            UiEvent::MouseWheel { delta, .. } => {
                let offset = scroll_delta_to_offset(*delta);
                if offset.y == 0.0 {
                    return EventResult::Unhandled;
                }
                self.scroll.scroll_by(offset.y);
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::KeyDown { key, .. } if enabled => {
                let count = self.rows.len();
                if count == 0 {
                    return EventResult::Unhandled;
                }
                let current = self.selected_index();
                if key.is_named(NamedKey::ArrowDown) {
                    let index = current.map_or(0, |index| (index + 1).min(count - 1));
                    let (node, _) = self.rows[index];
                    self.select_index(node);
                } else if key.is_named(NamedKey::ArrowUp) {
                    let index = current.map_or(0, |index| index.saturating_sub(1));
                    let (node, _) = self.rows[index];
                    self.select_index(node);
                } else if key.is_named(NamedKey::Home) {
                    let (node, _) = self.rows[0];
                    self.select_index(node);
                } else if key.is_named(NamedKey::End) {
                    let (node, _) = self.rows[count - 1];
                    self.select_index(node);
                } else if key.is_named(NamedKey::ArrowRight) {
                    let Some(index) = current else {
                        let (node, _) = self.rows[0];
                        self.select_index(node);
                        ctx.accept();
                        return EventResult::Handled;
                    };
                    let (node, _) = self.rows[index];
                    let child_count = self.model.with(|m| m.child_count(&node));
                    if child_count == 0 {
                        // A leaf has nothing to reveal.
                        ctx.accept();
                        return EventResult::Handled;
                    }
                    let expanded = self.model.with(|m| m.is_expanded(&node));
                    if !expanded {
                        self.model.with_mut(|m| m.set_expanded(&node, true));
                        self.refresh_rows();
                    } else if let Some(first) = self.model.with(|m| m.child(&node, 0)) {
                        self.select_index(first);
                    }
                } else if key.is_named(NamedKey::ArrowLeft) {
                    let Some(index) = current else {
                        let (node, _) = self.rows[0];
                        self.select_index(node);
                        ctx.accept();
                        return EventResult::Handled;
                    };
                    let (node, _) = self.rows[index];
                    let expanded = self.model.with(|m| m.is_expanded(&node));
                    if self.model.with(|m| m.child_count(&node)) > 0 && expanded {
                        self.model.with_mut(|m| m.set_expanded(&node, false));
                        self.refresh_rows();
                    } else if let Some(parent) = self.model.with(|m| m.parent(&node)) {
                        self.select_index(parent);
                    }
                } else if key.is_named(NamedKey::Return) {
                    if let Some(node) = self.selected {
                        self.activated.emit(&node);
                    }
                    ctx.accept();
                    return EventResult::Handled;
                } else {
                    return EventResult::Unhandled;
                }
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
        self.refresh_rows();
        let theme = ctx.ui().theme();
        self.row_height = theme.control_height();
        self.scroll.content = self.rows.len() as f32 * self.row_height;
        self.scroll.viewport = self.base.bounds().height;
        self.scroll.clamp();
        let pad = theme.content_padding();
        let indent_step = theme.spacing.md + INDENT_GAP;

        // Window coordinates for the raw painter, local coordinates for the ctx helpers.
        let bounds = ctx.to_window_rect(self.base.bounds());
        let radius = control_radius(ctx);
        ctx.painter().styled_rect(
            bounds,
            radius,
            Fill::rounded(theme.colors.surface, theme.radius.sm),
        );

        ctx.push_clip_rounded(self.base.bounds(), theme.radius.sm);

        let style = theme.text(
            TextRole::Body,
            Some(if state.enabled {
                theme.colors.text
            } else {
                theme.colors.text_disabled
            }),
        );
        let line_height = ctx.line_height(&style);
        let pointer = ctx.interaction().pointer;
        let hovered = if self.hover_row.is_some() {
            self.hover_row
        } else if state.hovered && bounds.contains(pointer) {
            item_at(
                self.row_height,
                &self.scroll,
                self.rows.len(),
                pointer.y - bounds.y,
            )
        } else {
            None
        };

        for index in visible_range(self.row_height, &self.scroll, self.rows.len()) {
            let (node, depth) = self.rows[index];
            let top = item_top(self.row_height, index) - self.scroll.offset;
            let row_rect = Rect::new(bounds.x, bounds.y + top, bounds.width, self.row_height);
            if self.selected == Some(node) && state.enabled {
                ctx.painter().rect(row_rect, theme.colors.selection);
            } else if Some(index) == hovered {
                ctx.painter().rect(row_rect, theme.colors.surface_hover);
            }

            let indent = depth as f32 * indent_step;
            let twisty_left = pad.left + indent;
            let child_count = self.model.with(|model| model.child_count(&node));
            if child_count > 0 {
                let expanded = self.model.with(|model| model.is_expanded(&node));
                let center = ctx.to_window(Point::new(
                    twisty_left + TWISTY_WIDTH * 0.5,
                    top + self.row_height * 0.5,
                ));
                let width = theme.borders.thin.max(1.0);
                let color = theme.colors.text_muted;
                let painter = ctx.painter();
                if expanded {
                    // Downward triangle: top-left, top-right, bottom-centre.
                    let a = Point::new(center.x - 4.0, center.y - 3.0);
                    let b = Point::new(center.x + 4.0, center.y - 3.0);
                    let c = Point::new(center.x, center.y + 4.0);
                    painter.line(a, b, width, color);
                    painter.line(b, c, width, color);
                    painter.line(c, a, width, color);
                } else {
                    // Rightward triangle: top, bottom, right-centre.
                    let a = Point::new(center.x - 3.0, center.y - 4.0);
                    let b = Point::new(center.x - 3.0, center.y + 4.0);
                    let c = Point::new(center.x + 4.0, center.y);
                    painter.line(a, b, width, color);
                    painter.line(b, c, width, color);
                    painter.line(c, a, width, color);
                }
            }

            let label_x = twisty_left + TWISTY_WIDTH;
            let label = self.model.with(|model| model.data(&node).as_text());
            if label.is_empty() {
                continue;
            }
            let max_width = (bounds.width - label_x - pad.right).max(1.0);
            let label = fit_text(ctx, &label, &style, max_width);
            let position = center_in(
                Rect::new(label_x, top, max_width, self.row_height),
                Size::new(max_width, line_height),
            );
            let position = ctx.to_window(position);
            ctx.painter().text(position, &label, &style);
        }

        ctx.pop_clip();

        paint_thumb(ctx, self.base.bounds(), &self.scroll);
        if state.focused {
            focus_ring(ctx, self.base.bounds(), theme.radius.sm);
        }
    }
}

/// `true` when the model still knows `node` (some value or children).
fn is_live(model: &dyn TreeModel, node: &NodeId) -> bool {
    !model.data(node).is_none() || model.child_count(node) > 0
}

/// The root nodes of `model`: live nodes without a parent.
///
/// `TreeModel` exposes only `root_count()`, never the root ids, so the ids are discovered by
/// probing the id space from `0` upwards. Arena models (such as `TreeListModel`) assign ids in
/// insertion order and never reuse a freed id, therefore "live, no parent" walked in ascending id
/// order yields the roots in insertion order. The scan stops once `root_count()` roots were found
/// or after [`MAX_EMPTY_PROBES`] consecutive unknown ids.
fn roots_of(model: &dyn TreeModel) -> Vec<NodeId> {
    let wanted = model.root_count();
    let mut roots = Vec::with_capacity(wanted);
    if wanted == 0 {
        return roots;
    }
    let mut id = 0u32;
    let mut empty = 0u32;
    while roots.len() < wanted && empty < MAX_EMPTY_PROBES && id != u32::MAX {
        let node = NodeId::new(id);
        if is_live(model, &node) {
            empty = 0;
            if model.parent(&node).is_none() {
                roots.push(node);
            }
        } else {
            empty += 1;
        }
        id += 1;
    }
    roots
}

/// Depth first preorder flattening of the **visible** nodes: children are only descended into
/// when their parent is expanded, which is exactly what the view paints.
fn flatten(model: &dyn TreeModel) -> Vec<(NodeId, usize)> {
    let mut rows = Vec::new();
    let mut stack: Vec<(NodeId, usize)> = roots_of(model)
        .into_iter()
        .rev()
        .map(|node| (node, 0))
        .collect();
    while let Some((node, depth)) = stack.pop() {
        rows.push((node, depth));
        if model.is_expanded(&node) {
            for index in (0..model.child_count(&node)).rev() {
                if let Some(child) = model.child(&node, index) {
                    stack.push((child, depth + 1));
                }
            }
        }
    }
    rows
}

/// Number of nodes reachable from the roots, descending regardless of expansion.
fn count_nodes(model: &dyn TreeModel) -> usize {
    let mut stack: Vec<NodeId> = roots_of(model);
    let mut count = 0;
    while let Some(node) = stack.pop() {
        count += 1;
        for index in (0..model.child_count(&node)).rev() {
            if let Some(child) = model.child(&node, index) {
                stack.push(child);
            }
        }
    }
    count
}
