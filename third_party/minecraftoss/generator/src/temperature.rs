//! Biome temperature at a block (vanilla `Biome.getTemperature`), used by
//! the snow material condition and frozen-ocean icebergs.

use minecraftoss_core::random::{LegacyRandom, RandomSource};

const SQRT_3: f64 = 1.732_050_807_568_877_2;

/// `SimplexNoise`, 2D form.
#[derive(Clone, Debug)]
pub struct Simplex {
    perms: [u8; 256],
    offset: [f64; 3],
}

const GRADIENT: [[i32; 3]; 12] = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1],
];

impl Simplex {
    /// `SimplexNoise(random, discardNoiseOffset)`: offsets still consume draws when discarded.
    pub fn new(random: &mut impl RandomSource, discard_offset: bool) -> Self {
        let scale = if discard_offset { 0.0 } else { 256.0 };
        let offset = [random.next_f64() * scale, random.next_f64() * scale, random.next_f64() * scale];
        let mut perms = [0u8; 256];
        for (i, p) in perms.iter_mut().enumerate() {
            *p = i as u8;
        }
        for i in 0..256 {
            let j = random.next_i32_bound(256 - i as i32) as usize + i;
            perms.swap(i, j);
        }
        Self { perms, offset }
    }

    fn permute(&self, x: i32) -> i32 {
        i32::from(self.perms[(x & 0xff) as usize])
    }

    fn corner(index: i32, x: f64, y: f64, base: f64) -> f64 {
        let mut t = base - x * x - y * y;
        if t < 0.0 {
            return 0.0;
        }
        t *= t;
        let g = GRADIENT[index as usize];
        t * t * (f64::from(g[0]) * x + f64::from(g[1]) * y + f64::from(g[2]) * 0.0)
    }

    pub fn get2(&self, x_in: f64, y_in: f64) -> f32 {
        let f2 = 0.5 * (SQRT_3 - 1.0);
        let g2 = (3.0 - SQRT_3) / 6.0;
        let xin = x_in + self.offset[0];
        let yin = y_in + self.offset[1];
        let s = (xin + yin) * f2;
        let i = crate::mth::floor(xin + s);
        let j = crate::mth::floor(yin + s);
        let t = f64::from(i.wrapping_add(j)) * g2;
        let x0 = xin - (f64::from(i) - t);
        let y0 = yin - (f64::from(j) - t);
        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let x1 = x0 - f64::from(i1) + g2;
        let y1 = y0 - f64::from(j1) + g2;
        let x2 = x0 - 1.0 + 2.0 * g2;
        let y2 = y0 - 1.0 + 2.0 * g2;
        let (ii, jj) = (i & 0xff, j & 0xff);
        let gi0 = self.permute(ii + self.permute(jj)) % 12;
        let gi1 = self.permute(ii + i1 + self.permute(jj + j1)) % 12;
        let gi2 = self.permute(ii + 1 + self.permute(jj + 1)) % 12;
        let n = Self::corner(gi0, x0, y0, 0.5) + Self::corner(gi1, x1, y1, 0.5) + Self::corner(gi2, x2, y2, 0.5);
        (70.0 * n) as f32
    }
}

/// The three fixed-seed noises behind biome temperature.
pub struct BiomeTemperature {
    temperature: Simplex,
    frozen: [(Simplex, f64, f32); 3],
    info: Simplex,
}

impl Default for BiomeTemperature {
    fn default() -> Self {
        Self::new()
    }
}

impl BiomeTemperature {
    pub fn new() -> Self {
        let temperature = Simplex::new(&mut LegacyRandom::new(1234), true);
        let mut r = LegacyRandom::new(3456);
        let frozen = [
            (Simplex::new(&mut r, true), 1.0, 0.142_857_15),
            (Simplex::new(&mut r, true), 0.5, 0.285_714_3),
            (Simplex::new(&mut r, true), 0.25, 0.571_428_6),
        ];
        let info = Simplex::new(&mut LegacyRandom::new(2345), true);
        Self { temperature, frozen, info }
    }

    fn frozen_noise(&self, x: f64, y: f64) -> f32 {
        let mut value = 0.0f32;
        for (noise, f, amplitude) in &self.frozen {
            value += amplitude * noise.get2(x * f, y * f);
        }
        value
    }

    /// `Biome.BIOME_INFO_NOISE.get(x, z)`.
    pub fn info(&self, x: f64, z: f64) -> f32 {
        self.info.get2(x, z)
    }

    /// `Biome.getHeightAdjustedTemperature` (the cache in vanilla does not change results).
    pub fn at(&self, base: f32, frozen_modifier: bool, x: i32, y: i32, z: i32, sea_level: i32) -> f32 {
        let mut adjusted = base;
        if frozen_modifier {
            let large = f64::from(self.frozen_noise(f64::from(x) * 0.05, f64::from(z) * 0.05) * 7.0);
            let edge = f64::from(self.info.get2(f64::from(x) * 0.2, f64::from(z) * 0.2));
            if large + edge < 0.3 && f64::from(self.info.get2(f64::from(x) * 0.09, f64::from(z) * 0.09)) < 0.8 {
                adjusted = 0.2;
            }
        }
        let snow_level = sea_level + 17;
        if y > snow_level {
            let v = self.temperature.get2(f64::from(x as f32 / 8.0), f64::from(z as f32 / 8.0)) * 8.0;
            return adjusted - (v + y as f32 - snow_level as f32) * 0.05 / 40.0;
        }
        adjusted
    }
}
