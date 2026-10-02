//! Density evaluation (vanilla `SamplerContext` and each sampler's
//! `sampleValue` / `sampleVolume`).
//!
//! Both paths are kept exactly as vanilla has them. They can differ in the
//! last bit, and caches filled by one path serve reads from the other, so the
//! order and path of every evaluation matter, including lazily sampled spline
//! coordinates.

use super::ir::{Axis, Round, leaky_relu, round_to_integer, squeeze};
use super::sampler::{CSpline, ContextField, Id, Program, Sampler};
use crate::mth;
use crate::noise::Volume;

mod fused;

#[derive(Default)]
struct CacheCell {
    volume: Option<Volume>,
    buffer: Vec<f32>,
    value_key: i64,
    value: f32,
}

/// `BlockPos.asLong`.
fn block_key(x: i32, y: i32, z: i32) -> i64 {
    ((i64::from(x) & 0x3ff_ffff) << 38) | (i64::from(y) & 0xfff) | ((i64::from(z) & 0x3ff_ffff) << 12)
}

fn java_pow(base: f32, exponent: f32) -> f32 {
    f64::from(base).powf(f64::from(exponent)) as f32
}

/// Per-chunk evaluation state: cache cells and scratch buffers.
pub struct Context<'p> {
    program: &'p Program,
    caches: Option<Vec<CacheCell>>,
    pool: Vec<Vec<f32>>,
    /// Set while `sample_columns` runs its columns.
    fused: Option<fused::Fused>,
    /// The chunk's `Beardifier`, when structures adapt its terrain.
    pub beardifier: Option<std::sync::Arc<crate::structure::beardifier::Beardifier>>,
}

enum SplineInput {
    Point { x: i32, y: i32, z: i32, cached: Vec<f32> },
    Buffer { volume: Volume, buffers: Vec<Option<Vec<f32>>>, index: usize },
}

impl<'p> Context<'p> {
    /// A context with caches enabled, as `NoiseChunk` builds.
    pub fn cached(program: &'p Program) -> Self {
        let caches = (0..program.cache_count).map(|_| CacheCell { value: f32::NAN, ..CacheCell::default() }).collect();
        Self { program, caches: Some(caches), pool: Vec::new(), fused: None, beardifier: None }
    }

    /// `SamplerContext.EMPTY_UNCACHED`.
    pub fn uncached(program: &'p Program) -> Self {
        Self { program, caches: None, pool: Vec::new(), fused: None, beardifier: None }
    }

    fn acquire(&mut self, len: usize) -> Vec<f32> {
        let mut buffer = self.pool.pop().unwrap_or_default();
        buffer.clear();
        buffer.resize(len, 0.0);
        buffer
    }

    fn release(&mut self, buffer: Vec<f32>) {
        self.pool.push(buffer);
    }

    /// Samples a whole volume into a new buffer (Y fastest).
    pub fn sample(&mut self, id: Id, volume: &Volume) -> Vec<f32> {
        let mut out = vec![0.0; volume.len()];
        self.volume(id, &mut out, volume);
        out
    }

