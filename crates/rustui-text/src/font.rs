//! Font discovery and font resources (design document §25).
//!
//! The engine keeps a [`fontdb::Database`] for discovery and one [`LoadedFont`] per family/weight
//! that has actually been used. Glyph rasterization uses `ab_glyph`, which is a small, pure-Rust
//! rasterizer; HarfBuzz style shaping (`rustybuzz`) handles ligatures, kerning, complex scripts
//! and bidi runs (T-9 / C-053), and both read glyph ids from the same `cmap`. The raw bytes of a
//! face are kept alongside the rasterizer's copy because the shaper borrows them while shaping.

use std::collections::HashMap;
use std::sync::Arc;

use ab_glyph::{Font as _, FontVec, GlyphId, ScaleFont};
use rustui_core::{FontId, Result, UiError};

/// Source bytes of one face, kept for `rustybuzz` (which borrows them while shaping).
#[derive(Clone)]
struct ShapeSource {
    bytes: Arc<Vec<u8>>,
    /// Face index inside a font collection (0 for plain files).
    index: u32,
}

/// A font that has been loaded into memory.
pub struct LoadedFont {
    pub id: FontId,
    pub family: String,
    pub data: FontVec,
    /// Design units per em.
    pub units_per_em: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    /// Bytes the shaper needs; `None` only for fonts registered as a bare `FontVec`.
    shape_source: Option<ShapeSource>,
}

impl std::fmt::Debug for LoadedFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedFont")
            .field("id", &self.id)
            .field("family", &self.family)
            .field("units_per_em", &self.units_per_em)
            .finish()
    }
}

impl LoadedFont {
    /// Returns the font scaled to `px` logical pixels.
    ///
    /// The returned value borrows the font data, so it must be dropped before the registry is
    /// mutated (a `PxScaleFont<&FontVec>` is a lightweight view, not an owned object).
    pub fn scaled(&self, px: f32) -> ab_glyph::PxScaleFont<&ab_glyph::FontVec> {
        self.data.as_scaled(px)
    }

    /// Maps a unicode scalar to a glyph, falling back to `.notdef` (glyph 0).
    pub fn glyph_id(&self, ch: char) -> GlyphId {
        self.data.glyph_id(ch)
    }

    /// Horizontal advance of `ch` at `px`.
    pub fn advance(&self, ch: char, px: f32) -> f32 {
        let scaled = self.scaled(px);
        scaled.h_advance(self.glyph_id(ch))
    }

    /// Builds a shaping face over the stored bytes; `None` when no source bytes were kept
    /// (fonts registered as a bare [`FontVec`], see [`FontRegistry::add_font`]).
    ///
    /// The face borrows the registry's bytes, so it must not outlive it - layout builds one per
    /// call (T-9 / C-053).
    pub fn shaper_face(&self) -> Option<rustybuzz::Face<'_>> {
        let source = self.shape_source.as_ref()?;
        rustybuzz::Face::from_slice(&source.bytes, source.index)
    }

    /// `true` when HarfBuzz style shaping is available for this font.
    pub fn can_shape(&self) -> bool {
        self.shape_source.is_some()
    }
}

/// Registry of the fonts available to the text engine.
#[derive(Debug, Default)]
pub struct FontRegistry {
    db: fontdb::Database,
    /// family + weight -> font id
    resolved: HashMap<(String, u16), FontId>,
    fonts: HashMap<FontId, LoadedFont>,
    next_id: u32,
    /// Font that [`FontId::DEFAULT`] maps to.
    defaults: Option<FontId>,
    /// Candidates used when a family is not installed.
    default_families: Vec<String>,
    mono_families: Vec<String>,
}

impl FontRegistry {
    /// Creates an empty registry with no system fonts loaded.
    pub fn new() -> Self {
        let mut registry = Self {
            db: fontdb::Database::new(),
            next_id: 1,
            ..Default::default()
        };
        registry.default_families = default_sans_families();
        registry.mono_families = default_mono_families();
        registry
    }

    /// Creates a registry pre-populated with the system fonts.
    pub fn with_system_fonts() -> Result<Self> {
        let mut registry = Self::new();
        registry.load_system_fonts()?;
        Ok(registry)
    }

    /// Loads every system font known to `fontdb` for this platform.
    pub fn load_system_fonts(&mut self) -> Result<()> {
        if self.db.is_empty() {
            self.db.load_system_fonts();
        }
        Ok(())
    }

