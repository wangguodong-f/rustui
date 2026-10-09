//! Application lifecycle: the wiring layer that puts `winit`, `wgpu` and `rustui-window`
//! together (design documents §9, §32, §41).
//!
//! ```text
//! winit::EventLoop ──> Platform ──> EventHandler (this module)
//!                                    ├── PlatformEvent::Start    create windows + run setup
//!                                    ├── PlatformEvent::Ui       WindowManager::handle_event
//!                                    └── PlatformEvent::Redraw   update → layout → paint → GPU
//! ```
//!
//! Nothing in `rustui-core`, `rustui-layout` or `rustui-widgets` sees a `winit` or `wgpu` type
//! (principle P1); only this module does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle as RawWindowHandle,
};
use rustui_core::{notify, CursorIcon, FrameClock, Rect, Result, TaskId, UiError, UiEvent};
use rustui_platform::{
    to_winit_cursor, EventHandler, EventTarget, Platform, PlatformContext, PlatformEvent,
    PlatformProxy, WindowHandle,
};
use rustui_renderer::{GpuContext, SurfaceRenderer};
use rustui_text::TextEngine;
use rustui_window::{Window, WindowId, WindowManager, WindowOptions};
use winit::window::WindowId as OsWindowId;

/// Task id used by [`spawn`]; the payload is the closure to run on the UI thread.
const SPAWN_TASK: TaskId = TaskId(0xC0_FF_EE_01);

/// A closure handed to [`spawn`] / [`AppContext::spawn`].
type SpawnTask = Box<dyn FnOnce() + Send>;

/// Per-frame hook run before `update`, handed the window manager it may mutate.
type UpdateHook = Box<dyn FnMut(&mut WindowManager)>;

thread_local! {
    /// Proxy of the application running on this thread, so [`spawn`] needs no context.
    static PROXY: RefCell<Option<PlatformProxy>> = const { RefCell::new(None) };
}

/// Runs `task` on the UI thread as soon as the event loop wakes up.
///
/// Returns `false` when no application is running or the loop is already gone. The task may not
/// touch widgets directly (`WindowManager` is not `Send`); it is meant for updating
/// [`SharedProperty`]s and models, which request a redraw on their own.
///
/// [`SharedProperty`]: rustui_core::SharedProperty
pub fn spawn(task: impl FnOnce() + Send + 'static) -> bool {
    PROXY.with(|slot| match slot.borrow().as_ref() {
        Some(proxy) => proxy.post_task(SPAWN_TASK.0, Box::new(task) as SpawnTask),
        None => false,
    })
}

fn install_proxy(proxy: &PlatformProxy) {
    PROXY.with(|slot| *slot.borrow_mut() = Some(proxy.clone()));
}

fn clear_proxy() {
    PROXY.with(|slot| *slot.borrow_mut() = None);
}

// ---------------------------------------------------------------------------
// window description
// ---------------------------------------------------------------------------

/// Description of a top level window, in logical pixels (§11).
///
/// The window is created by [`AppContext::open_window`] (or by [`Application::window`] before
/// the loop starts) and gets one [`WindowManager`](rustui_window::WindowManager) window of its
/// own, sharing the widget store, theme and text engine with every other window.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowSpec {
    /// Title shown by the window system.
    pub title: String,
    /// Client width in logical pixels.
    pub width: f32,
    /// Client height in logical pixels.
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
    /// `true` opens the window maximized.
    pub maximized: bool,
}

impl Default for WindowSpec {
    fn default() -> Self {
        Self {
            title: String::from("RustUI"),
            width: 1024.0,
            height: 768.0,
            min_width: None,
            min_height: None,
            resizable: true,
            decorated: true,
            visible: true,
            maximized: false,
        }
    }
}

