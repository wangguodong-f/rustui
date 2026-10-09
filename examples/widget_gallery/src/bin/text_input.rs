//! `TextInput` — placeholders, password mode, edit/submit signals and external control.
//!
//! Run with `cargo run -p widget-gallery --bin text_input`.

use rustui::prelude::*;
use widget_gallery::{frame, run, section, status};

fn main() -> rustui::Result<()> {
    run("TextInput", build)
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let status = status();

    // The field owns nothing: the text lives in this property.
    let text = SharedProperty::new(String::from("Hello, RustUI"));
    let input = TextInput::with_text(text.clone())
        .placeholder("Type here")
        .tooltip("TextInput::tooltip");
    let echo = Label::with_text(text.clone()).role(TextRole::Title);

    // `changed` fires on every edit, `submitted` on Enter.
    {
        let status = status.clone();
        input.changed().connect(move |value| {
            status.set(format!("{} characters", value.chars().count()));
        });
    }

    let last = SharedProperty::new(String::from("Press Enter to submit"));
    let last_line = Label::with_text(last.clone()).role(TextRole::Caption);
    {
        let (last, status) = (last.clone(), status.clone());
        input.submitted().connect(move |value| {
            last.set(format!("Submitted: {value}"));
            status.set(format!("Enter submitted “{value}”"));
        });
    }

    // Password fields hide their text but bind to the same kind of property.
    let secret = TextInput::new()
        .password(true)
        .placeholder("Password")
        .tooltip("password(true)");

    // External control: buttons write straight into the bound property.
    let clear = Button::new("Clear");
    clear.clicked().connect({
        let text = text.clone();
        move |_| text.set(String::new())
    });
    let fill = Button::new("Fill");
    fill.clicked().connect({
        let text = text.clone();
        move |_| text.set(String::from("Written from outside the widget"))
    });

    let wm = app.wm();

    let mut page = VBox::new()
        .with_spacing(16.0)
        .with_cross_align(CrossAxisAlign::Stretch);

    page.push(wm.insert(Box::new(section("Bound field"))));
    page.push(wm.insert(Box::new(echo)));
    page.push(wm.insert(Box::new(input)));

    let mut buttons = HBox::new().with_spacing(12.0);
    buttons.push(wm.insert(Box::new(clear)));
    buttons.push(wm.insert(Box::new(fill)));
    page.push(wm.insert(Box::new(buttons)));

    page.push(wm.insert(Box::new(section("Password mode"))));
    page.push(wm.insert(Box::new(secret)));

    page.push(wm.insert(Box::new(section("Submit"))));
    page.push(wm.insert(Box::new(last_line)));

    let page = wm.insert(Box::new(page));
    frame(
        app,
        window,
        "TextInput",
        "Placeholder, password mode, changed/submitted signals and writes from outside.",
        page,
        &status,
    );
}
