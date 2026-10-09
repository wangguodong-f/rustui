//! A concrete, observable tree of labelled nodes (design document §29).
//!
//! [`TreeListModel`] keeps its nodes in an arena (`Vec<Option<TreeNode>>`) and hands out
//! [`NodeId`]s as arena indices. Removing a node frees its slot, but the slot is **never reused**:
//! ids handed to a view therefore stay meaningful (a stale id simply answers `ModelValue::None` or
//! `None` instead of silently addressing a different node).
//!
//! ```
//! use rustui_model::{NodeId, TreeBuilder, TreeModel};
//!
//! let mut builder = TreeBuilder::new();
//! let root = builder.root("vcpkg");
//! let packages = builder.child(root, "packages");
//! builder.child(packages, "fmt");
//! let mut model = builder.build();
//!
//! assert_eq!(model.data(&root).as_text(), "vcpkg");
//! assert_eq!(model.child_count(&root), 1);
//!
//! assert!(model.remove(&root));
//! assert_eq!(model.root_count(), 0);
//! assert!(!model.is_valid(&root));
//! assert!(!model.is_valid(&NodeId::INVALID));
//! ```

use std::collections::VecDeque;
use std::fmt;

use rustui_core::{Shared, Signal};

use crate::traits::{ModelValue, NodeId, TreeModel};

/// What happened to a [`TreeListModel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeChange {
    /// A node was inserted; `parent` is `None` for roots.
    Inserted {
        /// The new node.
        node: NodeId,
        /// Parent of the new node, if any.
        parent: Option<NodeId>,
    },
    /// A node and its subtree were removed.
    Removed {
        /// The node that was removed (its descendants keep their ids but become unknown).
        node: NodeId,
    },
    /// The label of a node changed.
    LabelChanged {
        /// The node whose label changed.
        node: NodeId,
    },
    /// The expansion state of a node changed.
    Expanded {
        /// The node whose expansion state changed.
        node: NodeId,
        /// The new state.
        expanded: bool,
    },
    /// Every node was removed by [`TreeListModel::clear`].
    Cleared,
}

/// One node of a [`TreeListModel`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeNode {
    label: String,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    expanded: bool,
}

impl TreeNode {
    fn new(label: impl Into<String>, parent: Option<NodeId>) -> Self {
        TreeNode {
            label: label.into(),
            parent,
            children: Vec::new(),
            expanded: false,
        }
    }

    /// The text of the node.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The parent node, or `None` for roots.
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// The child ids, in display order.
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    /// Whether the node is expanded.
    pub fn expanded(&self) -> bool {
        self.expanded
    }

    /// `true` when the node has no children.
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    /// The node as a [`ModelValue`], which is what a tree view paints.
    pub fn to_value(&self) -> ModelValue {
        ModelValue::Text(self.label.clone())
    }
}

/// An arena backed [`TreeModel`] with expansion state and change notification.
#[derive(Clone)]
pub struct TreeListModel {
    nodes: Vec<Option<TreeNode>>,
    roots: Vec<NodeId>,
    live: usize,
    changed: Signal<TreeChange>,
}

impl Default for TreeListModel {
    fn default() -> Self {
        TreeListModel::new()
    }
}

impl fmt::Debug for TreeListModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeListModel")
            .field("roots", &self.roots)
            .field("nodes", &self.live)
            .field("capacity", &self.nodes.len())
            .field("connections", &self.changed.connection_count())
            .finish()
    }
}

impl TreeListModel {
    /// An empty tree.
    pub fn new() -> Self {
        TreeListModel {
            nodes: Vec::new(),
            roots: Vec::new(),
            live: 0,
            changed: Signal::new(),
        }
    }

    /// The signal describing every change; emits after the model was already mutated.
    pub fn changed(&self) -> &Signal<TreeChange> {
        &self.changed
    }

    /// Number of live nodes, including hidden (collapsed) subtrees.
    pub fn node_count(&self) -> usize {
        self.live
    }

    /// `true` when the tree has no node at all.
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// `true` when `node` refers to a live node.
    pub fn is_valid(&self, node: &NodeId) -> bool {
        self.slot(*node).is_some()
    }

    /// The top level node ids, in display order.
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    /// The `index`-th top level node id.
    pub fn root(&self, index: usize) -> Option<NodeId> {
        self.roots.get(index).copied()
    }

