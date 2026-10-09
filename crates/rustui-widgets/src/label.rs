//! A static / bound piece of text (design document §47).

use rustui_core::{
    impl_widget_base, ArrangeContext, Color, EventContext, EventResult, MeasureContext,
    PaintContext, Point, Rect, SharedProperty, Signal, Size, TextAlign, TextRole, TextStyle,
    Transform, UiContext, UiEvent, Widget, WidgetBase,
};

/// A label: the simplest widget, a piece of text with a role and an alignment.
///
/// The text lives in a [`SharedProperty`] so the application can keep a handle and update the
/// label at any time — no downcasting and no `Rc<RefCell<Widget>>` (§52).
///
/// ```ignore
/// let status = SharedProperty::new(String::from("Ready"));
/// let label = Label::with_text(status.clone()).role(TextRole::Caption);
/// status.set("Saved".into());
/// ```
#[derive(Debug)]
pub struct Label {
    base: WidgetBase,
    text: SharedProperty<String>,
    role: TextRole,
    color: Option<Color>,
    align: TextAlign,
    wrap: bool,
    /// Rotation of the text around its own ink box centre, in degrees (T-8 / C-049).
    rotation: f32,
    tooltip: Option<String>,
}

impl Label {
    /// A label showing `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self::with_text(SharedProperty::new(text.into()))
    }

    /// A label bound to `text`; the caller keeps a clone of the property to update it later.
    pub fn with_text(text: SharedProperty<String>) -> Self {
        Self {
            base: WidgetBase::new(),
            text,
            role: TextRole::Body,
            color: None,
            align: TextAlign::Left,
            wrap: false,
            rotation: 0.0,
            tooltip: None,
        }
    }

    /// The shared text property (clone it to keep a handle).
    pub fn text_property(&self) -> SharedProperty<String> {
        self.text.clone()
    }

    /// Current text.
    pub fn text(&self) -> String {
        self.text.get()
    }

    /// Replaces the text.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text.set(text.into());
    }

    /// Notifies observers whenever the text changes.
    pub fn changed(&self) -> &Signal<String> {
        self.text.changed()
    }

    /// Sets the typographic role (body / caption / title / heading / ...).
    pub fn role(mut self, role: TextRole) -> Self {
        self.role = role;
        self
    }

    /// Overrides the role's color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Sets the horizontal alignment inside the assigned rectangle.
    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// Wraps the text at the assigned width instead of letting it overflow.
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// Rotates the text around the centre of its own ink box, clockwise in degrees (T-8 / C-049).
    ///
    /// Measure and layout are unaffected: the label still occupies the same box, only the drawn
    /// glyphs turn. Works together with [`Label::wrap`] - the rotation applies to the whole block.
    pub fn rotation(mut self, degrees: f32) -> Self {
        self.rotation = degrees;
        self
    }

    /// Sets the tooltip shown while the pointer rests on the label.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    fn text_style(&self, ui: &UiContext) -> TextStyle {
        let mut style = ui.theme().text(self.role, self.color);
        style.align = self.align;
        style
    }
}

impl_widget_base!(Label);

