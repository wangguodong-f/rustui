//! Helpers shared by every layout widget.
//!
//! Layout widgets follow the coordinate conventions documented in the crate root: `measure` runs
//! top-down before `arrange`, `arrange` works entirely in the widget's local space (origin at its
//! top-left corner) and never re-measures — the size recorded by `measure` is read back through
//! the [`WidgetStore`].

use rustui_core::{Color, ObjectId, PaintContext, Rect, Size, SizeHint, WidgetStore};

/// Axis a box-like widget lays its children out along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Top to bottom.
    Vertical,
    /// Left to right.
    Horizontal,
}

impl Axis {
    /// Size component along this axis.
    #[inline]
    pub fn main(self, size: Size) -> f32 {
        match self {
            Axis::Vertical => size.height,
            Axis::Horizontal => size.width,
        }
    }

    /// Size component across this axis.
    #[inline]
    pub fn cross(self, size: Size) -> f32 {
        match self {
            Axis::Vertical => size.width,
            Axis::Horizontal => size.height,
        }
    }

    /// Builds a size from a main-axis and a cross-axis value.
    #[inline]
    pub fn size(self, main: f32, cross: f32) -> Size {
        match self {
            Axis::Vertical => Size::new(cross, main),
            Axis::Horizontal => Size::new(main, cross),
        }
    }

    /// `true` when `value` is the coordinate along this axis.
    #[inline]
    pub fn is_vertical(self) -> bool {
        matches!(self, Axis::Vertical)
    }
}

/// Visual decoration shared by the container widgets (background, border, radius, clipping).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContainerStyle {
    pub background: Color,
    pub border_color: Color,
    pub border_width: f32,
    pub corner_radius: f32,
    /// `true` to clip children to the container bounds while painting.
    pub clip: bool,
}

impl Default for ContainerStyle {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl ContainerStyle {
    /// Nothing is drawn and nothing is clipped.
    pub const EMPTY: Self = Self {
        background: Color::TRANSPARENT,
        border_color: Color::TRANSPARENT,
        border_width: 0.0,
        corner_radius: 0.0,
        clip: false,
    };

    /// Fills `bounds` (local coordinates) with the background color.
    pub fn paint_background(&self, ctx: &mut PaintContext<'_>, bounds: Rect) {
        if self.background.a <= 0.0 {
            return;
        }
        if self.corner_radius > 0.0 {
            ctx.fill_rounded(bounds, self.corner_radius, self.background);
        } else {
            ctx.fill_rect(bounds, self.background);
        }
    }

    /// Starts clipping children to `bounds` when the style asks for it.
    pub fn push_clip(&self, ctx: &mut PaintContext<'_>, bounds: Rect) {
        if !self.clip {
            return;
        }
        if self.corner_radius > 0.0 {
            ctx.push_clip_rounded(bounds, self.corner_radius);
        } else {
            ctx.push_clip(bounds);
        }
    }

    /// Ends the clip started by [`ContainerStyle::push_clip`].
    pub fn pop_clip(&self, ctx: &mut PaintContext<'_>) {
        if self.clip {
            ctx.pop_clip();
        }
    }

    /// Draws the border on top of the children.
    pub fn paint_border(&self, ctx: &mut PaintContext<'_>, bounds: Rect) {
        if self.border_width <= 0.0 || self.border_color.a <= 0.0 {
            return;
        }
        ctx.stroke_rect(
            bounds,
            self.corner_radius,
            self.border_color,
            self.border_width,
        );
    }

    /// Convenience: background + clip, the pair every container starts its `paint` with.
    pub fn paint_begin(&self, ctx: &mut PaintContext<'_>, bounds: Rect) {
        self.paint_background(ctx, bounds);
        self.push_clip(ctx, bounds);
    }

