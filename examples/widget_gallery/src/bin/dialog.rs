//! `Dialog` — a window overlay that centres itself over the viewport.
//!
//! Run with `cargo run -p widget-gallery --bin dialog`.
//!
//! A dialog is an ordinary widget plus two window-level facts: it lives in the window's overlay
//! list, and it owns keyboard focus while it is up. Signal callbacks never see the
//! [`WindowManager`], so both facts are reconciled once per frame in [`AppContext::on_update`]:
//! the callback only flips a `SharedProperty<bool>` and the update pass makes the window agree
//! with it.

use rustui::prelude::*;
use widget_gallery::{frame, run, section, status};

fn main() -> rustui::Result<()> {
    run("Dialog", build)
}

/// Parks `id` centred over `window`'s viewport, in **window** coordinates.
///
/// Measuring is a full `measure` pass, which is why it happens here rather than in a signal:
/// the window has to be asked, and only the update pass may ask it. The overlay pass keeps the
/// origin we write and re-measures the size every frame, so a resize recentres itself.
///
/// The horizontal term is mostly a formality here: the file-name field takes the width it is
/// offered (its `SizeHint` width is `INFINITY`), so the dialog measures to the viewport and `x`
/// comes out at zero. The vertical term is what actually centres it.
fn centre_on(wm: &mut WindowManager, window: WindowId, id: ObjectId) {
    let Some(view) = wm.window(window) else {
        return;
    };
    let viewport = view.size();
    let measured =
        MeasureContext::new(wm.ui(), wm.store(), view.interaction(), 0).measure_child(id, viewport);
    let rect = Rect::new(
        ((viewport.width - measured.width) * 0.5).max(0.0),
        ((viewport.height - measured.height) * 0.5).max(0.0),
        measured.width,
        measured.height,
    );
    if let Some(mut widget) = wm.store().get_mut(id) {
        widget.base_mut().set_window_bounds(rect);
    }
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let status = status();

    // The *intention* to show the dialog; the overlay list is the fact.
    let open = SharedProperty::new(false);

    // ---- the dialog itself ---------------------------------------------
    let file_name = SharedProperty::new(String::from("notes.txt"));
    let input = TextInput::with_text(file_name.clone()).placeholder("File name");
    let note = Label::new(
        "Escape, the ✕ and Cancel all converge on close_requested; Save reports the field.",
    )
    .role(TextRole::Caption)
    .wrap(true);
    let ok = Button::new("Save").variant(ButtonVariant::Primary);
    let cancel = Button::new("Cancel");

    ok.clicked().connect({
        let (open, status, file_name) = (open.clone(), status.clone(), file_name.clone());
        move |_| {
            open.set(false);
            status.set(format!("Saved “{}”", file_name.get()));
        }
    });
    cancel.clicked().connect({
        let (open, status) = (open.clone(), status.clone());
        move |_| {
            open.set(false);
            status.set(String::from("Cancelled"));
        }
    });

    // ---- the page --------------------------------------------------------
    let launch = Button::new("Open the dialog").variant(ButtonVariant::Primary);
    launch.clicked().connect({
        let (open, status) = (open.clone(), status.clone());
        move |_| {
            open.set(true);
            status.set(String::from("Opening the dialog…"));
        }
    });

    let wm = app.wm();

    let note = wm.insert(Box::new(note));
    let caption = wm.insert(Box::new(Label::new("File name").role(TextRole::Caption)));
    let input = wm.insert(Box::new(input));
    let ok = wm.insert(Box::new(ok));
    let cancel = wm.insert(Box::new(cancel));

    let mut buttons = HBox::new()
        .with_spacing(12.0)
        .with_main_align(MainAxisAlign::End);
    buttons.push(ok);
    buttons.push(cancel);
    let buttons = wm.insert(Box::new(buttons));

    let mut body = VBox::new()
        .with_spacing(12.0)
        .with_cross_align(CrossAxisAlign::Stretch);
    body.push(note);
    body.push(caption);
    body.push(input);
    body.push(buttons);
    let body = wm.insert(Box::new(body));

    let dialog_widget = Dialog::new("Save as").child(body).min_width(360.0);
    dialog_widget.close_requested().connect({
        let (open, status) = (open.clone(), status.clone());
        move |_| {
            open.set(false);
            status.set(String::from("Dialog closed"));
        }
    });
    let dialog = wm.insert(Box::new(dialog_widget));

    // `Dialog::child` only fills the base child list. Linking the registry as well is what
    // lets a key press bubble out of the text field up to the dialog, so Escape works no
    // matter which of its children happens to hold focus.
    wm.add_child(dialog, body);
    wm.add_child(body, note);
    wm.add_child(body, caption);
    wm.add_child(body, input);
    wm.add_child(body, buttons);
    wm.add_child(buttons, ok);
    wm.add_child(buttons, cancel);

    let launch = wm.insert(Box::new(launch));

    let mut page = VBox::new()
        .with_spacing(16.0)
        .with_cross_align(CrossAxisAlign::Stretch);
    page.push(wm.insert(Box::new(section("An overlay owned by the window"))));
    page.push(launch);
    let page = wm.insert(Box::new(page));

    // ---- reconcile the overlay every frame --------------------------------
    app.on_update({
        let open = open.clone();
        move |wm| {
            let shown = wm
                .window(window)
                .is_some_and(|view| view.overlays().contains(&dialog));
            let want = open.get();
            if want && !shown {
                centre_on(wm, window, dialog);
                wm.add_overlay(window, dialog, 10.0);
                wm.set_focus(window, dialog);
            } else if !want && shown {
                wm.remove_overlay(window, dialog);
                wm.set_focus(window, launch);
            }
        }
    });

    frame(
        app,
        window,
        "Dialog",
        "The signal only flips a flag; on_update centres the dialog, shows it and moves focus.",
        page,
        &status,
    );
}
