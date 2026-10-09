//! Selection state shared by list, table and tree views.
//!
//! A [`SelectionModel`] is deliberately *model agnostic*: it stores row indices and knows nothing
//! about where those indices come from, so the same instance can be paired with a
//! [`crate::StringListModel`], a [`crate::TableListModel`] or a view-local virtualized list.
//!
//! Two aspects of a selection are published separately:
//!
//! * [`SelectionModel::selection_changed`] carries a [`SelectionChange`] describing *what* changed
//!   (added / removed rows plus the new current row) — this is what a view uses to repaint only
//!   the affected rows;
//! * [`SelectionModel::current_property`] is a [`SharedProperty<Option<usize>>`] holding the
//!   current row, so that a cursor, an inspector panel or an undo stack can simply bind to it.

use rustui_core::{SharedProperty, Signal};

/// How many rows may be selected at the same time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SelectionMode {
    /// At most one row; selecting a new row replaces the previous selection.
    #[default]
    Single,
    /// Any number of rows.
    Multiple,
}

/// The payload of [`SelectionModel::selection_changed`].
///
/// Both vectors are sorted ascending and contain no duplicates, which lets a view apply the change
/// without extra bookkeeping. `current` is the row that should be drawn as "the cursor", whether or
/// not it is part of `added`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionChange {
    /// Rows that became selected.
    pub added: Vec<usize>,
    /// Rows that are no longer selected.
    pub removed: Vec<usize>,
    /// The current row after the change.
    pub current: Option<usize>,
}

impl SelectionChange {
    /// `true` when nothing at all changed; such a change is never emitted.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }

    /// `true` when `index` became selected.
    pub fn added_row(&self, index: usize) -> bool {
        self.added.contains(&index)
    }

    /// `true` when `index` is no longer selected.
    pub fn removed_row(&self, index: usize) -> bool {
        self.removed.contains(&index)
    }
}

/// The set of selected rows plus the current (focused) row.
///
/// ```
/// use rustui_model::{SelectionMode, SelectionModel};
///
/// let mut selection = SelectionModel::new(SelectionMode::Multiple);
/// selection.select(1);
/// selection.select(3);
/// selection.select_range(5, 7);
///
/// assert_eq!(selection.selected(), vec![1, 3, 5, 6, 7]);
/// assert_eq!(selection.current(), Some(7));
/// assert!(selection.is_selected(6));
/// assert!(!selection.is_selected(2));
///
/// selection.toggle(6);
/// assert_eq!(selection.selected(), vec![1, 3, 5, 7]);
/// ```
pub struct SelectionModel {
    mode: SelectionMode,
    selected: Vec<usize>,
    current: SharedProperty<Option<usize>>,
    selection_changed: Signal<SelectionChange>,
}

impl Default for SelectionModel {
    /// A single-selection model, the most common default for lists and trees.
    fn default() -> Self {
        SelectionModel::new(SelectionMode::Single)
    }
}

impl SelectionModel {
    /// Creates an empty model in `mode`.
    pub fn new(mode: SelectionMode) -> Self {
        SelectionModel {
            mode,
            selected: Vec::new(),
            current: SharedProperty::new(None),
            selection_changed: Signal::new(),
        }
    }

    /// The current selection mode.
    pub fn mode(&self) -> SelectionMode {
        self.mode
    }

    /// Switches the mode, collapsing or keeping the existing selection.
    ///
    /// Switching to [`SelectionMode::Single`] keeps the current row (or the first selected row)
    /// and drops everything else; switching to [`SelectionMode::Multiple`] keeps the selection
    /// as-is. A [`SelectionChange`] is emitted when rows are dropped.
    pub fn set_mode(&mut self, mode: SelectionMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        if mode == SelectionMode::Single && self.selected.len() > 1 {
            let keep = self.current.get().unwrap_or(self.selected[0]);
            let removed = self
                .selected
                .iter()
                .copied()
                .filter(|i| *i != keep)
                .collect();
            self.selected = vec![keep];
            let current = Some(keep);
            self.set_current_value(current);
            let change = SelectionChange {
                added: Vec::new(),
                removed,
                current,
            };
            self.selection_changed.emit(&change);
        }
    }

