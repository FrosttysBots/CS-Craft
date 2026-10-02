//! Value providers, vertical anchors and `Mth` trigonometry used by carvers
//! and material rules (vanilla `util.valueproviders`, `heightproviders`,
//! `VerticalAnchor`, `Mth.sin`/`cos`).

use minecraftoss_core::random::RandomSource;
use serde_json::Value;
use std::sync::OnceLock;

/// `WorldGenerationContext`: generation bounds and sea level.
#[derive(Clone, Copy, Debug)]
pub struct GenContext {
    pub min_y: i32,
    pub depth: i32,
    pub sea_level: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Absolute(i32),
    AboveBottom(i32),
    BelowTop(i32),
    RelativeToSeaLevel(i32),
}

impl Anchor {
    pub fn parse(json: &Value) -> Result<Self, String> {
        let get = |k: &str| json.get(k).and_then(Value::as_i64).map(|v| v as i32);
        if let Some(v) = get("absolute") {
            Ok(Self::Absolute(v))
        } else if let Some(v) = get("above_bottom") {
            Ok(Self::AboveBottom(v))
        } else if let Some(v) = get("below_top") {
            Ok(Self::BelowTop(v))
        } else if let Some(v) = get("relative_to_sea_level") {
            Ok(Self::RelativeToSeaLevel(v))
        } else {
            Err(format!("invalid vertical anchor {json}"))
        }
    }

    pub fn resolve(self, c: &GenContext) -> i32 {
        match self {
            Self::Absolute(y) => y,
            Self::AboveBottom(o) => c.min_y + o,
            Self::BelowTop(o) => c.depth - 1 + c.min_y - o,
            Self::RelativeToSeaLevel(o) => c.sea_level + o,
        }
    }
}

/// `Mth.randomBetweenInclusive`.
pub fn random_between_inclusive(random: &mut impl RandomSource, min: i32, max: i32) -> i32 {
    random.next_i32_bound(max - min + 1) + min
}

/// `Mth.nextInt(random, min, max)`: `min` when the range is empty.
pub fn next_int_between(random: &mut impl RandomSource, min: i32, max: i32) -> i32 {
    if min >= max { min } else { random.next_i32_bound(max - min + 1) + min }
}

/// A `WeightedList` in data-pack form: `[{"data": ..., "weight": n}, ...]`.
#[derive(Clone, Debug)]
pub struct Weighted<T> {
    entries: Vec<(T, i32)>,
    total: i32,
}

impl<T> Weighted<T> {
    /// The entries in list order.
    pub fn items(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|(item, _)| item)
    }

    pub fn parse(json: &Value, item: impl Fn(&Value) -> Result<T, String>) -> Result<Self, String> {
        let list = json.as_array().ok_or_else(|| format!("weighted list is not an array: {json}"))?;
        let mut entries = Vec::with_capacity(list.len());
        for entry in list {
            let weight = entry["weight"].as_i64().ok_or("weighted entry lacks weight")? as i32;
            entries.push((item(&entry["data"])?, weight));
        }
        let total = entries.iter().map(|(_, w)| w).sum();
        Ok(Self { entries, total })
    }

    pub fn from_entries(entries: Vec<(T, i32)>) -> Self {
        let total = entries.iter().map(|(_, w)| w).sum();
        Self { entries, total }
    }

    pub fn entries(&self) -> &[(T, i32)] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// `WeightedList.getRandom`: one draw over the total weight.
    pub fn pick(&self, random: &mut impl RandomSource) -> Option<&T> {
        if self.total == 0 {
            return None;
        }
        let mut selection = random.next_i32_bound(self.total);
        for (value, weight) in &self.entries {
            if selection < *weight {
                return Some(value);
            }
            selection -= weight;
        }
        None
    }
}

/// `Mth.normal`: the mean plus a Gaussian draw times the deviation.
pub fn normal(random: &mut impl RandomSource, mean: f32, deviation: f32) -> f32 {
    mean + random.next_gaussian() as f32 * deviation
}

/// `Mth.randomBetween`.
pub fn random_between(random: &mut impl RandomSource, min: f32, max_exclusive: f32) -> f32 {
    random.next_f32() * (max_exclusive - min) + min
}

#[derive(Clone, Debug)]
pub enum IntProvider {
    Constant(i32),
    Uniform(i32, i32),
    BiasedToBottom(i32, i32),
    VeryBiasedToBottom(i32, i32),
    Trapezoid { min: i32, max: i32, plateau: i32 },
    Clamped { source: Box<IntProvider>, min: i32, max: i32 },
    ClampedNormal { mean: f32, deviation: f32, min: i32, max: i32 },
    WeightedList(Weighted<IntProvider>),
}