    /// `DensitySampler.sampleValue`.
    pub fn value(&mut self, id: Id, x: i32, y: i32, z: i32) -> f32 {
        let program = self.program;
        match &program.samplers[id] {
            Sampler::Constant(v) => *v,
            Sampler::Noise { noise, xz_scale, y_scale } => {
                program.noises[*noise].get(f64::from(x) * xz_scale, f64::from(y) * y_scale, f64::from(z) * xz_scale)
            }
            Sampler::ShiftedXz { shift_x, shift_z, noise, xz_scale, y_scale } => {
                let nx = f64::from(x) * xz_scale + f64::from(self.value(*shift_x, x, y, z));
                let ny = f64::from(y) * y_scale;
                let nz = f64::from(z) * xz_scale + f64::from(self.value(*shift_z, x, y, z));
                program.noises[*noise].get(nx, ny, nz)
            }
            Sampler::ShiftedXyz { shift_x, shift_y, shift_z, noise, xz_scale, y_scale } => {
                let nx = f64::from(x) * xz_scale + f64::from(self.value(*shift_x, x, y, z));
                let ny = f64::from(y) * y_scale + f64::from(self.value(*shift_y, x, y, z));
                let nz = f64::from(z) * xz_scale + f64::from(self.value(*shift_z, x, y, z));
                program.noises[*noise].get(nx, ny, nz)
            }
            Sampler::ShiftB { noise } => program.noises[*noise].get(f64::from(z) * 0.25, f64::from(x) * 0.25, 0.0) * 4.0,
            Sampler::EndIslands { noise } => (super::sampler::end_island_height(&program.simplex[*noise], x / 8, z / 8) - 8.0) / 128.0,
            Sampler::DistanceToPoint { point, metric } => metric.compute((point[0] - x) as f32, (point[1] - y) as f32, (point[2] - z) as f32),
            Sampler::GradientClamped { axis, from, min, max, from_value, factor } => {
                let c = axis.choose(x, y, z).clamp(*min, *max) - from;
                from_value + c as f32 * factor
            }
            Sampler::GradientRepeat { axis, from, range, from_value, factor } => gradient_repeat(axis.choose(x, y, z), *from, *range, *from_value, *factor),
            Sampler::GradientMirrored { axis, from, range, from_value, factor } => gradient_mirrored(axis.choose(x, y, z), *from, *range, *from_value, *factor),
            Sampler::ContextBound { field: ContextField::Beardifier, .. } if self.beardifier.is_some() => {
                self.beardifier.as_ref().expect("checked").value(x, y, z)
            }
            Sampler::ContextBound { fallback, .. } => self.value(*fallback, x, y, z),
            Sampler::Abs(i) => self.value(*i, x, y, z).abs(),
            Sampler::Square(i) => {
                let v = self.value(*i, x, y, z);
                v * v
            }
            Sampler::Cube(i) => {
                let v = self.value(*i, x, y, z);
                v * v * v
            }
            Sampler::Sqrt(i) => (f64::from(self.value(*i, x, y, z))).sqrt() as f32,
            Sampler::LeakyRelu(i, f) => leaky_relu(*f, self.value(*i, x, y, z)),
            Sampler::Reciprocal(i) => 1.0 / self.value(*i, x, y, z),
            Sampler::Negate(i) => -self.value(*i, x, y, z),
            Sampler::Squeeze(i) => squeeze(self.value(*i, x, y, z)),
            Sampler::Log(i) => f64::from(self.value(*i, x, y, z)).ln() as f32,
            Sampler::Sign(i) => mth::signum(self.value(*i, x, y, z)),
            Sampler::RoundInteger(op, i) => round_to_integer(self.value(*i, x, y, z), *op),
            Sampler::Round(op, i, m) => {
                let input = self.value(*i, x, y, z);
                let multiple = self.value(*m, x, y, z);
                round_multiple(input, multiple, *op)
            }
            Sampler::ConstAdd(i, c) => self.value(*i, x, y, z) + c,
            Sampler::Add(l, r) => self.value(*l, x, y, z) + self.value(*r, x, y, z),
            Sampler::ConstSub(c, i) => c - self.value(*i, x, y, z),
            Sampler::Sub(l, r) => self.value(*l, x, y, z) - self.value(*r, x, y, z),
            Sampler::ConstMul(i, c) => self.value(*i, x, y, z) * c,
            Sampler::Mul(l, r) => {
                let a = self.value(*l, x, y, z);
                if a == 0.0 { 0.0 } else { a * self.value(*r, x, y, z) }
            }
            Sampler::ConstDiv(c, i) => c / self.value(*i, x, y, z),
            Sampler::Div(l, r) => {
                let a = self.value(*l, x, y, z);
                if a == 0.0 { 0.0 } else { a / self.value(*r, x, y, z) }
            }
            Sampler::ConstMin(i, c) => mth::min(self.value(*i, x, y, z), *c),
            Sampler::Min(l, r, right_min) => {
                let a = self.value(*l, x, y, z);
                if a <= *right_min { a } else { mth::min(a, self.value(*r, x, y, z)) }
            }
            Sampler::ConstMax(i, c) => mth::max(self.value(*i, x, y, z), *c),
            Sampler::Max(l, r, right_max) => {
                let a = self.value(*l, x, y, z);
                if a >= *right_max { a } else { mth::max(a, self.value(*r, x, y, z)) }
            }
            Sampler::PowConstBase(b, e) => java_pow(*b, self.value(*e, x, y, z)),
            Sampler::PowConstExponent(b, e) => java_pow(self.value(*b, x, y, z), *e),
            Sampler::Pow(b, e) => {
                let base = self.value(*b, x, y, z);
                java_pow(base, self.value(*e, x, y, z))
            }
            Sampler::LerpConstFirst(a, first, s) => {
                let alpha = self.value(*a, x, y, z);
                if alpha == 0.0 {
                    *first
                } else if alpha == 1.0 {
                    self.value(*s, x, y, z)
                } else {
                    mth::lerp(alpha, *first, self.value(*s, x, y, z))
                }
            }
            Sampler::LerpConstSecond(a, f, second) => {
                let alpha = self.value(*a, x, y, z);
                if alpha == 0.0 {
                    self.value(*f, x, y, z)
                } else if alpha == 1.0 {
                    *second
                } else {
                    mth::lerp(alpha, self.value(*f, x, y, z), *second)
                }
            }
            Sampler::Lerp(a, f, s) => {
                let alpha = self.value(*a, x, y, z);
                if alpha == 0.0 {
                    self.value(*f, x, y, z)
                } else if alpha == 1.0 {
                    self.value(*s, x, y, z)
                } else {
                    let first = self.value(*f, x, y, z);
                    mth::lerp(alpha, first, self.value(*s, x, y, z))
                }
            }
            Sampler::Clamp(i, lo, hi) => mth::clamp(self.value(*i, x, y, z), *lo, *hi),
            Sampler::RangeChoiceConst { input, min, max, in_range, out_of_range } => {
                let v = self.value(*input, x, y, z);
                if v >= *min && v < *max { *in_range } else { *out_of_range }
            }
            Sampler::RangeChoice { input, min, max, in_range, out_of_range } => {
                let v = self.value(*input, x, y, z);
                if v >= *min && v < *max { self.value(*in_range, x, y, z) } else { self.value(*out_of_range, x, y, z) }
            }
            Sampler::IntervalSelectSingle { input, threshold, below, above } => {
                if self.value(*input, x, y, z) < *threshold { self.value(*below, x, y, z) } else { self.value(*above, x, y, z) }
            }
            Sampler::IntervalSelect { input, thresholds, samplers } => {
                let v = self.value(*input, x, y, z);
                self.value(samplers[select_index(thresholds, samplers.len(), v)], x, y, z)
            }
            Sampler::Cache { id: cache, input } => self.value_cached(*cache, *input, x, y, z),
            Sampler::BlendDensity(i) => self.value(*i, x, y, z),
            Sampler::Interpolated { input, cell_xz, cell_y, .. } => {
                let (cx, cy) = (*cell_xz, *cell_y);
                let (ix, iy, iz) = (mth::floor_mod(x, cx), mth::floor_mod(y, cy), mth::floor_mod(z, cx));
                if ix == 0 && iy == 0 && iz == 0 {
                    return self.value(*input, x, y, z);
                }
                let v = Volume::new([2, 2, 2], [x - ix, y - iy, z - iz], [cx, cy, cx]);
                let mut b = self.acquire(8);
                self.volume(*input, &mut b, &v);
                let r = mth::lerp3(
                    ix as f32 / cx as f32,
                    iy as f32 / cy as f32,
                    iz as f32 / cx as f32,
                    b[v.index(0, 0, 0)],
                    b[v.index(1, 0, 0)],
                    b[v.index(0, 1, 0)],
                    b[v.index(1, 1, 0)],
                    b[v.index(0, 0, 1)],
                    b[v.index(1, 0, 1)],
                    b[v.index(0, 1, 1)],
                    b[v.index(1, 1, 1)],
                );
                self.release(b);
                r
            }
            Sampler::SliceX { input, x: sx } => self.value(*input, *sx, y, z),
            Sampler::SliceY { input, y: sy } => self.value(*input, x, *sy, z),
            Sampler::SliceZ { input, z: sz } => self.value(*input, x, y, *sz),
            Sampler::SliceXz { input, x: sx, z: sz } => self.value(*input, *sx, y, *sz),
            Sampler::FindTopSurface { density, upper_bound, lower_bound, cell_height } => {
                let upper = self.value(*upper_bound, x, y, z);
                self.find_surface(*density, *lower_bound, *cell_height, x, z, upper)
            }
            Sampler::Spline { spline, coordinate_count } => {
                let mut input = SplineInput::Point { x, y, z, cached: vec![f32::NAN; *coordinate_count] };
                self.spline(spline, &mut input)
            }
        }
    }