    /// The signal carrying every [`SelectionChange`].
    pub fn selection_changed(&self) -> &Signal<SelectionChange> {
        &self.selection_changed
    }

    /// The current row as a shareable property, for binding cursors and inspectors.
    pub fn current_property(&self) -> &SharedProperty<Option<usize>> {
        &self.current
    }

    /// Number of selected rows.
    pub fn count(&self) -> usize {
        self.selected.len()
    }

    /// `true` when nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    /// All selected rows, sorted ascending.
    pub fn selected(&self) -> Vec<usize> {
        self.selected.clone()
    }

    /// The current row, if any.
    pub fn current(&self) -> Option<usize> {
        self.current.get()
    }

    /// `true` when `index` is selected.
    pub fn is_selected(&self, index: usize) -> bool {
        self.selected.binary_search(&index).is_ok()
    }

    /// Selects `index`.
    ///
    /// In [`SelectionMode::Single`] this replaces the whole selection; in
    /// [`SelectionMode::Multiple`] it adds one row. The row becomes the current row. A
    /// [`SelectionChange`] is emitted only when something actually changed.
    pub fn select(&mut self, index: usize) {
        match self.mode {
            SelectionMode::Single => {
                if self.selected.as_slice() == [index] && self.current() == Some(index) {
                    return;
                }
                let was_selected = self.is_selected(index);
                let removed: Vec<usize> = self
                    .selected
                    .iter()
                    .copied()
                    .filter(|i| *i != index)
                    .collect();
                self.selected = vec![index];
                self.set_current_value(Some(index));
                let change = SelectionChange {
                    added: if was_selected {
                        Vec::new()
                    } else {
                        vec![index]
                    },
                    removed,
                    current: Some(index),
                };
                self.selection_changed.emit(&change);
            }
            SelectionMode::Multiple => {
                let already = self.is_selected(index);
                let current_changed = self.current() != Some(index);
                if already && !current_changed {
                    return;
                }
                if !already {
                    let pos = self.selected.partition_point(|i| *i < index);
                    self.selected.insert(pos, index);
                }
                self.set_current_value(Some(index));
                let change = SelectionChange {
                    added: if already { Vec::new() } else { vec![index] },
                    removed: Vec::new(),
                    current: Some(index),
                };
                self.selection_changed.emit(&change);
            }
        }
    }

    /// Removes `index` from the selection.
    ///
    /// No-op in [`SelectionMode::Single`] unless `index` is the selected row, in which case the
    /// selection becomes empty. Removing the current row also clears the current row of
    /// [`SelectionModel::current_property`].
    pub fn deselect(&mut self, index: usize) {
        let pos = match self.selected.binary_search(&index) {
            Ok(pos) => pos,
            Err(_) => return,
        };
        self.selected.remove(pos);
        let current = self.current().filter(|c| *c != index);
        self.set_current_value(current);
        let change = SelectionChange {
            added: Vec::new(),
            removed: vec![index],
            current,
        };
        self.selection_changed.emit(&change);
    }

    /// Selects `index` if it is not selected, deselects it otherwise.
    ///
    /// In [`SelectionMode::Single`] toggling the selected row clears the selection.
    pub fn toggle(&mut self, index: usize) {
        if self.is_selected(index) {
            self.deselect(index);
        } else {
            self.select(index);
        }
    }

    /// Selects every row in `from..=to` (inclusive); the arguments may be given in any order.
    ///
    /// In [`SelectionMode::Single`] only the upper bound (`to` after normalisation) is selected,
    /// which mirrors what a click-drag over a single-selection list does in practice.
    pub fn select_range(&mut self, from: usize, to: usize) {
        let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
        match self.mode {
            SelectionMode::Single => {
                self.select(hi);
            }
            SelectionMode::Multiple => {
                // `self.selected` is kept sorted and unique at all times.
                let previous = self.selected.clone();
                let added: Vec<usize> = (lo..=hi)
                    .filter(|i| previous.binary_search(i).is_err())
                    .collect();
                let removed: Vec<usize> = previous
                    .iter()
                    .copied()
                    .filter(|i| (lo..=hi).contains(i))
                    .collect();
                let mut next: Vec<usize> = previous
                    .iter()
                    .copied()
                    .filter(|i| *i < lo || *i > hi)
                    .collect();
                next.extend(lo..=hi);
                next.sort_unstable();
                next.dedup();
                self.selected = next;
                self.set_current_value(Some(hi));
                if added.is_empty() && removed.is_empty() {
                    return;
                }
                let change = SelectionChange {
                    added,
                    removed,
                    current: Some(hi),
                };
                self.selection_changed.emit(&change);
            }
        }
    }

