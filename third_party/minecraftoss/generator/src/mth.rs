//! Float helpers with vanilla `Mth` and `java.lang.Math` semantics.
//!
//! Rust's `f32::min`/`max`/`signum` differ from Java for NaN and signed zero,
//! and density results depend on those edge cases, so everything numeric in
//! the generator goes through these functions.

pub fn lerp(alpha: f32, p0: f32, p1: f32) -> f32 {
    p0 + alpha * (p1 - p0)
}

pub fn lerp2(a1: f32, a2: f32, x00: f32, x10: f32, x01: f32, x11: f32) -> f32 {
    lerp(a2, lerp(a1, x00, x10), lerp(a1, x01, x11))
}

#[allow(clippy::too_many_arguments)]
pub fn lerp3(a1: f32, a2: f32, a3: f32, x000: f32, x100: f32, x010: f32, x110: f32, x001: f32, x101: f32, x011: f32, x111: f32) -> f32 {
    lerp(a3, lerp2(a1, a2, x000, x100, x010, x110), lerp2(a1, a2, x001, x101, x011, x111))
}

pub fn lerp_f64(alpha: f64, p0: f64, p1: f64) -> f64 {
    p0 + alpha * (p1 - p0)
}

pub fn smoothstep(x: f32) -> f32 {
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

/// `Math.min(float, float)`: NaN wins and -0.0 is below 0.0.
pub fn min(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.to_bits() == (-0.0f32).to_bits() {
        return b;
    }
    if a <= b { a } else { b }
}

/// `Math.max(float, float)`: NaN wins and 0.0 is above -0.0.
pub fn max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.to_bits() == (-0.0f32).to_bits() {
        return b;
    }
    if a >= b { a } else { b }
}

/// `Mth.clamp(float, float, float)`.
pub fn clamp(value: f32, lo: f32, hi: f32) -> f32 {
    if value < lo { lo } else { min(value, hi) }
}

/// `Math.signum(float)`: keeps NaN and signed zeros.
pub fn signum(x: f32) -> f32 {
    if x.is_nan() || x == 0.0 { x } else if x > 0.0 { 1.0 } else { -1.0 }
}

/// `(int) Math.floor(double)`: saturating, NaN becomes 0 (Rust `as` matches).
pub fn floor(v: f64) -> i32 {
    v.floor() as i32
}

/// `Math.floorDiv(int, int)`.
pub fn floor_div(a: i32, b: i32) -> i32 {
    let q = a.wrapping_div(b);
    if (a % b != 0) && ((a ^ b) < 0) { q - 1 } else { q }
}

/// `Math.floorMod(int, int)`.
pub fn floor_mod(a: i32, b: i32) -> i32 {
    a.wrapping_sub(floor_div(a, b).wrapping_mul(b))
}

/// `Mth.inverseLerp(double, double, double)`.
pub fn inverse_lerp(value: f64, min: f64, max: f64) -> f64 {
    (value - min) / (max - min)
}

/// `Mth.clampedLerp(double, double, double)`.
pub fn clamped_lerp(factor: f64, min: f64, max: f64) -> f64 {
    if factor < 0.0 {
        min
    } else if factor > 1.0 {
        max
    } else {
        lerp_f64(factor, min, max)
    }
}

/// `Mth.clampedMap(double, ...)`.
pub fn clamped_map(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    clamped_lerp(inverse_lerp(value, from_min, from_max), to_min, to_max)
}

/// `Mth.map(double, ...)`.
pub fn map(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    lerp_f64(inverse_lerp(value, from_min, from_max), to_min, to_max)
}

/// `Mth.floor(float)`.
pub fn floor_f32(v: f32) -> i32 {
    v.floor() as i32
}

/// `Mth.ceil(double)`.
pub fn ceil(v: f64) -> i32 {
    v.ceil() as i32
}

/// `Mth.square(double)`.
pub fn square(v: f64) -> f64 {
    v * v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_edge_cases() {
        assert_eq!(min(0.0, -0.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(min(-0.0, 0.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(max(-0.0, 0.0).to_bits(), 0.0f32.to_bits());
        assert!(min(f32::NAN, 1.0).is_nan() && min(1.0, f32::NAN).is_nan());
        assert!(max(f32::NAN, 1.0).is_nan() && max(1.0, f32::NAN).is_nan());
        assert_eq!(signum(-0.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(signum(-3.0), -1.0);
        assert_eq!((floor_div(-7, 4), floor_mod(-7, 4)), (-2, 1));
        assert_eq!((floor_div(7, -4), floor_mod(7, -4)), (-2, -1));
        assert_eq!(floor(f64::NAN), 0);
        assert_eq!(floor(-1e20), i32::MIN);
        assert!(clamp(f32::NAN, 0.0, 1.0).is_nan());
    }
}

/// `Mth.inverseLerp(float, float, float)`.
pub fn inverse_lerp_f32(value: f32, min: f32, max: f32) -> f32 {
    (value - min) / (max - min)
}

/// `Mth.clampedLerp(float, float, float)`.
pub fn clamped_lerp_f32(factor: f32, min: f32, max: f32) -> f32 {
    if factor < 0.0 {
        min
    } else if factor > 1.0 {
        max
    } else {
        lerp(factor, min, max)
    }
}