    fn find_surface(&mut self, density: Id, lower_bound: i32, cell_height: i32, x: i32, z: i32, upper: f32) -> f32 {
        let top = mth::floor(f64::from(upper / cell_height as f32)) * cell_height;
        if top <= lower_bound {
            return lower_bound as f32;
        }
        let mut probe = top;
        while probe >= lower_bound {
            if self.value(density, x, probe, z) > 0.0 {
                return probe as f32;
            }
            probe -= cell_height;
        }
        lower_bound as f32
    }

    /// `SamplerContext.sampleValueCached`.
    fn value_cached(&mut self, cache: usize, input: Id, x: i32, y: i32, z: i32) -> f32 {
        let Some(caches) = &self.caches else {
            return self.value(input, x, y, z);
        };
        let cell = &caches[cache];
        let key = block_key(x, y, z);
        if cell.value_key == key && !cell.value.is_nan() {
            return cell.value;
        }
        if let Some(index) = cell.volume.as_ref().and_then(|v| v.index_of_block(x, y, z)) {
            return cell.buffer[index];
        }
        let value = self.value(input, x, y, z);
        let cell = &mut self.caches.as_mut().expect("checked above")[cache];
        cell.value_key = key;
        cell.value = value;
        value
    }

    /// `SamplerContext.sampleVolumeCached`.
    fn volume_cached(&mut self, cache: usize, input: Id, out: &mut [f32], volume: &Volume) {
        let Some(caches) = &mut self.caches else {
            return self.volume(input, out, volume);
        };
        if caches[cache].volume.as_ref() != Some(volume) {
            let mut buffer = std::mem::take(&mut caches[cache].buffer);
            buffer.clear();
            buffer.resize(volume.len(), 0.0);
            self.volume(input, &mut buffer, volume);
            let cell = &mut self.caches.as_mut().expect("enabled")[cache];
            cell.buffer = buffer;
            cell.volume = Some(*volume);
        }
        out.copy_from_slice(&self.caches.as_ref().expect("enabled")[cache].buffer);
    }