impl WindowSpec {
    /// New spec with sensible defaults (1024x768, resizable, decorated).
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the window title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Sets the initial client size (logical pixels).
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Sets the smallest accepted client size (logical pixels).
    pub fn min_size(mut self, width: f32, height: f32) -> Self {
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

    /// Opens the window maximized.
    pub fn maximized(mut self, maximized: bool) -> Self {
        self.maximized = maximized;
        self
    }

    /// Options for the platform layer.
    fn platform_options(&self) -> rustui_platform::WindowOptions {
        let mut options = rustui_platform::WindowOptions::new()
            .with_title(self.title.clone())
            .with_size(self.width, self.height)
            .resizable(self.resizable)
            .decorated(self.decorated)
            .visible(self.visible)
            .maximized(self.maximized);
        if let (Some(width), Some(height)) = (self.min_width, self.min_height) {
            options = options.with_min_size(width, height);
        }
        options
    }
}

// ---------------------------------------------------------------------------
// surface target
// ---------------------------------------------------------------------------

/// A `winit` window handed to `wgpu`.
///
/// `wgpu` needs the raw window *and* display handles when it creates a surface, and both have to
/// be shareable across threads; wrapping the (reference counted) [`WindowHandle`] is what makes
/// that possible without leaking `winit` types anywhere else (P1).
#[derive(Clone)]
struct SurfaceTarget(WindowHandle);

impl std::fmt::Debug for SurfaceTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceTarget")
            .field("id", &self.0.id())
            .finish_non_exhaustive()
    }
}

impl HasWindowHandle for SurfaceTarget {
    fn window_handle(&self) -> std::result::Result<RawWindowHandle<'_>, HandleError> {
        self.0.inner().window_handle()
    }
}

impl HasDisplayHandle for SurfaceTarget {
    fn display_handle(&self) -> std::result::Result<DisplayHandle<'_>, HandleError> {
        self.0.inner().display_handle()
    }
}

// ---------------------------------------------------------------------------
// application
// ---------------------------------------------------------------------------

/// Entry point of a RustUI program.
///
/// ```ignore
/// fn main() -> rustui::Result<()> {
///     rustui::Application::new()?
///         .window(rustui::WindowSpec::new().title("Hello").size(480.0, 320.0))
///         .run(|app| {
///             let window = app.primary_window().expect("one window");
///             app.set_root(window, Box::new(Label::new("Hello, RustUI!")));
///         })
/// }
/// ```
pub struct Application {
    platform: Platform,
    proxy: PlatformProxy,
    specs: Vec<WindowSpec>,
}

impl std::fmt::Debug for Application {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Application")
            .field("windows", &self.specs.len())
            .finish_non_exhaustive()
    }
}

impl Application {
    /// Creates the event loop; must be called from the main thread on desktop platforms.
    pub fn new() -> Result<Self> {
        let platform = Platform::new()?;
        let proxy = platform.proxy();
        Ok(Self {
            platform,
            proxy,
            specs: Vec::new(),
        })
    }

    /// A cloneable handle for posting events back to this thread ([`spawn`]).
    pub fn proxy(&self) -> PlatformProxy {
        self.proxy.clone()
    }

    /// Registers a window to be created when the loop starts.
    pub fn window(mut self, spec: WindowSpec) -> Self {
        self.specs.push(spec);
        self
    }

    /// Creates the registered windows, runs `setup` and enters the event loop.
    ///
    /// `setup` runs once, after the windows exist and before the first frame: it is where the
    /// application builds its widget trees. The call returns when every window is closed.
    pub fn run<F>(mut self, setup: F) -> Result<()>
    where
        F: FnOnce(&mut AppContext<'_>) + 'static,
    {
        let mut wm = WindowManager::new(Box::new(TextEngine::new()));
        wm.ui_mut()
            .set_clipboard(Some(Box::new(rustui_platform::PlatformClipboard::new())));
        // Any code on this thread can now ask for a repaint (`SharedProperty::set`, animations).
        notify::set_redraw_sink(wm.ui().redraw().clone());
        install_proxy(&self.proxy);

        let now = Instant::now();
        let state = State {
            wm,
            gpu: None,
            renderers: HashMap::new(),
            os_to_ui: HashMap::new(),
            ui_to_os: HashMap::new(),
            cursors: HashMap::new(),
            ime: HashMap::new(),
            proxy: self.proxy.clone(),
            on_task: None,
            on_custom: None,
            on_update: None,
            start: now,
            last: now,
            frame: 0,
        };
        let handler = Handler {
            state,
            specs: std::mem::take(&mut self.specs),
            setup: Some(setup),
        };

        let result = self.platform.run(handler);
        notify::clear_redraw_sink();
        clear_proxy();
        result
    }
}

// ---------------------------------------------------------------------------
// setup context
// ---------------------------------------------------------------------------

/// What the setup closure gets: the shared services plus the ability to open windows.
pub struct AppContext<'a> {
    state: &'a mut State,
    ctx: &'a PlatformContext<'a>,
}

