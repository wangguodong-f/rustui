//! # RustUI
//!
//! A native, cross platform, GPU accelerated, retained mode GUI framework for Rust.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ rustui            Application, prelude, examples            │
//! ├────────────┬───────────────┬────────────────────────────────┤
//! │ rustui-    │ rustui-layout │ rustui-widgets                 │
//! │ window     │               │ rustui-model  rustui-animation │
//! ├────────────┴───────────────┴────────────────────────────────┤
//! │ rustui-core      objects, events, painting, themes          │
//! ├─────────────────────────────────────────────────────────────┤
//! │ rustui-platform (winit)  rustui-text  rustui-renderer (wgpu)│
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! Only the wiring layer (this crate), `rustui-platform` and `rustui-renderer` see `winit` or
//! `wgpu`; widgets are plain data and stay testable without a window (principle P1).
//!
//! # Getting started
//!
//! ```ignore
//! use rustui::prelude::*;
//!
//! fn main() -> rustui::Result<()> {
//!     Application::new()?
//!         .window(WindowSpec::new().title("Hello").size(480.0, 320.0))
//!         .run(|app| {
//!             let window = app.primary_window().expect("one window");
//!             app.set_root(window, Box::new(Label::new("Hello, RustUI!")));
//!         })
//! }
//! ```

mod app;

pub use app::{spawn, AppContext, Application, WindowSpec};

// The building blocks, re-exported under short names so applications only depend on `rustui`.
pub use rustui_animation as animation;
pub use rustui_core as core;
pub use rustui_layout as layout;
pub use rustui_model as model;
pub use rustui_platform as platform;
pub use rustui_renderer as renderer;
pub use rustui_text as text;
pub use rustui_widgets as widgets;
pub use rustui_window as window;

pub use rustui_core::{Result, UiError};

/// Everything an application typically needs.
///
/// ```ignore
/// use rustui::prelude::*;
/// ```
pub mod prelude {
    pub use crate::{spawn, AppContext, Application, WindowSpec};
    pub use rustui_animation::prelude::*;
    pub use rustui_core::prelude::*;
    pub use rustui_layout::prelude::*;
    pub use rustui_model::prelude::*;
    pub use rustui_widgets::prelude::*;
    pub use rustui_window::{WindowId, WindowManager};
}
