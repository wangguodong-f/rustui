//! The UI event type (§8).
//!
//! Platform events are translated into [`UiEvent`] by `rustui-platform` before they ever reach a
//! widget, which guarantees that widgets never depend on `winit` (design principle P1).

use std::fmt;
use std::ops::Range;
use std::rc::Rc;

use crate::geometry::{Point, Size, Vec2};
use crate::object::ObjectId;

pub use crate::keyboard::{
    CursorIcon, FocusReason, Key, Modifiers, MouseButton, NamedKey, ScrollPhase,
};

/// Identifier of a background task spawned through `Application::spawn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(pub u64);

/// Payload delivered by a completed background task.
pub type TaskPayload = Rc<dyn std::any::Any>;

/// Result of one phase of a scroll gesture.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollDelta {
    /// Logical pixels (already scaled by the platform layer).
    pub delta: Vec2,
    pub phase: ScrollPhase,
}

/// A platform independent UI event.
#[derive(Clone, Debug)]
pub enum UiEvent {
    // ---- pointer ------------------------------------------------------
    MouseMove {
        position: Point,
    },
    /// The pointer entered the object.
    MouseEnter,
    /// The pointer left the object.
    MouseLeave,
    MouseDown {
        button: MouseButton,
        position: Point,
    },
    MouseUp {
        button: MouseButton,
        position: Point,
    },
    MouseDoubleClick {
        button: MouseButton,
        position: Point,
    },
    /// The object grabbed the pointer: every following pointer event goes to it until release.
    MouseCapture,
    /// The pointer grab was released.
    MouseRelease,
    MouseWheel {
        delta: Vec2,
        phase: ScrollPhase,
    },
    /// Touch contact (translated from `winit::WindowEvent::Touch`).
    Touch {
        position: Point,
        phase: ScrollPhase,
        id: u64,
    },

    // ---- keyboard -----------------------------------------------------
    KeyDown {
        key: Key,
        modifiers: Modifiers,
        /// Text produced by the keyboard layout, if the key inserts text.
        text: Option<String>,
        repeat: bool,
    },
    KeyUp {
        key: Key,
        modifiers: Modifiers,
    },
    /// Committed text (already produced by the input method).
    TextInput(String),

    // ---- IME ----------------------------------------------------------
    ImeStart,
    ImeUpdate {
        text: String,
        /// Cursor position inside the composing text, in bytes.
        cursor: Option<Range<usize>>,
    },
    ImeCommit(String),
    ImeEnd,

    // ---- focus --------------------------------------------------------
    FocusIn(FocusReason),
    FocusOut,

    // ---- window -------------------------------------------------------
    /// The drawable area of the window changed.
    Resize(Size),
    /// The window's display scale factor changed.
    ScaleFactorChanged(f64),
    /// The window wants to close (close button pressed).
    CloseRequested,
    /// The window gained or lost OS focus.
    WindowFocus(bool),
    /// A frame was requested by the window system (`AboutToWait` / animation).
    FrameTick {
        frame: u64,
        delta: std::time::Duration,
        elapsed: std::time::Duration,
    },

    // ---- data ---------------------------------------------------------
    /// A drag & drop payload entered the object.
    DragEnter {
        position: Point,
        payload: String,
    },
    /// The payload left the object.
    DragLeave,
    /// The payload was dropped on the object.
    Drop {
        position: Point,
        payload: String,
    },
    /// A background task finished; delivered on the UI thread.
    TaskCompleted {
        task: TaskId,
        result: TaskPayload,
    },
    /// Custom event created by application code.
    Custom {
        name: &'static str,
        payload: TaskPayload,
    },
}

impl UiEvent {
    /// Pointer position carried by this event, if any.
    pub fn position(&self) -> Option<Point> {
        match self {
            UiEvent::MouseMove { position }
            | UiEvent::MouseDown { position, .. }
            | UiEvent::MouseUp { position, .. }
            | UiEvent::MouseDoubleClick { position, .. }
            | UiEvent::Touch { position, .. }
            | UiEvent::DragEnter { position, .. }
            | UiEvent::Drop { position, .. } => Some(*position),
            _ => None,
        }
    }