    /// Removes every selected row and clears the current row.
    ///
    /// Emits a [`SelectionChange`] only when the selection was not already empty.
    pub fn clear(&mut self) {
        if self.selected.is_empty() {
            // Still make sure the current row does not dangle.
            self.set_current_value(None);
            return;
        }
        let removed = std::mem::take(&mut self.selected);
        self.set_current_value(None);
        let change = SelectionChange {
            added: Vec::new(),
            removed,
            current: None,
        };
        self.selection_changed.emit(&change);
    }

    /// Selects every row in `0..count`.
    ///
    /// In [`SelectionMode::Single`] this selects nothing, because "everything" is not a single
    /// row; use [`SelectionModel::select`] there.
    pub fn select_all(&mut self, count: usize) {
        if self.mode == SelectionMode::Single {
            return;
        }
        let previous = self.selected.clone();
        let added: Vec<usize> = (0..count).filter(|i| !previous.contains(i)).collect();
        let removed: Vec<usize> = previous.iter().copied().filter(|i| *i >= count).collect();
        self.selected = (0..count).collect();
        let current = self.current().filter(|c| *c < count);
        self.set_current_value(current);
        if added.is_empty() && removed.is_empty() {
            return;
        }
        let change = SelectionChange {
            added,
            removed,
            current,
        };
        self.selection_changed.emit(&change);
    }

    /// Moves the current row to the next selected row, wrapping around; `false` when empty.
    pub fn select_next(&mut self) -> bool {
        self.step_current(1)
    }

    /// Moves the current row to the previous selected row, wrapping around; `false` when empty.
    pub fn select_previous(&mut self) -> bool {
        self.step_current(-1)
    }

    /// Sets the current row without changing the selection.
    ///
    /// The row must be part of the selection (or `None` to clear it); anything else is ignored so
    /// that the cursor can never leave the selection.
    pub fn set_current(&mut self, index: Option<usize>) -> bool {
        if let Some(index) = index {
            if !self.is_selected(index) {
                return false;
            }
        }
        if self.current() == index {
            return true;
        }
        self.set_current_value(index);
        let change = SelectionChange {
            added: Vec::new(),
            removed: Vec::new(),
            current: index,
        };
        self.selection_changed.emit(&change);
        true
    }

    /// Drops selected rows with an index `>= index`, used after the model shrank.
    ///
    /// Returns the number of removed rows and emits a [`SelectionChange`] when anything was
    /// dropped.
    pub fn remove_rows_from(&mut self, index: usize) -> usize {
        let removed: Vec<usize> = self
            .selected
            .iter()
            .copied()
            .filter(|i| *i >= index)
            .collect();
        if removed.is_empty() {
            return 0;
        }
        let removed_count = removed.len();
        self.selected.retain(|i| *i < index);
        let current = self.current().filter(|c| *c < index);
        self.set_current_value(current);
        let change = SelectionChange {
            added: Vec::new(),
            removed,
            current,
        };
        self.selection_changed.emit(&change);
        removed_count
    }

    fn step_current(&mut self, delta: isize) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        let len = self.selected.len();
        let base = match self.current() {
            Some(c) => match self.selected.binary_search(&c) {
                Ok(pos) => pos as isize,
                // The current row left the selection: start at the first row.
                Err(pos) => pos as isize - 1,
            },
            None => -1,
        };
        let next = (base + delta).rem_euclid(len as isize) as usize;
        let Some(row) = self.selected.get(next).copied() else {
            return false;
        };
        self.set_current(Some(row));
        true
    }

    fn set_current_value(&self, current: Option<usize>) {
        // Only notify when the value really differs; `SharedProperty` also requests a redraw.
        if self.current.get() != current {
            self.current.set(current);
        }
    }
}

