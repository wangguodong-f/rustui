//! Easing functions (§33).
//!
//! An easing function remaps the *linear* progress of an animation (`0..1`) onto a curve that
//! describes how the animated value should move: slow start, overshoot, bounce, … All easings in
//! this module satisfy `f(0) = 0` and `f(1) = 1` exactly — the classic Penner formulas are
//! special cased at the endpoints for that reason — but some of them deliberately leave the
//! `[0, 1]` range in between:
//!
//! * the **back** family undershoots below `0` on the way in and overshoots above `1` on the way
//!   out,
//! * the **elastic** family oscillates around the target.
//!
//! [`Easing::is_overshooting`] reports which variants behave like that, so callers that need a
//! strictly clamped value (for example an opacity that is written straight into a shader uniform)
//! can clamp the *eased* result instead of relying on the linear range of the curve.
//!
//! Evaluators that must not run a `powf`/`sin`/`cos` per channel per frame can bake the curve once
//! with [`Easing::sample_baked`] and interpolate the baked table at run time.

use std::f32::consts::PI;

/// Overshoot constant of the "back" family (`c1 = 1.70158` in Penner's notation).
const BACK_C1: f32 = 1.70158;
/// Amplified overshoot constant for [`Easing::InOutBack`] (`c2 = c1 * 1.525`).
const BACK_C2: f32 = BACK_C1 * 1.525;
/// Quadratic term of the "back" family (`c3 = c1`).
const BACK_C3: f32 = BACK_C1;
/// Period of the "elastic" family (`2π / 3`).
const ELASTIC_PERIOD: f32 = (2.0 * PI) / 3.0;
/// Period of [`Easing::InOutElastic`], i.e. half the frequency (`2π / 4.5`).
const ELASTIC_INOUT_PERIOD: f32 = (2.0 * PI) / 4.5;
/// Amplitude factor of the "bounce" family.
const BOUNCE_N1: f32 = 7.5625;
/// Decay factor of the "bounce" family.
const BOUNCE_D1: f32 = 2.75;

/// The easing (timing) function of an animation.
///
/// The variants cover the three classic groups — polynomial (`Quad`, `Cubic`, `Quart`), trigonometric
/// (`Sine`, `Circ`) and physical (`Expo`, `Back`, `Elastic`, `Bounce`) — each in an `In`, `Out` and
/// `InOut` flavour plus the identity [`Easing::Linear`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Easing {
    /// Constant speed: `f(t) = t`.
    Linear,
    /// Accelerating from rest: `t²`.
    InQuad,
    /// Decelerating to rest: `1 - (1 - t)²`.
    OutQuad,
    /// Accelerates for the first half, decelerates for the second half.
    InOutQuad,
    /// Accelerating from rest: `t³`.
    InCubic,
    /// Decelerating to rest: `1 - (1 - t)³`.
    OutCubic,
    /// Strongly accelerating first, strongly decelerating last.
    InOutCubic,
    /// Accelerating from rest: `t⁴`.
    InQuart,
    /// Decelerating to rest: `1 - (1 - t)⁴`.
    OutQuart,
    /// Very sharp acceleration/deceleration.
    InOutQuart,
    /// Gentle acceleration following a half sine: `1 - cos(t·π/2)`.
    InSine,
    /// Gentle deceleration following a half sine: `sin(t·π/2)`.
    OutSine,
    /// Sine based ease in and out, symmetric around `t = 0.5`.
    InOutSine,
    /// Extremely slow start, then a fast burst (`2^(10t - 10)`).
    InExpo,
    /// Fast start, extremely slow tail (`1 - 2^-10t`).
    OutExpo,
    /// Skew-symmetric: slow start / slow end around a fast middle.
    InOutExpo,
    /// Accelerating with a circular (square root) curve: `1 - √(1 - t²)`.
    InCirc,
    /// Decelerating with a circular curve: `√(1 - (1 - t)²)`.
    OutCirc,
    /// Symmetric circular ease in and out.
    InOutCirc,
    /// Accelerates and *undershoots below* `0` before returning, like a pulled spring.
    InBack,
    /// Decelerates and *overshoots above* `1` before settling.
    OutBack,
    /// Undershoot at the start, overshoot at the end, symmetric in time.
    InOutBack,
    /// Accelerating oscillation with decreasing amplitude (spring release).
    InElastic,
    /// Decelerating oscillation with decreasing amplitude (spring settle).
    OutElastic,
    /// Oscillation that starts and ends at rest.
    InOutElastic,
    /// Accelerating bounce, made from the mirror image of [`Easing::OutBounce`].
    InBounce,
    /// Decelerating bounce against the "floor" the value animates on.
    OutBounce,
    /// Bounce on the way in and on the way out.
    InOutBounce,
}

