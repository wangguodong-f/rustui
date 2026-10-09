//! Painting primitives (§19, §20).
//!
//! Widgets never touch `wgpu`: they emit [`PaintCommand`]s into a [`Painter`], which the renderer
//! translates into GPU draw calls. This is the `Widget -> Painter -> PaintCommand -> Renderer`
//! pipeline of design document §10.

use std::collections::HashMap;
use std::ops::Range;

use crate::color::Color;
use crate::geometry::{EdgeRadius, EdgeWidths, Insets, Point, Rect};
use crate::style::{Border, Fill};

/// Handle to a texture uploaded through the renderer's resource manager (§24).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(pub u32);

impl TextureId {
    /// Placeholder texture (solid white / font atlas).
    pub const WHITE: TextureId = TextureId(0);
    /// The dynamically growing glyph atlas of the text engine.
    pub const GLYPH_ATLAS: TextureId = TextureId(u32::MAX);
}

/// Handle to a font registered with the text engine (§25).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(pub u32);

impl FontId {
    /// The engine's default font.
    pub const DEFAULT: FontId = FontId(0);
}

/// A resolved color stop of a gradient.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub color: Color,
    /// Position along the gradient axis, `0..1`.
    pub position: f32,
}

/// A two (or more) stop gradient.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub stops: Vec<GradientStop>,
    /// Gradient direction in degrees, `0` = left to right, `90` = top to bottom.
    pub angle: f32,
}

impl Gradient {
    pub fn linear(angle: f32, from: Color, to: Color) -> Self {
        Self {
            stops: vec![
                GradientStop {
                    color: from,
                    position: 0.0,
                },
                GradientStop {
                    color: to,
                    position: 1.0,
                },
            ],
            angle,
        }
    }
}

/// A 2D transform applied while generating vertices (rotation in radians).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transform {
    /// Rotation around the centre of the shape, in radians, clockwise on screen.
    pub rotation: f32,
}

impl Transform {
    pub const IDENTITY: Transform = Transform { rotation: 0.0 };

    #[inline]
    pub fn rotated(degrees: f32) -> Self {
        Transform {
            rotation: degrees.to_radians(),
        }
    }

    #[inline]
    pub fn is_identity(&self) -> bool {
        self.rotation == 0.0
    }
}

/// A reference into the painter's per-frame string arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextRunId(pub u32);

/// How text is aligned inside its layout box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// A resolved text style.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub font: FontId,
    /// Font size in logical pixels.
    pub size: f32,
    pub color: Color,
    pub align: TextAlign,
    /// Extra space added after every glyph.
    pub letter_spacing: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: FontId::DEFAULT,
            size: 14.0,
            color: Color::WHITE,
            align: TextAlign::Left,
            letter_spacing: 0.0,
        }
    }
}

impl TextStyle {
    pub fn new(size: f32, color: Color) -> Self {
        Self {
            size,
            color,
            ..Default::default()
        }
    }

    pub fn with_font(mut self, font: FontId) -> Self {
        self.font = font;
        self
    }

    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    pub fn with_letter_spacing(mut self, spacing: f32) -> Self {
        self.letter_spacing = spacing;
        self
    }

    /// Hashable key used by the text engine's layout cache.
    pub fn cache_key(&self) -> u64 {
        let size = self.size.to_bits();
        (self.font.0 as u64)
            ^ ((size << 8) as u64)
            ^ ((self.color.to_rgba8()[3] as u64) << 24)
            ^ (((self.letter_spacing.to_bits() & 0xffff) as u64) << 40)
    }
}

/// A single drawing instruction produced by a widget.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintCommand {
    /// Restricts subsequent drawing to `rect` (intersected with the current clip).
    PushClip { rect: Rect, radius: f32 },
    /// Restores the previous clip.
    PopClip,
    /// A filled (and optionally stroked / gradient) shape.
    Shape {
        rect: Rect,
        fill: Fill,
        transform: Transform,
    },
    /// A straight line, generated as a rotated quad.
    Line {
        from: Point,
        to: Point,
        width: f32,
        color: Color,
    },
    /// Text; `range` selects a slice of the frame arena entry so that no `String` is copied.
    Text {
        position: Point,
        run: TextRunId,
        range: Range<u32>,
        style: TextStyle,
        /// Maximum line width used by the layout engine, if the text wraps.
        max_width: Option<f32>,
        /// Rotation around the centre of the run's ink box, like [`PaintCommand::Shape`] rotates
        /// around its rect centre (T-8 / C-049).
        transform: Transform,
    },
    /// A textured quad.
    Image {
        rect: Rect,
        texture: TextureId,
        /// Normalised texture coordinates.
        uv: [f32; 4],
        tint: Color,
        radius: EdgeRadius,
        border: Border,
        transform: Transform,
    },
}

