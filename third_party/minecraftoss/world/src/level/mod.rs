//! The server world simulation (26.3 `Level`, `ServerLevel`, `LevelChunk`
//! block setting and `CollectingNeighborUpdater`): FULL chunks, `setBlock`
//! with its flags, neighbour and shape updates in vanilla's order,
//! scheduled block and fluid ticks, and block behaviours.
//!
//! Shape updates reuse the generator's rules (`update_shape`) through the
//! generator's `World` trait, so the same code runs in worldgen and here.
//! Implemented behaviours so far: liquid blocks and flowing fluids, and
//! redstone components (`redstone`).

pub mod container;
pub mod dispenser;
pub mod drops;
pub mod entity;
pub mod explosion;
pub mod falling;
pub mod fire;
pub mod grow;
pub mod plants;
pub mod bonemeal;
pub mod fluid;
pub mod physics;
pub mod piston;
pub mod rail;
pub mod random_tick;
pub mod redstone;
pub mod spawning;
pub mod ticks;
pub mod time;

use minecraftoss_core::block::flags;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::AnyRandom;
use minecraftoss_core::{BlockId, BlockPos, BlockStateId, Chunk, ChunkPos, Registries};
use minecraftoss_generator::feature::{Ctx, Library, World};
use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, VecDeque};
use ticks::{LevelTicks, TickType};

/// `Block.UPDATE_*` flags.
pub mod update {
    pub const NEIGHBORS: u32 = 1;
    pub const CLIENTS: u32 = 2;
    pub const INVISIBLE: u32 = 4;
    pub const KNOWN_SHAPE: u32 = 16;
    pub const SUPPRESS_DROPS: u32 = 32;
    pub const MOVE_BY_PISTON: u32 = 64;
    pub const SKIP_SHAPE_UPDATE_ON_WIRE: u32 = 128;
    pub const SKIP_BLOCK_ENTITY_SIDEEFFECTS: u32 = 256;
    pub const SKIP_ON_PLACE: u32 = 512;
    pub const ALL: u32 = NEIGHBORS | CLIENTS;
    /// `Block.UPDATE_LIMIT`.
    pub const LIMIT: i32 = 512;
}

/// `NeighborUpdater.UPDATE_ORDER`.
const UPDATE_ORDER: [Direction; 6] = [Direction::West, Direction::East, Direction::Down, Direction::Up, Direction::North, Direction::South];
/// `BlockBehaviour.UPDATE_SHAPE_ORDER`.
const UPDATE_SHAPE_ORDER: [Direction; 6] = [Direction::West, Direction::East, Direction::North, Direction::South, Direction::Down, Direction::Up];
/// `ServerLevel`'s `maxChainedNeighborUpdates` game rule default.
const MAX_CHAINED_NEIGHBOR_UPDATES: usize = 1_000_000;
/// `LevelTicks` drain limit per game tick.
const MAX_TICKS: usize = 65_536;

/// One queued update of `CollectingNeighborUpdater`.
#[derive(Clone, Debug)]
pub(crate) enum Update {
    Shape { direction: Direction, neighbor_state: BlockStateId, pos: BlockPos, neighbor_pos: BlockPos, flags: u32, limit: i32 },
    Neighbors { source: BlockPos, block: BlockId, skip: Option<Direction>, index: usize },
    /// `SimpleNeighborUpdate`: `neighborChanged` at a position.
    Simple { pos: BlockPos, block: BlockId },
    /// `FullNeighborUpdate`: `neighborChanged` with a given state.
    Full { state: BlockStateId, pos: BlockPos, block: BlockId },
}

#[derive(Default)]
struct Updater {
    stack: Vec<Update>,
    added: Vec<Update>,
    count: usize,
}

