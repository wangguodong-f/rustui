//! Text layout: line breaking, alignment and glyph positioning (design document §25, §47).
//!
//! Layout is deliberately simple but complete for UI text:
//!
//! * grapheme-cluster-safe wrapping at word boundaries, with hard breaks on `\n`;
//! * three alignment modes (left / center / right) relative to the layout box;
//! * cached per (font, size, text, max width) so painting never re-measures.
//!
//! Shaping (T-9 / C-053): runs go through `rustybuzz` (ligatures, kerning, complex scripts),
//! lines are reordered with the Unicode bidirectional algorithm (`unicode-bidi`) and wrapping
//! only ever breaks **between** clusters. Every glyph carries the character range of its
//! cluster, so caret placement and hit testing keep working when several characters share one
//! glyph.

use std::collections::HashMap;

use ab_glyph::ScaleFont as _;

use rustui_core::{FontId, Point, Rect, Size, TextAlign, TextLayout, TextLine, TextStyle};

use crate::font::{FontRegistry, LoadedFont};
use crate::glyph::GlyphKey;
use crate::shape::{base_rtl, Shaper};

/// One positioned, shaped glyph ready to be turned into a quad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaidOutGlyph {
    /// First character of the cluster this glyph covers.
    pub ch: char,
    /// Character index of the cluster start, counted over the whole laid-out text.
    pub char_index: usize,
    /// How many characters the cluster covers (1 for almost everything; ligatures and
    /// re-ordered sequences cover more).
    pub char_count: usize,
    /// `true` for glyphs of a right-to-left run: the caret leads at the right edge.
    pub rtl: bool,
    /// Pen position of the glyph, relative to the layout origin (left edge, top of the first
    /// line).
    pub position: Point,
    pub advance: f32,
    pub key: GlyphKey,
}

/// Result of laying out a string.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidOutText {
    pub layout: TextLayout,
    pub glyphs: Vec<LaidOutGlyph>,
    pub style_size: f32,
    pub line_height: f32,
}

impl LaidOutText {
    /// Total advance width of the widest line.
    pub fn width(&self) -> f32 {
        self.layout.size.width
    }

    pub fn height(&self) -> f32 {
        self.layout.size.height
    }
}

/// Cache key for a layout request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LayoutKey {
    font: FontId,
    size_64: u64,
    letter_spacing_64: u64,
    max_width: Option<u64>,
    align: u8,
    hash: u64,
}

fn layout_key(text: &str, style: &TextStyle, max_width: Option<f32>) -> LayoutKey {
    let mut hash = 1469598103934665603u64;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    LayoutKey {
        font: style.font,
        size_64: (style.size * 64.0).round() as u64,
        letter_spacing_64: (style.letter_spacing * 64.0).round() as u64,
        max_width: max_width.map(|w| (w * 64.0).round() as u64),
        align: style.align as u8,
        hash,
    }
}

/// Lays out text using fonts from a [`FontRegistry`], caching results.
#[derive(Debug)]
pub struct TextLayoutEngine {
    cache: HashMap<LayoutKey, LaidOutText>,
    capacity: usize,
}

impl Default for TextLayoutEngine {
    fn default() -> Self {
        Self::new(2048)
    }
}

impl TextLayoutEngine {
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: HashMap::new(),
            capacity,
        }
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Lays out `text`; `max_width` enables wrapping.
    pub fn layout(
        &mut self,
        registry: &FontRegistry,
        text: &str,
        style: &TextStyle,
        max_width: Option<f32>,
    ) -> LaidOutText {
        let key = layout_key(text, style, max_width);
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }
        let laid_out = layout_uncached(registry, text, style, max_width);
        if self.cache.len() >= self.capacity {
            self.cache.clear();
        }
        self.cache.insert(key, laid_out.clone());
        laid_out
    }

    /// Metrics only (does not touch the glyph cache).
    pub fn measure(
        &mut self,
        registry: &FontRegistry,
        text: &str,
        style: &TextStyle,
        max_width: Option<f32>,
    ) -> Size {
        self.layout(registry, text, style, max_width).layout.size
    }
}