    /// Borrows the node behind `id`, or `None` for an unknown id.
    pub fn node(&self, id: &NodeId) -> Option<&TreeNode> {
        self.slot(*id)
    }

    /// Mutably borrows the node behind `id`, or `None` for an unknown id.
    pub fn node_mut(&mut self, id: &NodeId) -> Option<&mut TreeNode> {
        self.nodes.get_mut(id.as_usize()?).and_then(Option::as_mut)
    }

    /// The label of `id`, or `None` for an unknown id.
    pub fn label(&self, id: &NodeId) -> Option<&str> {
        self.slot(*id).map(TreeNode::label)
    }

    /// Renames `id`; `false` for an unknown id or when the label is unchanged.
    pub fn set_label(&mut self, id: &NodeId, label: impl Into<String>) -> bool {
        let label = label.into();
        let Some(node) = self.node_mut(id) else {
            return false;
        };
        if node.label == label {
            return false;
        }
        node.label = label;
        self.changed.emit(&TreeChange::LabelChanged { node: *id });
        true
    }

    /// Inserts a node labelled `label`.
    ///
    /// * `parent` `None` (or an unknown id) inserts a top level node;
    /// * otherwise the node becomes the `index`-th child of `parent`, clamping `index` to the end
    ///   of the child list.
    ///
    /// The function never fails: an unusable parent degrades to "insert at the root", because a
    /// builder-style API should always produce a node to work with. Returns the id of the new node.
    pub fn insert(&mut self, parent: Option<NodeId>, index: usize, label: &str) -> NodeId {
        let parent = parent.filter(|p| self.is_valid(p));
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Some(TreeNode::new(label, parent)));
        self.live += 1;
        match parent {
            Some(p) => {
                if let Some(node) = self.node_mut(&p) {
                    let at = index.min(node.children.len());
                    node.children.insert(at, id);
                }
            }
            None => {
                let at = index.min(self.roots.len());
                self.roots.insert(at, id);
            }
        }
        self.changed
            .emit(&TreeChange::Inserted { node: id, parent });
        id
    }

    /// Appends a child to `parent` (or a root when `parent` is `None`).
    pub fn append(&mut self, parent: Option<NodeId>, label: &str) -> NodeId {
        let index = match parent {
            Some(p) => TreeListModel::child_count(self, &p),
            None => self.roots.len(),
        };
        self.insert(parent, index, label)
    }

    /// Removes `id` together with its whole subtree.
    ///
    /// Descendants keep their numeric ids but become unknown to the model. Returns `false` for an
    /// id that is already unknown (removing twice is therefore safe).
    pub fn remove(&mut self, id: &NodeId) -> bool {
        if !self.is_valid(id) {
            return false;
        }
        // Detach from the parent (or from the root list) before the slot is freed, so that the
        // remaining tree never points at a removed node.
        match TreeListModel::parent(self, id) {
            Some(parent) => {
                if let Some(node) = self.node_mut(&parent) {
                    node.children.retain(|child| child != id);
                }
            }
            None => self.roots.retain(|root| root != id),
        }
        let mut queue: VecDeque<NodeId> = VecDeque::new();
        queue.push_back(*id);
        while let Some(current) = queue.pop_front() {
            if let Some(node) = self.slot(current) {
                for child in node.children.iter() {
                    queue.push_back(*child);
                }
            }
            if let Some(index) = current.as_usize() {
                if let Some(slot) = self.nodes.get_mut(index) {
                    *slot = None;
                }
                self.live -= 1;
            }
        }
        self.changed.emit(&TreeChange::Removed { node: *id });
        true
    }

    /// Removes every node; emits [`TreeChange::Cleared`] when the tree was not already empty.
    pub fn clear(&mut self) {
        if self.live == 0 {
            return;
        }
        self.nodes.clear();
        self.roots.clear();
        self.live = 0;
        self.changed.emit(&TreeChange::Cleared);
    }

    /// Number of nodes that are currently expanded.
    pub fn expanded_count(&self) -> usize {
        self.nodes
            .iter()
            .filter_map(Option::as_ref)
            .filter(|node| node.expanded)
            .count()
    }

    /// Number of children of `id`; `0` for an unknown node.
    pub fn child_count(&self, id: &NodeId) -> usize {
        self.node(id).map(|n| n.children.len()).unwrap_or(0)
    }

    /// The child ids of `id`, or `None` for an unknown node.
    pub fn children(&self, id: &NodeId) -> Option<&[NodeId]> {
        self.node(id).map(TreeNode::children)
    }

    /// The `index`-th child of `id`, or `None` when out of range or unknown.
    pub fn child(&self, id: &NodeId, index: usize) -> Option<NodeId> {
        self.node(id)?.children.get(index).copied()
    }

    /// The parent of `id`, or `None` for roots and unknown nodes.
    pub fn parent(&self, id: &NodeId) -> Option<NodeId> {
        self.node(id)?.parent
    }

    /// `true` when `id` has no children (or is unknown).
    pub fn is_leaf(&self, id: &NodeId) -> bool {
        TreeListModel::child_count(self, id) == 0
    }

    /// The value a tree view paints for `id`; [`ModelValue::None`] for unknown nodes.
    pub fn data(&self, id: &NodeId) -> ModelValue {
        match self.label(id) {
            Some(label) => ModelValue::Text(label.to_string()),
            None => ModelValue::None,
        }
    }

    /// Whether `id` is expanded; unknown nodes answer `false`.
    pub fn is_expanded(&self, id: &NodeId) -> bool {
        self.node(id).map(TreeNode::expanded).unwrap_or(false)
    }

    /// Sets the expansion state of `id`; `false` for unknown ids or unchanged state.
    pub fn set_expanded(&mut self, id: &NodeId, expanded: bool) -> bool {
        let Some(node) = self.node_mut(id) else {
            return false;
        };
        if node.expanded == expanded {
            return false;
        }
        node.expanded = expanded;
        self.changed.emit(&TreeChange::Expanded {
            node: *id,
            expanded,
        });
        true
    }

    /// Expands or collapses every node; returns the number of nodes whose state changed.
    pub fn set_expanded_all(&mut self, expanded: bool) -> usize {
        let ids: Vec<NodeId> = self.ids().collect();
        let mut changed = 0;
        for id in ids {
            if TreeListModel::set_expanded(self, &id, expanded) {
                changed += 1;
            }
        }
        changed
    }

    /// Expands every node.
    pub fn expand_all(&mut self) -> usize {
        self.set_expanded_all(true)
    }

    /// Collapses every node.
    pub fn collapse_all(&mut self) -> usize {
        self.set_expanded_all(false)
    }

    /// Iterates over the ids of all live nodes, in arena order.
    pub fn ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(i, node)| node.as_ref().map(|_| NodeId(i as u32)))
    }

    /// The number of steps from the root down to `id`; `None` for roots and unknown nodes.
    pub fn depth(&self, id: &NodeId) -> Option<usize> {
        if !self.is_valid(id) {
            return None;
        }
        let mut depth = 0;
        let mut current = *id;
        while let Some(parent) = TreeListModel::parent(self, &current) {
            depth += 1;
            current = parent;
            // Defensive: a corrupted parent chain must not spin forever.
            if depth > self.live {
                return None;
            }
        }
        Some(depth)
    }

    /// The chain of ids from the root down to `id`, inclusive.
    ///
    /// `None` for unknown nodes; this is what a tree view needs to expand every ancestor of the
    /// current selection.
    pub fn path(&self, id: &NodeId) -> Option<Vec<NodeId>> {
        if !self.is_valid(id) {
            return None;
        }
        let mut chain = vec![*id];
        let mut current = *id;
        while let Some(parent) = TreeListModel::parent(self, &current) {
            chain.push(parent);
            current = parent;
            if chain.len() > self.live {
                return None;
            }
        }
        chain.reverse();
        Some(chain)
    }

    /// Expands every ancestor of `id` so that it becomes visible.
    pub fn reveal(&mut self, id: &NodeId) -> bool {
        let Some(chain) = self.path(id) else {
            return false;
        };
        // `chain` ends with `id` itself; every earlier entry is an ancestor that has to be open.
        for ancestor in chain.iter().take(chain.len().saturating_sub(1)) {
            TreeListModel::set_expanded(self, ancestor, true);
        }
        true
    }

    /// Depth-first iterator over the visible nodes, expanding only expanded nodes.
    ///
    /// This mirrors what a `TreeView` paints and is a convenient way to assert on a flattened
    /// tree in tests.
    pub fn visible_nodes(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut queue: VecDeque<NodeId> = self.roots.iter().rev().copied().collect();
        while let Some(id) = queue.pop_front() {
            out.push(id);
            if TreeListModel::is_expanded(self, &id) {
                if let Some(children) = TreeListModel::children(self, &id) {
                    // Push in reverse so that the first child is visited first.
                    for child in children.iter().rev() {
                        queue.push_front(*child);
                    }
                }
            }
        }
        out
    }

    fn slot(&self, id: NodeId) -> Option<&TreeNode> {
        self.nodes.get(id.as_usize()?).and_then(Option::as_ref)
    }
}

