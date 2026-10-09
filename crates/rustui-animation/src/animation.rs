//! Animations and property transitions (§33).
//!
//! [`Animation`] is a time based, single-value interpolation: it owns a start value, a target value,
//! a duration, an optional delay, an easing curve and a loop count, and it is advanced by explicit
//! frame deltas ([`Animation::update`]). It is *not* self driving — nothing sleeps and nothing spawns a
//! thread, the UI thread's frame loop feeds it deltas.
//!
//! Every value the animation produces is pushed to the optional `on_update` callback **and**
//! `rustui_core::notify::request_redraw()` is called, so a value that changes without any further
//! event still repaints the UI.
//!
//! [`Transition`] is the thin layer on top of it: it drives an [`Animation`] and writes each
//! intermediate value into a [`SharedProperty`], which is how a widget animates one of its own
//! properties.

use std::fmt;
use std::time::Duration;

use rustui_core::notify::request_redraw;
use rustui_core::SharedProperty;

use crate::easing::Easing;
use crate::value::{lerp, Animatable};

/// Life cycle of an [`Animation`].
///
/// Note that "paused" is not a separate state: an [`AnimationState::Idle`] animation that has been
/// started before is paused, and an [`AnimationState::Idle`] animation that has never been started
/// is waiting for its first [`Animation::start`]. Use [`Animation::is_started`] to tell the two
/// apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnimationState {
    /// Not advancing: either not started yet, or paused.
    Idle,
    /// Advancing on every [`Animation::update`].
    Running,
    /// Reached the target value (all loops completed).
    Finished,
    /// Stopped by [`Animation::cancel`]; it will not advance again unless restarted.
    Cancelled,
}

impl AnimationState {
    /// `true` for [`AnimationState::Running`].
    pub fn is_running(self) -> bool {
        matches!(self, AnimationState::Running)
    }

    /// `true` for [`AnimationState::Finished`] and [`AnimationState::Cancelled`], the two states an
    /// animation cannot leave without an explicit [`Animation::restart`].
    pub fn is_terminal(self) -> bool {
        matches!(self, AnimationState::Finished | AnimationState::Cancelled)
    }
}

/// A single-value, time driven animation.
///
/// ```text
///  delay            |<-- elapsed -->| duration   (loops = 1)
///  |------- wait ----|=====================|  progress = ease(elapsed / duration)
/// ```
///
/// With `loops > 1` the easing is applied *per pass* and the total duration is
/// `duration * loops`; `loops == 0` repeats forever (used for spinners and idle "breathing"
/// effects, for which the animation simply holds the target value once it is reached).
pub struct Animation<T: Animatable> {
    from: T,
    to: T,
    duration: Duration,
    elapsed: Duration,
    delay: Duration,
    delay_elapsed: Duration,
    easing: Easing,
    state: AnimationState,
    loops: u32,
    completed_loops: u32,
    override_value: Option<T>,
    started: bool,
    on_update: Option<Box<dyn Fn(T)>>,
    on_finish: Option<Box<dyn Fn()>>,
}

impl<T: Animatable> Animation<T> {
    /// Creates a stopped animation from `from` to `to` over `duration`.
    ///
    /// Call [`start`](Animation::start) (or hand it to the
    /// [`AnimationDriver`](crate::AnimationDriver), which starts it for you) to let it advance.
    pub fn new(from: T, to: T, duration: Duration) -> Self {
        Self {
            from,
            to,
            duration,
            elapsed: Duration::ZERO,
            delay: Duration::ZERO,
            delay_elapsed: Duration::ZERO,
            easing: Easing::Linear,
            state: AnimationState::Idle,
            loops: 1,
            completed_loops: 0,
            override_value: None,
            started: false,
            on_update: None,
            on_finish: None,
        }
    }

