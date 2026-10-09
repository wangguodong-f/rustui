//! A concrete, observable table of strings (design document §28).
//!
//! [`TableListModel`] stores a header row plus `Vec<Vec<String>>` cells and implements
//! [`TableModel`]. It is deliberately tolerant about ragged data: rows that are shorter than the
//! header are padded with empty cells and longer rows are truncated on insertion, so that a view
//! can index `0 .. column_count()` without checking every row.
//!
//! Sorting lives here as well ([`TableListModel::sort_by_column`], [`TableListModel::sort_by_key`]):
//! the model owns its rows, so reordering them and reporting a [`TableChange::Sorted`] is a
//! two-liner for the application and a plain repaint for the view.

use std::fmt;

use rustui_core::{Shared, Signal};

use crate::sort::{compare_text, SortKey};
use crate::traits::{ModelValue, TableModel};

/// What happened to a [`TableListModel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableChange {
    /// A row was inserted at `row`; existing rows shifted down.
    Inserted {
        /// Index the row was inserted at.
        row: usize,
    },
    /// The row at `row` was removed; rows below shifted up.
    Removed {
        /// Index the row was removed from.
        row: usize,
    },
    /// A cell or a whole row of the given row changed.
    Updated {
        /// Index of the affected row.
        row: usize,
    },
    /// The rows were reordered.
    Sorted {
        /// Column the sort was based on.
        column: usize,
        /// `true` for ascending order.
        ascending: bool,
    },
    /// Every row was removed, or the header was replaced.
    Cleared,
}

/// A table of strings with change notification, sorting and ragged-row tolerance.
///
/// ```
/// use rustui_model::{TableListModel, TableModel};
///
/// let mut model = TableListModel::from_headers(["Package", "Version"]);
/// model.push_row(["fmt", "10.1.0"]);
/// model.push_row(["boost", "1.84"]);
/// model.push_row(["openssl", "3.2"]);
///
/// model.sort_by_column(1, true);
/// assert_eq!(model.data(0, 0).as_text(), "boost");
///
/// assert_eq!(model.column_count(), 2);
/// assert_eq!(model.header_data(1).as_text(), "Version");
/// assert_eq!(model.data(9, 9), rustui_model::ModelValue::None);
/// ```
pub struct TableListModel {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    changed: Signal<TableChange>,
}

impl Default for TableListModel {
    /// An empty table without columns.
    fn default() -> Self {
        TableListModel::new()
    }
}

impl fmt::Debug for TableListModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableListModel")
            .field("columns", &self.headers.len())
            .field("rows", &self.rows.len())
            .field("connections", &self.changed.connection_count())
            .finish()
    }
}

impl TableListModel {
    /// An empty table with no columns.
    pub fn new() -> Self {
        TableListModel {
            headers: Vec::new(),
            rows: Vec::new(),
            changed: Signal::new(),
        }
    }

    /// A table with the given header row and no data rows.
    pub fn from_headers<I, S>(headers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut model = TableListModel::new();
        model.headers = headers.into_iter().map(Into::into).collect();
        model
    }

    /// The signal describing every change; emits after the model was already mutated.
    pub fn changed(&self) -> &Signal<TableChange> {
        &self.changed
    }

    /// The header row.
    pub fn headers(&self) -> &[String] {
        &self.headers
    }

    /// The header cell of `column`, or `None` when out of range.
    pub fn header(&self, column: usize) -> Option<&str> {
        self.headers.get(column).map(String::as_str)
    }

    /// Replaces the header row; existing rows are padded or truncated to the new width.
    ///
    /// Emits [`TableChange::Cleared`], because the meaning of every cell index may have changed.
    pub fn set_headers<I, S>(&mut self, headers: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.headers = headers.into_iter().map(Into::into).collect();
        let width = self.headers.len();
        for row in &mut self.rows {
            row.resize(width, String::new());
        }
        self.changed.emit(&TableChange::Cleared);
    }

    /// Number of columns (the width of the header row).
    pub fn column_count(&self) -> usize {
        self.headers.len()
    }

    /// Number of data rows.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// `true` when the table has no data rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// All data rows, each of exactly [`TableListModel::column_count`] cells.
    pub fn rows(&self) -> &[Vec<String>] {
        &self.rows
    }