/// A dimension's loaded FULL chunks and their simulation state.
pub struct Level<'a> {
    pub lib: &'a Library,
    chunks: minecraftoss_core::fast_hash::FxHashMap<ChunkPos, Chunk>,
    min_y: i32,
    height: i32,
    pub game_time: i64,
    block_ticks: LevelTicks,
    fluid_ticks: LevelTicks,
    sub_tick: i64,
    /// `Level.random` (`RandomSource.create()`, unseeded in vanilla).
    pub random: AnyRandom,
    updates: Updater,
    /// `gameplay/fast_lava` (the Nether): lava ticks every 10 and spreads 4.
    pub fast_lava: bool,
    /// Game rules `water_source_conversion` and `lava_source_conversion`.
    pub water_source_conversion: bool,
    pub lava_source_conversion: bool,
    /// Positions whose block changed, for clients.
    changed: BTreeSet<(i32, i32, i32)>,
    /// Every block set, in order, for the points of interest
    /// (`ServerLevel.updatePOIOnBlockStateChange`), until taken.
    block_log: Vec<(i32, i32, i32)>,
    void_air: BlockStateId,
    /// `#minecraft:washed_away_by_fluids`.
    washed_away: minecraftoss_core::tags::TagId,
    kinds: redstone::Kinds,
    /// `RedstoneWireBlock.shouldSignal`: off while a wire reads its
    /// neighbours' power.
    wire_signals: Cell<bool>,
    /// `RedstoneTorchBlock.RECENT_TOGGLES`: position and game time.
    torch_toggles: Vec<(BlockPos, i64)>,
    /// Moving piston block entities and their tickers.
    pub moving: piston::MovingBlocks,
    /// `ServerLevel.blockEvents`.
    block_events: VecDeque<piston::BlockEvent>,
    /// `ServerLevel.handlingTick`: scheduled ticks and block events running.
    handling_tick: bool,
    /// `#minecraft:rails`.
    rails_tag: minecraftoss_core::tags::TagId,
    /// `HopperBlockEntity.tickedGameTime` (not saved).
    hopper_ticked: HashMap<BlockPos, i64>,
    /// The dispenser slot being dispensed from.
    dispensing_slot: Option<usize>,
    /// Behaviours that ran but are not simulated yet.
    pub unsupported: Vec<String>,
    /// Time, weather and sky light, once `set_dimension` has run.
    pub sky: Option<time::Sky>,
    /// Entities in tick order (`entityTickList`).
    pub entities: Vec<entity::Entity>,
    /// Mobs spawned naturally (saved tags), for the entity world to take.
    pub spawned: Vec<minecraftoss_core::nbt::Tag>,
    /// Natural spawning around players, when the server runs it.
    pub natural_spawning: Option<spawning::NaturalSpawning>,
    /// `Entity.ENTITY_COUNTER`.
    pub next_entity_id: i32,
    /// Stands in for entities' own unseeded randoms (`Entity.random`).
    pub entity_random: AnyRandom,
    fences_tag: minecraftoss_core::tags::TagId,
    walls_tag: minecraftoss_core::tags::TagId,
    suppresses_bounce_tag: minecraftoss_core::tags::TagId,
    grows_crops_tag: minecraftoss_core::tags::TagId,
    maintains_farmland_tag: minecraftoss_core::tags::TagId,
    snow_tag: minecraftoss_core::tags::TagId,
    /// `Level.randValue`, the random tick position LCG.
    pub rand_value: i32,
    /// Game rule `random_tick_speed`.
    pub random_tick_speed: i32,
    /// Game rule `block_drops`.
    pub block_drops: bool,
    /// `Level.getSeaLevel` (the chunk generator's: 63, or -63 on flat).
    pub sea_level: i32,
    /// Game rule `max_snow_accumulation_height`.
    pub max_snow_accumulation_height: i32,
    /// Game rule `fire_spread_radius_around_player` (-1: everywhere).
    pub fire_spread_radius: i32,
    /// Positions of the (non-spectator) players, for rules that need one
    /// nearby.
    pub players: Vec<[f64; 3]>,
    /// Living, non-spectator players' feet and eye heights (experience
    /// orbs follow them).
    pub living_players: Vec<([f64; 3], f64)>,
    /// `Difficulty.getId` (peaceful 0 to hard 3).
    pub difficulty: i32,
    /// The dimension type's `infiniburn` tag.
    pub(super) infiniburn: Option<minecraftoss_core::tags::TagId>,
    /// The dimension type, once `set_dimension` has run.
    pub(super) dimension: Option<String>,
    /// Game rule `entity_drops`.
    pub entity_drops: bool,
    /// Milliseconds of the last tick's phases: block ticks, fluid ticks,
    /// chunk ticks, block events and entities, block entities (profiling).
    pub last_tick_phases: [f64; 6],
    /// Light solves since the last reset, and their milliseconds (profiling).
    pub light_solves: std::cell::Cell<(u32, f64)>,
    /// The `minecraft:fire` block tag.
    fire_tag: minecraftoss_core::tags::TagId,
    /// Game rules `tnt_explodes` and `tnt_explosion_drop_decay`.
    pub tnt_explodes: bool,
    pub tnt_explosion_drop_decay: bool,
    /// Entity-ticking chunks in vanilla's iteration order, when known.
    pub ticking_chunks: Option<Vec<ChunkPos>>,
    /// Chunk light solved for readers without `&mut` (`World::light`).
    lazy_light: std::cell::RefCell<minecraftoss_core::fast_hash::FxHashMap<ChunkPos, std::sync::Arc<minecraftoss_core::light::ChunkLight>>>,
    /// Randomly ticking blocks per section, per chunk.
    random_counts: minecraftoss_core::fast_hash::FxHashMap<ChunkPos, Vec<u32>>,
    /// The world's named random sequences (loot).
    pub random_sequences: minecraftoss_core::loot::RandomSequences,
    blocks_fluid_flow_tag: minecraftoss_core::tags::TagId,
}

impl<'a> Level<'a> {
    /// The lowest block Y.
    pub fn min_y(&self) -> i32 {
        self.min_y
    }

