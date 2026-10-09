//! Golden image regressions (T-10): real scenes rendered through the GPU pipeline into an
//! offscreen texture, then compared against committed goldens.
//!
//! [`pipeline.rs`] proves the command list is well formed; this file proves the pixels a window
//! would show. Every scene is built with the real widgets, laid out, painted, drawn by `wgpu` into
//! a texture and read back, then reduced to a coarse colour grid:
//!
//! ```text
//! # RustUI golden grid: controls 320x240, cell 8x8 -> 40x30 cells, 4 bits per channel
//! 111111111f0000...        <- one line per grid row, three hex digits per cell
//! ```
//!
//! The grid - not the raw pixels - is compared, with a per cell tolerance and a cap on how many
//! cells may drift: another GPU driver antialiases text slightly differently, and that must not
//! fail the test, while a missing widget, a wrong colour or a popup painted under its siblings
//! must.
//!
//! Regenerate the goldens after an intentional visual change:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p rustui --test golden
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use rustui::core::{PaintCommand, Point, Rect};
use rustui::prelude::*;
use rustui::renderer::{GpuContext, SurfaceRenderer};
use rustui::text::TextEngine;
use rustui::window::WindowOptions;

const WIDTH: f32 = 320.0;
const HEIGHT: f32 = 240.0;
/// Edge of one golden cell in pixels; each cell becomes a single averaged colour.
const CELL: usize = 8;
/// Per channel difference (on the 0-15 golden scale) below which two cells count as equal.
const CHANNEL_DELTA: i32 = 1;
/// Share of cells that may exceed [`CHANNEL_DELTA`] before the frame is considered changed.
///
/// Small enough that a recoloured button (about 15 cells of 1200) fails, loose enough that a
/// different GPU driver's antialiasing passes; scene builders additionally assert the *presence*
/// of the text they paint, which a grid cannot see.
const MAX_DRIFT: f64 = 0.01;

fn frame() -> FrameClock {
    FrameClock {
        frame: 1,
        delta: Duration::ZERO,
        elapsed: Duration::ZERO,
    }
}

/// Window space rect of `id` after the last layout pass.
fn bounds(wm: &WindowManager, id: ObjectId) -> Rect {
    wm.store()
        .get(id)
        .expect("widget still alive")
        .base()
        .window_bounds()
}

// ---------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------

/// A label above a text field and a button - the `hello` example in miniature.
fn controls_scene(wm: &mut WindowManager, window: WindowId) {
    let label = wm.insert(Box::new(Label::with_text(SharedProperty::new(
        String::from("Hello, RustUI!"),
    ))));
    let input = wm.insert(Box::new(
        TextInput::with_text(SharedProperty::new(String::from("Ada"))).placeholder("Your name"),
    ));
    let button = wm.insert(Box::new(Button::new("Greet")));

    let mut row = HBox::new().with_spacing(8.0);
    row.push(input);
    row.push(button);
    let row = wm.insert(Box::new(row));

    let mut root = VBox::new()
        .with_spacing(12.0)
        .with_padding(Insets::all(16.0))
        .with_fill();
    root.push(label);
    root.push(row);
    let root = wm.insert(Box::new(root));
    wm.set_root_id(window, root);
    wm.render(window, frame());

    // The grid cannot tell "label missing" from "label rendered very light": assert the strings
    // reached the paint list, then let the golden pin down where and how they were drawn.
    for expected in ["Hello, RustUI!", "Greet", "Ada"] {
        assert!(
            painted_texts(wm, window)
                .iter()
                .any(|text| text == expected),
            "{expected:?} was not painted"
        );
    }
}

/// Every text run in the current frame, decoded from the painter's string arena.
fn painted_texts(wm: &WindowManager, window: WindowId) -> Vec<String> {
    let Some(window) = wm.window(window) else {
        return Vec::new();
    };
    let arena = window.painter().arena();
    window
        .painter()
        .commands()
        .iter()
        .filter_map(|command| {
            let PaintCommand::Text { run, range, .. } = command else {
                return None;
            };
            arena
                .get(*run)
                .map(|text| text[range.start as usize..range.end as usize].to_string())
        })
        .collect()
}

