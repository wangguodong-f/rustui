//! Type safe signals and their connections.
//!
//! A [`Signal`] is a list of typed callbacks. Connecting uses `&self` so the ergonomic
//! `button.clicked().connect(..)` API of design document §14 works, which requires a small amount
//! of interior mutability **inside the signal only** — it is not the framework object model
//! (see §52). Emission is re-entrancy safe: the slot list is moved out while callbacks run, so a
//! callback may connect, disconnect or emit again without corrupting the list.

use std::cell::RefCell;
use std::fmt;
use std::rc::{Rc, Weak};

type SlotId = u64;

struct Slot<T> {
    id: SlotId,
    callback: Box<dyn Fn(&T)>,
}

/// Shared signal storage; `Rc` so that a [`Connection`] can keep a *weak* reference and never
/// extend the lifetime of the signal itself (avoiding reference cycles, §14).
struct SignalInner<T> {
    next_id: RefCell<SlotId>,
    slots: RefCell<Vec<Slot<T>>>,
}

impl<T> Default for SignalInner<T> {
    fn default() -> Self {
        Self {
            next_id: RefCell::new(1),
            slots: RefCell::new(Vec::new()),
        }
    }
}

/// A typed signal.
pub struct Signal<T> {
    inner: Rc<SignalInner<T>>,
}

impl<T> Default for Signal<T> {
    fn default() -> Self {
        Self {
            inner: Rc::new(SignalInner::default()),
        }
    }
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<T> Signal<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Connects a callback and returns a handle that can disconnect it later.
    pub fn connect<F>(&self, callback: F) -> Connection<T>
    where
        F: Fn(&T) + 'static,
    {
        let id = {
            let mut next = self.inner.next_id.borrow_mut();
            let id = *next;
            *next += 1;
            id
        };
        self.inner.slots.borrow_mut().push(Slot {
            id,
            callback: Box::new(callback),
        });
        Connection {
            inner: Rc::downgrade(&self.inner),
            id: Some(id),
        }
    }

    /// Connects a callback that is only kept alive while `guard` is alive.
    pub fn connect_scoped<F, G>(&self, guard: G, callback: F) -> Connection<T>
    where
        F: Fn(&T) + 'static,
        G: 'static,
    {
        self.connect(move |v| {
            // `guard` is captured, keeping it alive for as long as the connection exists.
            let _keep_alive = &guard;
            callback(v);
        })
    }

    /// Number of live connections.
    pub fn connection_count(&self) -> usize {
        self.inner.slots.borrow().len()
    }

    /// Removes every connection.
    pub fn disconnect_all(&self) {
        self.inner.slots.borrow_mut().clear();
    }

    /// Invokes every connected callback with `value`.
    ///
    /// Emission is re-entrancy safe:
    ///
    /// * connecting from inside a callback does **not** call the new slot in the same emission
    ///   (Qt-like deferred behaviour);
    /// * disconnecting from inside a callback prevents the remaining slots from running;
    /// * emitting from inside a callback does not re-enter the slot that is already running
    ///   (its callback is parked while it executes).
    pub fn emit(&self, value: &T) {
        let ids: Vec<SlotId> = self.inner.slots.borrow().iter().map(|s| s.id).collect();
        for id in ids {
            // Park the callback so that no `RefCell` borrow is held while user code runs.
            let taken = {
                let mut slots = self.inner.slots.borrow_mut();
                match slots.iter_mut().find(|s| s.id == id) {
                    Some(slot) => Some(std::mem::replace(&mut slot.callback, Box::new(|_| {}))),
                    None => None,
                }
            };
            if let Some(callback) = taken {
                callback(value);
                // Put the callback back unless the slot was disconnected while it was running.
                let mut slots = self.inner.slots.borrow_mut();
                if let Some(slot) = slots.iter_mut().find(|s| s.id == id) {
                    slot.callback = callback;
                }
            }
        }
    }

    /// `true` when `id` still has a connection attached.
    pub fn is_connected(&self, id: SlotId) -> bool {
        self.inner.slots.borrow().iter().any(|s| s.id == id)
    }
}

impl Signal<()> {
    /// Convenience for signals without a payload.
    pub fn emit_unit(&self) {
        self.emit(&());
    }
}

impl<T> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signal")
            .field("connections", &self.connection_count())
            .finish()
    }
}