impl PaintCommand {
    /// Approximate cost of the command, used to decide whether a frame needs a full rebuild.
    pub fn is_expensive(&self) -> bool {
        matches!(self, PaintCommand::Text { .. } | PaintCommand::Image { .. })
    }
}

/// Per-frame string arena: avoids allocating a `String` per text command per frame (§49).
#[derive(Debug, Default)]
pub struct TextArena {
    entries: Vec<Box<str>>,
    index: HashMap<Box<str>, TextRunId>,
}

impl TextArena {
    /// Interns `text` and returns its handle.
    pub fn intern(&mut self, text: &str) -> TextRunId {
        if let Some(id) = self.index.get(text) {
            return *id;
        }
        let boxed: Box<str> = text.into();
        let id = TextRunId(self.entries.len() as u32);
        self.index.insert(boxed.clone(), id);
        self.entries.push(boxed);
        id
    }

    /// Returns the text of a run.
    pub fn get(&self, id: TextRunId) -> Option<&str> {
        self.entries.get(id.0 as usize).map(|s| &**s)
    }

    /// Number of distinct strings in the arena.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Collects [`PaintCommand`]s while walking the widget tree.
#[derive(Debug)]
pub struct Painter {
    commands: Vec<PaintCommand>,
    clip_stack: Vec<Rect>,
    current_clip: Rect,
    arena: TextArena,
    /// Commands dropped because of an empty clip; used by tests and statistics.
    clipped_out: usize,
}

impl Default for Painter {
    fn default() -> Self {
        Self::new()
    }
}

impl Painter {
    /// Creates a painter for a drawable area of `size` logical pixels.
    pub fn new() -> Self {
        let infinite = Rect::new(f32::MIN / 2.0, f32::MIN / 2.0, f32::MAX, f32::MAX);
        Self {
            commands: Vec::with_capacity(1024),
            clip_stack: Vec::new(),
            current_clip: infinite,
            arena: TextArena::default(),
            clipped_out: 0,
        }
    }

    /// Clears all commands and clips, keeping allocated capacity.
    pub fn reset(&mut self, viewport: Rect) {
        self.commands.clear();
        self.clip_stack.clear();
        self.current_clip = viewport;
        self.arena = TextArena::default();
        self.clipped_out = 0;
    }

    /// The clip rectangle in effect.
    #[inline]
    pub fn current_clip(&self) -> Rect {
        self.current_clip
    }

    /// Number of recorded commands.
    #[inline]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Recorded commands.
    #[inline]
    pub fn commands(&self) -> &[PaintCommand] {
        &self.commands
    }

    /// The per-frame text arena.
    #[inline]
    pub fn arena(&self) -> &TextArena {
        &self.arena
    }

    /// Pushes an intersected clip rectangle.
    pub fn push_clip(&mut self, rect: Rect) {
        let clipped = self.current_clip.intersection(rect);
        self.clip_stack.push(self.current_clip);
        self.current_clip = clipped;
        self.commands.push(PaintCommand::PushClip {
            rect: clipped,
            radius: 0.0,
        });
    }

    /// Pushes a clip with a rounded shape (informational for renderers that support it).
    pub fn push_clip_rounded(&mut self, rect: Rect, radius: f32) {
        let clipped = self.current_clip.intersection(rect);
        self.clip_stack.push(self.current_clip);
        self.current_clip = clipped;
        self.commands.push(PaintCommand::PushClip {
            rect: clipped,
            radius,
        });
    }

    /// Restores the previous clip.
    pub fn pop_clip(&mut self) {
        if let Some(previous) = self.clip_stack.pop() {
            self.current_clip = previous;
            self.commands.push(PaintCommand::PopClip);
        }
    }

    /// Depth of the clip stack (0 means no clip).
    pub fn clip_depth(&self) -> usize {
        self.clip_stack.len()
    }

    #[inline]
    fn push(&mut self, command: PaintCommand) {
        self.commands.push(command);
    }