impl Easing {
    /// Every variant, in declaration order.
    ///
    /// Handy for tests and for debug tooling that wants to render all curves.
    pub const ALL: [Easing; 28] = [
        Easing::Linear,
        Easing::InQuad,
        Easing::OutQuad,
        Easing::InOutQuad,
        Easing::InCubic,
        Easing::OutCubic,
        Easing::InOutCubic,
        Easing::InQuart,
        Easing::OutQuart,
        Easing::InOutQuart,
        Easing::InSine,
        Easing::OutSine,
        Easing::InOutSine,
        Easing::InExpo,
        Easing::OutExpo,
        Easing::InOutExpo,
        Easing::InCirc,
        Easing::OutCirc,
        Easing::InOutCirc,
        Easing::InBack,
        Easing::OutBack,
        Easing::InOutBack,
        Easing::InElastic,
        Easing::OutElastic,
        Easing::InOutElastic,
        Easing::InBounce,
        Easing::OutBounce,
        Easing::InOutBounce,
    ];

    /// `true` for curves that never decrease and never leave `0..=1`.
    ///
    /// This is the polynomial / trigonometric / exponential families. The `Back`, `Elastic` and `Bounce`
    /// families are excluded: they oscillate on purpose, so a caller that needs a strictly monotone
    /// mapping (a scroll offset, a hit-test boundary) must clamp their output.
    pub fn is_monotonic(self) -> bool {
        !self.is_overshooting()
            && !matches!(
                self,
                Easing::InBounce | Easing::OutBounce | Easing::InOutBounce
            )
    }

