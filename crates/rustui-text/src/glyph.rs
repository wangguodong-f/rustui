//! Glyph rasterization and the dynamically growing glyph atlas.
//!
//! ```text
//! Text -> TextLayout -> Glyph -> GlyphAtlas (R8 coverage) -> wgpu texture
//! ```
//!
//! The atlas is packed with a simple shelf allocator and is owned by the text engine; the renderer
//! copies it out through [`rustui_core::GlyphSource::copy_atlas_into`] only when it is dirty, so
//! there is exactly one long-lived copy of the bitmap in the system (design document §25).

use std::collections::HashMap;

use ab_glyph::{Font, ScaleFont};

use rustui_core::{FontId, TextStyle};

/// Rasterized glyph metrics and coverage bitmap.
#[derive(Clone, Debug, PartialEq)]
pub struct RasterizedGlyph {
    /// Coverage bitmap, `width * height` bytes.
    pub bitmap: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Offset from the pen position to the top-left corner of the bitmap.
    pub bearing_x: f32,
    pub bearing_y: f32,
    /// Horizontal advance.
    pub advance: f32,
}

impl RasterizedGlyph {
    /// A blank glyph (whitespace or `.notdef`).
    pub fn blank(advance: f32) -> Self {
        Self {
            bitmap: Vec::new(),
            width: 0,
            height: 0,
            bearing_x: 0.0,
            bearing_y: 0.0,
            advance,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0 || self.bitmap.is_empty()
    }
}

/// Key identifying a rasterized glyph in the cache / atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlyphKey {
    pub font: FontId,
    /// Size in 1/64 pixel units so that small float differences do not create duplicate entries.
    pub size_64: u64,
    pub glyph: u32,
}

impl GlyphKey {
    pub fn new(font: FontId, size_px: f32, glyph: u32) -> Self {
        let size_64 = (size_px.max(1.0) * 64.0).round() as u64;
        Self {
            font,
            size_64,
            glyph,
        }
    }

    /// Logical font size the key was created with.
    pub fn size_px(&self) -> f32 {
        self.size_64 as f32 / 64.0
    }

    /// The key of the same glyph rasterized at `scale` - the cache stores rasters in **physical**
    /// pixels, so one logical key has one entry per display scale and they coexist (C-048).
    ///
    /// Layout produces logical keys; every cache lookup and insert must go through this first.
    /// At scale 1.0 the key is returned unchanged, and two pairs that land on the same physical
    /// size (14px @ 2.0 and 28px @ 1.0) share a single - identical - raster.
    pub fn at_scale(&self, scale: f32) -> Self {
        let scale = scale.max(0.01);
        let size_64 = (((self.size_64 as f64) * f64::from(scale)).round() as u64).max(64);
        Self {
            font: self.font,
            size_64,
            glyph: self.glyph,
        }
    }

    /// Index of the glyph inside the font (`0` is `.notdef`).
    pub fn base_glyph(&self) -> u32 {
        self.glyph
    }
}

/// Rectangular region inside the atlas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl AtlasRect {
    /// Normalised texture coordinates.
    pub fn uv(&self, atlas_width: u32, atlas_height: u32) -> [f32; 4] {
        let w = atlas_width.max(1) as f32;
        let h = atlas_height.max(1) as f32;
        [
            self.x as f32 / w,
            self.y as f32 / h,
            (self.x + self.width) as f32 / w,
            (self.y + self.height) as f32 / h,
        ]
    }
}

/// A shelf of the shelf packer.
#[derive(Clone, Copy, Debug)]
struct Shelf {
    /// Y offset of the shelf.
    y: u32,
    height: u32,
    /// X offset of the next free region.
    x: u32,
}

/// Single channel glyph coverage atlas.
#[derive(Debug)]
pub struct GlyphAtlas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    shelves: Vec<Shelf>,
    /// Region of `pixels` that changed since the last upload.
    dirty: Option<(u32, u32, u32, u32)>,
    /// Bumped whenever the atlas is reallocated, so the renderer can drop stale bind groups.
    generation: u32,
    /// Maximum region height, used when growing.
    used_height: u32,
}

