//! Translation of `winit` events into [`UiEvent`]s (§9 of the detailed design document).
//!
//! ```text
//! winit::WindowEvent  ->  EventTranslator  ->  UiEvent
//! ```
//!
//! The translator is stateful: it owns the DPI scale of its window, the modifier state (winit only
//! reports it in dedicated events) and the click timing needed to synthesize double clicks.

use std::any::Any;
use std::time::{Duration, Instant};

use rustui_core::{Key, Modifiers, MouseButton, NamedKey, Point, ScrollPhase, Size, UiEvent, Vec2};
use winit::event::{
    ElementState, Ime, MouseScrollDelta, Touch as WinitTouch, TouchPhase, WindowEvent,
};
use winit::keyboard::{Key as WinitKey, KeyLocation, ModifiersState, NamedKey as WinitNamedKey};

/// Logical pixels scrolled by one "line" tick of a wheel that reports lines instead of pixels.
const LINE_SCROLL: f32 = 16.0;
/// Two presses within this window and this distance (logical pixels) are a double click.
const DOUBLE_CLICK_TIME: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_SLOP: f32 = 4.0;

/// Payload posted from another thread through [`crate::PlatformProxy`].
///
/// It is `Send` so it may cross threads; the runner turns it into a [`UiEvent`] on the UI thread
/// (where it is wrapped in an `Rc`).
pub(crate) enum UserEvent {
    /// A background task finished: `(task id, payload)`.
    Task(u64, Box<dyn Any + Send>),
    /// A named application event: `(name, payload)`.
    Custom(&'static str, Box<dyn Any + Send>),
}

impl UserEvent {
    /// Converts the payload into the UI event delivered to the window.
    pub(crate) fn into_ui_event(self) -> UiEvent {
        match self {
            UserEvent::Task(id, payload) => UiEvent::TaskCompleted {
                task: rustui_core::TaskId(id),
                result: std::rc::Rc::new(payload),
            },
            UserEvent::Custom(name, payload) => UiEvent::Custom {
                name,
                payload: std::rc::Rc::new(payload),
            },
        }
    }
}

/// Stateless translation helper (the signature sketched in §9 of the design document).
///
/// [`EventTranslator`] is preferred for real windows: it tracks the DPI scale, the modifier state
/// and click timing. This helper assumes a fresh window at scale factor 1.0 and is meant for tests
/// and one shot conversions.
pub fn translate_event(event: &WindowEvent) -> Vec<UiEvent> {
    EventTranslator::new(1.0).translate(event)
}

/// Stateful `winit` -> [`UiEvent`] translator, one per window.
pub struct EventTranslator {
    scale: f64,
    modifiers: Modifiers,
    cursor: Point,
    last_click: Option<(MouseButton, Point, Instant)>,
}

impl EventTranslator {
    /// Creates a translator for a window with the given display scale factor.
    pub fn new(scale_factor: f64) -> Self {
        Self {
            scale: scale_factor,
            modifiers: Modifiers::NONE,
            cursor: Point::new(0.0, 0.0),
            last_click: None,
        }
    }

    /// The display scale factor the translator converts with.
    pub fn scale_factor(&self) -> f64 {
        self.scale
    }

    /// Updates the scale factor (from `WindowEvent::ScaleFactorChanged`).
    pub fn set_scale_factor(&mut self, scale: f64) {
        if scale > 0.0 {
            self.scale = scale;
        }
    }

    /// The modifier state reported by the last `ModifiersChanged` event.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Last known pointer position in logical pixels.
    pub fn cursor_position(&self) -> Point {
        self.cursor
    }

