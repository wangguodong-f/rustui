//! [`VBox`] / [`HBox`] — one dimensional stacks — plus [`Spacer`] and [`Separator`].

use std::collections::HashMap;

use rustui_core::impl_widget_base;
use rustui_core::widget::{align_child, distribute};
use rustui_core::{
    Align, AlignSelf, ArrangeContext, CrossAxisAlign, EventContext, EventResult, Insets, Layout,
    MainAxisAlign, MainAxisSize, MeasureContext, ObjectId, PaintContext, Rect, Size, SizeHint,
    UiEvent, Widget, WidgetBase,
};

use crate::style::{
    apply_fill, clamp_to, content_available, content_rect, expands, hint_of, measured_size,
    paint_children, visible_children, visible_children_in, Axis, ContainerStyle,
};

/// Children arrangement options shared by [`VBox`] and [`HBox`].
///
/// The struct only holds configuration; identity, children and bounds live in the
/// [`WidgetBase`] of the two widgets that use it.
#[derive(Clone, Debug)]
pub struct BoxLayout {
    /// Gap between two consecutive children, along the main axis.
    pub spacing: f32,
    /// Space between the container border and its content.
    pub padding: Insets,
    /// How free space is distributed when no child wants to expand.
    pub main_align: MainAxisAlign,
    /// Cross-axis alignment of every child without an override.
    pub cross_align: CrossAxisAlign,
    /// Whether the box hugs its content or takes the space it is offered on the main axis.
    pub main_axis_size: MainAxisSize,
    /// Background / border / clipping.
    pub style: ContainerStyle,
    /// Size hint reported by the container itself.
    pub hint: SizeHint,
    /// Per child cross-axis overrides.
    align_self: HashMap<ObjectId, AlignSelf>,
    /// Per child main-axis weights; fall back to `SizeHint::flex`.
    flex: HashMap<ObjectId, f32>,
}

impl Default for BoxLayout {
    fn default() -> Self {
        Self {
            spacing: 0.0,
            padding: Insets::ZERO,
            main_align: MainAxisAlign::Start,
            cross_align: CrossAxisAlign::Start,
            main_axis_size: MainAxisSize::Auto,
            style: ContainerStyle::EMPTY,
            hint: SizeHint::default(),
            align_self: HashMap::new(),
            flex: HashMap::new(),
        }
    }
}

/// Maps a per child override plus the container default onto an [`Align`].
fn resolve_align(align_self: AlignSelf, container: CrossAxisAlign) -> Align {
    match align_self {
        AlignSelf::Auto => match container {
            CrossAxisAlign::Start => Align::Start,
            CrossAxisAlign::Center => Align::Center,
            CrossAxisAlign::End => Align::End,
            CrossAxisAlign::Stretch => Align::Stretch,
        },
        AlignSelf::Start => Align::Start,
        AlignSelf::Center => Align::Center,
        AlignSelf::End => Align::End,
        AlignSelf::Stretch => Align::Stretch,
    }
}

impl BoxLayout {
    /// Overrides the cross-axis alignment of one child.
    pub fn set_align_of(&mut self, child: ObjectId, align: AlignSelf) {
        self.align_self.insert(child, align);
    }

    /// Overrides the main-axis weight of one child (`0` removes a previous override).
    pub fn set_flex_of(&mut self, child: ObjectId, weight: f32) {
        if weight > 0.0 {
            self.flex.insert(child, weight);
        } else {
            self.flex.remove(&child);
        }
    }

    fn align_of(&self, child: ObjectId) -> Align {
        resolve_align(
            self.align_self
                .get(&child)
                .copied()
                .unwrap_or(AlignSelf::Auto),
            self.cross_align,
        )
    }

    fn flex_of(&self, child: ObjectId, hint: &SizeHint) -> f32 {
        self.flex.get(&child).copied().unwrap_or(hint.flex).max(0.0)
    }

