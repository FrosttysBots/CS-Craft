//! The server level as the natural spawner sees it
//! ([`crate::natural_spawner::SpawnLevel`]): blocks, zoomed biomes, light,
//! the level random, inhabited time, and spawned mobs collected for the
//! entity world; and the spawning half of `ServerChunkCache.tickChunks`
//! around the server's players ([`NaturalSpawning`]).

use super::Level;
use crate::distance::PlayerChunkCounter;
use crate::natural_spawner::spawning::{SpawnContext, MOON_BRIGHTNESS_PER_PHASE};
use crate::natural_spawner::tick::{CensusMob, SpawnPlayer, TickInput, TickReport};
use crate::natural_spawner::{CreatureSpawns, SpawnLevel};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::nbt::Tag;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::{BiomeId, BlockPos, BlockStateId, ChunkPos};
use minecraftoss_generator::feature::World;
use std::collections::HashSet;
use std::sync::Arc;

/// Natural spawning's state on the server level: `DistanceManager`'s
/// natural spawn counter and what the server hands it every tick.
pub struct NaturalSpawning {
    pub spawns: Arc<CreatureSpawns>,
    counter: PlayerChunkCounter,
    /// The chunk section each player is registered in.
    registered: Vec<(i32, i32, i32)>,
    players: Vec<SpawnPlayer>,
    /// Every mob in the level (the entity world's), for the caps.
    pub census: Vec<CensusMob>,
    /// Game rules `spawn_mobs` and `spawn_monsters`.
    pub spawn_mobs: bool,
    pub spawn_monsters: bool,
    /// The world spawn when it is in this dimension.
    pub respawn: Option<BlockPos>,
    /// Chunks inside the simulation distance (`inEntityTickingRange`).
    pub simulation: HashSet<ChunkPos>,
    /// What the last tick's spawning decided.
    pub report: TickReport,
}

impl NaturalSpawning {
    pub fn new(spawns: Arc<CreatureSpawns>) -> Self {
        Self {
            spawns,
            counter: PlayerChunkCounter::new(8),
            registered: Vec::new(),
            players: Vec::new(),
            census: Vec::new(),
            spawn_mobs: true,
            spawn_monsters: true,
            respawn: None,
            simulation: HashSet::new(),
            report: TickReport::default(),
        }
    }

    /// The players, each registered with the spawn counter by chunk and
    /// moved between chunk sections as `ChunkMap.move` moves it (matched
    /// by index).
    pub fn set_players(&mut self, players: &[SpawnPlayer]) {
        let section = |p: &SpawnPlayer| ((p.pos[0].floor() as i32) >> 4, (p.pos[1].floor() as i32) >> 4, (p.pos[2].floor() as i32) >> 4);
        let chunk = |s: (i32, i32, i32)| ChunkPos::new(s.0, s.2);
        for (i, player) in players.iter().enumerate() {
            let now = section(player);
            match self.registered.get(i).copied() {
                None => {
                    self.counter.add_player(chunk(now));
                    self.registered.push(now);
                }
                Some(old) if old != now => {
                    self.counter.remove_player(chunk(old));
                    self.counter.add_player(chunk(now));
                    self.registered[i] = now;
                }
                Some(_) => {}
            }
        }
        while self.registered.len() > players.len() {
            let old = self.registered.pop().expect("longer than players");
            self.counter.remove_player(chunk(old));
        }
        self.players = players.to_vec();
    }
}

/// `SpecialDates.isHalloween`: October 31 (in UTC).
fn is_halloween() -> bool {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400) as i64;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    month == 10 && day == 31
}

/// What the spawner's context reads from the level, gathered once.
struct SpawnInputs {
    feet: Vec<[f64; 3]>,
    loaded: HashSet<ChunkPos>,
    slime_chances: Vec<f32>,
    overworld_time: i64,
    moon_brightness: f32,
    raining: bool,
    thundering: bool,
    difficulty: i32,
    seed: i64,
}