impl AppContext<'_> {
    /// Shared services and every window of the application.
    pub fn wm(&mut self) -> &mut WindowManager {
        &mut self.state.wm
    }

    /// Theme, text engine, clipboard and redraw flag.
    pub fn ui(&self) -> &rustui_core::UiContext {
        self.state.wm.ui()
    }

    /// Same, for switching the theme or the clipboard at runtime.
    pub fn ui_mut(&mut self) -> &mut rustui_core::UiContext {
        self.state.wm.ui_mut()
    }

    /// Registers the callback for background tasks started with [`PlatformProxy::post_task`].
    ///
    /// It runs on the UI thread, so it may touch widgets and `SharedProperty`s; the payload is
    /// the `Send` value the task returned. Replaces a previously registered callback.
    pub fn on_task(
        &mut self,
        callback: impl FnMut(TaskId, rustui_core::event::TaskPayload) + 'static,
    ) {
        self.state.on_task = Some(Box::new(callback));
    }

    /// Registers the application tick: it runs before every frame, right on the UI thread, and
    /// is the place where pending work (closing a dialog, applying a theme) can reach the
    /// [`WindowManager`] from a signal callback that cannot.
    pub fn on_update(&mut self, callback: impl FnMut(&mut WindowManager) + 'static) {
        self.state.on_update = Some(Box::new(callback));
    }

    /// Registers the callback for [`PlatformProxy::post`] events (same thread as [`on_task`]).
    pub fn on_custom(
        &mut self,
        callback: impl FnMut(&'static str, rustui_core::event::TaskPayload) + 'static,
    ) {
        self.state.on_custom = Some(Box::new(callback));
    }

    /// Every window id, in creation order.
    pub fn window_ids(&self) -> Vec<WindowId> {
        self.state.wm.window_ids()
    }

    /// The first window created, which applications with a single window use.
    pub fn primary_window(&self) -> Option<WindowId> {
        self.state.wm.primary_window()
    }

    /// Creates an additional top level window and returns its id.
    pub fn open_window(&mut self, spec: WindowSpec) -> Result<WindowId> {
        self.state.open_window(self.ctx, spec)
    }

    /// Installs `widget` as the content root of `window` and returns its id.
    pub fn set_root(
        &mut self,
        window: WindowId,
        widget: Box<dyn rustui_core::Widget>,
    ) -> rustui_core::ObjectId {
        self.state.wm.set_root(window, widget)
    }

    /// Closes `window`; the loop stops once the last one is gone.
    pub fn close_window(&mut self, window: WindowId) {
        self.state.close_window(self.ctx, window);
    }

    /// Stops the event loop after the current iteration.
    pub fn exit(&self) {
        self.ctx.exit();
    }

    /// A cloneable handle for posting events back to the UI thread.
    pub fn proxy(&self) -> PlatformProxy {
        self.state.proxy.clone()
    }

    /// Runs `task` on the UI thread soon (see the free function [`spawn`]).
    pub fn spawn(&self, task: impl FnOnce() + Send + 'static) -> bool {
        self.state
            .proxy
            .post_task(SPAWN_TASK.0, Box::new(task) as SpawnTask)
    }
}

// ---------------------------------------------------------------------------
// shared per application state
// ---------------------------------------------------------------------------

struct State {
    wm: WindowManager,
    /// Created with the first window: the instance then carries that window's display handle.
    gpu: Option<GpuContext>,
    renderers: HashMap<WindowId, SurfaceRenderer>,
    os_to_ui: HashMap<OsWindowId, (WindowHandle, WindowId)>,
    ui_to_os: HashMap<WindowId, OsWindowId>,
    cursors: HashMap<WindowId, CursorIcon>,
    /// Input method state last pushed to the platform per window: `(composition allowed, area)`.
    /// Kept so a frame where nothing changed issues no `winit` call at all.
    ime: HashMap<WindowId, (bool, Option<Rect>)>,
    proxy: PlatformProxy,
    /// Called on the UI thread for every finished background task but [`spawn`]'s own.
    on_task: Option<Box<dyn FnMut(TaskId, rustui_core::event::TaskPayload)>>,
    /// Called on the UI thread for every [`PlatformProxy::post`] event.
    on_custom: Option<Box<dyn FnMut(&'static str, rustui_core::event::TaskPayload)>>,
    /// Called before every frame with the shared services (§15: animations and pending work).
    on_update: Option<UpdateHook>,
    start: Instant,
    last: Instant,
    frame: u64,
}

impl State {
    // ---- windows ---------------------------------------------------------

