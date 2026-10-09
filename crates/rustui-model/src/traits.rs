//! Model traits and the value type they exchange with views (design document §27, §29).
//!
//! The Model/View split of RustUI is deliberately tiny: a model answers *"how many rows do you
//! have?"* and *"what is in row `r`?"*, and a view (`ListView`, `TableView`, `TreeView` in
//! `rustui-widgets`) turns those answers into widget subtrees. Large data sets never become
//! widgets: a view asks for the ~30 rows it can actually paint (§28, §51).
//!
//! Three shapes are covered:
//!
//! | trait         | shape | addressed by            |
//! |---------------|-------|-------------------------|
//! | [`ListModel`] | 1-D   | `row: usize`            |
//! | [`TableModel`]| 2-D   | `row`, `column`         |
//! | [`TreeModel`] | graph | [`NodeId`]              |
//!
//! All accessors are total: asking for a row, column or node that does not exist never panics and
//! never indexes out of bounds — the models simply answer [`ModelValue::None`]. That property is
//! what lets a view scroll, virtualize and refresh lazily without defensive code at every call
//! site.

use std::cmp::Ordering;
use std::fmt;

/// Identifies a node inside a [`TreeModel`].
///
/// `NodeId` is a plain newtype over `u32`: the concrete models assign it as the index of an arena
/// slot, but views must treat it as opaque and only ever compare it or hand it back to the model
/// that produced it.
///
/// [`NodeId::INVALID`] is the "no node" value; models answer `None` instead of returning it, and it
/// is what a default-constructed id carries.
///
/// ```
/// use rustui_model::{NodeId, TreeListModel};
///
/// let mut model = TreeListModel::new();
/// let root = model.insert(None, 0, "vcpkg");
/// let child = model.insert(Some(root), 0, "packages");
///
/// assert_eq!(child.to_u32(), 1);
/// assert_ne!(child, NodeId::INVALID);
/// assert_eq!(model.data(&child).as_text(), "packages");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    /// Id that never refers to a node; used as "null node".
    pub const INVALID: NodeId = NodeId(u32::MAX);

    /// Creates an id from a raw index.
    #[inline]
    pub const fn new(index: u32) -> Self {
        NodeId(index)
    }

    /// The raw index backing this id.
    #[inline]
    pub const fn to_u32(self) -> u32 {
        self.0
    }

    /// The raw index as a `usize`, or `None` for [`NodeId::INVALID`].
    #[inline]
    pub const fn as_usize(self) -> Option<usize> {
        if self.is_valid() {
            Some(self.0 as usize)
        } else {
            None
        }
    }

    /// `false` for [`NodeId::INVALID`].
    #[inline]
    pub const fn is_valid(self) -> bool {
        self.0 != u32::MAX
    }
}

impl Default for NodeId {
    fn default() -> Self {
        NodeId::INVALID
    }
}

impl From<u32> for NodeId {
    fn from(value: u32) -> Self {
        NodeId(value)
    }
}

impl From<NodeId> for u32 {
    fn from(value: NodeId) -> Self {
        value.0
    }
}

impl fmt::Display for NodeId {
    /// Formats as `#3`, mirroring the way [`rustui_core::ObjectId`] renders, or `#invalid`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_valid() {
            write!(f, "#{}", self.0)
        } else {
            f.write_str("#invalid")
        }
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_valid() {
            write!(f, "NodeId({})", self.0)
        } else {
            f.write_str("NodeId::INVALID")
        }
    }
}

/// A cell of model data.
///
/// Views do not need to know whether a cell holds a number, a label, a flag or an icon name; they
/// ask for it as a [`ModelValue`] and render it with [`Display`](std::fmt::Display). This keeps the
/// model traits object-safe and free of generics, which is what makes `Box<dyn TableModel>`
/// virtualized table views possible.
///
/// ```
/// use rustui_model::ModelValue;
///
/// assert_eq!(ModelValue::from("42").as_text(), "42");
/// assert_eq!(ModelValue::Number(1.5).to_string(), "1.5");
/// assert_eq!(ModelValue::Number(3.0).to_string(), "3");
/// assert_eq!(ModelValue::Bool(true).to_string(), "true");
/// assert!(ModelValue::None.is_none());
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ModelValue {
    /// A text cell.
    Text(String),
    /// A numeric cell.
    Number(f64),
    /// A checkbox-like cell.
    Bool(bool),
    /// A named icon (a ligature name or an asset key resolved by the widget layer).
    Icon(String),
    /// An absent / invalid cell — the answer of every model for an out-of-range index.
    #[default]
    None,
}

