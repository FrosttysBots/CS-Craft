//! Value ranges of density functions (vanilla `net.minecraft.util.Interval`).
//!
//! The density compiler uses ranges to drop `min`/`max` branches and to
//! short-circuit samplers, so endpoints must round exactly like vanilla.

use crate::mth;

/// A closed float range, or "not an interval" (both bounds NaN).
#[derive(Clone, Copy, Debug)]
pub struct Interval {
    min: f32,
    max: f32,
}

impl PartialEq for Interval {
    fn eq(&self, other: &Self) -> bool {
        (self.is_nai() && other.is_nai()) || (self.min == other.min && self.max == other.max)
    }
}

impl Interval {
    pub const NAI: Self = Self { min: f32::NAN, max: f32::NAN };
    pub const INFINITE: Self = Self { min: f32::NEG_INFINITY, max: f32::INFINITY };

    /// `Interval.of`; panics on reversed or NaN bounds like vanilla's exception.
    pub fn of(min: f32, max: f32) -> Self {
        assert!(!(max < min), "max ({max}) < min ({min})");
        assert!(!min.is_nan() && !max.is_nan(), "bounds cannot include NaN");
        Self { min, max }
    }

    pub fn symmetric(range: f32) -> Self {
        Self::of(-range, range)
    }

    pub fn exact(value: f32) -> Self {
        Self::of(value, value)
    }

    pub fn min(self) -> f32 {
        self.min
    }

    pub fn max(self) -> f32 {
        self.max
    }

    pub fn is_nai(self) -> bool {
        self.min.is_nan()
    }

    pub fn contains(self, value: f32) -> bool {
        value >= self.min && value <= self.max
    }

    pub fn encapsulating(intervals: &[Self]) -> Self {
        assert!(!intervals.is_empty(), "at least one interval required");
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for i in intervals.iter().filter(|i| !i.is_nai()) {
            lo = mth::min(i.min, lo);
            hi = mth::max(i.max, hi);
        }
        if hi < lo { Self::NAI } else { Self::of(lo, hi) }
    }

    pub fn encapsulating2(first: f32, second: f32) -> Self {
        match (first.is_nan(), second.is_nan()) {
            (true, true) => Self::NAI,
            (true, false) => Self::exact(second),
            (false, true) => Self::exact(first),
            _ => Self::of(mth::min(first, second), mth::max(first, second)),
        }
    }

    fn encapsulating_value(first: Self, second: f32) -> Self {
        if second.is_nan() {
            first
        } else if first.is_nai() {
            Self::exact(second)
        } else {
            Self::of(mth::min(first.min, second), mth::max(first.max, second))
        }
    }

    pub fn add(l: Self, r: Self) -> Self {
        let (lo, hi) = (l.min + r.min, l.max + r.max);
        if lo.is_nan() || hi.is_nan() { Self::NAI } else { Self::of(lo, hi) }
    }

    pub fn sub(l: Self, r: Self) -> Self {
        let (lo, hi) = (l.min - r.max, l.max - r.min);
        if lo.is_nan() || hi.is_nan() { Self::NAI } else { Self::of(lo, hi) }
    }

    fn mul_bound(l: f32, r: f32) -> f32 {
        if l == 0.0 || r == 0.0 { 0.0 } else { l * r }
    }

    pub fn mul(l: Self, r: Self) -> Self {
        if l.is_nai() || r.is_nai() {
            return Self::NAI;
        }
        let a = Self::mul_bound(l.min, r.min);
        let b = Self::mul_bound(l.min, r.max);
        let c = Self::mul_bound(l.max, r.min);
        let d = Self::mul_bound(l.max, r.max);
        Self::of(mth::min(mth::min(a, b), mth::min(c, d)), mth::max(mth::max(a, b), mth::max(c, d)))
    }

    pub fn reciprocal(i: Self) -> Self {
        if i.is_nai() || (i.min == 0.0 && i.max == 0.0) {
            Self::NAI
        } else if !i.contains(0.0) {
            Self::of(1.0 / i.max, 1.0 / i.min)
        } else if i.max == 0.0 {
            Self::of(f32::NEG_INFINITY, 1.0 / i.min)
        } else if i.min == 0.0 {
            Self::of(1.0 / i.max, f32::INFINITY)
        } else {
            Self::INFINITE
        }
    }

    pub fn div(l: Self, r: Self) -> Self {
        Self::mul(l, Self::reciprocal(r))
    }

    pub fn min_of(l: Self, r: Self) -> Self {
        if l.is_nai() || r.is_nai() { Self::NAI } else { Self::of(mth::min(l.min, r.min), mth::min(l.max, r.max)) }
    }