    fn spline(&mut self, spline: &CSpline, input: &mut SplineInput) -> f32 {
        let CSpline::Multipoint { sampler, index, locations, values, derivatives } = spline else {
            let CSpline::Constant(v) = spline else { unreachable!() };
            return *v;
        };
        let x = self.spline_coordinate(*sampler, *index, input);
        let extend = |x: f32, value: f32, i: usize| if derivatives[i] == 0.0 { value } else { value + derivatives[i] * (x - locations[i]) };
        // Mth.binarySearch(0, n, i -> x < locations[i]) - 1.
        let start = locations.partition_point(|&l| !(x < l)) as isize - 1;
        let last = locations.len() - 1;
        if start < 0 {
            let v = self.spline(&values[0], input);
            return extend(x, v, 0);
        }
        let start = start as usize;
        if start == last {
            let v = self.spline(&values[last], input);
            return extend(x, v, last);
        }
        let (x1, x2) = (locations[start], locations[start + 1]);
        let t = (x - x1) / (x2 - x1);
        let (d1, d2) = (derivatives[start], derivatives[start + 1]);
        let y1 = self.spline(&values[start], input);
        let y2 = self.spline(&values[start + 1], input);
        let a = d1 * (x2 - x1) - (y2 - y1);
        let b = -d2 * (x2 - x1) + (y2 - y1);
        mth::lerp(t, y1, y2) + t * (1.0 - t) * mth::lerp(t, a, b)
    }

    fn spline_coordinate(&mut self, sampler: Id, index: usize, input: &mut SplineInput) -> f32 {
        match input {
            SplineInput::Point { x, y, z, cached } => {
                if !cached[index].is_nan() {
                    return cached[index];
                }
                let v = self.value(sampler, *x, *y, *z);
                cached[index] = v;
                v
            }
            SplineInput::Buffer { volume, buffers, index: i } => {
                if buffers[index].is_none() {
                    let mut b = self.acquire(volume.len());
                    self.volume(sampler, &mut b, volume);
                    buffers[index] = Some(b);
                }
                buffers[index].as_ref().expect("filled above")[*i]
            }
        }
    }

