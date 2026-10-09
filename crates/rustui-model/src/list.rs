//! A concrete, observable list of strings (design document §27).
//!
//! [`StringListModel`] is the workhorse model behind a `ListView`: it owns `Vec<String>`, publishes
//! fine grained [`ListChange`] notifications and — because real applications almost always need
//! one — an optional case-insensitive substring [filter](StringListModel::set_filter).
//!
//! ## Rows versus view rows
//!
//! When a filter is active the model exposes **fewer rows than it stores**. [`ListModel::row_count`]
//! and [`ListModel::data`] speak in *view rows* (the filtered, visible list, with
//! `0 ..= row_count()`), while the mutating API ([`StringListModel::push`],
//! [`StringListModel::insert`], [`StringListModel::remove`], [`StringListModel::set`]) speaks in
//! *source rows* (the backing storage). [`StringListModel::source_row`] and
//! [`StringListModel::view_row`] translate between the two, which is how a selection stored in view
//! coordinates can be kept meaningful after the data changes.

use std::fmt;

use rustui_core::{Shared, Signal};

use crate::traits::{ListModel, ModelValue, TableModel};

/// What happened to a [`StringListModel`].
///
/// The indices refer to **source** rows. A filter change is reported as [`ListChange::Cleared`]
/// because it may invalidate every visible row at once and a view cannot patch its way out of that
/// more cheaply than a full repaint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListChange {
    /// `index` source rows were inserted before this position.
    Inserted {
        /// Index the rows were inserted at.
        index: usize,
    },
    /// The source row at `index` was removed.
    Removed {
        /// Index the row was removed from.
        index: usize,
    },
    /// The source row moved from `from` to `to`.
    Moved {
        /// Previous source index.
        from: usize,
        /// New source index.
        to: usize,
    },
    /// The source row at `index` now holds a different string.
    Replaced {
        /// Index of the changed row.
        index: usize,
    },
    /// The whole model became empty, or a filter changed.
    Cleared,
}

/// A list of strings with change notification and optional filtering.
///
/// ```
/// use rustui_model::{ListChange, ListModel, StringListModel};
/// use std::cell::RefCell;
/// use std::rc::Rc;
///
/// let mut model = StringListModel::from_values(["beta", "alpha", "gamma"]);
/// let log = Rc::new(RefCell::new(Vec::new()));
/// let sink = log.clone();
/// model.changed().connect(move |c| sink.borrow_mut().push(*c));
///
/// // Case-insensitive substring matching hides everything but "alpha".
/// model.set_filter("alp");
/// assert_eq!(model.row_count(), 1);
/// assert_eq!(model.data(0).as_text(), "alpha");
/// assert_eq!(model.source_row(0), Some(1), "view row 0 is source row 1");
/// assert_eq!(model.len(), 3, "storage is untouched");
///
/// assert_eq!(*log.borrow(), vec![ListChange::Cleared]);
/// ```
pub struct StringListModel {
    items: Vec<String>,
    header: String,
    filter: Option<String>,
    /// Source indices of the visible rows; empty when no filter is active.
    visible: Vec<usize>,
    changed: Signal<ListChange>,
}

impl Default for StringListModel {
    fn default() -> Self {
        StringListModel::new()
    }
}

impl fmt::Debug for StringListModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StringListModel")
            .field("items", &self.items)
            .field("filter", &self.filter)
            .field("visible_rows", &self.visible.len())
            .field("connections", &self.changed.connection_count())
            .finish()
    }
}

impl StringListModel {
    /// An empty model without a filter.
    pub fn new() -> Self {
        StringListModel {
            items: Vec::new(),
            header: "Item".to_string(),
            filter: None,
            visible: Vec::new(),
            changed: Signal::new(),
        }
    }

