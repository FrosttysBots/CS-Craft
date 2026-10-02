//! Placement modifiers and `FeaturePlacer` (vanilla `levelgen.placement`).

use super::predicate::{parse_direction, parse_vec3, BlockPredicate};
use super::{Ctx, FeatureId, Library, PlacedId};
use crate::providers::{HeightProvider, IntProvider};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::BlockPos;
use serde_json::Value;

#[derive(Debug)]
pub enum Modifier {
    Biome,
    BlockPredicateFilter(BlockPredicate),
    Rarity(i32),
    RandomChance(f32),
    SurfaceRelativeThreshold { heightmap: HeightmapKind, min: i32, max: i32 },
    SurfaceWaterDepth(i32),
    Count(IntProvider),
    NoiseBasedCount { ratio: i32, factor: f64, offset: f64 },
    NoiseThresholdCount { level: f64, below: i32, above: i32 },
    CountOnEveryLayer(IntProvider),
    Cuboid { xz_size: IntProvider, y_size: IntProvider, edges: bool, interior: bool },
    EnvironmentScan { direction: Direction, target: BlockPredicate, allowed: BlockPredicate, max_steps: i32 },
    Heightmap(HeightmapKind),
    HeightRange(HeightProvider),
    InSquare,
    Offset(IntProvider, IntProvider, IntProvider),
    RandomlySelected(Vec<Modifier>),
    Fixed(Vec<BlockPos>),
}

pub fn parse_heightmap(json: &Value) -> Result<HeightmapKind, String> {
    Ok(match json.as_str().ok_or("heightmap is not a string")? {
        "WORLD_SURFACE_WG" => HeightmapKind::WorldSurfaceWg,
        "WORLD_SURFACE" => HeightmapKind::WorldSurface,
        "OCEAN_FLOOR_WG" => HeightmapKind::OceanFloorWg,
        "OCEAN_FLOOR" => HeightmapKind::OceanFloor,
        "MOTION_BLOCKING" => HeightmapKind::MotionBlocking,
        "MOTION_BLOCKING_NO_LEAVES" => HeightmapKind::MotionBlockingNoLeaves,
        other => return Err(format!("unknown heightmap {other}")),
    })
}

impl Modifier {
    pub fn parse(lib: &Library, json: &Value) -> Result<Self, String> {
        let registries = &lib.registries;
        let kind = json["type"].as_str().ok_or_else(|| format!("placement modifier lacks a type: {json}"))?;
        let int = |key: &str| json[key].as_i64().map(|v| v as i32).ok_or_else(|| format!("{kind} lacks {key}"));
        let float = |key: &str| json[key].as_f64().ok_or_else(|| format!("{kind} lacks {key}"));
        Ok(match kind.trim_start_matches("minecraft:") {
            "biome" => Self::Biome,
            "block_predicate_filter" => Self::BlockPredicateFilter(BlockPredicate::parse(registries, &json["predicate"])?),
            "rarity_filter" => Self::Rarity(int("chance")?),
            "random_chance" => Self::RandomChance(float("chance")? as f32),
            "surface_relative_threshold_filter" => Self::SurfaceRelativeThreshold {
                heightmap: parse_heightmap(&json["heightmap"])?,
                min: json["min_inclusive"].as_i64().map_or(i32::MIN, |v| v as i32),
                max: json["max_inclusive"].as_i64().map_or(i32::MAX, |v| v as i32),
            },
            "surface_water_depth_filter" => Self::SurfaceWaterDepth(int("max_water_depth")?),
            "count" => Self::Count(IntProvider::parse(&json["count"])?),
            "noise_based_count" => Self::NoiseBasedCount {
                ratio: int("noise_to_count_ratio")?,
                factor: float("noise_factor")?,
                offset: json["noise_offset"].as_f64().unwrap_or(0.0),
            },
            "noise_threshold_count" => Self::NoiseThresholdCount { level: float("noise_level")?, below: int("below_noise")?, above: int("above_noise")? },
            "count_on_every_layer" => Self::CountOnEveryLayer(IntProvider::parse(&json["count"])?),
            "cuboid" => Self::Cuboid {
                xz_size: IntProvider::parse(&json["xz_size"])?,
                y_size: IntProvider::parse(&json["y_size"])?,
                edges: json["include_edges"].as_bool().unwrap_or(true),
                interior: json["include_interior"].as_bool().unwrap_or(true),
            },
            "environment_scan" => Self::EnvironmentScan {
                direction: parse_direction(&json["direction_of_search"])?,
                target: BlockPredicate::parse(registries, &json["target_condition"])?,
                allowed: match json.get("allowed_search_condition") {
                    Some(p) => BlockPredicate::parse(registries, p)?,
                    None => BlockPredicate::True,
                },
                max_steps: int("max_steps")?,
            },
            "heightmap" => Self::Heightmap(parse_heightmap(&json["heightmap"])?),
            "height_range" => Self::HeightRange(HeightProvider::parse(&json["height"])?),
            "in_square" => Self::InSquare,
            "offset" => Self::Offset(IntProvider::parse(&json["x"])?, IntProvider::parse(&json["y"])?, IntProvider::parse(&json["z"])?),
            "randomly_selected" => Self::RandomlySelected(
                json["placements"].as_array().ok_or("randomly_selected lacks placements")?.iter().map(|m| Self::parse(lib, m)).collect::<Result<_, _>>()?,
            ),
            "fixed_placement" => Self::Fixed(json["positions"].as_array().ok_or("fixed_placement lacks positions")?.iter().map(parse_vec3).collect::<Result<_, _>>()?),
            other => return Err(format!("unknown placement modifier {other}")),
        })
    }

