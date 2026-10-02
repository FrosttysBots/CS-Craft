//! `BlockPredicate` (vanilla `levelgen.blockpredicates`).

use super::blocks::{parse_state, BlockSet, FluidType};
use super::Ctx;
use crate::providers::Anchor;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::{BiomeId, BlockPos, BlockStateId, Registries, SupportType};
use serde_json::Value;

#[derive(Clone, Debug)]
pub enum BlockPredicate {
    True,
    MatchingBlocks { offset: BlockPos, blocks: BlockSet },
    MatchingBlockTag { offset: BlockPos, tag: minecraftoss_core::tags::TagId },
    MatchingFluids { offset: BlockPos, fluids: Vec<FluidType> },
    MatchingBiomes(Vec<BiomeId>),
    HasSturdyFace { offset: BlockPos, direction: Direction },
    Solid(BlockPos),
    Replaceable(BlockPos),
    WouldSurvive { offset: BlockPos, state: BlockStateId },
    InsideWorldBounds(BlockPos),
    Unobstructed,
    HeightRange(Anchor, Anchor),
    VolumeMatch { min: BlockPos, max: BlockPos, predicate: Box<BlockPredicate> },
    Not(Box<BlockPredicate>),
    AnyOf(Vec<BlockPredicate>),
    AllOf(Vec<BlockPredicate>),
}

pub fn parse_vec3(json: &Value) -> Result<BlockPos, String> {
    let a = json.as_array().filter(|a| a.len() == 3).ok_or_else(|| format!("invalid vector {json}"))?;
    let c = |i: usize| a[i].as_i64().map(|v| v as i32).ok_or_else(|| format!("invalid vector {json}"));
    Ok(BlockPos::new(c(0)?, c(1)?, c(2)?))
}

fn offset(json: &Value) -> Result<BlockPos, String> {
    json.get("offset").map_or(Ok(BlockPos::new(0, 0, 0)), parse_vec3)
}

pub fn parse_direction(json: &Value) -> Result<Direction, String> {
    json.as_str().and_then(Direction::from_name).ok_or_else(|| format!("invalid direction {json}"))
}

/// A biome holder set: one biome, a list, or `#tag`.
pub fn parse_biomes(registries: &Registries, json: &Value) -> Result<Vec<BiomeId>, String> {
    let one = |name: &str| -> Result<Vec<BiomeId>, String> {
        if let Some(tag) = name.strip_prefix('#') {
            let tag = registries.biome_tags.require(tag)?;
            Ok(registries.biomes.iter().filter(|(id, _)| registries.biome_tags.contains(tag, usize::from(id.0))).map(|(id, _)| id).collect())
        } else {
            Ok(vec![registries.biomes.id(name).ok_or_else(|| format!("unknown biome {name}"))?])
        }
    };
    match json {
        Value::String(name) => one(name),
        Value::Array(list) => {
            let mut out = Vec::new();
            for entry in list {
                out.extend(one(entry.as_str().ok_or("biome list entry is not a string")?)?);
            }
            Ok(out)
        }
        other => Err(format!("invalid biome set {other}")),
    }
}