    pub fn max_of(l: Self, r: Self) -> Self {
        if l.is_nai() || r.is_nai() { Self::NAI } else { Self::of(mth::max(l.min, r.min), mth::max(l.max, r.max)) }
    }

    pub fn clamp(i: Self, lo: f32, hi: f32) -> Self {
        assert!(!(lo > hi), "min ({lo}) > max ({hi})");
        if i.is_nai() {
            Self::NAI
        } else if i.min >= hi {
            Self::of(hi, hi)
        } else if i.max <= lo {
            Self::of(lo, lo)
        } else {
            Self::of(mth::max(i.min, lo), mth::min(i.max, hi))
        }
    }

    pub fn abs(i: Self) -> Self {
        if i.is_nai() {
            return Self::NAI;
        }
        let hi = mth::max(i.min.abs(), i.max.abs());
        if i.contains(0.0) { Self::of(0.0, hi) } else { Self::of(mth::min(i.min.abs(), i.max.abs()), hi) }
    }

    pub fn square(i: Self) -> Self {
        if i.is_nai() {
            return Self::NAI;
        }
        let (a, b) = (i.min * i.min, i.max * i.max);
        let hi = mth::max(a, b);
        if i.contains(0.0) { Self::of(0.0, hi) } else { Self::of(mth::min(a, b), hi) }
    }

    /// `Interval.mapMonotonic`.
    pub fn map_monotonic(i: Self, op: impl Fn(f32) -> f32) -> Self {
        if i.is_nai() {
            return Self::NAI;
        }
        let (a, b) = (op(i.min), op(i.max));
        assert!(!a.is_nan() && !b.is_nan(), "monotonic operator should not produce NaN");
        Self::of(mth::min(a, b), mth::max(a, b))
    }

    pub fn log(i: Self) -> Self {
        if i.max < 0.0 {
            return Self::NAI;
        }
        Self::map_monotonic(Self::max_of(i, Self::exact(0.0)), |x| (x as f64).ln() as f32)
    }

    pub fn sign(i: Self) -> Self {
        if i.is_nai() {
            Self::NAI
        } else if i.min == i.max {
            Self::exact(mth::signum(i.min))
        } else if i.contains(0.0) {
            if i.min == 0.0 {
                Self::of(0.0, 1.0)
            } else if i.max == 0.0 {
                Self::of(-1.0, 0.0)
            } else {
                Self::of(-1.0, 1.0)
            }
        } else {
            Self::exact(if i.min > 0.0 { 1.0 } else { -1.0 })
        }
    }

    pub fn lerp(alpha: Self, first: Self, second: Self) -> Self {
        if alpha.is_nai() || first.is_nai() || second.is_nai() {
            return Self::NAI;
        }
        Self::encapsulating(&[
            Self::lerp_values(alpha, first.min, second.min),
            Self::lerp_values(alpha, first.max, second.min),
            Self::lerp_values(alpha, first.min, second.max),
            Self::lerp_values(alpha, first.max, second.max),
        ])
    }

    pub fn lerp_values(alpha: Self, first: f32, second: f32) -> Self {
        if alpha.is_nai() || first.is_nan() || second.is_nan() {
            return Self::NAI;
        }
        if first.is_finite() && second.is_finite() {
            let bound = |a: f32| first + Self::mul_bound(a, second - first);
            return Self::encapsulating2(bound(alpha.min), bound(alpha.max));
        }
        if first == second {
            return Self::exact(first);
        }
        let bound = |a: f32| {
            let first_part = Self::mul_bound(1.0 - a, first);
            let second_part = Self::mul_bound(a, second);
            if first_part.is_infinite() && second_part.is_infinite() {
                if a <= 0.0 {
                    return if second > first { f32::NEG_INFINITY } else { f32::INFINITY };
                }
                if a >= 1.0 {
                    return if second > first { f32::INFINITY } else { f32::NEG_INFINITY };
                }
                return f32::NAN;
            }
            first_part + second_part
        };
        let (lo, hi) = (bound(alpha.min), bound(alpha.max));
        if lo.is_nan() || hi.is_nan() { Self::NAI } else { Self::encapsulating2(lo, hi) }
    }

    pub fn pow(base: Self, exponent: Self) -> Self {
        if base.is_nai() || exponent.is_nai() {
            return Self::NAI;
        }
        if base.min == base.max {
            return Self::pow_value(base.min, exponent);
        }
        let mut result = Self::encapsulating(&[Self::pow_value(base.min, exponent), Self::pow_value(base.max, exponent)]);
        if base.contains(0.0) {
            if base.max > 0.0 {
                result = Self::encapsulating(&[result, Self::pow_value(0.0, exponent)]);
            }
            if base.min < 0.0 {
                result = Self::encapsulating(&[result, Self::pow_value(-0.0, exponent)]);
            }
        }
        result
    }

