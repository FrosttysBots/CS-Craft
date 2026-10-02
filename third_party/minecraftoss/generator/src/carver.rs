//! Cave and canyon carvers writing into a carving mask (vanilla 26.3
//! `WorldCarver`, `CaveWorldCarver`, `CanyonWorldCarver`, `CarvingMask`).

use crate::providers::{self, FloatProvider, GenContext, HeightProvider, IntProvider};
use minecraftoss_core::random::{LegacyRandom, RandomSource};
use serde_json::Value;

/// `CarvingMask`: carved blocks of one chunk, one bit per block.
pub struct CarvingMask {
    min_y: i32,
    max_y: i32,
    height: i32,
    bits: Vec<u64>,
}

impl CarvingMask {
    pub fn new(min_y: i32, max_y: i32) -> Self {
        let height = max_y - min_y + 1;
        Self { min_y, max_y, height, bits: vec![0; (256 * height as usize).div_ceil(64)] }
    }

    fn index(&self, x: i32, y: i32, z: i32) -> usize {
        (y - self.min_y + (z + (x << 4)) * self.height) as usize
    }

    pub fn carve(&mut self, x: i32, y: i32, z: i32) {
        let i = self.index(x, y, z);
        self.bits[i / 64] |= 1 << (i % 64);
    }

