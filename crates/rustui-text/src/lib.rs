//! # rustui-text
//!
//! Text engine of RustUI (design document §25):
//!
//! ```text
//! Text -> TextLayout -> Glyph -> GlyphAtlas -> wgpu texture
//! ```
//!
//! The engine owns the font database, the glyph rasterization cache, the glyph atlas and the
//! layout cache. It implements [`rustui_core::TextMeasurer`] (used by widgets) and
//! [`rustui_core::GlyphSource`] (used by the renderer), which is why neither `rustui-widgets` nor
//! `rustui-renderer` needs to know about `ab_glyph` or `fontdb`.

use std::cell::RefCell;
use std::rc::Rc;

use ab_glyph::ScaleFont as _;

use rustui_core::{
    AtlasInfo, FontId, GlyphSource, Point, PositionedGlyph, Rect, Result, Size, TextLayout,
    TextMeasurer, TextStyle,
};

pub mod font;
pub mod glyph;
pub mod layout;
pub mod shape;

pub use font::{FontRegistry, LoadedFont};
pub use glyph::{AtlasRect, GlyphAtlas, GlyphCache, GlyphKey, RasterizedGlyph};
pub use layout::{caret_rect, LaidOutGlyph, LaidOutText, TextLayoutEngine};
pub use shape::{ShapedGlyph, Shaper};

use crate::glyph::rasterize;

/// Interior state of the engine, kept behind a `RefCell` so that the engine itself can be handed
/// out as `&dyn TextMeasurer` / `&dyn GlyphSource` (both trait methods take `&self`).
struct EngineState {
    registry: FontRegistry,
    cache: GlyphCache,
    layout: TextLayoutEngine,
    /// Current display scale factor used for rasterization.
    scale: f32,
    atlas_texture: u32,
}

/// The text engine.
///
/// Handles are cheap: cloning shares one state (it only bumps a reference count), so a caller can
/// keep a handle while handing another to a `WindowManager`. The multi-DPI tests use it to read
/// the raster statistics of the very engine the windows render with.
#[derive(Clone)]
pub struct TextEngine {
    state: Rc<RefCell<EngineState>>,
}

