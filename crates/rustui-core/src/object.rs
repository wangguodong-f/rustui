//! Stable, generational object identifiers and the object registry.
//!
//! RustUI deliberately avoids the C++ `QObject*` model: objects are addressed by an
//! [`ObjectId`] and stored in a central [`ObjectRegistry`]. Ids are *generational*, so an id that
//! refers to a destroyed object is never silently reused for a different object (see design
//! document §4, §52).

use std::fmt;

/// Handle to an object living inside an [`ObjectRegistry`].
///
/// The identifier is `index + generation`, which makes stale handles detectable: after a slot is
/// released its generation is bumped and any old [`ObjectId`] of that slot becomes invalid
/// instead of aliasing a brand new object.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(C)]
pub struct ObjectId {
    index: u32,
    generation: u32,
}

impl ObjectId {
    /// Index of the reserved slot used for "no object".
    pub const INVALID_INDEX: u32 = u32::MAX;
    /// The invalid / null object id.
    pub const INVALID: ObjectId = ObjectId {
        index: Self::INVALID_INDEX,
        generation: 0,
    };

    #[inline]
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    #[inline]
    pub const fn index(self) -> u32 {
        self.index
    }

    #[inline]
    pub const fn generation(self) -> u32 {
        self.generation
    }

    #[inline]
    pub const fn is_valid(self) -> bool {
        self.index != Self::INVALID_INDEX
    }

    /// Compact representation useful for logging / serialisation.
    #[inline]
    pub const fn to_bits(self) -> u64 {
        ((self.generation as u64) << 32) | self.index as u64
    }