    fn get(&self, i: usize) -> bool {
        self.bits[i / 64] & (1 << (i % 64)) != 0
    }

    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|&w| w == 0)
    }

    /// `CarvingMask.visit`: runs of carved blocks split per column, as `(x, z, bottom_y, top_y)`.
    pub fn visit(&self, mut visitor: impl FnMut(i32, i32, i32, i32)) {
        let total = 256 * self.height as usize;
        let mut i = 0;
        while i < total {
            if !self.get(i) {
                i += 1;
                continue;
            }
            let start = i;
            while i < total && self.get(i) {
                i += 1;
            }
            let end = i - 1;
            let (start_column, end_column) = (start as i32 / self.height, end as i32 / self.height);
            for column in start_column..=end_column {
                let base = column * self.height;
                let bottom = (start as i32 - base).max(0) + self.min_y;
                let top = (end as i32 - base).min(self.height - 1) + self.min_y;
                visitor(column >> 4 & 15, column & 15, bottom, top);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct CaveConfig {
    probability: f32,
    y: HeightProvider,
    count: IntProvider,
    thickness: FloatProvider,
    weird_thickness_bias: bool,
    room_vertical_radius_multiplier: FloatProvider,
    horizontal_radius_multiplier: FloatProvider,
    vertical_radius_multiplier: FloatProvider,
    start_vertical_radius_multiplier: FloatProvider,
    floor_level: FloatProvider,
}

#[derive(Clone, Debug)]
pub struct CanyonConfig {
    probability: f32,
    y: HeightProvider,
    vertical_rotation: FloatProvider,
    distance_factor: FloatProvider,
    thickness: FloatProvider,
    width_smoothness: i32,
    horizontal_radius_factor: FloatProvider,
    vertical_radius_default_factor: f32,
    vertical_radius_center_factor: f32,
    y_scale: FloatProvider,
}

#[derive(Clone, Debug)]
pub enum Carver {
    Cave(CaveConfig),
    Canyon(CanyonConfig),
}

fn float(json: &Value, key: &str) -> Result<FloatProvider, String> {
    FloatProvider::parse(json.get(key).ok_or_else(|| format!("carver lacks {key}"))?)
}

impl Carver {
    pub fn parse(json: &Value) -> Result<Self, String> {
        let probability = json["probability"].as_f64().ok_or("carver lacks probability")? as f32;
        let y = HeightProvider::parse(&json["y"])?;
        match json["type"].as_str() {
            Some("minecraft:cave") => Ok(Self::Cave(CaveConfig {
                probability,
                y,
                count: IntProvider::parse(&json["count"])?,
                thickness: float(json, "thickness")?,
                weird_thickness_bias: json.get("weird_thickness_bias").and_then(Value::as_bool).unwrap_or(false),
                room_vertical_radius_multiplier: float(json, "room_vertical_radius_multiplier")?,
                horizontal_radius_multiplier: float(json, "horizontal_radius_multiplier")?,
                vertical_radius_multiplier: float(json, "vertical_radius_multiplier")?,
                start_vertical_radius_multiplier: json.get("start_vertical_radius_multiplier").map(FloatProvider::parse).transpose()?.unwrap_or(FloatProvider::Constant(1.0)),
                floor_level: float(json, "floor_level")?,
            })),
            Some("minecraft:canyon") => {
                let shape = &json["shape"];
                Ok(Self::Canyon(CanyonConfig {
                    probability,
                    y,
                    vertical_rotation: float(json, "vertical_rotation")?,
                    distance_factor: float(shape, "distance_factor")?,
                    thickness: float(shape, "thickness")?,
                    width_smoothness: shape["width_smoothness"].as_i64().ok_or("canyon lacks width_smoothness")? as i32,
                    horizontal_radius_factor: float(shape, "horizontal_radius_factor")?,
                    vertical_radius_default_factor: shape["vertical_radius_default_factor"].as_f64().ok_or("canyon lacks vertical_radius_default_factor")? as f32,
                    vertical_radius_center_factor: shape["vertical_radius_center_factor"].as_f64().ok_or("canyon lacks vertical_radius_center_factor")? as f32,
                    y_scale: float(shape, "y_scale")?,
                }))
            }
            other => Err(format!("unsupported carver {other:?}")),
        }
    }

    pub fn is_start_chunk(&self, random: &mut impl RandomSource) -> bool {
        let p = match self {
            Self::Cave(c) => c.probability,
            Self::Canyon(c) => c.probability,
        };
        random.next_f32() <= p
    }

    /// `carve(context, random, chunkPos, sourceChunkPos, output)`.
    pub fn carve(&self, context: &GenContext, random: &mut impl RandomSource, chunk: (i32, i32), source: (i32, i32), mask: &mut CarvingMask) {
        match self {
            Self::Cave(c) => c.carve(context, random, chunk, source, mask),
            Self::Canyon(c) => c.carve(context, random, chunk, source, mask),
        }
    }
}

/// `WorldCarver.carveEllipsoid`.
#[allow(clippy::too_many_arguments)]
fn carve_ellipsoid(chunk: (i32, i32), x: f64, y: f64, z: f64, horizontal_radius: f64, vertical_radius: f64, mask: &mut CarvingMask, skip: &dyn Fn(f64, f64, f64, i32) -> bool) {
    let center_x = f64::from((chunk.0 << 4) + 8);
    let center_z = f64::from((chunk.1 << 4) + 8);
    let max_delta = 16.0 + horizontal_radius * 2.0;
    if (x - center_x).abs() > max_delta || (z - center_z).abs() > max_delta {
        return;
    }
    let (min_bx, min_bz) = (chunk.0 << 4, chunk.1 << 4);
    let min_xi = (crate::mth::floor(x - horizontal_radius) - min_bx - 1).max(0);
    let max_xi = (crate::mth::floor(x + horizontal_radius) - min_bx).min(15);
    let min_y = (crate::mth::floor(y - vertical_radius) - 1).max(mask.min_y);
    let max_y = (crate::mth::floor(y + vertical_radius) + 1).min(mask.max_y);
    let min_zi = (crate::mth::floor(z - horizontal_radius) - min_bz - 1).max(0);
    let max_zi = (crate::mth::floor(z + horizontal_radius) - min_bz).min(15);
    for xi in min_xi..=max_xi {
        let xd = (f64::from(min_bx + xi) + 0.5 - x) / horizontal_radius;
        for zi in min_zi..=max_zi {
            let zd = (f64::from(min_bz + zi) + 0.5 - z) / horizontal_radius;
            if xd * xd + zd * zd >= 1.0 {
                continue;
            }
            let mut wy = max_y;
            while wy > min_y {
                let yd = (f64::from(wy) - 0.5 - y) / vertical_radius;
                if !skip(xd, yd, zd, wy) {
                    mask.carve(xi, wy, zi);
                }
                wy -= 1;
            }
        }
    }
}

/// `WorldCarver.canReach`.
fn can_reach(chunk: (i32, i32), x: f64, z: f64, step: i32, total: i32, thickness: f32) -> bool {
    let xd = x - f64::from((chunk.0 << 4) + 8);
    let zd = z - f64::from((chunk.1 << 4) + 8);
    let remaining = f64::from(total - step);
    let rr = f64::from(thickness + 2.0 + 16.0);
    xd * xd + zd * zd - remaining * remaining <= rr * rr
}

impl CaveConfig {
    fn carve(&self, context: &GenContext, random: &mut impl RandomSource, chunk: (i32, i32), source: (i32, i32), mask: &mut CarvingMask) {
        let max_distance = (4 * 2 - 1) << 4;
        let count = self.count.sample(random);
        for _ in 0..count {
            let x = f64::from((source.0 << 4) + random.next_i32_bound(16));
            let y = f64::from(self.y.sample(random, context));
            let z = f64::from((source.1 << 4) + random.next_i32_bound(16));
            let horizontal = f64::from(self.horizontal_radius_multiplier.sample(random));
            let vertical = f64::from(self.vertical_radius_multiplier.sample(random));
            let start_vertical = f64::from(self.start_vertical_radius_multiplier.sample(random));
            let floor_level = f64::from(self.floor_level.sample(random));
            let skip = move |xd: f64, yd: f64, zd: f64, _y: i32| yd <= floor_level || xd * xd + yd * yd + zd * zd >= 1.0;
            let mut tunnels = 1;
            if random.next_i32_bound(4) == 0 {
                let y_scale = f64::from(self.room_vertical_radius_multiplier.sample(random));
                let thickness = 1.0 + random.next_f32() * 6.0;
                let horizontal_radius = 1.5 + f64::from(providers::sin(1.570_796_370_506_286_6) * thickness);
                carve_ellipsoid(chunk, x + 1.0, y, z, horizontal_radius, horizontal_radius * y_scale, mask, &skip);
                tunnels += random.next_i32_bound(4);
            }
            for _ in 0..tunnels {
                let horizontal_rotation = random.next_f32() * (std::f32::consts::PI * 2.0);
                let vertical_rotation = (random.next_f32() - 0.5) / 4.0;
                let mut thickness = self.thickness.sample(random);
                if self.weird_thickness_bias && random.next_i32_bound(10) == 0 {
                    thickness *= random.next_f32() * random.next_f32() * 3.0 + 1.0;
                }
                let distance = max_distance - random.next_i32_bound(max_distance / 4);
                let seed = random.next_i64();
                self.tunnel(chunk, seed, x, y, z, horizontal, vertical, thickness, horizontal_rotation, vertical_rotation, 0, distance, start_vertical, mask, &skip);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn tunnel(&self, chunk: (i32, i32), seed: i64, mut x: f64, mut y: f64, mut z: f64, h_mult: f64, v_mult: f64, thickness: f32, mut h_rot: f32, mut v_rot: f32, step: i32, dist: i32, y_scale: f64, mask: &mut CarvingMask, skip: &dyn Fn(f64, f64, f64, i32) -> bool) {
        let mut random = LegacyRandom::new(seed);
        let split = random.next_i32_bound(dist / 2) + dist / 4;
        let steep = random.next_i32_bound(6) == 0;
        let (mut y_rota, mut x_rota) = (0.0f32, 0.0f32);
        for current in step..dist {
            let horizontal_radius = 1.5 + f64::from(providers::sin(f64::from(std::f32::consts::PI * current as f32 / dist as f32)) * thickness);
            let vertical_radius = horizontal_radius * y_scale;
            let cos_x = providers::cos(f64::from(v_rot));
            x += f64::from(providers::cos(f64::from(h_rot)) * cos_x);
            y += f64::from(providers::sin(f64::from(v_rot)));
            z += f64::from(providers::sin(f64::from(h_rot)) * cos_x);
            v_rot *= if steep { 0.92 } else { 0.7 };
            v_rot += x_rota * 0.1;
            h_rot += y_rota * 0.1;
            x_rota *= 0.9;
            y_rota *= 0.75;
            x_rota += (random.next_f32() - random.next_f32()) * random.next_f32() * 2.0;
            y_rota += (random.next_f32() - random.next_f32()) * random.next_f32() * 4.0;
            if current == split && thickness > 1.0 {
                let s1 = random.next_i64();
                let t1 = random.next_f32() * 0.5 + 0.5;
                self.tunnel(chunk, s1, x, y, z, h_mult, v_mult, t1, h_rot - std::f32::consts::FRAC_PI_2, v_rot / 3.0, current, dist, 1.0, mask, skip);
                let s2 = random.next_i64();
                let t2 = random.next_f32() * 0.5 + 0.5;
                self.tunnel(chunk, s2, x, y, z, h_mult, v_mult, t2, h_rot + std::f32::consts::FRAC_PI_2, v_rot / 3.0, current, dist, 1.0, mask, skip);
                return;
            }
            if random.next_i32_bound(4) == 0 {
                continue;
            }
            if !can_reach(chunk, x, z, current, dist, thickness) {
                return;
            }
            carve_ellipsoid(chunk, x, y, z, horizontal_radius * h_mult, vertical_radius * v_mult, mask, skip);
        }
    }
}

impl CanyonConfig {
    fn carve(&self, context: &GenContext, random: &mut impl RandomSource, chunk: (i32, i32), source: (i32, i32), mask: &mut CarvingMask) {
        let max_distance = (4 * 2 - 1) * 16;
        let x = f64::from((source.0 << 4) + random.next_i32_bound(16));
        let y = self.y.sample(random, context);
        let z = f64::from((source.1 << 4) + random.next_i32_bound(16));
        let horizontal_rotation = random.next_f32() * (std::f32::consts::PI * 2.0);
        let vertical_rotation = self.vertical_rotation.sample(random);
        let y_scale = f64::from(self.y_scale.sample(random));
        let thickness = self.thickness.sample(random);
        let distance = (max_distance as f32 * self.distance_factor.sample(random)) as i32;
        let seed = random.next_i64();
        self.tunnel(context, chunk, seed, x, f64::from(y), z, thickness, horizontal_rotation, vertical_rotation, distance, y_scale, mask);
    }

    #[allow(clippy::too_many_arguments)]
    fn tunnel(&self, context: &GenContext, chunk: (i32, i32), seed: i64, mut x: f64, mut y: f64, mut z: f64, thickness: f32, mut h_rot: f32, mut v_rot: f32, distance: i32, y_scale: f64, mask: &mut CarvingMask) {
        let mut random = LegacyRandom::new(seed);
        let mut width = vec![0.0f32; context.depth as usize];
        let mut factor = 1.0f32;
        for (i, w) in width.iter_mut().enumerate() {
            if i == 0 || random.next_i32_bound(self.width_smoothness) == 0 {
                factor = 1.0 + random.next_f32() * random.next_f32();
            }
            *w = factor * factor;
        }
        let (mut y_rota, mut x_rota) = (0.0f32, 0.0f32);
        let min_gen_y = context.min_y;
        for current in 0..distance {
            let mut horizontal_radius = 1.5 + f64::from(providers::sin(f64::from(current as f32 * std::f32::consts::PI / distance as f32)) * thickness);
            let mut vertical_radius = horizontal_radius * y_scale;
            horizontal_radius *= f64::from(self.horizontal_radius_factor.sample(&mut random));
            let multiplier = 1.0 - (0.5 - current as f32 / distance as f32).abs() * 2.0;
            let f = self.vertical_radius_default_factor + self.vertical_radius_center_factor * multiplier;
            vertical_radius = f64::from(f) * vertical_radius * f64::from(providers::random_between(&mut random, 0.75, 1.0));
            let xc = providers::cos(f64::from(v_rot));
            let xs = providers::sin(f64::from(v_rot));
            x += f64::from(providers::cos(f64::from(h_rot)) * xc);
            y += f64::from(xs);
            z += f64::from(providers::sin(f64::from(h_rot)) * xc);
            v_rot *= 0.7;
            v_rot += x_rota * 0.05;
            h_rot += y_rota * 0.05;
            x_rota *= 0.8;
            y_rota *= 0.5;
            x_rota += (random.next_f32() - random.next_f32()) * random.next_f32() * 2.0;
            y_rota += (random.next_f32() - random.next_f32()) * random.next_f32() * 4.0;
            if random.next_i32_bound(4) == 0 {
                continue;
            }
            if !can_reach(chunk, x, z, current, distance, thickness) {
                return;
            }
            let width = &width;
            carve_ellipsoid(chunk, x, y, z, horizontal_radius, vertical_radius, mask, &|xd, yd, zd, wy| {
                let index = (wy - min_gen_y) as usize;
                (xd * xd + zd * zd) * f64::from(width[index - 1]) + yd * yd / 6.0 >= 1.0
            });
        }
    }
}