impl Default for GlyphAtlas {
    fn default() -> Self {
        Self::new(512, 512)
    }
}

impl GlyphAtlas {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; (width * height) as usize],
            shelves: Vec::new(),
            dirty: Some((0, 0, width, height)),
            generation: 0,
            used_height: 0,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.is_some()
    }

    /// Marks the atlas as clean (called by the renderer after uploading).
    pub fn clear_dirty(&mut self) {
        self.dirty = None;
    }

    /// Fraction of the atlas that is used, for diagnostics.
    pub fn occupancy(&self) -> f32 {
        self.used_height as f32 / self.height.max(1) as f32
    }

    /// Allocates a region of `width x height` pixels and copies `bitmap` into it.
    ///
    /// Returns `None` when the atlas is full; the caller then asks the cache to evict and retries.
    pub fn insert(&mut self, bitmap: &[u8], width: u32, height: u32) -> Option<AtlasRect> {
        if width == 0 || height == 0 {
            return Some(AtlasRect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            });
        }
        let rect = match self.allocate(width, height) {
            Some(rect) => rect,
            None => {
                self.grow();
                self.allocate(width, height)?
            }
        };
        for row in 0..height {
            let src = (row * width) as usize;
            let dst = ((rect.y + row) * self.width + rect.x) as usize;
            let len = width as usize;
            self.pixels[dst..dst + len].copy_from_slice(&bitmap[src..src + len]);
        }
        self.mark_dirty(rect.x, rect.y, width, height);
        Some(rect)
    }

    fn mark_dirty(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.dirty = Some(match self.dirty {
            None => (x, y, w, h),
            Some((dx, dy, dw, dh)) => {
                let x0 = dx.min(x);
                let y0 = dy.min(y);
                let x1 = (dx + dw).max(x + w);
                let y1 = (dy + dh).max(y + h);
                (x0, y0, x1 - x0, y1 - y0)
            }
        });
    }

    /// Region that changed since the last `clear_dirty`, if any.
    pub fn dirty_rect(&self) -> Option<AtlasRect> {
        self.dirty.map(|(x, y, w, h)| AtlasRect {
            x,
            y,
            width: w,
            height: h,
        })
    }

    fn allocate(&mut self, width: u32, height: u32) -> Option<AtlasRect> {
        for shelf in self.shelves.iter_mut() {
            if shelf.height >= height && shelf.x + width <= self.width {
                let rect = AtlasRect {
                    x: shelf.x,
                    y: shelf.y,
                    width,
                    height,
                };
                shelf.x += width;
                return Some(rect);
            }
        }
        let y = self.used_height;
        if y + height > self.height || width > self.width {
            return None;
        }
        self.shelves.push(Shelf {
            y,
            height,
            x: width,
        });
        self.used_height = y + height;
        Some(AtlasRect {
            x: 0,
            y,
            width,
            height,
        })
    }

    /// Doubles the atlas size and reallocates, keeping existing pixels.
    fn grow(&mut self) {
        let new_width = (self.width * 2).min(4096);
        let new_height = (self.height * 2).min(4096);
        if new_width == self.width && new_height == self.height {
            return;
        }
        let mut pixels = vec![0; (new_width * new_height) as usize];
        for row in 0..self.used_height.min(self.height) {
            let src = (row * self.width) as usize;
            let dst = (row * new_width) as usize;
            let len = self.width as usize;
            pixels[dst..dst + len].copy_from_slice(&self.pixels[src..src + len]);
        }
        self.pixels = pixels;
        self.width = new_width;
        self.height = new_height;
        self.generation += 1;
        self.dirty = Some((0, 0, new_width, new_height));
    }
}