    fn java_pow(base: f32, exponent: f32) -> f32 {
        (base as f64).powf(exponent as f64) as f32
    }

    fn pow_value(base: f32, exponent: Self) -> Self {
        if base.is_nan() || exponent.is_nai() {
            return Self::NAI;
        }
        if exponent.min == exponent.max {
            let value = Self::java_pow(base, exponent.min);
            return if value.is_nan() { Self::NAI } else { Self::exact(value) };
        }
        if base == 0.0 {
            return Self::mul(Self::pow_zero_base(exponent), Self::exact(1.0f32.copysign(base)));
        }
        if base == 1.0 {
            return Self::exact(1.0);
        }
        if base > 0.0 {
            if exponent.min.is_finite() && exponent.max.is_finite() {
                return Self::encapsulating2(Self::java_pow(base, exponent.min), Self::java_pow(base, exponent.max));
            }
            return Self::pow_infinite_exponent(base, exponent);
        }
        Self::pow_negative_base(base, exponent)
    }

    fn pow_zero_base(exponent: Self) -> Self {
        if exponent.contains(0.0) {
            if exponent.max == 0.0 {
                Self::of(1.0, f32::INFINITY)
            } else if exponent.min == 0.0 {
                Self::of(0.0, 1.0)
            } else {
                Self::of(0.0, f32::INFINITY)
            }
        } else if exponent.max < 0.0 {
            Self::exact(f32::INFINITY)
        } else {
            Self::exact(0.0)
        }
    }

    fn pow_infinite_exponent(base: f32, exponent: Self) -> Self {
        if exponent.min.is_infinite() && exponent.max.is_infinite() {
            Self::of(0.0, f32::INFINITY)
        } else if exponent.min.is_infinite() {
            if base < 1.0 { Self::of(Self::java_pow(base, exponent.max), f32::INFINITY) } else { Self::of(0.0, Self::java_pow(base, exponent.max)) }
        } else if base < 1.0 {
            Self::of(0.0, Self::java_pow(base, exponent.min))
        } else {
            Self::of(Self::java_pow(base, exponent.min), f32::INFINITY)
        }
    }

    fn pow_negative_base(base: f32, exponent: Self) -> Self {
        let min_int = (exponent.min as f64).ceil() as f32;
        let max_int = (exponent.max as f64).floor() as f32;
        if max_int < min_int {
            return Self::NAI;
        }
        let to_min = Self::java_pow(base, min_int);
        let to_max = Self::java_pow(base, max_int);
        let mut result = Self::encapsulating2(to_min, to_max);
        if min_int.is_infinite() {
            result = Self::encapsulating_value(result, -to_min);
        } else if min_int + 1.0 < max_int {
            result = Self::encapsulating_value(result, Self::java_pow(base, min_int + 1.0));
        }
        if max_int.is_infinite() {
            result = Self::encapsulating_value(result, -to_max);
        } else if max_int - 1.0 > min_int {
            result = Self::encapsulating_value(result, Self::java_pow(base, max_int - 1.0));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_matches_vanilla_rules() {
        let a = Interval::of(-1.0, 2.0);
        let b = Interval::of(3.0, 4.0);
        assert_eq!(Interval::add(a, b), Interval::of(2.0, 6.0));
        assert_eq!(Interval::sub(a, b), Interval::of(-5.0, -1.0));
        assert_eq!(Interval::mul(a, b), Interval::of(-4.0, 8.0));
        assert_eq!(Interval::square(a), Interval::of(0.0, 4.0));
        assert_eq!(Interval::abs(Interval::of(-3.0, -1.0)), Interval::of(1.0, 3.0));
        assert_eq!(Interval::reciprocal(Interval::of(0.0, 2.0)), Interval::of(0.5, f32::INFINITY));
        assert!(Interval::reciprocal(Interval::exact(0.0)).is_nai());
        // mulBound treats 0 * inf as 0, not NaN.
        assert_eq!(Interval::mul(Interval::exact(0.0), Interval::INFINITE), Interval::exact(0.0));
        assert_eq!(Interval::clamp(Interval::of(5.0, 9.0), 0.0, 1.0), Interval::exact(1.0));
        assert_eq!(Interval::sign(Interval::of(0.0, 3.0)), Interval::of(0.0, 1.0));
    }
}
