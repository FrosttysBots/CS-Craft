//! Compiled density samplers (vanilla `DensitySampler` implementations).

use super::ir::{Axis, Round};
use crate::noise::NoiseStack;
use std::collections::HashMap;

/// Index of a sampler in a `Program`.
pub type Id = usize;

/// Samplers that read a value supplied by the chunk being generated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextField {
    Beardifier,
    BlendAlpha,
    BlendOffset,
}

#[derive(Clone, Debug)]
pub enum CSpline {
    Constant(f32),
    Multipoint { sampler: Id, index: usize, locations: Vec<f32>, values: Vec<CSpline>, derivatives: Vec<f32> },
}

#[derive(Clone, Debug)]
pub enum Sampler {
    Constant(f32),
    Noise { noise: usize, xz_scale: f64, y_scale: f64 },
    ShiftedXz { shift_x: Id, shift_z: Id, noise: usize, xz_scale: f64, y_scale: f64 },
    ShiftedXyz { shift_x: Id, shift_y: Id, shift_z: Id, noise: usize, xz_scale: f64, y_scale: f64 },
    ShiftB { noise: usize },
    GradientClamped { axis: Axis, from: i32, min: i32, max: i32, from_value: f32, factor: f32 },
    GradientRepeat { axis: Axis, from: i32, range: i32, from_value: f32, factor: f32 },
    GradientMirrored { axis: Axis, from: i32, range: i32, from_value: f32, factor: f32 },
    ContextBound { field: ContextField, fallback: Id },
    Abs(Id),
    Square(Id),
    Cube(Id),
    Sqrt(Id),
    LeakyRelu(Id, f32),
    Reciprocal(Id),
    Negate(Id),
    Squeeze(Id),
    Log(Id),
    Sign(Id),
    RoundInteger(Round, Id),
    Round(Round, Id, Id),
    ConstAdd(Id, f32),
    Add(Id, Id),
    ConstSub(f32, Id),
    Sub(Id, Id),
    ConstMul(Id, f32),
    Mul(Id, Id),
    ConstDiv(f32, Id),
    Div(Id, Id),
    ConstMin(Id, f32),
    /// Left, right, and the right side's range minimum.
    Min(Id, Id, f32),
    ConstMax(Id, f32),
    /// Left, right, and the right side's range maximum.
    Max(Id, Id, f32),
    PowConstBase(f32, Id),
    PowConstExponent(Id, f32),
    Pow(Id, Id),
    LerpConstFirst(Id, f32, Id),
    LerpConstSecond(Id, Id, f32),
    Lerp(Id, Id, Id),
    Clamp(Id, f32, f32),
    RangeChoiceConst { input: Id, min: f32, max: f32, in_range: f32, out_of_range: f32 },
    RangeChoice { input: Id, min: f32, max: f32, in_range: Id, out_of_range: Id },
    IntervalSelectSingle { input: Id, threshold: f32, below: Id, above: Id },
    IntervalSelect { input: Id, thresholds: Vec<f32>, samplers: Vec<Id> },
    Cache { id: usize, input: Id },
    BlendDensity(Id),
    Interpolated { input: Id, cell_xz: i32, cell_y: i32, inv_xz: f32, inv_y: f32 },
    SliceX { input: Id, x: i32 },
    SliceY { input: Id, y: i32 },
    SliceZ { input: Id, z: i32 },
    SliceXz { input: Id, x: i32, z: i32 },
    FindTopSurface { density: Id, upper_bound: Id, lower_bound: i32, cell_height: i32 },
    Spline { spline: CSpline, coordinate_count: usize },
    /// `EndIslandFunction`: an index into `Program::simplex`.
    EndIslands { noise: usize },
    /// `DistanceToPointFunction`.
    DistanceToPoint { point: [i32; 3], metric: DistanceMetric },
}

/// `DistanceMetric`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceMetric {
    Euclidean,
    EuclideanSquared,
    Manhattan,
    Chebyshev,
}

impl DistanceMetric {
    pub fn parse(name: &str) -> Result<Self, String> {
        Ok(match name {
            "euclidean" => Self::Euclidean,
            "euclidean_squared" => Self::EuclideanSquared,
            "manhattan" => Self::Manhattan,
            "chebyshev" => Self::Chebyshev,
            other => return Err(format!("unknown distance metric {other}")),
        })
    }

    /// `DistanceMetric.compute(dx, dy, dz)` in float precision.
    pub fn compute(self, dx: f32, dy: f32, dz: f32) -> f32 {
        match self {
            Self::Euclidean => (dx * dx + dy * dy + dz * dz).sqrt(),
            Self::EuclideanSquared => dx * dx + dy * dy + dz * dz,
            Self::Manhattan => dx.abs() + dy.abs() + dz.abs(),
            Self::Chebyshev => crate::mth::max(crate::mth::max(dx.abs(), dy.abs()), dz.abs()),
        }
    }
}

/// `EndIslandFunction.getHeightValue`.
pub fn end_island_height(noise: &crate::temperature::Simplex, section_x: i32, section_z: i32) -> f32 {
    let (chunk_x, chunk_z) = (section_x / 2, section_z / 2);
    let (sub_x, sub_z) = (section_x % 2, section_z % 2);
    let mut doffs = -100.0f32;
    for xo in -12..=12 {
        for zo in -12..=12 {
            let tx = i64::from(chunk_x + xo);
            let tz = i64::from(chunk_z + zo);
            if tx * tx + tz * tz > 4096 && noise.get2(tx as f64, tz as f64) < -0.9 {
                let size = ((tx as f32).abs() * 3439.0 + (tz as f32).abs() * 147.0) % 13.0 + 9.0;
                let xd = (sub_x - xo * 2) as f32;
                let zd = (sub_z - zo * 2) as f32;
                let offs = crate::mth::clamp(100.0 - (xd * xd + zd * zd).sqrt() * size, -100.0, 80.0);
                doffs = crate::mth::max(doffs, offs);
            }
        }
    }
    doffs
}

/// All samplers and noise instances compiled for one world seed.
#[derive(Clone, Debug, Default)]
pub struct Program {
    pub samplers: Vec<Sampler>,
    pub noises: Vec<NoiseStack>,
    /// Cache ID to its caching sampler.
    pub cache_samplers: HashMap<usize, Id>,
    pub cache_count: usize,
    /// End island simplex noises.
    pub simplex: Vec<crate::temperature::Simplex>,
}

impl Program {
    pub fn push(&mut self, sampler: Sampler) -> Id {
        self.samplers.push(sampler);
        self.samplers.len() - 1
    }

    pub fn push_noise(&mut self, noise: NoiseStack) -> usize {
        self.noises.push(noise);
        self.noises.len() - 1
    }
}