    #[inline]
    pub const fn from_bits(bits: u64) -> Self {
        ObjectId::new(bits as u32, (bits >> 32) as u32)
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_valid() {
            write!(f, "#{}v{}", self.index, self.generation)
        } else {
            write!(f, "#invalid")
        }
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[derive(Debug, Clone)]
struct Slot {
    generation: u32,
    parent: Option<ObjectId>,
    name: Option<Box<str>>,
    /// Number of children, kept for O(1) queries.
    child_count: usize,
}

/// Central storage for the object tree.
///
/// The registry owns identity and tree relationships (parent / children). It deliberately does
/// *not* own the widget payloads; those live in a
/// [`WidgetStore`](crate::widget_store::WidgetStore) so that the two concerns stay separated.
#[derive(Debug, Default)]
pub struct ObjectRegistry {
    slots: Vec<Slot>,
    free: Vec<u32>,
    alive: usize,
    root: Vec<ObjectId>,
}

impl ObjectRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new root object and returns its id.
    pub fn create(&mut self) -> ObjectId {
        self.alloc(None)
    }

    /// Creates a new object attached to `parent`.
    ///
    /// Returns `None` if the parent is not alive.
    pub fn create_child(&mut self, parent: ObjectId) -> Option<ObjectId> {
        if !self.is_alive(parent) {
            return None;
        }
        Some(self.alloc(Some(parent)))
    }

    fn alloc(&mut self, parent: Option<ObjectId>) -> ObjectId {
        let id = match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.parent = parent;
                slot.child_count = 0;
                slot.name = None;
                ObjectId::new(index, slot.generation)
            }
            None => {
                let index = self.slots.len() as u32;
                self.slots.push(Slot {
                    generation: 1,
                    parent,
                    name: None,
                    child_count: 0,
                });
                ObjectId::new(index, 1)
            }
        };
        self.alive += 1;
        if let Some(p) = parent {
            self.slots[p.index() as usize].child_count += 1;
        } else {
            self.root.push(id);
        }
        id
    }

    /// `true` when `id` refers to a live slot.
    #[inline]
    pub fn is_alive(&self, id: ObjectId) -> bool {
        id.is_valid()
            && self
                .slots
                .get(id.index() as usize)
                .map(|s| s.generation == id.generation())
                .unwrap_or(false)
    }

    /// Number of live objects.
    #[inline]
    pub fn len(&self) -> usize {
        self.alive
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.alive == 0
    }

    /// Total number of allocated slots (including free ones).
    #[inline]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Attaches `child` to `parent`, detaching it from its previous parent first.
    ///
    /// Returns the new parent on success. Attaching an object to one of its own descendants is
    /// rejected to keep the tree acyclic.
    pub fn set_parent(
        &mut self,
        child: ObjectId,
        parent: Option<ObjectId>,
    ) -> Result<(), ObjectError> {
        if !self.is_alive(child) {
            return Err(ObjectError::InvalidObject(child));
        }
        if let Some(p) = parent {
            if !self.is_alive(p) {
                return Err(ObjectError::InvalidObject(p));
            }
            if p == child {
                return Err(ObjectError::Cycle(child));
            }
            if self.is_ancestor_of(child, p) {
                return Err(ObjectError::Cycle(child));
            }
        }
        if self.slot(child)?.parent == parent {
            return Ok(());
        }
        if let Some(old) = self.slot(child)?.parent {
            self.slots[old.index() as usize].child_count -= 1;
        } else {
            self.root.retain(|id| *id != child);
        }
        self.slots[child.index() as usize].parent = parent;
        match parent {
            Some(p) => self.slots[p.index() as usize].child_count += 1,
            None => self.root.push(child),
        }
        Ok(())
    }

    /// Parent of `id`, `None` for roots, invalid ids and dangling links.
    ///
    /// A dangling link happens when a parent is destroyed without its children; the generation
    /// check turns such a link into `None` instead of an aliased parent (§52).
    #[inline]
    pub fn parent(&self, id: ObjectId) -> Option<ObjectId> {
        let slot = self.slots.get(id.index() as usize)?;
        if slot.generation != id.generation() {
            return None;
        }
        let parent = slot.parent?;
        self.is_alive(parent).then_some(parent)
    }

    /// Number of direct children.
    #[inline]
    pub fn child_count(&self, id: ObjectId) -> usize {
        if !self.is_alive(id) {
            return 0;
        }
        self.slots[id.index() as usize].child_count
    }

    /// `true` when `ancestor` is a strict ancestor of `id`.
    pub fn is_ancestor_of(&self, ancestor: ObjectId, id: ObjectId) -> bool {
        let mut current = id;
        let mut guard = 0;
        while let Some(parent) = self.parent(current) {
            if parent == ancestor {
                return true;
            }
            current = parent;
            guard += 1;
            if guard > self.slots.len() {
                break;
            }
        }
        false
    }

    /// Root ancestor of `id`.
    pub fn root_of(&self, id: ObjectId) -> Option<ObjectId> {
        if !self.is_alive(id) {
            return None;
        }
        let mut current = id;
        while let Some(parent) = self.parent(current) {
            current = parent;
        }
        Some(current)
    }

    /// Depth of `id` in the tree (roots have depth 0).
    pub fn depth_of(&self, id: ObjectId) -> usize {
        let mut depth = 0;
        let mut current = id;
        while let Some(parent) = self.parent(current) {
            current = parent;
            depth += 1;
            if depth > self.slots.len() {
                break;
            }
        }
        depth
    }

    /// Ids of all live root objects.
    pub fn roots(&self) -> &[ObjectId] {
        &self.root
    }

    /// Assigns a debug name to an object.
    pub fn set_name(&mut self, id: ObjectId, name: impl Into<Box<str>>) -> Result<(), ObjectError> {
        if !self.is_alive(id) {
            return Err(ObjectError::InvalidObject(id));
        }
        self.slots[id.index() as usize].name = Some(name.into());
        Ok(())
    }

    /// Debug name of an object, if any.
    pub fn name(&self, id: ObjectId) -> Option<&str> {
        if !self.is_alive(id) {
            return None;
        }
        self.slots[id.index() as usize].name.as_deref()
    }

    /// Removes `id` from the registry and returns the slots to release.
    ///
    /// Only the object itself is released here: the *children* of an object are owned by its
    /// widget payload (`Widget::children`), so subtree destruction is driven by
    /// [`crate::widget_store::WidgetStore::destroy_subtree`] which walks both structures.
    pub fn destroy(&mut self, id: ObjectId) -> Option<ObjectId> {
        if !self.is_alive(id) {
            return None;
        }
        if let Some(parent) = self.parent(id) {
            self.slots[parent.index() as usize].child_count -= 1;
        } else {
            self.root.retain(|r| *r != id);
        }
        let slot = &mut self.slots[id.index() as usize];
        slot.parent = None;
        slot.name = None;
        slot.child_count = 0;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free.push(id.index());
        self.alive -= 1;
        Some(id)
    }

    /// Removes every object.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.free.clear();
        self.root.clear();
        self.alive = 0;
    }

    #[inline]
    fn slot(&self, id: ObjectId) -> Result<&Slot, ObjectError> {
        self.slots
            .get(id.index() as usize)
            .filter(|s| s.generation == id.generation())
            .ok_or(ObjectError::InvalidObject(id))
    }
}

