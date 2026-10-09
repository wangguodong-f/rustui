//! Window creation and window handles.

use std::fmt;
use std::sync::Arc;

use rustui_core::{CursorIcon, Rect, Size};
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::window::{Window as WinitWindow, WindowAttributes};

use crate::cursor::to_winit_cursor;

/// Description of a window before it exists (§11 of the detailed design document).
///
/// All sizes are *logical* pixels; the platform layer converts them with the display scale factor
/// (§41).
#[derive(Clone, Debug, PartialEq)]
pub struct WindowOptions {
    /// Window title.
    pub title: String,
    /// Client area width in logical pixels.
    pub width: f32,
    /// Client area height in logical pixels.
    pub height: f32,
    /// Smallest accepted client width, in logical pixels.
    pub min_width: Option<f32>,
    /// Smallest accepted client height, in logical pixels.
    pub min_height: Option<f32>,
    /// `false` fixes the window size.
    pub resizable: bool,
    /// `false` removes the system title bar and borders.
    pub decorated: bool,
    /// `false` creates the window hidden.
    pub visible: bool,
    /// `true` asks for a transparent framebuffer (needs platform support).
    pub transparent: bool,
    /// `true` opens the window maximized.
    pub maximized: bool,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: "RustUI".to_string(),
            width: 1024.0,
            height: 768.0,
            min_width: None,
            min_height: None,
            resizable: true,
            decorated: true,
            visible: true,
            transparent: false,
            maximized: false,
        }
    }
}

impl WindowOptions {
    /// New options with sensible defaults (1024x768, resizable, decorated, visible).
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the window title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Sets the initial client size (logical pixels).
    pub fn with_size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Sets the smallest accepted client size (logical pixels).
    pub fn with_min_size(mut self, width: f32, height: f32) -> Self {
        self.min_width = Some(width);
        self.min_height = Some(height);
        self
    }

    /// Makes the window resizable or fixed size.
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Shows or hides the system decorations.
    pub fn decorated(mut self, decorated: bool) -> Self {
        self.decorated = decorated;
        self
    }

    /// Shows the window as soon as it is created.
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Opens the window maximized.
    pub fn maximized(mut self, maximized: bool) -> Self {
        self.maximized = maximized;
        self
    }

    /// Builds the `winit` attributes for this description.
    pub(crate) fn attributes(&self) -> WindowAttributes {
        let mut attributes = WinitWindow::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(LogicalSize::new(self.width, self.height))
            .with_resizable(self.resizable)
            .with_decorations(self.decorated)
            .with_visible(self.visible)
            .with_transparent(self.transparent)
            .with_maximized(self.maximized);
        if self.min_width.is_some() || self.min_height.is_some() {
            attributes = attributes.with_min_inner_size(LogicalSize::new(
                self.min_width.unwrap_or(1.0).max(1.0),
                self.min_height.unwrap_or(1.0).max(1.0),
            ));
        }
        attributes
    }
}

/// A cloneable handle to an OS window.
///
/// The handle is reference counted: the event loop keeps one reference per window it knows about,
/// and [`crate::PlatformContext::destroy_window`] drops it. Closing a window therefore means
/// dropping every remaining handle (see `WindowHandle::strong_count`).
#[derive(Clone)]
pub struct WindowHandle(Arc<WinitWindow>);

impl WindowHandle {
    pub(crate) fn from_window(window: WinitWindow) -> Self {
        Self(Arc::new(window))
    }

    /// The underlying `winit` window.
    ///
    /// This is the escape hatch used by the wiring layer (`rustui`) to build a `wgpu` surface;
    /// widgets never see it (principle P1).
    pub fn inner(&self) -> &WinitWindow {
        &self.0
    }

    /// The `winit` window id, used to route events back to the right window.
    pub fn id(&self) -> winit::window::WindowId {
        self.0.id()
    }

    /// Display scale factor of this window (§41).
    pub fn scale_factor(&self) -> f64 {
        self.0.scale_factor()
    }

    /// Client size in logical pixels.
    pub fn size(&self) -> Size {
        let scale = self.scale_factor();
        let physical = self.0.inner_size();
        Size::new(
            physical.width as f32 / scale as f32,
            physical.height as f32 / scale as f32,
        )
    }

    /// Client size in physical pixels, as the renderer needs it.
    pub fn physical_size(&self) -> (u32, u32) {
        let physical = self.0.inner_size();
        (physical.width, physical.height)
    }

    /// Asks the window system for a new frame.
    pub fn request_redraw(&self) {
        self.0.request_redraw();
    }

    /// Changes the window title.
    pub fn set_title(&self, title: &str) {
        self.0.set_title(title);
    }

    /// Changes the cursor shown over the window (§40).
    pub fn set_cursor_icon(&self, icon: CursorIcon) {
        self.0.set_cursor(to_winit_cursor(icon));
    }

    /// Shows or hides the cursor.
    pub fn set_cursor_visible(&self, visible: bool) {
        self.0.set_cursor_visible(visible);
    }

    /// Enables or disables input method editing (IME) composition.
    pub fn set_ime_allowed(&self, allowed: bool) {
        self.0.set_ime_allowed(allowed);
    }

    /// Anchors the input method candidate window to `area`, in **logical** pixels (§41).
    ///
    /// Only honoured while [`Self::set_ime_allowed`] is `true`, so send it *after* enabling the
    /// input method. Sizes are clamped to at least one logical pixel: a zero sized rectangle is
    /// ignored by some compositors. On X11 only the position is used.
    pub fn set_ime_cursor_area(&self, area: Rect) {
        self.0.set_ime_cursor_area(
            LogicalPosition::new(f64::from(area.x), f64::from(area.y)),
            LogicalSize::new(
                f64::from(area.width.max(1.0)),
                f64::from(area.height.max(1.0)),
            ),
        );
    }

    /// Brings the window to the front and gives it OS focus.
    pub fn focus(&self) {
        self.0.focus_window();
    }

    /// Number of live handles. When it reaches `1` the window closes on drop.
    pub fn strong_count(&self) -> usize {
        Arc::strong_count(&self.0)
    }
}

impl fmt::Debug for WindowHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowHandle")
            .field("id", &self.id())
            .field("size", &self.size())
            .field("scale", &self.scale_factor())
            .finish()
    }
}

impl PartialEq for WindowHandle {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for WindowHandle {}

impl std::hash::Hash for WindowHandle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}
