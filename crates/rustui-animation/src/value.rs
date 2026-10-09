//! Animatable scalars.
//!
//! An animation is only interesting if it can interpolate. RustUI keeps this narrow on purpose:
//! [`Animatable`] is implemented for the primitive numbers a widget is likely to animate (a float
//! offset, an integer index, an unsigned id) and for nothing else. Composite types are interpolated
//! component wise by animating each component (see `rustui-widgets`, which animates
//! `Vec2` offsets by animating `x` and `y` separately) — that keeps the trait object free and the
//! generated code inlinable.
//!
//! Integer types interpolate through `f32`/`f64` and round to nearest, so
//! `lerp(0u32, 10, 0.5) == 5`.

/// A value that can be interpolated towards another value of the same type.
///
/// The trait is implemented for `f32`, `f64`, `i32`, `i64`, `u32` and `u64`.
pub trait Animatable: Copy + PartialOrd + 'static {
    /// Interpolates from `self` at `t = 0` to `other` at `t = 1`.
    ///
    /// `t` outside `0..=1` extrapolates linearly.
    fn interpolate(self, other: Self, t: f32) -> Self;

    /// Lossy conversion to `f32`, used for diagnostics and for normalised progress reporting.
    fn to_f32(self) -> f32;

    /// Multiplies the value by `k`.
    ///
    /// Integer implementations round to nearest.
    fn scale(self, k: f32) -> Self;
}

/// Linear interpolation between `a` and `b`.
///
/// `t` is clamped to `0..=1`, so the result is always within the closed interval `[a, b]`
/// (after rounding, for integer types). `t == 0` returns `a` and `t == 1` returns `b` exactly —
/// this is what lets a finished animation report the exact target value instead of an accumulated
/// floating point approximation.
pub fn lerp<T: Animatable>(a: T, b: T, t: f32) -> T {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    if t <= 0.0 {
        a
    } else if t >= 1.0 {
        b
    } else {
        a.interpolate(b, t)
    }
}

/// Inverse of [`lerp`]: where `value` sits between `a` and `b`, as a `0..=1` ratio.
///
/// Returns `0.0` when `a == b` (nothing to interpolate) and clamps the result to `0..=1`.
pub fn inv_lerp<T: Animatable>(a: T, b: T, value: T) -> f32 {
    let span = a.to_f32() - b.to_f32();
    if span.abs() <= f32::EPSILON {
        return 0.0;
    }
    let t = (a.to_f32() - value.to_f32()) / span;
    if t.is_nan() {
        0.0
    } else {
        t.clamp(0.0, 1.0)
    }
}

macro_rules! impl_float_animatable {
    ($ty:ty) => {
        impl Animatable for $ty {
            #[inline]
            fn interpolate(self, other: Self, t: f32) -> Self {
                self + (other - self) * (t as f64 as $ty)
            }

            #[inline]
            fn to_f32(self) -> f32 {
                self as f32
            }

            #[inline]
            fn scale(self, k: f32) -> Self {
                self * (k as $ty)
            }
        }
    };
}

macro_rules! impl_int_animatable {
    ($ty:ty) => {
        impl Animatable for $ty {
            #[inline]
            fn interpolate(self, other: Self, t: f32) -> Self {
                if t <= 0.0 {
                    self
                } else if t >= 1.0 {
                    other
                } else {
                    let span = other as f64 - self as f64;
                    (self as f64 + span * t as f64).round() as $ty
                }
            }

            #[inline]
            fn to_f32(self) -> f32 {
                self as f32
            }

            #[inline]
            fn scale(self, k: f32) -> Self {
                ((self as f64) * k as f64).round() as $ty
            }
        }
    };
}

impl_float_animatable!(f32);
impl_float_animatable!(f64);
impl_int_animatable!(i32);
impl_int_animatable!(i64);
impl_int_animatable!(u32);
impl_int_animatable!(u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_endpoints_are_exact() {
        assert_eq!(lerp(0.0f32, 10.0, 0.0), 0.0);
        assert_eq!(lerp(0.0f32, 10.0, 1.0), 10.0);
        assert_eq!(lerp(3u32, 9u32, 0.0), 3);
        assert_eq!(lerp(3u32, 9u32, 1.0), 9);
    }

    #[test]
    fn lerp_midpoints_match_the_analytic_value() {
        assert!((lerp(0.0f32, 10.0, 0.5) - 5.0).abs() < 1e-6);
        assert!((lerp(-4.0f32, 4.0, 0.25) - -2.0).abs() < 1e-6);
        assert!((lerp(0.0f64, 1.0, 0.25) - 0.25).abs() < 1e-12);
        assert_eq!(lerp(0u32, 10, 0.5), 5);
        assert_eq!(lerp(0i32, -10, 0.5), -5);
        assert_eq!(lerp(0i64, 100, 0.25), 25);
        assert_eq!(lerp(0u64, 3, 0.5), 2); // 1.5 rounds to nearest even
    }

    #[test]
    fn lerp_clamps_t() {
        assert_eq!(lerp(0.0f32, 10.0, -5.0), 0.0);
        assert_eq!(lerp(0.0f32, 10.0, 5.0), 10.0);
        assert_eq!(lerp(0.0f32, 10.0, f32::NAN), 0.0);
        assert_eq!(lerp(2u32, 8u32, -1.0), 2);
        assert_eq!(lerp(2u32, 8u32, 9.0), 8);
    }

    #[test]
    fn descending_lerp_clamps_to_the_closed_interval() {
        assert_eq!(lerp(10.0f32, 0.0, 0.0), 10.0);
        assert_eq!(lerp(10.0f32, 0.0, 1.0), 0.0);
        assert_eq!(lerp(10.0f32, 0.0, -1.0), 10.0);
    }

    #[test]
    fn scale_multiplies() {
        assert!((5.0f32.scale(2.5) - 12.5).abs() < 1e-6);
        assert!((4.0f64.scale(0.5) - 2.0).abs() < 1e-12);
        assert_eq!(5i32.scale(2.0), 10);
        assert_eq!(5i64.scale(-1.0), -5);
        assert_eq!(5u32.scale(3.0), 15);
        assert_eq!(5u64.scale(0.0), 0);
    }

    #[test]
    fn to_f32_converts_every_supported_type() {
        assert_eq!(1.5f32.to_f32(), 1.5);
        assert_eq!(1.5f64.to_f32(), 1.5);
        assert_eq!((-7i32).to_f32(), -7.0);
        assert_eq!((-7i64).to_f32(), -7.0);
        assert_eq!(7u32.to_f32(), 7.0);
        assert_eq!(7u64.to_f32(), 7.0);
    }

    #[test]
    fn inv_lerp_is_the_inverse_of_lerp() {
        assert!((inv_lerp(0.0f32, 10.0, 2.5) - 0.25).abs() < 1e-6);
        assert!((inv_lerp(10.0f32, 0.0, 5.0) - 0.5).abs() < 1e-6);
        assert_eq!(inv_lerp(4u32, 4u32, 4), 0.0);
        assert_eq!(inv_lerp(0.0f32, 10.0, -1.0), 0.0);
        assert_eq!(inv_lerp(0.0f32, 10.0, 11.0), 1.0);
    }

    #[test]
    fn interpolate_extrapolates_outside_the_unit_range() {
        assert!((0.0f32.interpolate(10.0, 2.0) - 20.0).abs() < 1e-6);
        assert!((10.0f32.interpolate(0.0, -1.0) - 20.0).abs() < 1e-6);
    }
}