    /// The cells of `row`, or `None` when out of range.
    pub fn row(&self, row: usize) -> Option<&[String]> {
        self.rows.get(row).map(Vec::as_slice)
    }

    /// The cell at `(row, column)` as a string slice, or an empty string when out of range.
    pub fn cell(&self, row: usize, column: usize) -> &str {
        self.rows
            .get(row)
            .and_then(|r| r.get(column))
            .map(String::as_str)
            .unwrap_or("")
    }

    /// Appends a row; returns its index.
    pub fn push_row<I, S>(&mut self, cells: I) -> usize
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let index = self.rows.len();
        self.rows.push(self.normalize_row(cells));
        self.changed.emit(&TableChange::Inserted { row: index });
        index
    }

    /// Inserts a row at `row`, clamping `row` to the end of the table.
    ///
    /// Short rows are padded and long rows are truncated to the column count. If the table has no
    /// header yet, the inserted cells define the column count. Returns the effective index.
    pub fn insert_row<I, S>(&mut self, row: usize, cells: I) -> usize
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut cells: Vec<String> = cells.into_iter().map(Into::into).collect();
        if self.headers.is_empty() && !cells.is_empty() {
            self.headers = cells.iter().map(|_| String::new()).collect();
        }
        cells.resize(self.headers.len(), String::new());
        let index = row.min(self.rows.len());
        self.rows.insert(index, cells);
        self.changed.emit(&TableChange::Inserted { row: index });
        index
    }

    /// Removes the row at `row`; `false` when out of range.
    pub fn remove_row(&mut self, row: usize) -> bool {
        if row >= self.rows.len() {
            return false;
        }
        self.rows.remove(row);
        self.changed.emit(&TableChange::Removed { row });
        true
    }

    /// Replaces every cell of `row`; `false` when out of range.
    pub fn set_row<I, S>(&mut self, row: usize, cells: I) -> bool
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if row >= self.rows.len() {
            return false;
        }
        let normalized = self.normalize_row(cells);
        if let Some(existing) = self.rows.get_mut(row) {
            *existing = normalized;
        }
        self.changed.emit(&TableChange::Updated { row });
        true
    }

    /// Replaces a single cell; `false` when the row or column is out of range.
    ///
    /// Setting a cell to the value it already holds is still reported as
    /// [`TableChange::Updated`], because the caller explicitly asked for a write.
    pub fn set_data(&mut self, row: usize, column: usize, value: impl Into<String>) -> bool {
        if row >= self.rows.len() || column >= self.headers.len() {
            return false;
        }
        if let Some(cell) = self.rows.get_mut(row).and_then(|r| r.get_mut(column)) {
            *cell = value.into();
        }
        self.changed.emit(&TableChange::Updated { row });
        true
    }

    /// Removes every data row, keeping the header; emits only when something was removed.
    pub fn clear(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        self.rows.clear();
        self.changed.emit(&TableChange::Cleared);
    }

    /// Sorts by the text of `column`, case-insensitively, with a stable tie break.
    ///
    /// Returns `false` when `column` does not exist. Sorting an empty or single row table is
    /// valid and reports [`TableChange::Sorted`].
    pub fn sort_by_column(&mut self, column: usize, ascending: bool) -> bool {
        self.sort_by_key(SortKey::text(column), ascending)
    }

    /// Sorts by an arbitrary [`SortKey`].
    ///
    /// The sort is stable: rows whose key compares equal keep their relative order in ascending
    /// mode, and in descending mode ties keep their original order too (only the key is reversed).
    /// Returns `false` when `key` refers to a column that does not exist.
    pub fn sort_by_key(&mut self, key: SortKey, ascending: bool) -> bool {
        let column = key.column();
        if column >= self.headers.len() {
            return false;
        }
        let keys: Vec<ModelValue> = self
            .rows
            .iter()
            .map(|row| {
                let cell = row.get(column).map(String::as_str).unwrap_or("");
                ModelValue::Text(cell.to_string())
            })
            .collect();
        let mut order: Vec<usize> = (0..self.rows.len()).collect();
        order.sort_by(|a, b| {
            let ord = key.compare(&keys[*a], &keys[*b]);
            let ord = if ascending { ord } else { ord.reverse() };
            ord.then_with(|| a.cmp(b))
        });
        let mut rows = Vec::with_capacity(self.rows.len());
        for index in order {
            if let Some(row) = self.rows.get(index) {
                rows.push(row.clone());
            }
        }
        self.rows = rows;
        self.changed
            .emit(&TableChange::Sorted { column, ascending });
        true
    }

    /// Sorts a copy of the row indices instead of the table; useful to preview an order.
    pub fn sorted_row_indices(&self, key: SortKey, ascending: bool) -> Vec<usize> {
        let column = key.column();
        let keys: Vec<ModelValue> = self
            .rows
            .iter()
            .map(|row| ModelValue::Text(row.get(column).cloned().unwrap_or_default()))
            .collect();
        let mut order: Vec<usize> = (0..self.rows.len()).collect();
        order.sort_by(|a, b| {
            let ord = key.compare(&keys[*a], &keys[*b]);
            let ord = if ascending { ord } else { ord.reverse() };
            ord.then_with(|| a.cmp(b))
        });
        order
    }

    /// The order a [`SortKey::Text`] sort would produce, without touching the table.
    pub fn preview_sort_by_column(&self, column: usize, ascending: bool) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.rows.len()).collect();
        order.sort_by(|a, b| {
            let ord = compare_text(self.cell(*a, column), self.cell(*b, column));
            let ord = if ascending { ord } else { ord.reverse() };
            ord.then_with(|| a.cmp(b))
        });
        order
    }

    fn normalize_row<I, S>(&self, cells: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut cells: Vec<String> = cells.into_iter().map(Into::into).collect();
        cells.resize(self.headers.len(), String::new());
        cells
    }
}