    /// `PlacementModifier.modify`, appending outputs in vanilla order.
    fn modify(&self, ctx: &Ctx, random: &mut WorldgenRandom, top: Option<PlacedId>, pos: BlockPos, out: &mut Vec<BlockPos>) {
        let lib = ctx.lib;
        match self {
            Self::Biome => {
                let top = top.expect("biome filter on a feature placed without a biome check");
                if ctx.biome(pos).is_some_and(|biome| lib.biome_has_feature(biome, top)) {
                    out.push(pos);
                }
            }
            Self::BlockPredicateFilter(predicate) => {
                if predicate.test(ctx, pos) {
                    out.push(pos);
                }
            }
            Self::Rarity(chance) => {
                if random.next_f32() < 1.0 / *chance as f32 {
                    out.push(pos);
                }
            }
            Self::RandomChance(chance) => {
                if random.next_f32() < *chance {
                    out.push(pos);
                }
            }
            Self::SurfaceRelativeThreshold { heightmap, min, max } => {
                let surface = i64::from(ctx.height(*heightmap, pos.x, pos.z));
                let y = i64::from(pos.y);
                if surface + i64::from(*min) <= y && y <= surface + i64::from(*max) {
                    out.push(pos);
                }
            }
            Self::SurfaceWaterDepth(max) => {
                let floor = ctx.height(HeightmapKind::OceanFloor, pos.x, pos.z);
                let surface = ctx.height(HeightmapKind::WorldSurface, pos.x, pos.z);
                if surface - floor <= *max {
                    out.push(pos);
                }
            }
            Self::Count(count) => {
                let n = count.sample(random);
                out.extend(std::iter::repeat_n(pos, n.max(0) as usize));
            }
            Self::NoiseBasedCount { ratio, factor, offset } => {
                let noise = f64::from(lib.temperature.info(f64::from(pos.x) / factor, f64::from(pos.z) / factor));
                let n = ((noise + offset) * f64::from(*ratio)).ceil() as i32;
                out.extend(std::iter::repeat_n(pos, n.max(0) as usize));
            }
            Self::NoiseThresholdCount { level, below, above } => {
                let noise = f64::from(lib.temperature.info(f64::from(pos.x) / 200.0, f64::from(pos.z) / 200.0));
                let n = if noise < *level { *below } else { *above };
                out.extend(std::iter::repeat_n(pos, n.max(0) as usize));
            }
            Self::CountOnEveryLayer(count) => {
                let mut layer = 0;
                loop {
                    let mut found = false;
                    let mut i = 0;
                    while i < count.sample(random) {
                        let x = random.next_i32_bound(16) + pos.x;
                        let z = random.next_i32_bound(16) + pos.z;
                        let start = ctx.height(HeightmapKind::MotionBlocking, x, z);
                        if let Some(y) = on_ground_y(ctx, x, start, z, layer) {
                            out.push(BlockPos::new(x, y, z));
                            found = true;
                        }
                        i += 1;
                    }
                    layer += 1;
                    if !found {
                        break;
                    }
                }
            }
            Self::Cuboid { xz_size, y_size, edges, interior } => {
                let height = y_size.sample(random);
                let width = xz_size.sample(random);
                let length = xz_size.sample(random);
                for x in 0..=width {
                    for y in 0..=height {
                        for z in 0..=length {
                            let ex = x == 0 || x == width;
                            let ey = y == 0 || y == height;
                            let ez = z == 0 || z == length;
                            let keep = (*edges || !ex || !ey) && (*edges || !ez || !ey) && (*edges || !ex || !ez) && (*interior || ex || ey || ez);
                            if keep {
                                out.push(pos.offset(x, y, z));
                            }
                        }
                    }
                }
            }
            Self::EnvironmentScan { direction, target, allowed, max_steps } => {
                let mut p = pos;
                if !allowed.test(ctx, p) {
                    return;
                }
                for _ in 0..*max_steps {
                    if target.test(ctx, p) {
                        out.push(p);
                        return;
                    }
                    p = p.relative(*direction, 1);
                    if ctx.region.is_outside_build_height(p.y) {
                        return;
                    }
                    if !allowed.test(ctx, p) {
                        break;
                    }
                }
                if target.test(ctx, p) {
                    out.push(p);
                }
            }
            Self::Heightmap(kind) => {
                let height = ctx.height(*kind, pos.x, pos.z);
                if height > ctx.region.min_y() {
                    out.push(BlockPos::new(pos.x, height, pos.z));
                }
            }
            Self::HeightRange(height) => out.push(pos.at_y(height.sample(random, &lib.generation))),
            Self::InSquare => {
                let x = random.next_i32_bound(16) + pos.x;
                let z = random.next_i32_bound(16) + pos.z;
                out.push(BlockPos::new(x, pos.y, z));
            }
            Self::Offset(x, y, z) => {
                let (dx, dy, dz) = (x.sample(random), y.sample(random), z.sample(random));
                out.push(pos.offset(dx, dy, dz));
            }
            Self::RandomlySelected(list) => {
                let choice = &list[random.next_i32_bound(list.len() as i32) as usize];
                choice.modify(ctx, random, top, pos, out);
            }
            Self::Fixed(positions) => {
                let (cx, cz) = (pos.x >> 4, pos.z >> 4);
                out.extend(positions.iter().filter(|p| p.x >> 4 == cx && p.z >> 4 == cz));
            }
        }
    }
}

