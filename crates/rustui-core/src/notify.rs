//! Frame scheduling helpers.
//!
//! Widgets need to ask for a repaint without having a reference to the application (a property may
//! change from anywhere on the UI thread). The UI thread therefore keeps a *redraw flag* here,
//! installed by [`set_redraw_sink`] when the application starts.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A shared "please repaint" flag.
#[derive(Debug, Clone, Default)]
pub struct RedrawFlag {
    flag: Arc<AtomicBool>,
}

impl RedrawFlag {
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests a repaint.
    #[inline]
    pub fn request(&self) {
        self.flag.store(true, Ordering::Release);
    }

    /// Returns `true` once per request.
    #[inline]
    pub fn take(&self) -> bool {
        self.flag.swap(false, Ordering::AcqRel)
    }

    /// Reads without clearing.
    #[inline]
    pub fn is_pending(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

thread_local! {
    static REDRAW_SINK: RefCell<Option<RedrawFlag>> = const { RefCell::new(None) };
}

/// Installs the flag used by [`request_redraw`]. Called by `Application` during startup.
pub fn set_redraw_sink(flag: RedrawFlag) {
    REDRAW_SINK.with(|slot| *slot.borrow_mut() = Some(flag));
}

/// Removes the installed redraw sink.
pub fn clear_redraw_sink() {
    REDRAW_SINK.with(|slot| *slot.borrow_mut() = None);
}

/// Requests a repaint of the UI thread (no-op when no application is running).
pub fn request_redraw() {
    REDRAW_SINK.with(|slot| {
        if let Some(flag) = slot.borrow().as_ref() {
            flag.request();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_take_is_edge_triggered() {
        let flag = RedrawFlag::new();
        assert!(!flag.take());
        flag.request();
        assert!(flag.take());
        assert!(!flag.take());
    }

    #[test]
    fn request_redraw_without_sink_is_safe() {
        clear_redraw_sink();
        request_redraw();
        let flag = RedrawFlag::new();
        set_redraw_sink(flag.clone());
        request_redraw();
        assert!(flag.take());
        clear_redraw_sink();
    }
}