impl ModelValue {
    /// Builds a text value.
    pub fn text(value: impl Into<String>) -> Self {
        ModelValue::Text(value.into())
    }

    /// Builds an icon value.
    pub fn icon(value: impl Into<String>) -> Self {
        ModelValue::Icon(value.into())
    }

    /// `true` only for [`ModelValue::None`].
    pub fn is_none(&self) -> bool {
        matches!(self, ModelValue::None)
    }

    /// Borrowed text of a [`ModelValue::Text`] or [`ModelValue::Icon`] value.
    ///
    /// Numeric and boolean values return `None`; use [`ModelValue::as_text`] for the rendered
    /// string of any variant.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ModelValue::Text(s) | ModelValue::Icon(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// The rendered text of any variant; `ModelValue::None` becomes an empty string.
    ///
    /// This is what a view paints when no special formatting is requested.
    pub fn as_text(&self) -> String {
        match self {
            ModelValue::Text(s) | ModelValue::Icon(s) => s.clone(),
            ModelValue::Number(_) | ModelValue::Bool(_) | ModelValue::None => {
                self.to_display_string()
            }
        }
    }

    /// The string a view should paint, identical to the [`Display`](fmt::Display) output.
    ///
    /// Numbers with a zero fractional part are printed without a decimal point so that
    /// `ModelValue::Number(3.0)` renders as `"3"`.
    pub fn to_display_string(&self) -> String {
        match self {
            ModelValue::Text(s) | ModelValue::Icon(s) => s.clone(),
            ModelValue::Number(n) => {
                if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }
            ModelValue::Bool(b) => {
                if *b {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            ModelValue::None => String::new(),
        }
    }

    /// Numeric payload of a [`ModelValue::Number`], else `None`.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            ModelValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Boolean payload of a [`ModelValue::Bool`], else `None`.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ModelValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Total ordering over values, see [`crate::compare_values`].
    pub fn compare(&self, other: &ModelValue) -> Ordering {
        crate::compare_values(self, other)
    }
}

impl fmt::Display for ModelValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_display_string())
    }
}

impl PartialOrd for ModelValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.compare(other))
    }
}

impl From<&str> for ModelValue {
    fn from(value: &str) -> Self {
        ModelValue::Text(value.to_string())
    }
}

impl From<String> for ModelValue {
    fn from(value: String) -> Self {
        ModelValue::Text(value)
    }
}

impl From<&String> for ModelValue {
    fn from(value: &String) -> Self {
        ModelValue::Text(value.clone())
    }
}

impl From<f64> for ModelValue {
    fn from(value: f64) -> Self {
        ModelValue::Number(value)
    }
}

impl From<f32> for ModelValue {
    fn from(value: f32) -> Self {
        ModelValue::Number(f64::from(value))
    }
}

impl From<bool> for ModelValue {
    fn from(value: bool) -> Self {
        ModelValue::Bool(value)
    }
}

impl From<i64> for ModelValue {
    fn from(value: i64) -> Self {
        ModelValue::Number(value as f64)
    }
}

impl From<i32> for ModelValue {
    fn from(value: i32) -> Self {
        ModelValue::Number(f64::from(value))
    }
}

impl From<u32> for ModelValue {
    fn from(value: u32) -> Self {
        ModelValue::Number(f64::from(value))
    }
}

impl From<u64> for ModelValue {
    fn from(value: u64) -> Self {
        ModelValue::Number(value as f64)
    }
}

impl From<usize> for ModelValue {
    fn from(value: usize) -> Self {
        ModelValue::Number(value as f64)
    }
}

impl From<ModelValue> for String {
    fn from(value: ModelValue) -> Self {
        value.to_display_string()
    }
}

