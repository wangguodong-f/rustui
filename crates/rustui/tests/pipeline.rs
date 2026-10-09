//! End-to-end frame without a GPU: real fonts, real layout, real painting.
//!
//! The widget unit tests use a stub text measurer; this one drives the pipeline a real window runs
//! (`update -> layout -> paint`) and checks the three failures that keep text from reaching the
//! screen: no [`GlyphSource`] behind the measurer, geometry drawn outside the clip that contains
//! it, and a filling child measuring its siblings down to zero width.

use std::time::Duration;

use rustui::core::{PaintCommand, Rect};
use rustui::prelude::*;
use rustui::text::TextEngine;
use rustui::window::WindowOptions;

const WIDTH: f32 = 560.0;
const HEIGHT: f32 = 380.0;
/// Row gap of the layout below; the row must still be filled end to end.
const GAP: f32 = 8.0;

fn frame() -> FrameClock {
    FrameClock {
        frame: 1,
        delta: Duration::ZERO,
        elapsed: Duration::ZERO,
    }
}

/// A label, a filling text field and a button in one row, wrapped in a filling root.
struct Fixture {
    wm: WindowManager,
    window: WindowId,
    root: ObjectId,
    row: ObjectId,
    input: ObjectId,
    button: ObjectId,
}

fn fixture() -> Fixture {
    let mut wm = WindowManager::new(Box::new(TextEngine::new()));
    let window = wm.create_window(
        WindowOptions::new()
            .title("test")
            .size(WIDTH, HEIGHT)
            .scale_factor(1.0),
    );

    let label = Label::with_text(SharedProperty::new(String::from("Hello, RustUI!")));
    let button = Button::new("Greet");
    let input = TextInput::with_text(SharedProperty::new(String::new())).placeholder("Your name");

    let label = wm.insert(Box::new(label));
    let button = wm.insert(Box::new(button));
    let input = wm.insert(Box::new(input));

    let mut row = HBox::new().with_spacing(GAP);
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

    Fixture {
        wm,
        window,
        root,
        row,
        input,
        button,
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

#[test]
fn a_real_text_engine_reaches_the_renderer() {
    let mut fx = fixture();
    fx.wm.render(fx.window, frame());

    // The measurer must hand the renderer a glyph source, otherwise `RenderInput::glyphs` is
    // `None` and the renderer drops every text command.
    let input = fx.wm.render_input(fx.window).expect("painted frame");
    let glyphs = input.glyphs.expect("TextEngine exposes its glyph atlas");
    assert_eq!(input.scale, 1.0);

    // ...and those glyphs must actually rasterise, for every string in the frame.
    let painter = fx
        .wm
        .window(fx.window)
        .expect("window")
        .painter()
        .commands()
        .to_vec();
    let mut seen = 0;
    for command in &painter {
        let PaintCommand::Text {
            run, range, style, ..
        } = command
        else {
            continue;
        };
        seen += 1;
        let arena = fx.wm.window(fx.window).expect("window").painter().arena();
        let interned = arena.get(*run).expect("interned run");
        let text = &interned[range.start as usize..range.end as usize];
        assert!(!text.is_empty(), "empty interned run");
        assert!(
            !glyphs
                .glyphs(text, style, None, rustui::core::Point::ZERO)
                .is_empty(),
            "no glyphs for {text:?}"
        );
    }
    assert!(seen >= 3, "expected at least 3 text runs, got {seen}");
}

#[test]
fn a_filling_child_does_not_starve_its_sibling() {
    let mut fx = fixture();
    fx.wm.render(fx.window, frame());

    let row = bounds(&fx.wm, fx.row);
    let input = bounds(&fx.wm, fx.input);
    let button = bounds(&fx.wm, fx.button);
    let root = bounds(&fx.wm, fx.root);

    // The filling root owns the whole window...
    assert!(
        (root.width - WIDTH).abs() < 0.5 && (root.height - HEIGHT).abs() < 0.5,
        "root {root:?} does not fill {WIDTH}x{HEIGHT}"
    );

    assert!(
        button.width > 1.0,
        "button collapsed to {button:?} behind {input:?}"
    );
    assert!(
        (input.right() + GAP - button.x).abs() < 0.5,
        "row not filled: input {input:?}, button {button:?}"
    );
    assert!(
        (button.right() - row.right()).abs() < 0.5,
        "button {button:?} does not reach the row end {row:?}"
    );
    assert!(
        input.width > 1.0 && (input.x - row.x).abs() < 0.5,
        "input {input:?} does not start the row {row:?}"
    );
}

#[test]
fn every_command_lands_inside_the_clip_that_contains_it() {
    let mut fx = fixture();
    fx.wm.render(fx.window, frame());

    let window = fx.wm.window(fx.window).expect("window");
    let glyphs = fx
        .wm
        .ui()
        .text()
        .glyph_source()
        .expect("TextEngine rasterises glyphs");
    let mut clips = vec![Rect::new(0.0, 0.0, WIDTH, HEIGHT)];

    for command in window.painter().commands() {
        let clip = *clips.last().expect("clip stack");
        match command {
            PaintCommand::PushClip { rect, .. } => clips.push(clip.intersection(*rect)),
            PaintCommand::PopClip => {
                clips.pop().expect("unbalanced PopClip");
                assert!(!clips.is_empty(), "clip stack underflow");
            }
            PaintCommand::Shape { rect, .. } | PaintCommand::Image { rect, .. }
                if rect.width > 0.0 && rect.height > 0.0 =>
            {
                assert!(
                    clip.width > 0.0 && clip.height > 0.0,
                    "shape {rect:?} drawn into the empty clip {clip:?}"
                );
            }
            PaintCommand::Text {
                position,
                run,
                range,
                style,
                ..
            } => {
                assert!(
                    clip.width > 0.0 && clip.height > 0.0,
                    "text {position:?} drawn into an empty clip"
                );
                assert!(
                    position.x >= clip.left() - 1.0
                        && position.x <= clip.right() + 1.0
                        && position.y >= clip.top() - 1.0
                        && position.y <= clip.bottom() + 1.0,
                    "text {position:?} escaped its clip {clip:?}"
                );
                let arena = window.painter().arena();
                let run = arena.get(*run).expect("interned run");
                let text = &run[range.start as usize..range.end as usize];
                assert!(!glyphs
                    .glyphs(text, style, None, rustui::core::Point::ZERO)
                    .is_empty());
            }
            _ => {}
        }
    }
    assert_eq!(clips.len(), 1, "unbalanced clip stack");

    // The window background is painted by the window itself, before anything else.
    let first = window.painter().commands().first();
    assert!(
        matches!(
            first,
            Some(PaintCommand::Shape { rect, .. }) if rect.x == 0.0
                && rect.y == 0.0
                && rect.width == WIDTH
                && rect.height == HEIGHT
        ),
        "expected a window background fill first, got {first:?}"
    );
}

/// A container paints its background, then its child, then its border — so the border must be a
/// ring, not a filled rect, or it hides what it wraps.
#[test]
fn a_container_border_is_a_ring_and_its_child_stays_visible() {
    let mut wm = WindowManager::new(Box::new(TextEngine::new()));
    let window = wm.create_window(
        WindowOptions::new()
            .title("t")
            .size(400.0, 300.0)
            .scale_factor(1.0),
    );

    let label = Label::with_text(SharedProperty::new(String::from("0")))
        .align(TextAlign::Right)
        .role(TextRole::Title);
    let label = wm.insert(Box::new(label));
    let screen = Container::new()
        .child(label)
        .with_background(rustui::core::Color::WHITE)
        .with_border(rustui::core::Color::BLACK, 1.0)
        .with_radius(4.0)
        .with_padding(Insets::all(12.0))
        .with_fill();
    let screen = wm.insert(Box::new(screen));
    wm.set_root_id(window, screen);

    wm.render(window, frame());

    let label_bounds = bounds(&wm, label);
    assert!(
        label_bounds.width > 0.0 && label_bounds.height > 0.0,
        "the child of a filling container must be arranged, got {label_bounds:?}"
    );

    let painted = wm.window(window).expect("window").painter().commands();
    let text = painted
        .iter()
        .find_map(|command| match command {
            PaintCommand::Text { position, run, .. } => Some((
                *position,
                wm.window(window)
                    .unwrap()
                    .painter()
                    .arena()
                    .get(*run)
                    .unwrap(),
            )),
            _ => None,
        })
        .expect("the label painted its text");
    assert_eq!(text.1, "0");
    assert!(
        label_bounds.contains(text.0),
        "text at {:?} outside the label {label_bounds:?}",
        text.0
    );

    // The border command is a ring: transparent base colour plus visible edge widths.
    let ring = painted
        .iter()
        .filter(|command| {
            matches!(
                command,
                PaintCommand::Shape { fill, .. }
                    if fill.border.widths.top > 0.0 || fill.border.widths.left > 0.0
            )
        })
        .count();
    assert!(ring >= 1, "the container border was not emitted as a ring");
    assert!(
        painted.iter().any(|command| {
            matches!(
                command,
                PaintCommand::Shape { fill, .. }
                    if fill.color.a == 0.0
                        && (fill.border.widths.top > 0.0 || fill.border.widths.left > 0.0)
            )
        }),
        "a solid rect must never stand in for a border: it covers the children"
    );
}
