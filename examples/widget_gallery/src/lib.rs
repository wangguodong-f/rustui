//! Shared scaffolding for the per-widget example binaries.
//!
//! Each binary under `src/bin` demonstrates exactly one widget:
//!
//! ```text
//! cargo run -p widget-gallery --bin label
//! cargo run -p widget-gallery --bin dialog
//! ```
//!
//! [`run`] opens the window and [`frame`] wraps the example's body in the standard page shell
//! (heading, hint, flexible body, status line) so every example reads the same way.

use rustui::prelude::*;

/// Runs `build` in a window titled after `title`.
pub fn run(title: &str, build: impl FnOnce(&mut AppContext<'_>) + 'static) -> rustui::Result<()> {
    Application::new()?
        .window(
            WindowSpec::new()
                .title(format!("RustUI — {title}"))
                .size(820.0, 640.0)
                .min_size(560.0, 400.0),
        )
        .run(build)
}

/// The status property every example starts from.
///
/// Callbacks write into it to narrate what just happened; [`frame`] renders it as the bottom
/// line of the page.
pub fn status() -> SharedProperty<String> {
    SharedProperty::new(String::from("Ready"))
}

/// A small heading that names a group of controls inside a page body.
pub fn section(text: &str) -> Label {
    Label::new(text).role(TextRole::Title)
}

/// Builds the standard page shell around `body` and installs it as the window root.
///
/// The heading and the hint are static text; `body` is flexed so it takes every remaining
/// pixel, which pushes `status` to the bottom edge of the window.
pub fn frame(
    app: &mut AppContext<'_>,
    window: WindowId,
    title: &str,
    hint: &str,
    body: ObjectId,
    status: &SharedProperty<String>,
) {
    let wm = app.wm();
    let heading = wm.insert(Box::new(Label::new(title).role(TextRole::Heading)));
    let hint = wm.insert(Box::new(
        Label::new(hint).role(TextRole::Caption).wrap(true),
    ));
    let status_line = wm.insert(Box::new(
        Label::with_text(status.clone()).role(TextRole::Caption),
    ));

    let mut root = VBox::new()
        .with_spacing(12.0)
        .with_padding(Insets::all(16.0))
        .with_cross_align(CrossAxisAlign::Stretch)
        .with_fill();
    root.push(heading);
    root.push(hint);
    root.push(body);
    root.push(status_line);
    root.layout_mut().set_flex_of(body, 1.0);

    let root = wm.insert(Box::new(root));
    wm.set_root_id(window, root);
}
