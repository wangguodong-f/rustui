//! CPU side tessellation: [`PaintCommand`]s become instanced quads (§20).
//!
//! The output mirrors the vertex input of `shader.wgs`, so this whole module can be unit tested
//! without a GPU. Everything is converted to *physical* pixels here, which keeps the vertex shader
//! free of transforms besides the viewport size.
//!
//! One quad = one instance; consecutive instances that share a texture and a clip rectangle are
//! grouped into a [`Batch`], so a frame costs one draw call per run instead of one per widget.

use bytemuck::{Pod, Zeroable};

use rustui_core::paint::GradientStop;
use rustui_core::{
    Border, Color, EdgeRadius, Fill, GlyphSource, Gradient, PaintCommand, Point, Rect, TextArena,
    TextRunId, TextStyle, TextureId, Transform,
};

/// The texture holds coverage (white / glyph atlas): `alpha = color.a * tex.r`.
pub const MODE_COVERAGE: f32 = 1.0;
/// The texture holds RGBA color: `alpha = color.a * tex.a`, `rgb = color.rgb * tex.rgb * alpha`.
pub const MODE_COLOR: f32 = 0.0;

/// Texture slot of the 1x1 white texture used for every untextured quad.
pub const SLOT_WHITE: u32 = 0;
/// Texture slot of the glyph atlas.
pub const SLOT_GLYPH_ATLAS: u32 = 1;

/// One instanced quad. The layout matches `Instance` in `shader.wgs` (176 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Instance {
    /// Quad corners in physical pixels: top-left, top-right, bottom-left, bottom-right.
    pub corners: [[f32; 2]; 4],
    /// Normalised texture coordinates (`u0, v0, u1, v1`).
    pub uv: [f32; 4],
    /// Straight (non premultiplied) RGBA color of each corner; gradients interpolate linearly.
    pub colors: [[f32; 4]; 4],
    /// Border inset in physical pixels (`left, top, right, bottom`); a non zero inset turns the
    /// quad into a rounded ring.
    pub inset: [f32; 4],
    /// `mode`, rounded clip radius, reserved, reserved.
    pub params: [f32; 4],
    /// Per corner radius in physical pixels, quadrant order (`top_left, top_right, bottom_left,
    /// bottom_right`) so the shader picks the radius of the quadrant a fragment sits in (T-6 /
    /// C-052).
    pub radii: [f32; 4],
    /// The active rounded clip rectangle in physical pixels (`x, y, w, h`), with its radius in
    /// [`Self::clip_radius`]; both are meaningless without a radius (T-5 / C-051).
    pub clip: [f32; 4],
}

impl Instance {
    /// Radius of the active rounded clip in physical pixels; `0.0` when only rectangular clips
    /// (the scissor) apply.
    pub fn clip_radius(&self) -> f32 {
        self.params[1]
    }

    /// The largest per corner radius in physical pixels - the single radius a pre C-052 caller
    /// would have seen.
    pub fn radius(&self) -> f32 {
        self.radii.iter().copied().fold(0.0, f32::max)
    }

    /// [`MODE_COVERAGE`] or [`MODE_COLOR`].
    pub fn mode(&self) -> f32 {
        self.params[0]
    }
}

/// Quadrant order of [`EdgeRadius`]: `[top_left, top_right, bottom_left, bottom_right]`.
fn corner_radii(radius: EdgeRadius) -> [f32; 4] {
    [
        radius.top_left,
        radius.top_right,
        radius.bottom_left,
        radius.bottom_right,
    ]
}

/// Per instance draw state. Index `i` belongs to `instances[i]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawItem {
    /// Texture slot (see [`SLOT_WHITE`] / [`SLOT_GLYPH_ATLAS`]).
    pub texture: u32,
    /// Scissor rectangle in physical pixels (`x, y, w, h`).
    pub scissor: [u32; 4],
}

/// A run of consecutive items that share a texture and a clip rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Batch {
    pub texture: u32,
    pub scissor: [u32; 4],
    /// Index of the first instance of the run.
    pub start: usize,
    /// Number of instances in the run.
    pub count: usize,
}

