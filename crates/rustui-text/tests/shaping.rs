//! Shaping, kerning and bidirectional layout tests (T-9 / C-053).
//!
//! The engine shapes runs with `rustybuzz` and reorders lines with `unicode-bidi`; these tests
//! pin the behaviours that differ from the old one-glyph-per-character layout: ligature
//! clusters, kerned advances, right-to-left visual order, mixed-direction runs and complex
//! script re-ordering. Fonts are loaded from the system and the tests skip when one is missing.

use rustui_core::{Color, TextMeasurer, TextStyle};
use rustui_text::{caret_rect, TextEngine};

fn read_first(paths: &[&str]) -> Option<Vec<u8>> {
    for path in paths {
        if let Ok(data) = std::fs::read(path) {
            if !data.is_empty() {
                return Some(data);
            }
        }
    }
    None
}

/// DejaVu Sans ships on every mainstream Linux distribution.
fn dejavu() -> Option<Vec<u8>> {
    read_first(&[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    ])
}

/// First system font whose `fc-list` path contains `pattern`.
fn noto(pattern: &str) -> Option<Vec<u8>> {
    let out = std::process::Command::new("fc-list")
        .arg("--format")
        .arg("%{file}\n")
        .output()
        .ok()?;
    let listing = String::from_utf8_lossy(&out.stdout);
    let hit = listing
        .lines()
        .find(|line| line.contains(pattern))
        .map(|line| line.to_string())?;
    std::fs::read(hit).ok()
}

fn engine_with(data: &[u8]) -> Option<TextEngine> {
    TextEngine::with_embedded_font(data, "test").ok()
}

fn style() -> TextStyle {
    TextStyle::new(24.0, Color::WHITE)
}

/// `ffi` collapses into one ligature glyph whose cluster still covers three characters, and the
/// caret keeps moving when it passes through the cluster.
#[test]
fn ligatures_collapse_into_a_multi_character_cluster() {
    let Some(data) = dejavu() else {
        return;
    };
    let engine = engine_with(&data).expect("dejavu loads");
    let ligature = engine.layout_text_full("ffi", &style(), None);
    assert_eq!(
        ligature.glyphs.len(),
        1,
        "ffi must shape into a single ligature glyph"
    );
    assert_eq!(ligature.glyphs[0].char_index, 0);
    assert_eq!(ligature.glyphs[0].char_count, 3);

    let spaced = engine.layout_text_full("f f i", &style(), None);
    assert!(ligature.width() < spaced.width(), "a ligature is narrower");

    // The caret interpolates inside the cluster instead of stalling.
    let c0 = caret_rect(&ligature, 0).x;
    let c1 = caret_rect(&ligature, 1).x;
    let c2 = caret_rect(&ligature, 2).x;
    let c3 = caret_rect(&ligature, 3).x;
    assert!(
        c0 < c1 && c1 < c2 && c2 < c3,
        "caret must advance: {c0} {c1} {c2} {c3}"
    );
    assert!(
        (c3 - ligature.width()).abs() < 0.5,
        "caret after last char at the end"
    );
}

/// Kerning comes out of the GPOS table: `AV` is narrower than `A` + `V`.
#[test]
fn kerning_narrows_pairs() {
    let Some(data) = dejavu() else {
        return;
    };
    let engine = engine_with(&data).expect("dejavu loads");
    let pair = engine.layout_text_full("AV", &style(), None);
    let a = engine.layout_text_full("A", &style(), None);
    let v = engine.layout_text_full("V", &style(), None);
    assert!(
        pair.width() < a.width() + v.width() - 0.1,
        "AV ({}) must be narrower than A+V ({})",
        pair.width(),
        a.width() + v.width()
    );
    // Positions still advance monotonically after kerning.
    assert!(pair.glyphs[1].position.x > 0.0);
    assert!((pair.glyphs[1].position.x + pair.glyphs[1].advance - pair.width()).abs() < 0.01);
}