    /// The highest block Y.
    pub fn max_y(&self) -> i32 {
        self.min_y + self.height - 1
    }

    pub fn new(lib: &'a Library, min_y: i32, height: i32) -> Self {
        let blocks = &lib.registries.blocks;
        let bedrock = blocks.parse_state("minecraft:bedrock").expect("bedrock exists");
        assert!(
            blocks.state(bedrock).destroy_speed == -1.0,
            "the block state catalog predates push reactions and hardness; re-export it (harness/export_block_state_catalog.py)"
        );
        Self {
            lib,
            chunks: Default::default(),
            min_y,
            height,
            game_time: 0,
            block_ticks: LevelTicks::default(),
            fluid_ticks: LevelTicks::default(),
            sub_tick: 0,
            random: AnyRandom::new(true, 0),
            updates: Updater::default(),
            fast_lava: false,
            water_source_conversion: true,
            lava_source_conversion: false,
            changed: BTreeSet::new(),
            block_log: Vec::new(),
            void_air: lib.registries.blocks.parse_state("minecraft:void_air").expect("void_air exists"),
            washed_away: lib.registries.block_tags.require("minecraft:washed_away_by_fluids").expect("tag exists"),
            kinds: redstone::Kinds::new(&lib.registries),
            wire_signals: Cell::new(true),
            torch_toggles: Vec::new(),
            moving: piston::MovingBlocks::default(),
            block_events: VecDeque::new(),
            handling_tick: false,
            rails_tag: lib.registries.block_tags.require("minecraft:rails").expect("tag exists"),
            hopper_ticked: HashMap::new(),
            dispensing_slot: None,
            unsupported: Vec::new(),
            sky: None,
            entities: Vec::new(),
            spawned: Vec::new(),
            natural_spawning: None,
            next_entity_id: 1,
            entity_random: AnyRandom::new(true, 0),
            fences_tag: lib.registries.block_tags.require("minecraft:fences").expect("tag exists"),
            walls_tag: lib.registries.block_tags.require("minecraft:walls").expect("tag exists"),
            suppresses_bounce_tag: lib.registries.block_tags.require("minecraft:suppresses_bounce").expect("tag exists"),
            grows_crops_tag: lib.registries.block_tags.require("minecraft:grows_crops").expect("tag exists"),
            maintains_farmland_tag: lib.registries.block_tags.require("minecraft:maintains_farmland").expect("tag exists"),
            snow_tag: lib.registries.block_tags.require("minecraft:snow").expect("tag exists"),
            rand_value: 0,
            random_tick_speed: 3,
            block_drops: true,
            sea_level: 63,
            max_snow_accumulation_height: 1,
            fire_spread_radius: 128,
            players: Vec::new(),
            living_players: Vec::new(),
            difficulty: 2,
            infiniburn: None,
            dimension: None,
            entity_drops: true,
            last_tick_phases: [0.0; 6],
            light_solves: std::cell::Cell::new((0, 0.0)),
            fire_tag: lib.registries.block_tags.require("minecraft:fire").expect("tag exists"),
            tnt_explodes: true,
            tnt_explosion_drop_decay: false,
            ticking_chunks: None,
            random_counts: Default::default(),
            lazy_light: std::cell::RefCell::new(Default::default()),
            random_sequences: minecraftoss_core::loot::RandomSequences::new(0),
            blocks_fluid_flow_tag: lib.registries.block_tags.require("minecraft:blocks_fluid_flow").expect("tag exists"),
        }
    }

    pub fn registries(&self) -> &Registries {
        &self.lib.registries
    }

    pub fn insert_chunk(&mut self, chunk: Chunk) {
        self.random_counts.remove(&chunk.pos);
        self.chunks.insert(chunk.pos, chunk);
    }

    /// Drops a chunk (unloading); its pending ticks stay queued.
    pub fn remove_chunk(&mut self, pos: ChunkPos) -> Option<Chunk> {
        self.random_counts.remove(&pos);
        self.lazy_light.get_mut().remove(&pos);
        self.chunks.remove(&pos)
    }

    pub fn chunk(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks.get(&pos)
    }

    pub fn chunks(&self) -> impl Iterator<Item = &Chunk> {
        self.chunks.values()
    }

    /// Positions changed since the last call.
    /// The blocks set since the last call, in order (repeats kept).
    pub fn take_block_log(&mut self) -> Vec<(i32, i32, i32)> {
        std::mem::take(&mut self.block_log)
    }

    pub fn take_changed(&mut self) -> Vec<(i32, i32, i32)> {
        std::mem::take(&mut self.changed).into_iter().collect()
    }

    fn outside(&self, y: i32) -> bool {
        y < self.min_y || y >= self.min_y + self.height
    }

    pub fn block(&self, pos: BlockPos) -> BlockStateId {
        if self.outside(pos.y) {
            return self.void_air;
        }
        self.chunks.get(&pos.chunk()).map_or(BlockStateId::AIR, |c| c.block((pos.x & 15) as usize, pos.y, (pos.z & 15) as usize))
    }