/// Cache of rasterized glyphs plus their atlas locations.
#[derive(Debug)]
pub struct GlyphCache {
    glyphs: HashMap<GlyphKey, RasterizedGlyph>,
    regions: HashMap<GlyphKey, AtlasRect>,
    /// Maximum number of rasterized glyphs kept around.
    capacity: usize,
    atlas: GlyphAtlas,
    stats_rasterized: u64,
    stats_hits: u64,
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new(4096)
    }
}

impl GlyphCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            glyphs: HashMap::new(),
            regions: HashMap::new(),
            capacity,
            atlas: GlyphAtlas::default(),
            stats_rasterized: 0,
            stats_hits: 0,
        }
    }

    /// The atlas backing this cache.
    pub fn atlas(&self) -> &GlyphAtlas {
        &self.atlas
    }

    /// Mutable access to the atlas (used by tests).
    pub fn atlas_mut(&mut self) -> &mut GlyphAtlas {
        &mut self.atlas
    }

    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    pub fn stats_rasterized(&self) -> u64 {
        self.stats_rasterized
    }

    pub fn stats_hits(&self) -> u64 {
        self.stats_hits
    }

    /// Atlas region of a cached glyph.
    pub fn region(&self, key: &GlyphKey) -> Option<AtlasRect> {
        self.regions.get(key).copied()
    }

    /// Rasterized glyph, if cached.
    pub fn glyph(&self, key: &GlyphKey) -> Option<&RasterizedGlyph> {
        self.glyphs.get(key)
    }

    /// `true` when the key is present in both caches.
    pub fn contains(&self, key: &GlyphKey) -> bool {
        self.glyphs.contains_key(key) && self.regions.contains_key(key)
    }

    /// Inserts a rasterized glyph into the cache (without touching the atlas).
    pub fn insert_glyph(&mut self, key: GlyphKey, glyph: RasterizedGlyph) {
        if self.glyphs.len() >= self.capacity {
            self.evict_one();
        }
        self.glyphs.insert(key, glyph);
    }

    /// Uploads a glyph into the atlas, returning its region.
    pub fn upload(&mut self, key: GlyphKey, glyph: &RasterizedGlyph) -> Option<AtlasRect> {
        if glyph.is_empty() {
            let rect = AtlasRect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            };
            self.regions.insert(key, rect);
            return Some(rect);
        }
        let rect = self
            .atlas
            .insert(&glyph.bitmap, glyph.width, glyph.height)?;
        self.regions.insert(key, rect);
        Some(rect)
    }

    /// Records a cache hit (statistics only).
    pub fn record_hit(&mut self) {
        self.stats_hits += 1;
    }

    /// Records a rasterization (statistics only).
    pub fn record_rasterized(&mut self) {
        self.stats_rasterized += 1;
    }

    fn evict_one(&mut self) {
        // Simple "clear everything when full" policy: cheap, predictable, and the atlas keeps
        // its pixels, so re-rasterized glyphs simply find their region again.
        if self.glyphs.len() < self.capacity {
            return;
        }
        self.glyphs.clear();
    }
}

/// Rasterizes `glyph_id` from `scaled`.
///
/// The glyph is outlined at the scale of `scaled` and positioned at the pen origin, so the
/// resulting `bearing_x` / `bearing_y` describe the offset from the pen (and from the baseline,
/// for `bearing_y`) to the top-left corner of the coverage bitmap.
pub fn rasterize<F: Font, S: ScaleFont<F>>(
    scaled: &S,
    glyph_id: ab_glyph::GlyphId,
) -> RasterizedGlyph {
    let advance = scaled.h_advance(glyph_id);
    if glyph_id.0 == 0 {
        // `.notdef` is drawn as a hollow box so that missing glyphs are obvious.
        return notdef_glyph(scaled, advance);
    }
    let glyph = glyph_id.with_scale_and_position(scaled.scale(), ab_glyph::point(0.0, 0.0));
    let Some(outlined) = scaled.font().outline_glyph(glyph) else {
        // No outline at all (typically whitespace): advance only.
        return RasterizedGlyph::blank(advance);
    };
    let bounds = outlined.px_bounds();
    let width = bounds.width().round().max(0.0) as u32;
    let height = bounds.height().round().max(0.0) as u32;
    if width == 0 || height == 0 {
        return RasterizedGlyph::blank(advance);
    }
    let mut bitmap = vec![0u8; (width * height) as usize];
    outlined.draw(|x, y, coverage| {
        let index = y as usize * width as usize + x as usize;
        let value = (coverage * 255.0).round().clamp(0.0, 255.0) as u8;
        if value > bitmap[index] {
            bitmap[index] = value;
        }
    });
    RasterizedGlyph {
        bitmap,
        width,
        height,
        // Offset from the pen position to the left edge of the bitmap.
        bearing_x: bounds.min.x,
        // Distance from the baseline up to the top edge of the bitmap (y grows downwards).
        bearing_y: -bounds.min.y,
        advance,
    }
}

