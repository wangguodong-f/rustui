//! [`Container`]: a single child wrapped in padding, background, border and clip.

use rustui_core::impl_widget_base;
use rustui_core::widget::align_child;
use rustui_core::{
    Align, ArrangeContext, EventContext, EventResult, Insets, MeasureContext, ObjectId,
    PaintContext, Rect, Size, SizeHint, UiEvent, Widget, WidgetBase,
};

use crate::style::{
    apply_fill, clamp_to, content_available, content_rect, measured_size, ContainerStyle,
};

/// A widget that gives one child a box: padding, background, border, radius, clipping and
/// alignment (design document §15).
///
/// ```text
/// ┌─ padding ────────────────────┐
/// │  ┌─ child (aligned) ──────┐  │   background + border
/// │  └────────────────────────┘  │
/// └──────────────────────────────┘
/// ```
#[derive(Debug)]
pub struct Container {
    base: WidgetBase,
    child: Option<ObjectId>,
    padding: Insets,
    align: (Align, Align),
    style: ContainerStyle,
    hint: SizeHint,
}

impl Container {
    pub fn new() -> Self {
        Self {
            base: WidgetBase::new(),
            child: None,
            padding: Insets::ZERO,
            align: (Align::Stretch, Align::Stretch),
            style: ContainerStyle::EMPTY,
            hint: SizeHint::default(),
        }
    }

    /// Wraps `child` (which must already be registered in the widget store).
    pub fn with_child(child: ObjectId) -> Self {
        Self::new().child(child)
    }

    /// Sets the wrapped child.
    pub fn child(mut self, id: ObjectId) -> Self {
        self.child = Some(id);
        self.base.add_child(id);
        self
    }

    pub fn with_padding(mut self, padding: Insets) -> Self {
        self.padding = padding;
        self
    }

    pub fn with_style(mut self, style: ContainerStyle) -> Self {
        self.style = style;
        self
    }

    pub fn with_background(mut self, color: rustui_core::Color) -> Self {
        self.style.background = color;
        self
    }

    pub fn with_border(mut self, color: rustui_core::Color, width: f32) -> Self {
        self.style.border_color = color;
        self.style.border_width = width;
        self
    }

    pub fn with_radius(mut self, radius: f32) -> Self {
        self.style.corner_radius = radius;
        self
    }

    pub fn with_clip(mut self, clip: bool) -> Self {
        self.style.clip = clip;
        self
    }

    /// Horizontal and vertical alignment of the child inside the content box.
    pub fn with_align(mut self, h: Align, v: Align) -> Self {
        self.align = (h, v);
        self
    }

    /// The container takes the space its parent offers instead of hugging the child.
    pub fn with_fill(mut self) -> Self {
        self.hint = SizeHint::fill();
        self
    }

    /// The container's own size hint.
    pub fn with_size_hint(mut self, hint: SizeHint) -> Self {
        self.hint = hint;
        self
    }

    pub fn padding(&self) -> Insets {
        self.padding
    }

    pub fn set_padding(&mut self, padding: Insets) {
        self.padding = padding;
    }

    pub fn style(&self) -> &ContainerStyle {
        &self.style
    }

    pub fn style_mut(&mut self) -> &mut ContainerStyle {
        &mut self.style
    }
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

impl_widget_base!(Container);
impl rustui_core::Layout for Container {}

impl Widget for Container {
    fn kind(&self) -> &'static str {
        "Container"
    }

    fn size_hint(&self) -> SizeHint {
        self.hint
    }

    fn is_opaque(&self) -> bool {
        self.style.background.a > 0.0
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let inner = content_available(available, self.padding);
        let content = match self.child {
            Some(id) => ctx.measure_child(id, inner),
            None => Size::ZERO,
        };
        let size = content.expand(self.padding);
        let size = apply_fill(clamp_to(size, available), available, self.hint);
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
        let content = content_rect(Rect::new(0.0, 0.0, rect.width, rect.height), self.padding);
        if let Some(id) = self.child {
            let natural = measured_size(ctx, id);
            ctx.arrange_child(
                id,
                align_child(content, natural, self.align.0, self.align.1),
            );
        }
    }

    fn event(&mut self, _ctx: &mut EventContext<'_>, _event: &UiEvent) -> EventResult {
        EventResult::Unhandled
    }

    fn paint(&mut self, ctx: &mut PaintContext<'_>) {
        let bounds = self.base.bounds();
        self.style.paint_begin(ctx, bounds);
        if let Some(id) = self.child {
            ctx.paint_child(id);
        }
        self.style.paint_end(ctx, bounds);
    }
}
