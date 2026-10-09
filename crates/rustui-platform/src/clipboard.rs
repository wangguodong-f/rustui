//! Clipboard access (§39 of the detailed design document).
//!
//! The [`Clipboard`] trait lives in `rustui-core` so widgets never depend on this crate; the
//! implementation lives here (T-3 / C-054).
//!
//! # Backing stores
//!
//! * **System clipboard** - `arboard` talks to the platform clipboard (X11/XWayland on Linux,
//!   native APIs elsewhere). This is the one other applications see.
//! * **In-process fallback** - when no system clipboard can be opened (headless test runs,
//!   a pure Wayland session without XWayland, ...) the implementation degrades to a buffer
//!   shared by every window of the process - the behaviour RustUI had before T-3.
//!
//! `arboard` is a platform-boundary dependency: it is allowed here and only here, never in
//! `rustui-core` or the widgets (see C-054 in `docs/Design_Corrections.md`).

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use rustui_core::Clipboard;

/// Where the clipboard text actually lives.
enum Backing {
    /// The platform clipboard, opened once and kept for the lifetime of the application so
    /// other processes can keep pasting from us.
    System(arboard::Clipboard),
    /// In-process buffer (graceful fallback, tests, headless mode).
    Memory(Option<String>),
}

/// Clipboard shared by every window of the process, backed by the system clipboard when one
/// is available and by an in-memory string otherwise.
#[derive(Clone)]
pub struct PlatformClipboard {
    backing: Rc<RefCell<Backing>>,
}

impl PlatformClipboard {
    /// Opens the system clipboard, falling back to an in-process buffer when none exists
    /// (no display server, no XWayland on Wayland, clipboard locked by another process, ...).
    pub fn new() -> Self {
        match arboard::Clipboard::new() {
            Ok(clipboard) => {
                log::debug!("system clipboard opened");
                Self {
                    backing: Rc::new(RefCell::new(Backing::System(clipboard))),
                }
            }
            Err(error) => {
                log::warn!("system clipboard unavailable ({error}); using an in-process buffer");
                Self::in_memory()
            }
        }
    }

    /// The in-process fallback, without touching the system clipboard (tests, headless mode).
    pub fn in_memory() -> Self {
        Self {
            backing: Rc::new(RefCell::new(Backing::Memory(None))),
        }
    }

    /// `true` when this clipboard exchanges text with other applications through the platform
    /// clipboard; `false` for the in-process fallback.
    pub fn is_system(&self) -> bool {
        matches!(*self.backing.borrow(), Backing::System(_))
    }
}

impl Default for PlatformClipboard {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl fmt::Debug for PlatformClipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.backing.borrow() {
            Backing::System(_) => f.debug_tuple("PlatformClipboard").field(&"system").finish(),
            Backing::Memory(text) => f
                .debug_tuple("PlatformClipboard")
                .field(&"memory")
                .field(text)
                .finish(),
        }
    }
}

impl Clipboard for PlatformClipboard {
    fn get_text(&self) -> Option<String> {
        let mut backing = self.backing.borrow_mut();
        match &mut *backing {
            Backing::System(clipboard) => match clipboard.get_text() {
                Ok(text) => Some(text),
                Err(error) => {
                    // No text on the clipboard (or the owner does not serve UTF-8) is not an
                    // error condition for the UI - it just means "nothing to paste".
                    log::debug!("clipboard read: {error}");
                    None
                }
            },
            Backing::Memory(text) => text.clone(),
        }
    }

    fn set_text(&self, text: &str) {
        let mut backing = self.backing.borrow_mut();
        match &mut *backing {
            Backing::System(clipboard) => {
                if let Err(error) = clipboard.set_text(text) {
                    log::warn!(
                        "clipboard write failed ({error}); degrading to an in-process buffer"
                    );
                    *backing = Backing::Memory(Some(text.to_string()));
                }
            }
            Backing::Memory(slot) => *slot = Some(text.to_string()),
        }
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The in-process fallback behaves exactly like the pre-T-3 clipboard and is shared
    /// between clones.
    #[test]
    fn in_memory_round_trips_and_is_shared_between_clones() {
        let clipboard = PlatformClipboard::in_memory();
        assert!(!clipboard.is_system());
        assert_eq!(clipboard.get_text(), None);

        clipboard.set_text("hello");
        assert_eq!(clipboard.get_text().as_deref(), Some("hello"));

        let other = clipboard.clone();
        assert_eq!(other.get_text().as_deref(), Some("hello"));
        other.set_text("world");
        assert_eq!(clipboard.get_text().as_deref(), Some("world"));
    }

    /// Opening the clipboard never panics, whatever the environment provides. Only probes
    /// opening - content operations live in the single system test below so concurrent tests
    /// cannot race over the one global platform clipboard.
    #[test]
    fn new_never_panics() {
        let clipboard = PlatformClipboard::new();
        assert!(clipboard.is_available());
        let memory = PlatformClipboard::in_memory();
        memory.set_text("probe");
        assert_eq!(memory.get_text().as_deref(), Some("probe"));
    }

    /// Real system clipboard round trip, including reads by other platform clients (T-3).
    /// Skipped when no system clipboard exists (run with `DISPLAY` pointing at XWayland to
    /// exercise it). All system assertions live in this one test because the platform
    /// clipboard is a single global resource - parallel tests would race over it.
    ///
    /// The test doubles as its own child process: re-executed with
    /// `RUSTUI_CLIPBOARD_CHILD_READ=<text>` it acts as a genuinely separate X11 client that
    /// pastes what the parent copied, proving the selection owner's serving thread answers
    /// foreign clients (a real cross process paste).
    #[test]
    fn system_clipboard_round_trips_across_clients() {
        if let Ok(expected) = std::env::var("RUSTUI_CLIPBOARD_CHILD_READ") {
            // Child mode: a separate process, a separate X client.
            let clipboard = PlatformClipboard::new();
            assert!(
                clipboard.is_system(),
                "child must reach the system clipboard"
            );
            assert_eq!(clipboard.get_text().as_deref(), Some(expected.as_str()));
            return;
        }

        let writer = PlatformClipboard::new();
        if !writer.is_system() {
            eprintln!("skipped: no system clipboard in this environment");
            return;
        }
        // Be a good citizen: remember whatever the user had and put it back afterwards.
        let original = writer.get_text();

        // Self round trip.
        writer.set_text("rustui-t3-round-trip");
        assert_eq!(writer.get_text().as_deref(), Some("rustui-t3-round-trip"));

        // A second handle reads through the platform (same process, second API user).
        let reader = PlatformClipboard::new();
        assert!(reader.is_system());
        assert_eq!(reader.get_text().as_deref(), Some("rustui-t3-round-trip"));

        // Cross process: re-exec this test binary as a separate X11 client that pastes our
        // text without sharing any process state.
        writer.set_text("rustui-t3-cross-process");
        let child = std::process::Command::new(std::env::current_exe().expect("test binary path"))
            .args([
                "clipboard::tests::system_clipboard_round_trips_across_clients",
                "--exact",
                "--nocapture",
            ])
            .env("RUSTUI_CLIPBOARD_CHILD_READ", "rustui-t3-cross-process")
            .output()
            .expect("spawn clipboard reader child");
        assert!(
            child.status.success(),
            "a separate process could not paste our clipboard text:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );

        // Emptying the clipboard is not an error.
        writer.set_text("");
        assert_eq!(writer.get_text().as_deref(), Some(""));

        // Restore the previous contents (an empty string is the closest we can get to
        // "nothing was on the clipboard").
        writer.set_text(original.as_deref().unwrap_or(""));
    }
}
