//! Gradient noise, noise stacks and `NormalNoise` as in 26.3 `levelgen.synth`.
//!
//! Every noise has two evaluation paths, as in vanilla: `get` for single
//! points and `add_to_volume` for batched volumes. They scale coordinates in
//! a different order (`(x * xzScale) * frequency` versus
//! `x * (xzScale * frequency)`), so results can differ in the last bit and
//! callers must use the path vanilla uses.

use crate::interval::Interval;
use crate::mth;
use minecraftoss_core::random::{AnyRandom, RandomSource};
use serde_json::Value;

/// A regular grid of block positions, sampled Y-fastest (vanilla `DensityVolume`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Volume {
    pub size: [i32; 3],
    pub min: [i32; 3],
    pub step: [i32; 3],
}

impl Volume {
    pub fn new(size: [i32; 3], min: [i32; 3], step: [i32; 3]) -> Self {
        assert!(size.iter().all(|&s| s > 0), "volume size must be positive: {size:?}");
        assert!(step.iter().all(|&s| s > 0), "volume step must be positive: {step:?}");
        Self { size, min, step }
    }

    pub fn blocks(size: [i32; 3], min: [i32; 3]) -> Self {
        Self::new(size, min, [1, 1, 1])
    }

    pub fn len(&self) -> usize {
        (self.size[0] * self.size[1] * self.size[2]) as usize
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// `DensityVolume.indexUnchecked`: Y fastest, then X, then Z.
    pub fn index(&self, x: i32, y: i32, z: i32) -> usize {
        (y + (x + z * self.size[0]) * self.size[1]) as usize
    }

    pub fn block_x(&self, x: i32) -> i32 {
        self.min[0] + x * self.step[0]
    }

    pub fn block_y(&self, y: i32) -> i32 {
        self.min[1] + y * self.step[1]
    }

    pub fn block_z(&self, z: i32) -> i32 {
        self.min[2] + z * self.step[2]
    }

    pub fn max_block(&self, axis: usize) -> i32 {
        self.min[axis] + self.size[axis] * self.step[axis] - 1
    }

    /// `DensityVolume.indexOfBlock`, or `None` if the block is not a sample point.
    pub fn index_of_block(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let rel = [x - self.min[0], y - self.min[1], z - self.min[2]];
        if self.step == [1, 1, 1] {
            if rel.iter().zip(self.size).all(|(&r, s)| r >= 0 && r < s) {
                return Some(self.index(rel[0], rel[1], rel[2]));
            }
            return None;
        }
        let inside = (0..3).all(|a| rel[a] >= 0 && rel[a] < self.size[a] * self.step[a] && mth::floor_mod(rel[a], self.step[a]) == 0);
        inside.then(|| {
            self.index(mth::floor_div(rel[0], self.step[0]), mth::floor_div(rel[1], self.step[1]), mth::floor_div(rel[2], self.step[2]))
        })
    }
}

/// `GradientNoise.GRADIENT`.
const GRADIENT: [[i32; 3]; 16] = [
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
    [1, 1, 0],
    [0, -1, 1],
    [-1, 1, 0],
    [0, -1, -1],
];

/// `Math.nextDown(1.6777216E7)`.
const HALF_ROUND_OFF: f64 = 16_777_215.999_999_998;

/// `GradientNoise.wrap`: keeps coordinates inside double precision range.
fn wrap(x: f64) -> f64 {
    if (-HALF_ROUND_OFF..HALF_ROUND_OFF).contains(&x) {
        x
    } else {
        x - (x / 33_554_432.0 + 0.5).floor() * 33_554_432.0
    }
}

fn grad_dot(hash: i32, x: f32, y: f32, z: f32) -> f32 {
    let g = GRADIENT[(hash & 0xf) as usize];
    g[0] as f32 * x + g[1] as f32 * y + g[2] as f32 * z
}

fn grad_dot_xz(g: [i32; 3], x: f32, z: f32) -> f32 {
    g[0] as f32 * x + g[2] as f32 * z
}

/// `PerlinNoise`, or `SmearedPerlinNoise` when `fudge_y_scale` is set.
#[derive(Clone, Debug)]
pub struct Perlin {
    perms: [u8; 256],
    offset: [f64; 3],
    fudge_y_scale: Option<f64>,
}

impl Perlin {
    /// `GradientNoise(RandomSource)`: three offsets then a Fisher-Yates shuffle.
    pub fn new(random: &mut impl RandomSource) -> Self {
        let offset = [random.next_f64() * 256.0, random.next_f64() * 256.0, random.next_f64() * 256.0];
        let mut perms = [0u8; 256];
        for (i, p) in perms.iter_mut().enumerate() {
            *p = i as u8;
        }
        for i in 0..256 {
            let j = random.next_i32_bound(256 - i as i32) as usize + i;
            perms.swap(i, j);
        }
        Self { perms, offset, fudge_y_scale: None }
    }