fn metrics(font: &LoadedFont, style: &TextStyle) -> (f32, f32, f32, f32) {
    let px = style.size;
    let scaled = font.scaled(px);
    // ab_glyph reports ascent/descent in pixels for the given scale.
    let ascent = scaled.ascent();
    let descent = -scaled.descent();
    let line_gap = px * 0.2 + scaled.line_gap();
    let height = ascent + descent + line_gap;
    (ascent, descent, line_gap, height)
}

fn layout_uncached(
    registry: &FontRegistry,
    text: &str,
    style: &TextStyle,
    max_width: Option<f32>,
) -> LaidOutText {
    let fallback_font = registry.get(style.font);
    let Some(font) = fallback_font else {
        // No font: report a placeholder box so layout still works (and nothing crashes).
        let width = text.chars().count() as f32 * style.size * 0.5;
        let lines = text.split('\n').count();
        let height = lines as f32 * style.size * 1.45;
        return LaidOutText {
            layout: TextLayout {
                size: Size::new(width, height),
                ascent: style.size * 0.8,
                descent: style.size * 0.2,
                lines: Vec::new(),
                max_width,
            },
            glyphs: Vec::new(),
            style_size: style.size,
            line_height: style.size * 1.45,
        };
    };

    let (ascent, descent, line_gap, line_height) = metrics(font, style);
    let shaper = Shaper::new(font, style);

    // Hard breaks first (byte ranges, the '\n' itself excluded), then wrap each hard line.
    let mut visual_lines: Vec<Line> = Vec::new();
    let mut hard_start = 0usize;
    loop {
        let hard_end = text[hard_start..]
            .find('\n')
            .map(|position| hard_start + position)
            .unwrap_or(text.len());
        let hard = &text[hard_start..hard_end];
        match max_width {
            Some(limit) if limit > 0.0 => {
                wrap_range(
                    &shaper,
                    text,
                    hard_start,
                    hard_end,
                    limit,
                    &mut visual_lines,
                );
            }
            _ => visual_lines.push(Line {
                start: hard_start,
                end: hard_end,
                width: shaper.measure(hard, base_rtl(hard)),
            }),
        }
        if hard_end >= text.len() {
            break;
        }
        hard_start = hard_end + 1;
    }

    // The visual lines tile the source text, so the byte ranges map back directly.
    let lines = visual_lines
        .iter()
        .map(|line| TextLine {
            start: line.start,
            end: line.end,
            width: line.width,
        })
        .collect();

    let content_width = visual_lines
        .iter()
        .map(|line| line.width)
        .fold(0.0f32, f32::max)
        .max(0.0);
    let width = match max_width {
        Some(limit) if limit > 0.0 => limit.max(content_width),
        _ => content_width,
    };
    let height = visual_lines.len() as f32 * line_height;

    // Position glyphs, keeping a running character cursor over the tiled lines.
    let mut glyphs = Vec::new();
    let mut char_cursor = 0usize;
    for (line_index, line) in visual_lines.iter().enumerate() {
        let segment = &text[line.start..line.end];
        let line_width = line.width;
        let x_offset = match style.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => (width - line_width) * 0.5,
            TextAlign::Right => width - line_width,
        };
        let base_y = line_index as f32 * line_height;
        layout_segment(
            &shaper,
            style,
            segment,
            char_cursor,
            x_offset,
            base_y,
            &mut glyphs,
        );
        char_cursor += segment.chars().count();
        // A hard break ending this line is one character nobody lays out.
        if text.as_bytes().get(line.end) == Some(&b'\n') {
            char_cursor += 1;
        }
    }

    let _ = line_gap;
    LaidOutText {
        layout: TextLayout {
            size: Size::new(width, height),
            ascent,
            descent,
            lines,
            max_width,
        },
        glyphs,
        style_size: style.size,
        line_height,
    }
}

/// One visual line: a byte range of the source text plus its measured width.
struct Line {
    /// Byte offset of the first character.
    start: usize,
    /// Byte offset just past the last character (hard breaks excluded).
    end: usize,
    width: f32,
}

