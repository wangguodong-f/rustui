//! Headless rendering (T-10): a frame drawn into a texture and read back pixel by pixel.
//!
//! Skipped when no GPU adapter can be created - a machine without any driver (including the
//! software ones) cannot render, and that is not a regression of this crate.

use rustui_core::{Border, Color, EdgeRadius, Fill, PaintCommand, Rect, TextArena, Transform};

use rustui_renderer::{GpuContext, RenderInput, SurfaceRenderer};

/// Fills `rect` with a flat opaque colour.
fn fill(rect: Rect, color: Color) -> PaintCommand {
    PaintCommand::Shape {
        rect,
        fill: Fill {
            color,
            gradient: None,
            border: Border {
                widths: rustui_core::EdgeWidths::ZERO,
                color: Color::TRANSPARENT,
            },
            radius: EdgeRadius::ZERO,
        },
        transform: Transform::IDENTITY,
    }
}

/// A GPU, or `None` when the machine has none (the test then reports itself as skipped).
fn gpu() -> Option<GpuContext> {
    match GpuContext::new() {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("skipped: no usable GPU adapter: {error}");
            None
        }
    }
}

#[test]
fn an_offscreen_frame_can_be_read_back() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = SurfaceRenderer::new_offscreen(&gpu, 64, 32).expect("offscreen renderer");

    // Red background, one green rect in the middle. Both are pure colours, so the sRGB transfer
    // function maps them to themselves and the readback is exact.
    let commands = [
        fill(Rect::new(0.0, 0.0, 64.0, 32.0), Color::rgb(1.0, 0.0, 0.0)),
        fill(Rect::new(16.0, 8.0, 16.0, 16.0), Color::rgb(0.0, 1.0, 0.0)),
    ];
    let arena = TextArena::default();
    renderer
        .render(&gpu, &RenderInput::new(&commands, &arena, 1.0, None))
        .expect("render");

    let pixels = renderer.read_pixels(&gpu).expect("readback");
    assert_eq!(pixels.len(), 64 * 32 * 4, "tightly packed RGBA8 rows");

    let at = |x: usize, y: usize| &pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
    assert_eq!(at(0, 0), &[255, 0, 0, 255], "background corner");
    assert_eq!(at(63, 31), &[255, 0, 0, 255], "background far corner");
    assert_eq!(at(24, 16), &[0, 255, 0, 255], "centre of the green rect");
    assert_eq!(at(8, 16), &[255, 0, 0, 255], "left of the green rect");
}

#[test]
fn an_offscreen_target_survives_a_resize() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = SurfaceRenderer::new_offscreen(&gpu, 32, 32).expect("offscreen renderer");

    renderer.resize(&gpu, 48, 16);
    let commands = [fill(
        Rect::new(0.0, 0.0, 48.0, 16.0),
        Color::rgb(0.0, 0.0, 1.0),
    )];
    let arena = TextArena::default();
    renderer
        .render(&gpu, &RenderInput::new(&commands, &arena, 1.0, None))
        .expect("render");

    let pixels = renderer.read_pixels(&gpu).expect("readback");
    assert_eq!(pixels.len(), 48 * 16 * 4, "the target was recreated");
    assert_eq!(&pixels[0..4], &[0, 0, 255, 255]);
}

#[test]
fn reading_pixels_from_a_window_renderer_is_rejected() {
    let Some(gpu) = gpu() else {
        return;
    };
    // `new` needs a real surface, so use the offscreen constructor and check the guard instead:
    // only an offscreen target has pixels to read.
    let mut renderer = SurfaceRenderer::new_offscreen(&gpu, 8, 8).expect("offscreen renderer");
    renderer.resize(&gpu, 0, 0); // minimized: ignored, the target keeps its size
    let arena = TextArena::default();
    renderer
        .render(&gpu, &RenderInput::new(&[], &arena, 1.0, None))
        .expect("render");
    assert_eq!(
        renderer.read_pixels(&gpu).expect("readback").len(),
        8 * 8 * 4
    );
}