/// Right-to-left text is laid out right to left: the first logical character sits rightmost,
/// and the shaper reports the run as `rtl`.
#[test]
fn rtl_runs_read_right_to_left() {
    let Some(data) = dejavu() else {
        return;
    };
    let engine = engine_with(&data).expect("dejavu loads");
    // Hebrew aleph-bet-gimel: logical index 0 must render at the right edge.
    let out = engine.layout_text_full("\u{05D0}\u{05D1}\u{05D2}", &style(), None);
    assert_eq!(out.glyphs.len(), 3);
    assert!(out.glyphs.iter().all(|glyph| glyph.rtl));

    let first = out
        .glyphs
        .iter()
        .find(|glyph| glyph.char_index == 0)
        .expect("first logical char is laid out");
    let last = out
        .glyphs
        .iter()
        .max_by_key(|glyph| glyph.char_index)
        .expect("some glyphs");
    assert!(
        first.position.x > last.position.x,
        "first logical char must be rightmost: first at {}, last at {}",
        first.position.x,
        last.position.x
    );
}

/// A mixed-direction line keeps both runs in visual order: the LTR run on the left, the RTL
/// run on the right, and inside the RTL run the logical order reversed.
#[test]
fn mixed_direction_runs_keep_visual_order() {
    let Some(data) = dejavu() else {
        return;
    };
    let engine = engine_with(&data).expect("dejavu loads");
    // "abc " + shin-lamed-mem-sof (Hebrew shalom), base direction LTR.
    let out = engine.layout_text_full("abc \u{05E9}\u{05DC}\u{05DD}", &style(), None);
    assert_eq!(out.glyphs.len(), 7);

    let ltr: Vec<_> = out.glyphs.iter().filter(|glyph| !glyph.rtl).collect();
    let rtl: Vec<_> = out.glyphs.iter().filter(|glyph| glyph.rtl).collect();
    assert_eq!(ltr.len(), 4, "latin run plus space");
    assert_eq!(rtl.len(), 3, "hebrew run");

    let ltr_max = ltr
        .iter()
        .map(|g| g.position.x + g.advance)
        .fold(0.0f32, f32::max);
    let rtl_min = rtl.iter().map(|g| g.position.x).fold(f32::MAX, f32::min);
    assert!(
        ltr_max <= rtl_min + 0.01,
        "latin run must sit left of the hebrew run"
    );

    // Inside the run the first logical character (shin, index 4) is rightmost.
    let first = rtl
        .iter()
        .find(|g| g.char_index == 4)
        .expect("shin laid out");
    let last = rtl
        .iter()
        .max_by_key(|g| g.char_index)
        .expect("some rtl glyphs");
    assert!(first.position.x > last.position.x);
}

/// The shaped glyph id is the same id `ab_glyph` rasterizes - both read the same `cmap`, so
/// the atlas keeps serving the right pixels.
#[test]
fn shaped_glyph_ids_match_the_rasterizer() {
    use ab_glyph::{Font as _, FontVec};

    let Some(data) = dejavu() else {
        return;
    };
    let expected = FontVec::try_from_vec(data.clone())
        .expect("dejavu parses")
        .glyph_id('A')
        .0 as u32;
    let engine = engine_with(&data).expect("dejavu loads");
    let out = engine.layout_text_full("A", &style(), None);
    let key = out.glyphs.first().expect("A lays out").key;
    assert_eq!(
        key.glyph, expected,
        "shaper and rasterizer must agree on glyph ids"
    );
}

/// Wrapping only ever breaks between clusters: an `ffi` ligature at a break point stays whole,
/// and the glyph count is identical with and without wrapping.
#[test]
fn wrapping_never_splits_a_cluster() {
    let Some(data) = dejavu() else {
        return;
    };
    let engine = engine_with(&data).expect("dejavu loads");
    let text = "ffi ffi ffi ffi ffi ffi";
    let unwrapped = engine.layout_text_full(text, &style(), None);
    let wrapped = engine.layout_text_full(text, &style(), Some(60.0));
    assert!(wrapped.layout.lines.len() > 1, "text must wrap");
    assert_eq!(
        wrapped.glyphs.len(),
        unwrapped.glyphs.len(),
        "wrapping must not change how many glyphs shape"
    );
    for line in &wrapped.layout.lines {
        assert!(
            line.width <= 60.0,
            "line wider than the limit: {}",
            line.width
        );
    }
    // Every glyph's cluster is a real cluster of the text (character ranges line up).
    let total = text.chars().count();
    for glyph in &wrapped.glyphs {
        assert!(glyph.char_index + glyph.char_count <= total);
    }
}

