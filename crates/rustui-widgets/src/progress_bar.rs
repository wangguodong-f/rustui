//! Progress bar: an indeterminate-free, 0.0 – 1.0 value indicator.

use rustui_core::{
    impl_widget_base, ArrangeContext, EventContext, EventResult, Fill, MeasureContext,
    PaintContext, Rect, SharedProperty, Size, SizeHint, UiEvent, Widget, WidgetBase,
};

use crate::common::focus_ring;

/// A horizontal progress bar.
///
/// The value is a `0.0 … 1.0` fraction kept in a [`SharedProperty`] so the application can drive
/// it from anywhere:
///
/// ```ignore
/// let progress = SharedProperty::new(0.0_f32);
/// let bar = ProgressBar::with_value(progress.clone());
/// progress.set(0.75);
/// ```
#[derive(Debug)]
pub struct ProgressBar {
    base: WidgetBase,
    value: SharedProperty<f32>,
}

impl ProgressBar {
    /// A bar showing `value` (`0.0 … 1.0`, clamped on paint).
    pub fn new(value: f32) -> Self {
        Self::with_value(SharedProperty::new(value))
    }

    /// A bar bound to `value`.
    pub fn with_value(value: SharedProperty<f32>) -> Self {
        Self {
            base: WidgetBase::new(),
            value,
        }
    }

    /// Current fraction (`0.0 … 1.0`).
    pub fn value(&self) -> f32 {
        self.value.get().clamp(0.0, 1.0)
    }

    /// Sets the fraction (clamped to `0.0 … 1.0`).
    pub fn set_value(&mut self, value: f32) {
        self.value.set(value.clamp(0.0, 1.0));
    }

    /// The shared fraction (clone it to keep a handle).
    pub fn value_property(&self) -> SharedProperty<f32> {
        self.value.clone()
    }
}

impl_widget_base!(ProgressBar);

impl Widget for ProgressBar {
    fn kind(&self) -> &'static str {
        "ProgressBar"
    }

    /// The bar takes the width it is offered but keeps its natural thickness.
    fn size_hint(&self) -> SizeHint {
        SizeHint {
            width: f32::INFINITY,
            ..SizeHint::default()
        }
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let theme = ctx.ui().theme();
        let height = theme.spacing.md.max(8.0);
        // A progress bar fills the width it is offered; only with an infinite width does it fall
        // back to a sane default so it can live in a shrink-to-fit container.
        let width = if available.width.is_finite() {
            available.width.max(0.0)
        } else {
            120.0
        };
        let size = Size::new(
            width,
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

    fn event(&mut self, _ctx: &mut EventContext<'_>, _event: &UiEvent) -> EventResult {
        EventResult::Unhandled
    }

    fn paint(&mut self, ctx: &mut PaintContext<'_>) {
        let id = self.id();
        let state = ctx.state_of(id, self.base.state());
        let theme = ctx.ui().theme();
        // Window coordinates: the painter takes no offset.
        let bounds = ctx.to_window_rect(self.base.bounds());
        let radius = theme.radius.pill;

        let track = if state.enabled {
            theme.colors.surface_alt
        } else {
            theme.colors.surface_hover
        };
        ctx.painter().styled_rect(
            bounds,
            rustui_core::EdgeRadius::all(radius),
            Fill::rounded(track, radius),
        );

        let fraction = self.value();
        if fraction > 0.0 {
            let width = (bounds.width - theme.borders.thin * 2.0).max(0.0);
            let fill = Rect::new(
                bounds.x + theme.borders.thin,
                bounds.y,
                width * fraction,
                bounds.height,
            );
            let color = if state.enabled {
                theme.colors.primary
            } else {
                theme.colors.text_disabled
            };
            ctx.painter().styled_rect(
                fill,
                rustui_core::EdgeRadius::all(radius),
                Fill::rounded(color, radius),
            );
        }

        if state.focused {
            focus_ring(ctx, self.base.bounds(), radius);
        }
    }
}