impl TableModel for TableListModel {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn column_count(&self) -> usize {
        self.headers.len()
    }

    fn header_data(&self, column: usize) -> ModelValue {
        match self.headers.get(column) {
            Some(text) => ModelValue::Text(text.clone()),
            None => ModelValue::None,
        }
    }

    fn data(&self, row: usize, column: usize) -> ModelValue {
        if row >= self.rows.len() {
            return ModelValue::None;
        }
        match self.rows.get(row).and_then(|cells| cells.get(column)) {
            Some(text) => ModelValue::Text(text.clone()),
            None => ModelValue::None,
        }
    }
}

// ---- shared handle -------------------------------------------------------
//
// See `Shared<StringListModel>`: the view reads the boxed model, the application edits through
// its own handle to the same value.

impl TableModel for Shared<TableListModel> {
    fn row_count(&self) -> usize {
        self.with(|model| model.row_count())
    }

    fn column_count(&self) -> usize {
        self.with(|model| model.column_count())
    }

    fn header_data(&self, column: usize) -> ModelValue {
        self.with(|model| <TableListModel as TableModel>::header_data(model, column))
    }

    fn data(&self, row: usize, column: usize) -> ModelValue {
        self.with(|model| <TableListModel as TableModel>::data(model, row, column))
    }
}

impl<'a> IntoIterator for &'a TableListModel {
    type Item = &'a Vec<String>;
    type IntoIter = std::slice::Iter<'a, Vec<String>>;