    fn block_id(&self, state: BlockStateId) -> BlockId {
        self.registries().blocks.block_of(state)
    }

    fn name(&self, state: BlockStateId) -> &str {
        let blocks = &self.registries().blocks;
        blocks.block(blocks.block_of(state)).name.as_str()
    }

    fn is_a(&self, state: BlockStateId, class: &str) -> bool {
        let blocks = &self.registries().blocks;
        blocks.block(blocks.block_of(state)).is_a(class)
    }

    // ---- setBlock ----------------------------------------------------------

    /// `Level.setBlockAndUpdate`: flags 3.
    pub fn set_block_and_update(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        self.set_block(pos, state, update::ALL, update::LIMIT)
    }

    /// `Level.setBlock(pos, state, flags, updateLimit)` with
    /// `LevelChunk.setBlockState`.
    pub fn set_block(&mut self, pos: BlockPos, state: BlockStateId, flags: u32, limit: i32) -> bool {
        if self.outside(pos.y) {
            return false;
        }
        let registries = self.lib.registries.clone();
        let Some(chunk) = self.chunks.get_mut(&pos.chunk()) else { return false };
        let (lx, lz) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
        let section = &chunk.sections()[((pos.y >> 4) - chunk.min_section_y()) as usize];
        if section.is_empty() && registries.blocks.is_air(state) {
            return false;
        }
        let old = chunk.block(lx, pos.y, lz);
        if old == state {
            return false;
        }
        let new_block = registries.blocks.block_of(state);
        let block_changed = registries.blocks.block_of(old) != new_block;
        let remove_block_entity = block_changed && registries.blocks.is(old, flags::HAS_BLOCK_ENTITY) && !self.keeps_block_entity(old, state);
        // `BlockEntity.preRemoveSideEffects` sees the entity before removal.
        let dropped = if remove_block_entity && flags & update::SKIP_BLOCK_ENTITY_SIDEEFFECTS == 0 { self.pre_remove_side_effects(pos, old) } else { Vec::new() };
        let Some(chunk) = self.chunks.get_mut(&pos.chunk()) else { return false };
        chunk.set_block_raw(lx, pos.y, lz, state, &registries);
        for kind in [HeightmapKind::MotionBlocking, HeightmapKind::MotionBlockingNoLeaves, HeightmapKind::OceanFloor, HeightmapKind::WorldSurface] {
            chunk.update_heightmap(kind, lx, pos.y, lz, state, &registries);
        }
        if remove_block_entity {
            chunk.block_entities.entities.remove(&(pos.x, pos.y, pos.z));
            chunk.block_entities.pending.remove(&(pos.x, pos.y, pos.z));
        }
        self.changed.insert((pos.x, pos.y, pos.z));
        self.block_log.push((pos.x, pos.y, pos.z));
        // `LightEngine.hasDifferentLightProperties`: light is solved again
        // for the chunks around, when next read.
        {
            let blocks = &registries.blocks;
            let (a, b) = (blocks.state(old), blocks.state(state));
            let shape = minecraftoss_core::block::flags::USE_SHAPE_FOR_LIGHT_OCCLUSION;
            let sky = minecraftoss_core::block::flags::PROPAGATES_SKYLIGHT_DOWN;
            if a.light_dampening != b.light_dampening
                || a.light_emission != b.light_emission
                || blocks.is(old, shape)
                || blocks.is(state, shape)
                || blocks.is(old, sky) != blocks.is(state, sky)
            {
                self.invalidate_light(pos);
            }
        }
        self.note_random_ticking(pos, old, state);
        for stack in dropped {
            self.drop_item_stack([f64::from(pos.x), f64::from(pos.y), f64::from(pos.z)], stack);
        }
        if remove_block_entity {
            self.moving.remove(pos);
        }
        let moved = flags & update::MOVE_BY_PISTON != 0;
        let is_rail = registries.blocks.block(new_block).is_a("BaseRailBlock");
        if (block_changed || is_rail) && (flags & update::NEIGHBORS != 0 || moved) {
            self.affect_neighbors_after_removal(old, pos, moved);
        }
        if self.block_id(self.block(pos)) != new_block {
            return false;
        }
        if flags & update::SKIP_ON_PLACE == 0 {
            self.on_place(state, pos, old, moved);
        }
        if registries.blocks.is(state, flags::HAS_BLOCK_ENTITY) && self.block_id(self.block(pos)) == new_block {
            self.ensure_block_entity(pos, state);
        }
        if self.block(pos) != state {
            return true;
        }
        if flags & update::NEIGHBORS != 0 {
            self.update_neighbors_at(pos, registries.blocks.block_of(old));
            if self.has_analog_output(state) {
                self.update_neighbour_for_output_signal(pos, new_block);
            }
        }
        if flags & update::KNOWN_SHAPE == 0 && limit > 0 {
            let neighbour_flags = flags & !(update::NEIGHBORS | update::SUPPRESS_DROPS);
            self.update_indirect_neighbour_shapes(old, pos, neighbour_flags, limit - 1);
            self.update_neighbour_shapes(state, pos, neighbour_flags, limit - 1);
            self.update_indirect_neighbour_shapes(state, pos, neighbour_flags, limit - 1);
        }
        true
    }