/// Hit testing maps a pixel position to a character index, splitting ligature clusters in
/// half instead of reporting glyph ordinals.
#[test]
fn hit_test_splits_clusters_by_position() {
    let Some(data) = dejavu() else {
        return;
    };
    let engine = engine_with(&data).expect("dejavu loads");
    let out = engine.layout_text_full("ffi", &style(), None);
    let width = out.width();
    assert_eq!(engine.hit_test_x("ffi", &style(), -100.0), 0);
    assert_eq!(engine.hit_test_x("ffi", &style(), width * 0.25), 0);
    assert_eq!(engine.hit_test_x("ffi", &style(), width * 0.9), 2);
    assert_eq!(engine.hit_test_x("ffi", &style(), 10_000.0), 3);

    // Multi-character text still reports plain character indices.
    assert_eq!(engine.hit_test_x("abcd", &style(), -100.0), 0);
    assert!(engine.hit_test_x("abcd", &style(), 10_000.0) <= 4);
}

/// Devanagari re-orders and merges: the 6-character sequence `नमस्ते` shapes into fewer
/// glyphs than characters, with at least one zero-advance mark. Skipped when the font is
/// missing.
#[test]
fn devanagari_reorders_complex_sequences() {
    let Some(data) = noto("Devanagari") else {
        return;
    };
    let engine = engine_with(&data).expect("devanagari font loads");
    let text = "\u{928}\u{92E}\u{938}\u{94D}\u{924}\u{947}"; // नमस्ते
    let out = engine.layout_text_full(text, &style(), None);
    assert!(
        out.glyphs.len() < text.chars().count(),
        "conjuncts must merge: {} glyphs for {} chars",
        out.glyphs.len(),
        text.chars().count()
    );
    assert!(
        out.glyphs.iter().any(|glyph| glyph.advance <= 0.5),
        "a matra must carry no advance"
    );
    // Logical order of the base characters is preserved in visual (LTR) glyph order.
    let indices: Vec<usize> = out.glyphs.iter().map(|glyph| glyph.char_index).collect();
    assert!(
        indices.windows(2).all(|pair| pair[0] <= pair[1]),
        "character indices must not go backwards: {indices:?}"
    );
}

/// Arabic joins: `سلام` shapes with contextual forms and the lam-alef ligature, so it needs
/// fewer glyphs than characters. Skipped when the font is missing.
#[test]
fn arabic_joins_contextually() {
    let Some(data) = noto("NotoSansArabic") else {
        return;
    };
    let engine = engine_with(&data).expect("arabic font loads");
    let text = "\u{633}\u{644}\u{627}\u{645}"; // سلام
    let out = engine.layout_text_full(text, &style(), None);
    assert!(
        out.glyphs.len() < text.chars().count(),
        "lam-alef must ligate: {} glyphs for {} chars",
        out.glyphs.len(),
        text.chars().count()
    );
    assert!(out.glyphs.iter().all(|glyph| glyph.rtl));
    let first = out
        .glyphs
        .iter()
        .find(|glyph| glyph.char_index == 0)
        .expect("first logical char laid out");
    let last = out
        .glyphs
        .iter()
        .max_by_key(|glyph| glyph.char_index)
        .expect("some glyphs");
    assert!(
        first.position.x > last.position.x,
        "arabic reads right to left"
    );
}

/// A font registered as a bare `FontVec` (no source bytes kept) falls back to one glyph per
/// character - the pre-shaping path still lays text out.
#[test]
fn fontvec_without_bytes_falls_back_per_character() {
    use ab_glyph::FontVec;

    let Some(data) = dejavu() else {
        return;
    };
    let mut registry = rustui_text::FontRegistry::new();
    let font = FontVec::try_from_vec(data).expect("dejavu parses");
    let id = registry.add_font("test", font);
    registry.set_default(id);
    assert!(!registry.get(id).expect("font stored").can_shape());

    let mut engine = rustui_text::TextLayoutEngine::default();
    let out = engine.layout(&registry, "plain text", &style(), None);
    assert_eq!(out.glyphs.len(), "plain text".chars().count());
    assert!(out.width() > 0.0);
}
