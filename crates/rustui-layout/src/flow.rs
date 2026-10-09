//! [`Flow`] — children laid out left to right, wrapping at the container width.

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

/// One wrapped line of a [`Flow`].
struct Line {
    items: Vec<usize>,
    width: f32,
    height: f32,
}

/// Children flow along the horizontal axis and wrap into new lines when the available width is
/// exceeded — the layout behind tag clouds, toolbars and wrapping button rows.
#[derive(Debug)]
pub struct Flow {
    base: WidgetBase,
    /// Gap between two children (both axes).
    spacing: Size,
    padding: Insets,
    /// `.0` aligns a line horizontally inside the container, `.1` aligns a child vertically
    /// inside its line.
    align: (Align, Align),
    style: ContainerStyle,
    hint: SizeHint,
}

impl Flow {
    pub fn new() -> Self {
        Self {
            base: WidgetBase::new(),
            spacing: Size::ZERO,
            padding: Insets::ZERO,
            align: (Align::Start, Align::Center),
            style: ContainerStyle::EMPTY,
            hint: SizeHint::default(),
        }
    }

    pub fn with_child(mut self, id: ObjectId) -> Self {
        self.base.add_child(id);
        self
    }

    /// Sets the horizontal and vertical gap between children.
    pub fn with_spacing(mut self, spacing: f32) -> Self {
        self.spacing = Size::new(spacing, spacing);
        self
    }

    pub fn with_gaps(mut self, horizontal: f32, vertical: f32) -> Self {
        self.spacing = Size::new(horizontal, vertical);
        self
    }

    pub fn with_padding(mut self, padding: Insets) -> Self {
        self.padding = padding;
        self
    }

    pub fn with_align(mut self, line: Align, item: Align) -> Self {
        self.align = (line, item);
        self
    }

    pub fn with_style(mut self, style: ContainerStyle) -> Self {
        self.style = style;
        self
    }

    pub fn with_fill(mut self) -> Self {
        self.hint = SizeHint::fill();
        self
    }

    pub fn push(&mut self, id: ObjectId) -> &mut Self {
        self.base.add_child(id);
        self
    }

    /// Breaks `sizes` into lines that fit `max_width`.
    fn lines(&self, sizes: &[Size], max_width: f32) -> Vec<Line> {
        let mut lines: Vec<Line> = Vec::new();
        let mut items: Vec<usize> = Vec::new();
        let mut width = 0.0f32;
        let mut height = 0.0f32;
        for (index, size) in sizes.iter().enumerate() {
            let gap = if items.is_empty() {
                0.0
            } else {
                self.spacing.width
            };
            let next = width + gap + size.width;
            if !items.is_empty() && max_width.is_finite() && next > max_width {
                lines.push(Line {
                    items: std::mem::take(&mut items),
                    width,
                    height,
                });
                width = 0.0;
                height = 0.0;
            }
            if !items.is_empty() {
                width += self.spacing.width;
            }
            items.push(index);
            width += size.width;
            height = height.max(size.height);
        }
        if !items.is_empty() {
            lines.push(Line {
                items,
                width,
                height,
            });
        }
        lines
    }
}

impl Default for Flow {
    fn default() -> Self {
        Self::new()
    }
}

impl_widget_base!(Flow);
impl rustui_core::Layout for Flow {}

impl Widget for Flow {
    fn kind(&self) -> &'static str {
        "Flow"
    }

    fn size_hint(&self) -> SizeHint {
        self.hint
    }

    fn is_opaque(&self) -> bool {
        self.style.background.a > 0.0
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let inner = content_available(available, self.padding);
        let children = visible_children_in(ctx, self.base.children());
        let sizes: Vec<Size> = children
            .iter()
            .map(|id| ctx.measure_child(*id, Size::new(inner.width, f32::INFINITY)))
            .collect();
        let lines = self.lines(&sizes, inner.width);
        let width = lines.iter().map(|l| l.width).fold(0.0f32, f32::max);
        let gaps = self.spacing.height * (lines.len().saturating_sub(1)) as f32;
        let height: f32 = lines.iter().map(|l| l.height).sum::<f32>() + gaps;
        let size = apply_fill(
            clamp_to(Size::new(width, height).expand(self.padding), available),
            available,
            self.hint,
        );
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
        let content = content_rect(Rect::new(0.0, 0.0, rect.width, rect.height), self.padding);
        let children = visible_children(ctx.store(), self.base.children());
        if children.is_empty() {
            return;
        }
        let sizes: Vec<Size> = children.iter().map(|id| measured_size(ctx, *id)).collect();
        let lines = self.lines(&sizes, content.width);

        let mut line_top = content.top();
        for line in &lines {
            let line_left = content.left()
                + match self.align.0 {
                    Align::Center => (content.width - line.width) * 0.5,
                    Align::End => content.width - line.width,
                    _ => 0.0,
                };
            let mut x = line_left;
            for index in &line.items {
                let size = sizes[*index];
                let cell = Rect::new(x, line_top, size.width, line.height);
                let rect = align_child(cell, size, Align::Start, self.align.1);
                ctx.arrange_child(children[*index], rect);
                x += size.width + self.spacing.width;
            }
            line_top += line.height + self.spacing.height;
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