    /// Sets the easing curve (consuming builder style).
    pub fn with_easing(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    /// Sets the initial delay (consuming builder style).
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Sets the number of passes; `0` means "repeat forever" (consuming builder style).
    pub fn with_loops(mut self, loops: u32) -> Self {
        self.loops = loops;
        self
    }

    /// Registers a callback that receives every value the animation produces (consuming builder
    /// style).
    pub fn on_update(mut self, f: impl Fn(T) + 'static) -> Self {
        self.on_update = Some(Box::new(f));
        self
    }

    /// Registers a callback that runs once when the animation reaches [`AnimationState::Finished`]
    /// (consuming builder style).
    pub fn on_finish(mut self, f: impl Fn() + 'static) -> Self {
        self.on_finish = Some(Box::new(f));
        self
    }

    /// The start value.
    pub fn from(&self) -> T {
        self.from
    }

    /// Replaces the start value; used when retargeting an animation from the value the user
    /// currently sees (a drag that hands over to an animation, for example).
    pub fn set_from(&mut self, from: T) {
        self.from = from;
    }

    /// The target value.
    pub fn to(&self) -> T {
        self.to
    }

    /// The configured target (replaces the current target while the animation runs; the animation
    /// then animates towards the new target from its current value).
    pub fn set_to(&mut self, to: T) {
        self.to = to;
    }

    /// The duration of a single pass.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Sets the duration of a single pass.
    pub fn set_duration(&mut self, duration: Duration) {
        self.duration = duration;
    }

    /// The initial delay.
    pub fn delay(&self) -> Duration {
        self.delay
    }

    /// Sets the initial delay; the part of the delay that was already served is kept.
    pub fn set_delay(&mut self, delay: Duration) {
        self.delay = delay;
        if self.delay_elapsed > delay {
            self.delay_elapsed = delay;
        }
    }

    /// The easing curve.
    pub fn easing(&self) -> Easing {
        self.easing
    }

    /// Replaces the easing curve; the current elapsed time is preserved, so the value may jump.
    pub fn set_easing(&mut self, easing: Easing) {
        self.easing = easing;
    }

    /// The configured pass count; `0` means "repeat forever".
    pub fn loops(&self) -> u32 {
        self.loops
    }

    /// Sets the pass count; `0` means "repeat forever". At least one pass is performed when
    /// restarting a finite animation.
    pub fn set_loops(&mut self, loops: u32) {
        self.loops = loops;
    }

    /// Number of passes completed so far.
    pub fn completed_loops(&self) -> u32 {
        self.completed_loops
    }

    /// Time spent inside the delay.
    pub fn delay_elapsed(&self) -> Duration {
        self.delay_elapsed
    }

    /// Time spent animating (delay excluded).
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// The total duration of a finite animation, or `None` when it repeats forever.
    pub fn total_duration(&self) -> Option<Duration> {
        if self.loops == 0 {
            None
        } else {
            Some(self.duration.saturating_mul(self.loops.max(1)))
        }
    }

    /// The current state.
    pub fn state(&self) -> AnimationState {
        self.state
    }

    /// `true` while the animation advances on [`update`](Animation::update).
    pub fn is_running(&self) -> bool {
        self.state.is_running()
    }

    /// `true` after [`start`](Animation::start) (or a restart), even while paused.
    pub fn is_started(&self) -> bool {
        self.started
    }

    /// `true` once the target value has been reached.
    pub fn is_finished(&self) -> bool {
        matches!(self.state, AnimationState::Finished)
    }

    /// `true` after [`cancel`](Animation::cancel).
    pub fn is_cancelled(&self) -> bool {
        matches!(self.state, AnimationState::Cancelled)
    }

    /// The current value: the override installed by [`set_value`](Animation::set_value) if there is
    /// one, otherwise the interpolation of [`from`](Animation::from) and [`to`](Animation::to)
    /// along the eased progress of the **current pass**.
    ///
    /// A looping animation returns to `from` when a new pass begins; a finished animation reports
    /// exactly `to`.
    pub fn value(&self) -> T {
        match self.override_value {
            Some(value) => value,
            None => lerp(self.from, self.to, self.pass_progress()),
        }
    }

    /// Overall progress in `0..=1` with the easing applied, measured across **all** passes.
    ///
    /// * A single pass animation reports `easing(elapsed / duration)`.
    /// * A looping animation reports `(completed_passes + easing(fraction_in_current_pass)) / loops`,
    ///   so it grows monotonically from `0` to `1` while [`value`](Self::value) cycles.
    /// * A never-ending animation (`loops == 0`) has no overall span, so it reports the progress of
    ///   the current pass.
    /// * A delay is not part of the animated span and therefore does not show up in the progress.
    pub fn progress(&self) -> f32 {
        let pass = self.duration.as_nanos();
        if pass == 0 {
            return if self.is_finished() { 1.0 } else { 0.0 };
        }
        if self.is_finished() {
            return 1.0;
        }
        if self.loops <= 1 {
            return self.pass_progress();
        }
        let loops = self.loops as u128;
        let elapsed = self.elapsed.as_nanos();
        let index = (elapsed / pass).min(loops - 1) as f32;
        let fraction = ((elapsed % pass) as f32 / pass as f32).clamp(0.0, 1.0);
        ((index + self.easing.apply(fraction)) / loops as f32).clamp(0.0, 1.0)
    }

    /// Eased progress of the current pass, in `0..=1` — the fraction that drives
    /// [`value`](Self::value).
    fn pass_progress(&self) -> f32 {
        let pass = self.duration.as_nanos();
        if pass == 0 {
            return if self.is_finished() { 1.0 } else { 0.0 };
        }
        if self.is_finished() {
            return 1.0;
        }
        let elapsed = self.elapsed.as_nanos();
        let fraction = if self.loops <= 1 {
            (elapsed as f32 / pass as f32).clamp(0.0, 1.0)
        } else {
            // Every pass after the first restarts at `from`.
            ((elapsed % pass) as f32 / pass as f32).clamp(0.0, 1.0)
        };
        self.easing.apply(fraction)
    }

    /// Forces the current value without touching `from`/`to` (used while the user drags a handle:
    /// the drag owns the value until the animation takes over again).
    ///
    /// The override is cleared by [`start`](Animation::start),
    /// [`restart`](Animation::restart) and by reaching [`AnimationState::Finished`] — a finished
    /// animation always reports its exact target value.
    pub fn set_value(&mut self, value: T) {
        self.override_value = Some(value);
        self.emit(value);
    }

    /// Removes a value installed by [`set_value`](Animation::set_value).
    pub fn clear_value(&mut self) {
        self.override_value = None;
    }

    /// Starts (or restarts) the animation and emits the current value once.
    ///
    /// Any pending delay, elapsed time, completed passes and value override are cleared. A zero
    /// duration finishes immediately.
    pub fn start(&mut self) {
        self.elapsed = Duration::ZERO;
        self.delay_elapsed = Duration::ZERO;
        self.completed_loops = 0;
        self.override_value = None;
        self.started = true;
        self.state = AnimationState::Running;
        let value = self.value();
        self.emit(value);
        if self.advance_passes() {
            self.finish();
        }
    }

    /// Pauses a running animation; its elapsed time is kept.
    pub fn pause(&mut self) {
        if self.state.is_running() {
            self.state = AnimationState::Idle;
        }
    }

    /// Resumes a paused animation (a never-started animation is simply started).
    pub fn resume(&mut self) {
        if self.state == AnimationState::Idle {
            self.started = true;
            self.state = AnimationState::Running;
        }
    }

    /// Stops the animation permanently (until [`restart`](Animation::restart)).
    pub fn cancel(&mut self) {
        self.state = AnimationState::Cancelled;
    }

    /// Starts over from `from`, clearing delay, elapsed time and value override.
    pub fn restart(&mut self) {
        self.start();
    }

    /// Drops the `on_update` / `on_finish` callbacks, breaking reference cycles a callback may have
    /// created (a callback that captured the animation itself, for example).
    pub fn clear_callbacks(&mut self) {
        self.on_update = None;
        self.on_finish = None;
    }

    /// Advances the animation by `delta` and returns `true` while it is still animating.
    ///
    /// `delta` is first spent on the pending delay; the remainder moves the value. Every step emits
    /// the interpolated value to `on_update` and calls [`request_redraw`]. Calling this on an
    /// animation that is not running is a no-op and returns `false`.
    ///
    /// The `on_update` callback runs while this `&mut self` borrow is live, so a callback that needs
    /// to touch the animation must not capture it; queue the follow-up work instead (for example
    /// through [`AnimationHandle::send`](crate::AnimationHandle::send)).
    pub fn update(&mut self, delta: Duration) -> bool {
        if !self.state.is_running() {
            return false;
        }
        let mut remaining = delta;
        if self.delay_elapsed < self.delay {
            let needed = self.delay - self.delay_elapsed;
            if remaining < needed {
                self.delay_elapsed += remaining;
                remaining = Duration::ZERO;
            } else {
                remaining -= needed;
                self.delay_elapsed = self.delay;
            }
        }
        if !remaining.is_zero() {
            self.elapsed += remaining;
        }
        let finished = self.advance_passes();
        if finished {
            // `finish` emits the final (exact) target value, so there is no double emission here.
            self.finish();
        } else {
            let value = self.value();
            self.emit(value);
        }
        self.state.is_running()
    }

    /// Counts completed passes and reports whether the whole animation is done.
    fn advance_passes(&mut self) -> bool {
        let pass_nanos = self.duration.as_nanos();
        if pass_nanos == 0 {
            // A zero duration animation has no time to distribute over passes; a single update
            // completes all of them.
            return true;
        }
        let loops = self.loops.max(1);
        if self.loops == 0 {
            // Repeating forever: keep wrapping, the value simply stays at the target.
            self.completed_loops =
                (self.elapsed.as_nanos() / pass_nanos).min(u32::MAX as u128) as u32;
            return false;
        }
        let total_nanos = pass_nanos.saturating_mul(loops as u128);
        if self.elapsed.as_nanos() >= total_nanos {
            self.elapsed = Duration::from_nanos(total_nanos.min(u64::MAX as u128) as u64);
            self.completed_loops = loops;
            return true;
        }
        self.completed_loops = (self.elapsed.as_nanos() / pass_nanos).min(u32::MAX as u128) as u32;
        false
    }

    fn finish(&mut self) {
        self.state = AnimationState::Finished;
        self.override_value = None;
        let value = self.value();
        self.emit(value);
        if let Some(callback) = self.on_finish.take() {
            callback();
        }
    }

    fn emit(&mut self, value: T) {
        if let Some(callback) = self.on_update.as_ref() {
            callback(value);
        }
        request_redraw();
    }
}

impl<T: Animatable + fmt::Debug> fmt::Debug for Animation<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Animation")
            .field("from", &self.from)
            .field("to", &self.to)
            .field("duration", &self.duration)
            .field("delay", &self.delay)
            .field("elapsed", &self.elapsed)
            .field("easing", &self.easing.name())
            .field("loops", &self.loops)
            .field("completed_loops", &self.completed_loops)
            .field("state", &self.state)
            .field("progress", &self.progress())
            .field("value", &self.value())
            .finish()
    }
}