/// A combo box whose popup drops onto the panel painted *after* it - the popup must come out
/// above that later sibling and outside its own control bounds (C-045).
fn popup_scene(wm: &mut WindowManager, window: WindowId) {
    let combo = wm.insert(Box::new(ComboBox::new(["Alpha", "Beta", "Gamma"])));
    let panel = wm.insert(Box::new(
        Container::new()
            .with_background(Color::rgb(0.85, 0.35, 0.20))
            .with_border(Color::rgb(0.2, 0.2, 0.2), 1.0)
            .with_fill(),
    ));

    let mut root = VBox::new()
        .with_spacing(8.0)
        .with_padding(Insets::all(12.0))
        .with_fill();
    root.push(combo);
    root.push(panel);
    let root = wm.insert(Box::new(root));
    wm.set_root_id(window, root);
    wm.render(window, frame());

    // Open the popup with a real click: hit testing has to find the control, and the press must
    // not be swallowed by the panel below it.
    let combo_bounds = bounds(wm, combo);
    let centre = Point::new(
        combo_bounds.x + combo_bounds.width / 2.0,
        combo_bounds.y + combo_bounds.height / 2.0,
    );
    wm.handle_event(
        window,
        UiEvent::MouseDown {
            button: MouseButton::Left,
            position: centre,
        },
    );
    wm.render(window, frame());

    // The second item only exists inside the open popup, so this fails if the click did nothing.
    assert!(
        painted_texts(wm, window).iter().any(|text| text == "Beta"),
        "the combo box popup did not open"
    );
}

/// A label rotated 30 degrees next to an upright one: `PaintCommand::Text` carries a transform
/// and the renderer turns the glyphs around their ink box (T-8 / C-049).
fn rotated_scene(wm: &mut WindowManager, window: WindowId) {
    let tilted = wm.insert(Box::new(
        Label::new("Tilted").rotation(30.0).role(TextRole::Title),
    ));
    let flat = wm.insert(Box::new(Label::new("Upright")));

    let mut root = VBox::new()
        .with_spacing(24.0)
        .with_padding(Insets::all(24.0))
        .with_fill();
    root.push(tilted);
    root.push(flat);
    let root = wm.insert(Box::new(root));
    wm.set_root_id(window, root);
    wm.render(window, frame());

    for expected in ["Tilted", "Upright"] {
        assert!(
            painted_texts(wm, window)
                .iter()
                .any(|text| text == expected),
            "{expected:?} was not painted"
        );
    }
    // The grid pins *where* the pixels land; this pins the intent behind them.
    let has_rotation = wm
        .window(window)
        .expect("window")
        .painter()
        .commands()
        .iter()
        .any(|command| matches!(command, PaintCommand::Text { transform, .. } if !transform.is_identity()));
    assert!(has_rotation, "no rotated text command was recorded");
}

// ---------------------------------------------------------------------------
// Offscreen rendering
// ---------------------------------------------------------------------------

/// Runs `build` (which may also interact) and returns the RGBA8 pixels of the final frame.
///
/// Returns `None` when the machine has no GPU at all; the test then reports itself as skipped
/// instead of failing on hardware it was never given.
fn render(build: impl FnOnce(&mut WindowManager, WindowId)) -> Option<Vec<u8>> {
    let mut wm = WindowManager::new(Box::new(TextEngine::new()));
    let window = wm.create_window(
        WindowOptions::new()
            .title("golden")
            .size(WIDTH, HEIGHT)
            .scale_factor(1.0),
    );
    build(&mut wm, window);

    let gpu = match GpuContext::new() {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("skipped: no usable GPU adapter: {error}");
            return None;
        }
    };
    let mut renderer =
        SurfaceRenderer::new_offscreen(&gpu, WIDTH as u32, HEIGHT as u32).expect("renderer");

    let input = wm
        .render_input(window)
        .expect("the scene painted at least one command");
    renderer.render(&gpu, &input).expect("render");
    Some(renderer.read_pixels(&gpu).expect("readback"))
}

// ---------------------------------------------------------------------------
// Golden grids
// ---------------------------------------------------------------------------

/// Averages every `CELL x CELL` block of an RGBA8 frame to one colour of 4 bits per channel.
fn grid(pixels: &[u8], width: usize, height: usize) -> Vec<[u8; 3]> {
    assert_eq!(
        pixels.len(),
        width * height * 4,
        "tightly packed RGBA8 rows"
    );
    let cols = width / CELL;
    let rows = height / CELL;
    let mut cells = Vec::with_capacity(cols * rows);
    for row in 0..rows {
        for col in 0..cols {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for y in row * CELL..(row + 1) * CELL {
                for x in col * CELL..(col + 1) * CELL {
                    let i = (y * width + x) * 4;
                    r += pixels[i] as u32;
                    g += pixels[i + 1] as u32;
                    b += pixels[i + 2] as u32;
                }
            }
            let n = (CELL * CELL) as u32;
            // Average, then quantise to 0..15 with rounding.
            let quantise = |sum: u32| (((sum + n / 2) / n) * 15 + 127) / 255;
            cells.push([quantise(r) as u8, quantise(g) as u8, quantise(b) as u8]);
        }
    }
    cells
}