fn notdef_glyph<F: Font, S: ScaleFont<F>>(scaled: &S, advance: f32) -> RasterizedGlyph {
    let size = scaled.height().max(2.0);
    let width = (size * 0.5).round().max(1.0) as u32;
    let height = size.round().max(1.0) as u32;
    let mut bitmap = vec![0u8; (width * height) as usize];
    let thickness = (width as f32 * 0.15).round().max(1.0) as u32;
    for x in 0..width {
        for y in 0..height {
            let border =
                x < thickness || y < thickness || x >= width - thickness || y >= height - thickness;
            if border {
                bitmap[(y * width + x) as usize] = 255;
            }
        }
    }
    RasterizedGlyph {
        bitmap,
        width,
        height,
        bearing_x: 0.0,
        bearing_y: size * 0.8,
        advance,
    }
}

/// Rasterization scale for a logical size and a display scale factor.
pub fn raster_scale(size_px: f32, device_scale: f32) -> f32 {
    size_px * device_scale.clamp(1.0, 4.0)
}

/// Style-independent cache key helper.
pub fn glyph_key(font: FontId, style: &TextStyle, glyph: ab_glyph::GlyphId) -> GlyphKey {
    GlyphKey::new(font, style.size, glyph.0 as u32)
}

/// Convenience: renders `text` with `font` at `px` into a coverage buffer.
///
/// Only used by tests and by the (optional) offscreen screenshot support.
pub fn render_text(font: &ab_glyph::FontVec, text: &str, px: f32) -> Vec<u8> {
    let scaled = font.as_scaled(px);
    let mut pen = 0.0f32;
    let mut out = Vec::new();
    for ch in text.chars() {
        let glyph = rasterize(&scaled, font.glyph_id(ch));
        out.extend_from_slice(&glyph.bitmap);
        pen += glyph.advance;
    }
    let _ = pen;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_font() -> Option<ab_glyph::FontVec> {
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
                if let Ok(font) = ab_glyph::FontVec::try_from_vec(data) {
                    return Some(font);
                }
            }
        }
        None
    }

    #[test]
    fn atlas_allocates_regions_without_overlap() {
        let mut atlas = GlyphAtlas::new(64, 64);
        let bitmap = vec![255u8; 4 * 4];
        let a = atlas.insert(&bitmap, 4, 4).unwrap();
        let b = atlas.insert(&bitmap, 4, 4).unwrap();
        assert_eq!((a.x, a.y), (0, 0));
        assert_eq!((b.x, b.y), (4, 0));
        assert_eq!(atlas.pixels()[(a.y * 64 + a.x) as usize], 255);
    }

    #[test]
    fn atlas_grows_when_full() {
        let mut atlas = GlyphAtlas::new(32, 16);
        let bitmap = vec![128u8; 16 * 8];
        for _ in 0..10 {
            atlas.insert(&bitmap, 16, 8);
        }
        assert!(atlas.width() > 32 || atlas.height() > 16);
        assert_eq!(atlas.generation(), 1);
        // pixels survived the growth
        assert!(atlas.pixels().contains(&128));
    }

    #[test]
    fn atlas_dirty_rect_grows() {
        let mut atlas = GlyphAtlas::new(64, 64);
        atlas.clear_dirty();
        assert!(!atlas.is_dirty());
        atlas.insert(&[7u8; 4], 2, 2);
        let dirty = atlas.dirty_rect().unwrap();
        assert_eq!((dirty.x, dirty.y, dirty.width, dirty.height), (0, 0, 2, 2));
        atlas.insert(&[7u8; 4], 2, 2);
        assert_eq!(atlas.dirty_rect().unwrap().width, 4);
    }

    #[test]
    fn zero_sized_glyph_is_empty() {
        let mut atlas = GlyphAtlas::new(64, 64);
        let rect = atlas.insert(&[], 0, 0).unwrap();
        assert!(rect.is_empty_rect());
    }

    impl AtlasRect {
        fn is_empty_rect(&self) -> bool {
            self.width == 0 || self.height == 0
        }
    }

    #[test]
    fn glyph_key_quantizes_size() {
        let a = GlyphKey::new(FontId(0), 14.0, 3);
        let b = GlyphKey::new(FontId(0), 14.0001, 3);
        assert_eq!(a, b);
        assert!((a.size_px() - 14.0).abs() < 0.02);
        assert_ne!(a, GlyphKey::new(FontId(0), 15.0, 3));
    }

    #[test]
    fn at_scale_keys_rasters_by_physical_size() {
        let logical = GlyphKey::new(FontId(0), 14.0, 3);
        // Scale 1.0 is the identity: logical and physical keys coincide.
        assert_eq!(logical.at_scale(1.0), logical);

        // A 14px glyph on a 2x screen rasterizes at 28px - a different cache entry...
        let two_x = logical.at_scale(2.0);
        assert_eq!(two_x.size_px(), 28.0);
        assert_ne!(two_x, logical);

        // ...that is shared with anything else landing on the same physical size (C-048).
        assert_eq!(two_x, GlyphKey::new(FontId(0), 28.0, 3));

        // Sub-pixel differences survive quantization at both scales.
        assert_eq!(GlyphKey::new(FontId(0), 14.0001, 3).at_scale(2.0), two_x);
        assert_ne!(logical.at_scale(1.5), two_x);
    }

    #[test]
    fn rasterize_produces_coverage_for_a_real_glyph() {
        let Some(font) = test_font() else { return };
        let scaled = font.as_scaled(32.0);
        let glyph = rasterize(&scaled, font.glyph_id('A'));
        assert!(glyph.width > 0 && glyph.height > 0);
        assert_eq!(glyph.bitmap.len(), (glyph.width * glyph.height) as usize);
        assert!(glyph.bitmap.iter().any(|p| *p > 0));
        assert!(glyph.advance > 0.0);
    }

    #[test]
    fn whitespace_glyphs_are_blank_but_have_advance() {
        let Some(font) = test_font() else { return };
        let scaled = font.as_scaled(20.0);
        let glyph = rasterize(&scaled, font.glyph_id(' '));
        assert!(glyph.is_empty());
        assert!(glyph.advance > 0.0);
    }

    #[test]
    fn cache_roundtrip() {
        let mut cache = GlyphCache::new(8);
        let key = GlyphKey::new(FontId(1), 12.0, 7);
        let glyph = RasterizedGlyph {
            bitmap: vec![255; 4],
            width: 2,
            height: 2,
            bearing_x: 0.0,
            bearing_y: 0.0,
            advance: 5.0,
        };
        cache.insert_glyph(key, glyph.clone());
        assert_eq!(cache.glyph(&key), Some(&glyph));
        let region = cache.upload(key, &glyph).unwrap();
        assert_eq!(cache.region(&key), Some(region));
        assert!(cache.contains(&key));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn raster_scale_is_clamped() {
        assert_eq!(raster_scale(10.0, 1.0), 10.0);
        assert_eq!(raster_scale(10.0, 2.0), 20.0);
        assert_eq!(raster_scale(10.0, 0.5), 10.0);
        assert_eq!(raster_scale(10.0, 99.0), 40.0);
    }
}
