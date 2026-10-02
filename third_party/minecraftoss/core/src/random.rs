//! Deterministic Java Edition 26.3 random streams used by world generation.
//! Source: `LegacyRandomSource`, `BitRandomSource`, `WorldgenRandom`,
//! `RandomSupport`, `Xoroshiro128PlusPlus`, `XoroshiroRandomSource` in the
//! remapped 26.3 common JAR identified in `specs/stage-1-worldgen.md`.

const LEGACY_MASK: u64 = (1_u64 << 48) - 1;
const LEGACY_MULTIPLIER: u64 = 0x5deece66d;
const GOLDEN_RATIO_64: u64 = 0x9e3779b97f4a7c15;
const SILVER_RATIO_64: u64 = 0x6a09e667f3bcc909;

#[derive(Clone, Debug)]
pub struct LegacyPositionalFactory {
    seed: i64,
}

impl LegacyPositionalFactory {
    pub fn at(&self, x: i32, y: i32, z: i32) -> LegacyRandom {
        LegacyRandom::new(positional_seed(x, y, z) ^ self.seed)
    }
    pub fn from_hash_of(&self, name: &str) -> LegacyRandom {
        LegacyRandom::new(i64::from(crate::seed::java_string_hash(name)) ^ self.seed)
    }
}

#[derive(Clone, Debug)]
pub struct LegacyRandom {
    state: u64,
    gaussian: Option<f64>,
}

impl LegacyRandom {
    pub fn new(seed: i64) -> Self {
        Self {
            state: ((seed as u64) ^ LEGACY_MULTIPLIER) & LEGACY_MASK,
            gaussian: None,
        }
    }

    pub fn set_seed(&mut self, seed: i64) {
        self.state = ((seed as u64) ^ LEGACY_MULTIPLIER) & LEGACY_MASK;
        self.gaussian = None;
    }

    /// The internal 48-bit state (`LegacyRandomSource.seed`).
    pub fn state(&self) -> u64 {
        self.state
    }

    /// The cached second Gaussian value (`MarsagliaPolarGaussian`).
    pub fn gaussian_cache(&self) -> Option<f64> {
        self.gaussian
    }

    pub fn set_gaussian_cache(&mut self, value: Option<f64>) {
        self.gaussian = value;
    }

    pub fn next_bits(&mut self, bits: u32) -> i32 {
        assert!((1..=32).contains(&bits));
        self.state = self.state.wrapping_mul(LEGACY_MULTIPLIER).wrapping_add(11) & LEGACY_MASK;
        (self.state >> (48 - bits)) as i32
    }

    pub fn next_i32(&mut self) -> i32 {
        self.next_bits(32)
    }

    pub fn next_i32_bound(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        if bound & (bound - 1) == 0 {
            return ((i64::from(bound) * i64::from(self.next_bits(31))) >> 31) as i32;
        }
        loop {
            let sample = self.next_bits(31);
            let modulo = sample % bound;
            if sample.wrapping_sub(modulo).wrapping_add(bound - 1) >= 0 {
                return modulo;
            }
        }
    }
    pub fn next_i64(&mut self) -> i64 {
        let upper = i64::from(self.next_i32());
        let lower = i64::from(self.next_i32());
        (upper << 32).wrapping_add(lower)
    }

    pub fn next_f32(&mut self) -> f32 {
        self.next_bits(24) as f32 * 5.960_464_5e-8_f32
    }

    pub fn next_f64(&mut self) -> f64 {
        let upper = i64::from(self.next_bits(26));
        let lower = i64::from(self.next_bits(27));
        ((upper << 27) + lower) as f64 * f64::from(1.110_223e-16_f32)
    }

