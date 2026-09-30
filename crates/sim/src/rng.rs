//! Small deterministic RNG (SplitMix64). The simulation must produce the
//! same spread and recoil on server, client prediction and replay, so all
//! randomness is derived from explicit seeds.

#[derive(Clone, Copy, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    /// Seed from several values (tick, player, shot...).
    pub fn from_parts(parts: &[u64]) -> Self {
        let mut h = 0xCBF2_9CE4_8422_2325u64;
        for p in parts {
            h ^= *p;
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
            h = h.rotate_left(27);
        }
        Self::new(h)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in [lo, hi).
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_in_range() {
        let mut a = Rng::from_parts(&[1, 2, 3]);
        let mut b = Rng::from_parts(&[1, 2, 3]);
        for _ in 0..1000 {
            let x = a.next_f32();
            assert_eq!(x, b.next_f32());
            assert!((0.0..1.0).contains(&x));
        }
        let mut c = Rng::from_parts(&[1, 2, 4]);
        assert_ne!(Rng::from_parts(&[1, 2, 3]).next_u64(), c.next_u64());
    }
}
