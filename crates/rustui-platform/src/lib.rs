//! # rustui-platform
//!
//! Everything that talks to the operating system through `winit` (§9 of the design documents).
//!
//! Responsibilities:
//!
//! * Own the `winit` event loop ([`Platform`]) and hand out windows ([`PlatformContext`]).
//! * Translate `winit` events into platform independent [`UiEvent`]s ([`EventTranslator`]) so that
//!   widgets never see a `winit` type (principle P1):
//!
//!   ```text
//!   winit::WindowEvent  ->  EventTranslator  ->  UiEvent  ->  Window  ->  Widget
//!   ```
//!
//! * Provide a cloneable, DPI aware [`WindowHandle`] plus [`WindowOptions`].
//! * Provide the [`Clipboard`] implementation used by the UI ([`PlatformClipboard`]).
//! * Let background threads post events back to the UI thread ([`PlatformProxy`]).
//!
//! The crate depends on `winit` only; GPU work lives in `rustui-renderer`.
//!
//! # Typical use
//!
//! ```ignore
//! let platform = Platform::new()?;
//! let proxy = platform.proxy();
//! platform.run(App::new(proxy))?;
//! ```
//!
//! [`UiEvent`]: rustui_core::UiEvent
//! [`Clipboard`]: rustui_core::Clipboard

mod clipboard;
mod cursor;
mod event;
mod run;
mod window;

pub use clipboard::PlatformClipboard;
pub use cursor::to_winit_cursor;
pub use event::{translate_event, EventTranslator};
pub use run::{EventHandler, EventTarget, Platform, PlatformContext, PlatformEvent, PlatformProxy};
pub use window::{WindowHandle, WindowOptions};

/// Result type used by this crate.
pub type Result<T> = rustui_core::Result<T>;