impl TreeModel for TreeListModel {
    fn root_count(&self) -> usize {
        self.roots.len()
    }

    fn child_count(&self, node: &NodeId) -> usize {
        TreeListModel::child_count(self, node)
    }

    fn child(&self, node: &NodeId, index: usize) -> Option<NodeId> {
        TreeListModel::child(self, node, index)
    }

    fn parent(&self, node: &NodeId) -> Option<NodeId> {
        TreeListModel::parent(self, node)
    }

    fn data(&self, node: &NodeId) -> ModelValue {
        TreeListModel::data(self, node)
    }

    fn is_expanded(&self, node: &NodeId) -> bool {
        TreeListModel::is_expanded(self, node)
    }

    fn set_expanded(&mut self, node: &NodeId, expanded: bool) {
        TreeListModel::set_expanded(self, node, expanded);
    }
}

// ---- shared handle -------------------------------------------------------
//
// See `Shared<StringListModel>`: the view reads the boxed model, the application edits through
// its own handle to the same value.

impl TreeModel for Shared<TreeListModel> {
    fn root_count(&self) -> usize {
        self.with(|model| model.root_count())
    }

    fn child_count(&self, node: &NodeId) -> usize {
        self.with(|model| TreeListModel::child_count(model, node))
    }

    fn child(&self, node: &NodeId, index: usize) -> Option<NodeId> {
        self.with(|model| TreeListModel::child(model, node, index))
    }

