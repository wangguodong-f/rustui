//! The `winit` event loop and its RustUI facing wrapper (§9 and §10).
//!
//! ```text
//! winit::EventLoop  ->  Platform  ->  EventHandler::on_event(PlatformEvent)
//!                                     ├── PlatformEvent::Ui(UiEvent)  ->  Window  ->  Widget
//!                                     └── PlatformEvent::Redraw       ->  render
//! ```
//!
//! Nothing above this crate ever sees a `winit` type except [`WindowHandle::inner`], which is used
//! by the wiring layer to build a GPU surface.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use rustui_core::{Result, UiError, UiEvent};
use winit::application::ApplicationHandler;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

use crate::event::{EventTranslator, UserEvent};
use crate::window::{WindowHandle, WindowOptions};

/// Where a [`PlatformEvent`] comes from.
#[derive(Clone, Debug)]
pub enum EventTarget {
    /// The event loop itself (start, exit, posted events).
    Loop,
    /// A specific window.
    Window(WindowHandle),
}

impl EventTarget {
    /// The window the event belongs to, if any.
    pub fn window(&self) -> Option<&WindowHandle> {
        match self {
            EventTarget::Loop => None,
            EventTarget::Window(window) => Some(window),
        }
    }
}

/// Everything the event loop reports to the application (§9).
#[derive(Debug)]
pub enum PlatformEvent {
    /// A platform independent event ready for dispatch ([`UiEvent`]).
    Ui(UiEvent),
    /// The window asks to be rendered (`RedrawRequested`).
    Redraw,
    /// The window was destroyed by the window system.
    WindowClosed,
    /// The loop started; create your windows here (first `resumed`).
    Start,
    /// The application was resumed after being suspended.
    Resume,
    /// The application was suspended (mobile / closing).
    Suspend,
    /// The loop is about to stop; tear everything down.
    Exit,
}

/// Receives events from [`Platform::run`].
pub trait EventHandler {
    /// Called for every platform event.
    ///
    /// `ctx` lets the handler create and destroy windows, quit the loop and post events.
    fn on_event(&mut self, ctx: &PlatformContext<'_>, target: &EventTarget, event: PlatformEvent);

    /// Called whenever the loop is about to wait for new events (§15: animations and pending
    /// state changes run here). Return `true` to ask for a redraw of every open window.
    fn on_idle(&mut self, _ctx: &PlatformContext<'_>) -> bool {
        false
    }
}

/// Event loop services handed to the [`EventHandler`].
///
/// The underlying `ActiveEventLoop` stays private: this keeps `winit` out of everything above the
/// platform layer.
pub struct PlatformContext<'a> {
    pub(crate) event_loop: &'a ActiveEventLoop,
    pub(crate) windows: &'a RefCell<HashMap<WindowId, WindowHandle>>,
}

impl PlatformContext<'_> {
    /// Creates a window and registers it with the event loop.
    ///
    /// Must be called from inside [`EventHandler::on_event`] (on the UI thread).
    pub fn create_window(&self, options: &WindowOptions) -> Result<WindowHandle> {
        let window = self
            .event_loop
            .create_window(options.attributes())
            .map_err(|error| UiError::WindowCreation(error.to_string()))?;
        let handle = WindowHandle::from_window(window);
        handle.request_redraw();
        self.windows
            .borrow_mut()
            .insert(handle.id(), handle.clone());
        Ok(handle)
    }

    /// Unregisters a window; the window closes once the last handle is dropped.
    pub fn destroy_window(&self, window: &WindowHandle) {
        self.windows.borrow_mut().remove(&window.id());
    }

    /// Every window the loop currently knows about.
    pub fn windows(&self) -> Vec<WindowHandle> {
        self.windows.borrow().values().cloned().collect()
    }

    /// Number of open windows.
    pub fn window_count(&self) -> usize {
        self.windows.borrow().len()
    }

    /// Stops the event loop after the current iteration.
    pub fn exit(&self) {
        self.event_loop.exit();
    }

    /// Switches between `Poll` (continuous animation) and `Wait` (idle until an event arrives).
    pub fn set_polling(&self, poll: bool) {
        let control_flow = if poll {
            ControlFlow::Poll
        } else {
            ControlFlow::Wait
        };
        self.event_loop.set_control_flow(control_flow);
    }

    /// Scale factor of the primary monitor, or `1.0` when it cannot be queried.
    pub fn primary_scale_factor(&self) -> f64 {
        self.event_loop
            .primary_monitor()
            .map(|monitor| monitor.scale_factor())
            .unwrap_or(1.0)
    }
}

/// Handle used by background threads to post events to the UI thread.
///
/// `UiEvent` itself is not `Send` (it carries `Rc` payloads), so the payload is kept `Send` here
/// and wrapped into an `Rc` only once the event reaches the loop.
#[derive(Clone)]
pub struct PlatformProxy {
    proxy: EventLoopProxy<UserEvent>,
}

