//! Multi window DPI (T-2 / C-048): one shared text engine, two windows, two display scales.
//!
//! The engine used to drop its whole glyph cache on every scale change, so alternating between a
//! 1x and a 2x window re-rasterized every glyph each frame. The cache is now keyed by *physical*
//! pixel size, so both scales live side by side and switching is free. `render_input` hands the
//! engine the scale of the window being drawn, which is also what this test drives: it rasterizes
//! every text run exactly like the renderer's `tessellate` does.

use std::time::Duration;

use rustui::core::PaintCommand;
use rustui::prelude::*;
use rustui::text::TextEngine;
use rustui::window::WindowOptions;

fn frame() -> FrameClock {
    FrameClock {
        frame: 1,
        delta: Duration::ZERO,
        elapsed: Duration::ZERO,
    }
}

/// One window whose root is a single label, at the given display scale.
fn label_window(wm: &mut WindowManager, scale: f64) -> WindowId {
    let window = wm.create_window(
        WindowOptions::new()
            .title("dpi")
            .size(320.0, 240.0)
            .scale_factor(scale),
    );
    let label = wm.insert(Box::new(Label::with_text(SharedProperty::new(
        String::from("Hello, DPI!"),
    ))));
    let mut root = VBox::new().with_fill();
    root.push(label);
    let root = wm.insert(Box::new(root));
    wm.set_root_id(window, root);
    window
}

/// Rasterizes every text run of `window` the way the renderer's `tessellate` does.
///
/// `render_input` is what switches the shared engine to this window's scale, so the raster
/// statistics afterwards say which display scale paid for which glyph.
fn rasterize(wm: &WindowManager, window: WindowId) {
    let input = wm.render_input(window).expect("painted frame");
    let glyphs = input.glyphs.expect("TextEngine exposes its glyph source");
    let painter = wm.window(window).expect("window").painter();
    for command in painter.commands() {
        let PaintCommand::Text {
            position,
            run,
            range,
            style,
            max_width,
            ..
        } = command
        else {
            continue;
        };
        let interned = painter.arena().get(*run).expect("interned run");
        glyphs.glyphs(
            &interned[range.start as usize..range.end as usize],
            style,
            *max_width,
            *position,
        );
    }
}

#[test]
fn two_windows_with_different_scales_share_one_cache_without_thrashing() {
    let engine = TextEngine::new();
    let mut wm = WindowManager::new(Box::new(engine.clone()));
    let low = label_window(&mut wm, 1.0);
    let high = label_window(&mut wm, 2.0);

    // First pass at 1x: the glyphs are rasterized at their physical size.
    wm.render(low, frame());
    wm.render(high, frame());

    // Every window renders with its own display scale (C-048).
    assert_eq!(wm.render_input(low).expect("painted").scale, 1.0);
    assert_eq!(wm.render_input(high).expect("painted").scale, 2.0);

    rasterize(&wm, low);
    let after_low = (engine.cached_glyphs(), engine.stats());
    if after_low.0 == 0 {
        // No system font to rasterize with - nothing to assert.
        return;
    }

    // The 2x window rasterizes *its own* rasters instead of reusing (or discarding) the 1x ones.
    wm.render(high, frame());
    rasterize(&wm, high);
    let after_high = (engine.cached_glyphs(), engine.stats());
    assert!(
        engine.cached_glyphs() > after_low.0,
        "the 2x window did not add its own rasters to the shared cache"
    );
    assert!(
        after_high.1 .0 > after_low.1 .0,
        "the 2x window rasterized nothing new ({after_low:?} -> {after_high:?})"
    );

    // Alternating back and forth must be free: the old code wiped the whole cache on every scale
    // change, so a 1x/2x pair re-rasterized everything on each frame. (Hits may - and should -
    // grow: that is the cache being used.)
    wm.render(low, frame());
    rasterize(&wm, low);
    wm.render(high, frame());
    rasterize(&wm, high);
    assert_eq!(
        (engine.cached_glyphs(), engine.stats().0),
        (after_high.0, after_high.1 .0),
        "switching between DPI windows re-rasterized cached glyphs"
    );
}
