//! Criterion benchmarks for the CPU hot paths (§59 of the design documents, T-11 / C-047).
//!
//! ```text
//! cargo bench -p rustui             # everything
//! cargo bench -p rustui -- layout   # one group
//! ```
//!
//! Results land in `target/criterion/`. The GPU is deliberately not measured: this machine uses
//! a software rasteriser, so draw call counts and frame times there would say nothing about a
//! real adapter. Everything below is pure CPU work driven through the same entry points the frame
//! pipeline uses (`WindowManager::layout` / `paint` / `render` / `hit_test`).

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rustui::prelude::*;
use rustui::text::TextEngine;
use rustui::window::WindowOptions;

const WINDOW_W: f32 = 800.0;
const WINDOW_H: f32 = 600.0;

fn frame() -> FrameClock {
    FrameClock {
        frame: 1,
        delta: Duration::ZERO,
        elapsed: Duration::ZERO,
    }
}

/// A window whose root stacks `count` labelled rows - the "N widgets" trees of §59.
fn labels(count: usize) -> (WindowManager, WindowId) {
    let mut wm = WindowManager::new(Box::new(TextEngine::new()));
    let window = wm.create_window(
        WindowOptions::new()
            .title("bench")
            .size(WINDOW_W, WINDOW_H)
            .scale_factor(1.0),
    );
    let mut root = VBox::new()
        .with_spacing(1.0)
        .with_padding(Insets::all(4.0))
        .with_fill();
    for i in 0..count {
        let label = wm.insert(Box::new(Label::with_text(SharedProperty::new(format!(
            "label {i}"
        )))));
        root.push(label);
    }
    let root = wm.insert(Box::new(root));
    wm.set_root_id(window, root);
    (wm, window)
}

/// A window whose root *is* a `TableView` of `rows` entries (virtualised: only the visible
/// window of rows is measured, arranged and painted).
fn table(rows: usize) -> (WindowManager, WindowId) {
    let mut wm = WindowManager::new(Box::new(TextEngine::new()));
    let window = wm.create_window(
        WindowOptions::new()
            .title("bench")
            .size(WINDOW_W, WINDOW_H)
            .scale_factor(1.0),
    );
    let model = Shared::new(TableListModel::from_headers(["Id", "Name", "Value"]));
    model.with_mut(|model| {
        for i in 0..rows {
            model.push_row([i.to_string(), format!("row {i}"), format!("{}", i * 7)]);
        }
    });
    let view = wm.insert(Box::new(TableView::with_model(model.boxed())));
    wm.set_root_id(window, view);
    (wm, window)
}

// ---------------------------------------------------------------------------
// Text measurement (§59 "Text rendering")
// ---------------------------------------------------------------------------

fn bench_text(c: &mut Criterion) {
    let (wm, _) = labels(1);
    let measurer = wm.ui().text();
    let style = TextStyle::default();
    let natural = "The quick brown fox jumps over the lazy dog, again and again.";
    let wrapped = "RustUI measures text on every layout pass: it breaks the paragraph into \
                   lines, shapes each line glyph by glyph and returns the block size the \
                   parent then arranges. This sentence exists so the wrapping path has enough \
                   words to actually produce more than one line at a modest max width.";

    let mut group = c.benchmark_group("text");
    group.bench_function("measure_natural", |b| {
        b.iter(|| measurer.measure_text(natural, &style, None));
    });
    group.bench_function("measure_wrapping", |b| {
        b.iter(|| measurer.measure_text(wrapped, &style, Some(320.0)));
    });
    group.finish();
}

// ---------------------------------------------------------------------------
// Layout: measure + arrange (§59 "Layout")
// ---------------------------------------------------------------------------