    /// `DensitySampler.sampleVolume`.
    pub fn volume(&mut self, id: Id, out: &mut [f32], v: &Volume) {
        debug_assert_eq!(out.len(), v.len());
        let program = self.program;
        match &program.samplers[id] {
            Sampler::Constant(c) => out.fill(*c),
            Sampler::EndIslands { noise } => {
                for z in 0..v.size[2] {
                    let bz = v.block_z(z);
                    for x in 0..v.size[0] {
                        let bx = v.block_x(x);
                        let value = (super::sampler::end_island_height(&program.simplex[*noise], bx / 8, bz / 8) - 8.0) / 128.0;
                        let start = v.index(x, 0, z);
                        out[start..start + v.size[1] as usize].fill(value);
                    }
                }
            }
            Sampler::DistanceToPoint { point, metric } => {
                let mut index = 0;
                for z in 0..v.size[2] {
                    let bz = v.block_z(z);
                    for x in 0..v.size[0] {
                        let bx = v.block_x(x);
                        for y in 0..v.size[1] {
                            let by = v.block_y(y);
                            out[index] = metric.compute((point[0] - bx) as f32, (point[1] - by) as f32, (point[2] - bz) as f32);
                            index += 1;
                        }
                    }
                }
            }
            Sampler::Noise { noise, xz_scale, y_scale } => {
                out.fill(0.0);
                program.noises[*noise].add_to_volume(out, v, *xz_scale, *y_scale, 1.0);
            }
            Sampler::ShiftedXz { shift_x, shift_z, noise, xz_scale, y_scale } => {
                self.volume(*shift_x, out, v);
                let mut sz = self.acquire(v.len());
                self.volume(*shift_z, &mut sz, v);
                let mut index = 0;
                // Runs of identical coordinates (a column of a 2D noise) repeat the last value.
                let mut last: Option<([u64; 3], f32)> = None;
                for z in 0..v.size[2] {
                    let bz = f64::from(v.block_z(z)) * xz_scale;
                    for x in 0..v.size[0] {
                        let bx = f64::from(v.block_x(x)) * xz_scale;
                        for y in 0..v.size[1] {
                            let nx = bx + f64::from(out[index]);
                            let ny = f64::from(v.block_y(y)) * y_scale;
                            let nz = bz + f64::from(sz[index]);
                            let key = [nx.to_bits(), ny.to_bits(), nz.to_bits()];
                            out[index] = match last {
                                Some((k, value)) if k == key => value,
                                _ => {
                                    let value = program.noises[*noise].get(nx, ny, nz);
                                    last = Some((key, value));
                                    value
                                }
                            };
                            index += 1;
                        }
                    }
                }
                self.release(sz);
            }
            Sampler::ShiftedXyz { shift_x, shift_y, shift_z, noise, xz_scale, y_scale } => {
                self.volume(*shift_x, out, v);
                let mut sy = self.acquire(v.len());
                self.volume(*shift_y, &mut sy, v);
                let mut sz = self.acquire(v.len());
                self.volume(*shift_z, &mut sz, v);
                let mut index = 0;
                let mut last: Option<([u64; 3], f32)> = None;
                for z in 0..v.size[2] {
                    let bz = f64::from(v.block_z(z)) * xz_scale;
                    for x in 0..v.size[0] {
                        let bx = f64::from(v.block_x(x)) * xz_scale;
                        for y in 0..v.size[1] {
                            let nx = bx + f64::from(out[index]);
                            let ny = f64::from(v.block_y(y)) * y_scale + f64::from(sy[index]);
                            let nz = bz + f64::from(sz[index]);
                            let key = [nx.to_bits(), ny.to_bits(), nz.to_bits()];
                            out[index] = match last {
                                Some((k, value)) if k == key => value,
                                _ => {
                                    let value = program.noises[*noise].get(nx, ny, nz);
                                    last = Some((key, value));
                                    value
                                }
                            };
                            index += 1;
                        }
                    }
                }
                self.release(sz);
                self.release(sy);
            }
            Sampler::ShiftB { noise } => {
                let t = Volume::new([v.size[2], v.size[0], 1], [v.min[2], v.min[0], 0], [v.step[2], v.step[0], 1]);
                let mut tb = self.acquire(t.len());
                tb.fill(0.0);
                program.noises[*noise].add_to_volume(&mut tb, &t, 0.25, 0.25, 4.0);
                for z in 0..v.size[2] {
                    for x in 0..v.size[0] {
                        let value = tb[t.index(z, x, 0)];
                        let start = v.index(x, 0, z);
                        out[start..start + v.size[1] as usize].fill(value);
                    }
                }
                self.release(tb);
            }
            Sampler::GradientClamped { axis, from, min, max, from_value, factor } => {
                gradient_volume(*axis, out, v, |c| from_value + (c.clamp(*min, *max) - from) as f32 * factor)
            }
            Sampler::GradientRepeat { axis, from, range, from_value, factor } => gradient_volume(*axis, out, v, |c| gradient_repeat(c, *from, *range, *from_value, *factor)),
            Sampler::GradientMirrored { axis, from, range, from_value, factor } => gradient_volume(*axis, out, v, |c| gradient_mirrored(c, *from, *range, *from_value, *factor)),
            Sampler::ContextBound { field: ContextField::Beardifier, .. } if self.beardifier.is_some() => {
                self.beardifier.as_ref().expect("checked").volume(out, v)
            }
            Sampler::ContextBound { fallback, .. } => self.volume(*fallback, out, v),
            Sampler::Abs(i) => self.map(*i, out, v, f32::abs),
            Sampler::Square(i) => self.map(*i, out, v, |x| x * x),
            Sampler::Cube(i) => self.map(*i, out, v, |x| x * x * x),
            Sampler::Sqrt(i) => self.map(*i, out, v, |x| f64::from(x).sqrt() as f32),
            Sampler::LeakyRelu(i, f) => {
                let f = *f;
                self.map(*i, out, v, |x| leaky_relu(f, x))
            }
            Sampler::Reciprocal(i) => self.map(*i, out, v, |x| 1.0 / x),
            Sampler::Negate(i) => self.map(*i, out, v, |x| -x),
            Sampler::Squeeze(i) => self.map(*i, out, v, squeeze),
            Sampler::Log(i) => self.map(*i, out, v, |x| f64::from(x).ln() as f32),
            Sampler::Sign(i) => self.map(*i, out, v, mth::signum),
            Sampler::RoundInteger(op, i) => {
                let op = *op;
                self.map(*i, out, v, |x| round_to_integer(x, op))
            }
            Sampler::Round(op, i, m) => self.combine(*i, *m, out, v, |a, b| round_multiple(a, b, *op)),
            Sampler::ConstAdd(i, c) => {
                let c = *c;
                self.map(*i, out, v, |x| x + c)
            }
            Sampler::Add(l, r) => self.combine(*l, *r, out, v, |a, b| a + b),
            Sampler::ConstSub(c, i) => {
                let c = *c;
                self.map(*i, out, v, |x| c - x)
            }
            Sampler::Sub(l, r) => self.combine(*l, *r, out, v, |a, b| a + -b),
            Sampler::ConstMul(i, c) => {
                let c = *c;
                self.map(*i, out, v, |x| x * c)
            }
            Sampler::Mul(l, r) => self.combine(*l, *r, out, v, |a, b| a * b),
            Sampler::ConstDiv(c, i) => {
                let c = *c;
                self.map(*i, out, v, |x| c / x)
            }
            Sampler::Div(l, r) => self.combine(*l, *r, out, v, |a, b| a / b),
            Sampler::ConstMin(i, c) => {
                let c = *c;
                self.map(*i, out, v, |x| if c < x { c } else { x })
            }
            Sampler::Min(l, r, _) => self.combine(*l, *r, out, v, |a, b| if b < a { b } else { a }),
            Sampler::ConstMax(i, c) => {
                let c = *c;
                self.map(*i, out, v, |x| if c > x { c } else { x })
            }
            Sampler::Max(l, r, _) => self.combine(*l, *r, out, v, |a, b| if b > a { b } else { a }),
            Sampler::PowConstBase(b, e) => {
                let b = *b;
                self.map(*e, out, v, |x| java_pow(b, x))
            }
            Sampler::PowConstExponent(b, e) => {
                let e = *e;
                self.map(*b, out, v, |x| java_pow(x, e))
            }
            Sampler::Pow(b, e) => self.combine(*b, *e, out, v, java_pow),
            Sampler::LerpConstFirst(a, first, s) => {
                let first = *first;
                self.combine(*a, *s, out, v, |alpha, second| if alpha == 0.0 { first } else if alpha == 1.0 { second } else { mth::lerp(alpha, first, second) })
            }
            Sampler::LerpConstSecond(a, f, second) => {
                let second = *second;
                self.combine(*a, *f, out, v, |alpha, first| if alpha == 0.0 { first } else if alpha == 1.0 { second } else { mth::lerp(alpha, first, second) })
            }
            Sampler::Lerp(a, f, s) => {
                self.volume(*a, out, v);
                let mut fb = self.acquire(v.len());
                let mut sb = self.acquire(v.len());
                if let (Sampler::Noise { noise: fn_, xz_scale: fx, y_scale: fy }, Sampler::Noise { noise: sn, xz_scale: sx, y_scale: sy }) = (&program.samplers[*f], &program.samplers[*s]) {
                    // Old blended noise: the first noise is read only where
                    // alpha is not 1 and the second only where it is not 0.
                    // Noises have no state, so the rest need not be sampled.
                    let first: Vec<bool> = out.iter().map(|&alpha| alpha != 1.0).collect();
                    let second: Vec<bool> = out.iter().map(|&alpha| alpha != 0.0).collect();
                    fb.fill(0.0);
                    program.noises[*fn_].add_to_volume_where(&mut fb, v, *fx, *fy, 1.0, Some(&first));
                    sb.fill(0.0);
                    program.noises[*sn].add_to_volume_where(&mut sb, v, *sx, *sy, 1.0, Some(&second));
                } else {
                    self.volume(*f, &mut fb, v);
                    self.volume(*s, &mut sb, v);
                }
                for i in 0..out.len() {
                    let (alpha, first, second) = (out[i], fb[i], sb[i]);
                    out[i] = if alpha == 0.0 { first } else if alpha == 1.0 { second } else { mth::lerp(alpha, first, second) };
                }
                self.release(sb);
                self.release(fb);
            }
            Sampler::Clamp(i, lo, hi) => {
                let (lo, hi) = (*lo, *hi);
                self.map(*i, out, v, |x| mth::clamp(x, lo, hi))
            }
            Sampler::RangeChoiceConst { input, min, max, in_range, out_of_range } => {
                let (min, max, a, b) = (*min, *max, *in_range, *out_of_range);
                self.map(*input, out, v, |x| if x >= min && x < max { a } else { b })
            }
            Sampler::RangeChoice { input, min, max, in_range, out_of_range } => {
                self.volume(*in_range, out, v);
                let mut ib = self.acquire(v.len());
                self.volume(*input, &mut ib, v);
                let mut ob = self.acquire(v.len());
                self.volume(*out_of_range, &mut ob, v);
                // A select per element, no branches (NaN inputs are out of range).
                let (min, max) = (*min, *max);
                for ((o, &input), &other) in out.iter_mut().zip(ib.iter()).zip(ob.iter()) {
                    *o = if input >= min && input < max { *o } else { other };
                }
                self.release(ob);
                self.release(ib);
            }
            Sampler::IntervalSelectSingle { input, threshold, below, above } => {
                self.volume(*input, out, v);
                let mut bb = self.acquire(v.len());
                self.volume(*below, &mut bb, v);
                let mut ab = self.acquire(v.len());
                self.volume(*above, &mut ab, v);
                let threshold = *threshold;
                for ((o, &below), &above) in out.iter_mut().zip(bb.iter()).zip(ab.iter()) {
                    *o = if *o < threshold { below } else { above };
                }
                self.release(ab);
                self.release(bb);
            }
            Sampler::IntervalSelect { input, thresholds, samplers } => {
                self.volume(*input, out, v);
                let mut buffers = Vec::with_capacity(samplers.len());
                for s in samplers {
                    let mut b = self.acquire(v.len());
                    self.volume(*s, &mut b, v);
                    buffers.push(b);
                }
                for i in 0..out.len() {
                    out[i] = buffers[select_index(thresholds, samplers.len(), out[i])][i];
                }
                for b in buffers {
                    self.release(b);
                }
            }
            Sampler::Cache { id: cache, input } => self.volume_cached(*cache, *input, out, v),
            Sampler::BlendDensity(i) => self.volume(*i, out, v),
            Sampler::Interpolated { input, cell_xz, cell_y, inv_xz, inv_y } => self.interpolated(*input, *cell_xz, *cell_y, *inv_xz, *inv_y, out, v),
            Sampler::SliceX { input, x } => {
                if v.size[0] == 1 && v.min[0] == *x {
                    return self.volume(*input, out, v);
                }
                let iv = Volume::new([1, v.size[1], v.size[2]], [*x, v.min[1], v.min[2]], v.step);
                let mut ib = self.acquire(iv.len());
                self.volume(*input, &mut ib, &iv);
                let mut index = 0;
                for z in 0..v.size[2] {
                    for _ in 0..v.size[0] {
                        for y in 0..v.size[1] {
                            out[index] = ib[iv.index(0, y, z)];
                            index += 1;
                        }
                    }
                }
                self.release(ib);
            }
            Sampler::SliceY { input, y } => {
                if v.size[1] == 1 && v.min[1] == *y {
                    return self.volume(*input, out, v);
                }
                let iv = Volume::new([v.size[0], 1, v.size[2]], [v.min[0], *y, v.min[2]], v.step);
                let mut ib = self.acquire(iv.len());
                self.volume(*input, &mut ib, &iv);
                for z in 0..v.size[2] {
                    for x in 0..v.size[0] {
                        let value = ib[iv.index(x, 0, z)];
                        let start = v.index(x, 0, z);
                        out[start..start + v.size[1] as usize].fill(value);
                    }
                }
                self.release(ib);
            }
            Sampler::SliceZ { input, z } => {
                if v.size[2] == 1 && v.min[2] == *z {
                    return self.volume(*input, out, v);
                }
                let iv = Volume::new([v.size[0], v.size[1], 1], [v.min[0], v.min[1], *z], v.step);
                let mut ib = self.acquire(iv.len());
                self.volume(*input, &mut ib, &iv);
                let mut index = 0;
                for _ in 0..v.size[2] {
                    for x in 0..v.size[0] {
                        for y in 0..v.size[1] {
                            out[index] = ib[iv.index(x, y, 0)];
                            index += 1;
                        }
                    }
                }
                self.release(ib);
            }
            Sampler::SliceXz { input, x, z } => {
                if v.size[0] == 1 && v.size[2] == 1 && v.min[0] == *x && v.min[2] == *z {
                    return self.volume(*input, out, v);
                }
                let iv = Volume::new([1, v.size[1], 1], [*x, v.min[1], *z], v.step);
                let mut ib = self.acquire(iv.len());
                self.volume(*input, &mut ib, &iv);
                // The input column (Y fastest, as here) into every column.
                if !out.is_empty() {
                    for column in out.chunks_exact_mut(v.size[1] as usize) {
                        column.copy_from_slice(&ib);
                    }
                }
                self.release(ib);
            }
            Sampler::FindTopSurface { density, upper_bound, lower_bound, cell_height } => {
                assert_eq!(v.size[1], 1, "cannot sample find_top_surface with sizeY={}", v.size[1]);
                self.volume(*upper_bound, out, v);
                let mut index = 0;
                for z in 0..v.size[2] {
                    let bz = v.block_z(z);
                    for x in 0..v.size[0] {
                        let bx = v.block_x(x);
                        out[index] = self.find_surface(*density, *lower_bound, *cell_height, bx, bz, out[index]);
                        index += 1;
                    }
                }
            }
            Sampler::Spline { spline, coordinate_count } => {
                let mut input = SplineInput::Buffer { volume: *v, buffers: vec![None; *coordinate_count], index: 0 };
                for i in 0..out.len() {
                    if let SplineInput::Buffer { index, .. } = &mut input {
                        *index = i;
                    }
                    out[i] = self.spline(spline, &mut input);
                }
                if let SplineInput::Buffer { buffers, .. } = input {
                    for b in buffers.into_iter().flatten() {
                        self.release(b);
                    }
                }
            }
        }
    }

