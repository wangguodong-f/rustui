//! Storage for widget payloads.
//!
//! The object registry owns identity and tree relationships; the widget store owns the trait
//! objects. The store lives in `rustui-core` because the *layout, event and paint contexts* must
//! be able to reach a widget's children (§6) without depending on the windowing layer.
//!
//! ## Why interior mutability?
//!
//! Design document §52 forbids `Rc<RefCell<T>>` as the *object model*. Traversal, however,
//! needs a way to hand a `&mut dyn Widget` to a widget while its siblings and children are still
//! reachable (a parent measures a child, a child forwards an event to its own child, ...).
//!
//! `WidgetStore` solves this with a **scoped take/put** protocol: [`WidgetStore::take`] *moves*
//! the payload out and immediately drops the `RefMut`, so no borrow is ever held across a call
//! into user code. `put` gives it back. Because the entry is removed, a re-entrant call for the
//! same widget returns `None` instead of panicking — the failure mode is a no-op, never a `RefCell`
//! double-borrow panic.

use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;

use crate::object::{ObjectError, ObjectId, ObjectRegistry};
use crate::widget::Widget;

/// Owns every live widget of one UI tree (typically one window).
#[derive(Debug, Default)]
pub struct WidgetStore {
    inner: RefCell<HashMap<ObjectId, Box<dyn Widget>>>,
}

impl WidgetStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `widget`, assigning a fresh id from `registry`.
    pub fn insert(&self, registry: &mut ObjectRegistry, widget: Box<dyn Widget>) -> ObjectId {
        let id = registry.create();
        self.attach(registry, id, widget);
        id
    }

    /// Registers `widget` under an explicit `id`.
    pub fn attach(
        &self,
        _registry: &mut ObjectRegistry,
        id: ObjectId,
        mut widget: Box<dyn Widget>,
    ) {
        let kind = widget_kind(widget.as_ref());
        widget.base_mut().set_id(id);
        widget.base_mut().set_kind(kind);
        self.inner.borrow_mut().insert(id, widget);
    }

    pub fn contains(&self, id: ObjectId) -> bool {
        self.inner.borrow().contains_key(&id)
    }

    pub fn get(&self, id: ObjectId) -> Option<Ref<'_, dyn Widget>> {
        Ref::filter_map(
            self.inner.borrow(),
            |map: &HashMap<ObjectId, Box<dyn Widget>>| map.get(&id).map(|b| b.as_ref()),
        )
        .ok()
    }

    pub fn get_mut(&self, id: ObjectId) -> Option<RefMut<'_, dyn Widget>> {
        RefMut::filter_map(
            self.inner.borrow_mut(),
            |map: &mut HashMap<ObjectId, Box<dyn Widget>>| map.get_mut(&id).map(|b| b.as_mut()),
        )
        .ok()
    }

    /// Moves the payload out of the store.
    pub fn take(&self, id: ObjectId) -> Option<Box<dyn Widget>> {
        self.inner.borrow_mut().remove(&id)
    }

    /// Puts a payload previously obtained from [`WidgetStore::take`] back.
    pub fn put(&self, id: ObjectId, widget: Box<dyn Widget>) {
        self.inner.borrow_mut().insert(id, widget);
    }

    pub fn remove(&self, id: ObjectId) -> Option<Box<dyn Widget>> {
        self.inner.borrow_mut().remove(&id)
    }

    /// Detaches `id` from its parent (in `registry`) and destroys it together with its subtree.
    ///
    /// Returns the ids of every destroyed object, children before parents.
    pub fn destroy_subtree(&self, registry: &mut ObjectRegistry, id: ObjectId) -> Vec<ObjectId> {
        let mut removed = Vec::new();
        self.destroy_recursive(id, registry, &mut removed);
        removed
    }

    fn destroy_recursive(
        &self,
        id: ObjectId,
        registry: &mut ObjectRegistry,
        removed: &mut Vec<ObjectId>,
    ) {
        let widget = match self.take(id) {
            Some(w) => w,
            None => return,
        };
        let children: Vec<ObjectId> = widget.children().to_vec();
        drop(widget);
        for child in children {
            self.destroy_recursive(child, registry, removed);
        }
        registry.set_parent(id, None).ok();
        if registry.destroy(id).is_some() {
            removed.push(id);
        }
    }

    /// Removes a single child from its parent widget and destroys its subtree.
    pub fn remove_child(&self, registry: &mut ObjectRegistry, parent: ObjectId, child: ObjectId) {
        if let Some(mut widget) = self.take(parent) {
            widget.base_mut().remove_child(child);
            self.put(parent, widget);
        }
        registry.set_parent(child, None).ok();
        self.destroy_subtree(registry, child);
    }

    /// Number of live widgets.
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    /// Ids of every widget, in unspecified order.
    pub fn ids(&self) -> Vec<ObjectId> {
        self.inner.borrow().keys().copied().collect()
    }

    /// Removes every widget.
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    /// Collects the ids of the subtree rooted at `id` (pre order, including `id`).
    pub fn subtree(&self, id: ObjectId) -> Vec<ObjectId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            if self.contains(current) {
                out.push(current);
                if let Some(widget) = self.get(current) {
                    let children = widget.children().to_vec();
                    drop(widget);
                    stack.extend(children);
                }
            }
        }
        out
    }

    /// Depth-first walk in paint order, calling `f(id, depth)` for every widget.
    pub fn for_each_in_order(&self, id: ObjectId, f: &mut impl FnMut(ObjectId, usize)) {
        let mut stack = vec![(id, 0usize)];
        while let Some((current, depth)) = stack.pop() {
            let children = match self.get(current) {
                Some(widget) => {
                    f(current, depth);
                    let children = widget.children().to_vec();
                    drop(widget);
                    children
                }
                None => continue,
            };
            for child in children.into_iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
}

fn widget_kind(widget: &dyn Widget) -> &'static str {
    widget.kind()
}

