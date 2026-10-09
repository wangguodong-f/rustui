//! A data heavy window: `TableView`, `TreeView`, `ComboBox`, a background task and a theme
//! switch.
//!
//! Run it with `cargo run -p table-view-example`. Everything the views show lives in
//! `rustui-model` (`TableListModel`, `TreeListModel`); the widgets only read them, so adding a
//! row is a model edit followed by `notify::request_redraw()`.

use rustui::core::notify;
use rustui::core::TaskId;
use rustui::prelude::*;

/// Task id used by the "Work" button; the answer comes back through [`AppContext::on_task`].
const STATS_TASK: TaskId = TaskId(1);

fn main() -> rustui::Result<()> {
    Application::new()?
        .window(
            WindowSpec::new()
                .title("RustUI — Tables and trees")
                .size(960.0, 640.0)
                .min_size(640.0, 420.0),
        )
        .run(build)
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let theme = app.ui().theme().clone();
    let status = SharedProperty::new(String::from("Ready"));

    // ---- models --------------------------------------------------------

    let table_model = Shared::new(TableListModel::from_headers(["Crate", "Owner", "Stars"]));
    table_model.with_mut(|model| {
        const OWNERS: [&str; 4] = ["gui", "render", "text", "input"];
        for index in 0..100 {
            model.push_row([
                format!("rustui-{index:03}"),
                OWNERS[index % OWNERS.len()].to_string(),
                format!("{}", (index * 37) % 512),
            ]);
        }
    });

    let tree_model = Shared::new(TreeListModel::new());
    tree_model.with_mut(|model| {
        let root = model.append(None, "rustui");
        let crates = model.append(Some(root), "crates");
        for name in [
            "rustui-core",
            "rustui-layout",
            "rustui-widgets",
            "rustui-renderer",
        ] {
            model.append(Some(crates), name);
        }
        let examples = model.append(Some(root), "examples");
        for name in ["hello", "calculator", "table_view"] {
            model.append(Some(examples), name);
        }
        model.set_expanded(&root, true);
        model.set_expanded(&crates, true);
    });

    // ---- views ---------------------------------------------------------

    let table = TableView::with_model(table_model.boxed());
    let table_selection = table.selection_changed();
    let table_activated = table.activated();

    let tree = TreeView::with_model(tree_model.boxed());
    let tree_activated = tree.activated();

    let themes = ComboBox::new(["Light theme", "Dark theme"]);
    let theme_choice = themes.selection_property();

    // Signals fire while the window is dispatching an event, so the only thing a callback can
    // do is edit shared state; `SharedProperty::set` schedules the repaint itself.
    {
        let status = status.clone();
        table_selection.connect(move |change| {
            status.set(match change.current {
                Some(row) => format!("Selected row {row}"),
                None => String::from("No row selected"),
            });
        });
    }
    {
        let status = status.clone();
        table_activated.connect(move |row| status.set(format!("Opened crate of row {row}")));
    }
    {
        let status = status.clone();
        tree_activated.connect(move |node| status.set(format!("Selected {node}")));
    }

    // ---- toolbar actions ----------------------------------------------

    let add = Button::new("Add row");
    {
        let (model, status) = (table_model.clone(), status.clone());
        add.clicked().connect(move |_| {
            let rows = model.with_mut(|model| {
                let row = model.row_count();
                model.push_row([
                    format!("rustui-{row:03}"),
                    String::from("you"),
                    String::from("0"),
                ]);
                model.row_count()
            });
            status.set(format!("Added row, {rows} rows now"));
            notify::request_redraw();
        });
    }

    let remove = Button::new("Delete row");
    {
        let (model, status) = (table_model.clone(), status.clone());
        remove.clicked().connect(move |_| {
            let removed = model.with_mut(|model| {
                let last = model.row_count().checked_sub(1)?;
                model.remove_row(last);
                Some(last)
            });
            match removed {
                Some(row) => status.set(format!("Deleted row {row}")),
                None => status.set(String::from("Nothing left to delete")),
            }
            notify::request_redraw();
        });
    }

    let work = Button::new("Work (background)");
    {
        let proxy = app.proxy();
        let status = status.clone();
        work.clicked().connect(move |_| {
            status.set(String::from("Working…"));
            let proxy = proxy.clone();
            std::thread::spawn(move || {
                // Real background work: only `Send` values cross the thread boundary.
                let checksum: u64 = (1..=250_000u64)
                    .map(|value| value.wrapping_mul(2_654_435_761) % 97)
                    .sum();
                proxy.post_task(STATS_TASK.0, checksum);
            });
        });
    }

    // The result arrives on the UI thread, where shared state may be touched again.
    let status_for_task = status.clone();
    app.on_task(move |task, payload| {
        if task != STATS_TASK {
            return;
        }
        let Ok(checksum) = payload.downcast::<u64>() else {
            return;
        };
        status_for_task.set(format!("Background checksum: {checksum}"));
    });

    // Theme switching needs the window manager, which a signal callback cannot reach; the
    // application tick can.
    let last_theme = std::cell::Cell::new(usize::MAX);
    app.on_update(move |wm| {
        let index = theme_choice.get().unwrap_or(0);
        if last_theme.replace(index) == index {
            return;
        }
        let theme = if index == 1 {
            Theme::dark()
        } else {
            Theme::light()
        };
        wm.ui_mut().set_theme(theme);
    });

    // ---- assembly ------------------------------------------------------

    let status_label = Label::with_text(status).role(TextRole::Caption);

    let wm = app.wm();
    let table = wm.insert(Box::new(table));
    let tree = wm.insert(Box::new(tree));
    let themes = wm.insert(Box::new(themes));
    let add = wm.insert(Box::new(add));
    let remove = wm.insert(Box::new(remove));
    let work = wm.insert(Box::new(work));
    let status_label = wm.insert(Box::new(status_label));

    let mut toolbar = HBox::new()
        .with_spacing(theme.spacing.sm)
        .with_cross_align(CrossAxisAlign::Center);
    toolbar.push(themes);
    toolbar.push(add);
    toolbar.push(remove);
    toolbar.push(work);
    toolbar.push(status_label);
    toolbar.layout_mut().set_flex_of(status_label, 1.0);
    let toolbar = wm.insert(Box::new(toolbar));

    let mut panes = HBox::new().with_spacing(theme.spacing.md);
    panes.push(table);
    panes.push(tree);
    panes.layout_mut().set_flex_of(table, 3.0);
    panes.layout_mut().set_flex_of(tree, 1.0);
    let panes = wm.insert(Box::new(panes));

    let mut root = VBox::new()
        .with_spacing(theme.spacing.md)
        .with_padding(Insets::all(theme.spacing.md))
        .with_fill();
    root.push(toolbar);
    root.push(panes);
    root.layout_mut().set_flex_of(panes, 1.0);
    let root = wm.insert(Box::new(root));
    wm.set_root_id(window, root);
}