impl PlatformProxy {
    /// Posts a finished background task (`UiEvent::TaskCompleted`).
    ///
    /// Returns `false` when the loop is already gone.
    pub fn post_task<T: Send + 'static>(&self, task: u64, payload: T) -> bool {
        self.proxy
            .send_event(UserEvent::Task(task, Box::new(payload)))
            .is_ok()
    }

    /// Posts a named application event (`UiEvent::Custom`).
    pub fn post<T: Send + 'static>(&self, name: &'static str, payload: T) -> bool {
        self.proxy
            .send_event(UserEvent::Custom(name, Box::new(payload)))
            .is_ok()
    }
}

impl fmt::Debug for PlatformProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformProxy").finish_non_exhaustive()
    }
}

/// The `winit` event loop, ready to run an application (§9).
///
/// Create it on the main thread, then call [`Platform::run`]:
///
/// ```ignore
/// let platform = Platform::new()?;
/// let proxy = platform.proxy();
/// platform.run(App::new(proxy))?;
/// ```
pub struct Platform {
    event_loop: EventLoop<UserEvent>,
    proxy: PlatformProxy,
}

impl fmt::Debug for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Platform").finish_non_exhaustive()
    }
}

impl Platform {
    /// Creates the event loop.
    ///
    /// Must be called from the main thread on desktop platforms.
    pub fn new() -> Result<Self> {
        let event_loop = EventLoop::<UserEvent>::with_user_event()
            .build()
            .map_err(|error| UiError::EventLoop(error.to_string()))?;
        let proxy = PlatformProxy {
            proxy: event_loop.create_proxy(),
        };
        Ok(Self { event_loop, proxy })
    }

    /// A cloneable handle for posting events from other threads.
    pub fn proxy(&self) -> PlatformProxy {
        self.proxy.clone()
    }

    /// Runs the event loop until every window is closed or [`PlatformContext::exit`] is called.
    ///
    /// Consumes the platform, exactly like `winit` does.
    pub fn run<H: EventHandler + 'static>(self, handler: H) -> Result<()> {
        self.event_loop.set_control_flow(ControlFlow::Wait);
        let mut runner = Runner {
            handler,
            windows: RefCell::new(HashMap::new()),
            translators: HashMap::new(),
            started: false,
        };
        self.event_loop
            .run_app(&mut runner)
            .map_err(|error| UiError::EventLoop(error.to_string()))
    }
}

/// Bridges `winit`'s [`ApplicationHandler`] to a RustUI [`EventHandler`].
struct Runner<H> {
    handler: H,
    /// Windows known to the loop; the loop owns one handle each.
    windows: RefCell<HashMap<WindowId, WindowHandle>>,
    /// One stateful translator per window (DPI, modifiers, click timing).
    translators: HashMap<WindowId, EventTranslator>,
    started: bool,
}

impl<H: EventHandler> ApplicationHandler<UserEvent> for Runner<H> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let already_started = self.started;
        self.started = true;
        let ctx = PlatformContext {
            event_loop,
            windows: &self.windows,
        };
        let event = if already_started {
            PlatformEvent::Resume
        } else {
            PlatformEvent::Start
        };
        self.handler.on_event(&ctx, &EventTarget::Loop, event);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        let ctx = PlatformContext {
            event_loop,
            windows: &self.windows,
        };
        self.handler
            .on_event(&ctx, &EventTarget::Loop, PlatformEvent::Suspend);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        let ctx = PlatformContext {
            event_loop,
            windows: &self.windows,
        };
        let event = PlatformEvent::Ui(event.into_ui_event());
        self.handler.on_event(&ctx, &EventTarget::Loop, event);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        // Windows are registered by `PlatformContext::create_window`; events for windows we do
        // not know (already closed) are dropped.
        let Some(window) = self.windows.borrow().get(&id).cloned() else {
            return;
        };
        let ctx = PlatformContext {
            event_loop,
            windows: &self.windows,
        };
        match event {
            winit::event::WindowEvent::RedrawRequested => {
                let target = EventTarget::Window(window);
                self.handler.on_event(&ctx, &target, PlatformEvent::Redraw);
            }
            winit::event::WindowEvent::Destroyed => {
                let target = EventTarget::Window(window);
                self.handler
                    .on_event(&ctx, &target, PlatformEvent::WindowClosed);
                self.windows.borrow_mut().remove(&id);
                self.translators.remove(&id);
            }
            other => {
                let scale = window.scale_factor();
                let events = {
                    let translator = self
                        .translators
                        .entry(id)
                        .or_insert_with(|| EventTranslator::new(scale));
                    translator.translate(&other)
                };
                if events.is_empty() {
                    return;
                }
                let target = EventTarget::Window(window);
                for event in events {
                    self.handler
                        .on_event(&ctx, &target, PlatformEvent::Ui(event));
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let ctx = PlatformContext {
            event_loop,
            windows: &self.windows,
        };
        if self.handler.on_idle(&ctx) {
            for window in self.windows.borrow().values() {
                window.request_redraw();
            }
        }
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        let ctx = PlatformContext {
            event_loop,
            windows: &self.windows,
        };
        self.handler
            .on_event(&ctx, &EventTarget::Loop, PlatformEvent::Exit);
    }
}