    fn into_iter(self) -> Self::IntoIter {
        self.rows.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn a_shared_handle_reads_the_model_a_view_owns() {
        let model = Shared::new(TableListModel::from_headers(["name", "qty"]));
        model.with_mut(|model| {
            model.push_row(["wgpu", "1"]);
        });
        let owned: Box<dyn TableModel> = Box::new(model.clone());

        assert_eq!(owned.row_count(), 1);
        assert_eq!(owned.column_count(), 2);
        assert_eq!(owned.header_data(1).as_text(), "qty");
        assert_eq!(owned.data(0, 0).as_text(), "wgpu");

        model.with_mut(|model| {
            model.push_row(["winit", "2"]);
        });
        assert_eq!(owned.row_count(), 2);
    }

    fn recorder(model: &TableListModel) -> Rc<RefCell<Vec<TableChange>>> {
        let log = Rc::new(RefCell::new(Vec::new()));
        let sink = log.clone();
        model.changed().connect(move |c| sink.borrow_mut().push(*c));
        log
    }

    fn sample() -> TableListModel {
        let mut model = TableListModel::from_headers(["Package", "Version"]);
        model.push_row(["fmt", "10.1.0"]);
        model.push_row(["boost", "1.84"]);
        model.push_row(["openssl", "3.2"]);
        model
    }

    #[test]
    fn empty_table_answers_none() {
        let model = TableListModel::new();
        assert_eq!(model.column_count(), 0);
        assert_eq!(model.row_count(), 0);
        assert!(model.is_empty());
        assert!(TableModel::is_empty(&model));
        assert_eq!(model.header_data(0), ModelValue::None);
        assert_eq!(model.data(0, 0), ModelValue::None);
        assert_eq!(model.header(0), None);
        assert_eq!(model.row(0), None);
        assert_eq!(model.cell(0, 0), "");
        assert_eq!(model.headers(), &[] as &[String]);
    }

    #[test]
    fn headers_and_rows_are_readable() {
        let model = sample();
        assert_eq!(model.column_count(), 2);
        assert_eq!(model.row_count(), 3);
        assert!(!model.is_empty());
        assert_eq!(model.header(1), Some("Version"));
        assert_eq!(model.header_data(0).as_text(), "Package");
        assert_eq!(model.header_data(2), ModelValue::None);
        assert_eq!(model.data(0, 1).as_text(), "10.1.0");
        assert_eq!(model.cell(2, 0), "openssl");
        assert_eq!(
            model.row(1),
            Some(&["boost".to_string(), "1.84".to_string()][..])
        );
        assert_eq!(model.rows().len(), 3);
        let all: Vec<&[String]> = (&model).into_iter().map(|row| row.as_slice()).collect();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn out_of_range_access_is_none() {
        let model = sample();
        assert_eq!(model.data(3, 0), ModelValue::None);
        assert_eq!(model.data(0, 2), ModelValue::None);
        assert_eq!(model.data(usize::MAX, usize::MAX), ModelValue::None);
        assert_eq!(model.cell(99, 99), "");
        assert_eq!(model.cell(0, 99), "");
        assert!(!TableModel::contains(&model, 3, 0));
        assert_eq!(
            TableModel::cell(&model, 0, 0),
            Some(ModelValue::Text("fmt".into()))
        );
        assert_eq!(TableModel::cell(&model, 3, 3), None);
    }

    #[test]
    fn insert_row_clamps_and_emits() {
        let mut model = sample();
        let log = recorder(&model);
        assert_eq!(model.insert_row(0, ["abc", "1.0"]), 0);
        assert_eq!(model.data(0, 0).as_text(), "abc");
        assert_eq!(model.row_count(), 4);
        assert_eq!(model.insert_row(99, ["tail", "0.1"]), 4, "index is clamped");
        assert_eq!(
            model.row(4),
            Some(&["tail".to_string(), "0.1".to_string()][..])
        );
        assert_eq!(
            *log.borrow(),
            vec![
                TableChange::Inserted { row: 0 },
                TableChange::Inserted { row: 4 }
            ]
        );
    }

    #[test]
    fn short_and_long_rows_are_normalised() {
        let mut model = TableListModel::from_headers(["a", "b", "c"]);
        model.push_row(["only"]);
        assert_eq!(
            model.row(0),
            Some(&["only".to_string(), String::new(), String::new()][..])
        );
        assert_eq!(model.data(0, 2).as_text(), "");
        model.push_row(["1", "2", "3", "4"]);
        assert_eq!(model.row(1).map(<[String]>::len), Some(3), "truncated");
        assert_eq!(model.data(1, 3), ModelValue::None);
    }

    #[test]
    fn insert_defines_columns_when_there_are_no_headers() {
        let mut model = TableListModel::new();
        assert_eq!(model.insert_row(0, ["x", "y"]), 0);
        assert_eq!(model.column_count(), 2);
        assert_eq!(model.header(0), Some(""));
        assert_eq!(model.data(0, 1).as_text(), "y");
    }

    #[test]
    fn remove_row_shifts_and_reports() {
        let mut model = sample();
        let log = recorder(&model);
        assert!(model.remove_row(1));
        assert_eq!(model.row_count(), 2);
        assert_eq!(model.data(1, 0).as_text(), "openssl");
        assert!(!model.remove_row(9), "out of range");
        assert_eq!(*log.borrow(), vec![TableChange::Removed { row: 1 }]);
    }

    #[test]
    fn set_data_and_set_row() {
        let mut model = sample();
        let log = recorder(&model);
        assert!(model.set_data(0, 1, "11.0.0"));
        assert_eq!(model.data(0, 1).as_text(), "11.0.0");
        assert!(!model.set_data(9, 0, "x"), "row out of range");
        assert!(!model.set_data(0, 9, "x"), "column out of range");
        assert!(model.set_row(2, ["openssl"]));
        assert_eq!(model.cell(2, 1), "", "row is padded to the column count");
        assert!(!model.set_row(9, ["x"]));
        assert_eq!(
            *log.borrow(),
            vec![
                TableChange::Updated { row: 0 },
                TableChange::Updated { row: 2 },
            ]
        );
    }

    #[test]
    fn set_headers_resizes_rows() {
        let mut model = sample();
        let log = recorder(&model);
        model.set_headers(["Only"]);
        assert_eq!(model.column_count(), 1);
        assert_eq!(model.cell(0, 1), "", "extra cells are dropped");
        model.set_headers(["a", "b", "c", "d"]);
        assert_eq!(model.cell(0, 3), "", "missing cells are padded");
        assert_eq!(
            *log.borrow(),
            vec![TableChange::Cleared, TableChange::Cleared]
        );
    }

    #[test]
    fn clear_keeps_headers() {
        let mut model = sample();
        let log = recorder(&model);
        model.clear();
        assert!(model.is_empty());
        assert_eq!(model.column_count(), 2);
        model.clear();
        assert_eq!(*log.borrow(), vec![TableChange::Cleared]);
    }

    #[test]
    fn sort_by_column_is_case_insensitive_and_stable() {
        let mut model = TableListModel::from_headers(["name", "v"]);
        model.push_row(["beta", "2"]);
        model.push_row(["Alpha", "1"]);
        model.push_row(["alpha", "0"]);
        assert!(model.sort_by_column(0, true));
        assert_eq!(model.cell(0, 0), "Alpha");
        assert_eq!(model.cell(1, 0), "alpha", "stable on the case tie break");
        assert_eq!(model.cell(2, 0), "beta");

        model.sort_by_column(0, false);
        assert_eq!(model.cell(0, 0), "beta");
        assert_eq!(model.cell(1, 0), "alpha");
        assert_eq!(model.cell(2, 0), "Alpha");
        assert!(!model.sort_by_column(7, true), "unknown column");
    }

    #[test]
    fn sort_emits_a_table_change() {
        let mut model = sample();
        let log = recorder(&model);
        assert!(model.sort_by_column(1, true));
        assert_eq!(
            *log.borrow(),
            vec![TableChange::Sorted {
                column: 1,
                ascending: true
            }]
        );
    }

    #[test]
    fn sort_by_numeric_key_orders_versions_numerically() {
        let mut model = TableListModel::from_headers(["package", "version"]);
        model.push_row(["c", "10"]);
        model.push_row(["a", "9"]);
        model.push_row(["b", "100"]);
        assert!(model.sort_by_key(SortKey::number(1), true));
        assert_eq!(model.cell(0, 1), "9");
        assert_eq!(model.cell(1, 1), "10");
        assert_eq!(model.cell(2, 1), "100");
    }

    #[test]
    fn preview_sort_does_not_mutate() {
        let model = sample();
        let preview = model.preview_sort_by_column(1, true);
        assert_eq!(preview, vec![1, 0, 2], "text order: 1.84 < 10.1.0 < 3.2");
        assert_eq!(model.cell(0, 0), "fmt", "the table is untouched");
        assert_eq!(
            model.sorted_row_indices(SortKey::number(1), true),
            vec![1, 2, 0],
            "numeric order: 1.84 < 3.2 < 10.1.0"
        );
        assert_eq!(model.cell(0, 0), "fmt");
        assert_eq!(model.preview_sort_by_column(1, false), vec![2, 0, 1]);
    }

    #[test]
    fn sorting_an_empty_or_single_row_table_is_fine() {
        let mut model = TableListModel::from_headers(["a"]);
        assert!(model.sort_by_column(0, true));
        assert!(model.row_count() == 0);
        model.push_row(["x"]);
        assert!(model.sort_by_column(0, false));
        assert_eq!(model.row_count(), 1);
        assert_eq!(model.preview_sort_by_column(0, true), vec![0]);
    }

    #[test]
    fn debug_format_is_informative() {
        let text = format!("{:?}", sample());
        assert!(text.contains("TableListModel"));
        assert!(text.contains("columns"));
    }
}