impl SpawnInputs {
    /// The context over these inputs: the census's mobs and the players
    /// block spawns, and chickens nobody rides can carry baby zombies.
    fn context<'a>(
        &'a self,
        respawn: Option<BlockPos>,
        census: &[CensusMob],
        in_loaded: &'a dyn Fn(ChunkPos) -> bool,
        slime_chance: &'a dyn Fn(BiomeId) -> f32,
    ) -> SpawnContext<'a> {
        let mut obstacles: Vec<[f64; 6]> = census.iter().map(|m| m.bb).collect();
        obstacles.extend(self.feet.iter().map(|p| [p[0] - 0.3, p[1], p[2] - 0.3, p[0] + 0.3, p[1] + 1.8, p[2] + 0.3]));
        let chickens = census.iter().filter(|m| m.kind == "minecraft:chicken" && !m.riding).map(|m| m.bb).collect();
        SpawnContext {
            players: &self.feet,
            respawn,
            difficulty: self.difficulty,
            overworld_time: self.overworld_time,
            moon_brightness: self.moon_brightness,
            raining: self.raining,
            thundering: self.thundering,
            seed: self.seed,
            surface_slime_chance: slime_chance,
            halloween: is_halloween(),
            can_spawn_in_chunk: in_loaded,
            obstacles,
            chickens,
        }
    }
}

impl Level<'_> {
    fn spawn_inputs(&self, players: &[SpawnPlayer]) -> SpawnInputs {
        SpawnInputs {
            feet: players.iter().filter(|p| !p.spectator).map(|p| p.pos).collect(),
            loaded: self.chunks.keys().copied().collect(),
            slime_chances: (0..self.registries().biomes.len()).map(|b| self.biome_value("gameplay/surface_slime_spawn_chance", b as u16)).collect(),
            overworld_time: self.sky.as_ref().and_then(|s| s.clocks.get("minecraft:overworld").copied()).unwrap_or(0),
            moon_brightness: MOON_BRIGHTNESS_PER_PHASE[self.moon_phase()],
            raining: self.is_raining(),
            thundering: self.sky.as_ref().is_some_and(|s| s.can_have_weather && s.weather.thunder_level > 0.9),
            difficulty: self.difficulty,
            seed: self.random_sequences.world_seed,
        }
    }

    /// The spawning half of `ServerChunkCache.tickChunks`, when the server
    /// runs natural spawning ([`Level::natural_spawning`]).
    pub(super) fn tick_natural_spawning(&mut self) {
        let Some(mut natural) = self.natural_spawning.take() else { return };
        let players = std::mem::take(&mut natural.players);
        let census = std::mem::take(&mut natural.census);
        let inputs = self.spawn_inputs(&players);
        let in_loaded = |c: ChunkPos| inputs.loaded.contains(&c);
        let slime_chance = |b: BiomeId| inputs.slime_chances.get(usize::from(b.0)).copied().unwrap_or(0.0);
        let simulation = std::mem::take(&mut natural.simulation);
        let in_simulation = |c: ChunkPos| simulation.contains(&c);
        let mut context = inputs.context(natural.respawn, &census, &in_loaded, &slime_chance);
        let mut input = TickInput {
            counter: &mut natural.counter,
            players: &players,
            census: &census,
            spawn_mobs: natural.spawn_mobs,
            spawn_enemies: natural.spawn_mobs && natural.spawn_monsters,
            spawn_persistent: self.game_time % 400 == 0,
            loaded: &in_loaded,
            ticking: &in_loaded,
            entity_ticking_range: &in_simulation,
        };
        let spawns = natural.spawns.clone();
        natural.report = spawns.tick_spawning(self, &mut context, &mut input);
        natural.players = players;
        natural.census = census;
        natural.simulation = simulation;
        self.natural_spawning = Some(natural);
    }

    /// `SummonCommand.createEntity` for a mob at `at`: its tag, ready for
    /// the entity world. Monsters cannot be summoned in peaceful, and
    /// positions must lie in `Level.isInSpawnableBounds`. Needs natural
    /// spawning's tables ([`Level::natural_spawning`]); `y_rot` is the new
    /// mob's own random yaw.
    pub fn summon_mob(&mut self, kind: &str, at: [f64; 3], nbt: Option<&Tag>, y_rot: f32) -> Result<Tag, String> {
        let block = [at[0].floor(), at[1].floor(), at[2].floor()];
        if block[0].abs() >= 30_000_000.0 || block[2].abs() >= 30_000_000.0 || block[1].abs() >= 20_000_000.0 {
            return Err("Invalid position for summon".to_owned());
        }
        if self.difficulty == 0 && crate::natural_spawner::spawning::MobCategory::of_type(kind) == Some(crate::natural_spawner::spawning::MobCategory::Monster) {
            return Err("You can not summon a monster in peaceful mode".to_owned());
        }
        let Some(natural) = self.natural_spawning.take() else { return Err("Unable to summon entity".to_owned()) };
        let inputs = self.spawn_inputs(&natural.players);
        let in_loaded = |c: ChunkPos| inputs.loaded.contains(&c);
        let slime_chance = |b: BiomeId| inputs.slime_chances.get(usize::from(b.0)).copied().unwrap_or(0.0);
        let mut context = inputs.context(natural.respawn, &natural.census, &in_loaded, &slime_chance);
        let spawns = natural.spawns.clone();
        let result = spawns.summon(self, &mut context, kind, at, nbt, y_rot);
        self.natural_spawning = Some(natural);
        result
    }
}