fn bench_layout(c: &mut Criterion) {
    let mut group = c.benchmark_group("layout");
    for &count in &[100usize, 1_000] {
        let (mut wm, window) = labels(count);
        // One untimed pass so font resolution and caches are warm.
        wm.layout(window);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &_count| {
            b.iter(|| wm.layout(window))
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Paint generation (§59 "Paint generation")
// ---------------------------------------------------------------------------

fn bench_paint(c: &mut Criterion) {
    let (mut wm, window) = labels(1_000);
    wm.layout(window);
    wm.paint(window);
    let painted = wm
        .window(window)
        .map(|target| target.painter().commands().len())
        .unwrap_or(0);
    assert!(
        painted > 1_000,
        "the 1000 label tree painted only {painted} commands"
    );

    let mut group = c.benchmark_group("paint");
    group.throughput(Throughput::Elements(1_000));
    group.bench_function("labels_1000", |b| {
        b.iter(|| wm.paint(window));
    });
    group.finish();
}

// ---------------------------------------------------------------------------
// Hit testing (§59 "Hit Test")
// ---------------------------------------------------------------------------

fn bench_hit_test(c: &mut Criterion) {
    let (mut wm, window) = labels(1_000);
    wm.layout(window);
    // Walk down the tree: a single point would measure one branch forever.
    let points: Vec<Point> = (0..16)
        .map(|i| Point::new(40.0, 10.0 + i as f32 * 37.0))
        .collect();

    let mut group = c.benchmark_group("hit_test");
    group.throughput(Throughput::Elements(points.len() as u64));
    group.bench_function("labels_1000", |b| {
        let mut cursor = 0usize;
        b.iter(|| {
            let point = points[cursor % points.len()];
            cursor += 1;
            black_box(wm.hit_test(window, point));
        })
    });
    group.finish();
}

// ---------------------------------------------------------------------------
// Whole frame: update + layout + paint (§59 "1000 widgets")
// ---------------------------------------------------------------------------

fn bench_frame(c: &mut Criterion) {
    let (mut wm, window) = labels(1_000);
    let painted = wm.render(window, frame()).map_or(0, |p| p.commands().len());
    assert!(
        painted > 1_000,
        "the 1000 label frame painted only {painted} commands"
    );

    let mut group = c.benchmark_group("frame");
    group.throughput(Throughput::Elements(1_000));
    group.bench_function("labels_1000", |b| {
        b.iter(|| wm.render(window, frame()).map_or(0, |p| p.commands().len()));
    });
    group.finish();
}

// ---------------------------------------------------------------------------
// Tables (§59 "10000 / 100000 table rows")
// ---------------------------------------------------------------------------

fn bench_table(c: &mut Criterion) {
    let mut group = c.benchmark_group("table");
    let mut baseline: Option<usize> = None;
    for &rows in &[10_000usize, 100_000] {
        let (mut wm, window) = table(rows);
        let painted = wm.render(window, frame()).map_or(0, |p| p.commands().len());
        // Virtualisation (§51): the frame describes what is *on screen*, so it must stay small
        // and - the real invariant - must not grow with the model.
        assert!(
            painted > 40 && painted < 500,
            "the {rows} row table painted {painted} commands"
        );
        match baseline {
            None => baseline = Some(painted),
            Some(expected) => assert_eq!(
                expected, painted,
                "painting 10x the rows produced a different frame - virtualisation lost?"
            ),
        }
        group.throughput(Throughput::Elements(rows as u64));
        group.bench_with_input(BenchmarkId::from_parameter(rows), &rows, |b, &_rows| {
            b.iter(|| wm.render(window, frame()).map_or(0, |p| p.commands().len()))
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Widget construction (§59 "Widget creation")
// ---------------------------------------------------------------------------

fn bench_widget_creation(c: &mut Criterion) {
    let texts: Vec<String> = (0..1_000).map(|i| format!("label {i}")).collect();
    let mut group = c.benchmark_group("widget");
    group.throughput(Throughput::Elements(1_000));
    group.bench_function("create_1000_labels", |b| {
        b.iter(|| {
            let widgets: Vec<Box<dyn Widget>> = texts
                .iter()
                .map(|text| {
                    Box::new(Label::with_text(SharedProperty::new(text.clone()))) as Box<dyn Widget>
                })
                .collect();
            black_box(widgets.len())
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_text,
    bench_layout,
    bench_paint,
    bench_hit_test,
    bench_frame,
    bench_table,
    bench_widget_creation,
);
criterion_main!(benches);