    /// Adds a font from raw bytes.
    pub fn add_memory_font(&mut self, data: &[u8], index: u32) -> Result<FontId> {
        // fontdb 0.24 exposes a single-face `load_font_data`; collections must be added face
        // by face, which is enough for the embedded-font use case.
        let _ = index;
        self.db.load_font_data(data.to_vec());
        self.resolved.clear();
        Ok(FontId::DEFAULT)
    }

    /// Number of families the platform reports.
    pub fn family_count(&self) -> usize {
        self.db.len()
    }

    /// `true` when at least one font is available.
    pub fn has_fonts(&self) -> bool {
        !self.db.is_empty()
    }

    /// Returns the font for a family, loading it on first use.
    pub fn font_for_family(&mut self, family: &str) -> Option<FontId> {
        let weight = 400u16;
        self.resolve(family, weight)
    }

    /// Returns the font for a family and weight (400 = regular, 700 = bold).
    pub fn resolve(&mut self, family: &str, weight: u16) -> Option<FontId> {
        let key = (family.to_string(), weight);
        if let Some(id) = self.resolved.get(&key) {
            return Some(*id);
        }
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight(weight),
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let face_id = match self.db.query(&query) {
            Some(id) => id,
            None => {
                // Fall back to any sans serif / monospace / any face.
                let fallback = [
                    fontdb::Query {
                        families: &[fontdb::Family::SansSerif],
                        ..query
                    },
                    fontdb::Query {
                        families: &[fontdb::Family::Monospace],
                        ..query
                    },
                    fontdb::Query {
                        families: &[],
                        ..query
                    },
                ];
                fallback.iter().find_map(|q| self.db.query(q))?
            }
        };
        let loaded = self.load_face(face_id)?;
        let id = loaded.id;
        self.resolved.insert(key, id);
        Some(id)
    }