impl SpawnLevel for Level<'_> {
    fn block(&self, pos: BlockPos) -> BlockStateId {
        Level::block(self, pos)
    }

    fn height(&self, kind: HeightmapKind, x: i32, z: i32) -> i32 {
        World::height_at(self, kind, x, z)
    }

    /// `getBiome`: the zoomed biome.
    fn biome(&self, pos: BlockPos) -> BiomeId {
        World::biome(self, pos.x, pos.y, pos.z).unwrap_or(BiomeId(0))
    }

    fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> BiomeId {
        self.chunk(ChunkPos::new(qx >> 2, qz >> 2)).map_or(BiomeId(0), |c| c.biome((qx & 3) as usize, qy, (qz & 3) as usize))
    }

    fn raw_brightness(&mut self, pos: BlockPos, darkening: i32) -> i32 {
        (self.sky_light(pos) - darkening).max(self.block_light(pos))
    }

    fn sky_brightness(&mut self, pos: BlockPos) -> i32 {
        self.sky_light(pos)
    }

    fn block_brightness(&mut self, pos: BlockPos) -> i32 {
        self.block_light(pos)
    }

    fn sky_darken(&self) -> i32 {
        self.sky.as_ref().map_or(0, |s| s.sky_darken)
    }

    fn sea_level(&self) -> i32 {
        self.sea_level
    }

    fn min_y(&self) -> i32 {
        Level::min_y(self)
    }

    fn random(&mut self) -> &mut dyn RandomSource {
        &mut self.random
    }

    /// `Mth.createInsecureUUID` from the entity's own random.
    fn next_uuid(&mut self) -> [i32; 4] {
        let most = (self.entity_random.next_i64() & -61441) | 16384;
        let least = (self.entity_random.next_i64() & 0x3FFF_FFFF_FFFF_FFFF) | i64::MIN;
        [(most >> 32) as i32, most as i32, (least >> 32) as i32, least as i32]
    }

    /// `addFreshEntityWithPassengers`: the entity world takes it.
    fn add_entity(&mut self, entity: Tag) {
        self.spawned.push(entity);
    }

    fn inhabited_time(&self, chunk: ChunkPos) -> Option<i64> {
        self.chunk(chunk).map(|c| c.inhabited_time)
    }

    fn add_inhabited_time(&mut self, chunk: ChunkPos) {
        if let Some(c) = self.chunks.get_mut(&chunk) {
            c.inhabited_time += 1;
        }
    }

    fn note_unsupported(&mut self, what: &str) {
        self.unsupported.push(what.to_owned());
    }
}
