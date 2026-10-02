//! Structure set placement (vanilla `StructurePlacement` types and
//! `ChunkGeneratorStructureState`): which chunks may start a structure.
//!
//! Source-informed from the pinned 26.3 JAR. Frequency reducers keep their
//! legacy seeding quirks (the default reducer passes the salt as the X
//! coordinate). Concentric ring positions resolve their biome search only
//! when a query comes within reach of one.

use crate::terrain::TerrainGenerator;
use minecraftoss_core::random::LegacyRandom;
use minecraftoss_core::{BiomeId, BlockPos, ChunkPos};
use serde_json::Value;
use std::sync::OnceLock;

/// `WorldgenRandom.setLargeFeatureSeed` over a legacy source.
pub fn large_feature_seed(random: &mut LegacyRandom, seed: i64, x: i32, z: i32) {
    random.set_seed(seed);
    let x_scale = random.next_i64();
    let z_scale = random.next_i64();
    random.set_seed(i64::from(x).wrapping_mul(x_scale) ^ i64::from(z).wrapping_mul(z_scale) ^ seed);
}

/// `WorldgenRandom.setLargeFeatureWithSalt`.
pub fn large_feature_with_salt(random: &mut LegacyRandom, seed: i64, x: i32, z: i32, salt: i32) {
    let s = i64::from(x).wrapping_mul(341_873_128_712).wrapping_add(i64::from(z).wrapping_mul(132_897_987_541)).wrapping_add(seed).wrapping_add(i64::from(salt));
    random.set_seed(s);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpreadType {
    Linear,
    Triangular,
}

impl SpreadType {
    fn evaluate(self, random: &mut LegacyRandom, limit: i32) -> i32 {
        match self {
            Self::Linear => random.next_i32_bound(limit),
            Self::Triangular => (random.next_i32_bound(limit) + random.next_i32_bound(limit)) / 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrequencyReduction {
    Default,
    /// `legacyPillagerOutpostReducer`.
    LegacyType1,
    /// `legacyArbitrarySaltProbabilityReducer`.
    LegacyType2,
    /// `legacyProbabilityReducerWithDouble`.
    LegacyType3,
}

impl FrequencyReduction {
    fn should_generate(self, seed: i64, salt: i32, x: i32, z: i32, probability: f32) -> bool {
        let mut random = LegacyRandom::new(0);
        match self {
            Self::Default => {
                // The salt and coordinates are passed shifted by one parameter.
                large_feature_with_salt(&mut random, seed, salt, x, z);
                random.next_f32() < probability
            }
            Self::LegacyType1 => {
                let (cx, cz) = (x >> 4, z >> 4);
                random.set_seed(i64::from(cx ^ (cz << 4)) ^ seed);
                random.next_i32();
                random.next_i32_bound((1.0 / probability) as i32) == 0
            }
            Self::LegacyType2 => {
                large_feature_with_salt(&mut random, seed, x, z, 10_387_320);
                random.next_f32() < probability
            }
            Self::LegacyType3 => {
                large_feature_seed(&mut random, seed, x, z);
                random.next_f64() < f64::from(probability)
            }
        }
    }
}

/// One concentric ring position: its initial chunk and the biome search
/// that may move it (up to seven chunks away).
#[derive(Debug)]
pub struct RingSlot {
    initial: ChunkPos,
    search_seed: i64,
    resolved: OnceLock<ChunkPos>,
}

#[derive(Debug)]
pub enum Kind {
    RandomSpread { spacing: i32, separation: i32, spread: SpreadType },
    ConcentricRings { preferred: Vec<bool>, slots: Vec<RingSlot> },
    DimensionOrigin,
}

/// One structure set's placement.
#[derive(Debug)]
pub struct Placement {
    pub kind: Kind,
    pub locate_offset: [i32; 3],
    reduction: FrequencyReduction,
    frequency: f32,
    salt: i32,
    /// Another set (by index into all sets) and its chunk range.
    pub exclusion: Option<(usize, i32)>,
}

/// What a placement query needs from the dimension.
pub struct PlacementContext<'a> {
    pub seed: i64,
    pub terrain: &'a TerrainGenerator,
    /// Every structure set's placement, for exclusion zones.
    pub all: &'a [Placement],
}

impl Placement {
    /// Parses a placement; exclusion zones name other sets, resolved by `set_index`.
    pub fn parse(json: &Value, biome_count: usize, biome_set: impl Fn(&Value) -> Result<Vec<bool>, String>, set_index: impl Fn(&str) -> Option<usize>, seed: i64) -> Result<Self, String> {
        let kind_name = json["type"].as_str().ok_or("placement lacks type")?;
        let int = |key: &str| json.get(key).and_then(Value::as_i64).map(|v| v as i32);
        let locate_offset = match json.get("locate_offset").and_then(Value::as_array) {
            Some(v) => [0, 1, 2].map(|i| v.get(i).and_then(Value::as_i64).unwrap_or(0) as i32),
            None => [0, 0, 0],
        };
        let reduction = match json.get("frequency_reduction_method").and_then(Value::as_str).unwrap_or("default") {
            "default" => FrequencyReduction::Default,
            "legacy_type_1" => FrequencyReduction::LegacyType1,
            "legacy_type_2" => FrequencyReduction::LegacyType2,
            "legacy_type_3" => FrequencyReduction::LegacyType3,
            other => return Err(format!("unknown frequency reduction {other}")),
        };
        let exclusion = match json.get("exclusion_zone") {
            Some(zone) => {
                let other = zone["other_set"].as_str().ok_or("exclusion zone other_set must be a set name")?;
                let index = set_index(other).ok_or_else(|| format!("unknown structure set {other}"))?;
                Some((index, zone["chunk_count"].as_i64().ok_or("exclusion zone lacks chunk_count")? as i32))
            }
            None => None,
        };
        let kind = match kind_name.trim_start_matches("minecraft:") {
            "random_spread" => Kind::RandomSpread {
                spacing: int("spacing").ok_or("random_spread lacks spacing")?,
                separation: int("separation").ok_or("random_spread lacks separation")?,
                spread: match json.get("spread_type").and_then(Value::as_str).unwrap_or("linear") {
                    "linear" => SpreadType::Linear,
                    "triangular" => SpreadType::Triangular,
                    other => return Err(format!("unknown spread type {other}")),
                },
            },
            "concentric_rings" => {
                let preferred = biome_set(&json["preferred_biomes"])?;
                debug_assert_eq!(preferred.len(), biome_count);
                Kind::ConcentricRings {
                    preferred,
                    slots: ring_slots(seed, int("distance").unwrap_or(0), int("spread").unwrap_or(0), int("count").unwrap_or(0)),
                }
            }
            "dimension_origin" => Kind::DimensionOrigin,
            other => return Err(format!("unknown structure placement {other}")),
        };
        Ok(Self {
            kind,
            locate_offset,
            reduction,
            frequency: json.get("frequency").and_then(Value::as_f64).map_or(1.0, |v| v as f32),
            salt: int("salt").unwrap_or(0),
            exclusion,
        })
    }

    /// `StructurePlacement.isStructureChunk`.
    pub fn is_structure_chunk(&self, ctx: &PlacementContext, x: i32, z: i32) -> bool {
        let placement = match &self.kind {
            Kind::RandomSpread { .. } => self.potential_chunk(ctx.seed, x, z) == Some(ChunkPos::new(x, z)),
            Kind::ConcentricRings { preferred, slots } => ring_contains(ctx, preferred, slots, x, z),
            Kind::DimensionOrigin => return ctx.terrain.spawn_origin() == ChunkPos::new(x, z),
        };
        placement
            && (self.frequency >= 1.0 || self.reduction.should_generate(ctx.seed, self.salt, x, z, self.frequency))
            && self.exclusion.is_none_or(|(other, range)| !has_structure_chunk_in_range(ctx, &ctx.all[other], x, z, range))
    }

    /// `RandomSpreadStructurePlacement.getPotentialStructureChunk`.
    pub fn potential_chunk(&self, seed: i64, x: i32, z: i32) -> Option<ChunkPos> {
        let Kind::RandomSpread { spacing, separation, spread } = self.kind else { return None };
        let grid_x = x.div_euclid(spacing);
        let grid_z = z.div_euclid(spacing);
        let mut random = LegacyRandom::new(0);
        large_feature_with_salt(&mut random, seed, grid_x, grid_z, self.salt);
        let limit = spacing - separation;
        let spread_x = spread.evaluate(&mut random, limit);
        let spread_z = spread.evaluate(&mut random, limit);
        Some(ChunkPos::new(grid_x * spacing + spread_x, grid_z * spacing + spread_z))
    }

    /// The first `count` concentric ring positions, resolved (none for
    /// other placements).
    pub fn ring_chunks(&self, terrain: &TerrainGenerator, count: usize) -> Vec<ChunkPos> {
        match &self.kind {
            Kind::ConcentricRings { preferred, slots } => slots.iter().take(count).map(|slot| resolve_ring(terrain, preferred, slot)).collect(),
            _ => Vec::new(),
        }
    }

    /// `StructurePlacement.getLocatePos`.
    pub fn locate_pos(&self, chunk: ChunkPos) -> BlockPos {
        BlockPos::new(chunk.min_block_x() + self.locate_offset[0], self.locate_offset[1], chunk.min_block_z() + self.locate_offset[2])
    }
}

/// `ChunkGeneratorStructureState.hasStructureChunkInRange`.
fn has_structure_chunk_in_range(ctx: &PlacementContext, placement: &Placement, x: i32, z: i32, range: i32) -> bool {
    for tx in x - range..=x + range {
        for tz in z - range..=z + range {
            if placement.is_structure_chunk(ctx, tx, tz) {
                return true;
            }
        }
    }
    false
}

/// `ChunkGeneratorStructureState.generateRingPositions`, without the biome
/// searches: the initial positions and each search's forked random.
fn ring_slots(seed: i64, distance: i32, mut spread: i32, count: i32) -> Vec<RingSlot> {
    let mut slots = Vec::with_capacity(count.max(0) as usize);
    if count == 0 {
        return slots;
    }
    let mut random = LegacyRandom::new(0);
    random.set_seed(seed);
    let mut angle = random.next_f64() * std::f64::consts::PI * 2.0;
    let mut position_in_circle = 0;
    let mut circle = 0;
    for i in 0..count {
        let dist = f64::from(4 * distance + distance * circle * 6) + (random.next_f64() - 0.5) * (f64::from(distance) * 2.5);
        // `Math.round`: ties round toward positive infinity.
        let initial_x = (angle.cos() * dist + 0.5).floor() as i32;
        let initial_z = (angle.sin() * dist + 0.5).floor() as i32;
        // `random.fork()`: a new legacy source seeded by the next long.
        let search_seed = random.next_i64();
        slots.push(RingSlot { initial: ChunkPos::new(initial_x, initial_z), search_seed, resolved: OnceLock::new() });
        angle += std::f64::consts::PI * 2.0 / f64::from(spread);
        position_in_circle += 1;
        if position_in_circle == spread {
            circle += 1;
            position_in_circle = 0;
            spread += 2 * spread / (circle + 1);
            spread = spread.min(count - i);
            angle += random.next_f64() * std::f64::consts::PI * 2.0;
        }
    }
    slots
}

fn ring_contains(ctx: &PlacementContext, preferred: &[bool], slots: &[RingSlot], x: i32, z: i32) -> bool {
    slots.iter().any(|slot| {
        // A biome search moves a position by at most seven chunks.
        if (slot.initial.x - x).abs() > 7 || (slot.initial.z - z).abs() > 7 {
            return false;
        }
        *slot.resolved.get_or_init(|| resolve_ring(ctx.terrain, preferred, slot)) == ChunkPos::new(x, z)
    })
}

/// `BiomeSource.findBiomeHorizontal` around a ring position (radius 112,
/// every quart, reservoir-sampling among preferred biomes).
fn resolve_ring(terrain: &TerrainGenerator, preferred: &[bool], slot: &RingSlot) -> ChunkPos {
    let mut random = LegacyRandom::new(slot.search_seed);
    let (origin_x, origin_z) = (slot.initial.x * 16 + 8, slot.initial.z * 16 + 8);
    let (cx, cz, radius, qy) = (origin_x >> 2, origin_z >> 2, 112 >> 2, 0);
    let mut result: Option<(i32, i32)> = None;
    let mut found = 0;
    for z in -radius..=radius {
        for x in -radius..=radius {
            let (nx, nz) = (cx + x, cz + z);
            let biome: BiomeId = terrain.biome_at_quart(nx, qy, nz);
            if preferred.get(usize::from(biome.0)).copied().unwrap_or(false) {
                if result.is_none() || random.next_i32_bound(found + 1) == 0 {
                    result = Some((nx << 2, nz << 2));
                }
                found += 1;
            }
        }
    }
    match result {
        Some((bx, bz)) => ChunkPos::new(bx >> 4, bz >> 4),
        None => slot.initial,
    }
}