    /// `EntityBlock.newBlockEntity` when the position has none.
    fn ensure_block_entity(&mut self, pos: BlockPos, state: BlockStateId) {
        let key = (pos.x, pos.y, pos.z);
        let name = self.name(state).to_owned();
        let Some(catalog) = self.lib.registries.block_entities.as_ref() else { return };
        let Some(chunk) = self.chunks.get_mut(&pos.chunk()) else { return };
        if chunk.block_entities.entities.contains_key(&key) {
            return;
        }
        if let Some(nbt) = catalog.default_nbt(&name, key) {
            chunk.block_entities.entities.insert(key, nbt);
            if self.has_ticker(state) {
                self.moving.register(pos);
            }
        }
    }

    /// `BlockBehaviour.updateIndirectNeighbourShapes` (only redstone wire).
    fn update_indirect_neighbour_shapes(&mut self, state: BlockStateId, pos: BlockPos, flags: u32, limit: i32) {
        if self.redstone_kind(state) == Some(redstone::Kind::Wire) {
            self.wire_update_indirect_shapes(state, pos, flags, limit);
        }
    }

    /// `Level.removeBlock`: the block becomes its fluid (flags 3, plus 64
    /// when a piston moves it).
    pub fn remove_block(&mut self, pos: BlockPos, moved: bool) -> bool {
        let replacement = self.fluid_legacy_block(self.fluid_state(self.block(pos)));
        self.set_block(pos, replacement, update::ALL | if moved { update::MOVE_BY_PISTON } else { 0 }, update::LIMIT)
    }

    /// `Level.destroyBlock`: the block becomes its fluid (drops and the
    /// break event are left out).
    pub fn destroy_block(&mut self, pos: BlockPos, limit: i32) -> bool {
        let state = self.block(pos);
        if self.registries().blocks.is_air(state) {
            return false;
        }
        let replacement = self.fluid_legacy_block(self.fluid_state(state));
        self.set_block(pos, replacement, update::ALL, limit)
    }

    /// `ServerLevel.updateNeighboursOnBlockSet` (after commands): the old
    /// block's removal effects, then neighbour and comparator updates.
    pub fn update_neighbours_on_block_set(&mut self, pos: BlockPos, old: BlockStateId) {
        let state = self.block(pos);
        let block = self.block_id(state);
        if self.block_id(old) != block {
            self.affect_neighbors_after_removal(old, pos, false);
        }
        self.update_neighbors_at(pos, block);
        if self.has_analog_output(state) {
            self.update_neighbour_for_output_signal(pos, block);
        }
    }

    // ---- neighbour updates ---------------------------------------------------

    pub(crate) fn update_neighbors_at(&mut self, pos: BlockPos, block: BlockId) {
        self.add_and_run(Update::Neighbors { source: pos, block, skip: None, index: 0 });
    }

    fn update_neighbour_shapes(&mut self, state: BlockStateId, pos: BlockPos, flags: u32, limit: i32) {
        for direction in UPDATE_SHAPE_ORDER {
            let neighbor = pos.relative(direction, 1);
            self.add_and_run(Update::Shape { direction: direction.opposite(), neighbor_state: state, pos: neighbor, neighbor_pos: pos, flags, limit });
        }
    }

    /// `CollectingNeighborUpdater.addAndRun`.
    pub(crate) fn add_and_run(&mut self, update: Update) {
        let running = self.updates.count > 0;
        let too_many = self.updates.count >= MAX_CHAINED_NEIGHBOR_UPDATES;
        self.updates.count += 1;
        if !too_many {
            if running {
                self.updates.added.push(update);
            } else {
                self.updates.stack.push(update);
            }
        }
        if !running {
            self.run_updates();
        }
    }

    /// `CollectingNeighborUpdater.runUpdates`: depth first, updates added
    /// while one runs going before the rest of it.
    fn run_updates(&mut self) {
        while !self.updates.stack.is_empty() || !self.updates.added.is_empty() {
            let added: Vec<Update> = self.updates.added.drain(..).rev().collect();
            self.updates.stack.extend(added);
            let Some(mut next) = self.updates.stack.pop() else { break };
            loop {
                let more = self.run_next(&mut next);
                if !more {
                    break;
                }
                if !self.updates.added.is_empty() {
                    self.updates.stack.push(next);
                    break;
                }
            }
        }
        self.updates = Updater::default();
    }