    fn open_window(&mut self, ctx: &PlatformContext<'_>, spec: WindowSpec) -> Result<WindowId> {
        let handle = ctx.create_window(&spec.platform_options())?;
        // Input methods are on by default: text fields expect them (§40). `sync_ime` turns them
        // off again whenever a widget that does not edit text owns focus.
        handle.set_ime_allowed(true);

        match self.create_ui_window(&handle, &spec) {
            Ok(id) => {
                self.os_to_ui.insert(handle.id(), (handle, id));
                Ok(id)
            }
            Err(error) => {
                // Nothing usable came out of it: do not leave a stray OS window behind.
                ctx.destroy_window(&handle);
                Err(error)
            }
        }
    }

    /// GPU instance, UI window and render target for `handle`.
    fn create_ui_window(&mut self, handle: &WindowHandle, spec: &WindowSpec) -> Result<WindowId> {
        if self.gpu.is_none() {
            let descriptor = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
                SurfaceTarget(handle.clone()),
            ));
            self.gpu = Some(GpuContext::with_descriptor(descriptor)?);
        }

        let scale = handle.scale_factor();
        let size = handle.size();
        let options = WindowOptions::new()
            .title(spec.title.clone())
            .size(size.width, size.height)
            .scale_factor(scale);
        let id = self.wm.create_window(options);
        self.ui_to_os.insert(id, handle.id());
        self.cursors.insert(id, CursorIcon::Default);
        // Matches the `set_ime_allowed(true)` done when the OS window was created; the first
        // `sync_ime` pushes the real state.
        self.ime.insert(id, (true, None));