    fn map(&mut self, input: Id, out: &mut [f32], v: &Volume, f: impl Fn(f32) -> f32) {
        self.volume(input, out, v);
        for x in out.iter_mut() {
            *x = f(*x);
        }
    }

    fn combine(&mut self, left: Id, right: Id, out: &mut [f32], v: &Volume, f: impl Fn(f32, f32) -> f32) {
        self.volume(left, out, v);
        let mut rb = self.acquire(v.len());
        self.volume(right, &mut rb, v);
        for (a, b) in out.iter_mut().zip(&rb) {
            *a = f(*a, *b);
        }
        self.release(rb);
    }

    /// `InterpolatedFunction.Sampler.sampleVolume`.
    #[allow(clippy::too_many_arguments)]
    fn interpolated(&mut self, input: Id, cell_xz: i32, cell_y: i32, inv_xz: f32, inv_y: f32, out: &mut [f32], v: &Volume) {
        if self.fused_interpolated(cell_xz, cell_y, inv_xz, inv_y, out, v) {
            return;
        }
        let aligned = !((v.step[0] != cell_xz && v.size[0] != 1)
            || (v.step[1] != cell_y && v.size[1] != 1)
            || (v.step[2] != cell_xz && v.size[2] != 1)
            || mth::floor_mod(v.min[0], cell_xz) != 0
            || mth::floor_mod(v.min[1], cell_y) != 0
            || mth::floor_mod(v.min[2], cell_xz) != 0);
        if aligned {
            return self.volume(input, out, v);
        }
        if v.step != [1, 1, 1] {
            let bv = Volume::blocks([v.size[0] * v.step[0], v.size[1] * v.step[1], v.size[2] * v.step[2]], v.min);
            let mut bb = self.acquire(bv.len());
            self.interpolated_blocks(input, cell_xz, cell_y, inv_xz, inv_y, &mut bb, &bv);
            for z in 0..v.size[2] {
                for x in 0..v.size[0] {
                    for y in 0..v.size[1] {
                        out[v.index(x, y, z)] = bb[bv.index(x * v.step[0], y * v.step[1], z * v.step[2])];
                    }
                }
            }
            self.release(bb);
            return;
        }
        self.interpolated_blocks(input, cell_xz, cell_y, inv_xz, inv_y, out, v);
    }

