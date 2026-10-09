//! Keyboard focus (§12).
//!
//! Focus is owned per window. [`FocusManager`] only stores the *state*; the actual traversal
//! (Tab order) lives in `rustui-window`, which knows the widget tree.
//!
//! Mutating focus uses `&self` (interior mutability) because widgets receive their contexts as
//! shared references while owning `&mut self`; this keeps event delivery free of aliased borrows.

use std::cell::{Cell, RefCell};

use crate::event::FocusReason;
use crate::object::ObjectId;

/// Tracks which object owns keyboard focus for one window.
#[derive(Debug)]
pub struct FocusManager {
    focused: Cell<Option<ObjectId>>,
    /// Previous focus owner, used to restore focus when a popup closes.
    previous: Cell<Option<ObjectId>>,
    reason: Cell<FocusReason>,
    /// Focus changes waiting to be delivered as `FocusIn`/`FocusOut` events.
    pending: RefCell<Vec<(ObjectId, Option<FocusReason>)>>,
    /// Whether the focus ring is painted around the focused widget.
    show_focus_ring: bool,
}

impl Default for FocusManager {
    fn default() -> Self {
        Self::new()
    }
}

impl FocusManager {
    pub fn new() -> Self {
        Self {
            focused: Cell::new(None),
            previous: Cell::new(None),
            reason: Cell::new(FocusReason::Lost),
            pending: RefCell::new(Vec::new()),
            show_focus_ring: true,
        }
    }

    /// Currently focused object.
    pub fn focused(&self) -> Option<ObjectId> {
        self.focused.get()
    }

    /// The reason the last focus change happened.
    pub fn reason(&self) -> FocusReason {
        self.reason.get()
    }

    /// Changes focus, queuing `FocusOut`/`FocusIn` notifications.
    pub fn set_focus(&self, id: ObjectId, reason: FocusReason) {
        if self.focused.get() == Some(id) {
            return;
        }
        self.previous.set(self.focused.get());
        if let Some(previous) = self.focused.take() {
            self.pending.borrow_mut().push((previous, None));
        }
        self.focused.set(Some(id));
        self.reason.set(reason);
        self.pending.borrow_mut().push((id, Some(reason)));
    }

    /// Removes focus from whatever currently owns it.
    pub fn clear_focus(&self) {
        if let Some(previous) = self.focused.take() {
            self.previous.set(Some(previous));
            self.pending.borrow_mut().push((previous, None));
            self.reason.set(FocusReason::Lost);
        }
    }

    /// Returns to the previously focused object (used when a popup closes).
    pub fn restore_previous(&self) {
        if let Some(previous) = self.previous.take() {
            self.set_focus(previous, FocusReason::Programmatic);
        } else {
            self.clear_focus();
        }
    }

    /// `true` when `id` owns focus.
    pub fn is_focused(&self, id: ObjectId) -> bool {
        self.focused.get() == Some(id)
    }

    /// Takes the queued focus notifications.
    pub fn take_pending(&self) -> Vec<(ObjectId, Option<FocusReason>)> {
        std::mem::take(&mut *self.pending.borrow_mut())
    }

    /// `true` when notifications are waiting to be delivered.
    pub fn has_pending(&self) -> bool {
        !self.pending.borrow().is_empty()
    }

    /// Focus ring visibility.
    pub fn show_focus_ring(&self) -> bool {
        self.show_focus_ring
    }

    pub fn set_show_focus_ring(&mut self, show: bool) {
        self.show_focus_ring = show;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_focus_queues_notifications() {
        let focus = FocusManager::new();
        let a = ObjectId::new(1, 1);
        let b = ObjectId::new(2, 1);
        assert!(focus.focused().is_none());
        focus.set_focus(a, FocusReason::Pointer);
        assert_eq!(focus.focused(), Some(a));
        assert_eq!(focus.take_pending(), vec![(a, Some(FocusReason::Pointer))]);
        focus.set_focus(b, FocusReason::Tab);
        assert_eq!(
            focus.take_pending(),
            vec![(a, None), (b, Some(FocusReason::Tab))]
        );
        focus.clear_focus();
        assert!(focus.focused().is_none());
        assert_eq!(focus.take_pending(), vec![(b, None)]);
    }

    #[test]
    fn set_same_focus_is_noop() {
        let focus = FocusManager::new();
        let a = ObjectId::new(1, 1);
        focus.set_focus(a, FocusReason::Pointer);
        focus.take_pending();
        focus.set_focus(a, FocusReason::Tab);
        assert!(!focus.has_pending());
    }

    #[test]
    fn restore_previous() {
        let focus = FocusManager::new();
        let a = ObjectId::new(1, 1);
        let b = ObjectId::new(2, 1);
        focus.set_focus(a, FocusReason::Pointer);
        focus.set_focus(b, FocusReason::Tab);
        focus.restore_previous();
        assert_eq!(focus.focused(), Some(a));
    }

    #[test]
    fn show_focus_ring_flag() {
        let mut focus = FocusManager::new();
        assert!(focus.show_focus_ring());
        focus.set_show_focus_ring(false);
        assert!(!focus.show_focus_ring());
    }
}
