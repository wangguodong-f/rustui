//! [`Stack`] — children drawn on top of each other in the same box.

use rustui_core::impl_widget_base;
use rustui_core::widget::align_child;
use rustui_core::{
    Align, ArrangeContext, EventContext, EventResult, Insets, MeasureContext, ObjectId,
    PaintContext, Rect, Size, SizeHint, UiEvent, Widget, WidgetBase,
};

use crate::style::{
    apply_fill, clamp_to, content_available, content_rect, measured_size, paint_children,
    visible_children, visible_children_in, ContainerStyle,
};

/// Overlapping children: every child is arranged inside the same content box and painted in
/// child order, so later children end up on top (design document §15).
///
/// Used for badges, hover overlays, scrollable canvases with a floating toolbar, and as the
/// backing store of `Overlay` / `Popup` widgets.
#[derive(Debug)]
pub struct Stack {
    base: WidgetBase,
    padding: Insets,
    /// Alignment of every child inside the content box; `(Align::Stretch, Align::Stretch)`
    /// (the default) makes the children fill the stack.
    align: (Align, Align),
    style: ContainerStyle,
    hint: SizeHint,
}

impl Stack {
    pub fn new() -> Self {
        Self {
            base: WidgetBase::new(),
            padding: Insets::ZERO,
            align: (Align::Stretch, Align::Stretch),
            style: ContainerStyle::EMPTY,
            hint: SizeHint::default(),
        }
    }

    pub fn with_child(mut self, id: ObjectId) -> Self {
        self.base.add_child(id);
        self
    }

    pub fn with_padding(mut self, padding: Insets) -> Self {
        self.padding = padding;
        self
    }

    pub fn with_align(mut self, h: Align, v: Align) -> Self {
        self.align = (h, v);
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

    pub fn with_clip(mut self, clip: bool) -> Self {
        self.style.clip = clip;
        self
    }

    /// The stack takes the space its parent offers instead of hugging its largest child.
    pub fn with_fill(mut self) -> Self {
        self.hint = SizeHint::fill();
        self
    }

    pub fn push(&mut self, id: ObjectId) -> &mut Self {
        self.base.add_child(id);
        self
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}

impl_widget_base!(Stack);
impl rustui_core::Layout for Stack {}

impl Widget for Stack {
    fn kind(&self) -> &'static str {
        "Stack"
    }

    fn size_hint(&self) -> SizeHint {
        self.hint
    }

    fn is_opaque(&self) -> bool {
        self.style.background.a > 0.0
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let inner = content_available(available, self.padding);
        let mut size = Size::ZERO;
        for id in visible_children_in(ctx, self.base.children()) {
            size = size.max(ctx.measure_child(id, inner));
        }
        let size = apply_fill(
            clamp_to(size.expand(self.padding), available),
            available,
            self.hint,
        );
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
        let content = content_rect(Rect::new(0.0, 0.0, rect.width, rect.height), self.padding);
        for id in visible_children(ctx.store(), self.base.children()) {
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
        paint_children(ctx, self.base.children());
        self.style.paint_end(ctx, bounds);
    }
}