/// Serialises a grid: one header line (documentation, ignored when comparing) plus one line per
/// row of three hex digits per cell.
fn encode(name: &str, cols: usize, rows: usize, cells: &[[u8; 3]]) -> String {
    let mut out = format!(
        "# RustUI golden grid: {name} {WIDTH}x{HEIGHT}, cell {CELL}x{CELL} -> {cols}x{rows} cells, 4 bits per channel\n"
    );
    for row in 0..rows {
        let line: String = cells[row * cols..(row + 1) * cols]
            .iter()
            .map(|[r, g, b]| format!("{r:01x}{g:01x}{b:01x}"))
            .collect();
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Parses the cell rows of a golden file (the `#` header is ignored).
fn decode(raw: &str) -> Vec<String> {
    raw.lines()
        .filter(|line| !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Compares `actual` against the golden file of `name`, writing it when `UPDATE_GOLDEN` is set.
fn check(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.grid"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        write_golden(&path, actual);
        return;
    }
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "no golden for `{name}` at {} ({error}); create it with \
             `UPDATE_GOLDEN=1 cargo test -p rustui --test golden`",
            path.display()
        )
    });

    let expected = decode(&raw);
    let current = decode(actual);
    if expected.len() != current.len() {
        panic!(
            "the golden grid of `{name}` has {} rows, this frame has {} - the window size or \
             cell size changed; regenerate with UPDATE_GOLDEN=1",
            expected.len(),
            current.len()
        );
    }

    let cols = current.first().map_or(0, |line| line.len()) / 3;
    let mut drift: Vec<String> = Vec::new();
    for (row, (want, got)) in expected.iter().zip(&current).enumerate() {
        if want.len() != got.len() {
            drift.push(format!("row {row}: width {} vs {}", want.len(), got.len()));
            continue;
        }
        for col in 0..cols {
            let parse = |line: &str, col: usize| -> [i32; 3] {
                let cell = &line[col * 3..col * 3 + 3];
                [
                    i32::from_str_radix(&cell[0..1], 16).unwrap_or(0),
                    i32::from_str_radix(&cell[1..2], 16).unwrap_or(0),
                    i32::from_str_radix(&cell[2..3], 16).unwrap_or(0),
                ]
            };
            let (e, a) = (parse(want, col), parse(got, col));
            if (0..3).any(|c| (a[c] - e[c]).abs() > CHANNEL_DELTA) {
                drift.push(format!(
                    "cell ({col},{row}): want {} got {}",
                    &want[col * 3..col * 3 + 3],
                    &got[col * 3..col * 3 + 3]
                ));
            }
        }
    }

    let total = current.len() * cols;
    let allowed = ((total as f64) * MAX_DRIFT).ceil() as usize;
    if drift.len() > allowed {
        let shown: Vec<&str> = drift.iter().take(12).map(String::as_str).collect();
        panic!(
            "`{name}` looks different: {} of {total} cells drifted (allowed {allowed}).\n\
             First differences:\n  {}\n\
             If this change is intended, run `UPDATE_GOLDEN=1 cargo test -p rustui --test golden`.",
            drift.len(),
            shown.join("\n  ")
        );
    }
}

fn write_golden(path: &Path, contents: &str) {
    let Some(parent) = path.parent() else {
        return;
    };
    std::fs::create_dir_all(parent).expect("golden directory");
    std::fs::write(path, contents).expect("write golden");
    println!("updated golden {}", path.display());
}

fn assert_matches(name: &str, pixels: Option<Vec<u8>>) {
    let Some(pixels) = pixels else {
        return; // no GPU on this machine: reported as skipped by `render`
    };
    let cols = WIDTH as usize / CELL;
    let rows = HEIGHT as usize / CELL;
    let cells = grid(&pixels, WIDTH as usize, HEIGHT as usize);
    assert_eq!(cells.len(), cols * rows);
    check(name, &encode(name, cols, rows, &cells));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn the_controls_scene_matches_its_golden() {
    assert_matches("controls", render(controls_scene));
}

#[test]
fn the_open_combo_box_popup_matches_its_golden() {
    assert_matches("combo_popup", render(popup_scene));
}

#[test]
fn the_rotated_label_matches_its_golden() {
    assert_matches("rotated_label", render(rotated_scene));
}