/// An [`Animation`] that writes its values into a [`SharedProperty`].
///
/// This is the form widgets use: the widget renders `property.get()` and the transition animates the
/// property, so every repaint sees the current value and every observer of
/// `property.changed()` is notified frame by frame.
pub struct Transition<T: Animatable> {
    property: SharedProperty<T>,
    animation: Animation<T>,
}

impl<T: Animatable> Transition<T> {
    /// Creates a transition of `property` to `target` over `duration` and starts it immediately.
    ///
    /// The animation starts at the property's current value, so a retarget of a running transition
    /// begins where the eye currently sees the value.
    pub fn new(property: SharedProperty<T>, target: T, duration: Duration) -> Self {
        let from = property.get();
        let writer = property.clone();
        let animation =
            Animation::new(from, target, duration).on_update(move |value| writer.set(value));
        let mut transition = Self {
            property,
            animation,
        };
        transition.animation.start();
        transition
    }

    /// The animated property.
    pub fn property(&self) -> &SharedProperty<T> {
        &self.property
    }

    /// The underlying animation (read only).
    pub fn animation(&self) -> &Animation<T> {
        &self.animation
    }

    /// The underlying animation, for retargeting or cancelling.
    pub fn animation_mut(&mut self) -> &mut Animation<T> {
        &mut self.animation
    }