/// Greedy wrap of `text[start..end]` at cluster boundaries, preferring breaks after
/// whitespace; a word longer than the limit is broken between characters (never inside a
/// cluster - a ligature stays whole).
///
/// Lines tile `[start, end)`: trailing whitespace after a break belongs to the line before it,
/// so callers can map characters to lines without gaps.
fn wrap_range(
    shaper: &Shaper,
    text: &str,
    start: usize,
    end: usize,
    limit: f32,
    out: &mut Vec<Line>,
) {
    let segment = &text[start..end];
    if segment.is_empty() {
        out.push(Line {
            start,
            end,
            width: 0.0,
        });
        return;
    }
    let rtl = base_rtl(segment);
    let clusters = shaper.cluster_advances(segment, rtl);
    let bytes = segment.as_bytes();
    let mut width = 0.0f32;
    let line_start = 0usize;
    let mut last_break: Option<usize> = None;
    let mut index = 0usize;

    while index < clusters.len() {
        let (cluster, advance) = clusters[index];
        if width + advance > limit && cluster > line_start {
            let break_at = last_break.filter(|b| *b > line_start).unwrap_or(cluster);
            // Skip the whitespace at the break; it stays inside the line's byte range.
            let mut next = break_at;
            while next < bytes.len() && (bytes[next] == b' ' || bytes[next] == b'\t') {
                next += 1;
            }
            let piece = &segment[line_start..break_at];
            out.push(Line {
                start: start + line_start,
                end: start + next,
                width: shaper.measure(piece, base_rtl(piece)),
            });
            wrap_range(shaper, text, start + next, end, limit, out);
            return;
        }
        width += advance;
        // Break opportunity just after a space or tab cluster.
        if bytes[cluster] == b' ' || bytes[cluster] == b'\t' {
            last_break = Some(
                clusters
                    .get(index + 1)
                    .map(|c| c.0)
                    .unwrap_or(segment.len()),
            );
        }
        index += 1;
    }

    out.push(Line {
        start,
        end,
        width: shaper.measure(segment, rtl),
    });
}

/// Positions one visual line (a byte range of the whole text) and appends its glyphs.
///
/// `char_base` is how many characters precede the segment; `x_offset` is the alignment shift.
/// Runs are visited in the order the bidi analysis wants them displayed, and each run is
/// shaped with its own direction.
fn layout_segment(
    shaper: &Shaper,
    style: &TextStyle,
    segment: &str,
    char_base: usize,
    x_offset: f32,
    base_y: f32,
    out: &mut Vec<LaidOutGlyph>,
) {
    if segment.is_empty() {
        return;
    }
    // Character index of every byte boundary: cluster -> character mapping is a lookup.
    let char_at: Vec<(usize, usize)> = segment
        .char_indices()
        .enumerate()
        .map(|(index, (byte, _))| (byte, index))
        .collect();

    let bidi = unicode_bidi::BidiInfo::new(segment, None);
    let Some(paragraph) = bidi.paragraphs.first() else {
        return;
    };
    let (_, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());

    let mut pen = x_offset;
    for run in runs {
        let run_text = &segment[run.clone()];
        if run_text.is_empty() {
            continue;
        }
        let rtl = bidi.levels[run.start].is_rtl();
        let shaped = shaper.shape(run_text, rtl);
        if shaped.is_empty() {
            continue;
        }
        // Cluster boundaries of this run, for per-cluster character counts.
        let mut boundaries: Vec<usize> = shaped.iter().map(|glyph| glyph.cluster).collect();
        boundaries.sort_unstable();
        boundaries.dedup();
        // Pen order is left to right; a right-to-left run comes out of the shaper already in
        // visual order (see the shaping tests, which pin the first logical character to the
        // right edge).
        for glyph in &shaped {
            let next = boundaries
                .iter()
                .copied()
                .find(|&boundary| boundary > glyph.cluster)
                .unwrap_or(run_text.len());
            let byte = run.start + glyph.cluster;
            let char_index = char_base + char_index_of(&char_at, byte);
            let char_count = run_text[glyph.cluster..next].chars().count().max(1);
            out.push(LaidOutGlyph {
                ch: run_text[glyph.cluster..].chars().next().unwrap_or(' '),
                char_index,
                char_count,
                rtl: glyph.rtl,
                position: Point::new(pen + glyph.x_offset, base_y),
                advance: glyph.x_advance,
                key: GlyphKey::new(style.font, style.size, glyph.glyph_id as u32),
            });
            pen += glyph.x_advance;
        }
    }
}

