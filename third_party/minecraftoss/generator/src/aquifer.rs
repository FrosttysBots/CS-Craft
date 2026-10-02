//! Aquifers (vanilla 26.3 `Aquifer.NoiseBasedAquifer` and the disabled form).
//!
//! Density samples go through the chunk's shared cached context, as in
//! vanilla, because cache state from one caller serves later callers.

use crate::density::{Context, Id};
use crate::mth;
use crate::noise::Volume;
use minecraftoss_core::BlockStateId;
use minecraftoss_core::random::{AnyPositional, RandomSource};
use std::collections::HashMap;

/// `DimensionType.WAY_BELOW_MIN_Y`.
pub const WAY_BELOW_MIN_Y: i32 = -2_032 * 2 - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidStatus {
    pub level: i32,
    pub fluid: BlockStateId,
}

impl FluidStatus {
    pub fn at(self, y: i32, air: BlockStateId) -> BlockStateId {
        if y < self.level { self.fluid } else { air }
    }
}

/// Fixed states the aquifer reasons about.
#[derive(Clone, Copy, Debug)]
pub struct FluidBlocks {
    pub air: BlockStateId,
    pub water: BlockStateId,
    pub lava: BlockStateId,
}

/// `NoiseBasedChunkGenerator.createFluidPicker`.
#[derive(Clone, Copy, Debug)]
pub struct GlobalFluidPicker {
    pub sea_level: i32,
    pub default_fluid: BlockStateId,
    pub lava: BlockStateId,
}

impl GlobalFluidPicker {
    pub fn compute(&self, _x: i32, y: i32, _z: i32) -> FluidStatus {
        if y < (-54).min(self.sea_level) {
            FluidStatus { level: -54, fluid: self.lava }
        } else {
            FluidStatus { level: self.sea_level, fluid: self.default_fluid }
        }
    }
}

/// Density functions of `Aquifer.Config`, compiled.
#[derive(Clone, Copy, Debug)]
pub struct AquiferSamplers {
    pub barrier: Id,
    pub floodedness: Id,
    pub spread: Id,
    pub lava: Id,
    pub exclusion: Id,
    pub surface_level: Id,
}

fn similarity(d1: i32, d2: i32) -> f64 {
    1.0 - f64::from(d2 - d1) / 25.0
}

fn grid_x(c: i32) -> i32 {
    c >> 4
}
fn grid_y(c: i32) -> i32 {
    mth::floor_div(c, 12)
}
fn from_grid_x(g: i32, o: i32) -> i32 {
    (g << 4) + o
}
fn from_grid_y(g: i32, o: i32) -> i32 {
    g * 12 + o
}

/// `ChunkPos.pack(x, z)` for the surface-level cache key.
fn pack(x: i32, z: i32) -> i64 {
    (i64::from(x) & 0xffff_ffff) | ((i64::from(z) & 0xffff_ffff) << 32)
}

pub enum Aquifer {
    Disabled { picker: GlobalFluidPicker, blocks: FluidBlocks },
    Noise(Box<NoiseAquifer>),
}

impl Aquifer {
    /// `computeSubstance`: `None` means "use the default block".
    pub fn compute_substance(&mut self, ctx: &mut Context, x: i32, y: i32, z: i32, density: f64) -> Option<BlockStateId> {
        match self {
            Self::Disabled { picker, blocks } => {
                if density > 0.0 {
                    None
                } else {
                    Some(picker.compute(x, y, z).at(y, blocks.air))
                }
            }
            Self::Noise(a) => a.compute_substance(ctx, x, y, z, density),
        }
    }

    pub fn should_schedule_fluid_update(&self) -> bool {
        match self {
            Self::Disabled { .. } => false,
            Self::Noise(a) => a.should_schedule_fluid_update,
        }
    }
}

pub struct NoiseAquifer {
    samplers: AquiferSamplers,
    random: AnyPositional,
    picker: GlobalFluidPicker,
    blocks: FluidBlocks,
    status_cache: Vec<Option<FluidStatus>>,
    location_cache: Vec<Option<(i32, i32, i32)>>,
    surface_cache: HashMap<i64, i32>,
    should_schedule_fluid_update: bool,
    skip_sampling_above_y: i32,
    min_grid: [i32; 3],
    grid_size_x: i32,
    grid_size_z: i32,
    /// The grid cell last sampled and its twelve candidate sources (cache
    /// index and location) in search order.
    candidates_of: Option<(i32, i32, i32)>,
    candidates: [(usize, (i32, i32, i32)); 12],
}

const SURFACE_SAMPLING_OFFSETS: [[i32; 2]; 13] = [[0, 0], [-2, -1], [-1, -1], [0, -1], [1, -1], [-3, 0], [-2, 0], [-1, 0], [1, 0], [-2, 1], [-1, 1], [0, 1], [1, 1]];