impl std::fmt::Debug for SelectionModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectionModel")
            .field("mode", &self.mode)
            .field("selected", &self.selected)
            .field("current", &self.current.get())
            .field("connections", &self.selection_changed.connection_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn recorder(model: &SelectionModel) -> Rc<RefCell<Vec<SelectionChange>>> {
        let log = Rc::new(RefCell::new(Vec::new()));
        let sink = log.clone();
        model
            .selection_changed()
            .connect(move |c| sink.borrow_mut().push(c.clone()));
        log
    }

    #[test]
    fn single_mode_replaces_selection() {
        let mut selection = SelectionModel::new(SelectionMode::Single);
        let log = recorder(&selection);
        selection.select(1);
        selection.select(4);
        assert_eq!(selection.selected(), vec![4]);
        assert_eq!(selection.current(), Some(4));
        let changes = log.borrow();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].added, vec![1]);
        assert!(changes[1].removed_row(1));
        assert_eq!(changes[1].added, vec![4]);
        assert_eq!(changes[1].current, Some(4));
    }

    #[test]
    fn multiple_mode_accumulates_rows() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.select(2);
        selection.select(0);
        selection.select(1);
        assert_eq!(selection.selected(), vec![0, 1, 2]);
        assert_eq!(selection.count(), 3);
        assert!(!selection.is_empty());
    }

    #[test]
    fn repeated_select_emits_nothing() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.select(3);
        let log = recorder(&selection);
        selection.select(3);
        assert!(log.borrow().is_empty(), "no-op selection is silent");
    }

    #[test]
    fn selecting_a_selected_row_only_moves_the_current_row() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.select(3);
        selection.select(5);
        let log = recorder(&selection);
        selection.select(3);
        let changes = log.borrow();
        assert_eq!(changes.len(), 1);
        assert!(changes[0].added.is_empty(), "nothing was added");
        assert!(changes[0].removed.is_empty(), "nothing was removed");
        assert_eq!(changes[0].current, Some(3));
        assert_eq!(selection.selected(), vec![3, 5]);
    }

    #[test]
    fn toggle_adds_and_removes() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.toggle(1);
        assert!(selection.is_selected(1));
        selection.toggle(1);
        assert!(!selection.is_selected(1));
        assert!(selection.is_empty());
        assert_eq!(selection.current(), None);
    }

    #[test]
    fn toggle_in_single_mode_clears() {
        let mut selection = SelectionModel::default();
        assert_eq!(selection.mode(), SelectionMode::Single);
        selection.toggle(2);
        assert_eq!(selection.selected(), vec![2]);
        selection.toggle(2);
        assert!(selection.is_empty());
        assert_eq!(selection.current(), None);
    }

    #[test]
    fn select_range_normalises_order() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.select_range(4, 1);
        assert_eq!(selection.selected(), vec![1, 2, 3, 4]);
        assert_eq!(selection.current(), Some(4));
    }

    #[test]
    fn select_range_keeps_rows_outside() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.select(0);
        selection.select(9);
        selection.select_range(2, 4);
        assert_eq!(selection.selected(), vec![0, 2, 3, 4, 9]);
        let log = recorder(&selection);
        selection.select_range(3, 5);
        assert_eq!(selection.selected(), vec![0, 2, 3, 4, 5, 9]);
        let changes = log.borrow();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].added, vec![5]);
    }

    #[test]
    fn select_range_in_single_mode_selects_the_end() {
        let mut selection = SelectionModel::new(SelectionMode::Single);
        selection.select(7);
        selection.select_range(2, 3);
        assert_eq!(selection.selected(), vec![3]);
    }

    #[test]
    fn clear_reports_every_removed_row() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.select(0);
        selection.select(2);
        let log = recorder(&selection);
        selection.clear();
        assert!(selection.is_empty());
        assert_eq!(selection.count(), 0);
        assert_eq!(selection.current(), None);
        let changes = log.borrow();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].removed, vec![0, 2]);
        assert_eq!(changes[0].current, None);
    }

    #[test]
    fn clear_on_empty_selection_is_silent() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        let log = recorder(&selection);
        selection.clear();
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn current_property_is_shared_and_updates() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        let property = selection.current_property().clone();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        property
            .changed()
            .connect(move |v| sink.borrow_mut().push(*v));
        selection.select(5);
        assert_eq!(property.get(), Some(5));
        assert_eq!(*seen.borrow(), vec![Some(5)]);
        selection.clear();
        assert_eq!(property.get(), None);
        assert_eq!(seen.borrow().len(), 2);
    }

    #[test]
    fn select_all_and_truncate() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.select_all(3);
        assert_eq!(selection.selected(), vec![0, 1, 2]);
        let log = recorder(&selection);
        selection.select_all(2);
        assert_eq!(selection.selected(), vec![0, 1]);
        assert!(log.borrow()[0].removed_row(2));
        assert!(!log.borrow()[0].added_row(0));
    }

    #[test]
    fn select_all_is_a_no_op_in_single_mode() {
        let mut selection = SelectionModel::new(SelectionMode::Single);
        selection.select_all(5);
        assert!(selection.is_empty());
    }

    #[test]
    fn deselect_clears_current() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.select(1);
        selection.select(2);
        selection.deselect(2);
        assert_eq!(selection.selected(), vec![1]);
        assert_eq!(selection.current(), None, "the current row was removed");
        selection.deselect(42);
        assert_eq!(selection.selected(), vec![1]);
    }

    #[test]
    fn set_current_only_accepts_selected_rows() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.select(1);
        selection.select(2);
        assert!(!selection.set_current(Some(7)), "row 7 is not selected");
        assert_eq!(selection.current(), Some(2));
        assert!(selection.set_current(Some(1)));
        assert_eq!(selection.current(), Some(1));
        assert_eq!(selection.selected(), vec![1, 2]);
    }

    #[test]
    fn select_next_and_previous_wrap_around() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        assert!(!selection.select_next(), "empty selection cannot move");
        selection.select_range(1, 3);
        assert_eq!(selection.current(), Some(3));
        assert!(selection.select_next());
        assert_eq!(selection.current(), Some(1), "wrapped to the first row");
        assert!(selection.select_previous());
        assert_eq!(selection.current(), Some(3));
        assert!(selection.select_previous());
        assert_eq!(selection.current(), Some(2));
    }

    #[test]
    fn switching_to_single_mode_keeps_the_current_row() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.select(5);
        selection.select(6);
        selection.select(7);
        let log = recorder(&selection);
        selection.set_mode(SelectionMode::Single);
        assert_eq!(selection.selected(), vec![7]);
        assert_eq!(selection.current(), Some(7));
        let changes = log.borrow();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].removed, vec![5, 6]);
    }

    #[test]
    fn switching_modes_without_rows_is_silent() {
        let mut selection = SelectionModel::new(SelectionMode::Single);
        let log = recorder(&selection);
        selection.set_mode(SelectionMode::Multiple);
        assert_eq!(selection.mode(), SelectionMode::Multiple);
        selection.set_mode(SelectionMode::Multiple);
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn selection_change_helpers() {
        let change = SelectionChange {
            added: vec![1],
            removed: vec![2],
            current: Some(1),
        };
        assert!(!change.is_empty());
        assert!(change.added_row(1));
        assert!(change.removed_row(2));
        assert!(!change.added_row(2));
        assert!(SelectionChange::default().is_empty());
    }

    #[test]
    fn remove_rows_from_drops_the_tail() {
        let mut selection = SelectionModel::new(SelectionMode::Multiple);
        selection.select_range(0, 4);
        let log = recorder(&selection);
        let removed = selection.remove_rows_from(3);
        assert_eq!(removed, 2);
        assert_eq!(selection.selected(), vec![0, 1, 2]);
        assert_eq!(selection.current(), None);
        assert_eq!(log.borrow()[0].removed.len(), 2);
    }
}