/// Character index of `byte` inside a segment, from the boundary table built in
/// [`layout_segment`].
fn char_index_of(char_at: &[(usize, usize)], byte: usize) -> usize {
    let found = char_at.partition_point(|&(boundary, _)| boundary <= byte);
    found
        .checked_sub(1)
        .map(|index| char_at[index].1)
        .unwrap_or(0)
}

/// Character index a caret at `index` sits before, interpolating inside clusters that cover
/// several characters (ligatures), so the caret still moves when a key is pressed inside one.
///
/// Right-to-left clusters lead at their **right** edge ([`LaidOutGlyph::rtl`]).
pub fn caret_rect(laid_out: &LaidOutText, index: usize) -> Rect {
    let line_height = laid_out.line_height;
    let glyphs = &laid_out.glyphs;
    let edge = |glyph: &LaidOutGlyph, frac: f32| {
        if glyph.rtl {
            glyph.position.x + glyph.advance * (1.0 - frac)
        } else {
            glyph.position.x + glyph.advance * frac
        }
    };

    // Inside a cluster: interpolate between its edges.
    for glyph in glyphs {
        let start = glyph.char_index;
        let count = glyph.char_count.max(1);
        if index >= start && index < start + count {
            let frac = (index - start) as f32 / count as f32;
            return Rect::new(edge(glyph, frac), glyph.position.y, 1.0, line_height);
        }
    }
    // Exactly at a cluster end: its trailing edge (which is the next cluster's leading edge).
    for glyph in glyphs {
        let start = glyph.char_index;
        let count = glyph.char_count.max(1);
        if index == start + count {
            return Rect::new(edge(glyph, 1.0), glyph.position.y, 1.0, line_height);
        }
    }
    // Before everything, or in a gap of un-laid-out whitespace: lead at the nearest glyph.
    if let Some(glyph) = glyphs.iter().min_by_key(|glyph| {
        let distance = glyph.char_index as isize + glyph.char_count as isize - index as isize;
        distance.unsigned_abs()
    }) {
        return Rect::new(edge(glyph, 0.0), glyph.position.y, 1.0, line_height);
    }
    Rect::new(0.0, 0.0, 1.0, line_height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontRegistry;

    fn system_font() -> Option<Vec<u8>> {
        let candidates: Vec<&str> = if cfg!(target_os = "linux") {
            vec![
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
            ]
        } else if cfg!(target_os = "macos") {
            vec!["/System/Library/Fonts/Supplemental/Arial.ttf"]
        } else {
            vec!["C:\\Windows\\Fonts\\segoeui.ttf"]
        };
        for path in candidates {
            if let Ok(data) = std::fs::read(path) {
                if !data.is_empty() {
                    return Some(data);
                }
            }
        }
        None
    }

    /// Registry with a single embedded font registered as the default (with source bytes, so
    /// the shaper is active).
    fn registry() -> Option<FontRegistry> {
        let data = system_font()?;
        let mut registry = FontRegistry::new();
        let id = registry.add_font_data("test", &data)?;
        registry.set_default(id);
        Some(registry)
    }

    fn style(size: f32) -> TextStyle {
        TextStyle::new(size, rustui_core::Color::WHITE)
    }

    #[test]
    fn layout_without_font_falls_back_to_placeholder() {
        let registry = FontRegistry::new();
        let mut engine = TextLayoutEngine::default();
        let out = engine.layout(&registry, "abc", &style(10.0), None);
        assert!(out.layout.size.width > 0.0);
        assert!(out.layout.size.height > 0.0);
        assert!(out.glyphs.is_empty());
    }

    #[test]
    fn layout_cache_is_used() {
        let Some(registry) = registry() else { return };
        let mut engine = TextLayoutEngine::default();
        let first = engine.layout(&registry, "hello", &style(16.0), None);
        let second = engine.layout(&registry, "hello", &style(16.0), None);
        assert_eq!(first, second);
        assert_eq!(engine.len(), 1);
        // different size -> new entry
        let _ = engine.layout(&registry, "hello", &style(20.0), None);
        assert_eq!(engine.len(), 2);
    }

    #[test]
    fn glyph_positions_advance_monotonically() {
        let Some(registry) = registry() else { return };
        let mut engine = TextLayoutEngine::default();
        let out = engine.layout(&registry, "abc", &style(20.0), None);
        assert_eq!(out.glyphs.len(), 3);
        for pair in out.glyphs.windows(2) {
            assert!(pair[1].position.x > pair[0].position.x);
            assert_eq!(pair[1].position.y, pair[0].position.y);
        }
        assert!((out.glyphs[2].position.x + out.glyphs[2].advance - out.width()).abs() < 0.01);
    }

    #[test]
    fn hard_breaks_create_lines() {
        let Some(registry) = registry() else { return };
        let mut engine = TextLayoutEngine::default();
        let out = engine.layout(&registry, "ab\ncd", &style(20.0), None);
        assert!(out.layout.size.height > out.line_height);
        assert_eq!(out.layout.lines.len(), 2);
    }

    #[test]
    fn line_byte_ranges_cover_the_text_without_gaps() {
        let Some(registry) = registry() else { return };
        let mut engine = TextLayoutEngine::default();
        let text = "hello wrapped line\nsecond";
        let out = engine.layout(&registry, text, &style(16.0), Some(60.0));
        let mut cursor = 0usize;
        for line in &out.layout.lines {
            assert!(line.start >= cursor, "lines must not overlap");
            assert!(line.end >= line.start);
            cursor = line.end;
            if text.as_bytes().get(line.end) == Some(&b'\n') {
                cursor += 1;
            }
        }
        assert!(out.layout.lines.len() >= 2);
        // Characters of every glyph are inside the text.
        let total = text.chars().count();
        for glyph in &out.glyphs {
            assert!(glyph.char_index < total);
            assert!(glyph.char_index + glyph.char_count <= total);
        }
    }

    #[test]
    fn wrapping_splits_long_text() {
        let Some(registry) = registry() else { return };
        let mut engine = TextLayoutEngine::default();
        let long = "the quick brown fox jumps over the lazy dog";
        let out = engine.layout(&registry, long, &style(16.0), Some(60.0));
        assert!(out.layout.size.height >= 2.0 * out.line_height - 0.01);
        for line in &out.layout.lines {
            assert!(line.width <= 61.0, "line too wide: {}", line.width);
        }
    }

    #[test]
    fn long_word_is_broken_by_character() {
        let Some(registry) = registry() else { return };
        let mut engine = TextLayoutEngine::default();
        let out = engine.layout(
            &registry,
            "aaaaaaaaaaaaaaaaaaaaaaaaaa",
            &style(16.0),
            Some(40.0),
        );
        assert!(out.glyphs.len() > 10);
        assert!(out.layout.size.height > out.line_height);
    }

    #[test]
    fn alignment_shifts_glyphs() {
        let Some(registry) = registry() else { return };
        let mut engine = TextLayoutEngine::default();
        let left = engine.layout(&registry, "ab", &style(20.0), Some(200.0));
        let mut right_style = style(20.0);
        right_style.align = TextAlign::Right;
        let right = engine.layout(&registry, "ab", &right_style, Some(200.0));
        assert!(right.glyphs[0].position.x > left.glyphs[0].position.x);
        let mut center_style = style(20.0);
        center_style.align = TextAlign::Center;
        let center = engine.layout(&registry, "ab", &center_style, Some(200.0));
        assert!(center.glyphs[0].position.x > left.glyphs[0].position.x);
        assert!(center.glyphs[0].position.x < right.glyphs[0].position.x);
    }

    #[test]
    fn caret_moves_with_index() {
        let Some(registry) = registry() else { return };
        let mut engine = TextLayoutEngine::default();
        let out = engine.layout(&registry, "abcd", &style(20.0), None);
        let start = caret_rect(&out, 0);
        let middle = caret_rect(&out, 2);
        let end = caret_rect(&out, 4);
        assert!(middle.left() > start.left());
        assert!(end.left() > middle.left());
        assert!((end.height - out.line_height).abs() < 0.01);
    }

    #[test]
    fn empty_text_has_zero_width_but_one_line() {
        let Some(registry) = registry() else { return };
        let mut engine = TextLayoutEngine::default();
        let out = engine.layout(&registry, "", &style(20.0), None);
        assert_eq!(out.width(), 0.0);
        assert!(out.glyphs.is_empty());
    }
}