    fn parent(&self, node: &NodeId) -> Option<NodeId> {
        self.with(|model| TreeListModel::parent(model, node))
    }

    fn data(&self, node: &NodeId) -> ModelValue {
        self.with(|model| TreeListModel::data(model, node))
    }

    fn is_expanded(&self, node: &NodeId) -> bool {
        self.with(|model| TreeListModel::is_expanded(model, node))
    }

    fn set_expanded(&mut self, node: &NodeId, expanded: bool) {
        self.with_mut(|model| TreeListModel::set_expanded(model, node, expanded));
    }
}

/// A small helper for assembling a tree from literals.
///
/// ```
/// use rustui_model::{TreeBuilder, TreeModel};
///
/// let mut builder = TreeBuilder::new();
/// let root = builder.root("vcpkg");
/// builder.child(root, "packages");
/// builder.child(root, "triplets");
/// let model = builder.build();
///
/// assert_eq!(model.node_count(), 3);
/// assert_eq!(model.data(&root).as_text(), "vcpkg");
/// assert_eq!(model.child_count(&root), 2);
/// ```
#[derive(Debug, Default, Clone)]
pub struct TreeBuilder {
    model: TreeListModel,
}

impl TreeBuilder {
    /// A builder around an empty tree.
    pub fn new() -> Self {
        TreeBuilder {
            model: TreeListModel::new(),
        }
    }

    /// Appends a root node and returns its id.
    pub fn root(&mut self, label: &str) -> NodeId {
        self.model.append(None, label)
    }

    /// Appends a child to `parent` and returns its id.
    pub fn child(&mut self, parent: NodeId, label: &str) -> NodeId {
        self.model.append(Some(parent), label)
    }

    /// Inserts a node at an explicit position.
    pub fn insert(&mut self, parent: Option<NodeId>, index: usize, label: &str) -> NodeId {
        self.model.insert(parent, index, label)
    }

    /// Borrows the tree being built.
    pub fn model(&self) -> &TreeListModel {
        &self.model
    }

    /// Mutably borrows the tree being built.
    pub fn model_mut(&mut self) -> &mut TreeListModel {
        &mut self.model
    }

