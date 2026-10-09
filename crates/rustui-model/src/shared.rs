//! Boxing a shared model handle for a view.
//!
//! A view stores its data as `Box<dyn ListModel>` / `Box<dyn TableModel>` / `Box<dyn TreeModel>`
//! and can therefore only *read* it. The application keeps a `Shared<ConcreteModel>` handle and
//! edits through it; [`SharedModel::boxed`] produces a second handle to the same value for the
//! view, which is what makes Model / View separation work without downcasting (§52).

use rustui_core::Shared;

use crate::list::StringListModel;
use crate::table::TableListModel;
use crate::traits::{ListModel, TableModel, TreeModel};
use crate::tree::TreeListModel;

/// Turns a shared, concrete model into the boxed handle a view is built from.
///
/// ```
/// use rustui_model::{Shared, SharedModel, StringListModel};
///
/// let model = Shared::new(StringListModel::from_values(["a", "b"]));
/// let view = model.boxed(); // `Shared<Box<dyn ListModel>>`, same value as `model`
///
/// model.with_mut(|model| {
///     model.push("c");
/// });
/// assert_eq!(view.with(|model| model.row_count()), 3);
/// ```
pub trait SharedModel {
    /// The object safe trait the view reads through.
    type Model: ?Sized + 'static;

    /// A second handle to the same value, shaped for `ListView::with_model` and friends.
    fn boxed(&self) -> Shared<Box<Self::Model>>;
}

impl SharedModel for Shared<StringListModel> {
    type Model = dyn ListModel;

    fn boxed(&self) -> Shared<Box<Self::Model>> {
        let boxed: Box<Self::Model> = Box::new(self.clone());
        Shared::new(boxed)
    }
}

impl SharedModel for Shared<TableListModel> {
    type Model = dyn TableModel;

    fn boxed(&self) -> Shared<Box<Self::Model>> {
        let boxed: Box<Self::Model> = Box::new(self.clone());
        Shared::new(boxed)
    }
}

impl SharedModel for Shared<TreeListModel> {
    type Model = dyn TreeModel;

    fn boxed(&self) -> Shared<Box<Self::Model>> {
        let boxed: Box<Self::Model> = Box::new(self.clone());
        Shared::new(boxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxed_is_a_second_handle_to_the_same_value() {
        let model = Shared::new(TableListModel::from_headers(["name"]));
        let view = model.boxed();
        assert_eq!(view.with(|model| model.row_count()), 0);

        model.with_mut(|model| {
            model.push_row(["row"]);
        });
        assert_eq!(view.with(|model| model.row_count()), 1);
        assert_eq!(view.with(|model| model.data(0, 0).as_text()), "row");
    }

    #[test]
    fn boxed_tree_handles_expansion_both_ways() {
        let model = Shared::new(TreeListModel::new());
        let view = model.boxed();
        let root = model.with_mut(|model| model.append(None, "root"));

        // The view expands a node through its own box...
        view.with_mut(|view| view.set_expanded(&root, true));
        // ...and the application reads that back.
        assert!(model.with(|model| model.is_expanded(&root)));
    }
}