/// `CountOnEveryLayerPlacement.findOnGroundYPosition`.
fn on_ground_y(ctx: &Ctx, x: i32, start: i32, z: i32, layer: i32) -> Option<i32> {
    let registries = ctx.registries();
    let empty = |s| registries.blocks.is_air(s) || s == ctx.lib.blocks.water || s == ctx.lib.blocks.lava;
    let mut current_layer = 0;
    let mut current = ctx.block(BlockPos::new(x, start, z));
    let mut y = start;
    while y >= ctx.region.min_y() + 1 {
        let below = ctx.block(BlockPos::new(x, y - 1, z));
        if !empty(below) && empty(current) && below != ctx.lib.blocks.bedrock {
            if current_layer == layer {
                return Some(y);
            }
            current_layer += 1;
        }
        current = below;
        y -= 1;
    }
    None
}

/// A placed feature: a configured feature and its placement.
#[derive(Debug)]
pub struct Placed {
    pub feature: FeatureId,
    pub placement: Vec<Modifier>,
}

/// `FeaturePlacer.place`: modifiers run depth first; the last modifier's
/// outputs are placed in order once all of them are computed.
pub fn place(ctx: &mut Ctx, random: &mut WorldgenRandom, placed: PlacedId, origin: BlockPos, biome_check: bool) -> bool {
    let lib = ctx.lib;
    let entry = lib.placed(placed);
    let top = biome_check.then_some(placed);
    if entry.placement.is_empty() {
        return super::place_feature(ctx, random, entry.feature, origin);
    }
    let mut placed_any = false;
    let mut stack = vec![(origin, 0usize)];
    let mut modified = Vec::new();
    while let Some((pos, index)) = stack.pop() {
        entry.placement[index].modify(ctx, random, top, pos, &mut modified);
        let next = index + 1;
        if next < entry.placement.len() {
            stack.extend(modified.iter().rev().map(|&p| (p, next)));
        } else {
            for &p in &modified {
                placed_any |= super::place_feature(ctx, random, entry.feature, p);
            }
        }
        modified.clear();
    }
    placed_any
}
