//! The natural spawning half of `ServerChunkCache.tickChunks` (26.3):
//! `NaturalSpawner.createState` (mob counts, spawn potentials and local mob
//! caps), `getFilteredSpawningCategories`, `ChunkMap.collectSpawningChunks`
//! in the spawn counter's order, the level random's `Util.shuffle`, then per
//! chunk the inhabited time, `tickThunder`'s roll and `spawnForChunk`
//! (`getRandomPosWithin`, then [`CreatureSpawns::spawn_category_for_position`]).

use super::spawning::{mob_box, MobCategory, SpawnCallbacks, SpawnContext};
use super::{type_info, CreatureSpawns, SpawnLevel};
use minecraftoss_core::nbt::Tag;
use crate::distance::PlayerChunkCounter;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::{BlockPos, ChunkPos};
use std::collections::{HashMap, HashSet};

/// `NaturalSpawner.MAGIC_NUMBER`: 17², the chunks one player spawns in.
const MAGIC_NUMBER: i32 = 289;

/// A player as spawning sees it.
#[derive(Clone, Copy, Debug)]
pub struct SpawnPlayer {
    pub pos: [f64; 3],
    pub spectator: bool,
}

/// An entity in the level, as `createState` counts it and spawning's
/// obstruction and jockey tests see it.
#[derive(Clone, Debug)]
pub struct CensusMob {
    pub kind: String,
    pub pos: [f64; 3],
    pub bb: [f64; 6],
    /// `isPersistenceRequired() || requiresCustomPersistence()` (riding or
    /// leashed mobs included).
    pub persistent: bool,
    /// A vehicle or a passenger (not a free chicken for jockeys).
    pub riding: bool,
}

/// A spawned entity's census entries (from its saved tag): the entity and
/// its passengers, which stay persistent while they ride.
pub fn census_of(tag: &Tag) -> Vec<CensusMob> {
    fn walk(tag: &Tag, passenger: bool, out: &mut Vec<CensusMob>) {
        let kind = tag.get("id").and_then(Tag::as_str).unwrap_or_default().to_owned();
        let at = |i: usize| tag.get("Pos").and_then(Tag::as_list).and_then(|p| p.get(i)).and_then(Tag::as_f64).unwrap_or(0.0);
        let pos = [at(0), at(1), at(2)];
        let passengers = tag.get("Passengers").and_then(Tag::as_list).unwrap_or(&[]);
        let bb = type_info(&kind).map_or([pos[0], pos[1], pos[2], pos[0], pos[1], pos[2]], |info| mob_box(tag, &info, pos));
        let persistent = passenger || tag.get("PersistenceRequired").and_then(Tag::as_i64) == Some(1);
        out.push(CensusMob { kind, pos, bb, persistent, riding: passenger || !passengers.is_empty() });
        for rider in passengers {
            walk(rider, true, out);
        }
    }
    let mut out = Vec::new();
    walk(tag, false, &mut out);
    out
}

/// `LocalMobCapCalculator`: each nearby player's counts.
struct LocalCaps {
    /// Non-spectator players in the chunk map.
    players: Vec<[f64; 3]>,
    /// The spawn counter's chunks (`hasPlayersNearby` short of false).
    counted: HashSet<ChunkPos>,
    near: HashMap<ChunkPos, Vec<usize>>,
    counts: HashMap<usize, [i32; MobCategory::COUNT]>,
}

/// `ChunkMap.euclideanDistanceSquared` below 128² (`playerIsCloseEnoughForSpawning`).
fn close_for_spawning(chunk: ChunkPos, pos: [f64; 3]) -> bool {
    let (x, z) = (f64::from(chunk.x * 16 + 8) - pos[0], f64::from(chunk.z * 16 + 8) - pos[2]);
    x * x + z * z < 16384.0
}

impl LocalCaps {
    /// `getPlayersCloseForSpawning`, cached per chunk.
    fn players_near(&mut self, chunk: ChunkPos) -> Vec<usize> {
        let (players, counted) = (&self.players, &self.counted);
        self.near
            .entry(chunk)
            .or_insert_with(|| {
                if !counted.contains(&chunk) {
                    return Vec::new();
                }
                (0..players.len()).filter(|&i| close_for_spawning(chunk, players[i])).collect()
            })
            .clone()
    }

