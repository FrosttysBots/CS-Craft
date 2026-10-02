//! `LegacyRandomSource`: the pinned `RandomSource.create(seed)` 48-bit LCG.
use md5::{Digest, Md5};

pub trait LootRandom {
    fn next_float(&mut self) -> f32;
    fn next_int(&mut self, bound: u32) -> u32;
}
#[derive(Clone, Debug)]
pub struct LegacyRandom {
    state: u64,
    next_gaussian: Option<f64>,
}

impl Default for LegacyRandom {
    fn default() -> Self {
        Self::new(0)
    }
}

impl LegacyRandom {
    pub fn new(seed: u64) -> Self {
        Self {
            state: (seed ^ 0x5deece66d) & ((1 << 48) - 1),
            next_gaussian: None,
        }
    }

    /// Raw 48-bit state, for exact comparison with LegacyRandomSource.seed.
    pub fn raw_state(&self) -> u64 {
        self.state
    }

    /// A random at a raw 48-bit state (as `raw_state` reports it).
    pub fn from_raw_state(state: u64) -> Self {
        Self { state: state & ((1 << 48) - 1), next_gaussian: None }
    }

    fn bits(&mut self, count: u32) -> u32 {
        self.state = (self.state.wrapping_mul(0x5deece66d).wrapping_add(11)) & ((1 << 48) - 1);
        (self.state >> (48 - count)) as u32
    }

    pub fn next_float(&mut self) -> f32 {
        self.bits(24) as f32 / (1_u32 << 24) as f32
    }

    pub fn next_boolean(&mut self) -> bool {
        self.bits(1) != 0
    }

    pub fn next_double(&mut self) -> f64 {
        (((self.bits(26) as u64) << 27) | self.bits(27) as u64) as f64 / (1_u64 << 53) as f64
    }

    /// `MarsagliaPolarGaussian`, shared by the pinned legacy random source.
    pub fn next_gaussian(&mut self) -> f64 {
        if let Some(value) = self.next_gaussian.take() {
            return value;
        }
        loop {
            let x = 2.0 * self.next_double() - 1.0;
            let y = 2.0 * self.next_double() - 1.0;
            let radius = x * x + y * y;
            if radius >= 1.0 || radius == 0.0 {
                continue;
            }
            let scale = (-2.0 * radius.ln() / radius).sqrt();
            self.next_gaussian = Some(y * scale);
            return x * scale;
        }
    }

    pub fn next_long(&mut self) -> u64 {
        let upper = self.bits(32) as i32 as i64;
        let lower = self.bits(32) as i32 as i64;
        ((upper << 32).wrapping_add(lower)) as u64
    }

    pub fn next_int(&mut self, bound: u32) -> u32 {
        assert!(bound > 0 && bound <= i32::MAX as u32);
        let mask = bound - 1;
        if bound & mask == 0 {
            return ((bound as u64 * self.bits(31) as u64) >> 31) as u32;
        }
        loop {
            let bits = self.bits(31);
            let value = bits % bound;
            if bits <= (i32::MAX as u32).saturating_sub(mask).saturating_add(value) {
                return value;
            }
        }
    }
}

impl LootRandom for LegacyRandom {
    fn next_float(&mut self) -> f32 {
        LegacyRandom::next_float(self)
    }
    fn next_int(&mut self, bound: u32) -> u32 {
        LegacyRandom::next_int(self, bound)
    }
}

/// `RandomSequence`: one persistent Xoroshiro128++ stream per loot-table ID.
#[derive(Clone, Debug)]
pub struct XoroshiroRandom {
    low: u64,
    high: u64,
}

impl XoroshiroRandom {
    pub fn for_sequence(world_seed: u64, id: &str) -> Self {
        let hash = Md5::digest(id.as_bytes());
        let hash_low = u64::from_be_bytes(hash[0..8].try_into().expect("eight hash bytes"));
        let hash_high = u64::from_be_bytes(hash[8..16].try_into().expect("eight hash bytes"));
        let seed_low = world_seed ^ 0x6a09_e667_f3bc_c909;
        let seed_high = seed_low.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut low = mix_stafford13(seed_low ^ hash_low);
        let mut high = mix_stafford13(seed_high ^ hash_high);
        if low | high == 0 {
            low = 0x9e37_79b9_7f4a_7c15;
            high = 0x6a09_e667_f3bc_c909;
        }
        Self { low, high }
    }

    pub fn next_long(&mut self) -> u64 {
        let low = self.low;
        let mut high = self.high;
        let result = low.wrapping_add(high).rotate_left(17).wrapping_add(low);
        high ^= low;
        self.low = low.rotate_left(49) ^ high ^ (high << 21);
        self.high = high.rotate_left(28);
        result
    }
}

impl LootRandom for XoroshiroRandom {
    fn next_float(&mut self) -> f32 {
        (self.next_long() >> 40) as f32 * (1.0 / (1_u32 << 24) as f32)
    }
    fn next_int(&mut self, bound: u32) -> u32 {
        assert!(bound > 0);
        let mut product = (self.next_long() as u32 as u64) * bound as u64;
        let mut fractional = product as u32;
        if fractional < bound {
            let threshold = bound.wrapping_neg() % bound;
            while fractional < threshold {
                product = (self.next_long() as u32 as u64) * bound as u64;
                fractional = product as u32;
            }
        }
        (product >> 32) as u32
    }
}

fn mix_stafford13(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_random_seed_zero_first_double() {
        let mut rng = LegacyRandom::new(0);
        assert_eq!(rng.next_double().to_bits(), 0.730967787376657_f64.to_bits());
    }

    #[test]
    fn pinned_named_sequence_matches_java_26_3_probe() {
        let mut rng = XoroshiroRandom::for_sequence(123456789, "minecraft:blocks/oak_leaves");
        assert_eq!(rng.next_long(), 7649034988924242724);
        assert_eq!(rng.next_float().to_bits(), 1027336032);
        assert_eq!(rng.next_int(5), 1);
    }
}