impl BlockPredicate {
    pub fn parse(registries: &Registries, json: &Value) -> Result<Self, String> {
        let kind = json["type"].as_str().ok_or_else(|| format!("block predicate lacks a type: {json}"))?;
        let list = |key: &str| -> Result<Vec<Self>, String> {
            json[key].as_array().ok_or_else(|| format!("{kind} lacks {key}"))?.iter().map(|p| Self::parse(registries, p)).collect()
        };
        Ok(match kind.trim_start_matches("minecraft:") {
            "true" => Self::True,
            "matching_blocks" => Self::MatchingBlocks { offset: offset(json)?, blocks: BlockSet::parse(registries, &json["blocks"])? },
            "matching_block_tag" => Self::MatchingBlockTag {
                offset: offset(json)?,
                tag: registries.block_tags.require(json["tag"].as_str().ok_or("matching_block_tag lacks tag")?)?,
            },
            "matching_fluids" => Self::MatchingFluids { offset: offset(json)?, fluids: FluidType::parse_set(&json["fluids"])? },
            "matching_biomes" => Self::MatchingBiomes(parse_biomes(registries, &json["biomes"])?),
            "has_sturdy_face" => Self::HasSturdyFace { offset: offset(json)?, direction: parse_direction(&json["direction"])? },
            "solid" => Self::Solid(offset(json)?),
            "replaceable" => Self::Replaceable(offset(json)?),
            "would_survive" => Self::WouldSurvive { offset: offset(json)?, state: parse_state(registries, &json["state"])? },
            "inside_world_bounds" => Self::InsideWorldBounds(offset(json)?),
            "unobstructed" => Self::Unobstructed,
            "height_range" => Self::HeightRange(Anchor::parse(&json["min_inclusive"])?, Anchor::parse(&json["max_inclusive"])?),
            "volume_match" => Self::VolumeMatch {
                min: parse_vec3(&json["min"])?,
                max: parse_vec3(&json["max"])?,
                predicate: Box::new(Self::parse(registries, &json["match"])?),
            },
            "not" => Self::Not(Box::new(Self::parse(registries, &json["predicate"])?)),
            "any_of" => Self::AnyOf(list("predicates")?),
            "all_of" => Self::AllOf(list("predicates")?),
            other => return Err(format!("unknown block predicate {other}")),
        })
    }

    /// `BlockPredicate.matchesTag(tag)` built in code.
    pub fn tag(registries: &Registries, name: &str) -> Result<Self, String> {
        Ok(Self::MatchingBlockTag { offset: BlockPos::new(0, 0, 0), tag: registries.block_tags.require(name)? })
    }

    pub fn test(&self, ctx: &Ctx, pos: BlockPos) -> bool {
        let registries = ctx.registries();
        let at = |o: BlockPos| ctx.block(pos.offset(o.x, o.y, o.z));
        match self {
            Self::True | Self::Unobstructed => true,
            Self::MatchingBlocks { offset, blocks } => blocks.contains(registries, at(*offset)),
            Self::MatchingBlockTag { offset, tag } => registries.block_in_tag(at(*offset), *tag),
            Self::MatchingFluids { offset, fluids } => fluids.contains(&ctx.fluid(at(*offset))),
            Self::MatchingBiomes(biomes) => ctx.biome(pos).is_some_and(|b| biomes.contains(&b)),
            Self::HasSturdyFace { offset, direction } => registries.blocks.is_face_sturdy(at(*offset), *direction, SupportType::Full),
            Self::Solid(offset) => ctx.is_solid(at(*offset)),
            Self::Replaceable(offset) => ctx.is_replaceable(at(*offset)),
            Self::WouldSurvive { offset, state } => ctx.can_survive(*state, pos.offset(offset.x, offset.y, offset.z)),
            Self::InsideWorldBounds(offset) => !ctx.region.is_outside_build_height(pos.y + offset.y),
            Self::HeightRange(min, max) => {
                let bounds = ctx.lib.generation;
                pos.y >= min.resolve(&bounds) && pos.y <= max.resolve(&bounds)
            }
            Self::VolumeMatch { min, max, predicate } => {
                for ox in min.x..=max.x {
                    for oz in min.z..=max.z {
                        for oy in min.y..=max.y {
                            if !predicate.test(ctx, pos.offset(ox, oy, oz)) {
                                return false;
                            }
                        }
                    }
                }
                true
            }
            Self::Not(p) => !p.test(ctx, pos),
            Self::AnyOf(list) => list.iter().any(|p| p.test(ctx, pos)),
            Self::AllOf(list) => list.iter().all(|p| p.test(ctx, pos)),
        }
    }
}