/// A one dimensional model: a list of rows, each row a single [`ModelValue`].
///
/// The trait is object-safe so that a `ListView` can hold a `Box<dyn ListModel>` and support models
/// that are generated lazily (a million row log, a paged HTTP resource, ...).
pub trait ListModel {
    /// Number of rows currently exposed by the model.
    fn row_count(&self) -> usize;

    /// The value of `row`, or [`ModelValue::None`] when `row` is out of range.
    fn data(&self, row: usize) -> ModelValue;

    /// Convenience: `true` when the model exposes no rows.
    fn is_empty(&self) -> bool {
        self.row_count() == 0
    }

    /// `true` when `row` is inside the model; lets a view skip the `ModelValue::None` dance.
    fn contains(&self, row: usize) -> bool {
        row < self.row_count()
    }

    /// [`ListModel::data`] wrapped in an `Option`, based on the index being in range.
    fn get(&self, row: usize) -> Option<ModelValue> {
        if self.contains(row) {
            Some(self.data(row))
        } else {
            None
        }
    }
}

/// A two dimensional model: a grid of rows and columns, with a header row.
///
/// Column indices refer to the *model's* column order; the header text is data too
/// ([`TableModel::header_data`]) so that a view can sort or resize columns without a second source
/// of truth.
pub trait TableModel {
    /// Number of data rows (the header row is not counted).
    fn row_count(&self) -> usize;

    /// Number of columns.
    fn column_count(&self) -> usize;

    /// The header cell of `column`, or [`ModelValue::None`] when `column` is out of range.
    fn header_data(&self, column: usize) -> ModelValue;

    /// The cell at `(row, column)`, or [`ModelValue::None`] when either index is out of range.
    fn data(&self, row: usize, column: usize) -> ModelValue;

    /// Convenience: `true` when the model exposes no data rows.
    fn is_empty(&self) -> bool {
        self.row_count() == 0
    }

    /// `true` when both indices are inside the model.
    fn contains(&self, row: usize, column: usize) -> bool {
        row < self.row_count() && column < self.column_count()
    }

    /// [`TableModel::data`] wrapped in an `Option`, based on both indices being in range.
    fn cell(&self, row: usize, column: usize) -> Option<ModelValue> {
        if self.contains(row, column) {
            Some(self.data(row, column))
        } else {
            None
        }
    }
}

/// A hierarchical model: roots, children and a parent link per [`NodeId`].
///
/// Expansion state lives in the model (and not in the view) so that it survives view recreation and
/// can be persisted; a model that does not care about expansion simply keeps the default
/// implementations.
pub trait TreeModel {
    /// Number of top level nodes.
    fn root_count(&self) -> usize;

    /// Number of children of `node`, `0` for an unknown or childless node.
    fn child_count(&self, node: &NodeId) -> usize;

    /// The `index`-th child of `node`, or `None` when out of range.
    fn child(&self, node: &NodeId, index: usize) -> Option<NodeId>;

    /// The parent of `node`, or `None` for roots and unknown nodes.
    fn parent(&self, node: &NodeId) -> Option<NodeId>;

    /// The label (or any display value) of `node`, or [`ModelValue::None`] for unknown nodes.
    fn data(&self, node: &NodeId) -> ModelValue;

    /// Whether `node` is expanded; models without expansion state answer `false`.
    fn is_expanded(&self, _node: &NodeId) -> bool {
        false
    }

    /// Sets the expansion state of `node`; models without expansion state ignore it.
    fn set_expanded(&mut self, _node: &NodeId, _expanded: bool) {}