/// Result of [`tessellate`].
#[derive(Debug, Default)]
pub struct Tessellation {
    /// Quad data uploaded to the GPU as one buffer (see [`Instance`]).
    pub instances: Vec<Instance>,
    /// Draw state, parallel to [`Self::instances`].
    pub items: Vec<DrawItem>,
    /// Coalesced draw calls.
    pub batches: Vec<Batch>,
}

/// Converts paint commands into instanced quads.
///
/// * `scale` - display scale factor (physical pixels per logical pixel, §41).
/// * `surface` - drawable size in physical pixels; clipping is clamped to it.
/// * `glyphs` - text engine; without it, `Text` commands are dropped.
pub fn tessellate(
    commands: &[PaintCommand],
    arena: &TextArena,
    scale: f32,
    surface: [u32; 2],
    glyphs: Option<&dyn GlyphSource>,
) -> Tessellation {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let viewport = Rect::new(
        0.0,
        0.0,
        surface[0] as f32 / scale,
        surface[1] as f32 / scale,
    );
    let mut builder = Builder {
        instances: Vec::with_capacity(commands.len().saturating_mul(2)),
        items: Vec::with_capacity(commands.len().saturating_mul(2)),
        scale,
        surface,
        clip: viewport,
        rounded: viewport,
        radius: 0.0,
        stack: Vec::new(),
    };

    for command in commands {
        match command {
            PaintCommand::PushClip { rect, radius } => {
                builder.stack.push(ClipFrame {
                    clip: builder.clip,
                    rounded: builder.rounded,
                    radius: builder.radius,
                });
                builder.clip = builder.clip.intersection(*rect);
                if *radius > 0.0 {
                    // The innermost rounded clip drives the per fragment SDF; the rectangles of
                    // any outer rounded clips are still enforced by the scissor, so nothing that
                    // used to be drawn can start leaking past a rectangle (C-051).
                    builder.rounded = *rect;
                    builder.radius = *radius;
                }
            }
            PaintCommand::PopClip => {
                if let Some(previous) = builder.stack.pop() {
                    builder.clip = previous.clip;
                    builder.rounded = previous.rounded;
                    builder.radius = previous.radius;
                }
            }
            PaintCommand::Shape {
                rect,
                fill,
                transform,
            } => builder.shape(*rect, fill, *transform),
            PaintCommand::Line {
                from,
                to,
                width,
                color,
            } => builder.line(*from, *to, *width, *color),
            PaintCommand::Text {
                position,
                run,
                range,
                style,
                max_width,
                transform,
            } => builder.text(
                arena,
                *position,
                *run,
                range.clone(),
                style,
                *max_width,
                *transform,
                glyphs,
            ),
            PaintCommand::Image {
                rect,
                texture,
                uv,
                tint,
                radius,
                border,
                transform,
            } => builder.image(
                *rect,
                *texture,
                *uv,
                *tint,
                corner_radii(*radius),
                border,
                *transform,
            ),
        }
    }

    let Builder {
        instances, items, ..
    } = builder;
    let batches = group_batches(&items);
    Tessellation {
        instances,
        items,
        batches,
    }
}

/// Groups consecutive items that can be drawn together.
pub fn group_batches(items: &[DrawItem]) -> Vec<Batch> {
    let mut batches: Vec<Batch> = Vec::with_capacity(items.len() / 8 + 1);
    for (index, item) in items.iter().enumerate() {
        match batches.last_mut() {
            Some(batch) if batch.texture == item.texture && batch.scissor == item.scissor => {
                batch.count += 1;
            }
            _ => batches.push(Batch {
                texture: item.texture,
                scissor: item.scissor,
                start: index,
                count: 1,
            }),
        }
    }
    batches
}

/// Texture slot of a [`TextureId`].
pub fn texture_slot(id: TextureId) -> u32 {
    if id.0 == TextureId::GLYPH_ATLAS.0 {
        SLOT_GLYPH_ATLAS
    } else {
        id.0
    }
}

/// Slots up to [`SLOT_GLYPH_ATLAS`] are coverage masks, everything else an RGBA image.
fn mode_for_slot(slot: u32) -> f32 {
    if slot <= SLOT_GLYPH_ATLAS {
        MODE_COVERAGE
    } else {
        MODE_COLOR
    }
}