    /// Measures `children` along `axis`.
    ///
    /// Each child sees the space still free on the main axis (so a scrolling child at the end
    /// of a `VBox` learns how tall it may become) and the full cross extent.
    pub fn measure(
        &self,
        axis: Axis,
        children: &[ObjectId],
        ctx: &MeasureContext<'_>,
        available: Size,
    ) -> Size {
        let children = visible_children_in(ctx, children);
        let content = content_available(available, self.padding);
        let content_main = axis.main(content);
        let content_cross = axis.cross(content);
        let count = children.len();

        // Rigid children first, flexible ones after them. A child that takes whatever it is
        // offered (a text field, a scroll area, anything with a flex weight) would otherwise
        // consume the whole axis and leave its siblings a measured size of zero.
        let mut order: Vec<ObjectId> = Vec::with_capacity(count);
        let mut weights: Vec<f32> = Vec::new();
        let mut total_weight = 0.0f32;
        for &id in &children {
            let hint = hint_of(ctx.store(), id);
            // Rigid: no flex weight and no infinite preference along this axis.
            if self.flex_of(id, &hint) <= 0.0 && !expands(&hint, axis) {
                order.push(id);
            }
        }
        let flexible_start = order.len();
        for &id in &children {
            let hint = hint_of(ctx.store(), id);
            let weight = self.flex_of(id, &hint);
            if weight <= 0.0 && !expands(&hint, axis) {
                continue;
            }
            let weight = if weight > 0.0 { weight } else { 1.0 };
            weights.push(weight);
            total_weight += weight;
            order.push(id);
        }

        let mut main_used = 0.0f32;
        let mut cross_used = 0.0f32;
        // Space the flexible group still has to split; only meaningful on a finite axis.
        let mut flex_pool = f32::INFINITY;
        let mut weight_left = total_weight;
        for (index, id) in order.iter().enumerate() {
            let remaining = if content_main.is_finite() {
                (content_main - main_used).max(0.0)
            } else {
                f32::INFINITY
            };
            if index == flexible_start {
                flex_pool = remaining;
            }
            // Each flexible child is offered its share of the pool so that two flex items in the
            // same row do not starve one another during measurement.
            let available_main = if index >= flexible_start && flex_pool.is_finite() {
                if weight_left > f32::EPSILON {
                    flex_pool * weights[index - flexible_start] / weight_left
                } else {
                    remaining
                }
            } else {
                remaining
            };
            let size = ctx.measure_child(*id, axis.size(available_main, content_cross));
            main_used += axis.main(size);
            if index + 1 < count {
                main_used += self.spacing;
            }
            cross_used = cross_used.max(axis.cross(size));
            if index >= flexible_start {
                if flex_pool.is_finite() {
                    flex_pool = (flex_pool - axis.main(size)).max(0.0);
                }
                weight_left -= weights[index - flexible_start];
            }
        }
        let mut size = axis.size(main_used, cross_used);
        // `MainAxisSize::Fill` asks for the whole main axis instead of hugging the content.
        if matches!(self.main_axis_size, MainAxisSize::Fill) && axis.main(available).is_finite() {
            size = axis.size(axis.main(available), axis.cross(size));
        }
        apply_fill(
            clamp_to(size.expand(self.padding), available),
            available,
            self.hint,
        )
    }

    /// Arranges `children` along `axis` inside the local rect `rect`.
    pub fn arrange(&self, axis: Axis, children: &[ObjectId], ctx: &ArrangeContext<'_>, rect: Rect) {
        let children = visible_children(ctx.store(), children);
        let content = content_rect(rect, self.padding);
        if children.is_empty() {
            return;
        }

        let sizes: Vec<Size> = children.iter().map(|id| measured_size(ctx, *id)).collect();
        let mut mains: Vec<f32> = sizes.iter().map(|s| axis.main(*s)).collect();
        let gaps = self.spacing * (children.len() - 1) as f32;
        let content_main = axis.main(content.size());
        let used: f32 = mains.iter().sum::<f32>() + gaps;
        let mut free = (content_main - used).max(0.0);

        // 1. Flexible children take everything the rigid ones leave, split by weight. Their
        //    measured size only decides the cross axis: during measurement a flex item is offered
        //    its share, but the authoritative split happens here where the whole row is known.
        let mut candidates: Vec<usize> = Vec::new();
        let mut weights: Vec<f32> = Vec::new();
        for (index, id) in children.iter().enumerate() {
            let hint = hint_of(ctx.store(), *id);
            let weight = self.flex_of(*id, &hint);
            if weight > 0.0 || expands(&hint, axis) {
                candidates.push(index);
                weights.push(if weight > 0.0 { weight } else { 1.0 });
            }
        }
        if !candidates.is_empty() {
            let rigid_used: f32 = (0..mains.len())
                .filter(|index| !candidates.contains(index))
                .map(|index| mains[index])
                .sum::<f32>()
                + gaps;
            let pool = (content_main - rigid_used).max(0.0);
            let shares = distribute(pool, &weights);
            for (slot, index) in candidates.into_iter().enumerate() {
                mains[index] = shares[slot];
            }
            free = 0.0;
        }

        // 2. Whatever is still free is positioned by `main_align`.
        let count = children.len();
        let (start_offset, gap_extra) = match self.main_align {
            MainAxisAlign::Start => (0.0, 0.0),
            MainAxisAlign::Center => (free * 0.5, 0.0),
            MainAxisAlign::End => (free, 0.0),
            MainAxisAlign::SpaceBetween => (
                0.0,
                if count > 1 {
                    free / (count - 1) as f32
                } else {
                    0.0
                },
            ),
            MainAxisAlign::SpaceAround => (free / (count as f32 * 2.0), free / count as f32),
            MainAxisAlign::SpaceEvenly => {
                (free / (count as f32 + 1.0), free / (count as f32 + 1.0))
            }
        };

        let mut cursor = start_offset
            + match axis {
                Axis::Vertical => content.top(),
                Axis::Horizontal => content.left(),
            };
        for (index, id) in children.iter().enumerate() {
            let main = mains[index];
            let cell = match axis {
                Axis::Vertical => Rect::new(content.left(), cursor, content.width, main),
                Axis::Horizontal => Rect::new(cursor, content.top(), main, content.height),
            };
            let natural = axis.size(main, axis.cross(sizes[index]));
            let (h, v) = match axis {
                Axis::Vertical => (self.align_of(*id), Align::Start),
                Axis::Horizontal => (Align::Start, self.align_of(*id)),
            };
            ctx.arrange_child(*id, align_child(cell, natural, h, v));
            cursor += main + self.spacing + gap_extra;
        }
    }