    fn run_next(&mut self, update: &mut Update) -> bool {
        match update {
            Update::Shape { direction, neighbor_state, pos, neighbor_pos, flags, limit } => {
                let (direction, neighbor_state, pos, neighbor_pos, flags, limit) = (*direction, *neighbor_state, *pos, *neighbor_pos, *flags, *limit);
                self.execute_shape_update(direction, pos, neighbor_pos, neighbor_state, flags, limit);
                false
            }
            Update::Simple { pos, block } => {
                let (pos, block) = (*pos, *block);
                let state = self.block(pos);
                self.neighbor_changed(state, pos, block);
                false
            }
            Update::Full { state, pos, block } => {
                let (state, pos, block) = (*state, *pos, *block);
                self.neighbor_changed(state, pos, block);
                false
            }
            Update::Neighbors { source, block, skip, index } => {
                if *index == 0 && Some(UPDATE_ORDER[0]) == *skip {
                    *index = 1;
                }
                let direction = UPDATE_ORDER[*index];
                *index += 1;
                let neighbor = source.relative(direction, 1);
                let state = self.block(neighbor);
                let block = *block;
                self.neighbor_changed(state, neighbor, block);
                if *index < UPDATE_ORDER.len() && Some(UPDATE_ORDER[*index]) == *skip {
                    *index += 1;
                }
                *index < UPDATE_ORDER.len()
            }
        }
    }

    /// `NeighborUpdater.executeShapeUpdate`.
    fn execute_shape_update(&mut self, direction: Direction, pos: BlockPos, _neighbor_pos: BlockPos, neighbor_state: BlockStateId, flags: u32, limit: i32) {
        let state = self.block(pos);
        if flags & update::SKIP_SHAPE_UPDATE_ON_WIRE != 0 && self.name(state) == "minecraft:redstone_wire" {
            return;
        }
        let lib = self.lib;
        let new_state = {
            let mut ctx = Ctx { lib, region: self };
            minecraftoss_generator::feature::update::update_shape(&mut ctx, state, pos, direction, neighbor_state)
        };
        self.update_or_destroy(state, new_state, pos, flags, limit);
    }

    /// `Block.updateOrDestroy`.
    fn update_or_destroy(&mut self, old: BlockStateId, new: BlockStateId, pos: BlockPos, flags: u32, limit: i32) {
        if new == old {
            return;
        }
        if self.registries().blocks.is_air(new) {
            self.destroy_block_drops(pos, flags & update::SUPPRESS_DROPS == 0, limit);
        } else {
            self.set_block(pos, new, flags & !update::SUPPRESS_DROPS, limit);
        }
    }

    // ---- block behaviours -------------------------------------------------

    /// `BlockBehaviour.onPlace`.
    fn on_place(&mut self, state: BlockStateId, pos: BlockPos, old: BlockStateId, _moved: bool) {
        if let Some(kind) = self.redstone_kind(state) {
            self.redstone_on_place(kind, state, pos, old);
        } else if self.is_a(state, "LiquidBlock") {
            self.liquid_block_changed(state, pos);
        } else if self.is_a(state, "BaseFireBlock") {
            self.fire_on_place(state, pos, old);
        } else if self.is_a(state, "TntBlock") && self.block_id(old) != self.block_id(state) {
            self.tnt_block_changed(pos);
        } else if self.name(state) == "minecraft:sponge" {
            if self.block_id(old) != self.block_id(state) {
                self.sponge_try_absorb(pos);
            }
        } else if self.name(state) == "minecraft:wet_sponge" {
            self.wet_sponge_on_place(pos);
        } else if self.is_a(state, "CoralPlantBlock") || self.is_a(state, "CoralFanBlock") || self.is_a(state, "CoralWallFanBlock") {
            // `onPlace`: `tryScheduleDieTick` with the level random.
            let lib = self.lib;
            let block = self.block_id(state);
            let mut ctx: minecraftoss_generator::feature::Ctx = minecraftoss_generator::feature::Ctx { lib, region: self };
            minecraftoss_generator::feature::update::coral_try_schedule_die_tick(&mut ctx, state, pos, block);
        } else if self.is_a(state, "FallingBlock") || self.is_a(state, "BrushableBlock") {
            // `FallingBlock.onPlace`: a tick after `getDelayAfterPlace`.
            let block = self.block_id(state);
            self.schedule_block_tick_priority(pos, block, 2, redstone::priority::NORMAL);
        }
    }

    /// `BlockBehaviour.affectNeighborsAfterRemoval`.
    fn affect_neighbors_after_removal(&mut self, old: BlockStateId, pos: BlockPos, moved: bool) {
        if let Some(kind) = self.redstone_kind(old) {
            self.redstone_after_removal(kind, old, pos, moved);
        }
    }

    /// `BlockBehaviour.neighborChanged`.
    fn neighbor_changed(&mut self, state: BlockStateId, pos: BlockPos, source: BlockId) {
        if let Some(kind) = self.redstone_kind(state) {
            self.redstone_neighbor_changed(kind, state, pos, source);
        } else if self.is_a(state, "LiquidBlock") {
            self.liquid_block_changed(state, pos);
        } else if self.is_a(state, "TntBlock") {
            self.tnt_block_changed(pos);
        } else if self.name(state) == "minecraft:sponge" {
            self.sponge_try_absorb(pos);
        }
    }