const WHITE_UV: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// Accumulates quads while walking the command list.
struct Builder {
    instances: Vec<Instance>,
    items: Vec<DrawItem>,
    scale: f32,
    surface: [u32; 2],
    /// Intersection of every clip rectangle - the scissor (logical pixels).
    clip: Rect,
    /// Innermost rounded clip: its **original** rectangle plus radius (logical pixels). Only
    /// rounded rectangles contribute a second constraint beyond the scissor, and with one of
    /// them alive the pair (scissor, rounded) is an exact description of the clip stack (C-051).
    rounded: Rect,
    radius: f32,
    /// One saved state of the three fields above per `PushClip`.
    stack: Vec<ClipFrame>,
}

/// Saved clip state, restored by `PopClip`.
#[derive(Clone, Copy)]
struct ClipFrame {
    clip: Rect,
    rounded: Rect,
    radius: f32,
}

impl Builder {
    fn shape(&mut self, rect: Rect, fill: &Fill, transform: Transform) {
        if rect.is_empty() {
            return;
        }
        let corners = rect_corners(rect, transform);
        let radii = corner_radii(fill.radius);
        if fill.color.a > 0.0 || fill.gradient.is_some() {
            let colors = fill_colors(rect, fill, &corners);
            self.push(corners, WHITE_UV, colors, [0.0; 4], radii, SLOT_WHITE);
        }
        if fill.border.is_visible() {
            let widths = fill.border.widths;
            self.push(
                corners,
                WHITE_UV,
                [fill.border.color.to_array(); 4],
                [widths.left, widths.top, widths.right, widths.bottom],
                radii,
                SLOT_WHITE,
            );
        }
    }