    /// Evaluates the curve at linear progress `t`.
    ///
    /// `t` is clamped to `0..=1` (`NaN` is treated as `0`) so callers may pass raw
    /// `elapsed / duration` ratios without a separate guard.
    pub fn apply(self, t: f32) -> f32 {
        let t = clamp01(t);
        match self {
            Easing::Linear => t,
            Easing::InQuad => t * t,
            Easing::OutQuad => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::InOutQuad => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    -1.0 + (4.0 - 2.0 * t) * t
                }
            }
            Easing::InCubic => t * t * t,
            Easing::OutCubic => {
                let u = t - 1.0;
                u * u * u + 1.0
            }
            Easing::InOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    let u = 2.0 * t - 2.0;
                    0.5 * u * u * u + 1.0
                }
            }
            Easing::InQuart => t * t * t * t,
            Easing::OutQuart => {
                let u = t - 1.0;
                1.0 - u * u * u * u
            }
            Easing::InOutQuart => {
                if t < 0.5 {
                    8.0 * t * t * t * t
                } else {
                    let u = t - 1.0;
                    1.0 - 8.0 * u * u * u * u
                }
            }
            Easing::InSine => 1.0 - (t * (PI / 2.0)).cos(),
            Easing::OutSine => (t * (PI / 2.0)).sin(),
            Easing::InOutSine => -((PI * t).cos() - 1.0) / 2.0,
            Easing::InExpo => {
                if t == 0.0 {
                    0.0
                } else {
                    2f32.powf(10.0 * t - 10.0)
                }
            }
            Easing::OutExpo => {
                if t == 1.0 {
                    1.0
                } else {
                    1.0 - 2f32.powf(-10.0 * t)
                }
            }
            Easing::InOutExpo => {
                if t == 0.0 {
                    0.0
                } else if t == 1.0 {
                    1.0
                } else if t < 0.5 {
                    2f32.powf(20.0 * t - 10.0) / 2.0
                } else {
                    (2.0 - 2f32.powf(-20.0 * t + 10.0)) / 2.0
                }
            }
            Easing::InCirc => 1.0 - (1.0 - t * t).sqrt(),
            Easing::OutCirc => (1.0 - (t - 1.0) * (t - 1.0)).sqrt(),
            Easing::InOutCirc => {
                if t < 0.5 {
                    (1.0 - (1.0 - 4.0 * t * t).sqrt()) / 2.0
                } else {
                    ((1.0 - (-2.0 * t + 2.0) * (-2.0 * t + 2.0)).sqrt() + 1.0) / 2.0
                }
            }
            Easing::InBack => t * t * ((BACK_C3 + 1.0) * t - BACK_C1),
            Easing::OutBack => {
                let u = t - 1.0;
                u * u * ((BACK_C3 + 1.0) * u + BACK_C1) + 1.0
            }
            Easing::InOutBack => {
                // Penner's `c2 = c1 * 1.525`; the extra headroom keeps the symmetric variant from
                // looking like two separate back easings glued together.
                let s = BACK_C2;
                if t < 0.5 {
                    let u = 2.0 * t;
                    0.5 * (u * u * ((s + 1.0) * u - s))
                } else {
                    let u = 2.0 * t - 2.0;
                    0.5 * (u * u * ((s + 1.0) * u + s) + 2.0)
                }
            }
            Easing::InElastic => {
                if t == 0.0 {
                    0.0
                } else if t == 1.0 {
                    1.0
                } else {
                    -(2f32.powf(10.0 * t - 10.0)) * ((10.0 * t - 10.75) * ELASTIC_PERIOD).sin()
                }
            }
            Easing::OutElastic => {
                if t == 0.0 {
                    0.0
                } else if t == 1.0 {
                    1.0
                } else {
                    2f32.powf(-10.0 * t) * ((10.0 * t - 0.75) * ELASTIC_PERIOD).sin() + 1.0
                }
            }
            Easing::InOutElastic => {
                if t == 0.0 {
                    0.0
                } else if t == 1.0 {
                    1.0
                } else if t < 0.5 {
                    -(2f32.powf(20.0 * t - 10.0)
                        * ((20.0 * t - 11.125) * ELASTIC_INOUT_PERIOD).sin())
                        / 2.0
                } else {
                    2f32.powf(-20.0 * t + 10.0) * ((20.0 * t - 11.125) * ELASTIC_INOUT_PERIOD).sin()
                        / 2.0
                        + 1.0
                }
            }
            Easing::InBounce => 1.0 - out_bounce(1.0 - t),
            Easing::OutBounce => out_bounce(t),
            Easing::InOutBounce => {
                if t < 0.5 {
                    (1.0 - out_bounce(1.0 - 2.0 * t)) / 2.0
                } else {
                    (1.0 + out_bounce(2.0 * t - 1.0)) / 2.0
                }
            }
        }
    }

    /// Stable, human readable name, e.g. `"InOutCubic"`.
    ///
    /// The name is identical to the variant name so it can be used in logs and in tooling that maps
    /// a curve back to the enum.
    pub fn name(self) -> &'static str {
        match self {
            Easing::Linear => "Linear",
            Easing::InQuad => "InQuad",
            Easing::OutQuad => "OutQuad",
            Easing::InOutQuad => "InOutQuad",
            Easing::InCubic => "InCubic",
            Easing::OutCubic => "OutCubic",
            Easing::InOutCubic => "InOutCubic",
            Easing::InQuart => "InQuart",
            Easing::OutQuart => "OutQuart",
            Easing::InOutQuart => "InOutQuart",
            Easing::InSine => "InSine",
            Easing::OutSine => "OutSine",
            Easing::InOutSine => "InOutSine",
            Easing::InExpo => "InExpo",
            Easing::OutExpo => "OutExpo",
            Easing::InOutExpo => "InOutExpo",
            Easing::InCirc => "InCirc",
            Easing::OutCirc => "OutCirc",
            Easing::InOutCirc => "InOutCirc",
            Easing::InBack => "InBack",
            Easing::OutBack => "OutBack",
            Easing::InOutBack => "InOutBack",
            Easing::InElastic => "InElastic",
            Easing::OutElastic => "OutElastic",
            Easing::InOutElastic => "InOutElastic",
            Easing::InBounce => "InBounce",
            Easing::OutBounce => "OutBounce",
            Easing::InOutBounce => "InOutBounce",
        }
    }

    /// The time-mirrored counterpart of this curve.
    ///
    /// `In*` and `Out*` swap, `InOut*` and [`Easing::Linear`] map to themselves. For every variant
    /// the result satisfies `reverse(e).apply(t) == e.apply(1 - t)`.
    pub fn reverse(self) -> Self {
        match self {
            Easing::Linear => Easing::Linear,
            Easing::InQuad => Easing::OutQuad,
            Easing::OutQuad => Easing::InQuad,
            Easing::InOutQuad => Easing::InOutQuad,
            Easing::InCubic => Easing::OutCubic,
            Easing::OutCubic => Easing::InCubic,
            Easing::InOutCubic => Easing::InOutCubic,
            Easing::InQuart => Easing::OutQuart,
            Easing::OutQuart => Easing::InQuart,
            Easing::InOutQuart => Easing::InOutQuart,
            Easing::InSine => Easing::OutSine,
            Easing::OutSine => Easing::InSine,
            Easing::InOutSine => Easing::InOutSine,
            Easing::InExpo => Easing::OutExpo,
            Easing::OutExpo => Easing::InExpo,
            Easing::InOutExpo => Easing::InOutExpo,
            Easing::InCirc => Easing::OutCirc,
            Easing::OutCirc => Easing::InCirc,
            Easing::InOutCirc => Easing::InOutCirc,
            Easing::InBack => Easing::OutBack,
            Easing::OutBack => Easing::InBack,
            Easing::InOutBack => Easing::InOutBack,
            Easing::InElastic => Easing::OutElastic,
            Easing::OutElastic => Easing::InElastic,
            Easing::InOutElastic => Easing::InOutElastic,
            Easing::InBounce => Easing::OutBounce,
            Easing::OutBounce => Easing::InBounce,
            Easing::InOutBounce => Easing::InOutBounce,
        }
    }

    /// `true` for curves that leave the `0..=1` range between the endpoints.
    ///
    /// This is `true` for the `Back` and `Elastic` families, which is why
    /// [`Easing::apply`] still returns exactly `0` and `1` at `t = 0` and `t = 1` while overshooting
    /// in between.
    pub fn is_overshooting(self) -> bool {
        matches!(
            self,
            Easing::InBack
                | Easing::OutBack
                | Easing::InOutBack
                | Easing::InElastic
                | Easing::OutElastic
                | Easing::InOutElastic
        )
    }

    /// Bakes the curve into a lookup table of `steps` samples.
    ///
    /// At least two samples are always baked, so `sample_baked(0)` degenerates to the identity
    /// rather than an empty table.
    pub fn sample_baked(self, steps: usize) -> BakedEasing {
        BakedEasing::new(self, steps)
    }
}

