//! Reactive values.
//!
//! [`Property`] is a value plus a `changed` signal (§13). In addition to the plain property,
//! [`SharedProperty`] provides a cheaply cloneable, `Rc` based property that user code can mutate
//! from anywhere on the UI thread — which is how widgets are wired to application state without
//! pulling `Rc<RefCell<Widget>>` into the object model.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::signal::Signal;

/// A value that notifies observers when it changes.
pub struct Property<T> {
    value: T,
    changed: Signal<T>,
}

impl<T> Property<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            changed: Signal::new(),
        }
    }

    /// Current value.
    #[inline]
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Mutable access to the value *without* emitting `changed`.
    ///
    /// Prefer [`Property::set`] whenever observers should be notified.
    #[inline]
    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// Signal emitted after every change, carrying the new value.
    #[inline]
    pub fn changed(&self) -> &Signal<T> {
        &self.changed
    }

    /// Consumes the property and returns the signal it was emitting on.
    pub fn into_changed(self) -> Signal<T> {
        self.changed
    }
}

impl<T: Clone> Property<T> {
    /// Cloned copy of the current value.
    pub fn get_cloned(&self) -> T {
        self.value.clone()
    }

    /// Sets a new value and emits `changed` with it.
    pub fn set(&mut self, value: T) {
        self.value = value;
        self.changed.emit(&self.value);
    }

    /// Applies a function to the current value.
    pub fn update(&mut self, f: impl FnOnce(&mut T)) {
        f(&mut self.value);
        self.changed.emit(&self.value);
    }
}

impl<T: Default> Default for Property<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Clone + fmt::Debug> fmt::Debug for Property<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Property")
            .field("value", &self.value)
            .field("connections", &self.changed.connection_count())
            .finish()
    }
}

impl<T: Clone> Clone for Property<T> {
    fn clone(&self) -> Self {
        // The value is copied; connections are *not* cloned (signals are not part of the value).
        Self {
            value: self.value.clone(),
            changed: Signal::new(),
        }
    }
}

/// A reference-counted property that widgets and application code can share.
///
/// This is the user-facing escape hatch for the widget tree: a widget holds a
/// [`SharedProperty`], the application holds a clone, and both observe the same value and the same
/// `changed` signal.
pub struct SharedProperty<T> {
    inner: Rc<SharedPropertyInner<T>>,
}

struct SharedPropertyInner<T> {
    value: RefCell<T>,
    changed: Signal<T>,
}

impl<T> SharedProperty<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Rc::new(SharedPropertyInner {
                value: RefCell::new(value),
                changed: Signal::new(),
            }),
        }
    }

    /// Current value (a copy).
    #[inline]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.inner.value.borrow().clone()
    }

    /// Borrows the value without copying.
    #[inline]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.inner.value.borrow())
    }

    /// Mutates the value *without* notifying observers.
    #[inline]
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        f(&mut self.inner.value.borrow_mut())
    }

    /// Signal emitted after every change.
    #[inline]
    pub fn changed(&self) -> &Signal<T> {
        &self.inner.changed
    }
}

impl<T: Clone> SharedProperty<T> {
    /// Sets a new value and notifies observers.
    pub fn set(&self, value: T) {
        *self.inner.value.borrow_mut() = value;
        let value = self.inner.value.borrow().clone();
        self.inner.changed.emit(&value);
        crate::notify::request_redraw();
    }

    /// Applies a function to the current value and notifies observers.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        f(&mut self.inner.value.borrow_mut());
        let value = self.inner.value.borrow().clone();
        self.inner.changed.emit(&value);
        crate::notify::request_redraw();
    }
}

impl<T: Clone + fmt::Debug> fmt::Debug for SharedProperty<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedProperty")
            .field("value", &self.inner.value.borrow())
            .field("strong_count", &Rc::strong_count(&self.inner))
            .finish()
    }
}

impl<T> Clone for SharedProperty<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<T: Default> Default for SharedProperty<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// A plain shared value with change notification, handy for application state.
pub struct Shared<T> {
    inner: SharedProperty<T>,
}

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Shared {
            inner: SharedProperty::new(value),
        }
    }

    #[inline]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.inner.get()
    }

    #[inline]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.inner.with(f)
    }

    #[inline]
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        self.inner.with_mut(f)
    }

    #[inline]
    pub fn changed(&self) -> &Signal<T> {
        self.inner.changed()
    }
}

impl<T: Clone> Shared<T> {
    pub fn set(&self, value: T) {
        self.inner.set(value);
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Shared {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Clone + fmt::Debug> fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shared")
            .field("value", &self.inner.get())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn property_emits_changed() {
        let mut p = Property::new(1u32);
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        p.changed().connect(move |v| l.borrow_mut().push(*v));
        p.set(5);
        assert_eq!(*p.get(), 5);
        assert_eq!(&*log.borrow(), &[5]);
    }

    #[test]
    fn shared_property_is_visible_to_all_clones() {
        let p = SharedProperty::new(String::from("hello"));
        let q = p.clone();
        let seen = Rc::new(RefCell::new(String::new()));
        let s = seen.clone();
        p.changed().connect(move |v| *s.borrow_mut() = v.clone());
        q.set(String::from("world"));
        assert_eq!(p.get(), "world");
        assert_eq!(*seen.borrow(), "world");
    }

    #[test]
    fn shared_property_update() {
        let p = Shared::<u32>::new(1);
        let q = p.clone();
        p.changed().connect(move |v| q.set(v * 10));
        p.set(4);
        assert_eq!(p.get(), 40);
    }
}
