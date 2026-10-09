//! Shared machinery for the self painting item views (design document §51, virtualization).
//!
//! The item views do not create a widget per row: they keep a [`Scroll`] state, work out which
//! rows intersect the viewport and paint exactly those. The widget tree therefore has a constant
//! number of nodes no matter how large the model is, and the painter only ever sees the rows the
//! user can actually see.

use std::ops::Range;

use rustui_core::{clamp_scroll, PaintContext, Rect};

/// One dimensional scroll state: an offset over a content size inside a viewport size.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Scroll {
    pub offset: f32,
    pub content: f32,
    pub viewport: f32,
}

impl Scroll {
    /// Clamps the offset into `0 … content - viewport`.
    pub fn clamp(&mut self) {
        self.offset = clamp_scroll(self.offset, self.content, self.viewport);
    }

    /// Scrolls by `delta` (positive = content moves up).
    pub fn scroll_by(&mut self, delta: f32) {
        self.offset += delta;
        self.clamp();
    }

    /// One page, used by `PageUp` / `PageDown`.
    pub fn page(&self) -> f32 {
        (self.viewport * 0.9).max(1.0)
    }

    /// Scrolls the minimum amount that brings `top .. top + height` into view.
    pub fn ensure_visible(&mut self, top: f32, height: f32) {
        if top - self.offset < 0.0 {
            self.offset = top;
        } else if top + height - self.offset > self.viewport {
            self.offset = top + height - self.viewport;
        }
        self.clamp();
    }

    /// `true` when the content overflows the viewport.
    pub fn overflows(&self) -> bool {
        self.content > self.viewport + 0.5
    }
}

/// Indices of the items that intersect the viewport.
pub(crate) fn visible_range(item_height: f32, scroll: &Scroll, total: usize) -> Range<usize> {
    if item_height <= 0.0 || total == 0 || scroll.viewport <= 0.0 {
        return 0..0;
    }
    let first = (scroll.offset / item_height).floor().max(0.0) as usize;
    let past = ((scroll.offset + scroll.viewport) / item_height).ceil() as usize;
    let last = past.min(total);
    if last <= first {
        return first..first;
    }
    first..last
}

/// Index of the item under a **local** `y`, when there is one.
pub(crate) fn item_at(item_height: f32, scroll: &Scroll, total: usize, y: f32) -> Option<usize> {
    if item_height <= 0.0 {
        return None;
    }
    let raw = (y + scroll.offset) / item_height;
    if raw < 0.0 {
        return None;
    }
    let index = raw.floor() as usize;
    (index < total).then_some(index)
}

/// Top edge of item `index` in the coordinate space of the viewport.
pub(crate) fn item_top(item_height: f32, index: usize) -> f32 {
    index as f32 * item_height
}

/// Vertical scrollbar thumb on the trailing edge of `bounds`.
///
/// `bounds` is expressed in the **local** coordinates of the calling widget.
pub(crate) fn paint_thumb(ctx: &mut PaintContext<'_>, bounds: Rect, scroll: &Scroll) {
    if !scroll.overflows() || scroll.content <= 0.0 || bounds.height <= 0.0 {
        return;
    }
    let theme = ctx.ui().theme();
    let width = theme.spacing.sm * 0.5;
    let ratio = (scroll.viewport / scroll.content).clamp(0.1, 1.0);
    let track_height = bounds.height;
    let thumb_height = (track_height * ratio).max(theme.spacing.sm);
    let travel = (track_height - thumb_height) * scroll.offset / scroll.content.max(1.0);
    let thumb = Rect::new(
        bounds.x + bounds.width - width,
        bounds.y + travel,
        width,
        thumb_height,
    );
    ctx.fill_rounded(thumb, width * 0.5, theme.colors.border);
}
