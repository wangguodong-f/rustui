//! Modal-ish dialog: a titled, closable surface over an application supplied content widget.

use rustui_core::{
    impl_widget_base, ArrangeContext, EventContext, EventResult, MeasureContext, MouseButton,
    NamedKey, ObjectId, PaintContext, Point, Rect, SharedProperty, Signal, Size, TextRole,
    TextStyle, UiContext, UiEvent, Widget, WidgetBase,
};

use crate::common::{center_in, focus_ring};

/// A titled popup panel.
///
/// The dialog is an ordinary widget that the application shows as an overlay, so the content is
/// built by the application (no hidden children, §52) and the dialog only adds the chrome: a
/// title bar, a close button and a shadow.
///
/// ```ignore
/// let body = ...;                                   // some content widget
/// let dialog = Dialog::new("Save changes?").child(body);
/// dialog.close_requested().connect(move |_| { /* hide the overlay */ });
/// ```
#[derive(Debug)]
pub struct Dialog {
    base: WidgetBase,
    title: SharedProperty<String>,
    closeable: bool,
    close_requested: Signal<()>,
    /// Content size measured during `measure`, used to place the child.
    content: Size,
    /// Width override for tiny content (`None` keeps the default of four control heights).
    min_width: Option<f32>,
}

impl Dialog {
    /// A dialog titled `title`.
    pub fn new(title: impl Into<String>) -> Self {
        Self::with_title(SharedProperty::new(title.into()))
    }

    /// A dialog whose title is bound to `title`.
    pub fn with_title(title: SharedProperty<String>) -> Self {
        Self {
            base: WidgetBase::new(),
            title,
            closeable: true,
            close_requested: Signal::new(),
            content: Size::ZERO,
            min_width: None,
        }
    }

    /// Overrides the minimum width of the dialog (T-7 / C-050).
    ///
    /// Defaults to four control heights - room for a title and a close button - which pushes very
    /// narrow content apart; a narrow dialog sets its own floor here (or a lower one). Content
    /// wider than the floor still wins.
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = Some(width);
        self
    }

    /// Adds the content widget (created by the application before the dialog is boxed).
    pub fn child(mut self, child: ObjectId) -> Self {
        self.base.add_child(child);
        self
    }

    /// Shows or hides the close button (default: shown).
    pub fn closeable(mut self, closeable: bool) -> Self {
        self.closeable = closeable;
        self
    }

    /// Current title.
    pub fn title(&self) -> String {
        self.title.get()
    }

    /// The shared title property.
    pub fn title_property(&self) -> SharedProperty<String> {
        self.title.clone()
    }

    /// Fires when the user asks the dialog to close (`✕` or `Escape`); the application decides
    /// whether to hide the overlay.
    pub fn close_requested(&self) -> Signal<()> {
        self.close_requested.clone()
    }

    /// The content widget, if any.
    pub fn content_id(&self) -> Option<ObjectId> {
        self.children().first().copied()
    }

    /// Height of the title bar.
    fn title_height(&self, ui: &UiContext) -> f32 {
        ui.theme().control_height()
    }

    /// Close button rectangle, in the same coordinate space as `rect`.
    fn close_rect(&self, ui: &UiContext, rect: Rect) -> Rect {
        let bar = self.title_height(ui);
        let size = (bar * 0.6).max(8.0);
        Rect::new(
            rect.right() - bar + (bar - size) * 0.5,
            rect.y + (bar - size) * 0.5,
            size,
            size,
        )
    }

    fn title_style(&self, ui: &UiContext) -> TextStyle {
        ui.theme()
            .text(TextRole::Title, Some(ui.theme().colors.text))
    }
}

impl_widget_base!(Dialog);

impl Widget for Dialog {
    fn kind(&self) -> &'static str {
        "Dialog"
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn is_opaque(&self) -> bool {
        true
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let theme = ctx.ui().theme();
        let pad = theme.content_padding();
        let content = match self.content_id() {
            Some(child) => ctx.measure_child(child, Size::new(f32::INFINITY, f32::INFINITY)),
            None => Size::ZERO,
        };
        self.content = content;
        let title_h = self.title_height(ctx.ui());
        let mut size = Size::new(
            content.width + pad.horizontal(),
            content.height + pad.vertical() + title_h,
        );
        // Room for a title even without content.
        size.width = size.width.max(
            self.min_width
                .unwrap_or_else(|| theme.control_height() * 4.0),
        );
        size.height = size.height.max(title_h);
        let size = Size::new(
            if available.width.is_finite() {
                size.width.min(available.width.max(0.0))
            } else {
                size.width
            },
            if available.height.is_finite() {
                size.height.min(available.height.max(0.0))
            } else {
                size.height
            },
        );
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
        let theme = ctx.ui().theme();
        let pad = theme.content_padding();
        let title_h = self.title_height(ctx.ui());
        if let Some(child) = self.content_id() {
            let area = Rect::new(
                pad.left,
                title_h + pad.top,
                (rect.width - pad.horizontal()).max(0.0),
                (rect.height - title_h - pad.vertical()).max(0.0),
            );
            // The content keeps its measured size, anchored at the top-left of the content area.
            let child_rect = Rect::new(
                area.x,
                area.y,
                self.content.width.min(area.width).max(0.0),
                self.content.height.min(area.height).max(0.0),
            );
            ctx.arrange_child(child, child_rect);
        }
    }

    fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult {
        let enabled = self.base.is_enabled();
        match event {
            UiEvent::MouseDown {
                button: MouseButton::Left,
                position,
            } if enabled && self.closeable => {
                let close = self.close_rect(ctx.ui(), self.base.window_bounds());
                if close.contains(*position) {
                    self.close_requested.emit(&());
                    ctx.request_redraw();
                    ctx.accept();
                    return EventResult::Handled;
                }
                EventResult::Unhandled
            }
            UiEvent::KeyDown { key, .. } if enabled => {
                if key.is_named(NamedKey::Escape) && self.closeable {
                    self.close_requested.emit(&());
                    ctx.request_redraw();
                    ctx.accept();
                    return EventResult::Handled;
                }
                EventResult::Unhandled
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
        let radius = rustui_core::EdgeRadius::all(theme.radius.lg);

        // Drop shadow, then the surface.
        let shadow = Rect::new(
            bounds.x + theme.spacing.sm * 0.5,
            bounds.y + theme.spacing.sm * 0.5,
            bounds.width,
            bounds.height,
        );
        ctx.painter()
            .rounded_rect(shadow, theme.radius.lg, theme.colors.shadow);
        let surface = rustui_core::Fill::rounded(theme.colors.surface, theme.radius.lg)
            .with_border(rustui_core::Border {
                widths: rustui_core::EdgeWidths::all(theme.borders.thin),
                color: theme.colors.border_strong,
            });
        ctx.painter().styled_rect(bounds, radius, surface);

        // Title bar.
        let title_h = self.title_height(ctx.ui());
        let bar = Rect::new(bounds.x, bounds.y, bounds.width, title_h);
        ctx.painter().rect(bar, theme.colors.surface_alt);
        ctx.painter().rect(
            Rect::new(
                bar.x,
                bar.bottom() - theme.borders.thin,
                bar.width,
                theme.borders.thin,
            ),
            theme.colors.border,
        );

        let pad = theme.content_padding();
        let mut title_style = self.title_style(ctx.ui());
        if !state.enabled {
            title_style.color = theme.colors.text_disabled;
        }
        let title = self.title.get();
        if !title.is_empty() {
            let line_height = ctx.line_height(&title_style);
            let mut area = bar.shrink(rustui_core::Insets::new(
                pad.left,
                pad.top * 0.5,
                pad.right,
                pad.top * 0.5,
            ));
            if self.closeable {
                area.width = (area.width - title_h).max(0.0);
            }
            let size = ctx.measure_text(&title, &title_style, Some(area.width));
            let position = center_in(area, Size::new(size.width, line_height));
            ctx.painter().text(position, &title, &title_style);
        }

        // Close button.
        if self.closeable {
            let close = self.close_rect(ctx.ui(), bounds);
            let pointer = ctx.interaction().pointer;
            if state.hovered && close.contains(pointer) {
                ctx.painter().rounded_rect(
                    close.inflate(theme.spacing.xs * 0.5),
                    theme.radius.sm,
                    theme.colors.surface_hover,
                );
            }
            let cross = close.shrink(rustui_core::Insets::all(close.width * 0.25));
            let color = if state.enabled {
                theme.colors.text_muted
            } else {
                theme.colors.text_disabled
            };
            let stroke = theme.borders.thin.max(1.5);
            let painter = ctx.painter();
            painter.line(
                Point::new(cross.x, cross.y),
                Point::new(cross.right(), cross.bottom()),
                stroke,
                color,
            );
            painter.line(
                Point::new(cross.right(), cross.y),
                Point::new(cross.x, cross.bottom()),
                stroke,
                color,
            );
        }

        // Content.
        ctx.push_clip(self.base.bounds());
        if let Some(child) = self.content_id() {
            ctx.paint_child(child);
        }
        ctx.pop_clip();

        if state.focused {
            focus_ring(ctx, self.base.bounds(), theme.radius.lg);
        }
    }
}