    /// Consumes the builder and returns the tree.
    pub fn build(self) -> TreeListModel {
        self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn a_shared_handle_reads_the_model_a_view_owns() {
        let model = Shared::new(TreeListModel::new());
        let root = model.with_mut(|model| model.append(None, "root"));
        let owned: Box<dyn TreeModel> = Box::new(model.clone());

        assert_eq!(owned.root_count(), 1);
        assert_eq!(owned.child_count(&root), 0);

        // The view expands nodes through the boxed model...
        let mut owned = owned;
        owned.set_expanded(&root, true);
        assert!(owned.is_expanded(&root));

        // ...while the application grows the tree through its own handle.
        model.with_mut(|model| {
            model.append(Some(root), "child");
        });
        assert_eq!(owned.child_count(&root), 1);
    }

    fn recorder(model: &TreeListModel) -> Rc<RefCell<Vec<TreeChange>>> {
        let log = Rc::new(RefCell::new(Vec::new()));
        let sink = log.clone();
        model.changed().connect(move |c| sink.borrow_mut().push(*c));
        log
    }

    /// ```
    /// vcpkg
    ///   packages
    ///     fmt
    ///   triplets
    ///     x64-linux
    /// ```
    fn sample() -> (TreeListModel, NodeId, NodeId, NodeId) {
        let mut model = TreeListModel::new();
        let root = model.append(None, "vcpkg");
        let packages = model.append(Some(root), "packages");
        let triplets = model.append(Some(root), "triplets");
        model.append(Some(packages), "fmt");
        model.append(Some(triplets), "x64-linux");
        (model, root, packages, triplets)
    }

    #[test]
    fn empty_tree() {
        let model = TreeListModel::new();
        assert!(model.is_empty());
        assert_eq!(model.node_count(), 0);
        assert_eq!(model.root_count(), 0);
        assert_eq!(model.roots(), &[] as &[NodeId]);
        assert_eq!(model.root(0), None);
        assert!(model.visible_nodes().is_empty());
    }

    #[test]
    fn insert_builds_the_structure() {
        let (model, root, packages, triplets) = sample();
        assert_eq!(model.root_count(), 1);
        assert_eq!(model.node_count(), 5);
        assert_eq!(model.root(0), Some(root));
        assert_eq!(model.label(&root), Some("vcpkg"));
        assert_eq!(model.label(&packages), Some("packages"));
        assert_eq!(model.child_count(&root), 2);
        assert_eq!(model.child(&root, 0), Some(packages));
        assert_eq!(model.child(&root, 1), Some(triplets));
        assert_eq!(model.child(&root, 2), None);
        assert_eq!(model.children(&packages), Some(&[NodeId(3)][..]));
        assert_eq!(model.parent(&packages), Some(root));
        assert_eq!(model.parent(&root), None);
        assert!(!model.is_leaf(&packages));
        assert!(model.is_leaf(&NodeId(3)), "fmt has no children");
        assert!(!model.is_leaf(&root));
        let node_value = match model.node(&packages) {
            Some(node) => node.to_value(),
            None => ModelValue::None,
        };
        assert_eq!(node_value.as_text(), "packages");
    }

    #[test]
    fn insert_respects_the_index_and_clamps_it() {
        let mut model = TreeListModel::new();
        let a = model.append(None, "a");
        let b = model.append(None, "b");
        let c = model.insert(None, 0, "first");
        assert_eq!(model.roots(), &[c, a, b]);
        let d = model.insert(None, 99, "last");
        assert_eq!(model.roots(), &[c, a, b, d]);

        let child_a = model.insert(Some(a), 0, "a0");
        let child_b = model.insert(Some(a), 0, "a1");
        assert_eq!(model.children(&a), Some(&[child_b, child_a][..]));
        let child_c = model.insert(Some(a), 99, "a2");
        assert_eq!(model.child(&a, 2), Some(child_c));
    }

    #[test]
    fn unknown_parent_degrades_to_a_root() {
        let mut model = TreeListModel::new();
        let orphan = model.insert(Some(NodeId(42)), 0, "orphan");
        assert_eq!(model.roots(), &[orphan]);
        assert_eq!(model.parent(&orphan), None);
        let invalid = model.insert(Some(NodeId::INVALID), 0, "invalid parent");
        assert_eq!(model.roots(), &[invalid, orphan]);
    }

    #[test]
    fn unknown_nodes_are_answered_not_panicked() {
        let (mut model, _, _, _) = sample();
        let ghost = NodeId(999);
        assert!(!model.is_valid(&ghost));
        assert!(!model.is_valid(&NodeId::INVALID));
        assert_eq!(model.label(&ghost), None);
        assert_eq!(model.child_count(&ghost), 0);
        assert_eq!(model.child(&ghost, 0), None);
        assert_eq!(model.children(&ghost), None);
        assert_eq!(model.parent(&ghost), None);
        assert!(model.is_leaf(&ghost));
        assert!(!model.is_expanded(&ghost));
        assert!(!model.set_expanded(&ghost, true));
        assert!(!model.set_label(&ghost, "x"));
        assert!(!model.remove(&ghost));
        assert_eq!(TreeListModel::data(&model, &ghost), ModelValue::None);
        assert_eq!(
            TreeListModel::data(&model, &NodeId::INVALID),
            ModelValue::None
        );
    }

    #[test]
    fn labels_can_be_renamed_once() {
        let (mut model, root, packages, _) = sample();
        let log = recorder(&model);
        assert!(model.set_label(&packages, "portages"));
        assert_eq!(model.label(&packages), Some("portages"));
        assert!(
            !model.set_label(&packages, "portages"),
            "unchanged label is silent"
        );
        assert_eq!(
            *log.borrow(),
            vec![TreeChange::LabelChanged { node: packages }]
        );
        assert!(
            !model.set_label(&root, "vcpkg"),
            "unchanged label is silent"
        );
        assert_eq!(model.label(&root), Some("vcpkg"));
    }

    #[test]
    fn remove_drops_the_whole_subtree() {
        let (mut model, root, packages, _) = sample();
        assert_eq!(model.node_count(), 5);
        assert!(model.remove(&packages));
        assert_eq!(model.node_count(), 3, "fmt went away with packages");
        assert!(!model.is_valid(&packages));
        assert!(
            !model.is_valid(&NodeId(3)),
            "descendants are unknown as well"
        );
        assert_eq!(model.child_count(&root), 1);
        assert_eq!(
            model.child(&root, 0).map(|c| model.label(&c)),
            Some(Some("triplets"))
        );
        assert!(!model.remove(&packages), "removing twice is safe");
    }

    #[test]
    fn remove_reports_the_change() {
        let (mut model, root, _, _) = sample();
        let log = recorder(&model);
        assert!(model.remove(&root));
        assert_eq!(*log.borrow(), vec![TreeChange::Removed { node: root }]);
        assert_eq!(model.root_count(), 0);
        assert_eq!(model.node_count(), 0);
    }

    #[test]
    fn remove_keeps_siblings_and_parents() {
        let (mut model, root, packages, triplets) = sample();
        assert!(model.remove(&packages));
        assert!(model.is_valid(&root));
        assert!(model.is_valid(&triplets));
        assert_eq!(model.child_count(&root), 1);
        assert_eq!(model.child(&root, 0), Some(triplets));
        assert_eq!(model.node_count(), 3);
    }

    #[test]
    fn clear_empties_the_tree() {
        let (mut model, root, _, _) = sample();
        let log = recorder(&model);
        model.clear();
        assert!(model.is_empty());
        assert_eq!(model.node_count(), 0);
        assert_eq!(model.roots().len(), 0);
        assert!(!model.is_valid(&root));
        assert_eq!(*log.borrow(), vec![TreeChange::Cleared]);
        model.clear();
        assert_eq!(log.borrow().len(), 1, "clearing an empty tree is silent");
    }

    #[test]
    fn expansion_state_is_per_node() {
        let (mut model, root, packages, _) = sample();
        assert!(!model.is_expanded(&root));
        assert!(model.set_expanded(&root, true));
        assert!(
            !model.set_expanded(&root, true),
            "unchanged state is silent"
        );
        assert!(model.is_expanded(&root));
        assert!(!model.is_expanded(&packages));
        assert!(TreeModel::is_expanded(&model, &root));
        model.set_expanded(&packages, true);
        assert_eq!(model.expanded_count(), 2);
        assert_eq!(model.collapse_all(), 2);
        assert_eq!(model.expanded_count(), 0);
        assert_eq!(model.expand_all(), model.node_count());
        assert_eq!(model.expanded_count(), 5);
        assert_eq!(model.expand_all(), 0, "already expanded");
    }

    #[test]
    fn visible_nodes_follow_the_expansion_state() {
        let (mut model, root, packages, _) = sample();
        assert_eq!(model.visible_nodes(), vec![root]);
        model.set_expanded(&root, true);
        assert_eq!(model.visible_nodes(), vec![root, packages, NodeId(2)]);
        model.set_expanded(&packages, true);
        assert_eq!(
            model.visible_nodes(),
            vec![root, packages, NodeId(3), NodeId(2)]
        );
    }

    #[test]
    fn depth_and_path_describe_the_hierarchy() {
        let (model, root, packages, _) = sample();
        let fmt = NodeId(3);
        assert_eq!(model.depth(&root), Some(0));
        assert_eq!(model.depth(&packages), Some(1));
        assert_eq!(model.depth(&fmt), Some(2));
        assert_eq!(model.depth(&NodeId(999)), None);
        assert_eq!(model.path(&fmt), Some(vec![root, packages, fmt]));
        assert_eq!(model.path(&root), Some(vec![root]));
        assert_eq!(model.path(&NodeId(999)), None);
    }

    #[test]
    fn reveal_expands_every_ancestor() {
        let (mut model, root, packages, _) = sample();
        let fmt = NodeId(3);
        assert!(model.reveal(&fmt));
        assert!(model.is_expanded(&root));
        assert!(model.is_expanded(&packages));
        assert!(!model.is_expanded(&fmt));
        assert!(!model.reveal(&NodeId(999)));
    }

    #[test]
    fn tree_model_trait_is_implemented() {
        let (mut model, root, packages, _) = sample();
        assert_eq!(TreeModel::root_count(&model), 1);
        assert_eq!(TreeModel::child_count(&model, &root), 2);
        assert_eq!(TreeModel::child(&model, &root, 1), Some(NodeId(2)));
        assert_eq!(TreeModel::child(&model, &root, 7), None);
        assert_eq!(TreeModel::parent(&model, &packages), Some(root));
        assert_eq!(TreeModel::data(&model, &packages).as_text(), "packages");
        assert!(!TreeModel::is_leaf(&model, &root));
        assert!(!TreeModel::is_leaf(&model, &packages));
        assert!(TreeModel::is_leaf(&model, &NodeId(3)));
        TreeModel::set_expanded(&mut model, &root, true);
        assert!(TreeModel::is_expanded(&model, &root));
        assert_eq!(TreeModel::child_count(&model, &NodeId::INVALID), 0);
    }

    #[test]
    fn insert_emits_a_change_with_the_parent() {
        let mut model = TreeListModel::new();
        let root = model.append(None, "root");
        let log = recorder(&model);
        let child = model.append(Some(root), "child");
        assert_eq!(
            *log.borrow(),
            vec![TreeChange::Inserted {
                node: child,
                parent: Some(root)
            }]
        );
    }

    #[test]
    fn ids_are_never_reused_after_removal() {
        let mut model = TreeListModel::new();
        let first = model.append(None, "first");
        assert!(model.remove(&first));
        let second = model.append(None, "second");
        assert_ne!(first, second);
        assert!(!model.is_valid(&first));
        assert_eq!(model.label(&second), Some("second"));
        assert_eq!(model.roots(), &[second]);
    }

    #[test]
    fn ids_iterates_over_live_nodes() {
        let (model, root, packages, _) = sample();
        let ids: Vec<NodeId> = model.ids().collect();
        assert_eq!(ids.len(), 5);
        assert!(ids.contains(&root));
        assert!(ids.contains(&packages));
    }

    #[test]
    fn builder_creates_the_tree() {
        let mut builder = TreeBuilder::new();
        let root = builder.root("vcpkg");
        let packages = builder.child(root, "packages");
        builder.child(packages, "fmt");
        builder.insert(None, 0, "first root");
        assert_eq!(builder.model().node_count(), 4);
        assert_eq!(
            builder.model().root(0),
            Some(NodeId(3)),
            "the explicit insert went to the front"
        );
        assert_eq!(builder.model().roots(), &[NodeId(3), root]);
        let mut model = builder.build();
        assert_eq!(model.node_count(), 4);
        assert_eq!(model.label(&root), Some("vcpkg"));
        model.set_label(&root, "vcpkg-registry");
        assert_eq!(model.label(&root), Some("vcpkg-registry"));
    }

    #[test]
    fn debug_output_describes_the_tree() {
        let (model, _, _, _) = sample();
        let text = format!("{:?}", model);
        assert!(text.contains("TreeListModel"));
        assert!(text.contains("nodes"));
        assert!(format!("{:?}", TreeBuilder::new()).contains("TreeBuilder"));
        let node_text = match model.node(&NodeId(0)) {
            Some(node) => format!("{node:?}"),
            None => String::new(),
        };
        assert!(node_text.contains("vcpkg"));
    }
}
