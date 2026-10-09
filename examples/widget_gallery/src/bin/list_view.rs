//! `ListView` — a model-backed list with selection, activation and a live filter.
//!
//! Run with `cargo run -p widget-gallery --bin list_view`.

use rustui::core::notify;
use rustui::prelude::*;
use widget_gallery::{frame, run, section, status};

fn main() -> rustui::Result<()> {
    run("ListView", build)
}

/// Text behind a *view* row: the list reports visible rows, the model knows the source.
fn label_of(model: &Shared<StringListModel>, view_row: usize) -> String {
    model.with(|m| {
        m.source_row(view_row)
            .and_then(|source| m.get(source))
            .unwrap_or("out of range")
            .to_owned()
    })
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let status = status();

    let model = Shared::new(StringListModel::from_values(
        (0..200).map(|index| format!("Row {index:03}")),
    ));

    // The list is single select; `activate_mode` decides what "open" means.
    let list = ListView::with_model(model.boxed()).activate_mode(ActivateMode::DoubleClickAndEnter);

    {
        let (model, status) = (model.clone(), status.clone());
        list.selection_changed().connect(move |change| {
            status.set(match change.current {
                Some(row) => format!("Selected view row {row}: {}", label_of(&model, row)),
                None => String::from("Selection cleared"),
            });
        });
    }
    {
        let (model, status) = (model.clone(), status.clone());
        list.activated().connect(move |row| {
            status.set(format!(
                "Opened {} — double click or press Enter",
                label_of(&model, *row)
            ));
        });
    }

    // Model edits are plain mutations; the view repaints when asked to.
    let add = Button::new("Add row");
    add.clicked().connect({
        let (model, status) = (model.clone(), status.clone());
        move |_| {
            let count = model.with(|m| m.len());
            model.with_mut(|m| m.push(format!("Row {count:03}")));
            notify::request_redraw();
            status.set(format!("Added a row, {} rows now", count + 1));
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

    // A filter narrows the *visible* rows; the selection index follows the view, not the model.
    let filter = TextInput::new().placeholder("Filter rows…");
    filter.changed().connect({
        let (model, status) = (model.clone(), status.clone());
        move |text| {
            model.with_mut(|m| m.set_filter(text.as_str()));
            notify::request_redraw();
            status.set(format!("Filter “{text}”"));
        }
    });

    let wm = app.wm();

    let list = wm.insert(Box::new(list));
    let filter = wm.insert(Box::new(filter));
    let add = wm.insert(Box::new(add));
    let remove = wm.insert(Box::new(remove));

    let mut tools = HBox::new().with_spacing(12.0);
    tools.push(filter);
    tools.push(add);
    tools.push(remove);
    let tools = wm.insert(Box::new(tools));

    let mut page = VBox::new()
        .with_spacing(16.0)
        .with_cross_align(CrossAxisAlign::Stretch);
    page.push(wm.insert(Box::new(section("200 rows behind a StringListModel"))));
    page.push(tools);
    page.push(list);
    // `SizeHint::fill()` already expands the list; the explicit weight documents the intent.
    page.layout_mut().set_flex_of(list, 1.0);
    let page = wm.insert(Box::new(page));

    frame(
        app,
        window,
        "ListView",
        "Selection and activation signals over a filtered model; edit the model and watch it follow.",
        page,
        &status,
    );
}
