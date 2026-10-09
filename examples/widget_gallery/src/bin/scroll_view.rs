//! `ScrollView` — a viewport over content that is taller than the window.
//!
//! Run with `cargo run -p widget-gallery --bin scroll_view`.
//!
//! The scroll offset is widget state, so the application cannot reach it once the widget is in
//! the store — what it *can* do is build content freely and let the view clip it. The widgets
//! inside keep receiving events because hit testing walks the real child tree.

use rustui::prelude::*;
use widget_gallery::{frame, run, section, status};

fn main() -> rustui::Result<()> {
    run("ScrollView", build)
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let status = status();

    let wm = app.wm();

    // The content is an ordinary tree; nothing about it says "scrollable".
    let mut content = VBox::new()
        .with_spacing(6.0)
        .with_padding(Insets::all(12.0));

    for index in 0..20 {
        content.push(wm.insert(Box::new(Label::new(format!(
            "Line {index:02} — part of the content, laid out normally."
        )))));
    }

    content.push(wm.insert(Box::new(section("Still interactive in here"))));
    content.push(wm.insert(Box::new(Label::new(
        "These controls live below the fold: wheel down to reach them.",
    ))));

    let bumps = SharedProperty::new(0i32);
    let bump = Button::new("Bump the counter").variant(ButtonVariant::Primary);
    bump.clicked().connect({
        let (bumps, status) = (bumps.clone(), status.clone());
        move |_| {
            let count = bumps.get() + 1;
            bumps.set(count);
            status.set(format!(
                "The button inside the scroller fired {count} time(s)"
            ));
        }
    });
    let remember = CheckBox::new("Remember this");
    remember.toggled().connect({
        let status = status.clone();
        move |on| status.set(format!("CheckBox inside the scroller is now {on}"))
    });

    let mut controls = HBox::new().with_spacing(12.0);
    controls.push(wm.insert(Box::new(bump)));
    controls.push(wm.insert(Box::new(remember)));
    content.push(wm.insert(Box::new(controls)));

    for index in 20..45 {
        content.push(wm.insert(Box::new(Label::new(format!(
            "Line {index:02} — reached by scrolling, not by resizing the window."
        )))));
    }

    let content = wm.insert(Box::new(content));
    let scroll = wm.insert(Box::new(ScrollView::new().child(content)));

    let mut page = VBox::new()
        .with_spacing(16.0)
        .with_cross_align(CrossAxisAlign::Stretch);
    page.push(wm.insert(Box::new(section("Content taller than the viewport"))));
    page.push(wm.insert(Box::new(Label::new(
        "The view clips its child and draws a thumb while the content overflows.",
    ))));
    page.push(scroll);
    page.layout_mut().set_flex_of(scroll, 1.0);
    let page = wm.insert(Box::new(page));

    frame(
        app,
        window,
        "ScrollView",
        "Build content as usual; the ScrollView supplies the viewport, the clip and the thumb.",
        page,
        &status,
    );
}