impl std::fmt::Debug for TextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.borrow();
        f.debug_struct("TextEngine")
            .field("scale", &state.scale)
            .field("cached_glyphs", &state.cache.len())
            .field("cached_layouts", &state.layout.len())
            .field(
                "atlas",
                &(state.cache.atlas().width(), state.cache.atlas().height()),
            )
            .finish()
    }
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    /// Creates an engine with the system fonts.
    pub fn new() -> TextEngine {
        let mut registry = FontRegistry::new();
        let _ = registry.load_system_fonts();
        let _ = registry.load_default();
        let _ = registry.load_monospace();
        TextEngine {
            state: Rc::new(RefCell::new(EngineState {
                registry,
                cache: GlyphCache::default(),
                layout: TextLayoutEngine::default(),
                scale: 1.0,
                atlas_texture: 1,
            })),
        }
    }

    /// Creates an engine that uses an embedded font (no system fonts required).
    pub fn with_embedded_font(data: &[u8], family: &str) -> Result<TextEngine> {
        let engine = TextEngine::new();
        {
            let mut state = engine.state.borrow_mut();
            let id = state
                .registry
                .add_font_data(family, data)
                .ok_or_else(|| rustui_core::UiError::FontDatabase("invalid font data".into()))?;
            state.registry.set_default(id);
        }
        Ok(engine)
    }

    /// Registers an embedded font, returning its id. The bytes are kept for the shaper (T-9).
    pub fn add_font(&self, family: &str, data: &[u8]) -> Result<FontId> {
        self.state
            .borrow_mut()
            .registry
            .add_font_data(family, data)
            .ok_or_else(|| rustui_core::UiError::FontDatabase("invalid font data".into()))
    }

    /// Font registry (immutable).
    pub fn fonts(&self) -> std::cell::Ref<'_, FontRegistry> {
        std::cell::Ref::map(self.state.borrow(), |s| &s.registry)
    }

    /// Sets the display scale factor used for glyph rasterization (§41).
    ///
    /// Nothing is invalidated: rasters are keyed by their physical size (C-048), so windows with
    /// different DPI keep their own entries side by side and switching between them is free.
    pub fn set_scale_factor(&self, scale: f32) {
        let mut state = self.state.borrow_mut();
        let scale = scale.clamp(0.5, 4.0);
        if (state.scale - scale).abs() > f32::EPSILON {
            state.scale = scale;
        }
    }

    /// Current display scale factor.
    pub fn scale_factor(&self) -> f32 {
        self.state.borrow().scale
    }

    /// Lays out `text` with the full pipeline (line breaks + glyph positions).
    pub fn layout_text_full(
        &self,
        text: &str,
        style: &TextStyle,
        max_width: Option<f32>,
    ) -> LaidOutText {
        let mut state = self.state.borrow_mut();
        let EngineState {
            registry, layout, ..
        } = &mut *state;
        layout.layout(registry, text, style, max_width)
    }

    /// Ensures every glyph of `text` is rasterized and present in the atlas.
    pub fn prepare(&self, text: &str, style: &TextStyle, max_width: Option<f32>) {
        let laid_out = self.layout_text_full(text, style, max_width);
        let scale = self.scale_factor();
        let mut state = self.state.borrow_mut();
        for glyph in laid_out.glyphs {
            ensure_glyph(&mut state, glyph.key.at_scale(scale));
        }
    }

    /// Number of cached glyphs.
    pub fn cached_glyphs(&self) -> usize {
        self.state.borrow().cache.len()
    }

    /// Number of cached layouts.
    pub fn cached_layouts(&self) -> usize {
        self.state.borrow().layout.len()
    }

    /// Diagnostics: (glyphs rasterized, cache hits).
    pub fn stats(&self) -> (u64, u64) {
        let state = self.state.borrow();
        (state.cache.stats_rasterized(), state.cache.stats_hits())
    }

    /// Atlas occupancy in `0..1`, for diagnostics.
    pub fn atlas_occupancy(&self) -> f32 {
        self.state.borrow().cache.atlas().occupancy()
    }

    /// Drops all caches (called when the window is resized heavily or on low memory pressure).
    pub fn trim(&self) {
        let mut state = self.state.borrow_mut();
        state.cache = GlyphCache::default();
        state.layout.clear();
    }

    /// Screen-space rectangle of the line `index` of a layout, relative to `origin`.
    pub fn line_rect(&self, origin: Point, laid_out: &LaidOutText, index: usize) -> Rect {
        Rect::new(
            origin.x,
            origin.y + index as f32 * laid_out.line_height,
            laid_out.width(),
            laid_out.line_height,
        )
    }
}

impl TextMeasurer for TextEngine {
    fn layout_text(&self, text: &str, style: &TextStyle, max_width: Option<f32>) -> TextLayout {
        self.layout_text_full(text, style, max_width).layout
    }

    /// The engine doubles as its own glyph source (the [`GlyphSource`] impl below), which is how
    /// the renderer reaches the atlas.
    fn glyph_source(&self) -> Option<&dyn GlyphSource> {
        Some(self)
    }

    /// Rasterisation scale (the layout itself stays in logical pixels).
    fn set_scale_factor(&self, scale: f32) {
        TextEngine::set_scale_factor(self, scale);
    }

    fn line_height(&self, style: &TextStyle) -> f32 {
        let state = self.state.borrow();
        let Some(font) = state.registry.get(style.font) else {
            return style.size * 1.45;
        };
        let scaled = font.scaled(style.size);
        scaled.ascent() - scaled.descent() + style.size * 0.2
    }

    fn space_width(&self, style: &TextStyle) -> f32 {
        let state = self.state.borrow();
        match state.registry.get(style.font) {
            Some(font) => font.advance(' ', style.size),
            None => style.size * 0.33,
        }
    }

    /// Character index whose cluster is closest to `x` (single-line layouts).
    ///
    /// A cluster that covers several characters (a ligature) is split in half, so the result
    /// stays a usable caret position after shaping (T-9 / C-053).
    fn hit_test_x(&self, text: &str, style: &TextStyle, x: f32) -> usize {
        let laid_out = self.layout_text_full(text, style, None);
        for glyph in &laid_out.glyphs {
            if x < glyph.position.x + glyph.advance {
                let count = glyph.char_count.max(1);
                if x <= glyph.position.x {
                    return if glyph.rtl {
                        glyph.char_index + count
                    } else {
                        glyph.char_index
                    };
                }
                let fraction = (x - glyph.position.x) / glyph.advance.max(f32::EPSILON);
                let visual_bin = (fraction * count as f32).floor() as usize;
                let offset = if glyph.rtl {
                    count.saturating_sub(visual_bin + 1)
                } else {
                    visual_bin.min(count)
                };
                return glyph.char_index + offset;
            }
        }
        text.chars().count()
    }
}