    /// `true` for events that carry a pointer position (and therefore participate in hit testing).
    pub fn is_pointer(&self) -> bool {
        self.position().is_some()
    }

    /// Short, stable name used for logging and debugging.
    pub fn name(&self) -> &'static str {
        match self {
            UiEvent::MouseMove { .. } => "MouseMove",
            UiEvent::MouseEnter => "MouseEnter",
            UiEvent::MouseLeave => "MouseLeave",
            UiEvent::MouseDown { .. } => "MouseDown",
            UiEvent::MouseUp { .. } => "MouseUp",
            UiEvent::MouseDoubleClick { .. } => "MouseDoubleClick",
            UiEvent::MouseCapture => "MouseCapture",
            UiEvent::MouseRelease => "MouseRelease",
            UiEvent::MouseWheel { .. } => "MouseWheel",
            UiEvent::Touch { .. } => "Touch",
            UiEvent::KeyDown { .. } => "KeyDown",
            UiEvent::KeyUp { .. } => "KeyUp",
            UiEvent::TextInput(_) => "TextInput",
            UiEvent::ImeStart => "ImeStart",
            UiEvent::ImeUpdate { .. } => "ImeUpdate",
            UiEvent::ImeCommit(_) => "ImeCommit",
            UiEvent::ImeEnd => "ImeEnd",
            UiEvent::FocusIn(_) => "FocusIn",
            UiEvent::FocusOut => "FocusOut",
            UiEvent::Resize(_) => "Resize",
            UiEvent::ScaleFactorChanged(_) => "ScaleFactorChanged",
            UiEvent::CloseRequested => "CloseRequested",
            UiEvent::WindowFocus(_) => "WindowFocus",
            UiEvent::FrameTick { .. } => "FrameTick",
            UiEvent::DragEnter { .. } => "DragEnter",
            UiEvent::DragLeave => "DragLeave",
            UiEvent::Drop { .. } => "Drop",
            UiEvent::TaskCompleted { .. } => "TaskCompleted",
            UiEvent::Custom { .. } => "Custom",
        }
    }

    /// Creates a `Custom` event from any value.
    pub fn custom<T: 'static>(name: &'static str, payload: T) -> UiEvent {
        UiEvent::Custom {
            name,
            payload: Rc::new(payload),
        }
    }
}

impl fmt::Display for UiEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Result of handling an event (§10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventResult {
    /// The widget is not interested; the event continues to bubble towards the root.
    Unhandled,
    /// The widget consumed the event; propagation stops.
    Handled,
}

impl EventResult {
    #[inline]
    pub fn is_handled(&self) -> bool {
        matches!(self, EventResult::Handled)
    }

    #[inline]
    pub fn or(self, other: EventResult) -> EventResult {
        if self.is_handled() {
            self
        } else {
            other
        }
    }

    #[inline]
    pub fn or_handled(self) -> EventResult {
        if self.is_handled() {
            self
        } else {
            EventResult::Handled
        }
    }
}

/// Which widget an event is being delivered to, and in which phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventPhase {
    /// Delivered to the widget under the pointer (or the focus owner for key events).
    Target,
    /// Delivered from the target upwards to the root.
    Bubble,
}

/// An event scheduled for delivery.
#[derive(Clone, Debug)]
pub struct QueuedEvent {
    pub target: ObjectId,
    pub event: UiEvent,
    pub phase: EventPhase,
    pub cursor: CursorIcon,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_result_combine() {
        assert_eq!(
            EventResult::Handled.or(EventResult::Unhandled),
            EventResult::Handled
        );
        assert_eq!(
            EventResult::Unhandled.or(EventResult::Handled),
            EventResult::Handled
        );
        assert_eq!(EventResult::Unhandled.or_handled(), EventResult::Handled);
    }

    #[test]
    fn position_extraction() {
        let e = UiEvent::MouseDown {
            button: MouseButton::Left,
            position: Point::new(3.0, 4.0),
        };
        assert_eq!(e.position(), Some(Point::new(3.0, 4.0)));
        assert!(e.is_pointer());
        assert!(!UiEvent::FocusOut.is_pointer());
    }

    #[test]
    fn custom_event_name() {
        let e = UiEvent::custom("Ping", 42u8);
        assert_eq!(e.name(), "Custom");
    }
}
