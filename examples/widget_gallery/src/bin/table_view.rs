//! `TableView` — columns over a `TableListModel`, with selection and activation.
//!
//! Run with `cargo run -p widget-gallery --bin table_view`.

use rustui::core::notify;
use rustui::prelude::*;
use widget_gallery::{frame, run, section, status};

fn main() -> rustui::Result<()> {
    run("TableView", build)
}

/// Text of the first column cell in `row`.
fn crate_of(model: &Shared<TableListModel>, row: usize) -> String {
    model.with(|m| {
        m.row(row)
            .and_then(|cells| cells.first().cloned())
            .unwrap_or_default()
    })
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let status = status();

    let model = Shared::new(TableListModel::from_headers(["Crate", "Owner", "Stars"]));
    model.with_mut(|m| {
        const OWNERS: [&str; 4] = ["gui", "render", "text", "input"];
        for index in 0..500 {
            m.push_row([
                format!("rustui-{index:03}"),
                OWNERS[index % OWNERS.len()].to_string(),
                ((index * 37) % 512).to_string(),
            ]);
        }
    });

    let mut table = TableView::with_model(model.boxed());
    // Columns default to an even split that truncates the crate names; size them instead.
    table.set_column_width(0, 260.0);
    table.set_column_width(1, 160.0);
    table.set_column_width(2, 160.0);

    {
        let (model, status) = (model.clone(), status.clone());
        table.selection_changed().connect(move |change| {
            status.set(match change.current {
                Some(row) => format!("Selected row {row}: {}", crate_of(&model, row)),
                None => String::from("Selection cleared"),
            });
        });
    }
    {
        let (model, status) = (model.clone(), status.clone());
        table.activated().connect(move |row| {
            status.set(format!(
                "Opened {} — double click or press Enter",
                crate_of(&model, *row)
            ));
        });
    }

    let add = Button::new("Add row");
    add.clicked().connect({
        let (model, status) = (model.clone(), status.clone());
        move |_| {
            let count = model.with(|m| m.row_count());
            model.with_mut(|m| {
                m.push_row([
                    format!("rustui-{count:03}"),
                    "input".to_string(),
                    ((count * 37) % 512).to_string(),
                ])
            });
            notify::request_redraw();
            status.set(format!("Added a row, {} rows now", count + 1));
        }
    });

    let remove = Button::new("Remove last");
    remove.clicked().connect({
        let (model, status) = (model.clone(), status.clone());
        move |_| {
            let removed = model.with_mut(|m| {
                let last = m.row_count().saturating_sub(1);
                m.remove_row(last)
            });
            notify::request_redraw();
            status.set(if removed {
                format!("Removed a row, {} rows left", model.with(|m| m.row_count()))
            } else {
                String::from("Nothing left to remove")
            });
        }
    });

    let clear = Button::new("Clear");
    clear.clicked().connect({
        let (model, status) = (model.clone(), status.clone());
        move |_| {
            model.with_mut(|m| m.clear());
            notify::request_redraw();
            status.set(String::from("Cleared the model"));
        }
    });

    let wm = app.wm();

    let table = wm.insert(Box::new(table));

    let mut tools = HBox::new().with_spacing(12.0);
    tools.push(wm.insert(Box::new(add)));
    tools.push(wm.insert(Box::new(remove)));
    tools.push(wm.insert(Box::new(clear)));
    let tools = wm.insert(Box::new(tools));

    let mut page = VBox::new()
        .with_spacing(16.0)
        .with_cross_align(CrossAxisAlign::Stretch);
    page.push(wm.insert(Box::new(section("500 rows over a TableListModel"))));
    page.push(tools);
    page.push(table);
    page.layout_mut().set_flex_of(table, 1.0);
    let page = wm.insert(Box::new(page));

    frame(
        app,
        window,
        "TableView",
        "Headers come from the model; edits to the model flow straight into the table.",
        page,
        &status,
    );
}