impl GlyphSource for TextEngine {
    fn glyphs(
        &self,
        text: &str,
        style: &TextStyle,
        max_width: Option<f32>,
        origin: Point,
    ) -> Vec<PositionedGlyph> {
        let laid_out = self.layout_text_full(text, style, max_width);
        let scale = self.scale_factor().max(0.01);
        let mut state = self.state.borrow_mut();
        let mut out = Vec::with_capacity(laid_out.glyphs.len());
        for glyph in laid_out.glyphs {
            // Cache under the physical size this window's DPI needs; the quad below converts the
            // raster back to logical units with *this* window's scale (C-048).
            let key = glyph.key.at_scale(scale);
            let Some(rasterized) = ensure_glyph(&mut state, key) else {
                continue;
            };
            let Some(region) = state.cache.region(&key) else {
                continue;
            };
            if rasterized.is_empty() || region.width == 0 {
                continue;
            }
            out.push(PositionedGlyph {
                rect: Rect::new(
                    origin.x + glyph.position.x + rasterized.bearing_x / scale,
                    origin.y + glyph.position.y + laid_out.layout.ascent
                        - rasterized.bearing_y / scale,
                    region.width as f32 / scale,
                    region.height as f32 / scale,
                ),
                uv: region.uv(state.cache.atlas().width(), state.cache.atlas().height()),
                atlas: state.atlas_texture,
                visible: true,
            });
        }
        out
    }

    fn atlas_info(&self) -> AtlasInfo {
        let state = self.state.borrow();
        let atlas = state.cache.atlas();
        AtlasInfo {
            width: atlas.width(),
            height: atlas.height(),
            generation: atlas.generation(),
            dirty: atlas.is_dirty(),
        }
    }

    fn copy_atlas_into(&self, dst: &mut Vec<u8>) {
        let state = self.state.borrow();
        dst.clear();
        dst.extend_from_slice(state.cache.atlas().pixels());
    }

    fn mark_atlas_uploaded(&self) {
        self.state.borrow_mut().cache.atlas_mut().clear_dirty();
    }

    fn atlas_texture_index(&self) -> u32 {
        self.state.borrow().atlas_texture
    }
}

/// Rasterizes and uploads the glyph for `key` if necessary.
///
/// `key` is a **physical size** key ([`GlyphKey::at_scale`]): glyphs are rasterized at
/// `logical_size * display_scale` for crisp text on HiDPI screens, so the returned bitmap metrics
/// are in physical pixels and callers convert them back to logical units with their own window
/// scale (see [`TextEngine::glyphs`]). Because the physical size is part of the key, rasters of
/// different display scales live side by side in one cache and changing the scale invalidates
/// nothing (C-048).
fn ensure_glyph(state: &mut EngineState, key: GlyphKey) -> Option<RasterizedGlyph> {
    if let Some(existing) = state.cache.glyph(&key).cloned() {
        state.cache.record_hit();
        if state.cache.region(&key).is_none() {
            state.cache.upload(key, &existing);
        }
        return Some(existing);
    }
    let font = state.registry.get(key.font)?;
    let glyph_id = ab_glyph::GlyphId(key.base_glyph() as u16);
    let scaled = font.scaled(key.size_px());
    let rasterized = rasterize(&scaled, glyph_id);
    state.cache.record_rasterized();
    state.cache.insert_glyph(key, rasterized);
    let uploaded = state.cache.glyph(&key)?.clone();
    state.cache.upload(key, &uploaded);
    state.cache.glyph(&key).cloned()
}