    /// Paints the container decoration and its children.
    pub fn paint(&self, children: &[ObjectId], ctx: &mut PaintContext<'_>, bounds: Rect) {
        self.style.paint_begin(ctx, bounds);
        paint_children(ctx, children);
        self.style.paint_end(ctx, bounds);
    }
}

/// Generates the widget definition shared by the two boxes.
macro_rules! define_box {
    ($ty:ident, $axis:expr, $name:literal) => {
        /// A one dimensional stack of children (design document §15).
        #[derive(Debug)]
        pub struct $ty {
            base: WidgetBase,
            layout: BoxLayout,
        }

        impl $ty {
            /// Creates an empty box.
            pub fn new() -> Self {
                Self {
                    base: WidgetBase::new(),
                    layout: BoxLayout::default(),
                }
            }

            /// Adds a child (which must already be registered in the widget store).
            pub fn with_child(mut self, id: ObjectId) -> Self {
                self.base.add_child(id);
                self
            }

            /// Gap between children.
            pub fn with_spacing(mut self, spacing: f32) -> Self {
                self.layout.spacing = spacing;
                self
            }

            /// Padding around the content.
            pub fn with_padding(mut self, padding: Insets) -> Self {
                self.layout.padding = padding;
                self
            }

            /// Distribution of free space when no child expands.
            pub fn with_main_align(mut self, align: MainAxisAlign) -> Self {
                self.layout.main_align = align;
                self
            }

            /// Cross-axis alignment of the children.
            pub fn with_cross_align(mut self, align: CrossAxisAlign) -> Self {
                self.layout.cross_align = align;
                self
            }

            /// Hug the content (`Auto`) or take the offered space (`Fill`).
            pub fn with_main_axis_size(mut self, size: MainAxisSize) -> Self {
                self.layout.main_axis_size = size;
                self
            }

            /// Background / border / clipping of the box.
            pub fn with_style(mut self, style: ContainerStyle) -> Self {
                self.layout.style = style;
                self
            }

            /// Convenience: a background color.
            pub fn with_background(mut self, color: rustui_core::Color) -> Self {
                self.layout.style.background = color;
                self
            }

            /// The box takes the space its parent offers instead of hugging its children.
            pub fn with_fill(mut self) -> Self {
                self.layout.hint = SizeHint::fill();
                self.layout.main_axis_size = MainAxisSize::Fill;
                self
            }

            /// Appends a child.
            pub fn push(&mut self, id: ObjectId) -> &mut Self {
                self.base.add_child(id);
                self
            }

            /// Shared configuration.
            pub fn layout(&self) -> &BoxLayout {
                &self.layout
            }

            /// Shared configuration, mutable.
            pub fn layout_mut(&mut self) -> &mut BoxLayout {
                &mut self.layout
            }
        }

        impl Default for $ty {
            fn default() -> Self {
                Self::new()
            }
        }

        impl_widget_base!($ty);
        impl Layout for $ty {}

        impl Widget for $ty {
            fn kind(&self) -> &'static str {
                $name
            }

            fn size_hint(&self) -> SizeHint {
                self.layout.hint
            }

            fn is_opaque(&self) -> bool {
                self.layout.style.background.a > 0.0
            }

            fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
                let size = self
                    .layout
                    .measure($axis, self.base.children(), ctx, available);
                self.base.set_measured(size);
                size
            }

            fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
                self.base.set_bounds(rect);
                self.layout.arrange($axis, self.base.children(), ctx, rect);
            }

            fn event(&mut self, _ctx: &mut EventContext<'_>, _event: &UiEvent) -> EventResult {
                EventResult::Unhandled
            }

            fn paint(&mut self, ctx: &mut PaintContext<'_>) {
                let bounds = self.base.bounds();
                let children = self.base.children().to_vec();
                self.layout.paint(&children, ctx, bounds);
            }
        }
    };
}