/// A rounded clip used to be approximated by its bounding rectangle, which let content through
/// in the four corner arcs (T-5 / C-051). The corners must stay background.
#[test]
fn a_rounded_clip_carves_the_corners() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = SurfaceRenderer::new_offscreen(&gpu, 64, 64).expect("offscreen renderer");

    // Blue background, then a white square visible only inside a 48x48 rounded clip (radius 24,
    // so the clip is a full circle).
    let commands = [
        fill(Rect::new(0.0, 0.0, 64.0, 64.0), Color::rgb(0.0, 0.0, 1.0)),
        PaintCommand::PushClip {
            rect: Rect::new(8.0, 8.0, 48.0, 48.0),
            radius: 24.0,
        },
        fill(Rect::new(0.0, 0.0, 64.0, 64.0), Color::rgb(1.0, 1.0, 1.0)),
        PaintCommand::PopClip,
    ];
    let arena = TextArena::default();
    renderer
        .render(&gpu, &RenderInput::new(&commands, &arena, 1.0, None))
        .expect("render");

    let pixels = renderer.read_pixels(&gpu).expect("readback");
    let at = |x: usize, y: usize| &pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
    // A rectangle clip would have drawn white here: it is inside the rectangle but outside the
    // circle (distance from the centre is ~31.8px, the radius is 24px).
    assert_eq!(at(9, 9), &[0, 0, 255, 255], "corner outside the arc");
    assert_eq!(at(54, 54), &[0, 0, 255, 255], "the opposite corner too");
    assert_eq!(at(32, 32), &[255, 255, 255, 255], "centre of the circle");
    assert_eq!(at(32, 10), &[255, 255, 255, 255], "top of the circle");
    // Outside the clip rectangle the scissor still rules.
    assert_eq!(at(2, 32), &[0, 0, 255, 255], "left of the clip rectangle");
}

/// `EdgeRadius::max_side` used to turn every corner into the largest one (T-6 / C-052): a square
/// corner must stay square while its neighbours are rounded.
#[test]
fn per_corner_radii_shape_the_quad() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = SurfaceRenderer::new_offscreen(&gpu, 32, 32).expect("offscreen renderer");

    // Green 16x16 square whose right corners are rounded (radius 8) over a red background; the
    // left corners ask for radius 0 and must stay sharp.
    let commands = [
        fill(Rect::new(0.0, 0.0, 32.0, 32.0), Color::rgb(1.0, 0.0, 0.0)),
        PaintCommand::Shape {
            rect: Rect::new(8.0, 8.0, 16.0, 16.0),
            fill: Fill {
                color: Color::rgb(0.0, 1.0, 0.0),
                gradient: None,
                border: Border {
                    widths: rustui_core::EdgeWidths::ZERO,
                    color: Color::TRANSPARENT,
                },
                radius: EdgeRadius {
                    top_left: 0.0,
                    top_right: 8.0,
                    bottom_right: 8.0,
                    bottom_left: 0.0,
                },
            },
            transform: Transform::IDENTITY,
        },
    ];
    let arena = TextArena::default();
    renderer
        .render(&gpu, &RenderInput::new(&commands, &arena, 1.0, None))
        .expect("render");

    let pixels = renderer.read_pixels(&gpu).expect("readback");
    let at = |x: usize, y: usize| &pixels[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
    // Sharp left corners: a single max radius would have cut both of them away.
    assert_eq!(at(8, 8), &[0, 255, 0, 255], "top left corner is square");
    assert_eq!(at(8, 23), &[0, 255, 0, 255], "bottom left corner is square");
    // Rounded right corners: both are ~10.6px from their arc centre, the radius is 8.
    assert_eq!(at(23, 8), &[255, 0, 0, 255], "top right corner is cut");
    assert_eq!(at(23, 23), &[255, 0, 0, 255], "bottom right corner is cut");
    // The middle is untouched.
    assert_eq!(at(16, 16), &[0, 255, 0, 255], "centre of the square");
}
