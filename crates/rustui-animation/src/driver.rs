//! The frame driven animation driver (§33).
//!
//! Widgets cannot own the frame loop, and the frame loop should not know about widget state. The
//! driver is the seam between them: it lives in the application next to the
//! [`RedrawFlag`], owns an [`AnimationClock`] and advances every
//! registered animation or transition by one frame delta.
//!
//! # Why [`AnimationHandle`] uses `Rc` interior mutability
//!
//! `AnimationDriver::tick` takes `&mut self` — it is called by the UI thread, which owns the driver,
//! right next to [`request_redraw`](rustui_core::notify::request_redraw). But a widget that wants
//! to start a hover transition while it is being painted only has `&self` plus a handle to the
//! application context. Rather than threading `&mut` through the entire widget tree (which the
//! retained-mode object model of §52 deliberately avoids — widgets address each other by
//! [`ObjectId`](rustui_core::ObjectId), never by pointer), the *handle* is an `Rc` to the driver's
//! command queue and animation registry with a `RefCell` inside.
//!
//! This is the one place where `Rc<RefCell<..>>` is acceptable, and it stays acceptable because:
//!
//! * the shared state is a **flat registry plus a queue**, not the widget tree — no widget holds a
//!   pointer to another widget, so there are no reference cycles and no `Weak` lifetime puzzles
//!   (§52),
//! * it is **UI thread only** (`Rc`, not `Arc`; no locks, no atomics, no cross-thread access),
//! * re-entrancy is contained: a command queued from inside an `on_update` callback is processed on
//!   the *next* tick, so the registry is never mutated while it is being iterated,
//! * no reference into the registry ever escapes — every accessor borrows it for the length of one
//!   expression and gives a copy back, so no caller can hold it across a mutation.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use rustui_core::notify::RedrawFlag;

use crate::animation::{Animation, AnimationState, Transition};
use crate::clock::{AnimationClock, FrameClock};
use crate::value::Animatable;

/// Stable handle for an animation or transition registered with an [`AnimationDriver`].
///
/// Ids are handed out by the driver in ascending order and are never reused, so a stale id in a
/// delayed command can never address a different animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnimationId(u64);

impl AnimationId {
    /// The raw counter value, mainly for logging and tests.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for AnimationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A type-erased animation or transition registered with the driver.
enum Driven {
    /// A free standing animation whose values go to its own `on_update` callback.
    Animation(Box<dyn Runner>),
    /// A transition whose values go into a `SharedProperty`.
    Transition(Box<dyn Runner>),
}

/// Type-erased update interface for both [`Animation`] and [`Transition`].
trait Runner {
    /// Advances by `delta`; returns `true` while still animating.
    fn update(&mut self, delta: Duration) -> bool;
    /// `true` while it should receive frame deltas.
    fn is_running(&self) -> bool;
    /// `true` when it can never run again (finished or cancelled).
    fn is_terminal(&self) -> bool;
    /// The current state, for diagnostics.
    fn state(&self) -> AnimationState;
    /// Restarts from the beginning.
    fn restart(&mut self);
}

impl<T: Animatable> Runner for Animation<T> {
    fn update(&mut self, delta: Duration) -> bool {
        Animation::update(self, delta)
    }

    fn is_running(&self) -> bool {
        Animation::is_running(self)
    }

    fn is_terminal(&self) -> bool {
        Animation::state(self).is_terminal()
    }

    fn state(&self) -> AnimationState {
        Animation::state(self)
    }

    fn restart(&mut self) {
        Animation::restart(self);
    }
}

impl<T: Animatable> Runner for Transition<T> {
    fn update(&mut self, delta: Duration) -> bool {
        Transition::update(self, delta)
    }

    fn is_running(&self) -> bool {
        Transition::is_running(self)
    }

    fn is_terminal(&self) -> bool {
        Transition::state(self).is_terminal()
    }

    fn state(&self) -> AnimationState {
        Transition::state(self)
    }

    fn restart(&mut self) {
        Transition::restart(self);
    }
}

impl Runner for Driven {
    fn update(&mut self, delta: Duration) -> bool {
        match self {
            Driven::Animation(runner) => runner.update(delta),
            Driven::Transition(runner) => runner.update(delta),
        }
    }