    /// A model pre-filled with `values`, preserving their order.
    pub fn from_values<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut model = StringListModel::new();
        model.items = values.into_iter().map(Into::into).collect();
        model.rebuild_visible();
        model
    }

    /// The signal describing every change; emits after the model was already mutated.
    pub fn changed(&self) -> &Signal<ListChange> {
        &self.changed
    }

    /// The header used by the single-column [`TableModel`] implementation.
    pub fn header(&self) -> &str {
        &self.header
    }

    /// Renames the header of the [`TableModel`] implementation.
    pub fn set_header(&mut self, header: impl Into<String>) {
        self.header = header.into();
    }

    /// Number of stored (unfiltered) rows.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// `true` when no row is stored, filter or not.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The stored row at `source_index`, or `None` when out of range.
    pub fn get(&self, source_index: usize) -> Option<&str> {
        self.items.get(source_index).map(String::as_str)
    }

    /// Iterates over all stored rows, in storage order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.items.iter().map(String::as_str)
    }

    /// Iterates over the visible rows, in view order (identical to [`StringListModel::iter`] while
    /// no filter is active).
    pub fn iter_visible(&self) -> impl Iterator<Item = &str> {
        self.visible_rows()
            .iter()
            .map(move |i| self.items[*i].as_str())
    }

    /// The visible row indices as source indices.
    ///
    /// While no filter is active this is simply `0 .. len()`.
    pub fn visible_rows(&self) -> &[usize] {
        &self.visible
    }

    /// Number of visible rows; equal to [`ListModel::row_count`].
    pub fn view_len(&self) -> usize {
        self.visible.len()
    }

    /// Maps a view row to its source index, or `None` when out of range / filtered away.
    pub fn source_row(&self, view_row: usize) -> Option<usize> {
        self.visible.get(view_row).copied()
    }

    /// Maps a source index to its view row, or `None` when out of range / filtered away.
    pub fn view_row(&self, source_index: usize) -> Option<usize> {
        if source_index >= self.items.len() {
            return None;
        }
        self.visible.iter().position(|i| *i == source_index)
    }

    /// The active filter, or `None` when everything is shown.
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    /// Installs a case-insensitive substring filter.
    ///
    /// An empty string removes the filter; an unchanged filter is a no-op. Any change to the filter
    /// is reported as [`ListChange::Cleared`] because the visible row indices are renumbered.
    ///
    /// The view-row mapping is rebuilt eagerly, so every mutation costs `O(n)` while a filter is
    /// active — fine for interactive lists, and the reason [`StringListModel::source_row`] and
    /// [`StringListModel::view_row`] never have to scan.
    pub fn set_filter(&mut self, filter: impl Into<String>) {
        let filter = filter.into();
        let filter = if filter.is_empty() {
            None
        } else {
            Some(filter)
        };
        if filter == self.filter {
            return;
        }
        self.filter = filter;
        self.rebuild_visible();
        self.changed.emit(&ListChange::Cleared);
    }

    /// Removes the filter, showing every stored row again.
    pub fn clear_filter(&mut self) {
        self.set_filter(String::new());
    }

    /// `true` when `source_index` passes the active filter.
    pub fn matches_filter(&self, source_index: usize) -> bool {
        match &self.filter {
            None => true,
            Some(filter) => self
                .items
                .get(source_index)
                .map(|s| s.to_lowercase().contains(&filter.to_lowercase()))
                .unwrap_or(false),
        }
    }

    /// Appends a row; returns its source index.
    pub fn push(&mut self, value: impl Into<String>) -> usize {
        let index = self.items.len();
        self.items.push(value.into());
        self.finish_insert(index);
        index
    }

    /// Inserts a row at `index`, clamping `index` to the end of the list.
    ///
    /// Returns the effective source index.
    pub fn insert(&mut self, index: usize, value: impl Into<String>) -> usize {
        let index = index.min(self.items.len());
        self.items.insert(index, value.into());
        self.finish_insert(index);
        index
    }

    /// Removes the row at `source_index`, returning it, or `None` when out of range.
    pub fn remove(&mut self, source_index: usize) -> Option<String> {
        if source_index >= self.items.len() {
            return None;
        }
        let removed = self.items.remove(source_index);
        self.rebuild_visible();
        self.changed.emit(&ListChange::Removed {
            index: source_index,
        });
        Some(removed)
    }

    /// Moves the row at `from` to `to`; `false` when `from` is out of range.
    pub fn move_row(&mut self, from: usize, to: usize) -> bool {
        if from >= self.items.len() {
            return false;
        }
        let to = to.min(self.items.len() - 1);
        if from == to {
            return true;
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        self.rebuild_visible();
        self.changed.emit(&ListChange::Moved { from, to });
        true
    }

    /// Replaces the row at `source_index`; `false` when out of range or when the value is equal to
    /// the current one (an unchanged row does not notify observers).
    pub fn set(&mut self, source_index: usize, value: impl Into<String>) -> bool {
        let value = value.into();
        match self.items.get(source_index) {
            Some(current) if *current == value => return false,
            Some(_) => self.items[source_index] = value,
            None => return false,
        }
        self.rebuild_visible();
        self.changed.emit(&ListChange::Replaced {
            index: source_index,
        });
        true
    }

    /// Removes every row; emits [`ListChange::Cleared`] only when something was removed.
    pub fn clear(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.items.clear();
        self.visible.clear();
        self.changed.emit(&ListChange::Cleared);
    }

    fn finish_insert(&mut self, index: usize) {
        self.rebuild_visible();
        self.changed.emit(&ListChange::Inserted { index });
    }

    fn rebuild_visible(&mut self) {
        match self.filter.as_deref() {
            None => self.visible = (0..self.items.len()).collect(),
            Some(filter) => {
                let needle = filter.to_lowercase();
                self.visible = self
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.to_lowercase().contains(&needle))
                    .map(|(i, _)| i)
                    .collect();
            }
        }
    }
}