/// Detaches `child` from whichever parent currently owns it. Returns the previous parent.
pub fn detach_from_parent(
    store: &WidgetStore,
    registry: &mut ObjectRegistry,
    child: ObjectId,
) -> Option<ObjectId> {
    let parent = registry.parent(child)?;
    if let Some(mut widget) = store.take(parent) {
        widget.base_mut().remove_child(child);
        store.put(parent, widget);
    }
    registry.set_parent(child, None).ok();
    Some(parent)
}

/// Ensures every child of `parent` is registered with `parent` in the registry.
pub fn link_children(
    store: &WidgetStore,
    registry: &mut ObjectRegistry,
    parent: ObjectId,
) -> Result<(), ObjectError> {
    let children = match store.get(parent) {
        Some(widget) => widget.children().to_vec(),
        None => return Ok(()),
    };
    for child in children {
        registry.set_parent(child, Some(parent))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::RootWidget;
    use crate::{
        impl_widget_base, ArrangeContext, EventContext, EventResult, MeasureContext, PaintContext,
        UiEvent, WidgetBase,
    };

    #[derive(Debug, Default)]
    struct Leaf {
        base: WidgetBase,
        label: String,
    }

    impl Leaf {
        fn new(label: &str) -> Self {
            Self {
                base: WidgetBase::new(),
                label: label.to_string(),
            }
        }
    }

    impl_widget_base!(Leaf);

    impl Widget for Leaf {
        fn kind(&self) -> &'static str {
            "Leaf"
        }

        fn measure(
            &mut self,
            _ctx: &MeasureContext<'_>,
            _available: crate::geometry::Size,
        ) -> crate::geometry::Size {
            self.base
                .set_measured(crate::geometry::Size::new(10.0, 10.0));
            self.base.measured()
        }

        fn arrange(&mut self, _ctx: &ArrangeContext<'_>, rect: crate::geometry::Rect) {
            self.base.set_bounds(rect);
        }

        fn event(&mut self, _ctx: &mut EventContext<'_>, _event: &UiEvent) -> EventResult {
            EventResult::Unhandled
        }

        fn paint(&mut self, ctx: &mut PaintContext<'_>) {
            ctx.fill_rect(self.base.bounds(), crate::Color::RED);
        }
    }

    fn store_with_tree() -> (WidgetStore, ObjectRegistry, ObjectId) {
        let mut registry = ObjectRegistry::new();
        let store = WidgetStore::new();
        let root = store.insert(&mut registry, Box::new(RootWidget::new()));
        let a = store.insert(&mut registry, Box::new(Leaf::new("a")));
        let b = store.insert(&mut registry, Box::new(Leaf::new("b")));
        {
            let mut widget = store.take(root).unwrap();
            widget.base_mut().add_child(a);
            widget.base_mut().add_child(b);
            store.put(root, widget);
        }
        link_children(&store, &mut registry, root).unwrap();
        (store, registry, root)
    }

    #[test]
    fn insert_assigns_ids_and_kind() {
        let mut registry = ObjectRegistry::new();
        let store = WidgetStore::new();
        let leaf = Leaf::new("x");
        assert_eq!(leaf.label, "x");
        let id = store.insert(&mut registry, Box::new(leaf));
        assert!(registry.is_alive(id));
        assert!(store.contains(id));
        assert_eq!(store.get(id).unwrap().kind(), "Leaf");
        assert_eq!(store.get(id).unwrap().id(), id);
    }

    #[test]
    fn take_put_roundtrip() {
        let (_store, _registry, root) = store_with_tree();
        assert!(_store.take(root).is_some());
        // taking twice is a no-op, never a panic
        assert!(_store.take(root).is_none());
        _store.put(root, Box::new(RootWidget::new()));
        assert!(_store.take(root).is_some());
    }

    #[test]
    fn destroy_subtree_is_post_order() {
        let (store, mut registry, root) = store_with_tree();
        let removed = store.destroy_subtree(&mut registry, root);
        assert_eq!(removed.len(), 3);
        assert_eq!(*removed.last().unwrap(), root);
        assert!(store.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn subtree_walks_children() {
        let (store, _registry, root) = store_with_tree();
        assert_eq!(store.subtree(root).len(), 3);
        let mut seen = Vec::new();
        store.for_each_in_order(root, &mut |id, depth| seen.push((id, depth)));
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].0, root);
        assert_eq!(seen[0].1, 0);
        assert_eq!(seen[1].1, 1);
    }

    #[test]
    fn detach_removes_from_parent() {
        let (store, mut registry, root) = store_with_tree();
        let child = store.get(root).unwrap().children()[0];
        let parent = detach_from_parent(&store, &mut registry, child);
        assert_eq!(parent, Some(root));
        assert_eq!(store.get(root).unwrap().children().len(), 1);
        assert_eq!(registry.parent(child), None);
    }
}