        let [width, height] = self
            .wm
            .window(id)
            .map(Window::physical_size)
            .unwrap_or([1, 1]);
        let Some(gpu) = self.gpu.as_ref() else {
            return Err(UiError::GpuUnavailable("no GPU context".to_string()));
        };
        let surface = gpu.create_surface(SurfaceTarget(handle.clone()))?;
        let renderer = SurfaceRenderer::new(gpu, surface, width, height)?;
        self.renderers.insert(id, renderer);
        Ok(id)
    }

    /// Closes `id`: drops the OS window, the render target and the UI window.
    ///
    /// The loop exits once no window is left.
    fn close_window(&mut self, ctx: &PlatformContext<'_>, id: WindowId) {
        if let Some(os_id) = self.ui_to_os.remove(&id) {
            if let Some((handle, _)) = self.os_to_ui.remove(&os_id) {
                ctx.destroy_window(&handle);
            }
        }
        self.renderers.remove(&id);
        self.cursors.remove(&id);
        self.ime.remove(&id);
        self.wm.destroy_window(id);
        if self.wm.windows().is_empty() {
            ctx.exit();
        }
    }

    // ---- input -----------------------------------------------------------

    fn on_ui_event(&mut self, ctx: &PlatformContext<'_>, target: &EventTarget, event: UiEvent) {
        let Some(handle) = target.window() else {
            self.on_posted_event(ctx, event);
            return;
        };
        let Some(&(_, id)) = self.os_to_ui.get(&handle.id()) else {
            return;
        };

        let outcome = self.wm.handle_event(id, event);
        self.sync_cursor(id);
        self.sync_ime(id);
        if outcome.close_requested || self.wm.take_close_request(id) {
            self.close_window(ctx, id);
        } else if outcome.request_redraw {
            handle.request_redraw();
        }
    }

    /// An event posted from this or another thread ([`spawn`], [`PlatformProxy::post`]).
    ///
    /// Posted events have no window: they go to the [`AppContext::on_task`] /
    /// [`AppContext::on_custom`] callbacks, and anything left over is offered to the primary
    /// window so a widget can react to [`UiEvent::Custom`].
    fn on_posted_event(&mut self, _ctx: &PlatformContext<'_>, event: UiEvent) {
        match event {
            UiEvent::TaskCompleted { task, result } if task == SPAWN_TASK => {
                run_spawn(result);
                self.request_redraw_all();
            }
            UiEvent::TaskCompleted { task, result } => {
                if let Some(callback) = self.on_task.as_mut() {
                    callback(task, result);
                    self.request_redraw_all();
                }
            }
            UiEvent::Custom { name, payload } => {
                if let Some(callback) = self.on_custom.as_mut() {
                    callback(name, payload);
                    self.request_redraw_all();
                } else if let Some(primary) = self.wm.primary_window() {
                    // No application callback: offer it to the widgets of the primary window.
                    let outcome = self
                        .wm
                        .handle_event(primary, UiEvent::Custom { name, payload });
                    if outcome.request_redraw {
                        self.request_redraw_all();
                    }
                }
            }
            other => {
                if let Some(primary) = self.wm.primary_window() {
                    let outcome = self.wm.handle_event(primary, other);
                    if outcome.request_redraw {
                        self.request_redraw_all();
                    }
                }
            }
        }
    }

    fn request_redraw_all(&self) {
        for (handle, _) in self.os_to_ui.values() {
            handle.request_redraw();
        }
    }

    fn sync_cursor(&mut self, id: WindowId) {
        let cursor = match self.wm.window(id) {
            Some(window) => window.cursor(),
            None => return,
        };
        if self.cursors.get(&id).copied() == Some(cursor) {
            return;
        }
        self.cursors.insert(id, cursor);
        if let Some(handle) = self.handle_of(id) {
            handle.inner().set_cursor(to_winit_cursor(cursor));
        }
    }

    /// Platform window handle of `id`, if the window still exists.
    fn handle_of(&self, id: WindowId) -> Option<&WindowHandle> {
        let os_id = self.ui_to_os.get(&id)?;
        self.os_to_ui.get(os_id).map(|(handle, _)| handle)
    }

    /// Pushes the input method state of `id` to the platform (T-1).
    ///
    /// The caret rectangle comes from the widget owning keyboard focus (`WindowManager::ime_area`),
    /// so it follows every layout change; `None` switches composition off, which is what keeps an
    /// input method from grabbing keys while a non text widget owns focus. Sending the area only
    /// makes sense once composition is allowed, hence `allowed` first.
    fn sync_ime(&mut self, id: WindowId) {
        let area = self.wm.ime_area(id);
        let state = (area.is_some(), area);
        if self.ime.get(&id) == Some(&state) {
            return;
        }
        self.ime.insert(id, state);
        let (allowed, area) = state;
        if let Some(handle) = self.handle_of(id) {
            handle.set_ime_allowed(allowed);
            if let Some(area) = area {
                handle.set_ime_cursor_area(area);
            }
        }
    }

    // ---- frames ----------------------------------------------------------

    fn next_clock(&mut self) -> FrameClock {
        let now = Instant::now();
        let delta = now.duration_since(self.last);
        let elapsed = now.duration_since(self.start);
        self.last = now;
        self.frame = self.frame.wrapping_add(1);
        FrameClock {
            frame: self.frame,
            delta,
            elapsed,
        }
    }

    /// One complete frame: `update → layout → paint → GPU` (§32).
    fn run_frame(&mut self, id: WindowId) {
        if let Some(update) = self.on_update.as_mut() {
            update(&mut self.wm);
        }
        let clock = self.next_clock();
        self.wm.update(id, clock);
        self.wm.layout(id);
        self.wm.paint(id);
        // `render_input` switches the shared text engine to this window's display scale (C-048),
        // so every window rasterizes glyphs at its own DPI.
        self.render(id);
        self.sync_cursor(id);
        // After `layout`, so the caret rectangle reflects the geometry just painted.
        self.sync_ime(id);
    }

    fn render(&mut self, id: WindowId) {
        let size = match self.wm.window(id) {
            Some(window) => window.size(),
            None => return,
        };
        // Minimized window: nothing to draw and a zero sized surface is invalid.
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        let [width, height] = self
            .wm
            .window(id)
            .map(Window::physical_size)
            .unwrap_or([1, 1]);
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };

        if !self.renderers.contains_key(&id) {
            let Some(handle) = self.handle_of(id) else {
                return;
            };
            let handle = handle.clone();
            let surface = match gpu.create_surface(SurfaceTarget(handle)) {
                Ok(surface) => surface,
                Err(error) => {
                    log::error!("surface creation failed: {error}");
                    return;
                }
            };
            match SurfaceRenderer::new(gpu, surface, width, height) {
                Ok(renderer) => {
                    self.renderers.insert(id, renderer);
                }
                Err(error) => {
                    log::error!("renderer creation failed: {error}");
                    return;
                }
            }
        }
        if let Some(renderer) = self.renderers.get_mut(&id) {
            renderer.resize(gpu, width, height);
        }

        let Some(input) = self.wm.render_input(id) else {
            return;
        };
        if let Some(renderer) = self.renderers.get_mut(&id) {
            if let Err(error) = renderer.render(gpu, &input) {
                log::error!("frame failed: {error}");
            }
        }
    }
}