    /// Loads the first face of the registry; must be called before rendering text.
    pub fn load_default(&mut self) -> Option<FontId> {
        if let Some(id) = self.defaults {
            return Some(id);
        }
        for family in self.default_families.clone() {
            if let Some(id) = self.resolve(&family, 400) {
                self.defaults = Some(id);
                return Some(id);
            }
        }
        // Last resort: the first monospace face, then literally any face.
        if let Some(id) = self.resolve("monospace", 400) {
            self.defaults = Some(id);
            return Some(id);
        }
        let query = fontdb::Query {
            families: &[],
            weight: fontdb::Weight(400),
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let face_id = self.db.query(&query)?;
        let id = self.load_face(face_id)?.id;
        self.defaults = Some(id);
        Some(id)
    }

    /// Loads a monospaced font.
    pub fn load_monospace(&mut self) -> Option<FontId> {
        for family in self.mono_families.clone() {
            if let Some(id) = self.resolve(&family, 400) {
                return Some(id);
            }
        }
        self.resolve("monospace", 400)
    }

    fn load_face(&mut self, face_id: fontdb::ID) -> Option<&LoadedFont> {
        let face = self.db.face(face_id)?;
        let (data, index) = self
            .db
            .with_face_data(face_id, |data, index| (data.to_vec(), index))?;
        let shape_source = Some(ShapeSource {
            bytes: Arc::new(data.clone()),
            index,
        });
        let font_vec = FontVec::try_from_vec(data)
            .map_err(|e| UiError::FontDatabase(e.to_string()))
            .ok()?;
        let units_per_em = font_vec.units_per_em().unwrap_or(1000.0);
        let scaled = font_vec.as_scaled(units_per_em);
        let ascent = scaled.ascent();
        let descent = scaled.descent();
        let line_gap = scaled.line_gap();
        let id = FontId(self.next_id);
        self.next_id += 1;
        let family = face
            .families
            .first()
            .map(|(name, _)| name.clone())
            .unwrap_or_default();
        let loaded = LoadedFont {
            id,
            family,
            data: font_vec,
            units_per_em,
            ascent,
            descent,
            line_gap,
            shape_source,
        };
        self.fonts.insert(id, loaded);
        self.fonts.get(&id)
    }

    /// Loads and stores a font, returning its id.
    ///
    /// The font is rasterizable but **not shapable** (no source bytes are kept): prefer
    /// [`FontRegistry::add_font_data`] whenever the original bytes are available (T-9).
    pub fn add_font(&mut self, family: &str, data: FontVec) -> FontId {
        let units_per_em = data.units_per_em().unwrap_or(1000.0);
        let scaled = data.as_scaled(units_per_em);
        let id = FontId(self.next_id);
        self.next_id += 1;
        self.fonts.insert(
            id,
            LoadedFont {
                id,
                family: family.to_string(),
                ascent: scaled.ascent(),
                descent: scaled.descent(),
                line_gap: scaled.line_gap(),
                units_per_em,
                data,
                shape_source: None,
            },
        );
        id
    }

    /// Loads a font from raw bytes, keeping those bytes for the shaper (T-9 / C-053).
    ///
    /// Returns `None` when `data` is not a single valid font (e.g. an unindexed collection).
    pub fn add_font_data(&mut self, family: &str, data: &[u8]) -> Option<FontId> {
        let shape_source = Some(ShapeSource {
            bytes: Arc::new(data.to_vec()),
            index: 0,
        });
        let font_vec = FontVec::try_from_vec(data.to_vec()).ok()?;
        let id = self.add_font(family, font_vec);
        if let Some(loaded) = self.fonts.get_mut(&id) {
            loaded.shape_source = shape_source;
        }
        Some(id)
    }

    /// Borrows a font.
    pub fn get(&self, id: FontId) -> Option<&LoadedFont> {
        if id == FontId::DEFAULT {
            if let Some(default) = self.default_font() {
                return Some(default);
            }
        }
        self.fonts.get(&id)
    }

    /// Designates `id` as the font used by [`FontId::DEFAULT`].
    ///
    /// Used when an application registers its own font (embedded fonts, tests).
    pub fn set_default(&mut self, id: FontId) {
        if self.fonts.contains_key(&id) {
            self.defaults = Some(id);
        }
    }

    /// Font that `FontId::DEFAULT` resolves to.
    pub fn default_font(&self) -> Option<&LoadedFont> {
        self.defaults.and_then(|id| self.fonts.get(&id))
    }

    /// Mutably borrows a font.
    pub fn get_mut(&mut self, id: FontId) -> Option<&mut LoadedFont> {
        self.fonts.get_mut(&id)
    }

    /// Ids of every loaded font.
    pub fn ids(&self) -> Vec<FontId> {
        self.fonts.keys().copied().collect()
    }
}

fn default_sans_families() -> Vec<String> {
    if cfg!(target_os = "macos") {
        vec!["Helvetica".into(), "Menlo".into(), "Arial".into()]
    } else if cfg!(target_os = "windows") {
        vec!["Segoe UI".into(), "Arial".into(), "Tahoma".into()]
    } else {
        vec![
            "DejaVu Sans".into(),
            "Noto Sans".into(),
            "Liberation Sans".into(),
            "Ubuntu".into(),
        ]
    }
}

fn default_mono_families() -> Vec<String> {
    if cfg!(target_os = "macos") {
        vec!["Menlo".into(), "Courier New".into(), "Monaco".into()]
    } else if cfg!(target_os = "windows") {
        vec![
            "Consolas".into(),
            "Cascadia Mono".into(),
            "Courier New".into(),
        ]
    } else {
        vec![
            "DejaVu Sans Mono".into(),
            "Liberation Mono".into(),
            "Noto Sans Mono".into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_starts_empty() {
        let mut registry = FontRegistry::new();
        assert!(!registry.has_fonts());
        assert!(registry.load_default().is_none());
    }

    #[test]
    fn add_font_and_query() {
        let mut registry = FontRegistry::new();
        // Build a font from any system file if available; otherwise skip the assertion.
        let id = match system_font_bytes() {
            Some(bytes) => {
                registry.db.load_font_data(bytes);
                registry.load_default()
            }
            None => None,
        };
        if let Some(id) = id {
            assert!(registry.get(id).is_some());
            assert!(registry.advance_for(id, 'A', 16.0) > 0.0);
        }
    }

    impl FontRegistry {
        fn advance_for(&self, id: FontId, ch: char, px: f32) -> f32 {
            self.get(id).map(|f| f.advance(ch, px)).unwrap_or(0.0)
        }
    }

    fn system_font_bytes() -> Option<Vec<u8>> {
        let candidates: Vec<&str> = if cfg!(target_os = "linux") {
            vec![
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            ]
        } else if cfg!(target_os = "macos") {
            vec![
                "/System/Library/Fonts/Supplemental/Arial.ttf",
                "/System/Library/Fonts/Helvetica.ttc",
            ]
        } else {
            vec![
                "C:\\Windows\\Fonts\\segoeui.ttf",
                "C:\\Windows\\Fonts\\arial.ttf",
            ]
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
}
