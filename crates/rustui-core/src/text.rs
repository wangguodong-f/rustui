//! Text measurement and glyph access traits (§25).
//!
//! `rustui-core` cannot depend on `rustui-text` (and certainly not on `wgpu`), so it only
//! declares what a widget needs: a way to measure a string, and a way to obtain positioned glyphs
//! plus an atlas for the renderer. `rustui-text` provides the implementation.

use crate::geometry::{Point, Rect, Size};
use crate::paint::TextStyle;

pub use crate::paint::TextStyle as Style;

/// A glyph positioned in window coordinates, ready to be turned into a quad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    /// Rectangle the glyph bitmap occupies, in logical pixels.
    pub rect: Rect,
    /// Normalised atlas coordinates (`u0, v0, u1, v1`).
    pub uv: [f32; 4],
    /// Atlas the glyph lives in.
    pub atlas: u32,
    /// `false` for whitespace and for glyphs that failed to rasterize.
    pub visible: bool,
}

/// Size of the glyph atlas as it currently exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasInfo {
    pub width: u32,
    pub height: u32,
    pub generation: u32,
    /// `true` when the atlas bitmap changed since the last upload.
    ///
    /// The atlas lives behind interior mutability in the text engine, so the renderer copies it
    /// out (see [`GlyphSource::copy_atlas_into`]) instead of borrowing it; this flag tells it
    /// whether that copy is needed at all.
    pub dirty: bool,
}

/// Metrics of a single line of text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    /// Byte range of this line inside the source string.
    pub start: usize,
    pub end: usize,
    /// Width of the line in logical pixels.
    pub width: f32,
}

/// The result of laying out a piece of text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayout {
    /// Total size of the layout.
    pub size: Size,
    /// Distance from the top of the layout to the baseline.
    pub ascent: f32,
    /// Distance from the baseline to the bottom of the layout.
    pub descent: f32,
    pub lines: Vec<TextLine>,
    /// Maximum line width requested (wrapping).
    pub max_width: Option<f32>,
}

impl TextLayout {
    /// Lays out a single line without wrapping.
    pub fn single_line(size: Size, ascent: f32, descent: f32) -> Self {
        Self {
            size,
            ascent,
            descent,
            lines: Vec::new(),
            max_width: None,
        }
    }
}

/// Measures text. Implemented by the text engine.
pub trait TextMeasurer {
    /// Lays out `text` and returns its size. `max_width` enables word wrapping.
    fn layout_text(&self, text: &str, style: &TextStyle, max_width: Option<f32>) -> TextLayout;

    /// Convenience wrapper returning only the size.
    fn measure_text(&self, text: &str, style: &TextStyle, max_width: Option<f32>) -> Size {
        self.layout_text(text, style, max_width).size
    }

    /// Height of a single line for `style`.
    fn line_height(&self, style: &TextStyle) -> f32;

    /// Updates the display scale used when rasterising glyphs (§41).
    ///
    /// Layout itself is expressed in logical pixels and never depends on the scale; only the
    /// glyph atlas does. The default implementation is a no-op for measurers that cannot
    /// rasterise anything (test stubs).
    fn set_scale_factor(&self, _scale: f32) {}

    /// The glyph source feeding the renderer, when this measurer can also rasterise glyphs.
    ///
    /// `None` means "text cannot be rendered" (a stub measurer used by tests), in which case the
    /// renderer drops text commands.
    fn glyph_source(&self) -> Option<&dyn GlyphSource> {
        None
    }

    /// Width of a single space character; used by layouts to approximate gaps.
    fn space_width(&self, style: &TextStyle) -> f32 {
        self.measure_text(" ", style, None).width
    }

    /// Index of the character position closest to `x` on the first line.
    fn hit_test_x(&self, text: &str, style: &TextStyle, x: f32) -> usize {
        let mut best = 0usize;
        let mut best_dist = f32::MAX;
        for (index, _) in text.char_indices() {
            let width = self.measure_text(&text[..index], style, None).width;
            let dist = (width - x).abs();
            if dist < best_dist {
                best_dist = dist;
                best = index;
            }
            if width > x {
                break;
            }
        }
        best
    }
}

/// Provides positioned glyphs and the atlas for the renderer.
///
/// The renderer only needs this trait, which keeps `rustui-renderer` independent from
/// `rustui-text`.
pub trait GlyphSource {
    /// Lays out `text` and returns every glyph quad.
    fn glyphs(
        &self,
        text: &str,
        style: &TextStyle,
        max_width: Option<f32>,
        origin: Point,
    ) -> Vec<PositionedGlyph>;

    /// Current atlas metadata (includes the `dirty` flag).
    fn atlas_info(&self) -> AtlasInfo;

    /// Copies the whole atlas bitmap (single channel coverage) into `dst`.
    ///
    /// Only called when [`AtlasInfo::dirty`] is set; the copy exists because the atlas is kept
    /// behind interior mutability and cannot be borrowed past the call.
    fn copy_atlas_into(&self, dst: &mut Vec<u8>);

    /// Clears [`AtlasInfo::dirty`]; called by the renderer once the bitmap reached the GPU.
    fn mark_atlas_uploaded(&self);

    /// Index of the texture the renderer must bind for atlas textures.
    fn atlas_texture_index(&self) -> u32;
}
