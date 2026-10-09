//! `ComboBox` — a static list, selection observation and a model you can edit at runtime.
//!
//! Run with `cargo run -p widget-gallery --bin combo_box`.

use rustui::core::notify;
use rustui::prelude::*;
use widget_gallery::{frame, run, section, status};

fn main() -> rustui::Result<()> {
    run("ComboBox", build)
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let status = status();

    // --- A combo over a list that never changes. ---
    let colors = vec![
        String::from("Crimson"),
        String::from("Amber"),
        String::from("Jade"),
        String::from("Indigo"),
    ];
    // An unselected combo draws an empty box, so start both on a real item.
    let mut color_box = ComboBox::new(colors.clone());
    color_box.set_selected(Some(1));

    // The widget signal fires when the user picks; the property observes every change.
    {
        let (status, colors) = (status.clone(), colors.clone());
        color_box.selection_changed().connect(move |index| {
            status.set(match index {
                Some(i) => format!(
                    "Picked {}",
                    colors.get(*i).map(String::as_str).unwrap_or("?")
                ),
                None => String::from("Selection cleared"),
            });
        });
    }
    let picked = SharedProperty::new(color_box.selected_text());
    let picked_line = Label::with_text(picked.clone());
    color_box.selection_property().changed().connect({
        let (picked, colors) = (picked.clone(), colors.clone());
        move |index| {
            picked.set(match index {
                Some(i) => colors
                    .get(*i)
                    .cloned()
                    .unwrap_or_else(|| format!("index {i}")),
                None => String::from("Nothing selected"),
            });
        }
    });

    // --- A combo over a model the application keeps editing. ---
    let model = Shared::new(StringListModel::from_values([
        "Alpha", "Bravo", "Charlie", "Delta", "Echo",
    ]));
    let mut letter_box = ComboBox::with_items(model.boxed());
    letter_box.set_selected(Some(0));
    letter_box.selection_changed().connect({
        let (model, status) = (model.clone(), status.clone());
        move |index| {
            let text = index
                .and_then(|i| model.with(|m| m.get(i).map(str::to_owned)))
                .unwrap_or_else(|| String::from("nothing"));
            status.set(format!("Model combo picked {text}"));
        }
    });

    // Editing the model is a plain mutation plus a repaint request.
    let add = Button::new("Add");
    add.clicked().connect({
        let (model, status) = (model.clone(), status.clone());
        move |_| {
            let count = model.with(|m| m.len());
            model.with_mut(|m| m.push(format!("Item {}", count + 1)));
            notify::request_redraw();
            status.set(format!("Model now holds {} items", count + 1));
        }
    });

    let remove = Button::new("Remove last");
    remove.clicked().connect({
        let (model, status) = (model.clone(), status.clone());
        move |_| {
            let removed = model.with_mut(|m| {
                let last = m.len().saturating_sub(1);
                m.remove(last)
            });
            notify::request_redraw();
            status.set(removed.map_or_else(
                || String::from("Nothing left to remove"),
                |v| format!("Removed {v}"),
            ));
        }
    });

    // A filter narrows the rows the combo shows.
    let filter = TextInput::new().placeholder("Filter the model…");
    filter.changed().connect({
        let model = model.clone();
        move |text| {
            model.with_mut(|m| m.set_filter(text.as_str()));
            notify::request_redraw();
        }
    });

    let wm = app.wm();

    let mut page = VBox::new()
        .with_spacing(16.0)
        .with_cross_align(CrossAxisAlign::Stretch);

    page.push(wm.insert(Box::new(section("Static items"))));
    page.push(wm.insert(Box::new(color_box)));
    page.push(wm.insert(Box::new(picked_line)));

    page.push(wm.insert(Box::new(section("Model driven"))));
    page.push(wm.insert(Box::new(letter_box)));
    page.push(wm.insert(Box::new(filter)));

    let mut buttons = HBox::new().with_spacing(12.0);
    buttons.push(wm.insert(Box::new(add)));
    buttons.push(wm.insert(Box::new(remove)));
    page.push(wm.insert(Box::new(buttons)));

    let page = wm.insert(Box::new(page));
    frame(
        app,
        window,
        "ComboBox",
        "A static list, selection signals, and a StringListModel edited at runtime.",
        page,
        &status,
    );
}
