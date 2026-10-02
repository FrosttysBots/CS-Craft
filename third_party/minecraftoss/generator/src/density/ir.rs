//! Parsed density functions (vanilla `DensityFunction` records).
//!
//! Nodes compare structurally with floats by bit pattern (Java record
//! equality uses `Float.compare`) and references by name, which is what
//! vanilla's cache deduplication and spline-coordinate sharing key on.

use crate::interval::Interval;
use crate::mth;
use crate::noise::NormalNoiseParameters;
use minecraftoss_core::Identifier;
use minecraftoss_core::datapack::DataPack;
use serde_json::Value;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// An `f32` compared and hashed by bit pattern.
#[derive(Clone, Copy, Debug)]
pub struct F32(pub f32);

impl PartialEq for F32 {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}
impl Eq for F32 {}
impl Hash for F32 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

/// An `f64` compared and hashed by bit pattern.
#[derive(Clone, Copy, Debug)]
pub struct F64(pub f64);

impl PartialEq for F64 {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}
impl Eq for F64 {}
impl Hash for F64 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

pub const AXIS_X: u8 = 1;
pub const AXIS_Y: u8 = 2;
pub const AXIS_Z: u8 = 4;
pub const ALL_AXES: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn bit(self) -> u8 {
        match self {
            Self::X => AXIS_X,
            Self::Y => AXIS_Y,
            Self::Z => AXIS_Z,
        }
    }

    pub fn choose(self, x: i32, y: i32, z: i32) -> i32 {
        match self {
            Self::X => x,
            Self::Y => y,
            Self::Z => z,
        }
    }

    fn parse(value: &Value) -> Result<Self, String> {
        match value.as_str() {
            Some("x") => Ok(Self::X),
            Some("y") => Ok(Self::Y),
            Some("z") => Ok(Self::Z),
            _ => Err(format!("invalid axis {value}")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tiling {
    ClampToEdge,
    Repeat,
    MirroredRepeat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unary {
    Abs,
    Square,
    Cube,
    Sqrt,
    HalfNegative,
    QuarterNegative,
    Reciprocal,
    Negate,
    Squeeze,
    Log,
    Sign,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Round {
    Floor,
    Round,
    Ceil,
    Truncate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Binary {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Simple {
    BlendAlpha,
    BlendOffset,
    Beardifier,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spline {
    Constant(F32),
    Multipoint { coordinate: Arc<Df>, locations: Vec<F32>, values: Vec<Spline>, derivatives: Vec<F32> },
}

/// One density-function node.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Df {
    Constant(F32),
    /// A registry reference (`HolderHolder`).
    Reference(Identifier),
    Noise { noise: Identifier, xz_scale: F64, y_scale: F64, shift: [Arc<Df>; 3] },
    ShiftA(Identifier),
    ShiftB(Identifier),
    Shift(Identifier),
    Gradient { axis: Axis, tiling: Tiling, from: i32, to: i32, from_value: F32, to_value: F32 },
    OldBlendedNoise { xz_scale: F64, y_scale: F64, xz_factor: F64, y_factor: F64, smear_scale_multiplier: F64 },
    Simple(Simple),
    EndIslands,
    DistanceToPoint { point: [i32; 3], metric: String },
    Unary(Unary, Arc<Df>),
    Round(Round, Arc<Df>, Arc<Df>),
    Binary(Binary, Arc<Df>, Arc<Df>),
    Pow(Arc<Df>, Arc<Df>),
    Lerp(Arc<Df>, Arc<Df>, Arc<Df>),
    Clamp(Arc<Df>, F32, F32),
    RangeChoice { input: Arc<Df>, min_inclusive: F32, max_exclusive: F32, in_range: Arc<Df>, out_of_range: Arc<Df> },
    IntervalSelect { input: Arc<Df>, thresholds: Vec<F32>, functions: Vec<Arc<Df>> },
    Cache(Arc<Df>),
    BlendDensity(Arc<Df>),
    Interpolated { input: Arc<Df>, cell_xz: i32, cell_y: i32 },
    Slice { axis: Axis, coordinate: i32, input: Arc<Df> },
    FindTopSurface { density: Arc<Df>, upper_bound: Arc<Df>, lower_bound: i32, cell_height: i32 },
    Spline(Spline),
    /// A deduplicated cache produced by the compiler (`PreparedCache`).
    Prepared { id: usize, range: IntervalKey, axes: u8 },
}

/// An `Interval` usable inside hashed nodes.
#[derive(Clone, Copy, Debug)]
pub struct IntervalKey(pub Interval);
impl PartialEq for IntervalKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.min().to_bits() == other.0.min().to_bits() && self.0.max().to_bits() == other.0.max().to_bits()
    }
}
impl Eq for IntervalKey {}
impl Hash for IntervalKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.0.min().to_bits(), self.0.max().to_bits()).hash(state);
    }
}

pub fn zero() -> Arc<Df> {
    Arc::new(Df::Constant(F32(0.0)))
}

/// Named density functions and noise parameters loaded from a data pack.
pub struct Registry {
    pack: DataPack,
    functions: HashMap<Identifier, Arc<Df>>,
    noises: HashMap<Identifier, NormalNoiseParameters>,
}

fn f32_field(json: &Value, key: &str) -> Result<F32, String> {
    let v = json.get(key).and_then(Value::as_f64).ok_or_else(|| format!("density function lacks numeric {key}"))?;
    let v = v as f32;
    if !(-1_000_000.0..=1_000_000.0).contains(&v) {
        return Err(format!("{key} {v} outside the noise value range"));
    }
    Ok(F32(v))
}

fn i32_field(json: &Value, key: &str) -> Result<i32, String> {
    json.get(key).and_then(Value::as_i64).map(|v| v as i32).ok_or_else(|| format!("density function lacks integer {key}"))
}

fn f64_field(json: &Value, key: &str) -> Result<F64, String> {
    json.get(key).and_then(Value::as_f64).map(F64).ok_or_else(|| format!("density function lacks numeric {key}"))
}

impl Registry {
    pub fn new(pack: DataPack) -> Self {
        Self { pack, functions: HashMap::new(), noises: HashMap::new() }
    }

    pub fn pack(&self) -> &DataPack {
        &self.pack
    }

    /// Loads a named function and everything it references.
    pub fn function(&mut self, name: &Identifier) -> Result<Arc<Df>, String> {
        if let Some(f) = self.functions.get(name) {
            return Ok(f.clone());
        }
        let json = self.pack.read_json("worldgen/density_function", name)?;
        let f = self.parse(&json).map_err(|e| format!("density function {name}: {e}"))?;
        self.functions.insert(name.clone(), f.clone());
        Ok(f)
    }

    pub fn noise(&mut self, name: &Identifier) -> Result<NormalNoiseParameters, String> {
        if let Some(p) = self.noises.get(name) {
            return Ok(p.clone());
        }
        let p = NormalNoiseParameters::from_json(&self.pack.read_json("worldgen/noise", name)?).map_err(|e| format!("noise {name}: {e}"))?;
        self.noises.insert(name.clone(), p.clone());
        Ok(p)
    }

    fn noise_ref(&mut self, json: &Value) -> Result<Identifier, String> {
        let name = Identifier::parse(json.as_str().ok_or("inline noise definitions are not supported")?)?;
        self.noise(&name)?;
        Ok(name)
    }

    /// Parses a density function in `DensityFunction.CODEC` form.
    pub fn parse(&mut self, json: &Value) -> Result<Arc<Df>, String> {
        if let Some(v) = json.as_f64() {
            return Ok(Arc::new(Df::Constant(F32(v as f32))));
        }
        if let Some(name) = json.as_str() {
            let name = Identifier::parse(name)?;
            self.function(&name)?;
            return Ok(Arc::new(Df::Reference(name)));
        }
        let kind = json["type"].as_str().ok_or("density function lacks a type")?;
        let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
        let child = |this: &mut Self, key: &str| -> Result<Arc<Df>, String> {
            this.parse(json.get(key).ok_or_else(|| format!("{kind} lacks {key}"))?)
        };
        let node = match kind {
            "constant" => Df::Constant(f32_field(json, "value")?),
            "noise" => {
                let noise = self.noise_ref(&json["noise"])?;
                let shift = |this: &mut Self, key: &str| match json.get(key) {
                    Some(v) => this.parse(v),
                    None => Ok(zero()),
                };
                Df::Noise {
                    noise,
                    xz_scale: f64_field(json, "xz_scale")?,
                    y_scale: f64_field(json, "y_scale")?,
                    shift: [shift(self, "shift_x")?, shift(self, "shift_y")?, shift(self, "shift_z")?],
                }
            }
            "shift_a" => Df::ShiftA(self.noise_ref(&json["noise"])?),
            "shift_b" => Df::ShiftB(self.noise_ref(&json["noise"])?),
            "shift" => Df::Shift(self.noise_ref(&json["noise"])?),
            "gradient" => {
                let tiling = match json.get("tiling").and_then(Value::as_str).unwrap_or("clamp_to_edge") {
                    "clamp_to_edge" => Tiling::ClampToEdge,
                    "repeat" => Tiling::Repeat,
                    "mirrored_repeat" => Tiling::MirroredRepeat,
                    other => return Err(format!("unknown tiling {other}")),
                };
                let (from, to) = (i32_field(json, "from_coordinate")?, i32_field(json, "to_coordinate")?);
                if from == to {
                    return Err("from_coordinate cannot be equal to to_coordinate".into());
                }
                Df::Gradient { axis: Axis::parse(&json["axis"])?, tiling, from, to, from_value: f32_field(json, "from_value")?, to_value: f32_field(json, "to_value")? }
            }
            "old_blended_noise" => Df::OldBlendedNoise {
                xz_scale: f64_field(json, "xz_scale")?,
                y_scale: f64_field(json, "y_scale")?,
                xz_factor: f64_field(json, "xz_factor")?,
                y_factor: f64_field(json, "y_factor")?,
                smear_scale_multiplier: f64_field(json, "smear_scale_multiplier")?,
            },
            "blend_alpha" => Df::Simple(Simple::BlendAlpha),
            "blend_offset" => Df::Simple(Simple::BlendOffset),
            "beardifier" => Df::Simple(Simple::Beardifier),
            "end_outer_islands" => Df::EndIslands,
            "distance_to_point" => {
                let p = json["point"].as_array().filter(|p| p.len() == 3).ok_or("point must be [x,y,z]")?;
                let c = |i: usize| p[i].as_i64().map(|v| v as i32).ok_or("point coordinate must be an integer");
                Df::DistanceToPoint { point: [c(0)?, c(1)?, c(2)?], metric: json["metric"].as_str().ok_or("metric must be a string")?.to_owned() }
            }
            "abs" | "square" | "cube" | "sqrt" | "half_negative" | "quarter_negative" | "reciprocal" | "negate" | "squeeze" | "log" | "sign" => {
                let op = match kind {
                    "abs" => Unary::Abs,
                    "square" => Unary::Square,
                    "cube" => Unary::Cube,
                    "sqrt" => Unary::Sqrt,
                    "half_negative" => Unary::HalfNegative,
                    "quarter_negative" => Unary::QuarterNegative,
                    "reciprocal" => Unary::Reciprocal,
                    "negate" => Unary::Negate,
                    "squeeze" => Unary::Squeeze,
                    "log" => Unary::Log,
                    _ => Unary::Sign,
                };
                Df::Unary(op, child(self, "input")?)
            }
            "floor" | "round" | "ceil" | "truncate" => {
                let op = match kind {
                    "floor" => Round::Floor,
                    "round" => Round::Round,
                    "ceil" => Round::Ceil,
                    _ => Round::Truncate,
                };
                let multiple = match json.get("multiple") {
                    Some(v) => self.parse(v)?,
                    None => Arc::new(Df::Constant(F32(1.0))),
                };
                Df::Round(op, child(self, "input")?, multiple)
            }
            "add" | "sub" | "mul" | "div" | "min" | "max" => {
                let op = match kind {
                    "add" => Binary::Add,
                    "sub" => Binary::Sub,
                    "mul" => Binary::Mul,
                    "div" => Binary::Div,
                    "min" => Binary::Min,
                    _ => Binary::Max,
                };
                Df::Binary(op, child(self, "left")?, child(self, "right")?)
            }
            "pow" => Df::Pow(child(self, "base")?, child(self, "exponent")?),
            "lerp" => Df::Lerp(child(self, "alpha")?, child(self, "first")?, child(self, "second")?),
            "clamp" => {
                let (lo, hi) = (f32_field(json, "min")?, f32_field(json, "max")?);
                if lo.0 > hi.0 {
                    return Err(format!("clamp min {} > max {}", lo.0, hi.0));
                }
                Df::Clamp(child(self, "input")?, lo, hi)
            }
            "range_choice" => Df::RangeChoice {
                input: child(self, "input")?,
                min_inclusive: f32_field(json, "min_inclusive")?,
                max_exclusive: f32_field(json, "max_exclusive")?,
                in_range: child(self, "when_in_range")?,
                out_of_range: child(self, "when_out_of_range")?,
            },
            "interval_select" => {
                let thresholds: Vec<F32> = json["thresholds"].as_array().ok_or("thresholds must be an array")?.iter().map(|t| t.as_f64().map(|v| F32(v as f32)).ok_or("threshold must be numeric")).collect::<Result<_, _>>()?;
                let functions = json["functions"].as_array().ok_or("functions must be an array")?.iter().map(|f| self.parse(f)).collect::<Result<Vec<_>, _>>()?;
                if functions.len() < 2 || thresholds.len() != functions.len() - 1 {
                    return Err(format!("expected {} thresholds for {} functions", functions.len().saturating_sub(1), functions.len()));
                }
                if thresholds.windows(2).any(|w| w[0].0 > w[1].0) {
                    return Err("threshold values must be ordered from smallest to largest".into());
                }
                Df::IntervalSelect { input: child(self, "input")?, thresholds, functions }
            }
            "cache" => Df::Cache(child(self, "input")?),
            "blend_density" => Df::BlendDensity(child(self, "input")?),
            "interpolated" => {
                let (cell_xz, cell_y) = (i32_field(json, "cell_size_xz")?, i32_field(json, "cell_size_y")?);
                if cell_xz <= 0 || cell_y <= 0 {
                    return Err("interpolation cell sizes must be positive".into());
                }
                Df::Interpolated { input: child(self, "input")?, cell_xz, cell_y }
            }
            "slice" => Df::Slice { axis: Axis::parse(&json["axis"])?, coordinate: i32_field(json, "coordinate")?, input: child(self, "input")? },
            "find_top_surface" => Df::FindTopSurface {
                density: child(self, "density")?,
                upper_bound: child(self, "upper_bound")?,
                lower_bound: i32_field(json, "lower_bound")?,
                cell_height: i32_field(json, "cell_height")?,
            },
            "spline" => Df::Spline(self.parse_spline(&json["spline"])?),
            other => return Err(format!("unknown density function type {other}")),
        };
        Ok(Arc::new(node))
    }

    fn parse_spline(&mut self, json: &Value) -> Result<Spline, String> {
        if let Some(v) = json.as_f64() {
            return Ok(Spline::Constant(F32(v as f32)));
        }
        let coordinate = self.parse(json.get("coordinate").ok_or("spline lacks coordinate")?)?;
        let points = json["points"].as_array().filter(|p| !p.is_empty()).ok_or("spline needs points")?;
        let (mut locations, mut values, mut derivatives) = (Vec::new(), Vec::new(), Vec::new());
        for p in points {
            locations.push(F32(p["location"].as_f64().ok_or("spline point lacks location")? as f32));
            values.push(self.parse_spline(&p["value"])?);
            derivatives.push(F32(p["derivative"].as_f64().ok_or("spline point lacks derivative")? as f32));
        }
        Ok(Spline::Multipoint { coordinate, locations, values, derivatives })
    }

    /// Resolves a reference to its definition.
    pub fn resolve(&self, name: &Identifier) -> Arc<Df> {
        self.functions.get(name).cloned().expect("references are loaded while parsing")
    }

    /// `NormalNoise.range()` from its parameters.
    pub fn noise_range(&self, name: &Identifier) -> Interval {
        crate::noise::NormalNoise::new(self.noises[name].clone()).range()
    }

    /// `DensityFunction.range()`.
    pub fn range(&self, df: &Df) -> Interval {
        match df {
            Df::Constant(v) => Interval::exact(v.0),
            Df::Reference(name) => self.range(&self.resolve(name)),
            Df::Noise { noise, .. } => self.noise_range(noise),
            Df::ShiftA(n) | Df::ShiftB(n) | Df::Shift(n) => Interval::mul(self.noise_range(n), Interval::exact(4.0)),
            Df::Gradient { from_value, to_value, .. } => Interval::encapsulating2(from_value.0, to_value.0),
            Df::OldBlendedNoise { y_scale, smear_scale_multiplier, .. } => blended_range(684.412 * y_scale.0 * smear_scale_multiplier.0),
            Df::Simple(Simple::BlendAlpha) => Interval::of(0.0, 1.0),
            Df::Simple(_) => Interval::INFINITE,
            Df::EndIslands => Interval::of(-0.84375, 0.5625),
            Df::DistanceToPoint { .. } => Interval::of(0.0, f32::INFINITY),
            Df::Unary(op, input) => {
                let i = self.range(input);
                match op {
                    Unary::Abs => Interval::abs(i),
                    Unary::Square => Interval::square(i),
                    Unary::Sqrt => Interval::pow(i, Interval::exact(0.5)),
                    Unary::Reciprocal => Interval::reciprocal(i),
                    Unary::Negate => Interval::sub(Interval::exact(0.0), i),
                    Unary::Cube => Interval::map_monotonic(i, |v| v * v * v),
                    Unary::HalfNegative => Interval::map_monotonic(i, |v| leaky_relu(0.5, v)),
                    Unary::QuarterNegative => Interval::map_monotonic(i, |v| leaky_relu(0.25, v)),
                    Unary::Squeeze => Interval::map_monotonic(i, squeeze),
                    Unary::Log => Interval::log(i),
                    Unary::Sign => Interval::sign(i),
                }
            }
            Df::Round(op, input, multiple) => {
                let m = self.range(multiple);
                Interval::mul(Interval::map_monotonic(Interval::div(self.range(input), m), |v| round_to_integer(v, *op)), m)
            }
            Df::Binary(op, l, r) => {
                let (l, r) = (self.range(l), self.range(r));
                match op {
                    Binary::Add => Interval::add(l, r),
                    Binary::Sub => Interval::sub(l, r),
                    Binary::Mul => Interval::mul(l, r),
                    Binary::Div => Interval::div(l, r),
                    Binary::Min => Interval::min_of(l, r),
                    Binary::Max => Interval::max_of(l, r),
                }
            }
            Df::Pow(b, e) => Interval::pow(self.range(b), self.range(e)),
            Df::Lerp(a, f, s) => Interval::lerp(self.range(a), self.range(f), self.range(s)),
            Df::Clamp(input, lo, hi) => Interval::clamp(self.range(input), lo.0, hi.0),
            Df::RangeChoice { in_range, out_of_range, .. } => Interval::encapsulating(&[self.range(in_range), self.range(out_of_range)]),
            Df::IntervalSelect { functions, .. } => Interval::encapsulating(&functions.iter().map(|f| self.range(f)).collect::<Vec<_>>()),
            Df::Cache(input) | Df::BlendDensity(input) | Df::Interpolated { input, .. } | Df::Slice { input, .. } => self.range(input),
            Df::FindTopSurface { upper_bound, lower_bound, .. } => {
                Interval::of(*lower_bound as f32, mth::max(*lower_bound as f32, self.range(upper_bound).max()))
            }
            Df::Spline(s) => self.spline_range(s),
            Df::Prepared { range, .. } => range.0,
        }
    }

    /// `CubicSpline.range()`.
    pub fn spline_range(&self, spline: &Spline) -> Interval {
        let Spline::Multipoint { coordinate, locations, values, derivatives } = spline else {
            let Spline::Constant(v) = spline else { unreachable!() };
            return Interval::exact(v.0);
        };
        let loc = |i: usize| locations[i].0;
        let der = |i: usize| derivatives[i].0;
        let extend = |input: f32, value: f32, i: usize| if der(i) == 0.0 { value } else { value + der(i) * (input - loc(i)) };
        let last = locations.len() - 1;
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        let input = self.range(coordinate);
        if input.is_nai() {
            return input;
        }
        if input.min() < loc(0) {
            let first = self.spline_range(&values[0]);
            let (e1, e2) = (extend(input.min(), first.min(), 0), extend(input.min(), first.max(), 0));
            lo = mth::min(lo, mth::min(e1, e2));
            hi = mth::max(hi, mth::max(e1, e2));
        }
        if input.max() > loc(last) {
            let tail = self.spline_range(&values[last]);
            let (e1, e2) = (extend(input.max(), tail.min(), last), extend(input.max(), tail.max(), last));
            lo = mth::min(lo, mth::min(e1, e2));
            hi = mth::max(hi, mth::max(e1, e2));
        }
        let ranges: Vec<Interval> = values.iter().map(|v| self.spline_range(v)).collect();
        for r in &ranges {
            lo = mth::min(lo, r.min());
            hi = mth::max(hi, r.max());
        }
        for i in 0..last {
            let x_diff = loc(i + 1) - loc(i);
            let (r1, r2) = (ranges[i], ranges[i + 1]);
            let (d1, d2) = (der(i), der(i + 1));
            if d1 == 0.0 && d2 == 0.0 {
                continue;
            }
            let (p1, p2) = (d1 * x_diff, d2 * x_diff);
            let min_lerp1 = mth::min(r1.min(), r2.min());
            let max_lerp1 = mth::max(r1.max(), r2.max());
            let min_a = p1 - r2.max() + r1.min();
            let max_a = p1 - r2.min() + r1.max();
            let min_b = -p2 + r2.min() - r1.max();
            let max_b = -p2 + r2.max() - r1.min();
            lo = mth::min(lo, min_lerp1 + 0.25 * mth::min(min_a, min_b));
            hi = mth::max(hi, max_lerp1 + 0.25 * mth::max(max_a, max_b));
        }
        Interval::of(lo, hi)
    }

    /// `DensityFunction.domainAxes()`.
    pub fn axes(&self, df: &Df) -> u8 {
        match df {
            Df::Constant(_) => 0,
            Df::Reference(name) => self.axes(&self.resolve(name)),
            Df::Noise { xz_scale, y_scale, shift, .. } => {
                let mut axes = ALL_AXES;
                if y_scale.0 == 0.0 {
                    axes &= !AXIS_Y;
                }
                if xz_scale.0 == 0.0 {
                    axes &= !(AXIS_X | AXIS_Z);
                }
                axes | self.axes(&shift[0]) | self.axes(&shift[1]) | self.axes(&shift[2])
            }
            Df::ShiftA(_) | Df::ShiftB(_) => AXIS_X | AXIS_Z,
            Df::Shift(_) | Df::OldBlendedNoise { .. } | Df::DistanceToPoint { .. } => ALL_AXES,
            Df::Gradient { axis, .. } => axis.bit(),
            Df::Simple(Simple::Beardifier) => ALL_AXES,
            Df::Simple(_) | Df::EndIslands => AXIS_X | AXIS_Z,
            Df::Unary(_, i) | Df::Clamp(i, ..) | Df::Cache(i) | Df::BlendDensity(i) | Df::Interpolated { input: i, .. } => self.axes(i),
            Df::Round(_, a, b) | Df::Binary(_, a, b) | Df::Pow(a, b) => self.axes(a) | self.axes(b),
            Df::Lerp(a, b, c) => self.axes(a) | self.axes(b) | self.axes(c),
            Df::RangeChoice { input, in_range, out_of_range, .. } => self.axes(input) | self.axes(in_range) | self.axes(out_of_range),
            Df::IntervalSelect { input, functions, .. } => functions.iter().fold(self.axes(input), |a, f| a | self.axes(f)),
            Df::Slice { axis, input, .. } => self.axes(input) & !axis.bit(),
            Df::FindTopSurface { density, upper_bound, .. } => (self.axes(density) | self.axes(upper_bound)) & !AXIS_Y,
            Df::Spline(s) => {
                let mut axes = 0;
                for_each_coordinate(s, &mut |c| axes |= self.axes(c));
                axes
            }
            Df::Prepared { axes, .. } => *axes,
        }
    }
}

pub fn for_each_coordinate(spline: &Spline, f: &mut impl FnMut(&Arc<Df>)) {
    if let Spline::Multipoint { coordinate, values, .. } = spline {
        f(coordinate);
        for v in values {
            for_each_coordinate(v, f);
        }
    }
}

pub fn leaky_relu(negative_factor: f32, input: f32) -> f32 {
    if input > 0.0 { input } else { input * negative_factor }
}

pub fn squeeze(input: f32) -> f32 {
    let c = mth::clamp(input, -1.0, 1.0);
    c / 2.0 - c * c * c / 24.0
}

/// `RoundFunction.roundToInteger`.
pub fn round_to_integer(input: f32, op: Round) -> f32 {
    let x = f64::from(input);
    match op {
        Round::Floor => x.floor() as f32,
        // Math.round(float): floor(x + 1/2) as an int, NaN -> 0, saturating.
        Round::Round => (x + 0.5).floor() as i32 as f32,
        Round::Ceil => x.ceil() as f32,
        Round::Truncate => if input > 0.0 { x.floor() as f32 } else { x.ceil() as f32 },
    }
}

/// `BlendedNoise.computeFbmRange(-15, smearScaleY, LIMIT_FACTOR)`.
fn blended_range(smear_scale_y: f64) -> Interval {
    let octaves = 16;
    let mut factor = 1.0;
    let mut value_factor = 0.999_984_741_210_937_5 / (2f64.powi(octaves) - 1.0);
    let mut range = Interval::exact(0.0);
    for _ in 0..octaves {
        let layer = Interval::mul(Interval::symmetric(((smear_scale_y * factor).abs() + 2.0) as f32), Interval::exact(value_factor as f32));
        range = Interval::add(range, layer);
        factor /= 2.0;
        value_factor *= 2.0;
    }
    range
}
