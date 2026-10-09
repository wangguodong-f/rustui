//! # rustui-model
//!
//! The Model/View data layer of RustUI (design document §27, §28, §29).
//!
//! This crate is the vocabulary between application data and widgets. It defines
//!
//! * the three model shapes — [`ListModel`], [`TableModel`] and [`TreeModel`] — plus the
//!   [`ModelValue`] cell type they exchange with views,
//! * concrete implementations that applications can actually use: [`StringListModel`],
//!   [`TableListModel`] and [`TreeListModel`],
//! * the [`SelectionModel`] shared by every item view,
//! * value ordering and [`SortKey`]s for tabular sorting.
//!
//! Every model publishes a [`rustui_core::Signal`] describing what changed, and every model is
//! *total*: asking for a row, column or node that does not exist returns
//! [`ModelValue::None`]/`None` instead of panicking, which is what lets a virtualized view refresh
//! lazily without defensive code at every call site.
//!
//! ## Example
//!
//! ```
//! use rustui_model::prelude::*;
//!
//! let mut table = TableListModel::from_headers(["Package", "Version"]);
//! table.push_row(["fmt", "10.1.0"]);
//! table.push_row(["boost", "1.84"]);
//! table.push_row(["openssl", "3.2"]);
//!
//! // Numeric column sorts numbers, not characters.
//! table.sort_by_key(SortKey::number(1), true);
//! assert_eq!(table.data(0, 0).as_text(), "boost");
//!
//! let mut selection = SelectionModel::new(SelectionMode::Multiple);
//! selection.select_range(0, 2);
//! assert_eq!(selection.selected(), vec![0, 1, 2]);
//! selection.toggle(1);
//! assert_eq!(selection.selected(), vec![0, 2]);
//! ```
//!
//! ## Ownership
//!
//! Models are plain `Send`-less values built on `Rc`/`Signal`, because RustUI is a single-threaded
//! UI framework (design document §37): a model is owned by the application and handed to a view by
//! reference or boxed into a `Box<dyn TableModel>`. Signals must be connected *before* the data is
//! loaded for a view to see the initial load.

pub mod list;
pub mod selection;
pub mod shared;
pub mod sort;
pub mod table;
pub mod traits;
pub mod tree;

pub use list::{ListChange, StringListModel};
pub use selection::{SelectionChange, SelectionMode, SelectionModel};
pub use shared::SharedModel;
pub use sort::{boolean_value, compare_text, compare_values, numeric_value, SortKey};
pub use table::{TableChange, TableListModel};
pub use traits::{ListModel, ModelValue, NodeId, TableModel, TreeModel};
pub use tree::{TreeBuilder, TreeChange, TreeListModel, TreeNode};

/// Re-exported core primitives that appear in this crate's API.
pub use rustui_core::{Connection, Shared, SharedProperty, Signal};

/// Everything an application typically needs to talk to item views.
pub mod prelude {
    pub use crate::list::{ListChange, StringListModel};
    pub use crate::selection::{SelectionChange, SelectionMode, SelectionModel};
    pub use crate::shared::SharedModel;
    pub use crate::sort::{compare_values, SortKey};
    pub use crate::table::{TableChange, TableListModel};
    pub use crate::traits::{ListModel, ModelValue, NodeId, TableModel, TreeModel};
    pub use crate::tree::{TreeBuilder, TreeChange, TreeListModel, TreeNode};
    pub use rustui_core::{Connection, SharedProperty, Signal};
}

#[cfg(test)]
mod tests {
    //! Integration level checks: the layers must fit together, not just work in isolation.

    use super::prelude::*;

    #[test]
    fn a_string_list_can_drive_a_table_view() {
        let mut model = StringListModel::from_values(["carrot", "apple", "banana"]);
        // A `StringListModel` is both a list and a one column table.
        {
            let as_table: &dyn TableModel = &model;
            assert_eq!(as_table.column_count(), 1);
            assert_eq!(as_table.row_count(), 3);
            assert_eq!(as_table.data(0, 0).as_text(), "carrot");
        }

        model.set_filter("AN");
        let as_table: &dyn TableModel = &model;
        assert_eq!(as_table.row_count(), 1, "only 'banana' contains 'an'");
        assert_eq!(as_table.data(0, 0).as_text(), "banana");
        assert_eq!(as_table.data(1, 0), ModelValue::None);
    }

    #[test]
    fn list_models_can_be_boxed_as_traits() {
        let models: Vec<Box<dyn ListModel>> = vec![
            Box::new(StringListModel::from_values(["a", "b"])),
            Box::new(StaticList(vec![1, 2, 3])),
        ];
        assert_eq!(models[0].row_count(), 2);
        assert_eq!(models[1].row_count(), 3);
        assert_eq!(models[1].data(2), ModelValue::Number(3.0));
        assert_eq!(models[1].data(3), ModelValue::None);
    }

    /// A hand written model, the way an application would write one.
    struct StaticList(Vec<i64>);