    fn line(&mut self, from: Point, to: Point, width: f32, color: Color) {
        if width <= 0.0 || color.a <= 0.0 {
            return;
        }
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let length = (dx * dx + dy * dy).sqrt();
        if length <= f32::EPSILON {
            return;
        }
        let half = width * 0.5;
        let nx = -dy / length * half;
        let ny = dx / length * half;
        let corners = [
            [from.x + nx, from.y + ny],
            [to.x + nx, to.y + ny],
            [from.x - nx, from.y - ny],
            [to.x - nx, to.y - ny],
        ];
        self.push(
            corners,
            WHITE_UV,
            [color.to_array(); 4],
            [0.0; 4],
            [0.0; 4],
            SLOT_WHITE,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        arena: &TextArena,
        position: Point,
        run: TextRunId,
        range: std::ops::Range<u32>,
        style: &TextStyle,
        max_width: Option<f32>,
        transform: Transform,
        glyphs: Option<&dyn GlyphSource>,
    ) {
        let Some(source) = glyphs else { return };
        let Some(full) = arena.get(run) else { return };
        let start = (range.start as usize).min(full.len());
        let end = (range.end as usize).min(full.len()).max(start);
        // The painter only emits whole strings today; fall back to the full run if a range ever
        // lands inside a multi byte character.
        let text = if start == 0 && end == full.len() {
            full
        } else {
            full.get(start..end).unwrap_or(full)
        };
        if text.is_empty() {
            return;
        }
        let glyphs = source.glyphs(text, style, max_width, position);
        // Rotated text (T-8 / C-049): pivot on the centre of the run's ink box - the union of its
        // glyph rectangles - so the caller only ever says "rotate by n degrees".
        let pivot = if transform.is_identity() {
            Point::ZERO
        } else {
            let mut left = f32::MAX;
            let mut top = f32::MAX;
            let mut right = f32::MIN;
            let mut bottom = f32::MIN;
            for glyph in glyphs.iter().filter(|g| g.visible && !g.rect.is_empty()) {
                left = left.min(glyph.rect.left());
                top = top.min(glyph.rect.top());
                right = right.max(glyph.rect.right());
                bottom = bottom.max(glyph.rect.bottom());
            }
            if right <= left || bottom <= top {
                // Nothing visible: no quads are pushed below anyway.
                return;
            }
            Point::new((left + right) * 0.5, (top + bottom) * 0.5)
        };
        for glyph in glyphs {
            if !glyph.visible || glyph.rect.is_empty() {
                continue;
            }
            let slot = texture_slot_raw(glyph.atlas);
            self.push(
                rotate_about(
                    rect_corners(glyph.rect, Transform::IDENTITY),
                    pivot,
                    transform.rotation,
                ),
                glyph.uv,
                [style.color.to_array(); 4],
                [0.0; 4],
                [0.0; 4],
                slot,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn image(
        &mut self,
        rect: Rect,
        texture: TextureId,
        uv: [f32; 4],
        tint: Color,
        radii: [f32; 4],
        border: &Border,
        transform: Transform,
    ) {
        if rect.is_empty() {
            return;
        }
        let corners = rect_corners(rect, transform);
        let slot = texture_slot(texture);
        self.push(corners, uv, [tint.to_array(); 4], [0.0; 4], radii, slot);
        if border.is_visible() {
            let widths = border.widths;
            self.push(
                corners,
                uv,
                [border.color.to_array(); 4],
                [widths.left, widths.top, widths.right, widths.bottom],
                radii,
                slot,
            );
        }
    }

    /// Appends one quad unless it is entirely outside the clip.
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        corners: [[f32; 2]; 4],
        uv: [f32; 4],
        colors: [[f32; 4]; 4],
        inset: [f32; 4],
        radii: [f32; 4],
        texture_slot: u32,
    ) {
        let bounds = bounds_of(&corners);
        if bounds.is_empty() || !bounds.intersects(self.clip) {
            return;
        }
        let scissor = scissor_of(self.clip, self.scale, self.surface);
        if scissor[2] == 0 || scissor[3] == 0 {
            return;
        }
        let scale = self.scale;
        let (rounded, radius) = (self.rounded, self.radius);
        self.instances.push(Instance {
            corners: corners.map(|[x, y]| [x * scale, y * scale]),
            uv,
            colors,
            inset: inset.map(|value| value * scale),
            params: [mode_for_slot(texture_slot), radius * scale, 0.0, 0.0],
            radii: radii.map(|value| value * scale),
            clip: [
                rounded.x * scale,
                rounded.y * scale,
                rounded.width * scale,
                rounded.height * scale,
            ],
        });
        self.items.push(DrawItem {
            texture: texture_slot,
            scissor,
        });
    }
}

/// Texture slot of a raw `u32` coming from the text engine.
fn texture_slot_raw(index: u32) -> u32 {
    if index == TextureId::GLYPH_ATLAS.0 {
        SLOT_GLYPH_ATLAS
    } else {
        index
    }
}

/// Corners of `rect`, rotated around its centre when `transform` asks for it.
fn rect_corners(rect: Rect, transform: Transform) -> [[f32; 2]; 4] {
    let left = rect.left();
    let top = rect.top();
    let right = rect.right();
    let bottom = rect.bottom();
    rotate_about(
        [[left, top], [right, top], [left, bottom], [right, bottom]],
        rect.center(),
        transform.rotation,
    )
}

/// Rotates four corners clockwise on screen around `pivot`; identity rotation returns them as is.
fn rotate_about(corners: [[f32; 2]; 4], pivot: Point, rotation: f32) -> [[f32; 2]; 4] {
    if rotation == 0.0 {
        return corners;
    }
    let (sin, cos) = rotation.sin_cos();
    corners.map(|[x, y]| {
        let dx = x - pivot.x;
        let dy = y - pivot.y;
        [pivot.x + dx * cos - dy * sin, pivot.y + dx * sin + dy * cos]
    })
}

/// Axis aligned bounds of a set of corners.
fn bounds_of(corners: &[[f32; 2]; 4]) -> Rect {
    let mut left = f32::MAX;
    let mut top = f32::MAX;
    let mut right = f32::MIN;
    let mut bottom = f32::MIN;
    for [x, y] in corners {
        left = left.min(*x);
        top = top.min(*y);
        right = right.max(*x);
        bottom = bottom.max(*y);
    }
    if right <= left || bottom <= top {
        Rect::ZERO
    } else {
        Rect::new(left, top, right - left, bottom - top)
    }
}

/// Clip rectangle converted to physical pixels and clamped to the surface.
fn scissor_of(clip: Rect, scale: f32, surface: [u32; 2]) -> [u32; 4] {
    let x0 = (clip.left() * scale).floor();
    let y0 = (clip.top() * scale).floor();
    let x1 = (clip.right() * scale).ceil();
    let y1 = (clip.bottom() * scale).ceil();
    let max_x = surface[0] as f32;
    let max_y = surface[1] as f32;
    let x0 = x0.clamp(0.0, max_x);
    let y0 = y0.clamp(0.0, max_y);
    let x1 = x1.clamp(x0, max_x);
    let y1 = y1.clamp(y0, max_y);
    [x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32]
}

/// Per corner color of a fill; gradients are sampled at each corner because a linear gradient
/// interpolates exactly across a quad.
fn fill_colors(rect: Rect, fill: &Fill, corners: &[[f32; 2]; 4]) -> [[f32; 4]; 4] {
    let Some(gradient) = &fill.gradient else {
        return [fill.color.to_array(); 4];
    };
    corners.map(|[x, y]| sample_gradient(gradient, rect, Point::new(x, y)).to_array())
}

/// Samples a linear gradient at `point` (design document §20).
fn sample_gradient(gradient: &Gradient, rect: Rect, point: Point) -> Color {
    if gradient.stops.is_empty() {
        return Color::TRANSPARENT;
    }
    let angle = gradient.angle.to_radians();
    let direction = (angle.cos(), angle.sin());
    let center = rect.center();
    let half_extent =
        (rect.width * 0.5 * direction.0).abs() + (rect.height * 0.5 * direction.1).abs();
    let t = if half_extent <= f32::EPSILON {
        0.0
    } else {
        let projection = (point.x - center.x) * direction.0 + (point.y - center.y) * direction.1;
        (projection / half_extent * 0.5 + 0.5).clamp(0.0, 1.0)
    };
    lerp_stops(&gradient.stops, t)
}

fn lerp_stops(stops: &[GradientStop], t: f32) -> Color {
    if stops.len() == 1 {
        return stops[0].color;
    }
    let first = stops.first().expect("stops is not empty");
    if t <= first.position {
        return first.color;
    }
    for pair in stops.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if t <= b.position {
            let span = b.position - a.position;
            if span <= f32::EPSILON {
                return b.color;
            }
            let local = (t - a.position) / span;
            return a.color.lerp(b.color, local);
        }
    }
    stops.last().expect("stops is not empty").color
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustui_core::{AtlasInfo, EdgeRadius, Painter, PositionedGlyph};

    const SURFACE: [u32; 2] = [100, 100];

    fn painter() -> Painter {
        let mut painter = Painter::new();
        painter.reset(Rect::new(0.0, 0.0, 100.0, 100.0));
        painter
    }

    fn tessellate_paint(painter: &Painter, scale: f32) -> Tessellation {
        tessellate(painter.commands(), painter.arena(), scale, SURFACE, None)
    }

    /// Two fixed glyph quads so text can be tested without a text engine:
    /// `(10,10)-(30,20)` and `(40,10)-(60,20)`; their union - the ink box - is `(10,10)-(60,20)`.
    struct StubGlyphs;

    impl GlyphSource for StubGlyphs {
        fn glyphs(
            &self,
            _text: &str,
            _style: &TextStyle,
            _max_width: Option<f32>,
            _origin: Point,
        ) -> Vec<PositionedGlyph> {
            [
                Rect::new(10.0, 10.0, 20.0, 10.0),
                Rect::new(40.0, 10.0, 20.0, 10.0),
            ]
            .into_iter()
            .map(|rect| PositionedGlyph {
                rect,
                uv: [0.0, 0.0, 1.0, 1.0],
                atlas: TextureId::GLYPH_ATLAS.0,
                visible: true,
            })
            .collect()
        }

        fn atlas_info(&self) -> AtlasInfo {
            AtlasInfo {
                width: 1,
                height: 1,
                generation: 0,
                dirty: false,
            }
        }

        fn copy_atlas_into(&self, _dst: &mut Vec<u8>) {}

        fn mark_atlas_uploaded(&self) {}

        fn atlas_texture_index(&self) -> u32 {
            TextureId::GLYPH_ATLAS.0
        }
    }

    /// Tessellates one text command drawn with `transform` over [`StubGlyphs`].
    fn tessellate_text(transform: Transform) -> Tessellation {
        let mut painter = painter();
        painter.text_transformed(
            Point::new(10.0, 10.0),
            "rotated",
            &TextStyle::default(),
            None,
            transform,
        );
        tessellate(
            painter.commands(),
            painter.arena(),
            1.0,
            SURFACE,
            Some(&StubGlyphs),
        )
    }

    #[test]
    fn identity_text_keeps_the_glyph_corners() {
        let frame = tessellate_text(Transform::IDENTITY);
        assert_eq!(frame.instances.len(), 2);
        assert_eq!(
            frame.instances[0].corners,
            [[10.0, 10.0], [30.0, 10.0], [10.0, 20.0], [30.0, 20.0]]
        );
        assert_eq!(
            frame.instances[1].corners,
            [[40.0, 10.0], [60.0, 10.0], [40.0, 20.0], [60.0, 20.0]]
        );
    }

    #[test]
    fn rotated_text_pivots_on_the_ink_box_centre() {
        let frame = tessellate_text(Transform::rotated(90.0));
        assert_eq!(frame.instances.len(), 2);
        // Ink box (10,10)-(60,20), centre (35,15); a clockwise 90 degrees maps
        // (x, y) to (cx - dy, cy + dx): both quads turn into vertical strips.
        let expected = [
            [[40.0, -10.0], [40.0, 10.0], [30.0, -10.0], [30.0, 10.0]],
            [[40.0, 20.0], [40.0, 40.0], [30.0, 20.0], [30.0, 40.0]],
        ];
        for (index, (instance, want)) in frame.instances.iter().zip(expected.iter()).enumerate() {
            for (corner, want) in instance.corners.iter().zip(want.iter()) {
                assert!(
                    (corner[0] - want[0]).abs() < 0.01 && (corner[1] - want[1]).abs() < 0.01,
                    "glyph {index} corner {corner:?} should be {want:?}"
                );
            }
        }
    }

    #[test]
    fn instance_matches_the_shader_layout() {
        assert_eq!(std::mem::size_of::<Instance>(), 176);
        assert_eq!(std::mem::align_of::<Instance>(), 4);
    }

    /// A rounded clip must survive into the shader: the scissor alone only has its rectangle.
    #[test]
    fn a_rounded_clip_travels_with_every_instance() {
        let mut painter = painter();
        painter.push_clip_rounded(Rect::new(4.0, 8.0, 40.0, 20.0), 6.0);
        painter.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED);
        painter.pop_clip();
        painter.rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLUE);

        let frame = tessellate_paint(&painter, 1.0);
        let clipped = &frame.instances[0];
        assert_eq!(clipped.clip_radius(), 6.0, "the radius reaches the shader");
        assert_eq!(
            clipped.clip,
            [4.0, 8.0, 40.0, 20.0],
            "the clip rectangle, not the intersection"
        );
        let plain = &frame.instances[1];
        assert_eq!(plain.clip_radius(), 0.0, "a plain clip stays scissor only");
    }

