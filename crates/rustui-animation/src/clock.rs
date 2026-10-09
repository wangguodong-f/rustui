//! Frame timing for animations (§33).
//!
//! The animation system never reads [`std::time::Instant`] itself: the application owns the frame
//! loop, produces a [`FrameClock`] per frame and hands it to [`AnimationDriver`](crate::AnimationDriver).
//! [`AnimationClock`] is the small helper that turns "now" into that clock, and
//! [`AnimationClock::push`] lets tests and fixed-step simulations drive animations deterministically
//! without sleeping.

use std::time::{Duration, Instant};

pub use rustui_core::FrameClock;

/// Nominal frame duration at 60 Hz; used for the very first tick and after a long stall.
pub const NOMINAL_FRAME: Duration = Duration::from_millis(16);

/// Upper bound for a single frame delta.
///
/// A frame that took longer than this (the window was dragged, the machine was suspended, the
/// process was paused) is reported as [`NOMINAL_FRAME`] instead: animations must not jump to their
/// end after a stall, they should simply run a bit behind.
pub const MAX_FRAME_DELTA: Duration = Duration::from_millis(100);

/// Produces [`FrameClock`] values from [`std::time::Instant`].
///
/// The clock keeps a monotonic frame counter and the total elapsed time, and clamps pathological
/// deltas so a single hitch cannot make every running animation jump.
#[derive(Debug, Clone)]
pub struct AnimationClock {
    last: Instant,
    frame: u64,
    elapsed: Duration,
    first: bool,
}

impl AnimationClock {
    /// Creates a clock whose first [`tick`](AnimationClock::tick) reports [`NOMINAL_FRAME`].
    pub fn new() -> Self {
        Self {
            last: Instant::now(),
            frame: 0,
            elapsed: Duration::ZERO,
            first: true,
        }
    }

    /// Advances the clock by the real time that passed and returns the frame.
    ///
    /// The first call after construction or [`reset`](AnimationClock::reset) reports
    /// [`NOMINAL_FRAME`]; later calls report the measured delta, clamped to [`MAX_FRAME_DELTA`].
    pub fn tick(&mut self) -> FrameClock {
        let now = Instant::now();
        let measured = now.saturating_duration_since(self.last);
        self.last = now;
        let delta = if self.first {
            self.first = false;
            NOMINAL_FRAME
        } else {
            measured.min(MAX_FRAME_DELTA)
        };
        self.advance(delta)
    }

    /// Advances the clock by an explicit `delta`.
    ///
    /// Used by tests and by fixed-step simulations that must not depend on wall clock time. The
    /// delta is clamped to [`MAX_FRAME_DELTA`] like [`tick`](AnimationClock::tick) does.
    pub fn push(&mut self, delta: Duration) -> FrameClock {
        self.last = Instant::now();
        self.advance(delta.min(MAX_FRAME_DELTA))
    }

    /// The frame index of the most recent frame (`0` before the first tick).
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Total time handed out by this clock, i.e. the sum of all (clamped) frame deltas.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Restarts the clock: frame counter and elapsed time go back to zero and the next
    /// [`tick`](AnimationClock::tick) reports [`NOMINAL_FRAME`] again.
    pub fn reset(&mut self) {
        self.last = Instant::now();
        self.frame = 0;
        self.elapsed = Duration::ZERO;
        self.first = true;
    }

    fn advance(&mut self, delta: Duration) -> FrameClock {
        self.frame += 1;
        self.elapsed += delta;
        FrameClock {
            frame: self.frame,
            delta,
            elapsed: self.elapsed,
        }
    }
}

impl Default for AnimationClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_tick_is_clamped_to_the_nominal_frame() {
        let mut clock = AnimationClock::new();
        let frame = clock.tick();
        assert_eq!(frame.frame, 1);
        assert_eq!(frame.delta, NOMINAL_FRAME);
        assert_eq!(frame.elapsed, NOMINAL_FRAME);
        assert_eq!(clock.frame(), 1);
    }

    #[test]
    fn ticks_increase_the_frame_counter_and_elapsed_time() {
        let mut clock = AnimationClock::new();
        clock.tick();
        let second = clock.tick();
        assert_eq!(second.frame, 2);
        assert!(second.delta <= MAX_FRAME_DELTA);
        assert!(second.elapsed >= second.delta);
        assert_eq!(clock.elapsed(), second.elapsed);
    }

    #[test]
    fn push_is_deterministic() {
        let mut clock = AnimationClock::new();
        let a = clock.push(Duration::from_millis(50));
        let b = clock.push(Duration::from_millis(50));
        assert_eq!(a.delta, Duration::from_millis(50));
        assert_eq!(a.frame, 1);
        assert_eq!(b.frame, 2);
        assert_eq!(b.elapsed, Duration::from_millis(100));
        assert!((b.delta_seconds() - 0.05).abs() < 1e-6);
        assert!((b.elapsed_seconds() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn push_clamps_a_long_stall() {
        let mut clock = AnimationClock::new();
        let frame = clock.push(Duration::from_secs(30));
        assert_eq!(frame.delta, MAX_FRAME_DELTA);
    }

    #[test]
    fn reset_restarts_the_counters() {
        let mut clock = AnimationClock::new();
        clock.push(Duration::from_millis(500));
        clock.reset();
        assert_eq!(clock.frame(), 0);
        assert_eq!(clock.elapsed(), Duration::ZERO);
        let frame = clock.tick();
        assert_eq!(frame.frame, 1);
        assert_eq!(frame.delta, NOMINAL_FRAME);
    }
}
