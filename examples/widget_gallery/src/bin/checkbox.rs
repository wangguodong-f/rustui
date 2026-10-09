//! `CheckBox` — captions, the bare box, a caption bound to a property and group control.
//!
//! Run with `cargo run -p widget-gallery --bin checkbox`.

use rustui::core::widget::Object;
use rustui::prelude::*;
use widget_gallery::{frame, run, section, status};

fn main() -> rustui::Result<()> {
    run("CheckBox", build)
}

/// How many of `boxes` are ticked.
fn count_of(boxes: &[SharedProperty<bool>; 3]) -> usize {
    boxes.iter().filter(|b| b.get()).count()
}

/// Rewrites the bound caption so the master always reports the tally.
fn sync_caption(caption: &SharedProperty<String>, boxes: &[SharedProperty<bool>; 3]) {
    caption.set(format!("Select all ({} of 3)", count_of(boxes)));
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let status = status();

    let releases = CheckBox::new("Notify me about releases");
    let digest = CheckBox::new("Send a weekly digest").tooltip("CheckBox::tooltip");
    let mut offline = CheckBox::new("Work offline (disabled)");
    offline.base_mut().set_enabled(false);

    // `bare()` draws only the box; the caption becomes an ordinary label you place yourself.
    let bare = CheckBox::bare();
    let bare_caption = Label::new("bare() next to a Label");

    // The children come first: the group reads their live properties to stay in sync.
    let alpha = CheckBox::new("Alpha");
    let beta = CheckBox::new("Beta");
    let gamma = CheckBox::new("Gamma");
    let boxes = [
        alpha.checked_property(),
        beta.checked_property(),
        gamma.checked_property(),
    ];

    // The master's caption is itself bound, so it reports the tally it just produced.
    let caption = SharedProperty::new(String::from("Select all (0 of 3)"));
    let master = CheckBox::with_text(caption.clone());

    {
        let (status, caption, boxes) = (status.clone(), caption.clone(), boxes.clone());
        master.toggled().connect(move |on| {
            for b in &boxes {
                b.set(*on);
            }
            sync_caption(&caption, &boxes);
            status.set(format!("Select all = {on} ({} of 3)", count_of(&boxes)));
        });
    }
    for (boxed, label) in [(&alpha, "Alpha"), (&beta, "Beta"), (&gamma, "Gamma")] {
        let (status, caption, boxes) = (status.clone(), caption.clone(), boxes.clone());
        boxed.toggled().connect(move |on| {
            sync_caption(&caption, &boxes);
            status.set(format!("{label} is now {on}"));
        });
    }

    let wm = app.wm();

    // Not stretched: `CheckBox` centres its caption in the space to the right of the box,
    // so a full-width row would strand the text in the middle of the window.
    let mut page = VBox::new().with_spacing(16.0);

    page.push(wm.insert(Box::new(section("Captions and state"))));
    page.push(wm.insert(Box::new(releases)));
    page.push(wm.insert(Box::new(digest)));
    page.push(wm.insert(Box::new(offline)));

    let mut bare_row = HBox::new().with_spacing(8.0);
    bare_row.push(wm.insert(Box::new(bare)));
    bare_row.push(wm.insert(Box::new(bare_caption)));
    page.push(wm.insert(Box::new(bare_row)));

    page.push(wm.insert(Box::new(section("One master, three children"))));

    let mut group = HBox::new().with_spacing(16.0);
    group.push(wm.insert(Box::new(master)));
    group.push(wm.insert(Box::new(alpha)));
    group.push(wm.insert(Box::new(beta)));
    group.push(wm.insert(Box::new(gamma)));
    page.push(wm.insert(Box::new(group)));

    let page = wm.insert(Box::new(page));
    frame(
        app,
        window,
        "CheckBox",
        "Captions, the bare box, a bound caption and a master that drives three children.",
        page,
        &status,
    );
}
