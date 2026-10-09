//! # rustui-window
//!
//! The window and frame pipeline of RustUI (§10-§12, §30, §32): it owns the per window state,
//! drives `update -> layout -> paint`, hit tests the pointer, routes events through the tree and
//! manages keyboard focus and overlays.
//!
//! ```text
//! UiEvent ──> Window::dispatch ──> EventQueue ──> Widget::event ──> bubble until handled
//! FrameClock ──> update ──> measure + arrange ──> paint ──> Painter (PaintCommand[])
//! ```
//!
//! ## Design decisions
//!
//! * **Shared services, per window state.** [`WindowManager`] owns the `UiContext` (theme,
//!   text / clipboard), the `WidgetStore` and the `ObjectRegistry`; every [`Window`] only owns
//!   focus, queue, interaction, overlays and its painter. Nothing is shared through
//!   `Rc<RefCell<T>>` (§52) — the manager lends the services to a window for the length of one
//!   call.
//! * **Queued delivery.** Events are never delivered re-entrantly: the target is pushed into the
//!   `EventQueue` and drained afterwards, which is what lets a widget mutate itself while still
//!   reaching descendants.
//! * **Hit test returns a chain.** The deepest widget under the pointer is the target, but the
//!   whole root-to-leaf path is kept in `Interaction::hover_path`, so ancestors (a button behind
//!   its label) still derive `hovered` correctly.
//! * **The window lays the tree out every frame.** Frames only run when something asked for a
//!   redraw, which keeps the pipeline correct without a dirty-flag protocol (§49).
//! * **No `winit`, no `wgpu`** except for `RenderInput` — this crate depends on
//!   `rustui-core` + `rustui-renderer` + `log` only (P1).

mod dispatch;
mod manager;
mod window;

pub use manager::WindowManager;
pub use window::{EventOutcome, FocusDirection, Window, WindowId, WindowOptions};
