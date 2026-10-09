//! Scroll view: a viewport over one larger child, with wheel / key driven scrolling.

use rustui_core::{
    clamp_scroll, impl_widget_base, scroll_delta_to_offset, ArrangeContext, EventContext,
    EventResult, MeasureContext, NamedKey, ObjectId, PaintContext, Rect, Size, SizeHint, UiEvent,
    Vec2, Widget, WidgetBase,
};

/// A viewport over a single child that is larger than the space it is given.
///
/// The child is arranged at `-offset`, so scrolling is pure geometry: the child's own layout never
/// changes and [`rustui_core::PaintContext::paint_child`] clips it to the viewport for free.
///
/// ```ignore
/// let body = ...;                       // some tall widget
/// let scroll = ScrollView::new().child(body);
/// ```
#[derive(Debug)]
pub struct ScrollView {
    base: WidgetBase,
    offset: Vec2,
    content: Size,
    viewport: Size,
    /// `true` while a thin scrollbar thumb should be shown.
    show_thumb: bool,
}

impl ScrollView {
    /// An empty scroll view.
    pub fn new() -> Self {
        Self {
            base: WidgetBase::new(),
            offset: Vec2::ZERO,
            content: Size::ZERO,
            viewport: Size::ZERO,
            show_thumb: false,
        }
    }

    /// A scroll view with one child (the scrolled content).
    pub fn child(mut self, child: ObjectId) -> Self {
        self.base.add_child(child);
        self
    }

    /// The scrolled content, if any.
    pub fn content_id(&self) -> Option<ObjectId> {
        self.children().first().copied()
    }

    /// Current scroll position in content coordinates.
    pub fn offset(&self) -> Vec2 {
        self.offset
    }

    /// Sets the scroll position (clamped to the content).
    pub fn set_offset(&mut self, offset: Vec2) {
        self.offset = Vec2::new(
            clamp_scroll(offset.x, self.content.width, self.viewport.width),
            clamp_scroll(offset.y, self.content.height, self.viewport.height),
        );
    }

    /// Size of the content measured during the last layout.
    pub fn content_size(&self) -> Size {
        self.content
    }

    /// Size of the viewport measured during the last layout.
    pub fn viewport_size(&self) -> Size {
        self.viewport
    }

    fn clamp(&mut self) {
        self.offset = Vec2::new(
            clamp_scroll(self.offset.x, self.content.width, self.viewport.width),
            clamp_scroll(self.offset.y, self.content.height, self.viewport.height),
        );
    }
}

impl Default for ScrollView {
    fn default() -> Self {
        Self::new()
    }
}

impl_widget_base!(ScrollView);

impl Widget for ScrollView {
    fn kind(&self) -> &'static str {
        "ScrollView"
    }

    /// A view fills whatever space it is offered on both axes.
    fn size_hint(&self) -> SizeHint {
        SizeHint::fill()
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let content = match self.content_id() {
            Some(child) => ctx.measure_child(child, Size::new(f32::INFINITY, f32::INFINITY)),
            None => Size::ZERO,
        };
        // The viewport fills what it is offered; with an infinite offer (shrink-to-fit parent)
        // it hugs the content so the scroll view never forces a scrollbar it does not need.
        let viewport = Size::new(
            if available.width.is_finite() {
                available.width.max(0.0)
            } else {
                content.width
            },
            if available.height.is_finite() {
                available.height.max(0.0)
            } else {
                content.height
            },
        );
        self.content = content;
        self.viewport = viewport;
        self.clamp();
        self.show_thumb =
            content.width > viewport.width + 0.5 || content.height > viewport.height + 0.5;
        self.base.set_measured(viewport);
        viewport
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
        // Re-clamp with the box we actually got: `arrange` may be tighter than `measure`.
        self.viewport = rect.size();
        self.clamp();
        if let Some(child) = self.content_id() {
            // The content gets its natural size and sits at `-offset`.
            let content = Rect::new(
                -self.offset.x,
                -self.offset.y,
                self.content.width,
                self.content.height,
            );
            ctx.arrange_child(child, content);
        }
    }

    fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult {
        match event {
            UiEvent::MouseWheel { delta, .. } => {
                self.offset += scroll_delta_to_offset(*delta);
                self.clamp();
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::KeyDown { key, .. } if self.content.height > self.viewport.height => {
                let page = self.viewport.height * 0.9;
                if key.is_named(NamedKey::PageDown) {
                    self.offset.y += page;
                } else if key.is_named(NamedKey::PageUp) {
                    self.offset.y -= page;
                } else if key.is_named(NamedKey::Home) {
                    self.offset.y = 0.0;
                } else if key.is_named(NamedKey::End) {
                    self.offset.y = self.content.height;
                } else {
                    return EventResult::Unhandled;
                }
                self.clamp();
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            _ => EventResult::Unhandled,
        }
    }

    fn paint(&mut self, ctx: &mut PaintContext<'_>) {
        let bounds = self.base.bounds();
        ctx.push_clip(bounds);
        if let Some(child) = self.content_id() {
            ctx.paint_child(child);
        }
        ctx.pop_clip();

        // A minimal scrollbar thumb on the trailing edge while the content overflows.
        if self.show_thumb && bounds.height > 0.0 && self.content.height > 0.0 {
            let theme = ctx.ui().theme();
            let track = Rect::new(
                bounds.right() - theme.spacing.sm * 0.5,
                bounds.y,
                theme.spacing.sm * 0.5,
                bounds.height,
            );
            let ratio = (self.viewport.height / self.content.height).clamp(0.1, 1.0);
            let thumb_height = (track.height * ratio).max(theme.spacing.sm);
            let travel = (track.height - thumb_height) * self.offset.y
                / (self.content.height - self.viewport.height).max(1.0);
            let thumb = Rect::new(track.x, track.y + travel, track.width, thumb_height);
            ctx.fill_rounded(thumb, thumb.width * 0.5, theme.colors.border);
        }
    }
}