    #[allow(clippy::too_many_arguments)]
    fn interpolated_blocks(&mut self, input: Id, cell_xz: i32, cell_y: i32, inv_xz: f32, inv_y: f32, out: &mut [f32], v: &Volume) {
        let min_cell = [mth::floor_div(v.min[0], cell_xz), mth::floor_div(v.min[1], cell_y), mth::floor_div(v.min[2], cell_xz)];
        let max_cell = [mth::floor_div(v.max_block(0), cell_xz), mth::floor_div(v.max_block(1), cell_y), mth::floor_div(v.max_block(2), cell_xz)];
        let count = [max_cell[0] - min_cell[0] + 1, max_cell[1] - min_cell[1] + 1, max_cell[2] - min_cell[2] + 1];
        let cv = fused::cell_volume(v, cell_xz, cell_y);
        let mut cb = self.acquire(cv.len());
        self.volume(input, &mut cb, &cv);
        for cz in 0..count[2] {
            let nz = (cz + 1).min(cv.size[2] - 1);
            for cx in 0..count[0] {
                let nx = (cx + 1).min(cv.size[0] - 1);
                let mut v000 = cb[cv.index(cx, 0, cz)];
                let mut v100 = cb[cv.index(nx, 0, cz)];
                let mut v001 = cb[cv.index(cx, 0, nz)];
                let mut v101 = cb[cv.index(nx, 0, nz)];
                for cy in 0..count[1] {
                    let ny = (cy + 1).min(cv.size[1] - 1);
                    let v010 = cb[cv.index(cx, ny, cz)];
                    let v110 = cb[cv.index(nx, ny, cz)];
                    let v011 = cb[cv.index(cx, ny, nz)];
                    let v111 = cb[cv.index(nx, ny, nz)];
                    // fillCell
                    let ox = cv.block_x(cx) - v.min[0];
                    let oy = cv.block_y(cy) - v.min[1];
                    let oz = cv.block_z(cz) - v.min[2];
                    let (x0, y0, z0) = ((-ox).max(0), (-oy).max(0), (-oz).max(0));
                    let x1 = cell_xz.min(v.size[0] - ox) - 1;
                    let y1 = cell_y.min(v.size[1] - oy) - 1;
                    let z1 = cell_xz.min(v.size[2] - oz) - 1;
                    for z in z0..=z1 {
                        let alpha_z = z as f32 * inv_xz;
                        let v00 = mth::lerp(alpha_z, v000, v001);
                        let v01 = mth::lerp(alpha_z, v010, v011);
                        let v10 = mth::lerp(alpha_z, v100, v101);
                        let v11 = mth::lerp(alpha_z, v110, v111);
                        for x in x0..=x1 {
                            let alpha_x = x as f32 * inv_xz;
                            let low = mth::lerp(alpha_x, v00, v10);
                            let high = mth::lerp(alpha_x, v01, v11);
                            let step = (high - low) * inv_y;
                            let mut value = low + step * y0 as f32;
                            let mut index = v.index(ox + x, oy + y0, oz + z);
                            for _ in y0..=y1 {
                                out[index] = value;
                                index += 1;
                                value += step;
                            }
                        }
                    }
                    v000 = v010;
                    v100 = v110;
                    v001 = v011;
                    v101 = v111;
                }
            }
        }
        self.release(cb);
    }
}