    /// Translates one `winit` window event into zero or more UI events.
    ///
    /// Events without a UI meaning (window movement, occlusion, redraw requests, ...) produce no
    /// output; `RedrawRequested` is handled by the event loop itself.
    pub fn translate(&mut self, event: &WindowEvent) -> Vec<UiEvent> {
        let mut out = Vec::new();
        match event {
            WindowEvent::Resized(size) => out.push(UiEvent::Resize(self.to_size(*size))),
            WindowEvent::CloseRequested => out.push(UiEvent::CloseRequested),
            WindowEvent::Focused(focused) => out.push(UiEvent::WindowFocus(*focused)),

            WindowEvent::KeyboardInput { event, .. } => out.extend(self.translate_key(
                &event.logical_key,
                event.location,
                event.state,
                event.text.as_deref(),
                event.repeat,
            )),

            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = to_modifiers(modifiers.state());
            }

            WindowEvent::Ime(ime) => match ime {
                Ime::Enabled => out.push(UiEvent::ImeStart),
                Ime::Preedit(text, cursor) => out.push(UiEvent::ImeUpdate {
                    text: text.clone(),
                    cursor: cursor.map(|(start, end)| start..end),
                }),
                Ime::Commit(text) => out.push(UiEvent::ImeCommit(text.clone())),
                Ime::Disabled => out.push(UiEvent::ImeEnd),
            },

            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = self.to_point(*position);
                out.push(UiEvent::MouseMove {
                    position: self.cursor,
                });
            }
            WindowEvent::CursorEntered { .. } => out.push(UiEvent::MouseEnter),
            WindowEvent::CursorLeft { .. } => out.push(UiEvent::MouseLeave),

            WindowEvent::MouseWheel { delta, phase, .. } => out.push(UiEvent::MouseWheel {
                delta: to_scroll_delta(*delta),
                phase: to_scroll_phase(*phase),
            }),

            WindowEvent::MouseInput { state, button, .. } => {
                out.extend(self.translate_button(*state, to_button(*button)))
            }

            WindowEvent::Touch(touch) => out.extend(self.translate_touch(touch)),

            WindowEvent::HoveredFile(path) => out.push(UiEvent::DragEnter {
                position: self.cursor,
                payload: path.to_string_lossy().into_owned(),
            }),
            WindowEvent::HoveredFileCancelled => out.push(UiEvent::DragLeave),
            WindowEvent::DroppedFile(path) => out.push(UiEvent::Drop {
                position: self.cursor,
                payload: path.to_string_lossy().into_owned(),
            }),

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.set_scale_factor(*scale_factor);
                out.push(UiEvent::ScaleFactorChanged(*scale_factor));
            }