    /// Fills a rectangle.
    pub fn rect(&mut self, rect: Rect, color: Color) {
        if color.a <= 0.0 || rect.is_empty() {
            return;
        }
        self.push(PaintCommand::Shape {
            rect,
            fill: Fill {
                color,
                gradient: None,
                border: Border {
                    widths: EdgeWidths::ZERO,
                    color: Color::TRANSPARENT,
                },
                radius: EdgeRadius::ZERO,
            },
            transform: Transform::IDENTITY,
        });
    }

    /// Fills a rounded rectangle.
    pub fn rounded_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        self.styled_rect(
            rect,
            EdgeRadius::all(radius),
            Fill {
                color,
                gradient: None,
                border: Border {
                    widths: EdgeWidths::ZERO,
                    color: Color::TRANSPARENT,
                },
                radius: EdgeRadius::all(radius),
            },
        );
    }

    /// Fills (and optionally strokes) a shape described by `fill`.
    ///
    /// `radius` is merged into the fill, so a single command can express "rounded rect with a
    /// 2px border and a vertical gradient".
    pub fn styled_rect(&mut self, rect: Rect, radius: EdgeRadius, mut fill: Fill) {
        if rect.is_empty() || fill.is_invisible() {
            return;
        }
        if !radius.is_zero() {
            fill.radius = radius;
        }
        self.push(PaintCommand::Shape {
            rect,
            fill,
            transform: Transform::IDENTITY,
        });
    }

    /// Strokes the outline of `rect`.
    pub fn border(&mut self, rect: Rect, radius: EdgeRadius, border: &Border) {
        if border.widths.is_zero() || border.color.a <= 0.0 || rect.is_empty() {
            return;
        }
        self.push(PaintCommand::Shape {
            rect,
            fill: Fill {
                color: Color::TRANSPARENT,
                gradient: None,
                border: *border,
                radius,
            },
            transform: Transform::IDENTITY,
        });
    }

    /// Draws a straight line.
    pub fn line(&mut self, from: Point, to: Point, width: f32, color: Color) {
        if color.a <= 0.0 || width <= 0.0 {
            return;
        }
        self.push(PaintCommand::Line {
            from,
            to,
            width,
            color,
        });
    }

    /// Draws a single line of text; `position` is the top-left corner of the text layout box.
    ///
    /// The string is interned in the frame arena, so repeated strings cost nothing after the
    /// first use.
    pub fn text(&mut self, position: Point, text: &str, style: &TextStyle) -> TextRunId {
        self.text_transformed(position, text, style, None, Transform::IDENTITY)
    }

    /// Draws text that wraps at `max_width`.
    pub fn text_wrapped(
        &mut self,
        position: Point,
        text: &str,
        style: &TextStyle,
        max_width: f32,
    ) -> TextRunId {
        self.text_transformed(position, text, style, Some(max_width), Transform::IDENTITY)
    }

    /// Draws text with an optional wrap width and an optional rotation (T-8 / C-049).
    ///
    /// The rotation happens around the centre of the run's ink box while tessellating - the text
    /// equivalent of [`Painter::styled_rect`], whose transform rotates around the rect centre -
    /// so a caller only states *how much*, never where the pivot is.
    pub fn text_transformed(
        &mut self,
        position: Point,
        text: &str,
        style: &TextStyle,
        max_width: Option<f32>,
        transform: Transform,
    ) -> TextRunId {
        let run = self.arena.intern(text);
        self.push(PaintCommand::Text {
            position,
            run,
            range: 0..text.len() as u32,
            style: style.clone(),
            max_width,
            transform,
        });
        run
    }

    /// Draws a texture.
    pub fn image(&mut self, rect: Rect, texture: TextureId, tint: Color) {
        if rect.is_empty() || tint.a <= 0.0 {
            return;
        }
        self.push(PaintCommand::Image {
            rect,
            texture,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint,
            radius: EdgeRadius::ZERO,
            border: Border::default(),
            transform: Transform::IDENTITY,
        });
    }

    /// Draws a sub-region of a texture (used by the virtualized views).
    pub fn image_uv(
        &mut self,
        rect: Rect,
        texture: TextureId,
        uv: [f32; 4],
        tint: Color,
        radius: EdgeRadius,
    ) {
        if rect.is_empty() || tint.a <= 0.0 {
            return;
        }
        self.push(PaintCommand::Image {
            rect,
            texture,
            uv,
            tint,
            radius,
            border: Border::default(),
            transform: Transform::IDENTITY,
        });
    }