    pub fn fork_positional(&mut self) -> LegacyPositionalFactory {
        LegacyPositionalFactory {
            seed: self.next_i64(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct XoroshiroRandom {
    lo: u64,
    hi: u64,
    gaussian: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct XoroshiroPositionalFactory {
    lo: u64,
    hi: u64,
}

impl XoroshiroPositionalFactory {
    pub fn at(&self, x: i32, y: i32, z: i32) -> XoroshiroRandom {
        let seed = positional_seed(x, y, z) as u64;
        XoroshiroRandom::from_state(seed ^ self.lo, self.hi)
    }

    pub fn from_hash_of(&self, name: &str) -> XoroshiroRandom {
        let digest = md5::compute(name.as_bytes());
        let lo = u64::from_be_bytes(digest[0..8].try_into().expect("MD5 half"));
        let hi = u64::from_be_bytes(digest[8..16].try_into().expect("MD5 half"));
        XoroshiroRandom::from_state(lo ^ self.lo, hi ^ self.hi)
    }
}

pub fn positional_seed(x: i32, y: i32, z: i32) -> i64 {
    // Mth.getSeed: the first product is a Java int before it widens to long.
    let seed = i64::from(x.wrapping_mul(3_129_871))
        ^ i64::from(z).wrapping_mul(116_129_781)
        ^ i64::from(y);
    seed.wrapping_mul(seed)
        .wrapping_mul(42_317_861)
        .wrapping_add(seed.wrapping_mul(11))
        >> 16
}

fn mix_stafford_13(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl XoroshiroRandom {
    pub fn new(seed: i64) -> Self {
        let lo = (seed as u64) ^ SILVER_RATIO_64;
        let hi = lo.wrapping_add(GOLDEN_RATIO_64);
        Self::from_state(mix_stafford_13(lo), mix_stafford_13(hi))
    }

    pub fn from_state(mut lo: u64, mut hi: u64) -> Self {
        if lo | hi == 0 {
            lo = GOLDEN_RATIO_64;
            hi = SILVER_RATIO_64;
        }
        Self { lo, hi, gaussian: None }
    }

    pub fn set_seed(&mut self, seed: i64) {
        *self = Self::new(seed);
    }

    pub fn next_i64(&mut self) -> i64 {
        let s0 = self.lo;
        let mut s1 = self.hi;
        let result = s0.wrapping_add(s1).rotate_left(17).wrapping_add(s0);
        s1 ^= s0;
        self.lo = s0.rotate_left(49) ^ s1 ^ (s1 << 21);
        self.hi = s1.rotate_left(28);
        result as i64
    }

    pub fn next_i32(&mut self) -> i32 {
        self.next_i64() as i32
    }

    pub fn next_i32_bound(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        let bound = bound as u32;
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let product = u64::from(self.next_i32() as u32) * u64::from(bound);
            if (product as u32) >= threshold {
                return (product >> 32) as i32;
            }
        }
    }

    pub fn next_bool(&mut self) -> bool {
        self.next_i64() & 1 != 0
    }
    pub fn next_f32(&mut self) -> f32 {
        ((self.next_i64() as u64) >> 40) as f32 * 5.960_464_5e-8_f32
    }
    pub fn next_f64(&mut self) -> f64 {
        ((self.next_i64() as u64) >> 11) as f64 * f64::from(1.110_223e-16_f32)
    }
    pub fn fork(&mut self) -> Self {
        Self::from_state(self.next_i64() as u64, self.next_i64() as u64)
    }

    pub fn fork_positional(&mut self) -> XoroshiroPositionalFactory {
        XoroshiroPositionalFactory {
            lo: self.next_i64() as u64,
            hi: self.next_i64() as u64,
        }
    }
}

/// The source behind a [`WorldgenRandom`]: Xoroshiro for world generation;
/// a legacy source when features run on the live level with its random
/// (`WorldgenRandom.next` delegates `next(bits)` to a `LegacyRandomSource`
/// directly, so draws match the level random's own).
#[derive(Clone, Debug)]
enum WorldgenSource {
    Xoroshiro(XoroshiroRandom),
    Legacy(LegacyRandom),
}

#[derive(Clone, Debug)]
pub struct WorldgenRandom {
    source: WorldgenSource,
    draws: u64,
    gaussian_next: Option<f64>,
}

impl WorldgenRandom {
    pub fn new(seed: i64) -> Self {
        Self {
            source: WorldgenSource::Xoroshiro(XoroshiroRandom::new(seed)),
            draws: 0,
            gaussian_next: None,
        }
    }
    /// Draws straight from a legacy random (the level random).
    pub fn from_legacy(random: LegacyRandom) -> Self {
        let gaussian_next = random.gaussian_cache();
        Self { source: WorldgenSource::Legacy(random), draws: 0, gaussian_next }
    }
    /// The legacy random back, with this random's Gaussian cache.
    pub fn into_legacy(self) -> Option<LegacyRandom> {
        match self.source {
            WorldgenSource::Legacy(mut random) => {
                random.set_gaussian_cache(self.gaussian_next);
                Some(random)
            }
            WorldgenSource::Xoroshiro(_) => None,
        }
    }
    pub fn draws(&self) -> u64 {
        self.draws
    }
    pub fn gaussian_cache_bits(&self) -> Option<u64> {
        self.gaussian_next.map(f64::to_bits)
    }
    pub fn set_gaussian_cache_bits(&mut self, bits: Option<u64>) {
        self.gaussian_next = bits.map(f64::from_bits);
    }
    pub fn set_seed(&mut self, seed: i64) {
        match &mut self.source {
            WorldgenSource::Xoroshiro(source) => source.set_seed(seed),
            WorldgenSource::Legacy(source) => source.set_seed(seed),
        }
    }
    pub fn next_i64(&mut self) -> i64 {
        // WorldgenRandom.nextLong uses BitRandomSource.nextLong: two next(32)
        // calls. For a Xoroshiro source, each next(32) consumes one long.
        let upper = i64::from(self.next_bits(32));
        let lower = i64::from(self.next_bits(32));
        (upper << 32).wrapping_add(lower)
    }
    pub fn next_bits(&mut self, bits: u32) -> i32 {
        self.draws += 1;
        match &mut self.source {
            WorldgenSource::Xoroshiro(source) => ((source.next_i64() as u64) >> (64 - bits)) as i32,
            WorldgenSource::Legacy(source) => source.next_bits(bits),
        }
    }
    pub fn next_i32_bound(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        if bound & (bound - 1) == 0 {
            return ((i64::from(bound) * i64::from(self.next_bits(31))) >> 31) as i32;
        }
        loop {
            let sample = self.next_bits(31);
            let modulo = sample % bound;
            if sample.wrapping_sub(modulo).wrapping_add(bound - 1) >= 0 {
                return modulo;
            }
        }
    }
    pub fn next_f32(&mut self) -> f32 {
        self.next_bits(24) as f32 * 5.960_464_5e-8_f32
    }
    pub fn next_f64(&mut self) -> f64 {
        let upper = i64::from(self.next_bits(26));
        let lower = i64::from(self.next_bits(27));
        ((upper << 27) + lower) as f64 * f64::from(1.110_223e-16_f32)
    }
    pub fn next_bool(&mut self) -> bool {
        self.next_bits(1) != 0
    }
    pub fn next_gaussian(&mut self) -> f64 {
        if let Some(saved) = self.gaussian_next.take() {
            return saved;
        }
        loop {
            let x = 2.0 * self.next_f64() - 1.0;
            let y = 2.0 * self.next_f64() - 1.0;
            let radius = x * x + y * y;
            if radius >= 1.0 || radius == 0.0 {
                continue;
            }
            let scale = (-2.0 * radius.ln() / radius).sqrt();
            self.gaussian_next = Some(y * scale);
            return x * scale;
        }
    }
    pub fn decoration_seed(&mut self, world_seed: i64, block_x: i32, block_z: i32) -> i64 {
        self.set_seed(world_seed);
        let x_scale = self.next_i64() | 1;
        let z_scale = self.next_i64() | 1;
        let result = i64::from(block_x)
            .wrapping_mul(x_scale)
            .wrapping_add(i64::from(block_z).wrapping_mul(z_scale))
            ^ world_seed;
        self.set_seed(result);
        result
    }
    pub fn feature_seed(&mut self, decoration_seed: i64, index: i32, step: i32) {
        self.set_seed(
            decoration_seed
                .wrapping_add(i64::from(index))
                .wrapping_add(i64::from(step.wrapping_mul(10_000))),
        );
    }
    pub fn large_feature_seed(&mut self, world_seed: i64, chunk_x: i32, chunk_z: i32) {
        self.set_seed(world_seed);
        let x_scale = self.next_i64();
        let z_scale = self.next_i64();
        self.set_seed(
            i64::from(chunk_x).wrapping_mul(x_scale)
                ^ i64::from(chunk_z).wrapping_mul(z_scale)
                ^ world_seed,
        );
    }
    pub fn large_feature_with_salt(&mut self, world_seed: i64, x: i32, z: i32, salt: i32) {
        let result = i64::from(x)
            .wrapping_mul(341_873_128_712)
            .wrapping_add(i64::from(z).wrapping_mul(132_897_987_541))
            .wrapping_add(world_seed)
            .wrapping_add(i64::from(salt));
        self.set_seed(result);
    }
}

/// Operations shared by vanilla's `RandomSource` implementations.
pub trait RandomSource {
    fn next_i32(&mut self) -> i32;
    fn next_i32_bound(&mut self, bound: i32) -> i32;
    fn next_i64(&mut self) -> i64;
    fn next_f32(&mut self) -> f32;
    fn next_f64(&mut self) -> f64;
    /// `RandomSource.nextBoolean`.
    fn next_bool(&mut self) -> bool;
    /// `RandomSource.nextGaussian` (`MarsagliaPolarGaussian`, one value cached).
    fn next_gaussian(&mut self) -> f64;
    /// `RandomSource.consumeCount`: advances by `count` `nextInt()` calls.
    fn consume_count(&mut self, count: usize) {
        for _ in 0..count {
            self.next_i32();
        }
    }
}

impl RandomSource for LegacyRandom {
    fn next_i32(&mut self) -> i32 {
        LegacyRandom::next_i32(self)
    }
    fn next_i32_bound(&mut self, bound: i32) -> i32 {
        LegacyRandom::next_i32_bound(self, bound)
    }
    fn next_i64(&mut self) -> i64 {
        LegacyRandom::next_i64(self)
    }
    fn next_f32(&mut self) -> f32 {
        LegacyRandom::next_f32(self)
    }
    fn next_f64(&mut self) -> f64 {
        LegacyRandom::next_f64(self)
    }
    fn next_bool(&mut self) -> bool {
        self.next_bits(1) != 0
    }
    fn next_gaussian(&mut self) -> f64 {
        if let Some(saved) = self.gaussian.take() {
            return saved;
        }
        let (value, saved) = marsaglia_polar(|| LegacyRandom::next_f64(self));
        self.gaussian = Some(saved);
        value
    }
}

/// `MarsagliaPolarGaussian.nextGaussian`: returns one value and the one to cache.
fn marsaglia_polar(mut next_f64: impl FnMut() -> f64) -> (f64, f64) {
    loop {
        let x = 2.0 * next_f64() - 1.0;
        let y = 2.0 * next_f64() - 1.0;
        let radius = x * x + y * y;
        if radius >= 1.0 || radius == 0.0 {
            continue;
        }
        let scale = (-2.0 * radius.ln() / radius).sqrt();
        return (x * scale, y * scale);
    }
}

impl RandomSource for XoroshiroRandom {
    fn next_i32(&mut self) -> i32 {
        XoroshiroRandom::next_i32(self)
    }
    fn next_i32_bound(&mut self, bound: i32) -> i32 {
        XoroshiroRandom::next_i32_bound(self, bound)
    }
    fn next_i64(&mut self) -> i64 {
        XoroshiroRandom::next_i64(self)
    }
    fn next_f32(&mut self) -> f32 {
        XoroshiroRandom::next_f32(self)
    }
    fn next_f64(&mut self) -> f64 {
        XoroshiroRandom::next_f64(self)
    }
    fn next_bool(&mut self) -> bool {
        XoroshiroRandom::next_bool(self)
    }
    fn next_gaussian(&mut self) -> f64 {
        if let Some(saved) = self.gaussian.take() {
            return saved;
        }
        let (value, saved) = marsaglia_polar(|| XoroshiroRandom::next_f64(self));
        self.gaussian = Some(saved);
        value
    }
}

/// A random source of either algorithm (`WorldgenRandom.Algorithm`).
#[derive(Clone, Debug)]
pub enum AnyRandom {
    Legacy(LegacyRandom),
    Xoroshiro(XoroshiroRandom),
}

impl AnyRandom {
    /// `WorldgenRandom.Algorithm.newInstance(seed)`.
    pub fn new(legacy: bool, seed: i64) -> Self {
        if legacy { Self::Legacy(LegacyRandom::new(seed)) } else { Self::Xoroshiro(XoroshiroRandom::new(seed)) }
    }

    pub fn fork_positional(&mut self) -> AnyPositional {
        match self {
            Self::Legacy(r) => AnyPositional::Legacy(r.fork_positional()),
            Self::Xoroshiro(r) => AnyPositional::Xoroshiro(r.fork_positional()),
        }
    }
}

impl RandomSource for AnyRandom {
    fn next_i32(&mut self) -> i32 {
        match self {
            Self::Legacy(r) => r.next_i32(),
            Self::Xoroshiro(r) => r.next_i32(),
        }
    }
    fn next_i32_bound(&mut self, bound: i32) -> i32 {
        match self {
            Self::Legacy(r) => r.next_i32_bound(bound),
            Self::Xoroshiro(r) => r.next_i32_bound(bound),
        }
    }
    fn next_i64(&mut self) -> i64 {
        match self {
            Self::Legacy(r) => r.next_i64(),
            Self::Xoroshiro(r) => r.next_i64(),
        }
    }
    fn next_f32(&mut self) -> f32 {
        match self {
            Self::Legacy(r) => r.next_f32(),
            Self::Xoroshiro(r) => r.next_f32(),
        }
    }
    fn next_f64(&mut self) -> f64 {
        match self {
            Self::Legacy(r) => r.next_f64(),
            Self::Xoroshiro(r) => r.next_f64(),
        }
    }
    fn next_bool(&mut self) -> bool {
        match self {
            Self::Legacy(r) => RandomSource::next_bool(r),
            Self::Xoroshiro(r) => RandomSource::next_bool(r),
        }
    }
    fn next_gaussian(&mut self) -> f64 {
        match self {
            Self::Legacy(r) => r.next_gaussian(),
            Self::Xoroshiro(r) => r.next_gaussian(),
        }
    }
}

impl RandomSource for WorldgenRandom {
    fn next_i32(&mut self) -> i32 {
        self.next_bits(32)
    }
    fn next_i32_bound(&mut self, bound: i32) -> i32 {
        WorldgenRandom::next_i32_bound(self, bound)
    }
    fn next_i64(&mut self) -> i64 {
        WorldgenRandom::next_i64(self)
    }
    fn next_f32(&mut self) -> f32 {
        WorldgenRandom::next_f32(self)
    }
    fn next_f64(&mut self) -> f64 {
        WorldgenRandom::next_f64(self)
    }
    fn next_bool(&mut self) -> bool {
        WorldgenRandom::next_bool(self)
    }
    fn next_gaussian(&mut self) -> f64 {
        WorldgenRandom::next_gaussian(self)
    }
}

/// A positional random factory of either algorithm.
#[derive(Clone, Debug)]
pub enum AnyPositional {
    Legacy(LegacyPositionalFactory),
    Xoroshiro(XoroshiroPositionalFactory),
}

impl AnyPositional {
    pub fn at(&self, x: i32, y: i32, z: i32) -> AnyRandom {
        match self {
            Self::Legacy(f) => AnyRandom::Legacy(f.at(x, y, z)),
            Self::Xoroshiro(f) => AnyRandom::Xoroshiro(f.at(x, y, z)),
        }
    }

    pub fn from_hash_of(&self, name: &str) -> AnyRandom {
        match self {
            Self::Legacy(f) => AnyRandom::Legacy(f.from_hash_of(name)),
            Self::Xoroshiro(f) => AnyRandom::Xoroshiro(f.from_hash_of(name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_legacy_known_vector() {
        let mut r = LegacyRandom::new(0);
        assert_eq!(r.next_i32(), -1_155_484_576); // java.util.Random(0).nextInt()
        assert_eq!(r.next_i32(), -723_955_400);
    }

    #[test]
    fn seed_upgrade_and_xoroshiro_zero_state() {
        let r = XoroshiroRandom::from_state(0, 0);
        assert_eq!((r.lo, r.hi), (GOLDEN_RATIO_64, SILVER_RATIO_64));
        let mut a = XoroshiroRandom::new(-123456789);
        let mut b = XoroshiroRandom::new(-123456789);
        for _ in 0..100 {
            assert_eq!(a.next_i64(), b.next_i64());
        }
    }

    #[test]
    fn signed_overflow_and_feature_seed_are_stable() {
        let mut a = WorldgenRandom::new(0);
        let value = a.decoration_seed(i64::MIN, -16, 16);
        let mut b = WorldgenRandom::new(0);
        assert_eq!(value, b.decoration_seed(i64::MIN, -16, 16));
        assert_eq!(a.draws(), 4);
        a.feature_seed(value, 42, 7);
        b.feature_seed(value, 42, 7);
        assert_eq!(a.next_i64(), b.next_i64());
    }

    #[test]
    fn pinned_26_3_minecraft_vectors() {
        // Captured with the pinned 26.3 remapped common JAR through
        // research/private/vectors/RandomVectors.java. Each row consumes
        // nextInt, nextInt(7), nextLong, nextFloat, nextDouble in order.
        let rows = [
            (
                0_i64,
                -1_155_484_576,
                2,
                4_437_113_781_045_784_766_i64,
                0x3f23_2dc9_u32,
                0x3fd3_c77c_08ce_970a_u64,
                -160_476_802,
                1,
                4_633_751_808_701_151_732_i64,
                0x3def_df28_u32,
                0x3fb9_86c1_cae1_d8f0_u64,
                7_069_528_835_409_849_632_i64,
                -3_787_864_342_176_001_462_i64,
            ),
            (
                -123_456_789_i64,
                1_442_175_866,
                5,
                -638_275_576_475_756_928_i64,
                0x3ef5_65de_u32,
                0x3fda_e865_4db1_6a3e_u64,
                1_475_262_532,
                0,
                -776_221_305_771_175_145_i64,
                0x3f4d_dacc_u32,
                0x3fc6_2e57_a876_cf60_u64,
                -7_802_466_808_044_889_013_i64,
                -6_686_399_352_354_891_802_i64,
            ),
            (
                i64::MIN,
                -1_155_484_576,
                2,
                4_437_113_781_045_784_766_i64,
                0x3f23_2dc9_u32,
                0x3fd3_c77c_08ce_970a_u64,
                -160_764_889,
                4,
                -3_866_801_757_029_155_188_i64,
                0x3f41_af67_u32,
                0x3fef_c327_1677_01a0_u64,
                5_977_109_141_121_285_184_i64,
                2_507_873_655_798_538_915_i64,
            ),
        ];
        for (seed, li, lb, ll, lf, ld, xi, xb, xl, xf, xd, ws, wl) in rows {
            let mut legacy = LegacyRandom::new(seed);
            assert_eq!(legacy.next_i32(), li, "legacy int seed {seed}");
            assert_eq!(legacy.next_i32_bound(7), lb, "legacy bound seed {seed}");
            assert_eq!(legacy.next_i64(), ll, "legacy long seed {seed}");
            assert_eq!(legacy.next_f32().to_bits(), lf, "legacy float seed {seed}");
            assert_eq!(legacy.next_f64().to_bits(), ld, "legacy double seed {seed}");
            let mut xoroshiro = XoroshiroRandom::new(seed);
            assert_eq!(xoroshiro.next_i32(), xi, "xoroshiro int seed {seed}");
            assert_eq!(
                xoroshiro.next_i32_bound(7),
                xb,
                "xoroshiro bound seed {seed}"
            );
            assert_eq!(xoroshiro.next_i64(), xl, "xoroshiro long seed {seed}");
            assert_eq!(
                xoroshiro.next_f32().to_bits(),
                xf,
                "xoroshiro float seed {seed}"
            );
            assert_eq!(
                xoroshiro.next_f64().to_bits(),
                xd,
                "xoroshiro double seed {seed}"
            );
            let mut worldgen = WorldgenRandom::new(0);
            assert_eq!(
                worldgen.decoration_seed(seed, -16, 16),
                ws,
                "decoration seed {seed}"
            );
            assert_eq!(worldgen.draws(), 4);
            assert_eq!(worldgen.next_i64(), wl, "worldgen long seed {seed}");
        }
    }
}