            // `RedrawRequested` is handled by the event loop; the remaining variants (Moved,
            // Occluded, Destroyed, ThemeChanged, ...) carry no UI meaning on their own.
            _ => {}
        }
        out
    }

    /// Translates one key press / release.
    ///
    /// A press produces [`UiEvent::KeyDown`] plus, when it inserts text and no shortcut modifier
    /// is held, an [`UiEvent::TextInput`]. Keeping both lets text widgets listen for `TextInput`
    /// while shortcut widgets listen for `KeyDown`.
    pub(crate) fn translate_key(
        &mut self,
        logical_key: &WinitKey,
        location: KeyLocation,
        state: ElementState,
        text: Option<&str>,
        repeat: bool,
    ) -> Vec<UiEvent> {
        let key = to_key(logical_key, location);
        let mut out = Vec::new();
        match state {
            ElementState::Pressed => {
                out.push(UiEvent::KeyDown {
                    key: key.clone(),
                    modifiers: self.modifiers,
                    text: text.map(str::to_owned),
                    repeat,
                });
                if let Some(text) = text {
                    let shortcut = self.modifiers.control || self.modifiers.super_key;
                    if !text.is_empty() && !shortcut {
                        out.push(UiEvent::TextInput(text.to_owned()));
                    }
                }
            }
            ElementState::Released => out.push(UiEvent::KeyUp {
                key,
                modifiers: self.modifiers,
            }),
        }
        out
    }

    /// Translates a mouse button press / release, synthesizing double clicks.
    ///
    /// The second press of a double click emits *both* `MouseDown` and `MouseDoubleClick`, so
    /// buttons keep working while list views can use the double click.
    pub(crate) fn translate_button(
        &mut self,
        state: ElementState,
        button: MouseButton,
    ) -> Vec<UiEvent> {
        let position = self.cursor;
        match state {
            ElementState::Pressed => {
                let mut out = vec![UiEvent::MouseDown { button, position }];
                let double = self
                    .last_click
                    .map(|(last_button, last_position, at)| {
                        last_button == button
                            && at.elapsed() <= DOUBLE_CLICK_TIME
                            && distance(last_position, position) <= DOUBLE_CLICK_SLOP
                    })
                    .unwrap_or(false);
                if double {
                    out.push(UiEvent::MouseDoubleClick { button, position });
                    self.last_click = None;
                } else {
                    self.last_click = Some((button, position, Instant::now()));
                }
                out
            }
            ElementState::Released => vec![UiEvent::MouseUp { button, position }],
        }
    }

    fn translate_touch(&mut self, touch: &WinitTouch) -> Vec<UiEvent> {
        self.cursor = self.to_point(touch.location);
        vec![UiEvent::Touch {
            position: self.cursor,
            phase: to_scroll_phase(touch.phase),
            id: touch.id,
        }]
    }

    fn to_point(&self, position: winit::dpi::PhysicalPosition<f64>) -> Point {
        Point::new(
            (position.x / self.scale) as f32,
            (position.y / self.scale) as f32,
        )
    }

    fn to_size(&self, size: winit::dpi::PhysicalSize<u32>) -> Size {
        Size::new(
            size.width as f32 / self.scale as f32,
            size.height as f32 / self.scale as f32,
        )
    }
}

fn distance(a: Point, b: Point) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

/// Converts a `winit` logical key into a RustUI [`Key`].
pub fn to_key(key: &WinitKey, location: KeyLocation) -> Key {
    match key {
        WinitKey::Named(named) => match to_named(*named, location) {
            Some(key) => Key::Named(key),
            None => Key::Unidentified,
        },
        WinitKey::Character(text) => Key::Character(text.to_string()),
        // `Dead` and `Unidentified` keys are reported as such; text still arrives through the
        // `text` field of the key event.
        _ => Key::Unidentified,
    }
}

/// Converts a `winit` named key, returning `None` for keys RustUI does not model (modifier keys
/// are reported through [`Modifiers`] instead).
pub fn to_named(key: WinitNamedKey, location: KeyLocation) -> Option<NamedKey> {
    use winit::keyboard::NamedKey as W;
    let mapped = match key {
        W::Escape => NamedKey::Escape,
        W::Enter => {
            if location == KeyLocation::Numpad {
                NamedKey::Return
            } else {
                NamedKey::Enter
            }
        }
        W::Tab => NamedKey::Tab,
        W::Backspace => NamedKey::Backspace,
        W::Delete => NamedKey::Delete,
        W::Insert => NamedKey::Insert,
        W::Space => NamedKey::Space,
        W::Home => NamedKey::Home,
        W::End => NamedKey::End,
        W::PageUp => NamedKey::PageUp,
        W::PageDown => NamedKey::PageDown,
        W::ArrowUp => NamedKey::ArrowUp,
        W::ArrowDown => NamedKey::ArrowDown,
        W::ArrowLeft => NamedKey::ArrowLeft,
        W::ArrowRight => NamedKey::ArrowRight,
        // Function keys: `winit` spells every one of them out.
        W::F1 => NamedKey::F(1),
        W::F2 => NamedKey::F(2),
        W::F3 => NamedKey::F(3),
        W::F4 => NamedKey::F(4),
        W::F5 => NamedKey::F(5),
        W::F6 => NamedKey::F(6),
        W::F7 => NamedKey::F(7),
        W::F8 => NamedKey::F(8),
        W::F9 => NamedKey::F(9),
        W::F10 => NamedKey::F(10),
        W::F11 => NamedKey::F(11),
        W::F12 => NamedKey::F(12),
        W::F13 => NamedKey::F(13),
        W::F14 => NamedKey::F(14),
        W::F15 => NamedKey::F(15),
        W::F16 => NamedKey::F(16),
        W::F17 => NamedKey::F(17),
        W::F18 => NamedKey::F(18),
        W::F19 => NamedKey::F(19),
        W::F20 => NamedKey::F(20),
        W::F21 => NamedKey::F(21),
        W::F22 => NamedKey::F(22),
        W::F23 => NamedKey::F(23),
        W::F24 => NamedKey::F(24),
        W::F25 => NamedKey::F(25),
        W::F26 => NamedKey::F(26),
        W::F27 => NamedKey::F(27),
        W::F28 => NamedKey::F(28),
        W::F29 => NamedKey::F(29),
        W::F30 => NamedKey::F(30),
        W::F31 => NamedKey::F(31),
        W::F32 => NamedKey::F(32),
        W::F33 => NamedKey::F(33),
        W::F34 => NamedKey::F(34),
        W::F35 => NamedKey::F(35),
        _ => return None,
    };
    Some(mapped)
}