    /// Stacked clips: rectangles all fold into the scissor, the innermost rounded clip drives the
    /// SDF, and popping restores what was underneath (C-051).
    #[test]
    fn the_innermost_rounded_clip_wins_and_pop_restores_it() {
        let mut painter = painter();
        painter.push_clip_rounded(Rect::new(0.0, 0.0, 100.0, 100.0), 2.0);
        painter.push_clip_rounded(Rect::new(10.0, 10.0, 60.0, 60.0), 8.0);
        painter.push_clip(Rect::new(20.0, 20.0, 40.0, 40.0));
        painter.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED);
        painter.pop_clip();
        painter.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED);
        painter.pop_clip();
        painter.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED);
        painter.pop_clip();

        let frame = tessellate_paint(&painter, 1.0);
        assert_eq!(frame.instances.len(), 3);

        // Underneath the plain clip: the inner rounded rectangle with the full intersection.
        assert_eq!(frame.instances[0].clip, [10.0, 10.0, 60.0, 60.0]);
        assert_eq!(frame.instances[0].clip_radius(), 8.0);
        assert_eq!(
            frame.items[0].scissor,
            [20, 20, 40, 40],
            "every rectangle still reaches the scissor"
        );

        // A plain clip on top neither clears nor replaces the rounded one.
        assert_eq!(frame.instances[1].clip, [10.0, 10.0, 60.0, 60.0]);
        assert_eq!(frame.instances[1].clip_radius(), 8.0);
        assert_eq!(frame.items[1].scissor, [10, 10, 60, 60]);

        // Popping the inner rounded clip brings the outer one back.
        assert_eq!(frame.instances[2].clip, [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(frame.instances[2].clip_radius(), 2.0);
    }

    /// All four corner radii ride to the GPU in quadrant order: top-left, top-right, bottom-left,
    /// bottom-right (C-052).
    #[test]
    fn corner_radii_reach_the_instance_in_quadrant_order() {
        let mut painter = painter();
        painter.styled_rect(
            Rect::new(0.0, 0.0, 40.0, 40.0),
            EdgeRadius {
                top_left: 1.0,
                top_right: 2.0,
                bottom_right: 3.0,
                bottom_left: 4.0,
            },
            Fill {
                color: Color::RED,
                gradient: None,
                border: Border {
                    widths: rustui_core::EdgeWidths::ZERO,
                    color: Color::TRANSPARENT,
                },
                radius: EdgeRadius::ZERO,
            },
        );

        let frame = tessellate_paint(&painter, 1.0);
        assert_eq!(frame.instances.len(), 1, "no border quad");
        assert_eq!(
            frame.instances[0].radii,
            [1.0, 2.0, 4.0, 3.0],
            "bottom_right sorts last, bottom_left third"
        );
        assert_eq!(
            frame.instances[0].radius(),
            4.0,
            "the legacy query is the max"
        );
    }

    #[test]
    fn shape_becomes_one_physical_quad() {
        let mut painter = painter();
        painter.rect(Rect::new(10.0, 10.0, 20.0, 20.0), Color::RED);
        let frame = tessellate_paint(&painter, 1.0);

        assert_eq!(frame.instances.len(), 1);
        assert_eq!(frame.instances.len(), frame.items.len());
        let instance = &frame.instances[0];
        assert_eq!(
            instance.corners,
            [[10.0, 10.0], [30.0, 10.0], [10.0, 30.0], [30.0, 30.0]]
        );
        assert_eq!(instance.uv, WHITE_UV);
        assert_eq!(instance.mode(), MODE_COVERAGE);
        assert_eq!(instance.radius(), 0.0);
        assert_eq!(frame.items[0].texture, SLOT_WHITE);
        assert_eq!(frame.items[0].scissor, [0, 0, 100, 100]);
        assert_eq!(frame.batches.len(), 1);
        assert_eq!(frame.batches[0].count, 1);
    }

    #[test]
    fn positions_and_radii_follow_the_scale_factor() {
        let mut painter = painter();
        painter.rounded_rect(Rect::new(10.0, 10.0, 20.0, 20.0), 4.0, Color::RED);
        // The painter works in logical pixels; the drawable is twice as large physically.
        let frame = tessellate(painter.commands(), painter.arena(), 2.0, [200, 200], None);

        let instance = &frame.instances[0];
        assert_eq!(instance.corners[1], [60.0, 20.0]);
        assert_eq!(instance.corners[3], [60.0, 60.0]);
        assert_eq!(instance.radius(), 8.0, "radius is scaled too");
        assert_eq!(frame.items[0].scissor, [0, 0, 200, 200]);
    }

    #[test]
    fn border_becomes_a_ring_with_inset() {
        let mut painter = painter();
        painter.border(
            Rect::new(0.0, 0.0, 20.0, 20.0),
            EdgeRadius::all(3.0),
            &Border::uniform(2.0, Color::BLACK),
        );
        let frame = tessellate_paint(&painter, 1.0);

        assert_eq!(frame.instances.len(), 1);
        assert_eq!(frame.instances[0].inset, [2.0, 2.0, 2.0, 2.0]);
        assert_eq!(frame.instances[0].radius(), 3.0);
    }

    #[test]
    fn fill_and_border_stack_into_two_quads() {
        let mut painter = painter();
        painter.styled_rect(
            Rect::new(0.0, 0.0, 20.0, 20.0),
            EdgeRadius::all(3.0),
            Fill::solid(Color::RED).with_border(Border::uniform(1.0, Color::BLACK)),
        );
        let frame = tessellate_paint(&painter, 1.0);
        assert_eq!(frame.instances.len(), 2, "fill first, then the border ring");
        assert_eq!(frame.instances[0].inset, [0.0; 4]);
        assert_eq!(frame.instances[1].inset, [1.0; 4]);
    }

    #[test]
    fn clip_restricts_the_scissor_and_is_restored() {
        let mut painter = painter();
        painter.push_clip(Rect::new(10.0, 20.0, 30.0, 40.0));
        painter.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED);
        painter.pop_clip();
        painter.rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLUE);

        let frame = tessellate_paint(&painter, 1.0);
        assert_eq!(frame.items.len(), 2);
        assert_eq!(frame.items[0].scissor, [10, 20, 30, 40]);
        assert_eq!(frame.items[1].scissor, [0, 0, 100, 100]);
        assert_eq!(frame.batches.len(), 2, "different clips cannot be batched");
    }

    #[test]
    fn items_outside_the_clip_are_dropped() {
        let mut painter = painter();
        painter.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
        painter.rect(Rect::new(50.0, 50.0, 10.0, 10.0), Color::RED);
        painter.pop_clip();

        let frame = tessellate_paint(&painter, 1.0);
        assert!(frame.instances.is_empty());
        assert!(frame.batches.is_empty());
    }

    #[test]
    fn consecutive_quads_share_one_batch() {
        let mut painter = painter();
        painter.rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);
        painter.rect(Rect::new(20.0, 0.0, 10.0, 10.0), Color::BLUE);
        let frame = tessellate_paint(&painter, 1.0);

        assert_eq!(frame.instances.len(), 2);
        assert_eq!(frame.batches.len(), 1);
        assert_eq!(frame.batches[0].count, 2);
        assert_eq!(frame.batches[0].start, 0);
    }

    #[test]
    fn gradient_colors_are_sampled_at_every_corner() {
        let mut painter = painter();
        let gradient = Gradient {
            stops: vec![
                GradientStop {
                    color: Color::BLACK,
                    position: 0.0,
                },
                GradientStop {
                    color: Color::WHITE,
                    position: 1.0,
                },
            ],
            angle: 0.0,
        };
        painter.styled_rect(
            Rect::new(0.0, 0.0, 100.0, 50.0),
            EdgeRadius::ZERO,
            Fill::solid(Color::RED).with_gradient(gradient),
        );
        let frame = tessellate_paint(&painter, 1.0);

        let colors = frame.instances[0].colors;
        assert!(colors[0][0] < 0.1, "left edge starts dark: {:?}", colors[0]);
        assert!(colors[1][0] > 0.9, "right edge ends light: {:?}", colors[1]);
        assert_eq!(colors[0], colors[2], "vertical edges match");
        assert_eq!(colors[1], colors[3], "vertical edges match");
    }

    #[test]
    fn line_becomes_a_quad_of_the_requested_width() {
        let mut painter = painter();
        painter.line(Point::new(0.0, 0.0), Point::new(10.0, 0.0), 4.0, Color::RED);
        let frame = tessellate_paint(&painter, 1.0);

        assert_eq!(frame.instances.len(), 1);
        let corners = frame.instances[0].corners;
        assert_eq!(corners[0], [0.0, 2.0], "quad spans the line width");
        assert_eq!(corners[3], [10.0, -2.0]);
        assert_eq!(frame.instances[0].radius(), 0.0);
    }

    #[test]
    fn text_is_dropped_without_a_glyph_source() {
        let mut painter = painter();
        painter.text(Point::new(4.0, 4.0), "hello", &TextStyle::default());
        assert_eq!(painter.commands().len(), 1);
        let frame = tessellate_paint(&painter, 1.0);
        assert!(frame.instances.is_empty());
    }

    #[test]
    fn glyph_slots_use_coverage_mode() {
        assert_eq!(texture_slot(TextureId::WHITE), SLOT_WHITE);
        assert_eq!(texture_slot(TextureId::GLYPH_ATLAS), SLOT_GLYPH_ATLAS);
        assert_eq!(texture_slot(TextureId(7)), 7);
        assert_eq!(mode_for_slot(SLOT_GLYPH_ATLAS), MODE_COVERAGE);
        assert_eq!(mode_for_slot(2), MODE_COLOR);
    }

    #[test]
    fn empty_commands_produce_no_batches() {
        let frame = tessellate(&[], &TextArena::default(), 1.0, SURFACE, None);
        assert!(frame.instances.is_empty());
        assert!(frame.batches.is_empty());
    }
}