impl IntProvider {
    pub fn parse(json: &Value) -> Result<Self, String> {
        if let Some(v) = json.as_i64() {
            return Ok(Self::Constant(v as i32));
        }
        let int = |k: &str| json.get(k).and_then(Value::as_i64).map(|v| v as i32).ok_or_else(|| format!("int provider lacks {k}"));
        match json["type"].as_str().unwrap_or_default() {
            "minecraft:constant" => Ok(Self::Constant(int("value")?)),
            "minecraft:uniform" => Ok(Self::Uniform(int("min_inclusive")?, int("max_inclusive")?)),
            "minecraft:biased_to_bottom" => Ok(Self::BiasedToBottom(int("min_inclusive")?, int("max_inclusive")?)),
            "minecraft:very_biased_to_bottom" => Ok(Self::VeryBiasedToBottom(int("min_inclusive")?, int("max_inclusive")?)),
            "minecraft:trapezoid" => Ok(Self::Trapezoid { min: int("min").or_else(|_| int("min_inclusive"))?, max: int("max").or_else(|_| int("max_inclusive"))?, plateau: int("plateau").unwrap_or(0) }),
            "minecraft:clamped" => Ok(Self::Clamped { source: Box::new(Self::parse(&json["source"])?), min: int("min_inclusive")?, max: int("max_inclusive")? }),
            "minecraft:clamped_normal" => {
                let float = |k: &str| json.get(k).and_then(Value::as_f64).map(|v| v as f32).ok_or_else(|| format!("clamped_normal lacks {k}"));
                Ok(Self::ClampedNormal { mean: float("mean")?, deviation: float("deviation")?, min: int("min_inclusive")?, max: int("max_inclusive")? })
            }
            "minecraft:weighted_list" => Ok(Self::WeightedList(Weighted::parse(&json["distribution"], Self::parse)?)),
            other => Err(format!("unsupported int provider {other}")),
        }
    }

    /// `IntProvider.minInclusive`.
    pub fn min_inclusive(&self) -> i32 {
        match self {
            Self::Constant(v) => *v,
            Self::Uniform(min, _) | Self::BiasedToBottom(min, _) | Self::VeryBiasedToBottom(min, _) => *min,
            Self::Trapezoid { min, .. } | Self::ClampedNormal { min, .. } => *min,
            Self::Clamped { source, min, .. } => (*min).max(source.min_inclusive()),
            Self::WeightedList(list) => list.entries().iter().map(|(p, _)| p.min_inclusive()).min().unwrap_or(i32::MAX),
        }
    }

    /// `IntProvider.maxInclusive`.
    pub fn max_inclusive(&self) -> i32 {
        match self {
            Self::Constant(v) => *v,
            Self::Uniform(_, max) | Self::BiasedToBottom(_, max) | Self::VeryBiasedToBottom(_, max) => *max,
            Self::Trapezoid { max, .. } | Self::ClampedNormal { max, .. } => *max,
            Self::Clamped { source, max, .. } => (*max).min(source.max_inclusive()),
            Self::WeightedList(list) => list.entries().iter().map(|(p, _)| p.max_inclusive()).max().unwrap_or(i32::MIN),
        }
    }