    /// `SmearedPerlinNoise(RandomSource, double)`.
    pub fn smeared(random: &mut impl RandomSource, fudge_y_scale: f64) -> Self {
        Self { fudge_y_scale: Some(fudge_y_scale), ..Self::new(random) }
    }

    pub fn range(&self) -> Interval {
        match self.fudge_y_scale {
            None => Interval::symmetric(2.0),
            Some(scale) => Interval::symmetric((scale.abs() + 2.0) as f32),
        }
    }

    fn permute(&self, x: i32) -> i32 {
        i32::from(self.perms[(x & 0xff) as usize])
    }

    fn gradient(&self, x: i32) -> [i32; 3] {
        GRADIENT[(self.permute(x) & 0xf) as usize]
    }

    fn fudge_y(&self, scale: f64, original_y: f64, relative_y: f64) -> f64 {
        let limit = if original_y >= 0.0 && original_y < relative_y { original_y } else { relative_y };
        f64::from(mth::floor(limit / scale + f64::from(1.0e-7f32))) * scale
    }

    #[allow(clippy::too_many_arguments)]
    fn sample_and_lerp(&self, x: i32, y: i32, z: i32, rx: f32, ry: f32, rz: f32, original_ry: f32) -> f32 {
        let x0 = self.permute(x);
        let x1 = self.permute(x + 1);
        let xy00 = self.permute(x0 + y);
        let xy01 = self.permute(x0 + y + 1);
        let xy10 = self.permute(x1 + y);
        let xy11 = self.permute(x1 + y + 1);
        let d000 = grad_dot(self.permute(xy00 + z), rx, ry, rz);
        let d100 = grad_dot(self.permute(xy10 + z), rx - 1.0, ry, rz);
        let d010 = grad_dot(self.permute(xy01 + z), rx, ry - 1.0, rz);
        let d110 = grad_dot(self.permute(xy11 + z), rx - 1.0, ry - 1.0, rz);
        let d001 = grad_dot(self.permute(xy00 + z + 1), rx, ry, rz - 1.0);
        let d101 = grad_dot(self.permute(xy10 + z + 1), rx - 1.0, ry, rz - 1.0);
        let d011 = grad_dot(self.permute(xy01 + z + 1), rx, ry - 1.0, rz - 1.0);
        let d111 = grad_dot(self.permute(xy11 + z + 1), rx - 1.0, ry - 1.0, rz - 1.0);
        mth::lerp3(mth::smoothstep(rx), mth::smoothstep(original_ry), mth::smoothstep(rz), d000, d100, d010, d110, d001, d101, d011, d111)
    }