    /// Convenience: end of clip + border, the pair every container ends its `paint` with.
    pub fn paint_end(&self, ctx: &mut PaintContext<'_>, bounds: Rect) {
        self.pop_clip(ctx);
        self.paint_border(ctx, bounds);
    }
}

/// Content rectangle of `bounds` once `padding` is removed, expressed in local coordinates.
pub fn content_rect(bounds: Rect, padding: rustui_core::Insets) -> Rect {
    Rect::new(
        padding.left,
        padding.top,
        (bounds.width - padding.horizontal()).max(0.0),
        (bounds.height - padding.vertical()).max(0.0),
    )
}

/// Size available to the content of a padded widget.
///
/// An infinite component stays infinite, so a container that is free to hug its content passes
/// that freedom on to its children.
pub fn content_available(available: Size, padding: rustui_core::Insets) -> Size {
    Size::new(
        if available.width.is_finite() {
            (available.width - padding.horizontal()).max(0.0)
        } else {
            f32::INFINITY
        },
        if available.height.is_finite() {
            (available.height - padding.vertical()).max(0.0)
        } else {
            f32::INFINITY
        },
    )
}

/// Clamps `size` to `available`, leaving infinite components untouched.
pub fn clamp_to(size: Size, available: Size) -> Size {
    Size::new(
        if available.width.is_finite() {
            size.width.min(available.width)
        } else {
            size.width
        },
        if available.height.is_finite() {
            size.height.min(available.height)
        } else {
            size.height
        },
    )
}

/// Applies the *fill* part of `hint`: an axis whose preference is infinite means "take the space
/// I am offered", which is how [`rustui_core::SizeHint::fill`] reaches `measure`.
pub fn apply_fill(size: Size, available: Size, hint: SizeHint) -> Size {
    Size::new(
        if !hint.width.is_finite() && available.width.is_finite() {
            available.width
        } else {
            size.width
        },
        if !hint.height.is_finite() && available.height.is_finite() {
            available.height
        } else {
            size.height
        },
    )
}

/// `true` when `hint` asks for more space than it measures, i.e. the widget should receive a
/// share of the free space left by its siblings along `axis`.
pub fn expands(hint: &SizeHint, axis: Axis) -> bool {
    let preferred = match axis {
        Axis::Vertical => hint.height,
        Axis::Horizontal => hint.width,
    };
    hint.flex > 0.0 || !preferred.is_finite()
}

/// Size of `id` recorded by the last `measure` pass, clamped by its size hint.
///
/// `arrange` never re-measures (the arrange context has no text engine behind it), so containers
/// read their children's measured sizes back from the store instead.
pub fn measured_size(ctx: &rustui_core::ArrangeContext<'_>, id: ObjectId) -> Size {
    let store = ctx.store();
    match store.get(id) {
        Some(widget) => {
            let hint = widget.size_hint();
            let size = widget.base().measured();
            hint.apply(size)
        }
        None => Size::ZERO,
    }
}

/// Same as [`measured_size`], but for code that only holds a store reference.
pub fn measured_size_in(store: &WidgetStore, id: ObjectId) -> Size {
    match store.get(id) {
        Some(widget) => {
            let hint = widget.size_hint();
            let size = widget.base().measured();
            hint.apply(size)
        }
        None => Size::ZERO,
    }
}

/// Size hint of `id`, or the default when it is gone.
pub fn hint_of(store: &WidgetStore, id: ObjectId) -> SizeHint {
    store.get(id).map(|w| w.size_hint()).unwrap_or_default()
}

/// Children that are currently visible; invisible widgets are skipped by measure, arrange and
/// paint alike, which keeps the three passes consistent.
pub fn visible_children(store: &WidgetStore, children: &[ObjectId]) -> Vec<ObjectId> {
    children
        .iter()
        .copied()
        .filter(|id| {
            store
                .get(*id)
                .map(|w| w.base().is_visible())
                .unwrap_or(false)
        })
        .collect()
}

/// Same as [`visible_children`], starting from a measure context.
pub fn visible_children_in(
    ctx: &rustui_core::MeasureContext<'_>,
    children: &[ObjectId],
) -> Vec<ObjectId> {
    visible_children(ctx.store(), children)
}

/// Paints every visible child of a container (used by `paint` implementations).
pub fn paint_children(ctx: &mut PaintContext<'_>, children: &[ObjectId]) {
    let ids = visible_children(ctx.store(), children);
    for id in ids {
        ctx.paint_child(id);
    }
}
