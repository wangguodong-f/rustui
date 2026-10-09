//! Value comparison and sort keys for tabular models (design document §28).
//!
//! Sorting is part of the *model* layer, not of the view: a [`crate::TableListModel`] knows its
//! columns, so it can reorder its own rows and emit a [`crate::TableChange::Sorted`]. A view then
//! only has to repaint.
//!
//! The ordering rules implemented here are deliberately explicit because cell data is heterogeneous
//! — a column may mix numbers, labels and flags:
//!
//! * different kinds are ordered by kind first: `Bool` < `Number` < `Text`/`Icon` < `None`;
//! * same-kind values compare naturally (`String` byte order, `f64` numeric order);
//! * [`ModelValue::None`] is greater than everything, so ascending sorts push empty cells to the
//!   bottom — the behaviour users expect from a spreadsheet.

use std::cmp::Ordering;

use crate::traits::ModelValue;

/// Total ordering over [`ModelValue`]s.
///
/// ```
/// use std::cmp::Ordering;
/// use rustui_model::{ModelValue, compare_values};
///
/// assert_eq!(compare_values(&ModelValue::Number(1.0), &ModelValue::Number(2.0)), Ordering::Less);
/// assert_eq!(compare_values(&ModelValue::Text("a".into()), &ModelValue::Text("b".into())), Ordering::Less);
/// assert_eq!(compare_values(&ModelValue::Bool(true), &ModelValue::Number(0.0)), Ordering::Less);
/// assert_eq!(compare_values(&ModelValue::None, &ModelValue::Text("z".into())), Ordering::Greater);
/// ```
pub fn compare_values(a: &ModelValue, b: &ModelValue) -> Ordering {
    let rank = |v: &ModelValue| match v {
        ModelValue::Bool(_) => 0u8,
        ModelValue::Number(_) => 1,
        ModelValue::Text(_) | ModelValue::Icon(_) => 2,
        ModelValue::None => 3,
    };
    match (a, b) {
        (ModelValue::Bool(x), ModelValue::Bool(y)) => x.cmp(y),
        (ModelValue::Number(x), ModelValue::Number(y)) => {
            x.partial_cmp(y).unwrap_or(Ordering::Equal)
        }
        (ModelValue::Text(x), ModelValue::Text(y)) => x.cmp(y),
        (ModelValue::Icon(x), ModelValue::Icon(y)) => x.cmp(y),
        (ModelValue::Text(x), ModelValue::Icon(y)) => x.cmp(y),
        (ModelValue::Icon(x), ModelValue::Text(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// Case-insensitive comparison with a case-sensitive tie break, so that `"a"` sorts before `"B"`
/// and `"apple"` before `"Apple"`.
pub fn compare_text(a: &str, b: &str) -> Ordering {
    let folded = a.to_lowercase().cmp(&b.to_lowercase());
    if folded == Ordering::Equal {
        a.cmp(b)
    } else {
        folded
    }
}

/// The numeric payload of a value, parsing text and icon values as `f64`.
///
/// `None` cells yield `None`; booleans yield `0.0` / `1.0`.
pub fn numeric_value(value: &ModelValue) -> Option<f64> {
    match value {
        ModelValue::Number(n) => Some(*n),
        ModelValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        ModelValue::Text(s) | ModelValue::Icon(s) => s.trim().parse::<f64>().ok(),
        ModelValue::None => None,
    }
}

/// The boolean payload of a value, understanding the usual textual spellings
/// (`"true"`, `"1"`, `"yes"`, `"on"` and their negatives, case-insensitively).
pub fn boolean_value(value: &ModelValue) -> Option<bool> {
    match value {
        ModelValue::Bool(b) => Some(*b),
        ModelValue::Number(n) => Some(*n != 0.0),
        ModelValue::Text(s) | ModelValue::Icon(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" | "y" => Some(true),
            "false" | "0" | "no" | "off" | "n" => Some(false),
            _ => None,
        },
        ModelValue::None => None,
    }
}

/// How the cells of one column are compared while sorting.
///
/// Not `PartialEq`: the [`SortKey::Custom`] variant holds a function pointer, whose address is not
/// a meaningful identity.
#[derive(Clone, Copy, Debug)]
pub enum SortKey {
    /// Case-insensitive text comparison.
    Text(usize),
    /// Numeric comparison; cells that are not numbers fall back to [`SortKey::Text`].
    Number(usize),
    /// Boolean comparison; cells that are not booleans fall back to [`SortKey::Text`].
    Bool(usize),
    /// A user supplied comparison applied to the two cells of the column.
    Custom {
        /// Column the comparator is applied to.
        column: usize,
        /// Comparison function.
        compare: fn(&ModelValue, &ModelValue) -> Ordering,
    },
}

impl SortKey {
    /// A case-insensitive text key on `column` (the default for user facing tables).
    pub fn text(column: usize) -> Self {
        SortKey::Text(column)
    }

    /// A numeric key on `column`.
    pub fn number(column: usize) -> Self {
        SortKey::Number(column)
    }

    /// A boolean key on `column`.
    pub fn bool(column: usize) -> Self {
        SortKey::Bool(column)
    }

    /// A custom key on `column`.
    pub fn custom(column: usize, compare: fn(&ModelValue, &ModelValue) -> Ordering) -> Self {
        SortKey::Custom { column, compare }
    }

    /// The column this key sorts by.
    pub fn column(self) -> usize {
        match self {
            SortKey::Text(column) | SortKey::Number(column) | SortKey::Bool(column) => column,
            SortKey::Custom { column, .. } => column,
        }
    }

    /// Compares two cells of the key's column.
    pub fn compare(self, a: &ModelValue, b: &ModelValue) -> Ordering {
        match self {
            SortKey::Text(_) => match (a, b) {
                (ModelValue::None, ModelValue::None) => Ordering::Equal,
                (ModelValue::None, _) => Ordering::Greater,
                (_, ModelValue::None) => Ordering::Less,
                _ => compare_text(&a.as_text(), &b.as_text()),
            },
            SortKey::Number(_) => match (numeric_value(a), numeric_value(b)) {
                (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
                (None, None) => compare_text(&a.as_text(), &b.as_text()),
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
            },
            SortKey::Bool(_) => match (boolean_value(a), boolean_value(b)) {
                (Some(x), Some(y)) => x.cmp(&y),
                (None, None) => compare_text(&a.as_text(), &b.as_text()),
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
            },
            SortKey::Custom { compare, .. } => compare(a, b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> ModelValue {
        ModelValue::Text(s.to_string())
    }

    #[test]
    fn numbers_compare_numerically() {
        assert_eq!(
            compare_values(&ModelValue::Number(2.0), &ModelValue::Number(10.0)),
            Ordering::Less
        );
        assert_eq!(
            compare_values(&ModelValue::Number(10.0), &ModelValue::Number(2.0)),
            Ordering::Greater
        );
        assert_eq!(
            compare_values(&ModelValue::Number(1.0), &ModelValue::Number(1.0)),
            Ordering::Equal
        );
    }

    #[test]
    fn nan_sorts_equal_instead_of_panicking() {
        assert_eq!(
            compare_values(&ModelValue::Number(f64::NAN), &ModelValue::Number(1.0)),
            Ordering::Equal
        );
    }

    #[test]
    fn kinds_are_ordered_by_rank() {
        assert_eq!(
            compare_values(&ModelValue::Bool(false), &ModelValue::Number(0.0)),
            Ordering::Less
        );
        assert_eq!(
            compare_values(&ModelValue::Number(9.0), &ModelValue::Text("a".into())),
            Ordering::Less
        );
        assert_eq!(
            compare_values(&ModelValue::Text("a".into()), &ModelValue::None),
            Ordering::Less
        );
        assert_eq!(
            compare_values(&ModelValue::None, &ModelValue::None),
            Ordering::Equal
        );
    }

    #[test]
    fn text_and_icon_compare_as_strings() {
        assert_eq!(compare_values(&text("a"), &text("b")), Ordering::Less);
        assert_eq!(
            compare_values(&text("b"), &ModelValue::Icon("a".into())),
            Ordering::Greater
        );
        assert_eq!(
            compare_values(&ModelValue::Icon("a".into()), &text("a")),
            Ordering::Equal
        );
    }

    #[test]
    fn compare_text_is_case_insensitive_then_case_sensitive() {
        assert_eq!(compare_text("apple", "Banana"), Ordering::Less);
        assert_eq!(compare_text("Apple", "apple"), Ordering::Less, "'A' < 'a'");
        assert_eq!(compare_text("apple", "APPLE"), Ordering::Greater);
        assert_eq!(
            compare_text("Apple", "APPLE"),
            Ordering::Greater,
            "'p' > 'P'"
        );
        assert_eq!(compare_text("same", "same"), Ordering::Equal);
    }

    #[test]
    fn numeric_value_parses_text_and_flags() {
        assert_eq!(numeric_value(&ModelValue::Number(1.0)), Some(1.0));
        assert_eq!(numeric_value(&text(" 2.5 ")), Some(2.5));
        assert_eq!(numeric_value(&ModelValue::Bool(true)), Some(1.0));
        assert_eq!(numeric_value(&text("abc")), None);
        assert_eq!(numeric_value(&ModelValue::None), None);
    }

    #[test]
    fn boolean_value_understands_spellings() {
        assert_eq!(boolean_value(&ModelValue::Bool(true)), Some(true));
        assert_eq!(boolean_value(&text("YES")), Some(true));
        assert_eq!(boolean_value(&text(" off ")), Some(false));
        assert_eq!(boolean_value(&ModelValue::Number(0.0)), Some(false));
        assert_eq!(boolean_value(&text("maybe")), None);
        assert_eq!(boolean_value(&ModelValue::None), None);
    }

    #[test]
    fn sort_keys_expose_their_column() {
        assert_eq!(SortKey::text(1).column(), 1);
        assert_eq!(SortKey::number(2).column(), 2);
        assert_eq!(SortKey::bool(3).column(), 3);
        assert_eq!(SortKey::custom(4, compare_values).column(), 4);
    }

    #[test]
    fn number_key_falls_back_to_text() {
        let key = SortKey::number(0);
        assert_eq!(
            key.compare(&text("10"), &text("9")),
            Ordering::Greater,
            "numeric: 10 > 9"
        );
        assert_eq!(
            key.compare(&text("abc"), &text("abd")),
            Ordering::Less,
            "textual"
        );
        assert_eq!(
            key.compare(&text("5"), &text("abc")),
            Ordering::Less,
            "numbers first"
        );
        assert_eq!(
            key.compare(&text("abc"), &text("5")),
            Ordering::Greater,
            "numbers first"
        );
        assert_eq!(
            key.compare(&text("abc"), &ModelValue::None),
            Ordering::Greater,
            "both are non numeric: compared as text, and the empty cell goes last"
        );
    }

    #[test]
    fn bool_key_sorts_false_before_true() {
        let key = SortKey::bool(0);
        assert_eq!(key.compare(&text("false"), &text("true")), Ordering::Less);
        assert_eq!(key.compare(&text("true"), &text("unknown")), Ordering::Less);
        assert_eq!(
            key.compare(&text("unknown"), &text("unknown")),
            Ordering::Equal
        );
    }

    #[test]
    fn text_key_pushes_none_to_the_end() {
        let key = SortKey::text(0);
        assert_eq!(
            key.compare(&ModelValue::None, &text("a")),
            Ordering::Greater
        );
        assert_eq!(
            key.compare(&ModelValue::None, &ModelValue::None),
            Ordering::Equal
        );
        assert_eq!(key.compare(&text("a"), &text("B")), Ordering::Less);
    }

    #[test]
    fn custom_key_uses_the_supplied_function() {
        let reverse_len =
            |a: &ModelValue, b: &ModelValue| b.as_text().len().cmp(&a.as_text().len());
        let key = SortKey::custom(0, reverse_len);
        assert_eq!(key.compare(&text("ab"), &text("c")), Ordering::Less);
    }
}