    /// `PerlinNoise.get(double, double, double)` / `SmearedPerlinNoise.get`.
    pub fn get(&self, x_in: f64, y_in: f64, z_in: f64) -> f32 {
        let x = wrap(x_in) + self.offset[0];
        let y = wrap(y_in) + self.offset[1];
        let z = wrap(z_in) + self.offset[2];
        let (fx, fy, fz) = (mth::floor(x), mth::floor(y), mth::floor(z));
        let rx = (x - f64::from(fx)) as f32;
        let rz = (z - f64::from(fz)) as f32;
        match self.fudge_y_scale {
            None => {
                let ry = (y - f64::from(fy)) as f32;
                self.sample_and_lerp(fx, fy, fz, rx, ry, rz, ry)
            }
            Some(scale) => {
                let ry = y - f64::from(fy);
                let fudged = (ry - self.fudge_y(scale, y_in, ry)) as f32;
                self.sample_and_lerp(fx, fy, fz, rx, fudged, rz, ry as f32)
            }
        }
    }

    /// `PerlinNoise.get(double, double)`.
    pub fn get2(&self, x: f64, y: f64) -> f32 {
        self.get(wrap(x), 0.0, wrap(y))
    }

    /// `addToVolume`: adds `amplitude * noise` to each sample of `volume`.
    pub fn add_to_volume(&self, buffer: &mut [f32], volume: &Volume, xz_scale: f64, y_scale: f64, amplitude: f32) {
        self.add_to_volume_where(buffer, volume, xz_scale, y_scale, amplitude, None);
    }

