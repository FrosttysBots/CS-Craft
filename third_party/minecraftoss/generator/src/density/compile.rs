//! Density-function optimization and compilation (vanilla
//! `DensityFunctionCompiler`, `DfRewriteRule`, per-type `compileSampler`).
//!
//! Optimization mirrors vanilla: references are inlined, `cache` nodes are
//! deduplicated by their original input and compiled once, then axes a
//! subtree does not depend on are sliced away. Compilation applies the same
//! constant and range specializations vanilla does, because several of them
//! change results (`x - c` becomes `x + (-c)`, `x / c` becomes `x * (1/c)`).

use super::ir::{Axis, Binary, Df, F32, IntervalKey, Registry, Simple, Spline, Tiling, Unary, for_each_coordinate};
use super::sampler::{CSpline, ContextField, Id, Program, Sampler};
use crate::interval::Interval;
use crate::noise::{NormalNoise, blended_fbm};
use minecraftoss_core::Identifier;
use minecraftoss_core::random::{AnyPositional, AnyRandom, LegacyRandom, RandomSource};
use std::collections::HashMap;
use std::sync::Arc;

struct PreparedCache {
    id: usize,
    range: Interval,
    axes: u8,
}

/// Compiles density functions for one world seed (vanilla `RandomState`).
pub struct Compiler {
    pub registry: Registry,
    seed: i64,
    legacy_random: bool,
    random: AnyPositional,
    program: Program,
    noise_instances: HashMap<Identifier, usize>,
    prepared: HashMap<Arc<Df>, PreparedCache>,
    samplers: HashMap<Arc<Df>, Id>,
    next_cache: usize,
}

impl Compiler {
    pub fn new(registry: Registry, seed: i64, legacy_random: bool) -> Self {
        Self {
            registry,
            seed,
            legacy_random,
            random: AnyRandom::new(legacy_random, seed).fork_positional(),
            program: Program::default(),
            noise_instances: HashMap::new(),
            prepared: HashMap::new(),
            samplers: HashMap::new(),
            next_cache: 0,
        }
    }

    pub fn random(&self) -> &AnyPositional {
        &self.random
    }

    pub fn program(&self) -> &Program {
        &self.program
    }

    pub fn into_program(self) -> Program {
        self.program
    }

    /// `DensityFunctionCompiler.getSampler`: optimizes and compiles once per function.
    pub fn sampler(&mut self, function: &Arc<Df>) -> Result<Id, String> {
        if let Some(&id) = self.samplers.get(function) {
            return Ok(id);
        }
        let optimized = self.optimize(function)?;
        let id = self.compile(&optimized)?;
        self.samplers.insert(function.clone(), id);
        Ok(id)
    }

    /// The optimizer rule: dedupe caches and inline references, then slice uniform axes.
    fn optimize(&mut self, function: &Arc<Df>) -> Result<Arc<Df>, String> {
        let deduped = self.dedupe(function)?;
        Ok(self.slice_uniform(&deduped, super::ir::ALL_AXES))
    }

    fn dedupe(&mut self, function: &Arc<Df>) -> Result<Arc<Df>, String> {
        let function = match &**function {
            Df::Reference(name) => self.registry.resolve(name),
            _ => function.clone(),
        };
        if let Df::Cache(input) = &*function {
            return self.reuse_or_prepare_cache(input);
        }
        self.map_children(&function, &mut |this, child| this.dedupe(child))
    }

    fn reuse_or_prepare_cache(&mut self, input: &Arc<Df>) -> Result<Arc<Df>, String> {
        if let Some(p) = self.prepared.get(input) {
            return Ok(Arc::new(Df::Prepared { id: p.id, range: IntervalKey(p.range), axes: p.axes }));
        }
        let id = self.next_cache;
        self.next_cache += 1;
        let optimized = self.optimize(input)?;
        let inner = self.compile(&optimized)?;
        let sampler = self.program.push(Sampler::Cache { id, input: inner });
        self.program.cache_samplers.insert(id, sampler);
        self.program.cache_count = self.program.cache_count.max(id + 1);
        let prepared = PreparedCache { id, range: self.registry.range(&optimized), axes: self.registry.axes(&optimized) };
        let node = Arc::new(Df::Prepared { id, range: IntervalKey(prepared.range), axes: prepared.axes });
        self.prepared.insert(input.clone(), prepared);
        Ok(node)
    }