    fn is_running(&self) -> bool {
        match self {
            Driven::Animation(runner) => runner.is_running(),
            Driven::Transition(runner) => runner.is_running(),
        }
    }

    fn is_terminal(&self) -> bool {
        match self {
            Driven::Animation(runner) => runner.is_terminal(),
            Driven::Transition(runner) => runner.is_terminal(),
        }
    }

    fn state(&self) -> AnimationState {
        match self {
            Driven::Animation(runner) => runner.state(),
            Driven::Transition(runner) => runner.state(),
        }
    }

    fn restart(&mut self) {
        match self {
            Driven::Animation(runner) => runner.restart(),
            Driven::Transition(runner) => runner.restart(),
        }
    }
}

/// Shared, type-erased registry of animations and transitions.
#[derive(Default)]
struct Registry {
    next_id: u64,
    entries: Vec<(AnimationId, Driven)>,
}

impl Registry {
    fn push(&mut self, driven: Driven) -> AnimationId {
        self.next_id += 1;
        let id = AnimationId(self.next_id);
        self.entries.push((id, driven));
        id
    }

    fn remove(&mut self, id: AnimationId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(entry_id, _)| *entry_id != id);
        self.entries.len() != before
    }

    fn contains(&self, id: AnimationId) -> bool {
        self.entries.iter().any(|(entry_id, _)| *entry_id == id)
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn running_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|(_, driven)| driven.is_running())
            .count()
    }

    /// Advances every running entry and drops the ones that can never run again.
    fn tick(&mut self, delta: Duration) {
        for (_, driven) in self.entries.iter_mut() {
            if driven.is_running() {
                driven.update(delta);
            }
        }
        self.entries.retain(|(_, driven)| !driven.is_terminal());
    }

    fn restart(&mut self, id: AnimationId) -> bool {
        for (entry_id, driven) in self.entries.iter_mut() {
            if *entry_id == id {
                driven.restart();
                return true;
            }
        }
        false
    }

    fn states(&self) -> Vec<AnimationState> {
        self.entries
            .iter()
            .map(|(_, driven)| driven.state())
            .collect()
    }
}

/// The state both the driver and its handles share.
#[derive(Default)]
struct Shared {
    inbox: Vec<DriverCommand>,
    registry: Registry,
}

impl Shared {
    /// Consumes the queued commands and advances the registry by one frame.
    ///
    /// Returns `true` while at least one entry is still running.
    fn tick(&mut self, clock: FrameClock, redraw: &RedrawFlag) -> bool {
        let mut tick = clock;
        for command in std::mem::take(&mut self.inbox) {
            match command {
                // The last queued frame wins: this is how a fixed-step simulation or a replay hands
                // its own timing to the driver.
                DriverCommand::Tick(next) => tick = next,
                DriverCommand::Add(id) => {
                    // The payload is already in the registry; announcing it schedules the frame its
                    // first value needs.
                    if self.registry.contains(id) {
                        redraw.request();
                    }
                }
                DriverCommand::Remove(id) => {
                    self.registry.remove(id);
                }
            }
        }
        self.registry.tick(tick.delta);
        self.registry.running_count() > 0
    }
}

/// A command queued by an [`AnimationHandle`] for the next [`AnimationDriver::tick`].
///
/// Commands exist so that a widget can steer the animation system through a shared handle without
/// holding `&mut` to the driver, and so that requests made from inside an `on_update` callback are
/// applied *after* the current frame instead of re-entering the registry that is being iterated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DriverCommand {
    /// Advance every registered animation/transition using this frame instead of the one the driver
    /// measured; the last queued frame wins.
    Tick(FrameClock),
    /// Announce that the animation/transition with this id was just registered, so the driver
    /// schedules the frame its first value needs.
    Add(AnimationId),
    /// Drop the animation/transition with this id before the next frame is written.
    Remove(AnimationId),
}

/// A cheaply cloneable handle to an [`AnimationDriver`].
///
/// Cloning a handle shares the same driver; the last clone keeps the driver alive. See the
/// [module documentation](crate::driver) for why this is the only `Rc`-based piece of the crate.
#[derive(Clone, Default)]
pub struct AnimationHandle {
    shared: Rc<RefCell<Shared>>,
    redraw: RedrawFlag,
}