fn select_index(thresholds: &[f32], count: usize, input: f32) -> usize {
    thresholds.iter().position(|&t| input < t).unwrap_or(count - 1)
}

fn round_multiple(input: f32, multiple: f32, op: Round) -> f32 {
    if multiple == 0.0 { input } else { round_to_integer(input / multiple, op) * multiple }
}

fn gradient_repeat(c: i32, from: i32, range: i32, from_value: f32, factor: f32) -> f32 {
    from_value + mth::floor_mod(c - from, range) as f32 * factor
}

fn gradient_mirrored(c: i32, from: i32, range: i32, from_value: f32, factor: f32) -> f32 {
    let relative = c - from;
    let tile = mth::floor_div(relative, range);
    let local = relative - tile * range;
    if tile & 1 == 0 { from_value + local as f32 * factor } else { from_value + (range - local) as f32 * factor }
}

/// `GradientFunction.GradientSampler.sampleVolume`.
fn gradient_volume(axis: Axis, out: &mut [f32], v: &Volume, compute: impl Fn(i32) -> f32) {
    match axis {
        Axis::X => {
            for x in 0..v.size[0] {
                let value = compute(v.block_x(x));
                for z in 0..v.size[2] {
                    let start = v.index(x, 0, z);
                    out[start..start + v.size[1] as usize].fill(value);
                }
            }
        }
        Axis::Y => {
            // One column of values, copied into every column.
            let height = v.size[1] as usize;
            if out.is_empty() {
                return;
            }
            let (first, rest) = out.split_at_mut(height);
            for (y, value) in first.iter_mut().enumerate() {
                *value = compute(v.block_y(y as i32));
            }
            for column in rest.chunks_exact_mut(height) {
                column.copy_from_slice(first);
            }
        }
        Axis::Z => {
            for z in 0..v.size[2] {
                let value = compute(v.block_z(z));
                let start = v.index(0, 0, z);
                out[start..start + (v.size[0] * v.size[1]) as usize].fill(value);
            }
        }
    }
}