impl NoiseAquifer {
    pub fn new(ctx: &mut Context, samplers: AquiferSamplers, random: AnyPositional, volume: &Volume, picker: GlobalFluidPicker, blocks: FluidBlocks) -> Self {
        let min_grid_x = grid_x(volume.min[0] - 5);
        let max_grid_x = grid_x(volume.max_block(0) - 5) + 1;
        let min_grid_y = grid_y(volume.min[1] + 1) - 1;
        let max_grid_y = grid_y(volume.max_block(1) + 1) + 1;
        let min_grid_z = grid_x(volume.min[2] - 5);
        let max_grid_z = grid_x(volume.max_block(2) - 5) + 1;
        let (gx, gy, gz) = (max_grid_x - min_grid_x + 1, max_grid_y - min_grid_y + 1, max_grid_z - min_grid_z + 1);
        let total = (gx * gy * gz) as usize;
        let mut aquifer = Self {
            samplers,
            random,
            picker,
            blocks,
            status_cache: vec![None; total],
            location_cache: vec![None; total],
            surface_cache: HashMap::new(),
            should_schedule_fluid_update: false,
            skip_sampling_above_y: 0,
            min_grid: [min_grid_x, min_grid_y, min_grid_z],
            grid_size_x: gx,
            grid_size_z: gz,
            candidates_of: None,
            candidates: [(0, (0, 0, 0)); 12],
        };
        let max_surface = aquifer.max_surface_level(ctx, from_grid_x(min_grid_x, 0), from_grid_x(min_grid_z, 0), from_grid_x(max_grid_x, 9), from_grid_x(max_grid_z, 9));
        let skip_grid_y = grid_y(max_surface + 8 + 12) + 1;
        aquifer.skip_sampling_above_y = from_grid_y(skip_grid_y, 11) - 1;
        aquifer
    }

    fn surface_level(&mut self, ctx: &mut Context, x: i32, z: i32) -> i32 {
        let (qx, qz) = ((x >> 2) << 2, (z >> 2) << 2);
        let key = pack(qx, qz);
        if let Some(&v) = self.surface_cache.get(&key) {
            return v;
        }
        let v = mth::floor(f64::from(ctx.value(self.samplers.surface_level, qx, 0, qz)));
        self.surface_cache.insert(key, v);
        v
    }