/// Runs the closure carried by a [`spawn`] payload; `false` when it was something else.
fn run_spawn(payload: rustui_core::event::TaskPayload) -> bool {
    let Ok(task) = payload.downcast::<SpawnTask>() else {
        return false;
    };
    // Unique reference: the closure can be moved out and called.
    match Rc::try_unwrap(task) {
        Ok(task) => {
            task();
            true
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// event loop glue
// ---------------------------------------------------------------------------

struct Handler<F> {
    state: State,
    /// Windows requested through [`Application::window`], created on start.
    specs: Vec<WindowSpec>,
    setup: Option<F>,
}

impl<F> EventHandler for Handler<F>
where
    F: FnOnce(&mut AppContext<'_>) + 'static,
{
    fn on_event(&mut self, ctx: &PlatformContext<'_>, target: &EventTarget, event: PlatformEvent) {
        match event {
            PlatformEvent::Start => {
                for spec in std::mem::take(&mut self.specs) {
                    if let Err(error) = self.state.open_window(ctx, spec) {
                        log::error!("window creation failed: {error}");
                    }
                }
                if let Some(setup) = self.setup.take() {
                    let mut app = AppContext {
                        state: &mut self.state,
                        ctx,
                    };
                    setup(&mut app);
                }
                self.state.request_redraw_all();
            }
            PlatformEvent::Ui(event) => self.state.on_ui_event(ctx, target, event),
            PlatformEvent::Redraw => {
                if let Some(handle) = target.window() {
                    if let Some(&(_, id)) = self.state.os_to_ui.get(&handle.id()) {
                        self.state.run_frame(id);
                    }
                }
            }
            PlatformEvent::WindowClosed => {
                if let Some(handle) = target.window() {
                    if let Some(&(_, id)) = self.state.os_to_ui.get(&handle.id()) {
                        self.state.close_window(ctx, id);
                    }
                }
            }
            PlatformEvent::Resume | PlatformEvent::Suspend => {}
            PlatformEvent::Exit => {
                // The loop is gone; drop GPU resources while the windows still exist.
                self.state.renderers.clear();
            }
        }
    }

    fn on_idle(&mut self, _ctx: &PlatformContext<'_>) -> bool {
        // A widget (or a `SharedProperty`) asked for a repaint while we were waiting.
        self.state.wm.take_redraw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_spec_builds_platform_options() {
        let spec = WindowSpec::new()
            .title("Demo")
            .size(640.0, 480.0)
            .min_size(320.0, 240.0)
            .resizable(false)
            .decorated(false);
        let options = spec.platform_options();
        assert_eq!(options.title, "Demo");
        assert_eq!((options.width, options.height), (640.0, 480.0));
        assert_eq!(
            (options.min_width, options.min_height),
            (Some(320.0), Some(240.0))
        );
        assert!(!options.resizable);
        assert!(!options.decorated);
        assert_eq!(WindowSpec::new().width, 1024.0);
    }

    #[test]
    fn spawn_without_an_application_is_a_no_op() {
        clear_proxy();
        assert!(!spawn(|| {}));
    }

    #[test]
    fn spawn_payload_runs_the_closure() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let flag = Arc::new(AtomicU32::new(0));
        let payload: Rc<dyn std::any::Any> = Rc::new(Box::new({
            let flag = flag.clone();
            move || {
                flag.fetch_add(1, Ordering::Relaxed);
            }
        }) as SpawnTask);
        assert!(run_spawn(payload));
        assert_eq!(flag.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn foreign_payloads_are_ignored() {
        assert!(!run_spawn(Rc::new(7u32)));
    }
}
