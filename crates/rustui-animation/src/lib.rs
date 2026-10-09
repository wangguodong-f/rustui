//! # rustui-animation
//!
//! Animation system of the RustUI framework (design document §33, §34 and §36): easings,
//! time-driven value animations, property transitions and the frame driver that ties them to the
//! UI thread.
//!
//! ## The pieces
//!
//! * [`Easing`] — the timing curve of an animation; [`Easing::apply`] evaluates it and
//!   [`Easing::sample_baked`] bakes it into a table for cheap per-frame sampling.
//! * [`Animatable`] — the small set of scalar types that can be interpolated (`f32`, `f64`, `i32`,
//!   `i64`, `u32`, `u64`) plus the [`lerp`] helper.
//! * [`Animation`] — a single-value animation with delay, easing, loops and an update callback.
//! * [`Transition`] — an [`Animation`] that writes into a
//!   [`SharedProperty`](rustui_core::SharedProperty), which is what widgets animate.
//! * [`AnimationClock`] / [`FrameClock`] — frame timing.
//! * [`AnimationDriver`] — owns the animations of an application and advances them once per frame;
//!   [`AnimationHandle`] lets widgets register animations without `&mut self`.
//!
//! ## Design
//!
//! Animations are **pure**: they never sleep, never spawn a thread and never read the wall clock
//! themselves — the UI thread feeds them frame deltas, which makes them deterministic and testable.
//! Every value change calls [`rustui_core::notify::request_redraw`], so an animation keeps the UI
//! repainting without any additional scheduling logic.
//!
//! ```no_run
//! use std::time::Duration;
//!
//! use rustui_animation::{Animation, AnimationDriver, Easing};
//! use rustui_core::notify::RedrawFlag;
//!
//! // The application owns the driver and hands handles to widgets.
//! let mut driver = AnimationDriver::new(RedrawFlag::new());
//! let handle = driver.handle();
//!
//! // A widget starts a fade-in whenever it appears.
//! handle.add_animation(
//!     Animation::new(0.0f32, 1.0, Duration::from_millis(250))
//!         .with_easing(Easing::OutCubic)
//!         .on_update(|opacity| println!("opacity = {opacity}")),
//! );
//!
//! // The frame loop advances it while something is running.
//! while driver.needs_frame() {
//!     driver.tick();
//! }
//! ```

pub mod animation;
pub mod clock;
pub mod driver;
pub mod easing;
pub mod value;

pub use animation::{Animation, AnimationState, Transition};
pub use clock::{AnimationClock, FrameClock, MAX_FRAME_DELTA, NOMINAL_FRAME};
pub use driver::{AnimationDriver, AnimationHandle, AnimationId, DriverCommand};
pub use easing::{BakedEasing, Easing};
pub use value::{inv_lerp, lerp, Animatable};

/// Everything an application or widget typically needs to animate something.
pub mod prelude {
    pub use crate::animation::{Animation, AnimationState, Transition};
    pub use crate::clock::{AnimationClock, FrameClock};
    pub use crate::driver::{AnimationDriver, AnimationHandle, AnimationId};
    pub use crate::easing::{BakedEasing, Easing};
    pub use crate::value::{lerp, Animatable};
    pub use rustui_core::SharedProperty;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::prelude::*;
    use rustui_core::notify::{clear_redraw_sink, set_redraw_sink, RedrawFlag};

    const MS_50: Duration = Duration::from_millis(50);
    const MS_250: Duration = Duration::from_millis(250);

    #[test]
    fn a_value_change_requests_a_redraw() {
        // Install the global redraw sink the way `Application` does and drive one animation
        // through it: every produced value must ask the UI thread to repaint.
        clear_redraw_sink();
        let redraw = RedrawFlag::new();
        set_redraw_sink(redraw.clone());
        assert!(!redraw.take());

        let mut animation = Animation::new(0.0f32, 1.0, MS_250);
        animation.start();
        assert!(redraw.take(), "start emits the initial value");
        animation.update(MS_50);
        assert!(redraw.take(), "each frame requests a repaint");
        animation.update(MS_50);
        assert!(redraw.take());
        clear_redraw_sink();
    }

    #[test]
    fn a_value_change_requests_a_redraw_through_a_transition() {
        clear_redraw_sink();
        let redraw = RedrawFlag::new();
        set_redraw_sink(redraw.clone());
        assert!(!redraw.take());

        let property = SharedProperty::new(0.0f32);
        let mut transition = Transition::new(property.clone(), 1.0, MS_250);
        assert!(redraw.take(), "the transition writes its start value");
        transition.update(MS_50);
        assert!(redraw.take());
        clear_redraw_sink();
    }

    #[test]
    fn a_finished_animation_does_not_request_more_frames() {
        clear_redraw_sink();
        let redraw = RedrawFlag::new();
        set_redraw_sink(redraw.clone());

        let mut animation = Animation::new(0.0f32, 1.0, MS_250);
        animation.start();
        let mut steps = 0;
        while animation.update(MS_50) {
            steps += 1;
        }
        assert_eq!(
            steps, 4,
            "250 ms in 50 ms steps is five updates, four of which keep running"
        );
        assert!(redraw.take(), "the last update produced the target value");
        assert!(
            !animation.update(MS_50),
            "a finished animation does nothing"
        );
        assert!(!redraw.take(), "and does not request a repaint");
        clear_redraw_sink();
    }

    #[test]
    fn the_prelude_exposes_the_public_api() {
        let driver = AnimationDriver::new(RedrawFlag::new());
        let handle: AnimationHandle = driver.handle();
        let _clock: FrameClock = FrameClock::default();
        let _clock = AnimationClock::new();
        let _id: AnimationId = handle.add_animation(Animation::new(0u32, 1, MS_250));
        let _state: AnimationState = AnimationState::Idle;
        let _baked: BakedEasing = Easing::OutCubic.sample_baked(16);
        let _value: f32 = lerp(0.0, 1.0, 0.5);
        let _property: SharedProperty<f32> = SharedProperty::new(0.0);
    }

    #[test]
    fn an_end_to_end_transition_driven_by_the_frame_clock() {
        let property = SharedProperty::new(0.0f32);
        let mut driver = AnimationDriver::new(RedrawFlag::new());
        driver
            .handle()
            .add_transition(Transition::new(property.clone(), 250.0, MS_250));

        let mut clock = AnimationClock::new();
        let mut frames = 0;
        while driver.needs_frame() {
            let frame = clock.push(MS_50);
            driver.advance(frame);
            frames += 1;
        }
        assert_eq!(frames, 5, "250 ms at 50 ms per frame");
        assert_eq!(property.get(), 250.0);
        assert_eq!(clock.frame(), 5);
        assert!(!driver.needs_frame());
    }
}
