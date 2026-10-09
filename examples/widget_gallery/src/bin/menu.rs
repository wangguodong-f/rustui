//! `Menu` — a popup anchored to a button, living entirely in the window's overlay list.
//!
//! Run with `cargo run -p widget-gallery --bin menu`.
//!
//! The widget does the interesting half on its own: it draws above the content, owns focus while
//! it is up, and hides itself when focus leaves. The application does the other half — it decides
//! *when* the menu should exist and *where* it goes, which only the update pass can do.

use rustui::prelude::*;
use widget_gallery::{frame, run, section, status};

fn main() -> rustui::Result<()> {
    run("Menu", build)
}

/// The point just below `anchor`, in **window** coordinates. The store guard is dropped inside
/// this function so the caller can take a mutable one — the store is a `RefCell`.
fn anchor_origin(wm: &WindowManager, anchor: ObjectId) -> Option<Point> {
    let widget = wm.store().get(anchor)?;
    let bounds = widget.base().window_bounds();
    Some(Point::new(bounds.x, bounds.bottom()))
}

/// Parks `id` at `rect`; the overlay pass keeps the origin and re-measures the size each frame.
fn park(wm: &mut WindowManager, id: ObjectId, rect: Rect) {
    if let Some(mut widget) = wm.store().get_mut(id) {
        widget.base_mut().set_window_bounds(rect);
    }
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let status = status();

    // The *intention* to show the menu; the overlay list is the fact.
    let open = SharedProperty::new(false);

    let entries = [
        MenuItem::new("New window").shortcut("Ctrl+N"),
        MenuItem::new("Open…").shortcut("Ctrl+O"),
        MenuItem::separator(),
        MenuItem::new("Save").shortcut("Ctrl+S").disabled(),
        MenuItem::separator(),
        MenuItem::new("Close tab").shortcut("Ctrl+W"),
        MenuItem::new("Quit").shortcut("Ctrl+Q"),
    ];
    let labels: Vec<String> = entries.iter().map(|item| item.label.clone()).collect();
    let menu = Menu::new(entries);

    // `activated` reports the raw index, separators included — the labels come from the app.
    {
        let (labels, status) = (labels.clone(), status.clone());
        menu.activated().connect(move |index| {
            let label = labels.get(*index).map(String::as_str).unwrap_or("");
            let name = if label.is_empty() {
                "(separator)"
            } else {
                label
            };
            status.set(format!("Picked {name} — item {index} of the menu"));
        });
    }
    // The menu removes itself from the overlay on selection, `Escape` and focus loss. All three
    // funnel through `closed`, which is what keeps the flag and the window in step.
    {
        let open = open.clone();
        menu.closed().connect(move |_| open.set(false));
    }

    let open_button = Button::new("Show menu")
        .variant(ButtonVariant::Primary)
        .tooltip("Arrows move, Enter picks, Escape closes");
    open_button.clicked().connect({
        let (open, status) = (open.clone(), status.clone());
        move |_| {
            open.set(true);
            status.set(String::from(
                "Menu opened — click elsewhere or press Escape to close",
            ));
        }
    });

    let wm = app.wm();

    let menu = wm.insert(Box::new(menu));
    let open_button = wm.insert(Box::new(open_button));

    let mut page = VBox::new()
        .with_spacing(16.0)
        .with_cross_align(CrossAxisAlign::Stretch);
    page.push(wm.insert(Box::new(section("An overlay anchored to a button"))));
    page.push(wm.insert(Box::new(Label::new(
        "The menu is its own root: it paints above the page, takes focus, and hides itself.",
    ))));
    page.push(open_button);
    let page = wm.insert(Box::new(page));

    app.on_update({
        let open = open.clone();
        move |wm| {
            let shown = wm
                .window(window)
                .is_some_and(|view| view.overlays().contains(&menu));
            let want = open.get();

            if want && !shown {
                // Two separate statements: the guard must be gone before the store is
                // borrowed mutably, and the shared reborrow of `wm` before it is exclusive.
                let origin = anchor_origin(wm, open_button);
                if let Some(origin) = origin {
                    park(wm, menu, Rect::new(origin.x, origin.y, 0.0, 0.0));
                    wm.add_overlay(window, menu, 20.0);
                    wm.set_focus(window, menu);
                }
            } else if !want && shown {
                wm.remove_overlay(window, menu);
            }

            // `hide` runs inside event dispatch, where there is no `WindowManager` in sight.
            // Catch up on the next frame: a menu that is gone should not keep the focus.
            let holds_focus = wm
                .window(window)
                .is_some_and(|view| view.focused() == Some(menu));
            if holds_focus && !want {
                wm.set_focus(window, open_button);
            }
        }
    });

    frame(
        app,
        window,
        "Menu",
        "The button flips a flag, on_update parks the menu under it, and the menu hides itself.",
        page,
        &status,
    );
}