impl AnimationHandle {
    fn new(shared: Rc<RefCell<Shared>>, redraw: RedrawFlag) -> Self {
        Self { shared, redraw }
    }

    /// Registers an animation and returns its id.
    ///
    /// The animation is started if it has not been started yet, so the common case is a one-liner:
    ///
    /// ```no_run
    /// # use std::time::Duration;
    /// # use rustui_animation::{Animation, AnimationDriver};
    /// # use rustui_core::notify::RedrawFlag;
    /// let driver = AnimationDriver::new(RedrawFlag::new());
    /// let handle = driver.handle();
    /// let id = handle.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(200)));
    /// assert!(!handle.remove(id));
    /// ```
    ///
    /// A repaint is requested immediately so the first frame of the animation is not missed. The
    /// registration takes effect at once; use [`send`](Self::send) with a
    /// [`DriverCommand::Remove`] to defer a removal to the next frame.
    pub fn add_animation<T: Animatable>(&self, mut animation: Animation<T>) -> AnimationId {
        if !animation.is_started() {
            animation.start();
        }
        let id = self
            .shared
            .borrow_mut()
            .registry
            .push(Driven::Animation(Box::new(animation)));
        self.redraw.request();
        id
    }

    /// Registers a property transition and returns its id.
    ///
    /// The transition writes into its [`SharedProperty`](rustui_core::SharedProperty) on every
    /// frame and is repainted by the same mechanism as [`add_animation`](Self::add_animation).
    pub fn add_transition<T: Animatable>(&self, transition: Transition<T>) -> AnimationId {
        let id = self
            .shared
            .borrow_mut()
            .registry
            .push(Driven::Transition(Box::new(transition)));
        self.redraw.request();
        id
    }

    /// Removes a registered animation or transition; returns `true` if it was still registered.
    ///
    /// The removal is immediate, so the animation stops receiving frame deltas right away.
    pub fn remove(&self, id: AnimationId) -> bool {
        self.shared.borrow_mut().registry.remove(id)
    }

    /// Restarts a registered animation from the beginning; returns `true` if it was found.
    pub fn restart(&self, id: AnimationId) -> bool {
        let restarted = self.shared.borrow_mut().registry.restart(id);
        if restarted {
            self.redraw.request();
        }
        restarted
    }

    /// `true` while the id is registered.
    pub fn contains(&self, id: AnimationId) -> bool {
        self.shared.borrow().registry.contains(id)
    }

    /// Number of registered animations and transitions (running or paused).
    pub fn len(&self) -> usize {
        self.shared.borrow().registry.len()
    }

    /// `true` when nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.shared.borrow().registry.is_empty()
    }

    /// Number of entries that are currently advancing.
    pub fn running_count(&self) -> usize {
        self.shared.borrow().registry.running_count()
    }

    /// The redraw flag of the driver this handle belongs to.
    pub fn redraw(&self) -> &RedrawFlag {
        &self.redraw
    }

    /// Advances every registered animation/transition by `clock.delta()`.
    ///
    /// Returns `true` while at least one animation is still running. This is what the driver calls;
    /// widgets normally never do.
    pub fn tick(&self, clock: FrameClock) -> bool {
        self.shared.borrow_mut().tick(clock, &self.redraw)
    }

    /// Queues a [`DriverCommand`] to be applied at the beginning of the next
    /// [`AnimationDriver::tick`].
    ///
    /// Queuing from inside an `on_update` callback is the supported way to chain animations: the
    /// mutation is applied after the current frame finished iterating the registry.
    pub fn send(&self, command: DriverCommand) {
        self.shared.borrow_mut().inbox.push(command);
    }

    /// The commands queued so far but not yet consumed by a tick.
    pub fn pending_commands(&self) -> Vec<DriverCommand> {
        self.shared.borrow().inbox.clone()
    }
}

impl fmt::Debug for AnimationHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shared = self.shared.borrow();
        f.debug_struct("AnimationHandle")
            .field("registered", &shared.registry.len())
            .field("pending_commands", &shared.inbox.len())
            .finish()
    }
}