/// Handle to a single connection. Dropping it does **not** disconnect; call
/// [`Connection::disconnect`].
pub struct Connection<T> {
    inner: Weak<SignalInner<T>>,
    id: Option<SlotId>,
}

impl<T> Connection<T> {
    /// Removes the callback from its signal. Idempotent.
    pub fn disconnect(&mut self) {
        if let Some(id) = self.id.take() {
            if let Some(inner) = self.inner.upgrade() {
                inner.slots.borrow_mut().retain(|s| s.id != id);
            }
        }
    }

    /// `true` while the connection is still registered.
    pub fn is_connected(&self) -> bool {
        match (self.inner.upgrade(), self.id) {
            (Some(inner), Some(id)) => inner.slots.borrow().iter().any(|s| s.id == id),
            _ => false,
        }
    }

    /// Forgets the handle without disconnecting.
    pub fn forget(&mut self) {
        self.id = None;
    }
}

impl<T> Drop for Connection<T> {
    fn drop(&mut self) {
        // Connections deliberately do not auto-disconnect: the observable behaviour of a
        // long-lived signal (e.g. `window.close_requested`) must not change just because a
        // temporary handle went out of scope. Use `disconnect()` or `Signal::disconnect_all`.
    }
}

impl<T> fmt::Debug for Connection<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("id", &self.id)
            .field("connected", &self.is_connected())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn emit_calls_slots_in_order() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let signal: Signal<i32> = Signal::new();
        let l1 = log.clone();
        signal.connect(move |v| l1.borrow_mut().push(*v));
        let l2 = log.clone();
        signal.connect(move |v| l2.borrow_mut().push(*v * 10));
        signal.emit(&2);
        assert_eq!(&*log.borrow(), &[2, 20]);
    }

    #[test]
    fn disconnect_removes_slot() {
        let count = Rc::new(RefCell::new(0));
        let signal: Signal<()> = Signal::new();
        let c = count.clone();
        let mut conn = signal.connect(move |_| *c.borrow_mut() += 1);
        signal.emit_unit();
        conn.disconnect();
        signal.emit_unit();
        assert_eq!(*count.borrow(), 1);
        assert!(!conn.is_connected());
        conn.disconnect(); // idempotent
    }

    #[test]
    fn disconnect_all_from_callback_stops_pending_slots() {
        let signal: Signal<()> = Signal::new();
        let signal2 = signal.clone();
        signal.connect(move |_| signal2.disconnect_all());
        let hits = Rc::new(RefCell::new(0));
        for _ in 0..3 {
            let h = hits.clone();
            signal.connect(move |_| *h.borrow_mut() += 1);
        }
        signal.emit_unit();
        // The killer runs first, so the three counters are never reached.
        assert_eq!(*hits.borrow(), 0);
        signal.emit_unit();
        assert_eq!(*hits.borrow(), 0);
        assert_eq!(signal.connection_count(), 0);
    }

    #[test]
    fn self_disconnect_during_emit_is_safe() {
        let signal: Signal<u32> = Signal::new();
        let hits = Rc::new(RefCell::new(0));
        let holder: Rc<RefCell<Option<Connection<u32>>>> = Rc::new(RefCell::new(None));
        let h = holder.clone();
        let count = hits.clone();
        *holder.borrow_mut() = Some(signal.connect(move |v| {
            *count.borrow_mut() += *v;
            if let Some(conn) = h.borrow_mut().take() {
                let mut conn = conn;
                conn.disconnect();
            }
        }));
        signal.emit(&2);
        signal.emit(&3);
        // only the first emission reached the slot
        assert_eq!(*hits.borrow(), 2);
    }

    #[test]
    fn connecting_from_callback_does_not_recurse() {
        let signal: Signal<i32> = Signal::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s2 = signal.clone();
        let seen2 = seen.clone();
        signal.connect(move |v| {
            seen2.borrow_mut().push(*v);
            if *v < 3 {
                s2.connect(|_| {});
            }
        });
        signal.emit(&1);
        signal.emit(&2);
        signal.emit(&3);
        assert_eq!(&*seen.borrow(), &[1, 2, 3]);
    }

    #[test]
    fn emit_unit_signal() {
        let signal: Signal<()> = Signal::new();
        let hits = Rc::new(RefCell::new(0));
        let h = hits.clone();
        signal.connect(move |_| *h.borrow_mut() += 1);
        signal.emit_unit();
        assert_eq!(*hits.borrow(), 1);
    }
}