    /// Current state of the underlying animation.
    pub fn state(&self) -> AnimationState {
        self.animation.state()
    }

    /// The value the transition animates towards.
    pub fn target(&self) -> T {
        self.animation.to()
    }

    /// Retargets the transition, continuing from the property's current value.
    ///
    /// The property keeps its current value; only the destination and the duration change, so a
    /// transition can be re-aimed mid flight without a visible jump.
    pub fn retarget(&mut self, target: T, duration: Duration) {
        let from = self.property.get();
        self.animation.set_duration(duration);
        self.animation.set_to(target);
        self.animation.set_from(from);
        self.animation.start();
    }

    /// Sets the easing curve.
    pub fn with_easing(&mut self, easing: Easing) -> &mut Self {
        self.animation.set_easing(easing);
        self
    }

    /// Sets the initial delay.
    pub fn with_delay(&mut self, delay: Duration) -> &mut Self {
        self.animation.set_delay(delay);
        self
    }

    /// Sets the pass count (`0` repeats forever).
    pub fn with_loops(&mut self, loops: u32) -> &mut Self {
        self.animation.set_loops(loops);
        self
    }

    /// `true` while the transition is still animating.
    pub fn is_running(&self) -> bool {
        self.animation.is_running()
    }

    /// `true` once the property has reached the target.
    pub fn finished(&self) -> bool {
        self.animation.is_finished()
    }

