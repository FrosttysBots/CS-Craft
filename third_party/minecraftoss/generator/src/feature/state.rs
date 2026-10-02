//! `BlockStateProvider` (vanilla `feature.stateproviders`).

use super::blocks::{parse_state, BlockSet};
use super::predicate::{parse_direction, BlockPredicate};
use super::{Ctx, Library};
use crate::noise::{NoiseStack, NormalNoise, NormalNoiseParameters};
use crate::providers::{IntProvider, Weighted};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{AnyRandom, WorldgenRandom};
use minecraftoss_core::{BlockId, BlockPos, BlockStateId, Registries};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug)]
pub enum StateProvider {
    Simple(BlockStateId),
    Weighted(Weighted<BlockStateId>),
    Rotated { state: Arc<StateProvider>, direction: Option<Direction> },
    RandomizedInt { source: Arc<StateProvider>, property: String, values: IntProvider },
    RandomBlock(Vec<BlockId>),
    CopyProperties(Arc<StateProvider>),
    RuleBased { fallback: Option<Arc<StateProvider>>, rules: Vec<(BlockPredicate, Arc<StateProvider>)> },
    Noise { noise: NoiseStack, scale: f32, states: Vec<BlockStateId> },
    DualNoise { noise: NoiseStack, scale: f32, states: Vec<BlockStateId>, variety: (i32, i32), slow_noise: NoiseStack, slow_scale: f32 },
    NoiseThreshold { noise: NoiseStack, scale: f32, threshold: f32, high_chance: f32, default_state: BlockStateId, low_states: Vec<BlockStateId>, high_states: Vec<BlockStateId> },
}

fn states(registries: &Registries, json: &Value) -> Result<Vec<BlockStateId>, String> {
    json.as_array().ok_or_else(|| format!("invalid state list {json}"))?.iter().map(|s| parse_state(registries, s)).collect()
}

fn noise(json: &Value, seed: i64) -> Result<NoiseStack, String> {
    let parameters = NormalNoiseParameters::from_json(&json["noise"])?;
    Ok(NormalNoise::new(parameters).create(&mut AnyRandom::new(true, seed)))
}

impl StateProvider {
    /// A `Holder<BlockStateProvider>`: a registry name, a typed provider, or a full block state.
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Arc<Self>, String> {
        if let Some(name) = json.as_str() {
            return lib.state_provider(name);
        }
        Self::parse_direct(lib, json).map(Arc::new)
    }

    fn parse_direct(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let registries = lib.registries.clone();
        let Some(kind) = json.get("type").and_then(Value::as_str) else {
            return Ok(Self::Simple(parse_state(&registries, json)?));
        };
        let float = |key: &str| json[key].as_f64().map(|v| v as f32).ok_or_else(|| format!("{kind} lacks {key}"));
        let seed = || json["seed"].as_i64().ok_or_else(|| format!("{kind} lacks seed"));
        Ok(match kind.trim_start_matches("minecraft:") {
            "simple" => Self::Simple(parse_state(&registries, &json["state"])?),
            "weighted" => Self::Weighted(Weighted::parse(&json["entries"], |s| parse_state(&registries, s))?),
            "rotated" => Self::Rotated {
                state: Self::parse(lib, &json["state"])?,
                direction: json.get("direction").map(parse_direction).transpose()?,
            },
            "randomized_int" => Self::RandomizedInt {
                source: Self::parse(lib, &json["source"])?,
                property: json["property"].as_str().ok_or("randomized_int lacks property")?.to_owned(),
                values: IntProvider::parse(&json["values"])?,
            },
            "random_block" => Self::RandomBlock(match BlockSet::parse(&registries, &json["blocks"])? {
                BlockSet::Blocks(blocks) => blocks,
                BlockSet::Tag(_) => lib.block_tag_elements(json["blocks"].as_str().unwrap_or_default())?,
            }),
            "copy_properties" => Self::CopyProperties(Self::parse(lib, &json["source"])?),
            "rule_based" => {
                let fallback = json.get("fallback").map(|f| Self::parse(lib, f)).transpose()?;
                let mut rules = Vec::new();
                for rule in json["rules"].as_array().ok_or("rule_based lacks rules")? {
                    rules.push((BlockPredicate::parse(&registries, &rule["if_true"])?, Self::parse(lib, &rule["then"])?));
                }
                Self::RuleBased { fallback, rules }
            }
            "noise" => Self::Noise { noise: noise(json, seed()?)?, scale: float("scale")?, states: states(&registries, &json["states"])? },
            "dual_noise" => {
                let variety = &json["variety"];
                let (lo, hi) = match variety {
                    Value::Array(a) => (a[0].as_i64().unwrap_or(1) as i32, a[1].as_i64().unwrap_or(1) as i32),
                    Value::Number(n) => (n.as_i64().unwrap_or(1) as i32, n.as_i64().unwrap_or(1) as i32),
                    other => (
                        other["min_inclusive"].as_i64().ok_or("bad variety")? as i32,
                        other["max_inclusive"].as_i64().ok_or("bad variety")? as i32,
                    ),
                };
                let seed = seed()?;
                let slow = NormalNoiseParameters::from_json(&json["slow_noise"])?;
                Self::DualNoise {
                    noise: noise(json, seed)?,
                    scale: float("scale")?,
                    states: states(&registries, &json["states"])?,
                    variety: (lo, hi),
                    slow_noise: NormalNoise::new(slow).create(&mut AnyRandom::new(true, seed)),
                    slow_scale: float("slow_scale")?,
                }
            }
            "noise_threshold" => Self::NoiseThreshold {
                noise: noise(json, seed()?)?,
                scale: float("scale")?,
                threshold: float("threshold")?,
                high_chance: float("high_chance")?,
                default_state: parse_state(&registries, &json["default_state"])?,
                low_states: states(&registries, &json["low_states"])?,
                high_states: states(&registries, &json["high_states"])?,
            },
            other => return Err(format!("unknown block state provider {other}")),
        })
    }