    fn add_mob(&mut self, chunk: ChunkPos, category: MobCategory) {
        for player in self.players_near(chunk) {
            self.counts.entry(player).or_default()[category as usize] += 1;
        }
    }

    fn can_spawn(&mut self, category: MobCategory, chunk: ChunkPos) -> bool {
        self.players_near(chunk)
            .into_iter()
            .any(|player| self.counts.get(&player).is_none_or(|c| c[category as usize] < category.max_instances_per_chunk()))
    }
}

/// `NaturalSpawner.SpawnState`.
pub struct SpawnState {
    spawnable_chunk_count: i32,
    counts: [i32; MobCategory::COUNT],
    /// `PotentialCalculator` point charges, in order.
    charges: Vec<(BlockPos, f64)>,
    local: LocalCaps,
    last_checked: Option<(BlockPos, String)>,
    last_charge: f64,
}

impl SpawnState {
    /// `canSpawnForCategoryGlobal`.
    fn can_spawn_global(&self, category: MobCategory) -> bool {
        let max = category.max_instances_per_chunk() * self.spawnable_chunk_count / MAGIC_NUMBER;
        self.counts[category as usize] < max
    }

    /// `PotentialCalculator.getPotentialEnergyChange`.
    fn potential_change(&self, pos: BlockPos, charge: f64) -> f64 {
        if charge == 0.0 {
            return 0.0;
        }
        let mut change = 0.0;
        for (at, point) in &self.charges {
            let (dx, dy, dz) = (f64::from(at.x - pos.x), f64::from(at.y - pos.y), f64::from(at.z - pos.z));
            let dist = dx * dx + dy * dy + dz * dz;
            change += if dist == 0.0 { f64::INFINITY } else { point / dist.sqrt() };
        }
        change * charge
    }

    pub fn counts(&self) -> [i32; MobCategory::COUNT] {
        self.counts
    }
}

impl SpawnCallbacks for SpawnState {
    fn can_spawn(&mut self, kind: &str, pos: BlockPos, cost: Option<(f64, f64)>) -> bool {
        self.last_checked = Some((pos, kind.to_owned()));
        match cost {
            None => {
                self.last_charge = 0.0;
                true
            }
            Some((budget, charge)) => {
                self.last_charge = charge;
                self.potential_change(pos, charge) <= budget
            }
        }
    }

    fn after_spawn(&mut self, kind: &str, pos: BlockPos, cost: Option<(f64, f64)>) {
        let same = self.last_checked.as_ref().is_some_and(|(p, k)| *p == pos && k == kind);
        let charge = if same { self.last_charge } else { cost.map_or(0.0, |c| c.1) };
        if charge != 0.0 {
            self.charges.push((pos, charge));
        }
        if let Some(category) = MobCategory::of_type(kind) {
            self.counts[category as usize] += 1;
            self.local.add_mob(pos.chunk(), category);
        }
    }
}

/// What one tick's spawning reads from the server.
pub struct TickInput<'a> {
    /// The natural spawn counter (players registered by chunk).
    pub counter: &'a mut PlayerChunkCounter,
    pub players: &'a [SpawnPlayer],
    /// Every entity in the level (the census for mob caps).
    pub census: &'a [CensusMob],
    /// Game rules `spawn_mobs`, and `spawn_monsters` with it.
    pub spawn_mobs: bool,
    pub spawn_enemies: bool,
    /// Every 400 ticks: creatures spawn too.
    pub spawn_persistent: bool,
    /// `getFullChunk`: the chunk is loaded at FULL.
    pub loaded: &'a dyn Fn(ChunkPos) -> bool,
    /// `ChunkHolder.getTickingChunk`: the chunk is loaded and block-ticking.
    pub ticking: &'a dyn Fn(ChunkPos) -> bool,
    /// `DistanceManager.inEntityTickingRange` (the simulation distance).
    pub entity_ticking_range: &'a dyn Fn(ChunkPos) -> bool,
}

/// What a tick's spawning decided, for diagnostics and comparisons.
#[derive(Clone, Debug, Default)]
pub struct TickReport {
    pub chunk_count: usize,
    pub categories: Vec<MobCategory>,
    /// The spawn counter's chunks in order.
    pub candidates: Vec<ChunkPos>,
    /// The spawning chunks before the shuffle.
    pub spawning: Vec<ChunkPos>,
    /// Mob counts by category after spawning.
    pub counts: [i32; MobCategory::COUNT],
}