/// Converts a `winit` mouse button into a RustUI [`MouseButton`].
pub fn to_button(button: winit::event::MouseButton) -> MouseButton {
    use winit::event::MouseButton as W;
    match button {
        W::Left => MouseButton::Left,
        W::Right => MouseButton::Right,
        W::Middle => MouseButton::Middle,
        // Side buttons keep the X11 numbering so `Other(4)` means "back" everywhere.
        W::Back => MouseButton::Other(4),
        W::Forward => MouseButton::Other(5),
        W::Other(number) => MouseButton::Other(number),
    }
}

/// Converts `winit` modifier flags into RustUI [`Modifiers`].
pub fn to_modifiers(state: ModifiersState) -> Modifiers {
    Modifiers::new(
        state.shift_key(),
        state.control_key(),
        state.alt_key(),
        state.super_key(),
    )
}

fn to_scroll_delta(delta: MouseScrollDelta) -> Vec2 {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => Vec2::new(x * LINE_SCROLL, y * LINE_SCROLL),
        MouseScrollDelta::PixelDelta(position) => Vec2::new(position.x as f32, position.y as f32),
    }
}

fn to_scroll_phase(phase: TouchPhase) -> ScrollPhase {
    match phase {
        TouchPhase::Started => ScrollPhase::Start,
        TouchPhase::Moved => ScrollPhase::Move,
        TouchPhase::Ended | TouchPhase::Cancelled => ScrollPhase::End,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::event::DeviceId;
    use winit::keyboard::NamedKey as W;

    fn translator() -> EventTranslator {
        // Scale factor 2: physical 800x600 is logical 400x300.
        EventTranslator::new(2.0)
    }

    #[test]
    fn resize_is_converted_to_logical_pixels() {
        let mut t = translator();
        let events = t.translate(&WindowEvent::Resized(winit::dpi::PhysicalSize::new(
            800, 600,
        )));
        match &events[..] {
            [UiEvent::Resize(size)] => assert_eq!(*size, Size::new(400.0, 300.0)),
            other => panic!("expected one Resize, got {other:?}"),
        }
    }

    #[test]
    fn cursor_moves_are_converted_to_logical_pixels() {
        let mut t = translator();
        let events = t.translate(&WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: winit::dpi::PhysicalPosition::new(100.0, 50.0),
        });
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].position(), Some(Point::new(50.0, 25.0)));
        assert_eq!(t.cursor_position(), Point::new(50.0, 25.0));
    }

    #[test]
    fn scale_factor_can_be_updated() {
        let mut t = translator();
        assert_eq!(t.scale_factor(), 2.0);
        t.set_scale_factor(1.0);
        assert_eq!(t.scale_factor(), 1.0);
        // Non positive factors are ignored instead of dividing by zero later.
        t.set_scale_factor(0.0);
        assert_eq!(t.scale_factor(), 1.0);
    }

    #[test]
    fn key_press_emits_key_down_and_text() {
        let mut t = translator();
        let events = t.translate_key(
            &WinitKey::Character("a".into()),
            KeyLocation::Standard,
            ElementState::Pressed,
            Some("a"),
            false,
        );
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            UiEvent::KeyDown {
                repeat: false,
                text: Some(text),
                ..
            } if text.as_str() == "a"
        ));
        match &events[1] {
            UiEvent::TextInput(text) => assert_eq!(text, "a"),
            other => panic!("expected TextInput, got {other:?}"),
        }
    }

    #[test]
    fn shortcut_presses_do_not_emit_text() {
        let mut t = translator();
        t.modifiers = Modifiers::CTRL;
        let events = t.translate_key(
            &WinitKey::Character("c".into()),
            KeyLocation::Standard,
            ElementState::Pressed,
            Some("c"),
            false,
        );
        assert_eq!(events.len(), 1, "Ctrl+C must not produce TextInput");
        assert!(matches!(events[0], UiEvent::KeyDown { .. }));
    }

    #[test]
    fn key_release_only_emits_key_up() {
        let mut t = translator();
        let events = t.translate_key(
            &WinitKey::Named(W::Escape),
            KeyLocation::Standard,
            ElementState::Released,
            None,
            false,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], UiEvent::KeyUp { .. }));
    }

    #[test]
    fn named_keys_and_modifiers_round_trip() {
        assert_eq!(
            to_key(&WinitKey::Named(W::Escape), KeyLocation::Standard),
            Key::Named(NamedKey::Escape)
        );
        assert_eq!(
            to_key(&WinitKey::Named(W::Enter), KeyLocation::Numpad),
            Key::Named(NamedKey::Return)
        );
        // Modifier keys themselves are not part of the key vocabulary.
        assert_eq!(
            to_key(&WinitKey::Named(W::Shift), KeyLocation::Standard),
            Key::Unidentified
        );
        let modifiers = to_modifiers(ModifiersState::SHIFT | ModifiersState::CONTROL);
        assert!(modifiers.shift && modifiers.control && !modifiers.alt);
        assert!(!modifiers.super_key);
    }

    #[test]
    fn double_click_is_detected_within_slop_and_time() {
        let mut t = translator();
        let first = t.translate_button(ElementState::Pressed, MouseButton::Left);
        assert!(matches!(first[0], UiEvent::MouseDown { .. }));
        assert_eq!(first.len(), 1);

        let second = t.translate_button(ElementState::Pressed, MouseButton::Left);
        assert_eq!(second.len(), 2);
        assert!(matches!(second[1], UiEvent::MouseDoubleClick { .. }));

        // After a double click the third press starts over.
        let third = t.translate_button(ElementState::Pressed, MouseButton::Left);
        assert_eq!(third.len(), 1);
    }

    #[test]
    fn wheel_lines_are_converted_to_logical_pixels() {
        let mut t = translator();
        let events = t.translate(&WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -1.0),
            phase: TouchPhase::Moved,
        });
        match &events[..] {
            [UiEvent::MouseWheel { delta, phase, .. }] => {
                assert_eq!(delta.y, -LINE_SCROLL);
                assert_eq!(*phase, ScrollPhase::Move);
            }
            other => panic!("expected one MouseWheel, got {other:?}"),
        }
    }

    #[test]
    fn events_without_ui_meaning_are_ignored() {
        let mut t = translator();
        assert!(t.translate(&WindowEvent::Occluded(true)).is_empty());
        assert!(t.translate(&WindowEvent::RedrawRequested).is_empty());
    }
}