    /// `add_to_volume` for the samples `mask` marks (others may be left
    /// alone); each computed sample is exactly what `add_to_volume` gives.
    pub fn add_to_volume_where(&self, buffer: &mut [f32], volume: &Volume, xz_scale: f64, y_scale: f64, amplitude: f32, mask: Option<&[bool]>) {
        // Everything that depends on Y alone, once per volume rather than
        // per column: the lattice row, the (fudged) offset and its fade.
        let height = volume.size[1] as usize;
        let mut rows_stack = [(0i32, 0.0f32, 0.0f32); 64];
        let mut rows_heap = Vec::new();
        let rows: &mut [(i32, f32, f32)] = if height <= rows_stack.len() {
            &mut rows_stack[..height]
        } else {
            rows_heap.resize(height, (0, 0.0, 0.0));
            &mut rows_heap
        };
        for (iy, row) in rows.iter_mut().enumerate() {
            let original_y = f64::from(volume.block_y(iy as i32)) * y_scale;
            let y = wrap(original_y) + self.offset[1];
            let fy = mth::floor(y);
            let ry_d = y - f64::from(fy);
            let ay = mth::smoothstep(ry_d as f32);
            let ry = match self.fudge_y_scale {
                None => ry_d as f32,
                Some(scale) => (ry_d - self.fudge_y(scale, original_y, ry_d)) as f32,
            };
            *row = (fy, ry, ay);
        }
        let rows = &*rows;
        let mut dxz = [0.0f32; 8];
        let mut gy = [0.0f32; 8];
        let mut index = 0;
        for iz in 0..volume.size[2] {
            let z = wrap(f64::from(volume.block_z(iz)) * xz_scale) + self.offset[2];
            let fz = mth::floor(z);
            let rz = (z - f64::from(fz)) as f32;
            let az = mth::smoothstep(rz);
            for ix in 0..volume.size[0] {
                let x = wrap(f64::from(volume.block_x(ix)) * xz_scale) + self.offset[0];
                let fx = mth::floor(x);
                let rx = (x - f64::from(fx)) as f32;
                let x0 = self.permute(fx);
                let x1 = self.permute(fx + 1);
                let ax = mth::smoothstep(rx);
                let column = &mut buffer[index..index + height];
                let wanted = mask.map(|m| &m[index..index + height]);
                index += height;
                // Runs of rows in the same lattice cell share their corners.
                let mut start = 0;
                while start < height {
                    let fy = rows[start].0;
                    let mut end = start + 1;
                    while end < height && rows[end].0 == fy {
                        end += 1;
                    }
                    if wanted.is_some_and(|w| !w[start..end].contains(&true)) {
                        start = end;
                        continue;
                    }
                    let xy00 = self.permute(x0 + fy);
                    let xy01 = self.permute(x0 + fy + 1);
                    let xy10 = self.permute(x1 + fy);
                    let xy11 = self.permute(x1 + fy + 1);
                    // Corner order 000, 100, 010, 110, 001, 101, 011, 111.
                    let corners = [
                        (xy00 + fz, rx, rz),
                        (xy10 + fz, rx - 1.0, rz),
                        (xy01 + fz, rx, rz),
                        (xy11 + fz, rx - 1.0, rz),
                        (xy00 + fz + 1, rx, rz - 1.0),
                        (xy10 + fz + 1, rx - 1.0, rz - 1.0),
                        (xy01 + fz + 1, rx, rz - 1.0),
                        (xy11 + fz + 1, rx - 1.0, rz - 1.0),
                    ];
                    for (i, (hash, cx, cz)) in corners.into_iter().enumerate() {
                        let g = self.gradient(hash);
                        dxz[i] = grad_dot_xz(g, cx, cz);
                        gy[i] = g[1] as f32;
                    }
                    for (out, &(_, ry, ay)) in column[start..end].iter_mut().zip(&rows[start..end]) {
                        let value = mth::lerp3(
                            ax,
                            ay,
                            az,
                            dxz[0] + gy[0] * ry,
                            dxz[1] + gy[1] * ry,
                            dxz[2] + gy[2] * (ry - 1.0),
                            dxz[3] + gy[3] * (ry - 1.0),
                            dxz[4] + gy[4] * ry,
                            dxz[5] + gy[5] * ry,
                            dxz[6] + gy[6] * (ry - 1.0),
                            dxz[7] + gy[7] * (ry - 1.0),
                        );
                        *out += amplitude * value;
                    }
                    start = end;
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Layer {
    noise: Perlin,
    frequency: f64,
    amplitude: f32,
}

/// A weighted sum of Perlin layers (`NoiseStack`).
#[derive(Clone, Debug, Default)]
pub struct NoiseStack {
    layers: Vec<Layer>,
}

impl NoiseStack {
    pub fn add(&mut self, noise: Perlin, frequency: f64, amplitude: f32) {
        self.layers.push(Layer { noise, frequency, amplitude });
    }

    /// `NoiseStack.Builder.addStack`.
    pub fn add_stack(&mut self, stack: NoiseStack, frequency: f64, amplitude: f32) {
        for layer in stack.layers {
            self.add(layer.noise, layer.frequency * frequency, layer.amplitude * amplitude);
        }
    }

    pub fn range(&self) -> Interval {
        self.layers.iter().fold(Interval::exact(0.0), |range, layer| {
            Interval::add(range, Interval::mul(layer.noise.range(), Interval::exact(layer.amplitude)))
        })
    }

    pub fn get(&self, x: f64, y: f64, z: f64) -> f32 {
        let mut value = 0.0f32;
        for layer in &self.layers {
            let f = layer.frequency;
            value += layer.amplitude * layer.noise.get(x * f, y * f, z * f);
        }
        value
    }

    pub fn get2(&self, x: f64, y: f64) -> f32 {
        let mut value = 0.0f32;
        for layer in &self.layers {
            let f = layer.frequency;
            value += layer.amplitude * layer.noise.get2(x * f, y * f);
        }
        value
    }

    pub fn add_to_volume(&self, buffer: &mut [f32], volume: &Volume, xz_scale: f64, y_scale: f64, amplitude: f32) {
        self.add_to_volume_where(buffer, volume, xz_scale, y_scale, amplitude, None);
    }

    /// `add_to_volume` for the samples `mask` marks.
    pub fn add_to_volume_where(&self, buffer: &mut [f32], volume: &Volume, xz_scale: f64, y_scale: f64, amplitude: f32, mask: Option<&[bool]>) {
        for layer in &self.layers {
            let f = layer.frequency;
            layer.noise.add_to_volume_where(buffer, volume, xz_scale * f, y_scale * f, amplitude * layer.amplitude, mask);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Normalization {
    Disabled,
    Enabled,
    Legacy,
}

/// `NormalNoise.Parameters` from a data-pack `worldgen/noise` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalNoiseParameters {
    pub base_amplitude: f64,
    pub base_octave: i32,
    pub octave_count: i32,
    pub normalize: Normalization,
    pub amplitude_modifiers: Vec<f64>,
}

impl NormalNoiseParameters {
    pub fn from_json(json: &Value) -> Result<Self, String> {
        let base_octave = json["base_octave"].as_i64().ok_or("noise lacks base_octave")? as i32;
        let octave_count = json.get("octave_count").map_or(Some(1), Value::as_i64).ok_or("octave_count must be an integer")? as i32;
        let normalize = match json.get("normalize") {
            None | Some(Value::Bool(true)) => Normalization::Enabled,
            Some(Value::Bool(false)) => Normalization::Disabled,
            Some(Value::String(s)) if s == "legacy" => Normalization::Legacy,
            Some(other) => return Err(format!("invalid normalize value {other}")),
        };
        let amplitude_modifiers = match json.get("amplitude_modifiers") {
            None => Vec::new(),
            Some(v) => v.as_array().ok_or("amplitude_modifiers must be an array")?.iter().map(|a| a.as_f64().ok_or("amplitude must be numeric")).collect::<Result<_, _>>()?,
        };
        if !(-32..=32).contains(&base_octave) || !(1..=32).contains(&octave_count) {
            return Err("noise octave settings out of range".into());
        }
        if !amplitude_modifiers.is_empty() && amplitude_modifiers.len() != octave_count as usize {
            return Err(format!("amplitude_modifiers had size {}, but octave_count was {octave_count}", amplitude_modifiers.len()));
        }
        Ok(Self { base_amplitude: json.get("base_amplitude").and_then(Value::as_f64).unwrap_or(1.0), base_octave, octave_count, normalize, amplitude_modifiers })
    }

    fn modifier(modifiers: &[f64], index: i32) -> f64 {
        if modifiers.is_empty() { 1.0 } else { modifiers[index as usize] }
    }
}

struct Octave {
    index: i32,
    frequency: f64,
    amplitude: f64,
}

/// A configured `NormalNoise`; `create` instantiates it for one random source.
#[derive(Clone, Debug)]
pub struct NormalNoise {
    octaves: Vec<(i32, f64, f64)>,
    normalization_factor: f64,
    target_amplitude: f64,
    parameters: NormalNoiseParameters,
}

fn build_octaves(base_octave: i32, base_amplitude: f64, count: i32, normalize: bool, modifiers: &[f64]) -> Vec<Octave> {
    let mut frequency = 2f64.powi(base_octave);
    let mut amplitude = base_amplitude;
    if normalize {
        amplitude *= 0.5f64.powi(-(count - 1)) / (0.5f64.powi(-count) - 1.0);
    }
    let mut octaves = Vec::new();
    for i in 0..count {
        let modifier = NormalNoiseParameters::modifier(modifiers, i);
        if modifier != 0.0 {
            octaves.push(Octave { index: base_octave + i, frequency, amplitude: amplitude * modifier });
        }
        frequency *= 2.0;
        amplitude *= 0.5;
    }
    octaves
}

fn normalization_factor(target_amplitude: f64, octaves: &[Octave]) -> f64 {
    let variance: f64 = octaves.iter().map(|o| {
        let deviation = 0.270_224_783_124_521_1 * o.amplitude.abs();
        deviation * deviation
    }).fold(0.0, |a, b| a + b);
    let deviation = variance.sqrt();
    if deviation == 0.0 {
        return 0.0;
    }
    target_amplitude * 0.333_333_333_333_333_3 / (deviation * 2f64.sqrt())
}

fn parity_normalization_factor(base_amplitude: f64, count: i32, modifiers: &[f64]) -> f64 {
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for i in 0..count {
        if NormalNoiseParameters::modifier(modifiers, i) != 0.0 {
            lo = lo.min(i);
            hi = hi.max(i);
        }
    }
    let span = hi.wrapping_sub(lo);
    base_amplitude * 0.5 * 0.333_333_333_333_333_3 / (0.1 * (1.0 + 1.0 / f64::from(span + 1)))
}

impl NormalNoise {
    /// `NormalNoise.createParity(firstOctave, amplitudes)`: a base amplitude
    /// that keeps the pre-1.18 normalization.
    pub fn create_parity(first_octave: i32, amplitudes: &[f64]) -> Self {
        let count = amplitudes.len() as i32;
        let octaves = build_octaves(first_octave, 1.0, count, true, amplitudes);
        let target: f64 = octaves.iter().map(|o| o.amplitude.abs()).fold(0.0, |a, b| a + b);
        let new = normalization_factor(target, &octaves);
        let base_amplitude = if new == 0.0 { 1.0 } else { parity_normalization_factor(1.0, count, amplitudes) / new };
        let amplitude_modifiers = if amplitudes.iter().any(|&a| a != 1.0) { amplitudes.to_vec() } else { Vec::new() };
        Self::new(NormalNoiseParameters { base_amplitude, base_octave: first_octave, octave_count: count, normalize: Normalization::Enabled, amplitude_modifiers })
    }

    pub fn new(parameters: NormalNoiseParameters) -> Self {
        let p = &parameters;
        let octaves = build_octaves(p.base_octave, p.base_amplitude, p.octave_count, p.normalize != Normalization::Disabled, &p.amplitude_modifiers);
        let mut target: f64 = octaves.iter().map(|o| o.amplitude.abs()).fold(0.0, |a, b| a + b);
        let mut factor = normalization_factor(target, &octaves);
        if p.normalize == Normalization::Legacy && factor != 0.0 {
            let parity = parity_normalization_factor(p.base_amplitude, p.octave_count, &p.amplitude_modifiers);
            target *= parity / factor;
            factor = parity;
        }
        Self { octaves: octaves.iter().map(|o| (o.index, o.frequency, o.amplitude)).collect(), normalization_factor: factor, target_amplitude: target, parameters }
    }

    /// `NormalNoise.range()`.
    pub fn range(&self) -> Interval {
        Interval::symmetric((self.target_amplitude * 0.333_333_333_333_333_3 * 6.0) as f32)
    }

    /// `NormalNoise.create`: two positional forks, two Perlin layers per octave.
    pub fn create(&self, random: &mut AnyRandom) -> NoiseStack {
        let first = random.fork_positional();
        let second = random.fork_positional();
        let mut stack = NoiseStack::default();
        for &(index, frequency, amplitude) in &self.octaves {
            let seed = format!("octave_{index}");
            let first_noise = Perlin::new(&mut first.from_hash_of(&seed));
            let second_noise = Perlin::new(&mut second.from_hash_of(&seed));
            let value_factor = (self.normalization_factor * amplitude) as f32;
            stack.add(first_noise, frequency, value_factor);
            stack.add(second_noise, frequency * 1.018_126_888_217_522_7, value_factor);
        }
        stack
    }

    /// `NormalNoise.createForLegacyNetherBiome`.
    pub fn create_legacy_nether(&self, random: &mut AnyRandom) -> NoiseStack {
        let p = &self.parameters;
        let modifiers = if p.amplitude_modifiers.is_empty() { vec![1.0; p.octave_count as usize] } else { p.amplitude_modifiers.clone() };
        let first = legacy_fbm(random, p.base_octave, &modifiers);
        let second = legacy_fbm(random, p.base_octave, &modifiers);
        let value_factor = (self.normalization_factor * p.base_amplitude) as f32;
        let mut stack = NoiseStack::default();
        stack.add_stack(first, 1.0, value_factor);
        stack.add_stack(second, 1.018_126_888_217_522_7, value_factor);
        stack
    }
}

/// `LegacyFbmInitializer.createForLegacyNetherBiome`.
fn legacy_fbm(random: &mut AnyRandom, first_octave: i32, amplitudes: &[f64]) -> NoiseStack {
    let octaves = amplitudes.len() as i32;
    let zero_index = -first_octave;
    let mut levels: Vec<Option<Perlin>> = vec![None; amplitudes.len()];
    let zero = Perlin::new(random);
    if zero_index >= 0 && zero_index < octaves && amplitudes[zero_index as usize] != 0.0 {
        levels[zero_index as usize] = Some(zero);
    }
    for i in (0..zero_index).rev() {
        if i < octaves && amplitudes[i as usize] != 0.0 {
            levels[i as usize] = Some(Perlin::new(random));
        } else {
            random.consume_count(262);
        }
    }
    assert!(zero_index >= octaves - 1, "positive octaves are temporarily disabled");
    let mut factor = 2f64.powi(-zero_index);
    let mut value_factor = 2f64.powi(octaves - 1) / (2f64.powi(octaves) - 1.0);
    let mut stack = NoiseStack::default();
    for (i, level) in levels.into_iter().enumerate() {
        if let Some(noise) = level {
            stack.add(noise, factor, (value_factor * amplitudes[i]) as f32);
        }
        factor *= 2.0;
        value_factor /= 2.0;
    }
    stack
}

/// `BlendedNoise.createFbm`: smeared Perlin octaves from finest to coarsest.
pub fn blended_fbm(random: &mut AnyRandom, first_octave: i32, smear_scale_y: f64, value_factor: f64) -> NoiseStack {
    assert!(first_octave <= 0, "firstOctave>0");
    let octaves = -first_octave + 1;
    let mut factor = 1.0;
    let mut value_factor = value_factor / (2f64.powi(octaves) - 1.0);
    let mut stack = NoiseStack::default();
    for _ in 0..octaves {
        stack.add(Perlin::smeared(random, smear_scale_y * factor), factor, value_factor as f32);
        factor /= 2.0;
        value_factor *= 2.0;
    }
    stack
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_indexing_matches_vanilla() {
        let v = Volume::new([2, 3, 2], [-8, -64, 16], [4, 8, 4]);
        assert_eq!(v.len(), 12);
        assert_eq!(v.index(1, 2, 1), 2 + (1 + 2) * 3);
        assert_eq!(v.index_of_block(-4, -48, 20), Some(v.index(1, 2, 1)));
        assert_eq!(v.index_of_block(-5, -48, 20), None);
        assert_eq!(v.max_block(1), -64 + 24 - 1);
    }

    #[test]
    fn volume_and_point_paths_agree_on_unit_scales() {
        // With xz/y scale 1 both paths compute identical coordinates.
        let mut random = AnyRandom::new(false, 42);
        let noise = NormalNoise::new(NormalNoiseParameters::from_json(&serde_json::json!({"base_octave": -3, "base_amplitude": 1.0})).unwrap()).create(&mut random);
        let volume = Volume::blocks([3, 5, 2], [-7, 60, 100]);
        let mut buffer = vec![0.0; volume.len()];
        noise.add_to_volume(&mut buffer, &volume, 1.0, 1.0, 1.0);
        for z in 0..2 {
            for x in 0..3 {
                for y in 0..5 {
                    let point = noise.get(f64::from(volume.block_x(x)), f64::from(volume.block_y(y)), f64::from(volume.block_z(z)));
                    assert_eq!(point.to_bits(), buffer[volume.index(x, y, z)].to_bits());
                }
            }
        }
    }
}