    /// `DfRewriteRule.SliceUniformAxes`.
    fn slice_uniform(&mut self, function: &Arc<Df>, parent_axes: u8) -> Arc<Df> {
        if matches!(**function, Df::Constant(_) | Df::Gradient { .. }) {
            return function.clone();
        }
        let axes = self.registry.axes(function);
        let rewritten = self
            .map_children(function, &mut |this, child| Ok(this.slice_uniform(child, if parent_axes == axes { parent_axes } else { axes })))
            .expect("slicing cannot fail");
        if parent_axes == axes {
            return rewritten;
        }
        let removed = parent_axes & !axes;
        let mut existing = 0;
        let mut inner = &rewritten;
        while let Df::Slice { axis, input, .. } = &**inner {
            existing |= axis.bit();
            inner = input;
        }
        let filtered = removed & !existing;
        let mut result = rewritten;
        for axis in [Axis::X, Axis::Z, Axis::Y] {
            if filtered & axis.bit() != 0 {
                result = Arc::new(Df::Slice { axis, coordinate: 0, input: result });
            }
        }
        result
    }

    /// `rewriteChildren` for every node type.
    fn map_children(&mut self, function: &Arc<Df>, f: &mut impl FnMut(&mut Self, &Arc<Df>) -> Result<Arc<Df>, String>) -> Result<Arc<Df>, String> {
        let node = match &**function {
            Df::Reference(name) => return f(self, &self.registry.resolve(name)),
            Df::Constant(_) | Df::ShiftA(_) | Df::ShiftB(_) | Df::Shift(_) | Df::Gradient { .. } | Df::OldBlendedNoise { .. } | Df::Simple(_) | Df::EndIslands | Df::DistanceToPoint { .. } | Df::Prepared { .. } => {
                return Ok(function.clone());
            }
            Df::Noise { noise, xz_scale, y_scale, shift } => Df::Noise { noise: noise.clone(), xz_scale: *xz_scale, y_scale: *y_scale, shift: [f(self, &shift[0])?, f(self, &shift[1])?, f(self, &shift[2])?] },
            Df::Unary(op, i) => Df::Unary(*op, f(self, i)?),
            Df::Round(op, i, m) => Df::Round(*op, f(self, i)?, f(self, m)?),
            Df::Binary(op, l, r) => Df::Binary(*op, f(self, l)?, f(self, r)?),
            Df::Pow(b, e) => Df::Pow(f(self, b)?, f(self, e)?),
            Df::Lerp(a, x, y) => Df::Lerp(f(self, a)?, f(self, x)?, f(self, y)?),
            Df::Clamp(i, lo, hi) => Df::Clamp(f(self, i)?, *lo, *hi),
            Df::RangeChoice { input, min_inclusive, max_exclusive, in_range, out_of_range } => Df::RangeChoice {
                input: f(self, input)?,
                min_inclusive: *min_inclusive,
                max_exclusive: *max_exclusive,
                in_range: f(self, in_range)?,
                out_of_range: f(self, out_of_range)?,
            },
            Df::IntervalSelect { input, thresholds, functions } => Df::IntervalSelect {
                input: f(self, input)?,
                thresholds: thresholds.clone(),
                functions: functions.iter().map(|g| f(self, g)).collect::<Result<_, _>>()?,
            },
            Df::Cache(i) => Df::Cache(f(self, i)?),
            Df::BlendDensity(i) => Df::BlendDensity(f(self, i)?),
            Df::Interpolated { input, cell_xz, cell_y } => Df::Interpolated { input: f(self, input)?, cell_xz: *cell_xz, cell_y: *cell_y },
            Df::Slice { axis, coordinate, input } => Df::Slice { axis: *axis, coordinate: *coordinate, input: f(self, input)? },
            Df::FindTopSurface { density, upper_bound, lower_bound, cell_height } => {
                Df::FindTopSurface { density: f(self, density)?, upper_bound: f(self, upper_bound)?, lower_bound: *lower_bound, cell_height: *cell_height }
            }
            Df::Spline(s) => Df::Spline(self.map_spline(s, f)?),
        };
        Ok(Arc::new(node))
    }

