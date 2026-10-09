//! HarfBuzz style shaping through `rustybuzz` (T-9 / C-053).
//!
//! The shaper turns a run of text into positioned glyphs the way a text stack should: ligatures
//! and contextual forms are substituted (GSUB), kerning is applied (GPOS) and complex scripts
//! re-order (Devanagari conjuncts, Arabic joining, Thai marks). `ab_glyph` stays the rasterizer:
//! both read glyph ids from the same `cmap`, so the atlas pipeline does not change.
//!
//! Everything a run needs is decided by the caller:
//!
//! * direction comes from the bidi analysis (`base_rtl` / `unicode_bidi` levels) and is passed
//!   in explicitly, because a mixed paragraph contains runs of both directions;
//! * the [`Shaper`] borrows the font (the `rustybuzz` face has the font's lifetime), so it is
//!   built once per layout call and dropped with it;
//! * fonts registered without source bytes fall back to one-glyph-per-character layout - the
//!   pre T-9 behaviour, still correct for Latin text without shaping features.

use ab_glyph::ScaleFont as _;

use rustui_core::TextStyle;

use crate::font::LoadedFont;

/// One shaped glyph; advances are in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapedGlyph {
    /// Glyph id inside the font (the same id `ab_glyph` rasterizes).
    pub glyph_id: u16,
    /// Byte offset of the cluster this glyph belongs to, relative to the shaped slice.
    pub cluster: usize,
    /// Horizontal advance in pixels, the cluster's letter spacing included.
    pub x_advance: f32,
    /// Horizontal offset from the pen in pixels.
    pub x_offset: f32,
    /// `true` when the glyph came from a right-to-left run.
    pub rtl: bool,
}

/// Shaper for one (font, size, letter spacing) triple.
///
/// The `rustybuzz` face borrows the font's bytes, which is why this is a short-lived view built
/// per layout rather than a stored engine field.
pub struct Shaper<'a> {
    face: Option<rustybuzz::Face<'a>>,
    font: &'a LoadedFont,
    px: f32,
    letter_spacing: f32,
}

impl<'a> Shaper<'a> {
    pub fn new(font: &'a LoadedFont, style: &TextStyle) -> Self {
        Self {
            face: font.shaper_face(),
            font,
            px: style.size,
            letter_spacing: style.letter_spacing,
        }
    }

    /// `true` when this shaper uses HarfBuzz (the font kept its bytes).
    pub fn has_features(&self) -> bool {
        self.face.is_some()
    }

    /// Shapes `text` as a single run in the given direction.
    pub fn shape(&self, text: &str, rtl: bool) -> Vec<ShapedGlyph> {
        if text.is_empty() {
            return Vec::new();
        }
        let Some(face) = &self.face else {
            return self.shape_by_char(text);
        };
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        buffer.set_direction(if rtl {
            rustybuzz::Direction::RightToLeft
        } else {
            rustybuzz::Direction::LeftToRight
        });
        let output = rustybuzz::shape(face, &[], buffer);
        let infos = output.glyph_infos();
        let positions = output.glyph_positions();
        let scale = self.px / self.font.units_per_em;

        // Cluster boundaries in byte order: letter spacing is charged per character of the
        // cluster, so a ligature covering two characters keeps the spacing of two (and wrapping
        // only ever breaks between clusters, never inside one).
        let mut boundaries: Vec<usize> = infos.iter().map(|info| info.cluster as usize).collect();
        boundaries.sort_unstable();
        boundaries.dedup();

        let mut glyphs = Vec::with_capacity(infos.len());
        for (info, position) in infos.iter().zip(positions.iter()) {
            let cluster = info.cluster as usize;
            let end = boundaries
                .iter()
                .copied()
                .find(|&next| next > cluster)
                .unwrap_or(text.len());
            let chars = text[cluster..end].chars().count().max(1) as f32;
            glyphs.push(ShapedGlyph {
                glyph_id: info.glyph_id as u16,
                cluster,
                x_advance: position.x_advance as f32 * scale + self.letter_spacing * chars,
                x_offset: position.x_offset as f32 * scale,
                rtl,
            });
        }
        glyphs
    }

    /// Per-cluster advances of `text` in logical order (byte ascending), for wrapping.
    ///
    /// Several glyphs may share a cluster (a base character plus its marks); their advances are
    /// summed so a break decision can never split the cluster apart.
    pub fn cluster_advances(&self, text: &str, rtl: bool) -> Vec<(usize, f32)> {
        let mut out: Vec<(usize, f32)> = self
            .shape(text, rtl)
            .into_iter()
            .map(|glyph| (glyph.cluster, glyph.x_advance))
            .collect();
        out.sort_by_key(|(cluster, _)| *cluster);
        let mut merged: Vec<(usize, f32)> = Vec::with_capacity(out.len());
        for (cluster, advance) in out {
            match merged.last_mut() {
                Some(last) if last.0 == cluster => last.1 += advance,
                _ => merged.push((cluster, advance)),
            }
        }
        merged
    }

    /// Total advance width of `text` in pixels.
    pub fn measure(&self, text: &str, rtl: bool) -> f32 {
        self.shape(text, rtl)
            .iter()
            .map(|glyph| glyph.x_advance)
            .sum()
    }

    /// Pre T-9 fallback: one glyph per character, `ab_glyph` metrics only.
    fn shape_by_char(&self, text: &str) -> Vec<ShapedGlyph> {
        let scaled = self.font.scaled(self.px);
        text.char_indices()
            .map(|(offset, ch)| {
                let gid = self.font.glyph_id(ch);
                ShapedGlyph {
                    glyph_id: gid.0,
                    cluster: offset,
                    x_advance: scaled.h_advance(gid) + self.letter_spacing,
                    x_offset: 0.0,
                    rtl: false,
                }
            })
            .collect()
    }
}

/// Paragraph direction of `text` according to UAX#9 (`true` = right-to-left), used when a run
/// does not inherit its level from a bidi analysis.
pub fn base_rtl(text: &str) -> bool {
    let info = unicode_bidi::BidiInfo::new(text, None);
    info.paragraphs
        .first()
        .map(|paragraph| paragraph.level.is_rtl())
        .unwrap_or(false)
}