/// Convenience: measures text without a `&dyn TextMeasurer` reference.
pub fn measure(engine: &TextEngine, text: &str, style: &TextStyle, max_width: Option<f32>) -> Size {
    engine.measure_text(text, style, max_width)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn engine() -> Option<TextEngine> {
        let data = system_font()?;
        let engine = TextEngine::new();
        let id = engine
            .state
            .borrow_mut()
            .registry
            .add_font_data("embedded", &data)?;
        engine.state.borrow_mut().registry.set_default(id);
        Some(engine)
    }

    fn style() -> TextStyle {
        TextStyle::new(18.0, rustui_core::Color::WHITE)
    }

    #[test]
    fn measure_reports_sane_size() {
        let Some(engine) = engine() else { return };
        let size = engine.measure_text("Hello", &style(), None);
        assert!(size.width > 10.0, "width {}", size.width);
        assert!(size.height > 10.0, "height {}", size.height);
        let narrow = engine.measure_text("i", &style(), None);
        assert!(narrow.width < size.width);
    }

    #[test]
    fn line_height_grows_with_size() {
        let Some(engine) = engine() else { return };
        let small = engine.line_height(&TextStyle::new(10.0, rustui_core::Color::WHITE));
        let large = engine.line_height(&TextStyle::new(40.0, rustui_core::Color::WHITE));
        assert!(large > small);
    }

    #[test]
    fn glyphs_are_positioned_in_screen_space() {
        let Some(engine) = engine() else { return };
        let glyphs = engine.glyphs("AB", &style(), None, Point::new(100.0, 50.0));
        assert!(!glyphs.is_empty());
        assert!(glyphs[0].rect.left() >= 100.0);
        assert!(glyphs[0].rect.top() >= 50.0);
        assert!(glyphs[1].rect.left() > glyphs[0].rect.left());
        for glyph in &glyphs {
            assert!(glyph.visible);
            assert!(glyph.uv[0] >= 0.0 && glyph.uv[2] <= 1.0);
        }
    }

    #[test]
    fn atlas_is_populated_and_uploaded_once() {
        let Some(engine) = engine() else { return };
        engine.glyphs("abc", &style(), None, Point::ZERO);
        let info = engine.atlas_info();
        assert!(info.width >= 512 && info.height >= 512);
        assert!(info.dirty);
        let mut buffer = Vec::new();
        engine.copy_atlas_into(&mut buffer);
        assert_eq!(buffer.len(), (info.width * info.height) as usize);
        assert!(buffer.iter().any(|p| *p > 0), "atlas has no glyph coverage");
        engine.mark_atlas_uploaded();
        assert!(!engine.atlas_info().dirty);
    }

    #[test]
    fn repeated_prepare_is_cached() {
        let Some(engine) = engine() else { return };
        engine.prepare("cache me", &style(), None);
        let first = engine.cached_glyphs();
        engine.prepare("cache me", &style(), None);
        assert_eq!(engine.cached_glyphs(), first);
        assert!(engine.stats().0 >= 1);
    }

    #[test]
    fn scale_change_keeps_every_dpi_cached() {
        let Some(engine) = engine() else { return };
        engine.set_scale_factor(1.0);
        engine.prepare("scale", &style(), None);
        let glyphs_1x = engine.cached_glyphs();
        let rasterized_1x = engine.stats().0;
        assert!(glyphs_1x > 0);

        // A second display scale adds its own (larger) rasters instead of dropping the first.
        engine.set_scale_factor(2.0);
        assert_eq!(engine.scale_factor(), 2.0);
        engine.prepare("scale", &style(), None);
        assert!(
            engine.cached_glyphs() > glyphs_1x,
            "the 2x rasters were not kept alongside the 1x ones"
        );
        assert!(engine.stats().0 > rasterized_1x);

        // Back to 1x: everything is still there - nothing rasterizes twice (C-048).
        let settled = (engine.cached_glyphs(), engine.stats().0);
        engine.set_scale_factor(1.0);
        engine.prepare("scale", &style(), None);
        assert_eq!(
            (engine.cached_glyphs(), engine.stats().0),
            settled,
            "switching back to a cached scale re-rasterized glyphs"
        );
    }

    #[test]
    fn hit_test_finds_closest_char() {
        let Some(engine) = engine() else { return };
        let text = "abcd";
        assert_eq!(engine.hit_test_x(text, &style(), -100.0), 0);
        let far = engine.hit_test_x(text, &style(), 10_000.0);
        assert!(far <= text.chars().count());
    }

    #[test]
    fn engine_without_fonts_still_measures() {
        let engine = TextEngine::new();
        let size = engine.measure_text("hello", &style(), None);
        // either the platform has fonts (real metrics) or the placeholder (still positive)
        assert!(size.width > 0.0 && size.height > 0.0);
    }

    #[test]
    fn debug_impl_does_not_panic() {
        let engine = TextEngine::new();
        assert!(!format!("{engine:?}").is_empty());
    }
}