impl ListModel for StringListModel {
    fn row_count(&self) -> usize {
        self.view_len()
    }

    fn data(&self, row: usize) -> ModelValue {
        match self.source_row(row) {
            Some(source) => match self.items.get(source) {
                Some(item) => ModelValue::Text(item.clone()),
                None => ModelValue::None,
            },
            None => ModelValue::None,
        }
    }
}

impl TableModel for StringListModel {
    fn row_count(&self) -> usize {
        self.view_len()
    }

    fn column_count(&self) -> usize {
        1
    }

    fn header_data(&self, column: usize) -> ModelValue {
        if column == 0 && !self.header.is_empty() {
            ModelValue::Text(self.header.clone())
        } else {
            ModelValue::None
        }
    }

    fn data(&self, row: usize, column: usize) -> ModelValue {
        if column != 0 {
            return ModelValue::None;
        }
        ListModel::data(self, row)
    }
}

// ---- shared handle -------------------------------------------------------
//
// A view owns its model as `Box<dyn ListModel>`, which it can only read. The application keeps a
// second handle to the *same* model so that it can edit it: clone the `Shared` before boxing it.

impl ListModel for Shared<StringListModel> {
    fn row_count(&self) -> usize {
        self.with(|model| model.view_len())
    }

    fn data(&self, row: usize) -> ModelValue {
        self.with(|model| <StringListModel as ListModel>::data(model, row))
    }
}

impl TableModel for Shared<StringListModel> {
    fn row_count(&self) -> usize {
        self.with(|model| model.view_len())
    }

    fn column_count(&self) -> usize {
        1
    }

    fn header_data(&self, column: usize) -> ModelValue {
        self.with(|model| <StringListModel as TableModel>::header_data(model, column))
    }

    fn data(&self, row: usize, column: usize) -> ModelValue {
        self.with(|model| <StringListModel as TableModel>::data(model, row, column))
    }
}

