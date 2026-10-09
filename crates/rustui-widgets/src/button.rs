//! Push button (design document §46).

use rustui_core::{
    impl_widget_base, ArrangeContext, Color, EdgeRadius, EventContext, EventResult, Fill,
    MeasureContext, MouseButton, NamedKey, PaintContext, Point, Rect, SharedProperty, Signal, Size,
    TextRole, TextStyle, UiContext, UiEvent, Widget, WidgetBase,
};

use crate::common::focus_ring;

/// Visual weight of a button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    /// Filled with the theme's primary color.
    #[default]
    Primary,
    /// Outlined, filled with the theme's surface color.
    Secondary,
    /// No surface and no border: text only.
    Flat,
}

/// A push button.
///
/// The button captures the pointer while it is pressed so that a release outside of its rectangle
/// does not fire a click, emits [`Button::clicked`] on release (or on `Enter` / `Space` while
/// focused) and mirrors its state into [`rustui_core::WidgetState`] for painting.
#[derive(Debug)]
pub struct Button {
    base: WidgetBase,
    text: SharedProperty<String>,
    variant: ButtonVariant,
    /// Pressed by this widget; reset on release.
    pressing: bool,
    clicked: Signal<()>,
    tooltip: Option<String>,
}

impl Button {
    /// A button showing `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self::with_text(SharedProperty::new(text.into()))
    }

    /// A button bound to `text`.
    pub fn with_text(text: SharedProperty<String>) -> Self {
        Self {
            base: WidgetBase::new(),
            text,
            variant: ButtonVariant::default(),
            pressing: false,
            clicked: Signal::new(),
            tooltip: None,
        }
    }

    /// Current label.
    pub fn text(&self) -> String {
        self.text.get()
    }

    /// Replaces the label.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text.set(text.into());
    }

    /// The shared label property (clone it to keep a handle).
    pub fn text_property(&self) -> SharedProperty<String> {
        self.text.clone()
    }

    /// Fires whenever the button is activated.
    pub fn clicked(&self) -> &Signal<()> {
        &self.clicked
    }

    /// Sets the visual variant.
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Sets the tooltip.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    fn text_style(&self, ui: &UiContext, enabled: bool) -> TextStyle {
        let theme = ui.theme();
        let color = match (enabled, self.variant) {
            (false, _) => theme.colors.text_disabled,
            (true, ButtonVariant::Primary) => theme.colors.on_primary,
            (true, _) => theme.colors.text,
        };
        theme.text(TextRole::Button, Some(color))
    }
}

impl_widget_base!(Button);

impl Widget for Button {
    fn kind(&self) -> &'static str {
        "Button"
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn is_opaque(&self) -> bool {
        self.variant != ButtonVariant::Flat
    }

    fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let style = self.text_style(ctx.ui(), self.base.is_enabled());
        let text = self.text.get();
        let text_size = if text.is_empty() {
            Size::new(0.0, ctx.measure_text(" ", &style, None).height)
        } else {
            ctx.measure_text(&text, &style, None)
        };
        let theme = ctx.ui().theme();
        let pad = theme.content_padding();
        let size = Size::new(
            text_size.width + pad.horizontal(),
            (text_size.height + pad.vertical()).max(theme.control_height()),
        );
        let clamped = Size::new(
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
        self.base.set_measured(clamped);
        clamped
    }

    fn arrange(&mut self, _ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
    }

    fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult {
        let id = self.id();
        let enabled = self.base.is_enabled();
        match event {
            UiEvent::MouseDown {
                button: MouseButton::Left,
                ..
            }
            | UiEvent::MouseDoubleClick {
                button: MouseButton::Left,
                ..
            } if enabled => {
                self.pressing = true;
                ctx.capture_pointer(id);
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::MouseUp {
                button: MouseButton::Left,
                position,
            } => {
                let was_pressing = std::mem::take(&mut self.pressing);
                let inside = self.base.window_bounds().contains(*position);
                if ctx.interaction().owns_pointer(id) {
                    ctx.release_pointer();
                }
                if was_pressing && enabled && inside {
                    self.clicked.emit(&());
                }
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::KeyDown { key, .. }
                if enabled
                    && (key.is_named(NamedKey::Return)
                        || key.is_named(NamedKey::Enter)
                        || key.is_named(NamedKey::Space)) =>
            {
                self.clicked.emit(&());
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
        // Everything below is in **window** coordinates: `painter()` takes no offset.
        let bounds = ctx.to_window_rect(self.base.bounds());
        let radius = EdgeRadius::all(theme.radius.sm);
        let colors = if self.variant == ButtonVariant::Primary {
            &theme.button
        } else {
            &theme.surface_button
        };
        let bg = match self.variant {
            ButtonVariant::Flat => Color::TRANSPARENT,
            _ => colors.background(state),
        };
        let mut fill = Fill::rounded(bg, theme.radius.sm);
        if self.variant != ButtonVariant::Flat {
            fill.border = colors.border(state);
        }
        ctx.painter().styled_rect(bounds, radius, fill);

        let text = self.text.get();
        if text.is_empty() {
            if state.focused {
                focus_ring(ctx, self.base.bounds(), theme.radius.sm);
            }
            return;
        }
        let style = self.text_style(ctx.ui(), state.enabled);
        let pad = theme.content_padding();
        let content = Rect::new(
            bounds.x + pad.left,
            bounds.y + pad.top,
            (bounds.width - pad.horizontal()).max(0.0),
            (bounds.height - pad.vertical()).max(0.0),
        );
        let size = ctx.measure_text(&text, &style, None);
        let position = Point::new(
            content.x + ((content.width - size.width) * 0.5).max(0.0),
            content.y + ((content.height - size.height) * 0.5).max(0.0),
        );
        ctx.painter().text(position, &text, &style);

        if state.focused {
            focus_ring(ctx, self.base.bounds(), theme.radius.sm);
        }
    }
}