/// Errors produced by the object registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectError {
    /// The object id does not refer to a live object.
    InvalidObject(ObjectId),
    /// The operation would create a cycle in the object tree.
    Cycle(ObjectId),
}

impl fmt::Display for ObjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjectError::InvalidObject(id) => write!(f, "invalid object {id:?}"),
            ObjectError::Cycle(id) => write!(f, "operation would create a cycle at {id:?}"),
        }
    }
}

impl std::error::Error for ObjectError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generational_ids_are_not_reused() {
        let mut reg = ObjectRegistry::new();
        let a = reg.create();
        assert!(reg.is_alive(a));
        reg.destroy(a);
        assert!(!reg.is_alive(a));
        let b = reg.create();
        // Same slot, different generation: the old handle stays invalid.
        assert_eq!(a.index(), b.index());
        assert_ne!(a.generation(), b.generation());
        assert!(!reg.is_alive(a));
        assert!(reg.is_alive(b));
    }

    #[test]
    fn parent_child_bookkeeping() {
        let mut reg = ObjectRegistry::new();
        let root = reg.create();
        let child = reg.create_child(root).unwrap();
        let grand = reg.create_child(child).unwrap();
        assert_eq!(reg.parent(grand), Some(child));
        assert_eq!(reg.child_count(root), 1);
        assert_eq!(reg.depth_of(grand), 2);
        assert!(reg.is_ancestor_of(root, grand));
        assert_eq!(reg.root_of(grand), Some(root));

        reg.set_parent(grand, None).unwrap();
        assert_eq!(reg.child_count(child), 0);
        assert!(reg.roots().contains(&grand));
    }

    #[test]
    fn cycles_are_rejected() {
        let mut reg = ObjectRegistry::new();
        let a = reg.create();
        let b = reg.create_child(a).unwrap();
        assert_eq!(reg.set_parent(a, Some(b)), Err(ObjectError::Cycle(a)));
        assert_eq!(reg.set_parent(a, Some(a)), Err(ObjectError::Cycle(a)));
    }

    #[test]
    fn destroy_detaches_from_parent() {
        let mut reg = ObjectRegistry::new();
        let a = reg.create();
        let b = reg.create_child(a).unwrap();
        assert_eq!(reg.destroy(a), Some(a));
        // `b` keeps a stale link to the freed slot; generation checks turn it into "no parent".
        assert_eq!(reg.parent(b), None);
        assert_eq!(reg.child_count(a), 0);
        assert_eq!(reg.destroy(a), None);
    }

    #[test]
    fn id_bits_roundtrip() {
        let id = ObjectId::new(7, 3);
        assert_eq!(ObjectId::from_bits(id.to_bits()), id);
        assert!(!ObjectId::INVALID.is_valid());
    }
}