impl CreatureSpawns {
    /// `NaturalSpawner.createState`.
    fn create_state(&self, level: &dyn SpawnLevel, input: &TickInput, chunk_count: usize, counted: HashSet<ChunkPos>) -> SpawnState {
        let players = input.players.iter().filter(|p| !p.spectator).map(|p| p.pos).collect();
        let local = LocalCaps { players, counted, near: HashMap::new(), counts: HashMap::new() };
        let mut state = SpawnState {
            spawnable_chunk_count: chunk_count as i32,
            counts: [0; MobCategory::COUNT],
            charges: Vec::new(),
            local,
            last_checked: None,
            last_charge: 0.0,
        };
        for mob in input.census {
            if mob.persistent {
                continue;
            }
            let Some(category) = MobCategory::of_type(&mob.kind) else { continue };
            let pos = BlockPos::new(mob.pos[0].floor() as i32, mob.pos[1].floor() as i32, mob.pos[2].floor() as i32);
            if !(input.loaded)(pos.chunk()) {
                continue;
            }
            if let Some((_, charge)) = self.cost_at(level, &mob.kind, pos) {
                if charge != 0.0 {
                    state.charges.push((pos, charge));
                }
            }
            state.local.add_mob(pos.chunk(), category);
            state.counts[category as usize] += 1;
        }
        state
    }

    /// One tick of natural spawning around the players.
    pub fn tick_spawning(&self, level: &mut dyn SpawnLevel, context: &mut SpawnContext, input: &mut TickInput) -> TickReport {
        let chunk_count = input.counter.chunk_count();
        let candidates = input.counter.candidates();
        let mut state = self.create_state(level, input, chunk_count, candidates.iter().copied().collect());
        let categories: Vec<MobCategory> = if input.spawn_mobs {
            MobCategory::SPAWNING
                .into_iter()
                .filter(|&c| (input.spawn_enemies || c.friendly()) && (input.spawn_persistent || !c.persistent()) && state.can_spawn_global(c))
                .collect()
        } else {
            Vec::new()
        };
        let near_players: Vec<[f64; 3]> = input.players.iter().filter(|p| !p.spectator).map(|p| p.pos).collect();
        let spawning: Vec<ChunkPos> =
            candidates.iter().copied().filter(|&c| (input.ticking)(c) && near_players.iter().any(|&p| close_for_spawning(c, p))).collect();
        // `Util.shuffle`.
        let mut order = spawning.clone();
        for i in (2..=order.len()).rev() {
            let to = level.random().next_i32_bound(i as i32) as usize;
            order.swap(i - 1, to);
        }
        for chunk in order {
            level.add_inhabited_time(chunk);
            if (input.entity_ticking_range)(chunk) && context.raining && context.thundering && level.random().next_i32_bound(100_000) == 0 {
                level.note_unsupported("lightning from thunder");
            }
            if !categories.is_empty() && (context.can_spawn_in_chunk)(chunk) {
                for &category in &categories {
                    if state.local.can_spawn(category, chunk) {
                        self.spawn_category_for_chunk(level, context, &mut state, category, chunk);
                    }
                }
            }
        }
        TickReport { chunk_count, categories, candidates, spawning, counts: state.counts }
    }

    /// `spawnCategoryForChunk`: a random start column and height.
    fn spawn_category_for_chunk(&self, level: &mut dyn SpawnLevel, context: &mut SpawnContext, state: &mut SpawnState, category: MobCategory, chunk: ChunkPos) {
        // `getRandomPosWithin`: up to one above the surface.
        let x = chunk.min_block_x() + level.random().next_i32_bound(16);
        let z = chunk.min_block_z() + level.random().next_i32_bound(16);
        let top_empty = level.height(HeightmapKind::WorldSurface, x, z);
        let min_y = level.min_y();
        let y = level.random().next_i32_bound(top_empty - min_y + 1) + min_y;
        if y >= min_y + 1 {
            self.spawn_category_for_position(level, context, state, category, chunk, BlockPos::new(x, y, z));
        }
    }
}
