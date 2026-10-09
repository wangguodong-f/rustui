//! `Label` — text roles, wrapping, alignment, colour, rotation, tooltips and live text.
//!
//! Run with `cargo run -p widget-gallery --bin label`.

use rustui::prelude::*;
use widget_gallery::{frame, run, status};

fn main() -> rustui::Result<()> {
    run("Label", build)
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let status = status();

    // Roles: the theme maps each one to a size, weight and colour.
    let heading = Label::new("Heading role").role(TextRole::Heading);
    let title = Label::new("Title role").role(TextRole::Title);
    let body = Label::new("Body role — the default for paragraphs.").role(TextRole::Body);
    let caption = Label::new("Caption role — small secondary text.").role(TextRole::Caption);
    let mono = Label::new("let answer = 42; // Mono role").role(TextRole::Mono);

    // A label is a single line unless it is told to wrap.
    let wrapped = Label::new(
        "wrap(true) lets this sentence break into several lines inside the width the layout \
         gives it; without it a label keeps one line and lets the text run past its box.",
    )
    .wrap(true);

    // Alignment only shows once the label is wider than its text, so the page stretches it.
    let left = Label::new("align(TextAlign::Left)").align(TextAlign::Left);
    let center = Label::new("align(TextAlign::Center)").align(TextAlign::Center);
    let right = Label::new("align(TextAlign::Right)").align(TextAlign::Right);

    // Colour overrides the role's text colour; rotation turns the whole measured block.
    let colored = Label::new("color(Color::…) overrides the role colour")
        .color(Color::from_rgb32(0x2f_9e_44));
    let rotated = Label::new("rotation(90.0)").rotation(90.0);
    let hovered = Label::new("Hover me for a tooltip").tooltip("Labels carry tooltips too");

    // Live text: one property feeds both the label and the field below it.
    let typed = SharedProperty::new(String::from("Edit me below"));
    let live = Label::with_text(typed.clone()).role(TextRole::Title);
    let input = TextInput::with_text(typed).placeholder("Type here");
    {
        let status = status.clone();
        input.changed().connect(move |text| {
            status.set(format!("{} characters typed", text.chars().count()));
        });
    }

    let wm = app.wm();
    let mut page = VBox::new()
        .with_spacing(8.0)
        .with_cross_align(CrossAxisAlign::Stretch);

    page.push(wm.insert(Box::new(heading)));
    page.push(wm.insert(Box::new(title)));
    page.push(wm.insert(Box::new(body)));
    page.push(wm.insert(Box::new(caption)));
    page.push(wm.insert(Box::new(mono)));
    page.push(wm.insert(Box::new(wrapped)));
    page.push(wm.insert(Box::new(left)));
    page.push(wm.insert(Box::new(center)));
    page.push(wm.insert(Box::new(right)));
    page.push(wm.insert(Box::new(colored)));
    page.push(wm.insert(Box::new(rotated)));
    page.push(wm.insert(Box::new(hovered)));
    page.push(wm.insert(Box::new(live)));
    page.push(wm.insert(Box::new(input)));

    let page = wm.insert(Box::new(page));
    frame(
        app,
        window,
        "Label",
        "Roles, wrapping, alignment, colour, rotation, tooltips and a live binding.",
        page,
        &status,
    );
}