    pub fn sample(&self, random: &mut impl RandomSource) -> i32 {
        match *self {
            Self::Constant(v) => v,
            Self::Uniform(min, max) => random_between_inclusive(random, min, max),
            Self::Trapezoid { min, max, plateau } => {
                if plateau == 0 && max == -min {
                    return random.next_i32_bound(max + 1) - random.next_i32_bound(max + 1);
                }
                let range = max - min;
                if plateau == range {
                    return random_between_inclusive(random, min, max);
                }
                let plateau_start = (range - plateau) / 2;
                let plateau_end = range - plateau_start;
                min + random_between_inclusive(random, 0, plateau_end) + random_between_inclusive(random, 0, plateau_start)
            }
            Self::Clamped { ref source, min, max } => source.sample(random).clamp(min, max),
            Self::ClampedNormal { mean, deviation, min, max } => crate::mth::clamp(normal(random, mean, deviation), min as f32, max as f32) as i32,
            Self::WeightedList(ref list) => list.pick(random).expect("weighted int list has entries").sample(random),
            Self::BiasedToBottom(min, max) => {
                let bound = random.next_i32_bound(max - min + 1) + 1;
                min + random.next_i32_bound(bound)
            }
            Self::VeryBiasedToBottom(min, max) => {
                let a = random.next_i32_bound(max - min + 1) + 1;
                let b = random.next_i32_bound(a) + 1;
                min + random.next_i32_bound(b)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum FloatProvider {
    Constant(f32),
    Uniform(f32, f32),
    Trapezoid { min: f32, max: f32, plateau: f32 },
    ClampedNormal { mean: f32, deviation: f32, min: f32, max: f32 },
}

impl FloatProvider {
    pub fn parse(json: &Value) -> Result<Self, String> {
        if let Some(v) = json.as_f64() {
            return Ok(Self::Constant(v as f32));
        }
        let float = |k: &str| json.get(k).and_then(Value::as_f64).map(|v| v as f32).ok_or_else(|| format!("float provider lacks {k}"));
        match json["type"].as_str().unwrap_or_default() {
            "minecraft:constant" => Ok(Self::Constant(float("value")?)),
            "minecraft:uniform" => Ok(Self::Uniform(float("min_inclusive")?, float("max_exclusive")?)),
            "minecraft:trapezoid" => Ok(Self::Trapezoid { min: float("min")?, max: float("max")?, plateau: float("plateau")? }),
            "minecraft:clamped_normal" => Ok(Self::ClampedNormal { mean: float("mean")?, deviation: float("deviation")?, min: float("min")?, max: float("max")? }),
            other => Err(format!("unsupported float provider {other}")),
        }
    }

    pub fn sample(&self, random: &mut impl RandomSource) -> f32 {
        match *self {
            Self::Constant(v) => v,
            Self::Uniform(min, max) => random_between(random, min, max),
            Self::Trapezoid { min, max, plateau } => {
                let range = max - min;
                let plateau_start = (range - plateau) / 2.0;
                let plateau_end = range - plateau_start;
                min + random.next_f32() * plateau_end + random.next_f32() * plateau_start
            }
            Self::ClampedNormal { mean, deviation, min, max } => crate::mth::clamp(normal(random, mean, deviation), min, max),
        }
    }
}

#[derive(Clone, Debug)]
pub enum HeightProvider {
    Constant(Anchor),
    Uniform(Anchor, Anchor),
    Trapezoid(Anchor, Anchor, i32),
    BiasedToBottom(Anchor, Anchor, i32),
    VeryBiasedToBottom(Anchor, Anchor, i32),
    WeightedList(Weighted<HeightProvider>),
}

impl HeightProvider {
    pub fn parse(json: &Value) -> Result<Self, String> {
        match json.get("type").and_then(Value::as_str) {
            None => Ok(Self::Constant(Anchor::parse(json)?)),
            Some("minecraft:constant") => Ok(Self::Constant(Anchor::parse(&json["value"])?)),
            Some("minecraft:uniform") => Ok(Self::Uniform(Anchor::parse(&json["min_inclusive"])?, Anchor::parse(&json["max_inclusive"])?)),
            Some(kind) if matches!(kind, "minecraft:trapezoid" | "minecraft:biased_to_bottom" | "minecraft:very_biased_to_bottom") => {
                let (min, max) = (Anchor::parse(&json["min_inclusive"])?, Anchor::parse(&json["max_inclusive"])?);
                Ok(match kind {
                    "minecraft:trapezoid" => Self::Trapezoid(min, max, json["plateau"].as_i64().unwrap_or(0) as i32),
                    "minecraft:biased_to_bottom" => Self::BiasedToBottom(min, max, json["inner"].as_i64().unwrap_or(1) as i32),
                    _ => Self::VeryBiasedToBottom(min, max, json["inner"].as_i64().unwrap_or(1) as i32),
                })
            }
            Some("minecraft:weighted_list") => Ok(Self::WeightedList(Weighted::parse(&json["distribution"], Self::parse)?)),
            Some(other) => Err(format!("unsupported height provider {other}")),
        }
    }

    pub fn sample(&self, random: &mut impl RandomSource, context: &GenContext) -> i32 {
        match self {
            Self::Constant(a) => a.resolve(context),
            Self::Uniform(lo, hi) => {
                let (min, max) = (lo.resolve(context), hi.resolve(context));
                if min > max { min } else { random_between_inclusive(random, min, max) }
            }
            Self::Trapezoid(lo, hi, plateau) => {
                let (min, max) = (lo.resolve(context), hi.resolve(context));
                if min > max {
                    return min;
                }
                let range = max - min;
                if *plateau >= range {
                    return random_between_inclusive(random, min, max);
                }
                let plateau_start = (range - plateau) / 2;
                let plateau_end = range - plateau_start;
                min + random_between_inclusive(random, 0, plateau_end) + random_between_inclusive(random, 0, plateau_start)
            }
            Self::BiasedToBottom(lo, hi, inner) => {
                let (min, max) = (lo.resolve(context), hi.resolve(context));
                if max - min - inner + 1 <= 0 {
                    return min;
                }
                let limit = random.next_i32_bound(max - min - inner + 1);
                random.next_i32_bound(limit + inner) + min
            }
            Self::VeryBiasedToBottom(lo, hi, inner) => {
                let (min, max) = (lo.resolve(context), hi.resolve(context));
                if max - min - inner + 1 <= 0 {
                    return min;
                }
                let upper = next_int_between(random, min + inner, max);
                let biased = next_int_between(random, min, upper - 1);
                next_int_between(random, min, biased - 1 + inner)
            }
            Self::WeightedList(list) => list.pick(random).expect("weighted height list has entries").sample(random, context),
        }
    }
}

fn sin_table() -> &'static [f32; 65536] {
    static TABLE: OnceLock<Box<[f32; 65536]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = Box::new([0.0f32; 65536]);
        for (i, v) in t.iter_mut().enumerate() {
            *v = (i as f64 / 10_430.378_350_470_453).sin() as f32;
        }
        t
    })
}

/// `Mth.sin(double)`: vanilla's lookup-table sine.
pub fn sin(value: f64) -> f32 {
    sin_table()[((value * 10_430.378_350_470_453) as i64 & 0xffff) as usize]
}

/// `Mth.cos(double)`.
pub fn cos(value: f64) -> f32 {
    sin_table()[((value * 10_430.378_350_470_453 + 16384.0) as i64 & 0xffff) as usize]
}