    impl ListModel for StaticList {
        fn row_count(&self) -> usize {
            self.0.len()
        }

        fn data(&self, row: usize) -> ModelValue {
            match self.0.get(row) {
                Some(v) => ModelValue::from(*v),
                None => ModelValue::None,
            }
        }
    }

    #[test]
    fn table_models_can_be_boxed_as_traits() {
        let mut table = TableListModel::from_headers(["a", "b"]);
        table.push_row(["1", "2"]);
        let as_table: &dyn TableModel = &table;
        assert_eq!(as_table.header_data(0).as_text(), "a");
        assert_eq!(as_table.data(0, 1).as_text(), "2");
        assert_eq!(as_table.data(7, 7), ModelValue::None);
    }

    #[test]
    fn selection_reflects_table_removals() {
        let mut table = TableListModel::from_headers(["a"]);
        table.push_row(["1"]);
        table.push_row(["2"]);
        table.push_row(["3"]);

        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.select(0);
        selection.select(2);
        assert_eq!(selection.count(), 2);

        table.remove_row(0);
        assert_eq!(selection.remove_rows_from(0), 2);
        assert!(selection.is_empty());
        assert_eq!(selection.current(), None);
    }

    #[test]
    fn tree_views_walk_roots_and_expansion() {
        let mut builder = TreeBuilder::new();
        let root = builder.root("root");
        let a = builder.child(root, "a");
        builder.child(a, "a1");
        let b = builder.child(root, "b");
        let model = builder.build();

        assert_eq!(model.root_count(), 1);
        assert_eq!(model.child_count(&root), 2);
        assert_eq!(model.parent(&a), Some(root));
        assert_eq!(model.parent(&b), Some(root));
        assert_eq!(model.data(&a).as_text(), "a");
        assert_eq!(model.depth(&a), Some(1));
    }

    #[test]
    fn sorting_a_table_with_a_custom_key() {
        let mut table = TableListModel::from_headers(["name", "size"]);
        table.push_row(["bbb", "10"]);
        table.push_row(["a", "300"]);
        table.push_row(["cc", "20"]);

        let by_length = SortKey::custom(0, |a, b| {
            a.as_text()
                .len()
                .cmp(&b.as_text().len())
                .then_with(|| a.as_text().cmp(&b.as_text()))
        });
        table.sort_by_key(by_length, true);
        assert_eq!(table.cell(0, 0), "a");
        assert_eq!(table.cell(1, 0), "cc");
        assert_eq!(table.cell(2, 0), "bbb");
    }

    #[test]
    fn change_signals_are_observable_through_the_crate_root_reexports() {
        let mut list = StringListModel::new();
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = seen.clone();
        list.changed()
            .connect(move |change| sink.borrow_mut().push(*change));
        list.push("one");
        list.insert(0, "zero");
        list.remove(0);
        list.clear();
        assert_eq!(
            *seen.borrow(),
            vec![
                ListChange::Inserted { index: 0 },
                ListChange::Inserted { index: 0 },
                ListChange::Removed { index: 0 },
                ListChange::Cleared,
            ]
        );

        let mut tree = TreeListModel::new();
        let tree_seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let tree_sink = tree_seen.clone();
        tree.changed()
            .connect(move |change| tree_sink.borrow_mut().push(*change));
        let root = tree.append(None, "root");
        tree.set_expanded(&root, true);
        tree.remove(&root);
        assert_eq!(
            *tree_seen.borrow(),
            vec![
                TreeChange::Inserted {
                    node: root,
                    parent: None
                },
                TreeChange::Expanded {
                    node: root,
                    expanded: true
                },
                TreeChange::Removed { node: root },
            ]
        );
    }

    #[test]
    fn signals_and_properties_of_the_crate_root_reexport() {
        let mut selection = SelectionModel::default();
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = log.clone();
        let _: &Signal<SelectionChange> = selection.selection_changed();
        let _: &SharedProperty<Option<usize>> = selection.current_property();
        selection
            .selection_changed()
            .connect(move |c| sink.borrow_mut().push(c.clone()));
        selection.select(4);
        assert_eq!(log.borrow().len(), 1);
        assert_eq!(log.borrow()[0].current, Some(4));
    }

    #[test]
    fn prelude_exposes_the_whole_layer() {
        let mut list = StringListModel::new();
        list.push("x");
        let _: ModelValue = ListModel::data(&list, 0);
        let _: NodeId = NodeId::new(0);
        let _: TableListModel = TableListModel::new();
        let _: TreeListModel = TreeListModel::new();
        let _: SelectionModel = SelectionModel::new(SelectionMode::Single);
        let _: SelectionChange = SelectionChange::default();
        let _: ListChange = ListChange::Cleared;
        let _: TableChange = TableChange::Cleared;
        let _: TreeChange = TreeChange::Cleared;
        let _ = SortKey::text(0);
        let _ = compare_values(&ModelValue::None, &ModelValue::None);
    }
}