    /// `LiquidBlock.onPlace` / `neighborChanged`: lava next to water turns
    /// to obsidian or cobblestone; otherwise the fluid ticks.
    fn liquid_block_changed(&mut self, state: BlockStateId, pos: BlockPos) {
        if self.should_spread_liquid(state, pos) {
            let delay = self.fluid_tick_delay(state);
            self.schedule_fluid_state_tick(pos, state, delay);
        }
    }

    // ---- ticks ---------------------------------------------------------------

    fn next_sub_tick(&mut self) -> i64 {
        let sub = self.sub_tick;
        self.sub_tick += 1;
        sub
    }

    /// `scheduleTick(pos, block, delay)` for the block of a state.
    pub fn schedule_block_tick(&mut self, pos: BlockPos, state: BlockStateId, delay: i32) {
        let block = self.block_id(state);
        self.schedule_block_tick_priority(pos, block, delay, redstone::priority::NORMAL);
    }

    /// `scheduleTick(pos, block, delay, priority)`.
    pub fn schedule_block_tick_priority(&mut self, pos: BlockPos, block: BlockId, delay: i32, priority: i32) {
        let sub = self.next_sub_tick();
        self.block_ticks.schedule(TickType::Block(block.0), (pos.x, pos.y, pos.z), self.game_time + i64::from(delay), priority, sub);
    }

    /// `getBlockTicks().willTickThisTick(pos, block)`.
    fn will_tick_this_tick(&self, pos: BlockPos, block: BlockId) -> bool {
        self.block_ticks.will_tick_this_tick(TickType::Block(block.0), (pos.x, pos.y, pos.z))
    }

    /// `getBlockTicks().hasScheduledTick(pos, block)`.
    pub fn has_block_tick_at(&self, pos: BlockPos, block: BlockId) -> bool {
        self.block_ticks.has_scheduled(TickType::Block(block.0), (pos.x, pos.y, pos.z))
    }

    /// `scheduleTick(pos, Fluids.WATER, 5)`.
    fn schedule_water_tick(&mut self, pos: BlockPos) {
        let sub = self.next_sub_tick();
        self.fluid_ticks.schedule(TickType::Fluid(ticks::FluidType::Water), (pos.x, pos.y, pos.z), self.game_time + 5, 0, sub);
    }

    /// `scheduleTick(pos, fluid, delay)` for the fluid of a state.
    fn schedule_fluid_state_tick(&mut self, pos: BlockPos, state: BlockStateId, delay: i32) {
        if let Some(kind) = self.fluid_state(state).map(|f| fluid::fluid_type(&f)) {
            let sub = self.next_sub_tick();
            self.fluid_ticks.schedule(TickType::Fluid(kind), (pos.x, pos.y, pos.z), self.game_time + i64::from(delay), 0, sub);
        }
    }

    pub fn pending_fluid_ticks(&self) -> usize {
        self.fluid_ticks.len()
    }

    /// One server tick of the world simulation: `tickTime`, then block and
    /// fluid ticks (`ServerLevel.tick`).
    pub fn tick(&mut self) {
        let mut phase = std::time::Instant::now();
        let mut lap = |i: usize, phases: &mut [f64; 6]| {
            let now = std::time::Instant::now();
            phases[i] = (now - phase).as_secs_f64() * 1000.0;
            phase = now;
        };
        let mut phases = [0.0; 6];
        // `MinecraftServer.tickChildren`: clocks, then the level's weather,
        // sky brightness and game time.
        self.tick_clocks();
        self.advance_weather_cycle();
        self.update_sky_brightness();
        self.game_time += 1;
        self.handling_tick = true;
        self.block_ticks.begin(self.game_time, MAX_TICKS);
        while let Some(tick) = self.block_ticks.next() {
            let pos = BlockPos::new(tick.pos.0, tick.pos.1, tick.pos.2);
            let TickType::Block(block) = tick.kind else { continue };
            let state = self.block(pos);
            if self.block_id(state).0 == block {
                self.tick_block(state, pos);
            }
        }
        lap(0, &mut phases);
        for tick in self.fluid_ticks.collect(self.game_time, MAX_TICKS) {
            let pos = BlockPos::new(tick.pos.0, tick.pos.1, tick.pos.2);
            let TickType::Fluid(kind) = tick.kind else { continue };
            let state = self.block(pos);
            if self.fluid_state(state).is_some_and(|f| fluid::fluid_type(&f) == kind) {
                self.tick_fluid(pos, state);
            }
        }
        lap(1, &mut phases);
        self.tick_chunks();
        lap(2, &mut phases);
        self.run_block_events();
        self.handling_tick = false;
        self.tick_entities();
        lap(3, &mut phases);
        self.tick_block_entities();
        lap(4, &mut phases);
        self.last_tick_phases = phases;
    }