    /// Advances the transition by `delta`; returns `true` while it is still animating.
    pub fn update(&mut self, delta: Duration) -> bool {
        self.animation.update(delta)
    }

    /// Restarts the transition, animating from the property's current value to the same target.
    pub fn restart(&mut self) {
        let from = self.property.get();
        self.animation.set_from(from);
        self.animation.restart();
    }

    /// Stops the transition; the property keeps its last value.
    pub fn cancel(&mut self) {
        self.animation.cancel();
    }
}

impl<T: Animatable + fmt::Debug> fmt::Debug for Transition<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transition")
            .field("animation", &self.animation)
            .field("property", &self.property.get())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS_25: Duration = Duration::from_millis(25);
    const MS_50: Duration = Duration::from_millis(50);
    const MS_100: Duration = Duration::from_millis(100);
    const MS_250: Duration = Duration::from_millis(250);

    #[test]
    fn linear_animation_finishes_after_exact_steps() {
        let mut anim = Animation::new(0.0f32, 100.0, MS_250);
        anim.start();
        assert!(anim.is_running());
        for _ in 0..4 {
            assert!(anim.update(MS_50), "still running after 50ms steps");
            assert!(anim.is_running());
        }
        assert!(!anim.update(MS_50), "must stop after the fifth step");
        assert_eq!(anim.state(), AnimationState::Finished);
        assert_eq!(anim.value(), 100.0);
        assert_eq!(anim.progress(), 1.0);
        assert_eq!(anim.elapsed(), MS_250);
    }

    #[test]
    fn progress_follows_elapsed_time() {
        let mut anim = Animation::new(0.0f32, 1.0, MS_100);
        anim.start();
        anim.update(MS_50);
        assert!((anim.progress() - 0.5).abs() < 1e-6);
        assert!((anim.value() - 0.5).abs() < 1e-6);
        anim.update(MS_25);
        assert!((anim.progress() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn easing_is_applied_to_the_progress() {
        let mut anim = Animation::new(0.0f32, 1.0, MS_100).with_easing(Easing::InQuad);
        anim.start();
        anim.update(MS_50);
        assert!((anim.progress() - 0.25).abs() < 1e-6);
        anim.update(MS_50);
        assert!((anim.progress() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn delay_defers_the_animation() {
        let mut anim = Animation::new(0.0f32, 1.0, MS_100).with_delay(MS_100);
        anim.start();
        assert!(anim.update(MS_50));
        assert_eq!(anim.value(), 0.0);
        assert_eq!(anim.progress(), 0.0);
        assert_eq!(anim.delay_elapsed(), MS_50);
        assert!(anim.update(MS_50));
        assert_eq!(anim.delay_elapsed(), MS_100);
        assert!(anim.update(MS_50));
        assert!((anim.progress() - 0.5).abs() < 1e-6);
        assert!(!anim.update(MS_50), "100 ms of animation are done");
        assert_eq!(anim.state(), AnimationState::Finished);
    }

    #[test]
    fn a_single_update_can_cross_the_whole_delay() {
        let mut anim = Animation::new(0.0f32, 1.0, MS_100).with_delay(MS_50);
        anim.start();
        assert!(anim.update(Duration::from_millis(100)));
        assert!((anim.value() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn pause_and_resume_keep_the_elapsed_time() {
        let mut anim = Animation::new(0.0f32, 1.0, MS_100);
        anim.start();
        anim.update(MS_50);
        anim.pause();
        assert_eq!(anim.state(), AnimationState::Idle);
        assert!(!anim.is_running());
        assert!(!anim.update(MS_50), "a paused animation must not advance");
        assert!((anim.value() - 0.5).abs() < 1e-6);
        anim.resume();
        assert!(anim.is_running());
        anim.update(MS_50);
        assert_eq!(anim.state(), AnimationState::Finished);
    }

    #[test]
    fn cancel_stops_the_animation_for_good() {
        let mut anim = Animation::new(0.0f32, 1.0, MS_100);
        anim.start();
        anim.update(MS_50);
        anim.cancel();
        assert_eq!(anim.state(), AnimationState::Cancelled);
        assert!(anim.state().is_terminal());
        assert!(!anim.update(MS_100));
        assert!((anim.value() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn restart_resets_everything() {
        let mut anim = Animation::new(0.0f32, 1.0, MS_100);
        anim.start();
        anim.update(MS_50);
        anim.restart();
        assert_eq!(anim.elapsed(), Duration::ZERO);
        assert_eq!(anim.value(), 0.0);
        assert_eq!(anim.progress(), 0.0);
        anim.update(MS_100);
        assert_eq!(anim.value(), 1.0);
    }

    #[test]
    fn loops_repeat_and_finish_after_the_last_pass() {
        let mut anim = Animation::new(0.0f32, 1.0, MS_100).with_loops(3);
        assert_eq!(anim.total_duration(), Some(Duration::from_millis(300)));
        anim.start();
        assert!(anim.update(MS_50));
        assert_eq!(anim.completed_loops(), 0);
        assert!((anim.progress() - 1.0 / 6.0).abs() < 1e-5);
        anim.update(MS_50);
        assert_eq!(anim.completed_loops(), 1);
        assert!((anim.progress() - 1.0 / 3.0).abs() < 1e-5);
        assert!(anim.update(MS_100));
        assert_eq!(anim.completed_loops(), 2);
        assert!((anim.progress() - 2.0 / 3.0).abs() < 1e-5);
        assert!(!anim.update(MS_100), "300 ms complete all three passes");
        assert_eq!(anim.completed_loops(), 3);
        assert_eq!(anim.state(), AnimationState::Finished);
        assert_eq!(anim.progress(), 1.0);
        assert_eq!(anim.value(), 1.0);
    }

    #[test]
    fn looping_progress_is_reported_across_all_passes() {
        let mut anim = Animation::new(0.0f32, 1.0, MS_100).with_loops(2);
        anim.start();
        anim.update(MS_100);
        assert!((anim.progress() - 0.5).abs() < 1e-6);
        anim.update(MS_50);
        assert!((anim.progress() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn every_pass_applies_the_easing() {
        // A looping animation eases *inside* every pass, so the value returns to `from` when a new
        // pass begins, while `progress()` keeps reporting the overall position across all passes.
        let mut anim = Animation::new(0.0f32, 1.0, MS_100)
            .with_loops(2)
            .with_easing(Easing::InQuad);
        anim.start();
        anim.update(MS_100);
        assert_eq!(anim.completed_loops(), 1);
        assert_eq!(anim.value(), 0.0, "a new pass restarts at `from`");
        assert!((anim.progress() - 0.5).abs() < 1e-6);
        anim.update(MS_50);
        assert_eq!(
            anim.value(),
            0.25,
            "InQuad of the quarter of the second pass"
        );
        assert!((anim.progress() - 0.625).abs() < 1e-5);
    }

    #[test]
    fn zero_loops_repeats_forever() {
        let mut anim = Animation::new(0.0f32, 1.0, MS_100).with_loops(0);
        assert_eq!(anim.total_duration(), None);
        anim.start();
        for _ in 0..10 {
            assert!(anim.update(MS_100));
        }
        assert_eq!(anim.state(), AnimationState::Running);
        assert_eq!(anim.completed_loops(), 10);
        assert_eq!(anim.value(), 1.0);
    }

    #[test]
    fn zero_duration_finishes_immediately() {
        let mut anim = Animation::new(0.0f32, 1.0, Duration::ZERO);
        anim.start();
        assert_eq!(anim.state(), AnimationState::Finished);
        assert_eq!(anim.value(), 1.0);
        assert_eq!(anim.progress(), 1.0);
        assert!(!anim.update(MS_50));
    }

    #[test]
    fn on_update_receives_every_interpolated_value() {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = log.clone();
        let mut anim =
            Animation::new(0.0f32, 2.0, MS_100).on_update(move |v| sink.borrow_mut().push(v));
        anim.start();
        anim.update(MS_50);
        anim.update(MS_50);
        let seen = log.borrow().clone();
        assert_eq!(seen.len(), 3, "start + two frames, got {seen:?}");
        assert_eq!(seen[0], 0.0);
        assert_eq!(seen[1], 1.0);
        assert_eq!(seen[2], 2.0);
    }

    #[test]
    fn on_finish_runs_exactly_once() {
        let count = std::rc::Rc::new(std::cell::RefCell::new(0));
        let sink = count.clone();
        let mut anim = Animation::new(0u32, 10, MS_100).on_finish(move || *sink.borrow_mut() += 1);
        anim.start();
        anim.update(MS_100);
        anim.update(MS_100);
        assert_eq!(*count.borrow(), 1);
    }

    #[test]
    fn set_value_overrides_the_interpolated_value() {
        let mut anim = Animation::new(0.0f32, 10.0, MS_100);
        anim.start();
        anim.set_value(4.0);
        assert_eq!(anim.value(), 4.0);
        anim.update(MS_50);
        assert_eq!(
            anim.value(),
            4.0,
            "the override wins over the interpolation"
        );
        assert!(anim.is_running());
        anim.update(MS_50);
        // Finishing snaps to the exact target and drops the override, which is what makes a
        // finished animation report `to` rather than whatever the drag left behind.
        assert_eq!(anim.value(), 10.0);
        assert_eq!(anim.state(), AnimationState::Finished);
        anim.restart();
        assert_eq!(anim.value(), 0.0, "restart clears the override");
    }

    #[test]
    fn integer_animations_end_on_the_exact_target() {
        let mut anim = Animation::new(0u32, 3, MS_100);
        anim.start();
        assert!(anim.update(MS_50));
        assert_eq!(anim.value(), 2); // 1.5 rounds to nearest even
        assert!(!anim.update(MS_50));
        assert_eq!(anim.value(), 3);
        assert_eq!(anim.state(), AnimationState::Finished);
    }

    #[test]
    fn descending_animations_are_supported() {
        let mut anim = Animation::new(10i32, 0, MS_100);
        anim.start();
        anim.update(MS_50);
        assert_eq!(anim.value(), 5);
        anim.update(MS_50);
        assert_eq!(anim.value(), 0);
    }

    #[test]
    fn retargeting_changes_the_target_value() {
        let mut anim = Animation::new(0.0f32, 1.0, MS_100);
        anim.start();
        anim.update(MS_50);
        anim.set_to(3.0);
        anim.update(MS_50);
        assert_eq!(anim.value(), 3.0);
        assert_eq!(anim.to(), 3.0);
    }

    #[test]
    fn clearing_callbacks_stops_the_notifications() {
        let log = std::rc::Rc::new(std::cell::RefCell::new(0usize));
        let sink = log.clone();
        let mut anim =
            Animation::new(0.0f32, 1.0, MS_100).on_update(move |_| *sink.borrow_mut() += 1);
        anim.start();
        anim.update(MS_50);
        let before = *log.borrow();
        anim.clear_callbacks();
        anim.update(MS_50);
        assert_eq!(*log.borrow(), before);
    }

    #[test]
    fn transitions_write_into_a_shared_property() {
        let property = SharedProperty::new(0.0f32);
        let mut transition = Transition::new(property.clone(), 100.0, MS_250);
        assert!(transition.is_running());
        for _ in 0..4 {
            assert!(transition.update(MS_50), "250 ms is five 50 ms frames");
        }
        assert!(
            (property.get() - 80.0).abs() < 1e-4,
            "got {}",
            property.get()
        );
        assert!(!transition.update(MS_50), "the fifth frame completes it");
        assert_eq!(property.get(), 100.0);
        assert!(transition.finished());
        assert_eq!(transition.target(), 100.0);
    }

    #[test]
    fn transitions_emit_the_changed_signal_every_frame() {
        let property = SharedProperty::new(0.0f32);
        let seen = std::rc::Rc::new(std::cell::RefCell::new(0usize));
        let sink = seen.clone();
        property.changed().connect(move |_| *sink.borrow_mut() += 1);
        let mut transition = Transition::new(property.clone(), 1.0, MS_100);
        // Starting the transition emits its start value, so observers are notified before the
        // first frame as well.
        transition.update(MS_50);
        transition.update(MS_50);
        assert_eq!(*seen.borrow(), 3, "start value plus two frames");
        assert!(transition.finished());
    }

    #[test]
    fn transition_restart_animates_from_the_current_value() {
        let property = SharedProperty::new(0.0f32);
        let mut transition = Transition::new(property.clone(), 10.0, MS_100);
        transition.update(MS_50);
        assert!((property.get() - 5.0).abs() < 1e-4);
        transition.restart();
        assert!(transition.is_running());
        transition.update(MS_50);
        assert!(
            (property.get() - 7.5).abs() < 1e-4,
            "got {}",
            property.get()
        );
        transition.update(MS_50);
        assert_eq!(property.get(), 10.0);
    }

    #[test]
    fn transition_can_be_cancelled_and_retargeted() {
        let property = SharedProperty::new(0.0f32);
        let mut transition = Transition::new(property.clone(), 10.0, MS_100);
        transition.update(MS_50);
        transition.cancel();
        assert!(!transition.update(MS_50));
        assert!((property.get() - 5.0).abs() < 1e-4);

        transition.retarget(20.0, MS_100);
        assert!(transition.is_running());
        transition.update(MS_50);
        assert!(
            (property.get() - 12.5).abs() < 1e-4,
            "got {}",
            property.get()
        );
    }

    #[test]
    fn transition_honours_easing_and_delay() {
        let property = SharedProperty::new(0.0f32);
        let mut transition = Transition::new(property.clone(), 1.0, MS_100);
        transition.with_easing(Easing::InQuad).with_delay(MS_50);
        transition.update(MS_50);
        assert_eq!(property.get(), 0.0, "still inside the delay");
        transition.update(MS_50);
        assert!(
            (property.get() - 0.25).abs() < 1e-4,
            "got {}",
            property.get()
        );
    }
}
