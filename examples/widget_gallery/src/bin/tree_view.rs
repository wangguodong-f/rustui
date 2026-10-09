//! `TreeView` — a nested model with selection, activation and expansion control.
//!
//! Run with `cargo run -p widget-gallery --bin tree_view`.

use rustui::core::notify;
use rustui::prelude::*;
use widget_gallery::{frame, run, section, status};

fn main() -> rustui::Result<()> {
    run("TreeView", build)
}

/// Display text behind an optional node id; `fallback` when nothing is selected.
fn name_of(model: &Shared<TreeListModel>, node: Option<NodeId>, fallback: &str) -> String {
    node.and_then(|id| model.with(|m| m.label(&id).map(str::to_owned)))
        .unwrap_or_else(|| fallback.to_owned())
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let status = status();

    let model = Shared::new(TreeListModel::new());
    model.with_mut(|m| {
        let root = m.append(None, "rustui");
        let crates = m.append(Some(root), "crates");
        for name in [
            "rustui-core",
            "rustui-layout",
            "rustui-widgets",
            "rustui-renderer",
            "rustui-window",
        ] {
            m.append(Some(crates), name);
        }
        let examples = m.append(Some(root), "examples");
        for name in ["hello", "calculator", "table_view", "widget_gallery"] {
            m.append(Some(examples), name);
        }
        m.set_expanded_all(true);
    });

    let tree = TreeView::with_model(model.boxed());

    // The property and the signal are two doors onto the same selection.
    let selection = tree.selection_property();
    let picked = SharedProperty::new(String::from("(nothing selected)"));
    let picked_line = Label::with_text(picked.clone()).role(TextRole::Title);
    selection.changed().connect({
        let (model, picked) = (model.clone(), picked.clone());
        move |node| picked.set(name_of(&model, *node, "(nothing selected)"))
    });

    {
        let (model, status) = (model.clone(), status.clone());
        tree.selection_changed().connect(move |node| {
            status.set(format!("Selected {}", name_of(&model, *node, "(nothing)")));
        });
    }
    {
        let (model, status) = (model.clone(), status.clone());
        tree.activated().connect(move |node| {
            status.set(format!(
                "Activated {} — double click or press Enter",
                name_of(&model, Some(*node), "?")
            ));
        });
    }

    // Expansion is model state, so the buttons just rewrite the model and ask for a frame.
    let expand = Button::new("Expand all");
    expand.clicked().connect({
        let status = status.clone();
        let model = model.clone();
        move |_| {
            let changed = model.with_mut(|m| m.set_expanded_all(true));
            notify::request_redraw();
            status.set(format!("Expanded {changed} nodes"));
        }
    });
    let collapse = Button::new("Collapse all");
    collapse.clicked().connect({
        let status = status.clone();
        let model = model.clone();
        move |_| {
            let changed = model.with_mut(|m| m.set_expanded_all(false));
            notify::request_redraw();
            status.set(format!("Collapsed {changed} nodes"));
        }
    });

    // New nodes land under whatever is selected right now.
    let add = Button::new("Add node").variant(ButtonVariant::Primary);
    add.clicked().connect({
        let status = status.clone();
        let model = model.clone();
        let selection = selection.clone();
        move |_| {
            let parent = selection.get();
            let label = format!("Node {}", model.with(|m| m.node_count()) + 1);
            model.with_mut(|m| {
                let _id = m.append(parent, &label);
                if let Some(id) = parent {
                    m.set_expanded(&id, true);
                }
            });
            notify::request_redraw();
            status.set(format!(
                "Appended {label} under {}",
                name_of(&model, parent, "the root")
            ));
        }
    });

    let wm = app.wm();

    let tree = wm.insert(Box::new(tree));

    let mut tools = HBox::new().with_spacing(12.0);
    tools.push(wm.insert(Box::new(expand)));
    tools.push(wm.insert(Box::new(collapse)));
    tools.push(wm.insert(Box::new(add)));
    let tools = wm.insert(Box::new(tools));

    let mut page = VBox::new()
        .with_spacing(16.0)
        .with_cross_align(CrossAxisAlign::Stretch);
    page.push(wm.insert(Box::new(section("A TreeListModel behind a TreeView"))));
    page.push(wm.insert(Box::new(picked_line)));
    page.push(tools);
    page.push(tree);
    // `SizeHint::fill()` already expands the tree; the weight documents the intent.
    page.layout_mut().set_flex_of(tree, 1.0);
    let page = wm.insert(Box::new(page));

    frame(
        app,
        window,
        "TreeView",
        "Selection and activation over nested data; expansion lives in the model, not the widget.",
        page,
        &status,
    );
}