impl<'a> IntoIterator for &'a StringListModel {
    type Item = &'a str;
    type IntoIter = Box<dyn Iterator<Item = &'a str> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

impl FromIterator<String> for StringListModel {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        StringListModel::from_values(iter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn a_shared_handle_reads_the_model_a_view_owns() {
        let model = Shared::new(StringListModel::from_values(["alpha", "beta"]));
        let owned: Box<dyn ListModel> = Box::new(model.clone());
        assert_eq!(owned.row_count(), 2);
        assert_eq!(owned.data(1).as_text(), "beta");

        // The application edits through its own handle; the view sees it on the next frame.
        model.with_mut(|model| {
            model.push("gamma");
        });
        assert_eq!(owned.row_count(), 3);
        assert_eq!(owned.data(2).as_text(), "gamma");
    }

    fn recorder(model: &StringListModel) -> Rc<RefCell<Vec<ListChange>>> {
        let log = Rc::new(RefCell::new(Vec::new()));
        let sink = log.clone();
        model.changed().connect(move |c| sink.borrow_mut().push(*c));
        log
    }

    fn list(values: &[&str]) -> StringListModel {
        StringListModel::from_values(values.iter().copied())
    }

    #[test]
    fn empty_model() {
        let model = StringListModel::new();
        assert_eq!(model.len(), 0);
        assert!(model.is_empty());
        assert!(ListModel::is_empty(&model));
        assert_eq!(ListModel::row_count(&model), 0);
        assert_eq!(ListModel::data(&model, 0), ModelValue::None);
        assert_eq!(model.get(0), None);
        assert_eq!(model.filter(), None);
        assert!(model.iter().next().is_none());
        assert_eq!(model.iter().count(), 0);
    }

    #[test]
    fn from_values_and_iter() {
        let model = list(&["a", "b", "c"]);
        assert_eq!(model.len(), 3);
        assert_eq!(ListModel::row_count(&model), 3);
        assert_eq!(ListModel::data(&model, 1).as_text(), "b");
        assert_eq!(model.get(2), Some("c"));
        assert_eq!(model.iter().collect::<Vec<_>>(), vec!["a", "b", "c"]);
        let collected: Vec<&str> = (&model).into_iter().collect();
        assert_eq!(collected, vec!["a", "b", "c"]);
        let collected: StringListModel = ["x", "y"].into_iter().map(String::from).collect();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn out_of_bounds_rows_return_none() {
        let mut model = list(&["a"]);
        assert_eq!(ListModel::data(&model, 1), ModelValue::None);
        assert_eq!(ListModel::data(&model, usize::MAX), ModelValue::None);
        assert_eq!(model.get(5), None);
        assert_eq!(model.source_row(5), None);
        assert_eq!(model.view_row(5), None);
        assert!(!ListModel::contains(&model, 1));
        assert_eq!(model.remove(9), None);
        assert!(!model.set(9, "z"));
        assert!(!model.move_row(9, 0));
    }

    #[test]
    fn push_insert_and_remove() {
        let mut model = StringListModel::new();
        let log = recorder(&model);
        assert_eq!(model.push("a"), 0);
        assert_eq!(model.insert(0, "z"), 0);
        assert_eq!(model.push("b"), 2);
        assert_eq!(model.len(), 3);
        assert_eq!(model.get(0), Some("z"));
        assert_eq!(model.insert(99, "tail"), 3, "index is clamped");
        assert_eq!(model.remove(1), Some("a".to_string()));
        assert_eq!(model.get(1), Some("b"));
        assert_eq!(
            *log.borrow(),
            vec![
                ListChange::Inserted { index: 0 },
                ListChange::Inserted { index: 0 },
                ListChange::Inserted { index: 2 },
                ListChange::Inserted { index: 3 },
                ListChange::Removed { index: 1 },
            ]
        );
    }

    #[test]
    fn set_reports_only_real_changes() {
        let mut model = list(&["a", "b"]);
        let log = recorder(&model);
        assert!(model.set(1, "c"));
        assert!(!model.set(1, "c"), "identical value does not notify");
        assert_eq!(*log.borrow(), vec![ListChange::Replaced { index: 1 }]);
        assert_eq!(ListModel::data(&model, 1).as_text(), "c");
    }

    #[test]
    fn clear_only_notifies_when_non_empty() {
        let mut model = list(&["a"]);
        let log = recorder(&model);
        model.clear();
        assert!(model.is_empty());
        model.clear();
        assert_eq!(*log.borrow(), vec![ListChange::Cleared]);
    }

    #[test]
    fn move_row_reorders_and_notifies() {
        let mut model = list(&["a", "b", "c"]);
        let log = recorder(&model);
        assert!(model.move_row(0, 2));
        assert_eq!(model.iter().collect::<Vec<_>>(), vec!["b", "c", "a"]);
        assert_eq!(*log.borrow(), vec![ListChange::Moved { from: 0, to: 2 }]);
        assert!(model.move_row(2, 0), "moving a row is always allowed");
        assert_eq!(model.iter().collect::<Vec<_>>(), vec!["a", "b", "c"]);
        assert_eq!(log.borrow().len(), 2);
        assert!(model.move_row(1, 1), "same position is a no-op");
        assert_eq!(log.borrow().len(), 2);
        assert!(model.move_row(0, 99), "target index is clamped");
        assert_eq!(model.iter().collect::<Vec<_>>(), vec!["b", "c", "a"]);
    }

    #[test]
    fn filter_is_case_insensitive_substring_matching() {
        let mut model = list(&["Alpha", "beta", "ALPHABET", "gamma"]);
        model.set_filter("alpha");
        assert_eq!(model.filter(), Some("alpha"));
        assert_eq!(ListModel::row_count(&model), 2);
        assert_eq!(ListModel::data(&model, 0).as_text(), "Alpha");
        assert_eq!(ListModel::data(&model, 1).as_text(), "ALPHABET");
        assert_eq!(ListModel::data(&model, 2), ModelValue::None);
        assert_eq!(model.len(), 4, "storage is untouched");
        assert_eq!(
            model.iter_visible().collect::<Vec<_>>(),
            vec!["Alpha", "ALPHABET"]
        );
    }

    #[test]
    fn filter_can_be_cleared_and_changed() {
        let mut model = list(&["a", "b", "c"]);
        let log = recorder(&model);
        model.set_filter("b");
        assert_eq!(ListModel::row_count(&model), 1);
        model.set_filter("b");
        assert_eq!(log.borrow().len(), 1, "identical filter is a no-op");
        model.clear_filter();
        assert_eq!(ListModel::row_count(&model), 3);
        model.set_filter("");
        assert_eq!(model.filter(), None);
        assert_eq!(log.borrow().len(), 2);
    }

    #[test]
    fn filter_mapping_translates_both_directions() {
        let mut model = list(&["aa", "ba", "ca"]);
        model.set_filter("a");
        assert_eq!(model.visible_rows(), &[0, 1, 2]);
        assert_eq!(model.source_row(1), Some(1));
        assert_eq!(model.view_row(2), Some(2));

        model.set_filter("b");
        assert_eq!(model.visible_rows(), &[1]);
        assert_eq!(model.source_row(0), Some(1));
        assert_eq!(model.view_row(1), Some(0));
        assert_eq!(model.view_row(0), None, "row 0 is filtered away");
        assert_eq!(model.view_row(9), None);
    }

    #[test]
    fn mutations_keep_the_filter_mapping_in_sync() {
        let mut model = list(&["alpha", "beta"]);
        model.set_filter("al");
        assert_eq!(ListModel::row_count(&model), 1);
        model.push("gamma");
        assert_eq!(
            ListModel::row_count(&model),
            1,
            "'gamma' does not match 'al'"
        );
        assert_eq!(ListModel::data(&model, 0).as_text(), "alpha");

        model.insert(0, "alphabet");
        assert_eq!(ListModel::row_count(&model), 2);
        assert_eq!(ListModel::data(&model, 0).as_text(), "alphabet");
        assert_eq!(ListModel::data(&model, 1).as_text(), "alpha");

        assert_eq!(model.remove(0), Some("alphabet".to_string()));
        assert_eq!(ListModel::row_count(&model), 1);
        assert_eq!(ListModel::data(&model, 0).as_text(), "alpha");

        model.set(0, "delta");
        assert_eq!(ListModel::row_count(&model), 0, "'delta' no longer matches");
    }

    #[test]
    fn matches_filter_reports_per_row_result() {
        let mut model = list(&["alpha", "beta"]);
        assert!(model.matches_filter(0));
        model.set_filter("bet");
        assert!(!model.matches_filter(0));
        assert!(model.matches_filter(1));
        assert!(!model.matches_filter(42));
    }

    #[test]
    fn table_view_of_the_single_column_model() {
        let mut model = list(&["a", "b"]);
        assert_eq!(model.column_count(), 1);
        assert_eq!(model.header_data(0), ModelValue::Text("Item".into()));
        assert_eq!(model.header_data(1), ModelValue::None);
        assert_eq!(TableModel::data(&model, 0, 0).as_text(), "a");
        assert_eq!(TableModel::data(&model, 0, 1), ModelValue::None);
        assert_eq!(
            TableModel::cell(&model, 1, 0),
            Some(ModelValue::Text("b".into()))
        );
        assert_eq!(TableModel::cell(&model, 1, 1), None);
        assert_eq!(TableModel::row_count(&model), 2);
        model.set_header("");
        assert_eq!(model.header_data(0), ModelValue::None);
        model.set_header("Names");
        assert_eq!(model.header(), "Names");
        assert_eq!(model.header_data(0).as_text(), "Names");
    }

    #[test]
    fn header_can_be_hidden_by_emptying_it() {
        let mut model = list(&["a"]);
        model.set_header("");
        assert_eq!(model.header_data(0), ModelValue::None);
        assert!(!model.is_empty());
    }

    #[test]
    fn debug_format_is_informative() {
        let model = list(&["a"]);
        let text = format!("{:?}", model);
        assert!(text.contains("StringListModel"));
        assert!(text.contains("items"));
    }
}