    /// Convenience: `true` when `node` has no children.
    fn is_leaf(&self, node: &NodeId) -> bool {
        self.child_count(node) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_display_and_debug() {
        assert_eq!(NodeId(3).to_string(), "#3");
        assert_eq!(format!("{:?}", NodeId(3)), "NodeId(3)");
        assert_eq!(NodeId::INVALID.to_string(), "#invalid");
        assert_eq!(format!("{:?}", NodeId::INVALID), "NodeId::INVALID");
    }

    #[test]
    fn node_id_conversions_and_validity() {
        assert_eq!(NodeId::from(7u32), NodeId(7));
        assert_eq!(u32::from(NodeId(7)), 7);
        assert!(NodeId(7).is_valid());
        assert!(!NodeId::INVALID.is_valid());
        assert_eq!(NodeId::default(), NodeId::INVALID);
        assert_eq!(NodeId(7).as_usize(), Some(7));
        assert_eq!(NodeId::INVALID.as_usize(), None);
    }

    #[test]
    fn node_id_orders_by_index() {
        let mut ids = vec![NodeId(9), NodeId(1), NodeId(5)];
        ids.sort();
        assert_eq!(ids, vec![NodeId(1), NodeId(5), NodeId(9)]);
    }

    #[test]
    fn model_value_from_conversions() {
        assert_eq!(ModelValue::from("a"), ModelValue::Text("a".into()));
        assert_eq!(
            ModelValue::from(String::from("b")),
            ModelValue::Text("b".into())
        );
        assert_eq!(ModelValue::from(true), ModelValue::Bool(true));
        assert_eq!(ModelValue::from(1.5f64), ModelValue::Number(1.5));
        assert_eq!(ModelValue::from(1.5f32), ModelValue::Number(1.5));
        assert_eq!(ModelValue::from(9i64), ModelValue::Number(9.0));
        assert_eq!(ModelValue::from(-3i32), ModelValue::Number(-3.0));
        assert_eq!(ModelValue::from(3u32), ModelValue::Number(3.0));
        assert_eq!(ModelValue::from(4u64), ModelValue::Number(4.0));
        assert_eq!(ModelValue::from(5usize), ModelValue::Number(5.0));
        assert_eq!(ModelValue::text("t"), ModelValue::Text("t".into()));
        assert_eq!(ModelValue::icon("i"), ModelValue::Icon("i".into()));
        assert_eq!(ModelValue::default(), ModelValue::None);
    }

    #[test]
    fn model_value_accessors() {
        assert_eq!(ModelValue::Text("x".into()).as_str(), Some("x"));
        assert_eq!(ModelValue::Icon("x".into()).as_str(), Some("x"));
        assert_eq!(ModelValue::Number(1.0).as_str(), None);
        assert_eq!(ModelValue::Number(2.5).as_number(), Some(2.5));
        assert_eq!(ModelValue::Bool(true).as_bool(), Some(true));
        assert_eq!(ModelValue::None.as_number(), None);
        assert_eq!(ModelValue::Bool(false).as_text(), "false");
        assert_eq!(ModelValue::Text("hi".into()).as_text(), "hi");
        assert!(ModelValue::None.is_none());
        assert!(!ModelValue::Text(String::new()).is_none());
    }

    #[test]
    fn model_value_display() {
        assert_eq!(ModelValue::Text("abc".into()).to_string(), "abc");
        assert_eq!(ModelValue::Icon("folder".into()).to_string(), "folder");
        assert_eq!(ModelValue::Number(3.0).to_string(), "3");
        assert_eq!(ModelValue::Number(-12.0).to_string(), "-12");
        assert_eq!(ModelValue::Number(1.25).to_string(), "1.25");
        assert_eq!(ModelValue::Bool(true).to_string(), "true");
        assert_eq!(ModelValue::None.to_string(), "");
        assert_eq!(ModelValue::Number(f64::INFINITY).to_string(), "inf");
        assert_eq!(
            String::from(ModelValue::Number(1.0)),
            "1".to_string(),
            "conversion into String"
        );
    }

    #[test]
    fn model_value_partial_order() {
        let mut values = [
            ModelValue::None,
            ModelValue::Text("b".into()),
            ModelValue::Number(1.0),
            ModelValue::Bool(false),
        ];
        values.sort_by(ModelValue::compare);
        assert_eq!(values[0], ModelValue::Bool(false));
        assert_eq!(values[1], ModelValue::Number(1.0));
        assert_eq!(values[2], ModelValue::Text("b".into()));
        assert_eq!(values[3], ModelValue::None);
    }

    /// Minimal table used to exercise the trait defaults and bounds behaviour.
    struct TwoByTwo;

    impl TableModel for TwoByTwo {
        fn row_count(&self) -> usize {
            2
        }

        fn column_count(&self) -> usize {
            2
        }

        fn header_data(&self, column: usize) -> ModelValue {
            ModelValue::Text(format!("h{column}"))
        }

        fn data(&self, row: usize, column: usize) -> ModelValue {
            if self.contains(row, column) {
                ModelValue::Number((row * 10 + column) as f64)
            } else {
                ModelValue::None
            }
        }
    }

    #[test]
    fn table_model_defaults_and_bounds() {
        let model = TwoByTwo;
        assert!(!model.is_empty());
        assert!(model.contains(1, 1));
        assert!(!model.contains(2, 0));
        assert!(!model.contains(0, 2));
        assert_eq!(model.cell(1, 1), Some(ModelValue::Number(11.0)));
        assert_eq!(model.cell(9, 9), None);
        assert_eq!(model.data(9, 9), ModelValue::None);
        assert_eq!(model.header_data(1).as_text(), "h1");
    }

    /// Minimal tree that keeps the default expansion behaviour.
    struct PlainTree {
        roots: Vec<NodeId>,
        children: Vec<(NodeId, Vec<NodeId>)>,
    }

    impl PlainTree {
        fn new() -> Self {
            let root = NodeId(0);
            let child = NodeId(1);
            PlainTree {
                roots: vec![root],
                children: vec![(root, vec![child]), (child, vec![])],
            }
        }
    }

    impl TreeModel for PlainTree {
        fn root_count(&self) -> usize {
            self.roots.len()
        }

        fn child_count(&self, node: &NodeId) -> usize {
            self.children
                .iter()
                .find(|(id, _)| id == node)
                .map(|(_, c)| c.len())
                .unwrap_or(0)
        }

        fn child(&self, node: &NodeId, index: usize) -> Option<NodeId> {
            self.children
                .iter()
                .find(|(id, _)| id == node)
                .and_then(|(_, c)| c.get(index).copied())
        }

        fn parent(&self, node: &NodeId) -> Option<NodeId> {
            if *node == NodeId(1) {
                Some(NodeId(0))
            } else {
                None
            }
        }

        fn data(&self, node: &NodeId) -> ModelValue {
            ModelValue::Text(format!("node{}", node.0))
        }
    }

    #[test]
    fn tree_model_defaults() {
        let mut model = PlainTree::new();
        assert_eq!(model.root_count(), 1);
        assert_eq!(model.child_count(&NodeId(0)), 1);
        assert_eq!(model.child_count(&NodeId::INVALID), 0);
        assert_eq!(model.child(&NodeId(0), 0), Some(NodeId(1)));
        assert_eq!(model.child(&NodeId(0), 5), None);
        assert_eq!(model.parent(&NodeId(1)), Some(NodeId(0)));
        assert_eq!(model.parent(&NodeId(0)), None);
        assert_eq!(model.data(&NodeId(2)).as_text(), "node2");
        assert!(model.is_leaf(&NodeId(1)));
        assert!(!model.is_leaf(&NodeId(0)));
        // Default implementations of the expansion hooks.
        assert!(!model.is_expanded(&NodeId(0)));
        model.set_expanded(&NodeId(0), true);
        assert!(!model.is_expanded(&NodeId(0)));
    }

    /// List model returning `ModelValue::None` for anything out of range.
    struct Numbers(Vec<f64>);

    impl ListModel for Numbers {
        fn row_count(&self) -> usize {
            self.0.len()
        }

        fn data(&self, row: usize) -> ModelValue {
            match self.0.get(row) {
                Some(v) => ModelValue::Number(*v),
                None => ModelValue::None,
            }
        }
    }

    #[test]
    fn list_model_defaults() {
        let model = Numbers(vec![1.0, 2.0]);
        assert_eq!(model.row_count(), 2);
        assert!(!model.is_empty());
        assert!(model.contains(1));
        assert!(!model.contains(2));
        assert_eq!(model.get(0), Some(ModelValue::Number(1.0)));
        assert_eq!(model.get(2), None);
        assert!(Numbers(Vec::new()).is_empty());
    }

    #[test]
    fn out_of_range_data_is_none_not_a_panic() {
        let model = Numbers(vec![42.0]);
        assert_eq!(model.data(usize::MAX), ModelValue::None);
        assert_eq!(model.get(usize::MAX), None);
    }
}