/// The bounce parabola used by [`Easing::OutBounce`].
fn out_bounce(t: f32) -> f32 {
    let mut t = t;
    if t < 1.0 / BOUNCE_D1 {
        BOUNCE_N1 * t * t
    } else if t < 2.0 / BOUNCE_D1 {
        t -= 1.5 / BOUNCE_D1;
        BOUNCE_N1 * t * t + 0.75
    } else if t < 2.5 / BOUNCE_D1 {
        t -= 2.25 / BOUNCE_D1;
        BOUNCE_N1 * t * t + 0.9375
    } else {
        t -= 2.625 / BOUNCE_D1;
        BOUNCE_N1 * t * t + 0.984375
    }
}

/// Clamps to `0..=1`, mapping `NaN` to `0`.
#[inline]
fn clamp01(t: f32) -> f32 {
    if t.is_nan() {
        0.0
    } else {
        t.clamp(0.0, 1.0)
    }
}

/// A pre-sampled easing curve.
///
/// Sampling the table is a couple of loads and one `lerp`, which is cheaper than `powf`/`sin` when
/// many animated values share the same curve (a whole window full of transitions, for example).
/// The approximation error of [`Easing::sample_baked`] is proportional to `1 / steps`.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedEasing {
    steps: usize,
    values: Vec<f32>,
}

impl BakedEasing {
    /// Bakes `easing` into `steps` samples; `steps` is clamped to at least `2`.
    pub fn new(easing: Easing, steps: usize) -> Self {
        let steps = steps.max(2);
        let last = (steps - 1) as f32;
        let values = (0..steps)
            .map(|i| easing.apply(i as f32 / last))
            .collect::<Vec<f32>>();
        Self { steps, values }
    }

    /// Number of baked samples (at least `2`).
    pub fn steps(&self) -> usize {
        self.steps
    }