/// Owns the animations and transitions of an application and advances them once per frame.
///
/// Typical use in the UI thread's frame loop:
///
/// ```no_run
/// # use rustui_animation::AnimationDriver;
/// # use rustui_core::notify::RedrawFlag;
/// let mut driver = AnimationDriver::new(RedrawFlag::new());
/// // ... hand `driver.handle()` to the widgets that need it ...
/// loop {
///     if driver.needs_frame() {
///         driver.tick(); // advances every animation and writes its property values
///     }
///     // ... paint the widget tree if `driver.redraw().take()` ...
/// }
/// ```
#[derive(Default)]
pub struct AnimationDriver {
    clock: AnimationClock,
    shared: Rc<RefCell<Shared>>,
    redraw: RedrawFlag,
}

impl AnimationDriver {
    /// Creates a driver that requests repaints through `redraw`.
    pub fn new(redraw: RedrawFlag) -> Self {
        Self {
            clock: AnimationClock::new(),
            shared: Rc::new(RefCell::new(Shared::default())),
            redraw,
        }
    }

    /// A cloneable handle that widgets can use to register animations without `&mut self`.
    pub fn handle(&self) -> AnimationHandle {
        AnimationHandle::new(Rc::clone(&self.shared), self.redraw.clone())
    }

    /// `true` while at least one animation or transition is running and needs more frames.
    ///
    /// The frame loop should keep scheduling frames while this is `true`; when it is `false` the
    /// animation system is idle and costs nothing.
    pub fn needs_frame(&self) -> bool {
        self.shared.borrow().registry.running_count() > 0
    }

    /// Advances every registered animation/transition by one real frame and returns the frame.
    ///
    /// The driver also requests a repaint when animations are still running, so an application that
    /// only repaints on [`RedrawFlag::take`] keeps painting until everything settled.
    pub fn tick(&mut self) -> FrameClock {
        let clock = self.clock.tick();
        self.advance(clock);
        clock
    }

    /// Advances every registered animation/transition by `clock.delta()` and returns whether
    /// anything is still animating.
    ///
    /// Identical to [`tick`](Self::tick) except that the caller supplies the timing, which is what
    /// tests and fixed-step simulations use.
    pub fn advance(&mut self, clock: FrameClock) -> bool {
        let running = self.shared.borrow_mut().tick(clock, &self.redraw);
        if running {
            self.redraw.request();
        }
        running
    }

    /// The repaint flag this driver requests through.
    pub fn redraw(&self) -> &RedrawFlag {
        &self.redraw
    }

    /// The underlying frame clock (frame counter and elapsed time).
    pub fn clock(&self) -> &AnimationClock {
        &self.clock
    }

    /// Number of registered animations and transitions.
    pub fn len(&self) -> usize {
        self.shared.borrow().registry.len()
    }

    /// `true` when nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.shared.borrow().registry.is_empty()
    }

    /// Number of entries that are currently advancing.
    pub fn running_count(&self) -> usize {
        self.shared.borrow().registry.running_count()
    }

    /// The state of every registered entry, in registration order (for a debug overlay).
    pub fn states(&self) -> Vec<AnimationState> {
        self.shared.borrow().registry.states()
    }

    /// Drops every animation and transition and restarts the frame clock.
    pub fn clear(&mut self) {
        let mut shared = self.shared.borrow_mut();
        shared.registry = Registry::default();
        shared.inbox.clear();
        self.clock.reset();
    }
}

