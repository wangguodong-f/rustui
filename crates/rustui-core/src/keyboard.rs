//! Keyboard definitions, deliberately independent from `winit` (§9).

use std::fmt;

/// A named (non printable) key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Escape,
    Return,
    Enter,
    Tab,
    Backspace,
    Delete,
    Insert,
    Space,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    F(u32),
}

/// A platform independent key.
///
/// Printable keys are reported both as [`Key::Character`] (for text entry, respecting the current
/// keyboard layout) and as `winit`'s physical key, so the platform layer can translate.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Key {
    /// A key identified by its meaning on the current layout.
    Named(NamedKey),
    /// A printable character produced by the keyboard layout.
    Character(String),
    /// A key we could not identify.
    #[default]
    Unidentified,
}

impl Key {
    /// Returns the character this key inserts into a text editor, if any.
    pub fn to_text(&self) -> Option<&str> {
        match self {
            Key::Character(s) if !s.is_empty() => Some(s),
            _ => None,
        }
    }

    pub fn is_named(&self, key: NamedKey) -> bool {
        matches!(self, Key::Named(k) if *k == key)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Named(k) => write!(f, "{k:?}"),
            Key::Character(s) => write!(f, "{s}"),
            Key::Unidentified => write!(f, "Unidentified"),
        }
    }
}

/// Keyboard modifier state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_key: bool,
}

impl Modifiers {
    pub const NONE: Modifiers = Modifiers {
        shift: false,
        control: false,
        alt: false,
        super_key: false,
    };
    pub const CTRL: Modifiers = Modifiers {
        control: true,
        ..Modifiers::NONE
    };
    pub const SHIFT: Modifiers = Modifiers {
        shift: true,
        ..Modifiers::NONE
    };

    #[inline]
    pub const fn new(shift: bool, control: bool, alt: bool, super_key: bool) -> Self {
        Self {
            shift,
            control,
            alt,
            super_key,
        }
    }

    #[inline]
    pub const fn is_none(&self) -> bool {
        !self.shift && !self.control && !self.alt && !self.super_key
    }

    /// `true` when the platform's "command" modifier is held (Ctrl on Windows/Linux, Cmd on
    /// macOS). This is the modifier used for editing shortcuts such as Ctrl+C.
    #[inline]
    pub const fn command(&self) -> bool {
        self.control || self.super_key
    }
}

/// A mouse (or touch) button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

/// Direction of a scroll gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollPhase {
    Start,
    Move,
    End,
}

/// Why an object gained or lost keyboard focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FocusReason {
    /// The user clicked or tapped the object.
    Pointer,
    /// The user pressed Tab / Shift+Tab.
    Tab,
    /// Application code called `set_focus`.
    Programmatic,
    /// The object was removed or the window lost focus.
    #[default]
    Lost,
}

/// The mouse cursor shape requested by a widget (§40).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorIcon {
    #[default]
    Default,
    Pointer,
    Text,
    ResizeHorizontal,
    ResizeVertical,
    ResizeAll,
    Crosshair,
    Grab,
    Grabbing,
    Wait,
    NotAllowed,
    ColumnResize,
    RowResize,
    Cell,
}

impl CursorIcon {
    /// `true` when the cursor should be shown as a busy indicator.
    pub fn is_waiting(&self) -> bool {
        matches!(self, CursorIcon::Wait | CursorIcon::Grabbing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_text() {
        assert_eq!(Key::Character("a".into()).to_text(), Some("a"));
        assert_eq!(Key::Named(NamedKey::ArrowUp).to_text(), None);
        assert!(Key::Named(NamedKey::Tab).is_named(NamedKey::Tab));
    }

    #[test]
    fn modifiers() {
        assert!(Modifiers::NONE.is_none());
        assert!(Modifiers::CTRL.command());
        assert!(!Modifiers::SHIFT.command());
    }
}
