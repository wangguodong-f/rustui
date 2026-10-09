//! Smallest complete RustUI program: a label, a text field and a button.
//!
//! Run it with `cargo run -p hello`. It exercises the whole MVP checklist: a real window,
//! Label / Button / TextInput, keyboard and mouse input, resize, layout and GPU rendering.

use rustui::prelude::*;

fn main() -> rustui::Result<()> {
    Application::new()?
        .window(WindowSpec::new().title("RustUI — Hello").size(560.0, 380.0))
        .run(build)
}

/// Builds the widget tree; runs once, after the window exists and before the first frame.
fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");

    // State lives outside the widgets: `SharedProperty` notifies observers and requests a
    // repaint by itself, so nothing has to poke the widget tree.
    let greeting = SharedProperty::new(String::from("Type your name below."));
    let name = SharedProperty::new(String::new());

    let title = Label::new("Hello, RustUI!").role(TextRole::Heading);
    let message = Label::with_text(greeting.clone()).wrap(true);
    let input = TextInput::with_text(name.clone()).placeholder("Your name");
    let greet = Button::new("Greet");
    let hint = Label::new("Tab moves focus · Enter submits · resize me").role(TextRole::Caption);

    // Both the button and Enter run the same action.
    {
        let (greeting, name) = (greeting.clone(), name.clone());
        greet.clicked().connect(move |_| show(&greeting, &name));
    }
    {
        let (greeting, name) = (greeting.clone(), name.clone());
        input.submitted().connect(move |_| show(&greeting, &name));
    }

    let wm = app.wm();
    let title = wm.insert(Box::new(title));
    let message = wm.insert(Box::new(message));
    let input = wm.insert(Box::new(input));
    let greet = wm.insert(Box::new(greet));
    let hint = wm.insert(Box::new(hint));

    // Text field on the left, button on the right of the same row.
    let mut row = HBox::new().with_spacing(8.0);
    row.push(input);
    row.push(greet);
    row.layout_mut().set_flex_of(input, 1.0);
    let row = wm.insert(Box::new(row));

    let root = VBox::new()
        .with_spacing(12.0)
        .with_padding(Insets::all(16.0))
        .with_cross_align(CrossAxisAlign::Stretch)
        .with_fill()
        .with_child(title)
        .with_child(message)
        .with_child(row)
        .with_child(hint);
    let root = wm.insert(Box::new(root));
    wm.set_root_id(window, root);
}

/// One action shared by the button and the text field.
fn show(greeting: &SharedProperty<String>, name: &SharedProperty<String>) {
    let name = name.get();
    let text = if name.trim().is_empty() {
        String::from("Type your name below.")
    } else {
        format!("Hello, {name}!")
    };
    greeting.set(text);
}
