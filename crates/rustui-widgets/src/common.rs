//! Small painting helpers shared by the widget library.

use rustui_core::{Border, Color, EdgeRadius, EdgeWidths, Fill, PaintContext, Point, Rect, Size};

/// Draws the keyboard focus ring around the **local** rectangle `rect`.
///
/// The ring is a one pixel border drawn just inside the widget's outline, which is what the
/// renderer's SDF border path is for: no extra clip, no erasing pass.
pub(crate) fn focus_ring(ctx: &mut PaintContext<'_>, rect: Rect, radius: f32) {
    let theme = ctx.ui().theme();
    let width = theme.borders.thin.max(1.0);
    let rect = ctx.to_window_rect(rect);
    let fill = Fill {
        color: Color::TRANSPARENT,
        gradient: None,
        border: Border {
            widths: EdgeWidths::all(width),
            color: theme.colors.focus_ring,
        },
        radius: EdgeRadius::all(radius + width * 0.5),
    };
    ctx.painter().styled_rect(rect, fill.radius, fill);
}

/// Top-left corner that centres a `size` box inside `rect` (never leaves the rectangle).
pub(crate) fn center_in(rect: Rect, size: Size) -> Point {
    Point::new(
        rect.x + ((rect.width - size.width) * 0.5).max(0.0),
        rect.y + ((rect.height - size.height) * 0.5).max(0.0),
    )
}

/// Corner radius used by the small surfaces (buttons, inputs, rows).
pub(crate) fn control_radius(ctx: &PaintContext<'_>) -> EdgeRadius {
    EdgeRadius::all(ctx.ui().theme().radius.sm)
}