define_box!(VBox, Axis::Vertical, "VBox");
define_box!(HBox, Axis::Horizontal, "HBox");

/// Invisible free space; absorbs leftover room in a [`VBox`] / [`HBox`] proportionally to its
/// `flex` weight.
///
/// ```text
/// [Button] [Spacer flex=1] [Button]   ->   [Button]      [Button]
/// ```
#[derive(Debug)]
pub struct Spacer {
    base: WidgetBase,
    weight: f32,
    fixed: Option<Size>,
}

impl Spacer {
    /// A flexible spacer with the given weight (1.0 by default).
    pub fn new(weight: f32) -> Self {
        Self {
            base: WidgetBase::new(),
            weight: if weight > 0.0 { weight } else { 1.0 },
            fixed: None,
        }
    }

    /// A spacer with a fixed size that never absorbs free space.
    pub fn fixed(size: Size) -> Self {
        Self {
            base: WidgetBase::new(),
            weight: 0.0,
            fixed: Some(size),
        }
    }

    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = if weight > 0.0 { weight } else { 1.0 };
        self
    }
}

impl_widget_base!(Spacer);
impl Layout for Spacer {}

impl Widget for Spacer {
    fn kind(&self) -> &'static str {
        "Spacer"
    }

    fn size_hint(&self) -> SizeHint {
        match self.fixed {
            Some(size) => SizeHint::fixed(size.width, size.height),
            None => SizeHint {
                flex: self.weight,
                ..SizeHint::fill()
            },
        }
    }

    fn measure(&mut self, _ctx: &MeasureContext<'_>, _available: Size) -> Size {
        let size = self.fixed.unwrap_or(Size::ZERO);
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, _ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
    }

    fn event(&mut self, _ctx: &mut EventContext<'_>, _event: &UiEvent) -> EventResult {
        EventResult::Unhandled
    }

    fn paint(&mut self, _ctx: &mut PaintContext<'_>) {}
}

/// A thin line separating two groups of children.
///
/// [`Separator::horizontal`] belongs in a [`VBox`], [`Separator::vertical`] in an [`HBox`].
#[derive(Debug)]
pub struct Separator {
    base: WidgetBase,
    axis: Axis,
    thickness: f32,
    color: rustui_core::Color,
}

impl Separator {
    /// A rule spanning the horizontal axis (used inside a [`VBox`]).
    pub fn horizontal() -> Self {
        Self::new(Axis::Horizontal, 1.0)
    }

    /// A rule spanning the vertical axis (used inside an [`HBox`]).
    pub fn vertical() -> Self {
        Self::new(Axis::Vertical, 1.0)
    }

    fn new(axis: Axis, thickness: f32) -> Self {
        Self {
            base: WidgetBase::new(),
            axis,
            thickness,
            color: rustui_core::Color::TRANSPARENT,
        }
    }

    pub fn with_thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness.max(0.0);
        self
    }

    pub fn with_color(mut self, color: rustui_core::Color) -> Self {
        self.color = color;
        self
    }
}

impl_widget_base!(Separator);
impl Layout for Separator {}

impl Widget for Separator {
    fn kind(&self) -> &'static str {
        "Separator"
    }

    fn size_hint(&self) -> SizeHint {
        // The cross axis expands (the container decides how long the rule is); the main axis is
        // fixed to the thickness so free space is never handed to a rule.
        match self.axis {
            Axis::Horizontal => SizeHint {
                width: f32::INFINITY,
                height: self.thickness,
                ..SizeHint::default()
            },
            Axis::Vertical => SizeHint {
                width: self.thickness,
                height: f32::INFINITY,
                ..SizeHint::default()
            },
        }
    }

    fn measure(&mut self, _ctx: &MeasureContext<'_>, available: Size) -> Size {
        let size = match self.axis {
            Axis::Horizontal => Size::new(
                if available.width.is_finite() {
                    available.width
                } else {
                    0.0
                },
                self.thickness,
            ),
            Axis::Vertical => Size::new(
                self.thickness,
                if available.height.is_finite() {
                    available.height
                } else {
                    0.0
                },
            ),
        };
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
        let bounds = self.base.bounds();
        let color = if self.color.a > 0.0 {
            self.color
        } else {
            rustui_core::Color::gray8(0xdd)
        };
        let line = match self.axis {
            Axis::Horizontal => Rect::new(0.0, 0.0, bounds.width, self.thickness),
            Axis::Vertical => Rect::new(0.0, 0.0, self.thickness, bounds.height),
        };
        ctx.fill_rect(line, color);
    }
}