    /// Fills the whole clip region (used for window backgrounds).
    pub fn fill_all(&mut self, color: Color) {
        let clip = self.current_clip;
        if clip.is_empty() {
            self.clipped_out += 1;
            return;
        }
        self.rect(clip, color);
    }

    /// Fills a rectangle deflated by `padding`.
    pub fn rect_inset(&mut self, rect: Rect, padding: Insets, color: Color) {
        self.rect(rect.shrink(padding), color);
    }

    /// Consumes the painter and returns the recorded commands.
    pub fn into_commands(self) -> Vec<PaintCommand> {
        self.commands
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Vec2;

    #[test]
    fn clip_stack_tracks_intersection() {
        let mut p = Painter::new();
        let viewport = Rect::new(0.0, 0.0, 100.0, 100.0);
        p.reset(viewport);
        assert_eq!(p.current_clip(), viewport);
        p.push_clip(Rect::new(10.0, 10.0, 200.0, 200.0));
        assert_eq!(p.current_clip(), Rect::new(10.0, 10.0, 90.0, 90.0));
        p.push_clip(Rect::new(0.0, 0.0, 20.0, 20.0));
        assert_eq!(p.current_clip(), Rect::new(10.0, 10.0, 10.0, 10.0));
        assert_eq!(p.clip_depth(), 2);
        p.pop_clip();
        p.pop_clip();
        assert_eq!(p.current_clip(), viewport);
        assert_eq!(p.clip_depth(), 0);
    }

    #[test]
    fn transparent_commands_are_skipped() {
        let mut p = Painter::new();
        p.rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::TRANSPARENT);
        p.rounded_rect(Rect::new(0.0, 0.0, 0.0, 10.0), 2.0, Color::RED);
        assert!(p.is_empty());
    }

    #[test]
    fn text_arena_interns_repeated_strings() {
        let mut p = Painter::new();
        let style = TextStyle::default();
        p.text(Point::ZERO, "abc", &style);
        p.text(Point::new(1.0, 1.0), "abc", &style);
        p.text(Point::new(2.0, 2.0), "xyz", &style);
        assert_eq!(p.arena().len(), 2);
        assert_eq!(p.len(), 3);
        assert_eq!(p.arena().get(TextRunId(0)), Some("abc"));
    }

    #[test]
    fn text_commands_carry_their_transform_and_width() {
        let mut p = Painter::new();
        let style = TextStyle::default();
        p.text(Point::ZERO, "flat", &style);
        p.text_wrapped(Point::ZERO, "wide", &style, 80.0);
        p.text_transformed(
            Point::ZERO,
            "tilted",
            &style,
            None,
            Transform::rotated(30.0),
        );

        let field = |index: usize| match &p.commands()[index] {
            PaintCommand::Text {
                transform,
                max_width,
                ..
            } => (*transform, *max_width),
            other => panic!("unexpected command {other:?}"),
        };
        // `text` and `text_wrapped` delegate to `text_transformed` with the identity (T-8).
        assert_eq!(field(0), (Transform::IDENTITY, None));
        assert_eq!(field(1), (Transform::IDENTITY, Some(80.0)));
        assert_eq!(field(2), (Transform::rotated(30.0), None));
        assert_eq!(p.arena().len(), 3);
    }

    #[test]
    fn commands_carry_geometry_without_copying_strings() {
        let mut p = Painter::new();
        let long = "x".repeat(4096);
        p.text(Point::ZERO, &long, &TextStyle::default());
        // the string lives in the arena, not inside the command
        match &p.commands()[0] {
            PaintCommand::Text { run, range, .. } => {
                assert_eq!(range.end, 4096);
                assert_eq!(p.arena().get(*run), Some(long.as_str()));
            }
            other => panic!("unexpected command {other:?}"),
        }
    }

    #[test]
    fn line_command_recorded() {
        let mut p = Painter::new();
        p.line(Point::ZERO, Point::new(0.0, 10.0), 2.0, Color::BLUE);
        assert!(matches!(
            p.commands()[0],
            PaintCommand::Line { width: 2.0, .. }
        ));
    }

    #[test]
    fn transform_rotation() {
        assert!(Transform::IDENTITY.is_identity());
        assert!((Transform::rotated(90.0).rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        let _ = Vec2::ZERO;
    }
}