impl fmt::Debug for AnimationDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shared = self.shared.borrow();
        f.debug_struct("AnimationDriver")
            .field("frame", &self.clock.frame())
            .field("registered", &shared.registry.len())
            .field("running", &shared.registry.running_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn clock(frame: u64, delta_ms: u64) -> FrameClock {
        FrameClock {
            frame,
            delta: Duration::from_millis(delta_ms),
            elapsed: Duration::from_millis(delta_ms * frame),
        }
    }

    #[test]
    fn a_fresh_driver_is_idle() {
        let driver = AnimationDriver::new(RedrawFlag::new());
        assert!(!driver.needs_frame());
        assert!(driver.is_empty());
        assert_eq!(driver.len(), 0);
        assert_eq!(driver.running_count(), 0);
        assert!(driver.states().is_empty());
    }

    #[test]
    fn needs_frame_is_true_while_an_animation_runs() {
        let mut driver = AnimationDriver::new(RedrawFlag::new());
        let handle = driver.handle();
        assert!(!driver.needs_frame());
        handle.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(100)));
        assert!(driver.needs_frame());
        assert_eq!(driver.running_count(), 1);
        assert!(driver.advance(clock(1, 50)), "50 ms of a 100 ms animation");
        assert!(driver.needs_frame());
        assert!(!driver.advance(clock(2, 50)), "the animation finished");
        assert!(!driver.needs_frame());
        assert!(driver.is_empty(), "finished entries are dropped");
    }

    #[test]
    fn the_driver_advances_an_animation_to_its_target() {
        let mut driver = AnimationDriver::new(RedrawFlag::new());
        let handle = driver.handle();
        let seen = Rc::new(Cell::new(0.0f32));
        let sink = seen.clone();
        handle.add_animation(
            Animation::new(0.0f32, 1.0, Duration::from_millis(100)).on_update(move |v| sink.set(v)),
        );

        driver.advance(clock(1, 50));
        assert!((seen.get() - 0.5).abs() < 1e-6);
        assert!(driver.needs_frame());
        driver.advance(clock(2, 50));
        assert_eq!(seen.get(), 1.0);
        assert!(!driver.needs_frame());
    }

    #[test]
    fn the_driver_writes_transition_values_into_the_property() {
        let property = rustui_core::SharedProperty::new(0.0f32);
        let mut driver = AnimationDriver::new(RedrawFlag::new());
        let handle = driver.handle();
        handle.add_transition(Transition::new(
            property.clone(),
            10.0,
            Duration::from_millis(100),
        ));
        driver.advance(clock(1, 50));
        assert!(
            (property.get() - 5.0).abs() < 1e-4,
            "got {}",
            property.get()
        );
        driver.advance(clock(2, 50));
        assert_eq!(property.get(), 10.0);
        assert!(driver.is_empty());
    }

    #[test]
    fn adding_an_animation_requests_a_repaint() {
        let redraw = RedrawFlag::new();
        let mut driver = AnimationDriver::new(redraw.clone());
        let handle = driver.handle();
        assert!(!redraw.take());
        handle.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(100)));
        assert!(redraw.take(), "the driver must ask for a repaint");
        driver.advance(clock(1, 16));
        assert!(redraw.take(), "running animations keep repainting");
        driver.advance(clock(2, 200));
        assert!(!driver.needs_frame());
        assert!(!redraw.take(), "nothing runs, nothing to repaint");
    }

    #[test]
    fn removing_an_animation_stops_it() {
        let mut driver = AnimationDriver::new(RedrawFlag::new());
        let handle = driver.handle();
        let seen = Rc::new(Cell::new(0.0f32));
        let sink = seen.clone();
        let id = handle.add_animation(
            Animation::new(0.0f32, 1.0, Duration::from_millis(100)).on_update(move |v| sink.set(v)),
        );
        assert!(handle.contains(id));
        assert!(handle.remove(id));
        assert!(
            !handle.remove(id),
            "removing twice reports nothing left to do"
        );
        assert!(handle.is_empty());
        driver.advance(clock(1, 50));
        assert!(!driver.needs_frame());
        assert_eq!(seen.get(), 0.0, "a removed animation is not updated");
    }

    #[test]
    fn handles_share_one_driver() {
        let driver = AnimationDriver::new(RedrawFlag::new());
        let a = driver.handle();
        let b = a.clone();
        a.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(100)));
        assert_eq!(b.len(), 1);
        b.remove(AnimationId(1));
        assert!(a.is_empty());
        assert_eq!(driver.len(), 0);
    }

    #[test]
    fn ids_are_ascending_and_unique() {
        let driver = AnimationDriver::new(RedrawFlag::new());
        let handle = driver.handle();
        let first = handle.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(10)));
        let second = handle.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(10)));
        assert!(second > first);
        assert_ne!(first, second);
        assert_eq!(first.to_string(), "#1");
        assert_eq!(first.get(), 1);
    }

    #[test]
    fn a_restarted_animation_runs_again() {
        let mut driver = AnimationDriver::new(RedrawFlag::new());
        let handle = driver.handle();
        let id = handle.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(100)));
        driver.advance(clock(1, 50));
        assert!(handle.restart(id));
        assert!(driver.needs_frame());
        driver.advance(clock(2, 100));
        assert!(!driver.needs_frame());
        assert!(!handle.restart(AnimationId(99)));
    }

    #[test]
    fn cancelling_an_animation_ends_the_frame_loop() {
        let mut driver = AnimationDriver::new(RedrawFlag::new());
        let handle = driver.handle();
        let mut animation = Animation::new(0.0f32, 1.0, Duration::from_millis(100));
        animation.start();
        animation.cancel();
        handle.add_animation(animation);
        assert!(!driver.needs_frame(), "a cancelled animation is not active");
        driver.advance(clock(1, 16));
        assert!(
            driver.is_empty(),
            "terminal entries are dropped on the next tick"
        );
    }

    #[test]
    fn a_never_started_animation_is_started_on_registration() {
        let driver = AnimationDriver::new(RedrawFlag::new());
        let handle = driver.handle();
        handle.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(100)));
        assert_eq!(driver.states(), vec![AnimationState::Running]);
    }

    #[test]
    fn a_paused_animation_does_not_keep_the_loop_busy() {
        let mut driver = AnimationDriver::new(RedrawFlag::new());
        let handle = driver.handle();
        let mut animation = Animation::new(0.0f32, 1.0, Duration::from_millis(100));
        animation.start();
        animation.pause();
        let id = handle.add_animation(animation);
        assert!(!driver.needs_frame());
        assert_eq!(driver.len(), 1, "a paused animation stays registered");
        driver.advance(clock(1, 16));
        assert_eq!(driver.states(), vec![AnimationState::Idle]);
        handle.restart(id);
        assert!(driver.needs_frame());
    }

    #[test]
    fn queued_commands_are_consumed_on_the_next_tick() {
        let mut driver = AnimationDriver::new(RedrawFlag::new());
        let handle = driver.handle();
        let id = handle.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(100)));
        handle.send(DriverCommand::Add(id));
        handle.send(DriverCommand::Tick(clock(1, 50)));
        handle.send(DriverCommand::Remove(id));
        assert_eq!(handle.pending_commands().len(), 3);

        // The queued `Tick` replaces the clock passed to `advance`, so the animation sees 50 ms.
        driver.advance(clock(1, 1));
        assert!(handle.pending_commands().is_empty());
        assert!(!driver.needs_frame(), "the queued Remove dropped the entry");
        assert!(driver.is_empty());
    }

    #[test]
    fn a_queued_add_command_schedules_a_repaint() {
        let redraw = RedrawFlag::new();
        let mut driver = AnimationDriver::new(redraw.clone());
        let handle = driver.handle();
        let id = handle.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(100)));
        assert!(redraw.take(), "the registration itself already repainted");
        handle.send(DriverCommand::Add(id));
        driver.advance(clock(1, 16));
        assert!(
            redraw.take(),
            "announcing an animation schedules its first frame"
        );

        // An `Add` for an id that is not registered schedules nothing.
        handle.send(DriverCommand::Add(AnimationId(999)));
        driver.advance(clock(2, 16));
        assert!(redraw.take(), "the animation itself is still running");
        assert!(!redraw.take());
    }

    #[test]
    fn a_queued_remove_command_can_be_applied_again() {
        let mut driver = AnimationDriver::new(RedrawFlag::new());
        let handle = driver.handle();
        let doomed = handle.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(100)));
        handle.send(DriverCommand::Remove(doomed));
        handle.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(100)));
        assert_eq!(handle.len(), 2);
        driver.advance(clock(1, 16));
        assert_eq!(handle.len(), 1);
    }

    #[test]
    fn tick_produces_monotonic_frames() {
        let mut driver = AnimationDriver::new(RedrawFlag::new());
        let first = driver.tick();
        let second = driver.tick();
        assert_eq!(first.frame, 1);
        assert_eq!(second.frame, 2);
        assert!(second.elapsed >= first.elapsed);
        assert_eq!(driver.clock().frame(), 2);
    }

    #[test]
    fn clear_drops_everything() {
        let mut driver = AnimationDriver::new(RedrawFlag::new());
        let handle = driver.handle();
        handle.add_animation(Animation::new(0.0f32, 1.0, Duration::from_millis(100)));
        handle.send(DriverCommand::Add(AnimationId(1)));
        driver.clear();
        assert!(driver.is_empty());
        assert!(handle.pending_commands().is_empty());
        assert_eq!(driver.clock().frame(), 0);
    }
}