    /// `BlockBehaviour.tick`.
    fn tick_block(&mut self, state: BlockStateId, pos: BlockPos) {
        if let Some(kind) = self.redstone_kind(state) {
            self.redstone_tick(kind, state, pos);
        } else if self.is_a(state, "FireBlock") {
            self.fire_tick(state, pos);
        } else if self.is_a(state, "FallingBlock") || self.is_a(state, "BrushableBlock") {
            self.falling_block_tick(state, pos);
        } else if self.is_a(state, "CoralPlantBlock") || self.is_a(state, "CoralFanBlock") || self.is_a(state, "CoralWallFanBlock") || self.is_a(state, "CoralBlock") {
            self.coral_tick(state, pos);
        } else if self.is_a(state, "FarmlandBlock") || self.is_a(state, "PathBlock") {
            self.covered_ground_tick(state, pos);
        } else if self.is_a(state, "ComposterBlock") {
            // `ComposterBlock.tick`: a composter at 7 is ready (8); its
            // sound is the client's.
            if self.registries().blocks.property(state, "level") == Some("7") {
                let ready = self.registries().blocks.with_property(state, "level", "8").unwrap_or(state);
                self.set_block_and_update(pos, ready);
            }
        } else if self.is_a(state, "LeavesBlock") {
            // `LeavesBlock.tick`: the distance to the nearest log.
            let mut distance = 7;
            for direction in Direction::ALL {
                let neighbor = self.block(pos.relative(direction, 1));
                let at = if self.lib.registries.block_in_tag(neighbor, self.lib.tags.logs) {
                    0
                } else if self.is_a(neighbor, "LeavesBlock") {
                    self.registries().blocks.property(neighbor, "distance").and_then(|d| d.parse().ok()).unwrap_or(7)
                } else {
                    7
                };
                distance = distance.min(at + 1);
                if distance == 1 {
                    break;
                }
            }
            let next = self.registries().blocks.with_property(state, "distance", &distance.to_string()).unwrap_or(state);
            self.set_block_and_update(pos, next);
        }
    }
}

impl World for Level<'_> {
    fn block_at(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.block(BlockPos::new(x, y, z))
    }

    fn set_block_with_flags(&mut self, _lib: &Library, pos: BlockPos, state: BlockStateId, flags: u32) -> bool {
        self.set_block(pos, state, flags, update::LIMIT)
    }

    fn schedule(&mut self, pos: BlockPos, fluid: bool, delay: i32) {
        let state = self.block(pos);
        if fluid {
            self.schedule_fluid_state_tick(pos, state, delay);
        } else {
            self.schedule_block_tick(pos, state, delay);
        }
    }

    fn lava_tick_delay(&self) -> i32 {
        if self.fast_lava { 10 } else { 30 }
    }

    fn random(&mut self) -> &mut AnyRandom {
        &mut self.random
    }

    fn has_block_tick(&self, pos: BlockPos, block: BlockId) -> bool {
        self.has_block_tick_at(pos, block)
    }

    fn schedule_fluid(&mut self, pos: BlockPos, state: BlockStateId, delay: i32) {
        self.schedule_fluid_state_tick(pos, state, delay);
    }

    fn schedule_block(&mut self, pos: BlockPos, block: BlockId, delay: i32) {
        self.schedule_block_tick_priority(pos, block, delay, redstone::priority::NORMAL);
    }

    fn comparator_output(&self, pos: BlockPos) -> i32 {
        self.comparator_output_at(pos)
    }

    fn world_seed(&self) -> i64 {
        self.random_sequences.world_seed
    }

    fn light(&self, pos: BlockPos, darkening: i32) -> Option<(i32, bool)> {
        let sky = self.solved_sky_light(pos)?;
        let block = self.solved_block_light(pos)?;
        Some(((sky - darkening).max(block), sky >= 15))
    }

    fn min_y(&self) -> i32 {
        self.min_y
    }

    fn max_y(&self) -> i32 {
        self.min_y + self.height - 1
    }

    fn height_at(&self, kind: minecraftoss_core::chunk::HeightmapKind, x: i32, z: i32) -> i32 {
        self.chunk(ChunkPos::new(x >> 4, z >> 4)).map_or(self.min_y, |c| c.heightmaps.get(kind, (x & 15) as usize, (z & 15) as usize))
    }

    fn biome(&self, x: i32, y: i32, z: i32) -> Option<minecraftoss_core::BiomeId> {
        let zoom = minecraftoss_generator::zoom::zoom_seed(self.random_sequences.world_seed);
        let [qx, qy, qz] = minecraftoss_generator::zoom::quart_for_block(zoom, x, y, z);
        let chunk = self.chunk(ChunkPos::new(qx >> 2, qz >> 2))?;
        Some(chunk.biome((qx & 3) as usize, qy, (qz & 3) as usize))
    }

    fn note_unsupported(&mut self, kind: &str) {
        self.unsupported.push(kind.to_owned());
    }
}