    fn max_surface_level(&mut self, ctx: &mut Context, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> i32 {
        let (min_qx, max_qx, min_qz, max_qz) = (min_x >> 2, max_x >> 2, min_z >> 2, max_z >> 2);
        let v = Volume::new([max_qx - min_qx + 1, 1, max_qz - min_qz + 1], [min_qx << 2, 0, min_qz << 2], [4, 1, 4]);
        let buffer = ctx.sample(self.samplers.surface_level, &v);
        let mut max_y = i32::MIN;
        for z in 0..v.size[2] {
            for x in 0..v.size[0] {
                let level = mth::floor(f64::from(buffer[v.index(x, 0, z)]));
                self.surface_cache.insert(pack(v.block_x(x), v.block_z(z)), level);
                max_y = max_y.max(level);
            }
        }
        max_y
    }

    fn index(&self, gx: i32, gy: i32, gz: i32) -> usize {
        let (x, y, z) = (gx - self.min_grid[0], gy - self.min_grid[1], gz - self.min_grid[2]);
        ((y * self.grid_size_z + z) * self.grid_size_x + x) as usize
    }

    fn is(&self, state: BlockStateId, block: BlockStateId) -> bool {
        state == block
    }

    fn compute_substance(&mut self, ctx: &mut Context, x: i32, y: i32, z: i32, density: f64) -> Option<BlockStateId> {
        let air = self.blocks.air;
        if density > 0.0 {
            self.should_schedule_fluid_update = false;
            return None;
        }
        let global = self.picker.compute(x, y, z);
        if y > self.skip_sampling_above_y {
            self.should_schedule_fluid_update = false;
            return Some(global.at(y, air));
        }
        if self.is(global.at(y, air), self.blocks.lava) {
            self.should_schedule_fluid_update = false;
            return Some(self.blocks.lava);
        }
        let (xa, ya, za) = (grid_x(x - 5), grid_y(y + 1), grid_x(z - 5));
        if self.candidates_of != Some((xa, ya, za)) {
            self.candidates_of = Some((xa, ya, za));
            let mut n = 0;
            for x1 in 0..=1 {
                for y1 in -1..=1 {
                    for z1 in 0..=1 {
                        let (sx, sy, sz) = (xa + x1, ya + y1, za + z1);
                        let index = self.index(sx, sy, sz);
                        let location = match self.location_cache[index] {
                            Some(l) => l,
                            None => {
                                let mut r = self.random.at(sx, sy, sz);
                                let l = (from_grid_x(sx, r.next_i32_bound(10)), from_grid_y(sy, r.next_i32_bound(9)), from_grid_x(sz, r.next_i32_bound(10)));
                                self.location_cache[index] = Some(l);
                                l
                            }
                        };
                        self.candidates[n] = (index, location);
                        n += 1;
                    }
                }
            }
        }
        // Nearest first, a later candidate first on equal distance (how
        // inserting each with `>=` orders them): the smallest keys of
        // distance << 4 | (15 - candidate). The third and fourth are only
        // needed past the first two's early exit.
        let mut keys = [0u64; 12];
        for (i, &(_, location)) in self.candidates.iter().enumerate() {
            let (dx, dy, dz) = (location.0 - x, location.1 - y, location.2 - z);
            let nd = dx * dx + dy * dy + dz * dz;
            keys[i] = (nd as u64) << 4 | (15 - i) as u64;
        }
        let indices = self.candidates.map(|(index, _)| index);
        let take_nearest = |keys: &mut [u64; 12]| -> (i32, usize) {
            let (mut best, mut at) = (u64::MAX, 0);
            for (i, &key) in keys.iter().enumerate() {
                if key < best {
                    best = key;
                    at = i;
                }
            }
            keys[at] = u64::MAX;
            ((best >> 4) as i32, indices[at])
        };
        let mut d = [0i32; 4];
        let mut closest = [0usize; 4];
        (d[0], closest[0]) = take_nearest(&mut keys);
        (d[1], closest[1]) = take_nearest(&mut keys);
        let flowing = similarity(100, 144);
        let status1 = self.status(ctx, closest[0]);
        let similarity12 = similarity(d[0], d[1]);
        let fluid_state = status1.at(y, air);
        if similarity12 <= 0.0 {
            self.should_schedule_fluid_update = if similarity12 >= flowing { status1 != self.status(ctx, closest[1]) } else { false };
            return Some(fluid_state);
        }
        (d[2], closest[2]) = take_nearest(&mut keys);
        (d[3], closest[3]) = take_nearest(&mut keys);
        if self.is(fluid_state, self.blocks.water) && self.is(self.picker.compute(x, y - 1, z).at(y - 1, air), self.blocks.lava) {
            self.should_schedule_fluid_update = true;
            return Some(fluid_state);
        }
        let mut barrier_value = f64::NAN;
        let status2 = self.status(ctx, closest[1]);
        let barrier12 = similarity12 * self.pressure(ctx, x, y, z, &mut barrier_value, status1, status2);
        if density + barrier12 > 0.0 {
            self.should_schedule_fluid_update = false;
            return None;
        }
        let status3 = self.status(ctx, closest[2]);
        let similarity13 = similarity(d[0], d[2]);
        if similarity13 > 0.0 {
            let barrier13 = similarity12 * similarity13 * self.pressure(ctx, x, y, z, &mut barrier_value, status1, status3);
            if density + barrier13 > 0.0 {
                self.should_schedule_fluid_update = false;
                return None;
            }
        }
        let similarity23 = similarity(d[1], d[2]);
        if similarity23 > 0.0 {
            let barrier23 = similarity12 * similarity23 * self.pressure(ctx, x, y, z, &mut barrier_value, status2, status3);
            if density + barrier23 > 0.0 {
                self.should_schedule_fluid_update = false;
                return None;
            }
        }
        let may_flow12 = status1 != status2;
        let may_flow23 = similarity23 >= flowing && status2 != status3;
        let may_flow13 = similarity13 >= flowing && status1 != status3;
        self.should_schedule_fluid_update = if may_flow12 || may_flow23 || may_flow13 {
            true
        } else {
            similarity13 >= flowing && similarity(d[0], d[3]) >= flowing && status1 != self.status(ctx, closest[3])
        };
        Some(fluid_state)
    }

    fn pressure(&mut self, ctx: &mut Context, x: i32, y: i32, z: i32, barrier_value: &mut f64, s1: FluidStatus, s2: FluidStatus) -> f64 {
        let air = self.blocks.air;
        let (t1, t2) = (s1.at(y, air), s2.at(y, air));
        let (w, l) = (self.blocks.water, self.blocks.lava);
        if (t1 == l && t2 == w) || (t1 == w && t2 == l) {
            return 2.0;
        }
        let diff = (s1.level - s2.level).abs();
        if diff == 0 {
            return 0.0;
        }
        let average = 0.5 * f64::from(s1.level + s2.level);
        let above = f64::from(y) + 0.5 - average;
        let base = f64::from(diff) / 2.0;
        let toward_middle = base - above.abs();
        let gradient = if above > 0.0 {
            let c = toward_middle;
            if c > 0.0 { c / 1.5 } else { c / 2.5 }
        } else {
            let c = 3.0 + toward_middle;
            if c > 0.0 { c / 3.0 } else { c / 10.0 }
        };
        let noise = if !(-2.0..=2.0).contains(&gradient) {
            0.0
        } else if barrier_value.is_nan() {
            let v = f64::from(ctx.value(self.samplers.barrier, x, y, z));
            *barrier_value = v;
            v
        } else {
            *barrier_value
        };
        2.0 * (noise + gradient)
    }

    fn status(&mut self, ctx: &mut Context, index: usize) -> FluidStatus {
        if let Some(s) = self.status_cache[index] {
            return s;
        }
        let (x, y, z) = self.location_cache[index].expect("location computed before status");
        let s = self.compute_fluid(ctx, x, y, z);
        self.status_cache[index] = Some(s);
        s
    }

    fn compute_fluid(&mut self, ctx: &mut Context, x: i32, y: i32, z: i32) -> FluidStatus {
        let air = self.blocks.air;
        let global = self.picker.compute(x, y, z);
        let mut lowest = i32::MAX;
        let (top, bottom) = (y + 12, y - 12);
        let mut center_under_global = false;
        for [ox, oz] in SURFACE_SAMPLING_OFFSETS {
            let (sx, sz) = (x + (ox << 4), z + (oz << 4));
            let surface = self.surface_level(ctx, sx, sz);
            let adjusted = surface + 8;
            let start = ox == 0 && oz == 0;
            if start && bottom > adjusted {
                return global;
            }
            let pokes_above = top > adjusted;
            if pokes_above || start {
                let at_surface = self.picker.compute(sx, adjusted, sz);
                if at_surface.at(adjusted, air) != air {
                    if start {
                        center_under_global = true;
                    }
                    if pokes_above {
                        return at_surface;
                    }
                }
            }
            lowest = lowest.min(surface);
        }
        let level = self.compute_surface_level(ctx, x, y, z, global, lowest, center_under_global);
        FluidStatus { level, fluid: self.compute_fluid_type(ctx, x, y, z, global, level) }
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_surface_level(&mut self, ctx: &mut Context, x: i32, y: i32, z: i32, global: FluidStatus, lowest: i32, center_under_global: bool) -> i32 {
        let (partially, fully);
        if f64::from(ctx.value(self.samplers.exclusion, x, y, z)) > 0.0 {
            partially = -1.0;
            fully = -1.0;
        } else {
            let below = lowest + 8 - y;
            let factor = if center_under_global { clamped_map(f64::from(below), 0.0, 64.0, 1.0, 0.0) } else { 0.0 };
            let noise = f64::from(ctx.value(self.samplers.floodedness, x, y, z)).clamp(-1.0, 1.0);
            let fully_threshold = map(factor, 1.0, 0.0, -0.3, 0.8);
            let partially_threshold = map(factor, 1.0, 0.0, -0.8, 0.4);
            partially = noise - partially_threshold;
            fully = noise - fully_threshold;
        }
        if fully > 0.0 {
            global.level
        } else if partially > 0.0 {
            self.randomized_surface_level(ctx, x, y, z, lowest)
        } else {
            WAY_BELOW_MIN_Y
        }
    }

    fn randomized_surface_level(&mut self, ctx: &mut Context, x: i32, y: i32, z: i32, lowest: i32) -> i32 {
        let (cx, cy, cz) = (mth::floor_div(x, 16), mth::floor_div(y, 40), mth::floor_div(z, 16));
        let middle = cy * 40 + 20;
        let spread = f64::from(ctx.value(self.samplers.spread, cx, cy, cz) * 10.0);
        let quantized = mth::floor(spread / 3.0) * 3;
        lowest.min(middle + quantized)
    }

    fn compute_fluid_type(&mut self, ctx: &mut Context, x: i32, y: i32, z: i32, global: FluidStatus, level: i32) -> BlockStateId {
        if level <= -10 && level != WAY_BELOW_MIN_Y && global.fluid != self.blocks.lava {
            let v = ctx.value(self.samplers.lava, mth::floor_div(x, 64), mth::floor_div(y, 40), mth::floor_div(z, 64));
            if f64::from(v).abs() > 0.3 {
                return self.blocks.lava;
            }
        }
        global.fluid
    }
}

fn map(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    mth::lerp_f64((value - from_min) / (from_max - from_min), to_min, to_max)
}

fn clamped_map(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    let t = (value - from_min) / (from_max - from_min);
    if t < 0.0 {
        to_min
    } else if t > 1.0 {
        to_max
    } else {
        mth::lerp_f64(t, to_min, to_max)
    }
}