    /// `getState`.
    pub fn get(&self, ctx: &Ctx, random: &mut WorldgenRandom, pos: BlockPos) -> BlockStateId {
        self.get_optional(ctx, random, pos).unwrap_or_else(|| ctx.block(pos))
    }

    /// `getOptionalState`: `None` where vanilla returns null.
    pub fn get_optional(&self, ctx: &Ctx, random: &mut WorldgenRandom, pos: BlockPos) -> Option<BlockStateId> {
        let blocks = &ctx.registries().blocks;
        Some(match self {
            Self::Simple(state) => *state,
            Self::Weighted(list) => *list.pick(random).expect("weighted provider is never empty"),
            Self::Rotated { state, direction } => {
                let direction = direction.unwrap_or_else(|| Direction::ALL[random.next_i32_bound(6) as usize]);
                let base = state.get(ctx, random, pos);
                let rotated = try_with(ctx.registries(), base, "axis", direction.axis().name());
                let rotated = try_with(ctx.registries(), rotated, "facing", direction.name());
                // HORIZONTAL_FACING shares the name `facing`; the value set differs.
                rotated
            }
            Self::RandomizedInt { source, property, values } => {
                let base = source.get(ctx, random, pos);
                if blocks.property(base, property).is_none() {
                    return Some(base);
                }
                let value = values.sample(random);
                blocks.with_property(base, property, &value.to_string()).unwrap_or(base)
            }
            Self::RandomBlock(list) => {
                if list.is_empty() {
                    return None;
                }
                blocks.block(list[random.next_i32_bound(list.len() as i32) as usize]).default_state()
            }
            Self::CopyProperties(source) => {
                let state = source.get(ctx, random, pos);
                copy_properties(ctx.registries(), state, ctx.block(pos))
            }
            Self::RuleBased { fallback, rules } => {
                for (predicate, then) in rules {
                    if predicate.test(ctx, pos) {
                        if let Some(state) = then.get_optional(ctx, random, pos) {
                            return Some(state);
                        }
                    }
                }
                return fallback.as_ref().and_then(|f| f.get_optional(ctx, random, pos));
            }
            Self::Noise { noise, scale, states } => noise_state(states, noise_value(noise, pos, f64::from(*scale))),
            Self::DualNoise { noise, scale, states, variety, slow_noise, slow_scale } => {
                let slow = |p: BlockPos| slow_noise.get(f64::from(p.x as f32 * slow_scale), f64::from(p.y as f32 * slow_scale), f64::from(p.z as f32 * slow_scale));
                let variety_noise = f64::from(slow(pos));
                let local = crate::mth::clamped_map(variety_noise, -1.0, 1.0, f64::from(variety.0), f64::from(variety.1 + 1)) as i32;
                let mut possible = Vec::with_capacity(local.max(0) as usize);
                for i in 0..local {
                    possible.push(noise_state(states, slow(pos.offset(i * 54545, 0, i * 34234))));
                }
                noise_state(&possible, noise_value(noise, pos, f64::from(*scale)))
            }
            Self::NoiseThreshold { noise, scale, threshold, high_chance, default_state, low_states, high_states } => {
                let local = f64::from(noise_value(noise, pos, f64::from(*scale)));
                if local < f64::from(*threshold) {
                    low_states[random.next_i32_bound(low_states.len() as i32) as usize]
                } else if random.next_f32() < *high_chance {
                    high_states[random.next_i32_bound(high_states.len() as i32) as usize]
                } else {
                    *default_state
                }
            }
        })
    }
}

fn noise_value(noise: &NoiseStack, pos: BlockPos, scale: f64) -> f32 {
    noise.get(f64::from(pos.x) * scale, f64::from(pos.y) * scale, f64::from(pos.z) * scale)
}

/// `NoiseProvider.getRandomState(states, noiseValue)`.
fn noise_state(states: &[BlockStateId], value: f32) -> BlockStateId {
    let placement = ((1.0 + value) / 2.0).clamp(0.0, 0.9999);
    states[(placement * states.len() as f32) as usize]
}

/// `StateHolder.trySetValue`: unchanged when the block lacks the property or value.
pub fn try_with(registries: &Registries, state: BlockStateId, property: &str, value: &str) -> BlockStateId {
    registries.blocks.with_property(state, property, value).unwrap_or(state)
}

/// `BlockState.withPropertiesOf(source)`: copies every property both blocks share.
pub fn copy_properties(registries: &Registries, state: BlockStateId, source: BlockStateId) -> BlockStateId {
    let blocks = &registries.blocks;
    let mut out = state;
    for property in blocks.block(blocks.block_of(source)).properties() {
        if let Some(value) = blocks.property(source, &property.name) {
            out = blocks.with_property(out, &property.name, value).unwrap_or(out);
        }
    }
    out
}