    fn map_spline(&mut self, spline: &Spline, f: &mut impl FnMut(&mut Self, &Arc<Df>) -> Result<Arc<Df>, String>) -> Result<Spline, String> {
        Ok(match spline {
            Spline::Constant(v) => Spline::Constant(*v),
            Spline::Multipoint { coordinate, locations, values, derivatives } => Spline::Multipoint {
                coordinate: f(self, coordinate)?,
                locations: locations.clone(),
                values: values.iter().map(|v| self.map_spline(v, f)).collect::<Result<_, _>>()?,
                derivatives: derivatives.clone(),
            },
        })
    }

    /// `RandomState.getOrCreateNoise` / the compile context's `createNoiseSampler`.
    fn noise(&mut self, name: &Identifier) -> Result<usize, String> {
        let parameters = self.registry.noise(name)?;
        let noise = NormalNoise::new(parameters);
        // The two legacy Nether biome noises are seeded from the world seed directly.
        let legacy_offset = match name.as_str() {
            "minecraft:nether/temperature" => Some(0),
            "minecraft:nether/vegetation" => Some(1),
            _ => None,
        };
        if let Some(offset) = legacy_offset {
            let mut random = AnyRandom::Legacy(LegacyRandom::new(self.seed.wrapping_add(offset)));
            return Ok(self.program.push_noise(noise.create_legacy_nether(&mut random)));
        }
        if let Some(&index) = self.noise_instances.get(name) {
            return Ok(index);
        }
        let index = self.program.push_noise(noise.create(&mut self.random.from_hash_of(name.as_str())));
        self.noise_instances.insert(name.clone(), index);
        Ok(index)
    }

    /// The compile context's `createRandom`.
    fn named_random(&self, name: &str) -> AnyRandom {
        if self.legacy_random && name == "minecraft:terrain" {
            return AnyRandom::Legacy(LegacyRandom::new(self.seed));
        }
        self.random.from_hash_of(name)
    }

    fn constant(df: &Df) -> Option<f32> {
        match df {
            Df::Constant(v) => Some(v.0),
            _ => None,
        }
    }