impl Widget for Label {
    fn kind(&self) -> &'static str {
        "Label"
    }

    fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let text = self.text.get();
        let style = self.text_style(ctx.ui());
        // A wrapping label fills the width it is offered; a single line label keeps its natural
        // width. An empty label still needs one line of height so rows stay aligned.
        let rotated = self.rotation != 0.0;
        let mut size = if text.is_empty() {
            Size::new(0.0, ctx.measure_text(" ", &style, None).height)
        } else if self.wrap && rotated {
            // Wrapping narrower makes the block taller, which widens the turned extent again:
            // search for a wrap width whose turned block still fits the offered width.
            let wrap = fitting_wrap_width(
                |max| ctx.measure_text(&text, &style, max),
                available.width,
                self.rotation,
            );
            ctx.measure_text(&text, &style, Some(wrap))
        } else {
            ctx.measure_text(
                &text,
                &style,
                if self.wrap {
                    Some(available.width)
                } else {
                    None
                },
            )
        };
        // Children are clipped to their own box (`PaintContext::paint_child`), so a turned block
        // must *reserve* its axis aligned extent or its corners get cut off (T-8 / C-049).
        if rotated {
            size = rotated_extent(size, self.rotation);
        }
        // Never ask for more than we were given: the parent may be tighter than the text.
        if available.width.is_finite() {
            size.width = size.width.min(available.width.max(0.0));
        }
        if available.height.is_finite() {
            size.height = size.height.min(available.height.max(0.0));
        }
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
        let text = self.text.get();
        if text.is_empty() {
            return;
        }
        // `painter()` takes window coordinates (no offset is applied), while the helpers on
        // `ctx` take local ones.
        let bounds = ctx.to_window_rect(self.base.bounds());
        let mut style = self.text_style(ctx.ui());
        if !self.base.is_enabled() {
            style.color = ctx.ui().theme().colors.text_disabled;
        }
        let wrapped = self.wrap;
        let rotated = self.rotation != 0.0;
        // The block is built from the same wrap width `measure` reserved room for; turning it
        // means that width is narrower than the box, which is exactly what still fits when turned.
        let wrap_width = if !wrapped {
            None
        } else if rotated {
            Some(fitting_wrap_width(
                |max| ctx.measure_text(&text, &style, max),
                bounds.width,
                self.rotation,
            ))
        } else {
            Some(bounds.width)
        };
        let size = ctx.measure_text(&text, &style, wrap_width);
        // The renderer turns the run around the centre of its ink box, which sits at the centre of
        // the block: centre the block and the turned text lands in the middle of the box it
        // reserved. A turned block may stick out of the box *before* turning - that is the point.
        let (x, y) = if rotated {
            (
                (bounds.width - size.width) * 0.5,
                (bounds.height - size.height) * 0.5,
            )
        } else {
            // Single line labels sit vertically centred; wrapped blocks grow downwards.
            let y = ((bounds.height - size.height) * 0.5).max(0.0);
            // With `wrap` the engine aligns each line inside `bounds.width`; without it the engine
            // aligns inside the natural block width, so the block itself is placed here.
            let x = if wrapped {
                0.0
            } else {
                match self.align {
                    TextAlign::Left => 0.0,
                    TextAlign::Center => ((bounds.width - size.width) * 0.5).max(0.0),
                    TextAlign::Right => (bounds.width - size.width).max(0.0),
                }
            };
            (x, y)
        };
        let position = Point::new(bounds.x + x, bounds.y + y);
        // One entry point for both plain and rotated text: `Transform::rotated(0.0)` is the
        // identity, so a normal label records exactly the command it always did.
        ctx.painter().text_transformed(
            position,
            &text,
            &style,
            wrap_width,
            Transform::rotated(self.rotation),
        );
    }
}

/// Axis aligned extent of a `size` block turned around its centre by `degrees`.
fn rotated_extent(size: Size, degrees: f32) -> Size {
    let (sin, cos) = degrees.to_radians().sin_cos();
    Size::new(
        size.width * cos.abs() + size.height * sin.abs(),
        size.width * sin.abs() + size.height * cos.abs(),
    )
}