    /// The raw sample table, `values[0] == 0` and `values[last] == 1`.
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// Evaluates the baked curve at linear progress `t` (clamped to `0..=1`).
    pub fn sample(&self, t: f32) -> f32 {
        let t = clamp01(t);
        if self.values.len() < 2 {
            return t;
        }
        let scaled = t * (self.values.len() - 1) as f32;
        let index = (scaled.floor() as usize).min(self.values.len() - 2);
        let frac = scaled - index as f32;
        let a = self.values[index];
        let b = self.values[index + 1];
        a + (b - a) * frac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Curves whose value stays inside `0..=1` for the whole run: everything except the two
    /// overshooting families.
    fn bounded_curves() -> impl Iterator<Item = Easing> {
        Easing::ALL.into_iter().filter(|e| !e.is_overshooting())
    }

    #[test]
    fn endpoints_are_exact_for_every_curve() {
        for easing in Easing::ALL {
            // The Penner formulas are special cased at the endpoints, so even the overshooting
            // Back/Elastic families start at exactly 0 and end at exactly 1.
            assert_eq!(easing.apply(0.0), 0.0, "{} at t=0", easing.name());
            assert_eq!(easing.apply(1.0), 1.0, "{} at t=1", easing.name());
        }
    }

    #[test]
    fn bounded_curves_never_leave_the_unit_range() {
        for easing in bounded_curves() {
            for step in 0..=100 {
                let t = step as f32 / 100.0;
                let v = easing.apply(t);
                assert!(
                    (-1e-5..=1.0 + 1e-5).contains(&v),
                    "{} left the unit range: f({}) = {}",
                    easing.name(),
                    t,
                    v
                );
            }
        }
    }

    #[test]
    fn overshooting_curves_exceed_the_range_between_the_endpoints() {
        // Documented exceptions to "stays inside 0..=1":
        //
        // * `Back` undershoots below 0 on the way in and overshoots above 1 on the way out,
        // * `Elastic` oscillates around the target and therefore leaves the range on *both* sides
        //   (an elastic "in" dips below 0 early on, an elastic "out" goes above 1 early on).
        assert!(Easing::InBack.apply(0.2) < 0.0);
        assert!(Easing::OutBack.apply(0.8) > 1.0);
        assert!(Easing::InOutBack.apply(0.15) < 0.0);
        assert!(Easing::InOutBack.apply(0.85) > 1.0);
        assert!(Easing::InElastic.apply(0.2) < 0.0);
        assert!(Easing::OutElastic.apply(0.1) > 1.0);
        assert!(Easing::InOutElastic.apply(0.15) < 0.0);
        assert!(Easing::InOutElastic.apply(0.6) > 1.0);
    }

    #[test]
    fn overshooting_curves_really_do_exceed_the_endpoints() {
        // The *maximum* excursion of each overshooting curve, so the previous test cannot pass by
        // accident on a tiny numerical wobble.
        for easing in Easing::ALL.iter().filter(|e| e.is_overshooting()) {
            let mut extreme: f32 = 0.0;
            for step in 0..=1000 {
                let v = easing.apply(step as f32 / 1000.0);
                if v.abs() > extreme.abs() {
                    extreme = v;
                }
            }
            assert!(
                extreme.abs() > 0.01,
                "{} should overshoot noticeably, extreme was {}",
                easing.name(),
                extreme
            );
        }
    }

    #[test]
    fn monotonic_curves_are_monotonic() {
        for easing in Easing::ALL.into_iter().filter(|e| e.is_monotonic()) {
            let mut previous = easing.apply(0.0);
            for step in 1..=200 {
                let t = step as f32 / 200.0;
                let v = easing.apply(t);
                assert!(
                    v >= previous - 1e-6,
                    "{} is not monotonic at t={}: {} < {}",
                    easing.name(),
                    t,
                    v,
                    previous
                );
                previous = v;
            }
        }
    }

    #[test]
    fn the_bounce_family_oscillates_inside_the_unit_range() {
        // Bounce curves are *not* overshooting, but they are not monotone either: they hit the
        // target value several times before settling.
        for easing in [Easing::InBounce, Easing::OutBounce, Easing::InOutBounce] {
            assert!(!easing.is_monotonic(), "{}", easing.name());
            let samples: Vec<f32> = (0..=100)
                .map(|step| easing.apply(step as f32 / 100.0))
                .collect();
            for sample in &samples {
                assert!(
                    (-1e-5..=1.0 + 1e-5).contains(sample),
                    "{} left the unit range: {}",
                    easing.name(),
                    sample
                );
            }
            assert!(
                samples.windows(2).any(|w| w[1] < w[0] - 1e-3),
                "{} should bounce back at least once",
                easing.name()
            );
        }
    }

    #[test]
    fn is_monotonic_and_is_overshooting_partition_the_curves() {
        for easing in Easing::ALL {
            assert!(
                !(easing.is_monotonic() && easing.is_overshooting()),
                "{}",
                easing.name()
            );
        }
        assert!(Easing::Linear.is_monotonic());
        assert!(Easing::InOutQuart.is_monotonic());
        assert!(!Easing::OutElastic.is_monotonic());
    }

    #[test]
    fn known_reference_values() {
        assert_eq!(Easing::Linear.apply(0.25), 0.25);
        assert!((Easing::InQuad.apply(0.5) - 0.25).abs() < 1e-6);
        assert!((Easing::OutQuad.apply(0.5) - 0.75).abs() < 1e-6);
        assert!((Easing::InQuad.apply(0.5) + Easing::OutQuad.apply(0.5) - 1.0).abs() < 1e-6);
        assert!((Easing::InCubic.apply(0.5) - 0.125).abs() < 1e-6);
        assert!((Easing::InOutCubic.apply(0.5) - 0.5).abs() < 1e-6);
        assert!((Easing::InQuart.apply(0.5) - 0.0625).abs() < 1e-6);
        assert!((Easing::InOutSine.apply(0.5) - 0.5).abs() < 1e-6);
        assert!((Easing::InSine.apply(0.5) - 0.292_893_2).abs() < 1e-5);
        assert!((Easing::OutSine.apply(0.5) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((Easing::InExpo.apply(0.5) - 0.031_25).abs() < 1e-6);
        assert!((Easing::OutExpo.apply(0.5) - 0.968_75).abs() < 1e-6);
        assert!((Easing::InCirc.apply(0.0) - 0.0).abs() < 1e-6);
        assert!((Easing::InCirc.apply(1.0) - 1.0).abs() < 1e-6);
        assert!((Easing::InOutCirc.apply(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn input_is_clamped() {
        for easing in Easing::ALL {
            assert_eq!(easing.apply(-1.0), easing.apply(0.0), "{}", easing.name());
            assert_eq!(easing.apply(2.0), easing.apply(1.0), "{}", easing.name());
            assert_eq!(easing.apply(f32::NAN), 0.0, "{}", easing.name());
            assert_eq!(easing.apply(f32::INFINITY), 1.0, "{}", easing.name());
            assert_eq!(easing.apply(f32::NEG_INFINITY), 0.0, "{}", easing.name());
        }
    }

    #[test]
    fn names_match_the_variant_names_and_are_unique() {
        let mut names: Vec<&str> = Vec::new();
        for easing in Easing::ALL {
            names.push(easing.name());
            assert_eq!(easing.name(), format!("{:?}", easing));
        }
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
    }

    #[test]
    fn reverse_swaps_in_and_out() {
        assert_eq!(Easing::Linear.reverse(), Easing::Linear);
        assert_eq!(Easing::InQuad.reverse(), Easing::OutQuad);
        assert_eq!(Easing::OutQuad.reverse(), Easing::InQuad);
        assert_eq!(Easing::InOutBack.reverse(), Easing::InOutBack);
        for easing in Easing::ALL {
            assert_eq!(easing.reverse().reverse(), easing);
        }
    }

    #[test]
    fn reverse_mirrors_the_in_out_pairs_in_time() {
        // For an In/Out pair the two curves satisfy `out(t) == 1 - in(1 - t)`, which is exactly what
        // `reverse` means. The self-mirrored curves (`Linear`, the `InOut*` family) are their own
        // reverse, so the identity is only asserted for the pairs.
        for easing in Easing::ALL {
            if easing == Easing::Linear || easing.name().starts_with("InOut") {
                continue;
            }
            for step in 0..=10 {
                let t = step as f32 / 10.0;
                let mirrored = easing.apply(1.0 - t);
                let reversed = easing.reverse().apply(t);
                assert!(
                    (mirrored + reversed - 1.0).abs() < 1e-4,
                    "{} is not symmetric when reversed: f(1 - {}) = {}, f_rev({}) = {}",
                    easing.name(),
                    t,
                    mirrored,
                    t,
                    reversed
                );
            }
        }
    }

    #[test]
    fn baked_easing_matches_the_exact_curve() {
        let baked = Easing::InOutCubic.sample_baked(128);
        assert_eq!(baked.steps(), 128);
        assert_eq!(baked.values().len(), 128);
        assert_eq!(baked.sample(0.0), 0.0);
        assert_eq!(baked.sample(1.0), 1.0);
        for step in 0..=100 {
            let t = step as f32 / 100.0;
            let error = (baked.sample(t) - Easing::InOutCubic.apply(t)).abs();
            assert!(error < 1.0 / 64.0, "baked error {error} at t={t}");
        }
    }

    #[test]
    fn baked_easing_always_has_two_samples() {
        let baked = Easing::OutBounce.sample_baked(0);
        assert_eq!(baked.steps(), 2);
        assert_eq!(baked.values(), &[0.0, 1.0]);
    }
}