    fn compile(&mut self, function: &Arc<Df>) -> Result<Id, String> {
        let sampler = match &**function {
            Df::Constant(v) => Sampler::Constant(v.0),
            Df::Reference(name) => return self.compile(&self.registry.resolve(name)),
            Df::Prepared { id, .. } => return Ok(self.program.cache_samplers[id]),
            Df::Noise { noise, xz_scale, y_scale, shift } => {
                let noise = self.noise(noise)?;
                let is_zero = |d: &Df| matches!(d, Df::Constant(F32(v)) if v.to_bits() == 0);
                if shift.iter().all(|s| is_zero(s)) {
                    Sampler::Noise { noise, xz_scale: xz_scale.0, y_scale: y_scale.0 }
                } else {
                    let shift_x = self.compile(&shift[0])?;
                    let shift_z = self.compile(&shift[2])?;
                    if is_zero(&shift[1]) {
                        Sampler::ShiftedXz { shift_x, shift_z, noise, xz_scale: xz_scale.0, y_scale: y_scale.0 }
                    } else {
                        let shift_y = self.compile(&shift[1])?;
                        Sampler::ShiftedXyz { shift_x, shift_y, shift_z, noise, xz_scale: xz_scale.0, y_scale: y_scale.0 }
                    }
                }
            }
            Df::ShiftA(noise) => {
                let noise = self.noise(noise)?;
                let inner = self.program.push(Sampler::Noise { noise, xz_scale: 0.25, y_scale: 0.0 });
                Sampler::ConstMul(inner, 4.0)
            }
            Df::ShiftB(noise) => Sampler::ShiftB { noise: self.noise(noise)? },
            Df::Shift(noise) => {
                let noise = self.noise(noise)?;
                let inner = self.program.push(Sampler::Noise { noise, xz_scale: 0.25, y_scale: 0.25 });
                Sampler::ConstMul(inner, 4.0)
            }
            Df::Gradient { axis, tiling, from, to, from_value, to_value } => {
                let range = to - from;
                let factor = (to_value.0 - from_value.0) / range as f32;
                match tiling {
                    Tiling::ClampToEdge => Sampler::GradientClamped { axis: *axis, from: *from, min: (*from).min(*to), max: (*from).max(*to), from_value: from_value.0, factor },
                    Tiling::Repeat => Sampler::GradientRepeat { axis: *axis, from: *from, range, from_value: from_value.0, factor },
                    Tiling::MirroredRepeat => Sampler::GradientMirrored { axis: *axis, from: *from, range, from_value: from_value.0, factor },
                }
            }
            Df::OldBlendedNoise { xz_scale, y_scale, xz_factor, y_factor, smear_scale_multiplier } => {
                // BlendedNoise.compileSampler(context.createRandom("minecraft:terrain")).
                let mut random = self.named_random("minecraft:terrain");
                let xz_multiplier = 684.412 * xz_scale.0;
                let y_multiplier = 684.412 * y_scale.0;
                let limit_smear = y_multiplier * smear_scale_multiplier.0;
                let main_smear = limit_smear / y_factor.0;
                let min_limit = blended_fbm(&mut random, -15, limit_smear, 0.999_984_741_210_937_5);
                let max_limit = blended_fbm(&mut random, -15, limit_smear, 0.999_984_741_210_937_5);
                let main = blended_fbm(&mut random, -7, main_smear, 12.75);
                let (min_limit, max_limit, main) = (self.program.push_noise(min_limit), self.program.push_noise(max_limit), self.program.push_noise(main));
                let min_noise = self.program.push(Sampler::Noise { noise: min_limit, xz_scale: xz_multiplier, y_scale: y_multiplier });
                let max_noise = self.program.push(Sampler::Noise { noise: max_limit, xz_scale: xz_multiplier, y_scale: y_multiplier });
                let main_noise = self.program.push(Sampler::Noise { noise: main, xz_scale: xz_multiplier / xz_factor.0, y_scale: y_multiplier / y_factor.0 });
                let shifted = self.program.push(Sampler::ConstAdd(main_noise, 0.5));
                let choice = self.program.push(Sampler::Clamp(shifted, 0.0, 1.0));
                Sampler::Lerp(choice, min_noise, max_noise)
            }
            Df::Simple(kind) => {
                let (field, fallback) = match kind {
                    Simple::Beardifier => (ContextField::Beardifier, 0.0),
                    Simple::BlendAlpha => (ContextField::BlendAlpha, 1.0),
                    Simple::BlendOffset => (ContextField::BlendOffset, 0.0),
                };
                let fallback = self.program.push(Sampler::Constant(fallback));
                Sampler::ContextBound { field, fallback }
            }
            Df::EndIslands => {
                // createEndIslandRandom: a legacy random from the world seed.
                let mut random = LegacyRandom::new(self.seed);
                random.consume_count(17292);
                let noise = crate::temperature::Simplex::new(&mut random, true);
                self.program.simplex.push(noise);
                Sampler::EndIslands { noise: self.program.simplex.len() - 1 }
            }
            Df::DistanceToPoint { point, metric } => Sampler::DistanceToPoint { point: *point, metric: super::sampler::DistanceMetric::parse(metric)? },
            Df::Unary(op, input) => {
                let i = self.compile(input)?;
                match op {
                    Unary::Abs => Sampler::Abs(i),
                    Unary::Square => Sampler::Square(i),
                    Unary::Cube => Sampler::Cube(i),
                    Unary::Sqrt => Sampler::Sqrt(i),
                    Unary::HalfNegative => Sampler::LeakyRelu(i, 0.5),
                    Unary::QuarterNegative => Sampler::LeakyRelu(i, 0.25),
                    Unary::Reciprocal => Sampler::Reciprocal(i),
                    Unary::Negate => Sampler::Negate(i),
                    Unary::Squeeze => Sampler::Squeeze(i),
                    Unary::Log => Sampler::Log(i),
                    Unary::Sign => Sampler::Sign(i),
                }
            }
            Df::Round(op, input, multiple) => {
                let i = self.compile(input)?;
                if Self::constant(multiple) == Some(1.0) {
                    Sampler::RoundInteger(*op, i)
                } else {
                    Sampler::Round(*op, i, self.compile(multiple)?)
                }
            }
            Df::Binary(op, left, right) => return self.compile_binary(*op, left, right),
            Df::Pow(base, exponent) => {
                let b = self.compile(base)?;
                let e = self.compile(exponent)?;
                if let Some(base_value) = Self::constant(base) {
                    Sampler::PowConstBase(base_value, e)
                } else if let Some(exponent_value) = Self::constant(exponent) {
                    return Ok(self.compile_const_exponent(b, exponent_value));
                } else {
                    Sampler::Pow(b, e)
                }
            }
            Df::Lerp(alpha, first, second) => {
                let a = self.compile(alpha)?;
                let f = self.compile(first)?;
                let s = self.compile(second)?;
                if let Some(first_value) = Self::constant(first) {
                    Sampler::LerpConstFirst(a, first_value, s)
                } else if let Some(second_value) = Self::constant(second) {
                    Sampler::LerpConstSecond(a, f, second_value)
                } else {
                    Sampler::Lerp(a, f, s)
                }
            }
            Df::Clamp(input, lo, hi) => Sampler::Clamp(self.compile(input)?, lo.0, hi.0),
            Df::RangeChoice { input, min_inclusive, max_exclusive, in_range, out_of_range } => {
                let i = self.compile(input)?;
                match (Self::constant(in_range), Self::constant(out_of_range)) {
                    (Some(a), Some(b)) => Sampler::RangeChoiceConst { input: i, min: min_inclusive.0, max: max_exclusive.0, in_range: a, out_of_range: b },
                    _ => Sampler::RangeChoice { input: i, min: min_inclusive.0, max: max_exclusive.0, in_range: self.compile(in_range)?, out_of_range: self.compile(out_of_range)? },
                }
            }
            Df::IntervalSelect { input, thresholds, functions } => {
                let i = self.compile(input)?;
                if thresholds.len() == 1 {
                    let below = self.compile(&functions[0])?;
                    let above = self.compile(functions.last().expect("two or more functions"))?;
                    Sampler::IntervalSelectSingle { input: i, threshold: thresholds[0].0, below, above }
                } else {
                    let samplers = functions.iter().map(|f| self.compile(f)).collect::<Result<_, _>>()?;
                    Sampler::IntervalSelect { input: i, thresholds: thresholds.iter().map(|t| t.0).collect(), samplers }
                }
            }
            Df::Cache(_) => return Err("cannot compile a cache before it has been deduplicated".into()),
            Df::BlendDensity(input) => Sampler::BlendDensity(self.compile(input)?),
            Df::Interpolated { input, cell_xz, cell_y } => Sampler::Interpolated {
                input: self.compile(input)?,
                cell_xz: *cell_xz,
                cell_y: *cell_y,
                inv_xz: 1.0 / *cell_xz as f32,
                inv_y: 1.0 / *cell_y as f32,
            },
            Df::Slice { axis, coordinate, input } => {
                if let Df::Slice { axis: inner_axis, coordinate: inner_coordinate, input: inner_input } = &**input {
                    let merged = match (axis, inner_axis) {
                        (Axis::X, Axis::Z) => Some((*coordinate, *inner_coordinate)),
                        (Axis::Z, Axis::X) => Some((*inner_coordinate, *coordinate)),
                        _ => None,
                    };
                    if let Some((x, z)) = merged {
                        let i = self.compile(inner_input)?;
                        return Ok(self.program.push(Sampler::SliceXz { input: i, x, z }));
                    }
                }
                let i = self.compile(input)?;
                match axis {
                    Axis::X => Sampler::SliceX { input: i, x: *coordinate },
                    Axis::Y => Sampler::SliceY { input: i, y: *coordinate },
                    Axis::Z => Sampler::SliceZ { input: i, z: *coordinate },
                }
            }
            Df::FindTopSurface { density, upper_bound, lower_bound, cell_height } => {
                let d = self.compile(density)?;
                let u = self.compile(upper_bound)?;
                let inner = self.program.push(Sampler::FindTopSurface { density: d, upper_bound: u, lower_bound: *lower_bound, cell_height: *cell_height });
                Sampler::SliceY { input: inner, y: 0 }
            }
            Df::Spline(spline) => {
                // Coordinates are shared by structural equality, indexed in first-seen order.
                let mut coordinates: Vec<(Arc<Df>, Id)> = Vec::new();
                let mut order = Vec::new();
                for_each_coordinate(spline, &mut |c| order.push(c.clone()));
                for c in order {
                    if !coordinates.iter().any(|(k, _)| *k == c) {
                        let id = self.compile(&c)?;
                        coordinates.push((c, id));
                    }
                }
                let compiled = Self::compile_spline(spline, &coordinates);
                Sampler::Spline { spline: compiled, coordinate_count: coordinates.len() }
            }
        };
        Ok(self.program.push(sampler))
    }