/// The widest wrap width whose turned block still fits `limit` horizontally (T-8 / C-049).
///
/// Wrapping narrower makes the block taller, which widens the turned extent again, so there is no
/// closed form: step the width down proportionally until the turned block fits. When nothing fits
/// (a very narrow box around a tall block) fall back to `limit`, i.e. the unrotated behaviour,
/// and let the corners clip like any other overflowing content.
fn fitting_wrap_width(
    mut measure: impl FnMut(Option<f32>) -> Size,
    limit: f32,
    degrees: f32,
) -> f32 {
    if degrees == 0.0 || !limit.is_finite() || limit <= 0.0 {
        return limit;
    }
    let mut width = limit;
    for _ in 0..12 {
        let turned = rotated_extent(measure(Some(width)), degrees).width;
        if turned <= limit + 0.5 {
            return width;
        }
        let next = (width * limit / turned).max(1.0);
        if next >= width {
            break;
        }
        width = next;
    }
    limit
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustui_core::{Interaction, WidgetStore};

    struct StubText;

    impl rustui_core::TextMeasurer for StubText {
        fn layout_text(
            &self,
            text: &str,
            _style: &TextStyle,
            _max: Option<f32>,
        ) -> rustui_core::TextLayout {
            rustui_core::TextLayout::single_line(
                Size::new(text.len() as f32 * 7.0, 14.0),
                10.0,
                4.0,
            )
        }

        fn line_height(&self, _style: &TextStyle) -> f32 {
            14.0
        }
    }

    fn measured(label: &mut Label, available: Size) -> Size {
        let ui = UiContext::new(Box::new(StubText));
        let store = WidgetStore::new();
        let interaction = Interaction::default();
        let ctx = MeasureContext::new(&ui, &store, &interaction, 0);
        label.measure(&ctx, available)
    }

    #[test]
    fn a_label_measures_its_text_and_clamps_to_the_offered_space() {
        let mut label = Label::new("Hello");
        assert_eq!(
            measured(&mut label, Size::new(1000.0, 1000.0)),
            Size::new(35.0, 14.0)
        );
        assert_eq!(
            measured(&mut label, Size::new(20.0, 1000.0)),
            Size::new(20.0, 14.0)
        );
    }

    #[test]
    fn an_empty_label_still_occupies_one_line() {
        let mut label = Label::new("");
        assert_eq!(
            measured(&mut label, Size::new(100.0, 100.0)),
            Size::new(0.0, 14.0)
        );
    }

    #[test]
    fn rotated_extent_swaps_the_axes_at_a_quarter_turn() {
        let quarter = rotated_extent(Size::new(30.0, 10.0), 90.0);
        assert!((quarter.width - 10.0).abs() < 0.01, "{quarter:?}");
        assert!((quarter.height - 30.0).abs() < 0.01, "{quarter:?}");
        let half = rotated_extent(Size::new(30.0, 10.0), 180.0);
        assert!((half.width - 30.0).abs() < 0.01, "{half:?}");
        assert!((half.height - 10.0).abs() < 0.01, "{half:?}");
    }

    /// A turned block has to *reserve* its turned extent, or `paint_child` clips its corners.
    #[test]
    fn a_rotated_label_measures_the_turned_extent_and_still_clamps() {
        let turned = measured(
            &mut Label::new("Hello").rotation(90.0),
            Size::new(1000.0, 1000.0),
        );
        // The stub measures "Hello" as 35 x 14; a quarter turn swaps the axes.
        assert!((turned.width - 14.0).abs() < 0.01, "{turned:?}");
        assert!((turned.height - 35.0).abs() < 0.01, "{turned:?}");

        let clamped = measured(
            &mut Label::new("a very long label indeed").rotation(45.0),
            Size::new(30.0, 1000.0),
        );
        assert_eq!(clamped.width, 30.0, "rotation never beats the parent");
    }

    #[test]
    fn fitting_wrap_width_shrinks_until_the_turned_block_fits() {
        // Content 100 wide re wrapping into 20px lines: the turned block must fit into 64px.
        let measure = |max: Option<f32>| {
            let limit = max.unwrap_or(f32::INFINITY);
            let lines = (100.0 / limit).ceil().max(1.0);
            Size::new(limit.min(100.0), 20.0 * lines)
        };
        let wrap = fitting_wrap_width(measure, 64.0, 45.0);
        assert!(wrap < 64.0, "no shrink means no fit (wrap {wrap})");
        let turned = rotated_extent(measure(Some(wrap)), 45.0);
        assert!(
            turned.width <= 64.5,
            "turned {turned:?} exceeds the limit at wrap {wrap}"
        );
    }

    #[test]
    fn fitting_wrap_width_gives_up_rather_than_wrapping_to_nothing() {
        // 80px of space around a block whose turned extent is at least ~92px however it wraps.
        let measure = |max: Option<f32>| {
            let limit = max.unwrap_or(f32::INFINITY);
            let lines = (100.0 / limit).ceil().max(1.0);
            Size::new(limit.min(100.0), 40.0 * lines)
        };
        assert_eq!(fitting_wrap_width(measure, 80.0, 45.0), 80.0);
        // Degenerate inputs pass through untouched: nothing to fit for an unrotated or an
        // unlimited block.
        let constant = |max: Option<f32>| Size::new(max.unwrap_or(1.0), 1.0);
        assert_eq!(fitting_wrap_width(constant, 60.0, 0.0), 60.0);
        assert_eq!(
            fitting_wrap_width(constant, f32::INFINITY, 45.0),
            f32::INFINITY
        );
    }

    #[test]
    fn the_shared_property_drives_the_text() {
        let text = SharedProperty::new(String::from("before"));
        let label = Label::with_text(text.clone());
        assert_eq!(label.text(), "before");
        text.set(String::from("after"));
        assert_eq!(label.text(), "after");
    }

    #[test]
    fn arrange_pins_the_local_bounds_to_the_origin() {
        let mut label = Label::new("x");
        let ui = UiContext::new(Box::new(StubText));
        let store = WidgetStore::new();
        let interaction = Interaction::default();
        let ctx = ArrangeContext::new(&ui, &store, &interaction, 0);
        label.arrange(&ctx, Rect::new(4.0, 5.0, 40.0, 14.0));
        assert_eq!(label.base.bounds(), Rect::new(0.0, 0.0, 40.0, 14.0));
    }
}
