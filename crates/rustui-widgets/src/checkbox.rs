//! Check box: a binary on / off control with an optional caption.

use rustui_core::{
    impl_widget_base, ArrangeContext, EventContext, EventResult, Fill, MeasureContext, MouseButton,
    NamedKey, PaintContext, Point, Rect, SharedProperty, Signal, Size, TextRole, TextStyle,
    UiContext, UiEvent, Widget, WidgetBase,
};

use crate::common::{center_in, control_radius, focus_ring};

/// A check box with an optional caption.
///
/// ```ignore
/// let box_ = CheckBox::new("Wrap text").checked(true);
/// box_.toggled().connect(|on| println!("wrap = {on}"));
/// ```
#[derive(Debug)]
pub struct CheckBox {
    base: WidgetBase,
    text: Option<SharedProperty<String>>,
    checked: SharedProperty<bool>,
    toggled: Signal<bool>,
    pressing: bool,
    tooltip: Option<String>,
}

impl CheckBox {
    /// A check box with a caption.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: Some(SharedProperty::new(text.into())),
            ..Self::bare()
        }
    }

    /// A check box without a caption (just the box).
    pub fn bare() -> Self {
        Self {
            base: WidgetBase::new(),
            text: None,
            checked: SharedProperty::new(false),
            toggled: Signal::new(),
            pressing: false,
            tooltip: None,
        }
    }

    /// A check box whose caption is bound to `text`.
    pub fn with_text(text: SharedProperty<String>) -> Self {
        Self {
            text: Some(text),
            ..Self::bare()
        }
    }

    /// `true` when the box is checked.
    pub fn is_checked(&self) -> bool {
        self.checked.get()
    }

    /// Checks / unchecks the box without emitting [`CheckBox::toggled`].
    pub fn set_checked(&mut self, checked: bool) {
        self.checked.set(checked);
    }

    /// The shared checked flag (clone it to keep a handle).
    pub fn checked_property(&self) -> SharedProperty<bool> {
        self.checked.clone()
    }

    /// Fires with the new value whenever the user toggles the box.
    pub fn toggled(&self) -> &Signal<bool> {
        &self.toggled
    }

    /// The caption property, when the check box has one.
    pub fn caption_property(&self) -> Option<SharedProperty<String>> {
        self.text.clone()
    }

    /// Sets the tooltip.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    fn text_style(&self, ui: &UiContext, enabled: bool) -> TextStyle {
        let theme = ui.theme();
        let color = if enabled {
            theme.colors.text
        } else {
            theme.colors.text_disabled
        };
        theme.text(TextRole::Body, Some(color))
    }

    /// The square the check mark is drawn in, in window coordinates.
    pub fn box_rect(&self, ui: &UiContext) -> Rect {
        let side = (ui.theme().control_height() - 4.0).max(8.0);
        let bounds = self.base.window_bounds();
        let y = bounds.y + ((bounds.height - side) * 0.5).max(0.0);
        Rect::new(bounds.x, y, side, side)
    }
}

impl_widget_base!(CheckBox);

impl Widget for CheckBox {
    fn kind(&self) -> &'static str {
        "CheckBox"
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let theme = ctx.ui().theme();
        let side = (theme.control_height() - 4.0).max(8.0);
        let mut width = side;
        let mut height = side;
        if let Some(text) = &self.text {
            let text = text.get();
            let style = self.text_style(ctx.ui(), self.base.is_enabled());
            let text_size = if text.is_empty() {
                ctx.measure_text(" ", &style, None)
            } else {
                ctx.measure_text(&text, &style, None)
            };
            width += theme.spacing.sm + text_size.width;
            height = height.max(text_size.height);
        }
        height = height.max(theme.control_height());
        let size = Size::new(
            if available.width.is_finite() {
                width.min(available.width.max(0.0))
            } else {
                width
            },
            if available.height.is_finite() {
                height.min(available.height.max(0.0))
            } else {
                height
            },
        );
        self.base.set_measured(size);
        size
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
                    let value = !self.checked.get();
                    self.checked.set(value);
                    self.toggled.emit(&value);
                }
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::KeyDown { key, .. }
                if enabled && (key.is_named(NamedKey::Space) || key.is_named(NamedKey::Return)) =>
            {
                let value = !self.checked.get();
                self.checked.set(value);
                self.toggled.emit(&value);
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
        // Window coordinates: the painter takes no offset.
        let bounds = ctx.to_window_rect(self.base.bounds());
        let radius = control_radius(ctx);
        let side = (theme.control_height() - 4.0).max(8.0);
        let y = bounds.y + ((bounds.height - side) * 0.5).max(0.0);
        let box_rect = Rect::new(bounds.x, y, side, side);

        let bg = if !state.enabled {
            theme.colors.surface_alt
        } else if self.checked.get() {
            theme.colors.primary
        } else if state.pressed || state.hovered {
            theme.colors.surface_hover
        } else {
            theme.colors.surface
        };
        let border_color = if state.focused {
            theme.colors.focus_ring
        } else {
            theme.colors.border
        };
        let border = rustui_core::Border {
            widths: rustui_core::EdgeWidths::all(theme.borders.thin),
            color: border_color,
        };
        ctx.painter().styled_rect(
            box_rect,
            radius,
            Fill::rounded(bg, theme.radius.sm).with_border(border),
        );

        if self.checked.get() {
            // Check mark: two strokes inside the box.
            let inset = side * 0.24;
            let r = box_rect.shrink(rustui_core::Insets::all(inset));
            let stroke = theme.borders.thin.max(2.0);
            let color = if state.enabled {
                theme.colors.on_primary
            } else {
                theme.colors.text_disabled
            };
            let painter = ctx.painter();
            painter.line(
                Point::new(r.x, r.y + r.height * 0.55),
                Point::new(r.x + r.width * 0.4, r.y + r.height),
                stroke,
                color,
            );
            painter.line(
                Point::new(r.x + r.width * 0.4, r.y + r.height),
                Point::new(r.x + r.width, r.y),
                stroke,
                color,
            );
        }

        if let Some(text) = &self.text {
            let text = text.get();
            if !text.is_empty() {
                let mut style = self.text_style(ctx.ui(), state.enabled);
                style.align = rustui_core::TextAlign::Left;
                let text_size = ctx.measure_text(&text, &style, None);
                let origin = center_in(
                    Rect::new(
                        box_rect.right() + theme.spacing.sm,
                        bounds.y,
                        (bounds.width - box_rect.width - theme.spacing.sm).max(0.0),
                        bounds.height,
                    ),
                    text_size,
                );
                ctx.painter().text(origin, &text, &style);
            }
        }

        if state.focused {
            focus_ring(ctx, self.base.bounds(), theme.radius.sm);
        }
    }
}
