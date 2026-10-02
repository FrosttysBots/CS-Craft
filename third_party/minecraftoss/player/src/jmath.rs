//! `Math.sin` and `Math.cos` as the pinned JVM computes them, to the last
//! bit where the game keeps the double result (random looks, strolls, item
//! throws). HotSpot's x86_64 intrinsics return the correctly rounded value
//! for all but about one argument in 800 (measured on Temurin 25 against a
//! 200-bit reference); the platform's `f64::sin` differs from them for about
//! one argument in 30. These compute the correctly rounded value: the
//! argument is reduced by pi/2 in double-double arithmetic (pi/2 split in
//! three 33-bit parts, as fdlibm splits it), the Taylor series is summed in
//! double-double, and the double-double result is rounded once. The
//! intrinsics' rare other roundings are not reproduced.

/// A double-double: `hi + lo` with `|lo| <= ulp(hi) / 2`.
#[derive(Clone, Copy, Debug)]
struct Dd {
    hi: f64,
    lo: f64,
}

fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let bb = s - a;
    Dd { hi: s, lo: (a - (s - bb)) + (b - bb) }
}

fn quick_two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    Dd { hi: s, lo: b - (s - a) }
}

fn two_prod(a: f64, b: f64) -> Dd {
    let p = a * b;
    Dd { hi: p, lo: a.mul_add(b, -p) }
}

impl Dd {
    fn from(x: f64) -> Self {
        Self { hi: x, lo: 0.0 }
    }

    fn add(self, other: Self) -> Self {
        let s = two_sum(self.hi, other.hi);
        let t = two_sum(self.lo, other.lo);
        let s = quick_two_sum(s.hi, s.lo + t.hi);
        quick_two_sum(s.hi, s.lo + t.lo)
    }

    fn neg(self) -> Self {
        Self { hi: -self.hi, lo: -self.lo }
    }

    fn mul(self, other: Self) -> Self {
        let p = two_prod(self.hi, other.hi);
        quick_two_sum(p.hi, p.lo + (self.hi * other.lo + self.lo * other.hi))
    }

    fn div_f64(self, d: f64) -> Self {
        let q1 = self.hi / d;
        let p = two_prod(q1, d);
        let rest = ((self.hi - p.hi) - p.lo + self.lo) / d;
        quick_two_sum(q1, rest)
    }
}

/// pi/2 as fdlibm splits it: three parts of 33 bits and the rest, so that
/// small multiples of the first three are exact.
const PIO2_1: f64 = f64::from_bits(0x3FF9_21FB_5440_0000);
const PIO2_2: f64 = f64::from_bits(0x3DD0_B461_1A60_0000);
const PIO2_3: f64 = f64::from_bits(0x3BA3_198A_2E00_0000);
const PIO2_3T: f64 = f64::from_bits(0x397B_839A_2520_49C1);
/// 2/pi.
const INV_PIO2: f64 = f64::from_bits(0x3FE4_5F30_6DC9_C883);

/// `x - k pi/2` in double-double for `|k| < 2^20`.
fn reduce(x: f64) -> (i64, Dd) {
    let k = (x * INV_PIO2).round();
    let mut r = two_sum(x, -k * PIO2_1);
    r = r.add(Dd::from(-k * PIO2_2));
    r = r.add(Dd::from(-k * PIO2_3));
    r = r.add(Dd::from(-k * PIO2_3T));
    (k as i64, r)
}

/// The series of `sin r` (odd) or `cos r` (even) for `|r| <= pi/4`.
fn series(r: Dd, odd: bool) -> Dd {
    let r2 = r.mul(r);
    let mut term = if odd { r } else { Dd::from(1.0) };
    let mut sum = term;
    let mut n = if odd { 1.0 } else { 0.0 };
    for _ in 0..30 {
        term = term.mul(r2).div_f64((n + 1.0) * (n + 2.0)).neg();
        n += 2.0;
        sum = sum.add(term);
        if term.hi.abs() < sum.hi.abs() * 1.0e-36 {
            break;
        }
    }
    sum
}

/// Arguments this handles exactly; others fall back to the platform's.
const LIMIT: f64 = 1.0e6;

/// `Math.sin`, correctly rounded.
pub fn sin(x: f64) -> f64 {
    if !x.is_finite() || x.abs() >= LIMIT {
        return x.sin();
    }
    // Below 2^-26 the cube term is under half an ulp: sin x rounds to x.
    if x.abs() < f64::from_bits(0x3E50_0000_0000_0000) {
        return x;
    }
    let (k, r) = reduce(x);
    let value = match k.rem_euclid(4) {
        0 => series(r, true),
        1 => series(r, false),
        2 => series(r, true).neg(),
        _ => series(r, false).neg(),
    };
    value.hi + value.lo
}

/// `Math.cos`, correctly rounded.
pub fn cos(x: f64) -> f64 {
    if !x.is_finite() || x.abs() >= LIMIT {
        return x.cos();
    }
    if x.abs() < f64::from_bits(0x3E40_0000_0000_0000) {
        return 1.0;
    }
    let (k, r) = reduce(x);
    let value = match k.rem_euclid(4) {
        0 => series(r, false),
        1 => series(r, true).neg(),
        2 => series(r, false).neg(),
        _ => series(r, true),
    };
    value.hi + value.lo
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_jvm_on_measured_arguments() {
        // Temurin 25 `Math.sin`/`Math.cos` (x86_64) where the platform's
        // functions round the other way.
        for (x, s) in [
            (4.009633200116204_f64, -0.7630639629122593_f64),
            (4.373526274823043, -0.943133328764149),
            (0.8039341593153455, 0.7200914875480277),
            (0.7757826690399453, 0.7002750162015416),
        ] {
            assert_eq!(sin(x).to_bits(), s.to_bits(), "sin {x}");
        }
        // One of the intrinsic's own roundings, not reproduced: the JVM
        // gives -0.4491878756024808, the correctly rounded value is next.
        assert_eq!(sin(3.607448796048443), -0.44918787560248086);
        assert_eq!(sin(0.0).to_bits(), 0.0_f64.to_bits());
        assert_eq!(sin(-0.0).to_bits(), (-0.0_f64).to_bits());
        assert_eq!(cos(0.0), 1.0);
        assert_eq!(sin(std::f64::consts::FRAC_PI_2), 1.0);
        assert!(sin(f64::NAN).is_nan());
    }
}