    fn compile_spline(spline: &Spline, coordinates: &[(Arc<Df>, Id)]) -> CSpline {
        match spline {
            Spline::Constant(v) => CSpline::Constant(v.0),
            Spline::Multipoint { coordinate, locations, values, derivatives } => {
                let index = coordinates.iter().position(|(k, _)| k == coordinate).expect("coordinate registered");
                CSpline::Multipoint {
                    sampler: coordinates[index].1,
                    index,
                    locations: locations.iter().map(|l| l.0).collect(),
                    values: values.iter().map(|v| Self::compile_spline(v, coordinates)).collect(),
                    derivatives: derivatives.iter().map(|d| d.0).collect(),
                }
            }
        }
    }

    /// `BinaryFunction.compileSampler`.
    fn compile_binary(&mut self, op: Binary, left: &Arc<Df>, right: &Arc<Df>) -> Result<Id, String> {
        let l = self.compile(left)?;
        let r = self.compile(right)?;
        let (lc, rc) = (Self::constant(left), Self::constant(right));
        let sampler = match op {
            Binary::Add => match (lc, rc) {
                (Some(c), _) => Sampler::ConstAdd(r, c),
                (None, Some(c)) => Sampler::ConstAdd(l, c),
                _ => Sampler::Add(l, r),
            },
            Binary::Sub => match (lc, rc) {
                (Some(c), _) => Sampler::ConstSub(c, r),
                (None, Some(c)) => Sampler::ConstAdd(l, -c),
                _ => Sampler::Sub(l, r),
            },
            Binary::Mul => match (lc, rc) {
                (Some(c), _) => Sampler::ConstMul(r, c),
                (None, Some(c)) => Sampler::ConstMul(l, c),
                _ => Sampler::Mul(l, r),
            },
            Binary::Div => match (lc, rc) {
                (Some(c), _) => Sampler::ConstDiv(c, r),
                (None, Some(c)) => Sampler::ConstMul(l, 1.0 / c),
                _ => Sampler::Div(l, r),
            },
            Binary::Min => {
                let (lr, rr) = (self.registry.range(left), self.registry.range(right));
                if lr.max() < rr.min() {
                    return Ok(l);
                }
                if rr.max() < lr.min() {
                    return Ok(r);
                }
                match (lc, rc) {
                    (Some(c), _) => Sampler::ConstMin(r, c),
                    (None, Some(c)) => Sampler::ConstMin(l, c),
                    _ => Sampler::Min(l, r, rr.min()),
                }
            }
            Binary::Max => {
                let (lr, rr) = (self.registry.range(left), self.registry.range(right));
                if lr.min() > rr.max() {
                    return Ok(l);
                }
                if rr.min() > lr.max() {
                    return Ok(r);
                }
                match (lc, rc) {
                    (Some(c), _) => Sampler::ConstMax(r, c),
                    (None, Some(c)) => Sampler::ConstMax(l, c),
                    _ => Sampler::Max(l, r, rr.max()),
                }
            }
        };
        Ok(self.program.push(sampler))
    }

    /// `PowFunction.compileConstExponent`.
    fn compile_const_exponent(&mut self, base: Id, exponent: f32) -> Id {
        let special = match exponent.abs() {
            0.5 => self.program.push(Sampler::Sqrt(base)),
            1.0 => base,
            2.0 => self.program.push(Sampler::Square(base)),
            3.0 => self.program.push(Sampler::Cube(base)),
            _ => return self.program.push(Sampler::PowConstExponent(base, exponent)),
        };
        if exponent >= 0.0 { special } else { self.program.push(Sampler::Reciprocal(special)) }
    }
}
