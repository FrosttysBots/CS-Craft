//! The chunks one FEATURES step may touch (`WorldGenRegion` for the
//! `FEATURES` chunk step: dependencies and write radius of one chunk).

use crate::zoom;
use minecraftoss_core::chunk::{ChunkStatus, HeightmapKind};
use minecraftoss_core::nbt::Tag;
use minecraftoss_core::random::AnyRandom;
use std::collections::BTreeMap;
use minecraftoss_core::{BiomeId, BlockStateId, Chunk, ChunkPos, Registries};
use std::sync::Arc;

/// The fluid registry ID of a block state's fluid (`FluidState.getType`).
pub fn fluid_id(fluid: Option<&minecraftoss_core::block::FluidInfo>) -> &'static str {
    match fluid {
        Some(f) => match (matches!(f.kind, minecraftoss_core::block::FluidKind::Water), f.source) {
            (true, true) => "minecraft:water",
            (true, false) => "minecraft:flowing_water",
            (false, true) => "minecraft:lava",
            (false, false) => "minecraft:flowing_lava",
        },
        None => "minecraft:empty",
    }
}

/// A 3x3 chunk area centred on the chunk being decorated.
pub struct Region {
    center: ChunkPos,
    /// `(dz + 1) * 3 + (dx + 1)`.
    chunks: Vec<Chunk>,
    registries: Arc<Registries>,
    /// `getBiome` lookups keep the last 4x4x4 cell's zoom offsets.
    zoom: std::cell::RefCell<zoom::ZoomCache>,
    world_seed: i64,
    void_air: BlockStateId,
    min_y: i32,
    height: i32,
    /// Positions `WorldGenRegion.markPosForPostProcessing` recorded, per chunk.
    post_processing: [Vec<(i32, i32, i32)>; 9],
    /// `WorldGenRegion.getRandom()`: the `worldgen_region_random` stream at the center.
    level_random: AnyRandom,
    /// Feature types placed here that this engine does not implement.
    unsupported: Vec<String>,
    /// Entities features and structures added, as saved, in order.
    entities: Vec<Tag>,
    /// Entities created here, for their UUIDs.
    created: u32,
}

impl Region {
    /// Takes the nine chunks in `(dz + 1) * 3 + (dx + 1)` order.
    pub fn new(center: ChunkPos, chunks: Vec<Chunk>, registries: Arc<Registries>, world_seed: i64) -> Self {
        assert_eq!(chunks.len(), 9, "a FEATURES region has nine chunks");
        for (i, chunk) in chunks.iter().enumerate() {
            let expected = ChunkPos::new(center.x + i as i32 % 3 - 1, center.z + i as i32 / 3 - 1);
            assert_eq!(chunk.pos, expected, "region chunk out of order");
        }
        let (min_y, height) = (chunks[4].min_y(), chunks[4].height());
        let void_air = registries.blocks.parse_state("minecraft:void_air").expect("void_air exists");
        Self {
            center,
            chunks,
            registries,
            zoom: std::cell::RefCell::new(zoom::ZoomCache::new(zoom::zoom_seed(world_seed))),
            world_seed,
            void_air,
            min_y,
            height,
            post_processing: Default::default(),
            level_random: AnyRandom::new(false, 0),
            unsupported: Vec::new(),
            entities: Vec::new(),
            created: 0,
        }
    }

    /// The chunks, with this region's post-processing marks, ticks and
    /// descriptors appended to each chunk's generation data.
    pub fn into_chunks(self) -> Vec<Chunk> {
        let mut chunks = self.chunks;
        let (x0, z0) = (self.center.x - 1, self.center.z - 1);
        for (i, chunk) in chunks.iter_mut().enumerate() {
            chunk.generation.post_processing.extend_from_slice(&self.post_processing[i]);
        }
        // `WorldGenRegion.addFreshEntity`: the chunk holding the entity's position.
        for entity in self.entities {
            let Some(pos) = entity.get("Pos").and_then(Tag::as_list) else { continue };
            let block = |i: usize| pos.get(i).and_then(Tag::as_f64).map_or(0, |v| v.floor() as i32);
            let (dx, dz) = ((block(0) >> 4) - x0, (block(2) >> 4) - z0);
            if (0..3).contains(&dx) && (0..3).contains(&dz) {
                chunks[(dz * 3 + dx) as usize].generation.entities.push(entity);
            }
        }
        chunks
    }

    /// `WorldGenLevel.getSeed`.
    pub fn world_seed(&self) -> i64 {
        self.world_seed
    }

    pub fn center(&self) -> ChunkPos {
        self.center
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    pub fn registries(&self) -> &Registries {
        &self.registries
    }

    pub fn min_y(&self) -> i32 {
        self.min_y
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// `LevelHeightAccessor.getMaxY`: the highest buildable Y.
    pub fn max_y(&self) -> i32 {
        self.min_y + self.height - 1
    }

    pub fn is_outside_build_height(&self, y: i32) -> bool {
        y < self.min_y || y > self.max_y()
    }

    pub fn post_processing(&self) -> &[Vec<(i32, i32, i32)>; 9] {
        &self.post_processing
    }

    fn index(&self, x: i32, z: i32) -> Option<usize> {
        let (dx, dz) = ((x >> 4) - self.center.x, (z >> 4) - self.center.z);
        ((-1..=1).contains(&dx) && (-1..=1).contains(&dz)).then(|| ((dz + 1) * 3 + dx + 1) as usize)
    }

    /// Whether a position is in the region. Vanilla crashes reading outside it.
    pub fn contains(&self, x: i32, z: i32) -> bool {
        self.index(x, z).is_some()
    }

    /// A block in the region. Beyond it vanilla reads chunks at
    /// STRUCTURE_STARTS, which hold no blocks yet: air.
    pub fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        if self.is_outside_build_height(y) {
            // ProtoChunk.getBlockState outside the build height.
            return self.void_air;
        }
        match self.index(x, z) {
            Some(i) => self.chunks[i].block((x & 15) as usize, y, (z & 15) as usize),
            None => self.registries.plain_air,
        }
    }

    /// Sets the region random (`RandomState.getOrCreateRandomFactory("worldgen_region_random").at(center)`).
    pub fn set_level_random(&mut self, random: AnyRandom) {
        self.level_random = random;
    }

    pub fn level_random(&mut self) -> &mut AnyRandom {
        &mut self.level_random
    }

    pub fn note_unsupported(&mut self, kind: &str) {
        if !self.unsupported.iter().any(|k| k == kind) {
            self.unsupported.push(kind.to_owned());
        }
    }

    /// `WorldGenRegion.addFreshEntity`: an entity, as saved.
    pub fn add_entity(&mut self, entity: Tag) {
        self.entities.push(entity);
    }

    /// Entities added so far.
    pub fn entities(&self) -> &[Tag] {
        &self.entities
    }

    /// A UUID for a created entity. Vanilla's comes from the entity's own
    /// unseeded random; this one is a hash of the world seed, the region and
    /// a counter, so generation stays deterministic.
    pub fn next_uuid(&mut self) -> [i32; 4] {
        self.created += 1;
        let mut state = (self.world_seed as u64) ^ (self.center.pack() as u64).rotate_left(17) ^ u64::from(self.created).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let (a, b) = (next(), next());
        // Version 4, IETF variant, as `Mth.createInsecureUUID`.
        let a = (a & !0xF000) | 0x4000;
        let b = (b & 0x3FFF_FFFF_FFFF_FFFF) | 0x8000_0000_0000_0000;
        [(a >> 32) as i32, a as i32, (b >> 32) as i32, b as i32]
    }

    fn block_name(&self, i: usize, x: i32, y: i32, z: i32) -> String {
        let state = self.chunks[i].block((x & 15) as usize, y, (z & 15) as usize);
        self.registries.blocks.block(self.registries.blocks.block_of(state)).name.as_str().to_owned()
    }

    /// `WorldGenRegion.getBlockEntity`: the saved form of the block entity at
    /// a position, made from its pending tag on first access. `None` outside
    /// the region, without a block entity, or without the block entity
    /// catalog.
    pub fn block_entity_mut(&mut self, x: i32, y: i32, z: i32) -> Option<&mut Tag> {
        let i = self.index(x, z)?;
        let block = self.block_name(i, x, y, z);
        let catalog = self.registries.block_entities.as_ref()?;
        catalog.get_mut(&mut self.chunks[i].block_entities, (x, y, z), &block)
    }

    /// The type of the block entity at a position, made as `block_entity_mut` does.
    pub fn block_entity_type(&mut self, x: i32, y: i32, z: i32) -> Option<String> {
        self.block_entity_mut(x, y, z).and_then(|nbt| nbt.get("id").and_then(Tag::as_str).map(str::to_owned))
    }

    fn typed_block_entity(&mut self, x: i32, y: i32, z: i32, kind: &str) -> Option<&mut BTreeMap<String, Tag>> {
        match self.block_entity_mut(x, y, z) {
            Some(Tag::Compound(map)) if map.get("id").and_then(Tag::as_str) == Some(kind) => Some(map),
            _ => None,
        }
    }

    /// `RandomizableContainer.setBlockEntityLootTable` with the drawn seed
    /// (callers draw it only for containers).
    pub fn set_loot_table(&mut self, x: i32, y: i32, z: i32, table: &str, seed: i64) {
        if let Some(nbt) = self.block_entity_mut(x, y, z) {
            minecraftoss_core::block_entity::set_loot_table(nbt, table, seed);
        }
    }

    /// `BrushableBlockEntity.setLootTable`.
    pub fn set_brushable_loot(&mut self, x: i32, y: i32, z: i32, table: &str, seed: i64) {
        if self.typed_block_entity(x, y, z, "minecraft:brushable_block").is_some() {
            self.set_loot_table(x, y, z, table, seed);
        }
    }

    /// `BaseSpawner.setEntityId`: the next spawn's entity type.
    pub fn set_spawner_entity(&mut self, x: i32, y: i32, z: i32, entity: &str) {
        let Some(map) = self.typed_block_entity(x, y, z, "minecraft:mob_spawner") else { return };
        let data = map.entry("SpawnData".to_owned()).or_insert_with(|| Tag::Compound(BTreeMap::new()));
        if let Tag::Compound(data) = data {
            let spawned = data.entry("entity".to_owned()).or_insert_with(|| Tag::Compound(BTreeMap::new()));
            if let Tag::Compound(spawned) = spawned {
                spawned.insert("id".to_owned(), Tag::String(entity.to_owned()));
            }
        }
    }

    /// `BeehiveBlockEntity.storeBee(Occupant.create(ticks))` for each value.
    pub fn add_bees(&mut self, x: i32, y: i32, z: i32, ticks_in_hive: &[i32]) {
        let Some(map) = self.typed_block_entity(x, y, z, "minecraft:beehive") else { return };
        if let Tag::List(bees) = map.entry("bees".to_owned()).or_insert_with(|| Tag::List(Vec::new())) {
            for &ticks in ticks_in_hive {
                let mut entity = BTreeMap::new();
                entity.insert("id".to_owned(), Tag::String("minecraft:bee".to_owned()));
                let mut bee = BTreeMap::new();
                bee.insert("entity_data".to_owned(), Tag::Compound(entity));
                bee.insert("ticks_in_hive".to_owned(), Tag::Int(ticks));
                bee.insert("min_ticks_in_hive".to_owned(), Tag::Int(600));
                bees.push(Tag::Compound(bee));
            }
        }
    }

    /// `TheEndGatewayBlockEntity.setExitPosition`.
    pub fn set_gateway_exit(&mut self, x: i32, y: i32, z: i32, exit: (i32, i32, i32), exact: bool) {
        let Some(map) = self.typed_block_entity(x, y, z, "minecraft:end_gateway") else { return };
        map.insert("exit_portal".to_owned(), Tag::IntArray(vec![exit.0, exit.1, exit.2]));
        if exact {
            map.insert("ExactTeleport".to_owned(), Tag::Byte(1));
        } else {
            map.remove("ExactTeleport");
        }
    }

    /// `BlockEntity.loadWithComponents` of a structure template tag into the
    /// block entity at a position.
    pub fn load_block_entity(&mut self, x: i32, y: i32, z: i32, nbt: &Tag) {
        let Some(i) = self.index(x, z) else { return };
        let block = self.block_name(i, x, y, z);
        let Some(catalog) = self.registries.block_entities.as_ref() else { return };
        if catalog.get_mut(&mut self.chunks[i].block_entities, (x, y, z), &block).is_some() {
            if let Some(loaded) = catalog.load_and_save(&block, (x, y, z), nbt) {
                self.chunks[i].block_entities.entities.insert((x, y, z), loaded);
            }
        }
    }

    pub fn unsupported(&self) -> &[String] {
        &self.unsupported
    }

    /// `LevelAccessor.scheduleTick` into the chunk's `ProtoChunkTicks`: the
    /// block at the position, or its fluid, with normal priority.
    pub fn schedule_tick(&mut self, x: i32, y: i32, z: i32, fluid: bool) {
        let Some(i) = self.index(x, z) else { return };
        let state = self.block(x, y, z);
        let blocks = &self.registries.blocks;
        let id = if fluid {
            fluid_id(blocks.state(state).fluid.as_ref()).to_owned()
        } else {
            blocks.block(blocks.block_of(state)).name.as_str().to_owned()
        };
        let tick = minecraftoss_core::chunk::ScheduledTick { pos: (x, y, z), fluid, id, delay: 0, priority: 0 };
        self.chunks[i].generation.schedule_tick(tick);
    }

    /// `ProtoChunkTicks.hasScheduledTick` for the block ticks of a chunk.
    pub fn has_block_tick(&self, x: i32, y: i32, z: i32, block: &str) -> bool {
        let Some(i) = self.index(x, z) else { return false };
        self.chunks[i].generation.ticks.iter().any(|t| !t.fluid && t.pos == (x, y, z) && t.id == block)
    }

    /// `WorldGenRegion.setBlock` into a `ProtoChunk` at `TERRAIN`: plain air
    /// into an all-air section is dropped, and the final heightmaps follow.
    /// Returns false only outside the write radius.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, state: BlockStateId) -> bool {
        let Some(i) = self.index(x, z) else {
            return false;
        };
        if self.is_outside_build_height(y) {
            return true;
        }
        let registries = self.registries.clone();
        let chunk = &mut self.chunks[i];
        let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
        let section = &chunk.sections()[((y >> 4) - chunk.min_section_y()) as usize];
        if section.is_empty() && state == registries.plain_air {
            return true;
        }
        let old = chunk.set_block_raw(lx, y, lz, state, &registries).unwrap_or(BlockStateId::AIR);
        for &kind in ChunkStatus::Terrain.heightmaps() {
            chunk.update_heightmap(kind, lx, y, lz, state, &registries);
        }
        // A proto chunk records a DUMMY tag; an existing block entity stays.
        let store = &mut self.chunks[i].block_entities;
        if registries.blocks.is(state, minecraftoss_core::block::flags::HAS_BLOCK_ENTITY) {
            store.pending.insert((x, y, z), minecraftoss_core::block_entity::dummy((x, y, z)));
        } else if registries.blocks.is(old, minecraftoss_core::block::flags::HAS_BLOCK_ENTITY) {
            store.pending.remove(&(x, y, z));
            store.entities.remove(&(x, y, z));
        }
        true
    }

    /// `LevelChunkSection.setBlockState` through `BulkSectionAccess`: no
    /// heightmap, block-entity or post-processing side effects.
    pub fn set_block_section(&mut self, x: i32, y: i32, z: i32, state: BlockStateId) -> bool {
        let Some(i) = self.index(x, z) else {
            return false;
        };
        if self.is_outside_build_height(y) {
            return false;
        }
        let registries = self.registries.clone();
        self.chunks[i].set_block_raw((x & 15) as usize, y, (z & 15) as usize, state, &registries);
        true
    }

    /// `ChunkAccess.markPosForPostProcessing`.
    pub fn mark_post_processing(&mut self, x: i32, y: i32, z: i32) {
        if let Some(i) = self.index(x, z) {
            if !self.is_outside_build_height(y) {
                self.post_processing[i].push((x, y, z));
            }
        }
    }

    /// Whether any column in the block rectangle has `height_at >= y`.
    /// Chunks and 4x4 column groups that never reached `y` are skipped.
    pub fn any_height_at_least(&self, kind: HeightmapKind, min_x: i32, min_z: i32, max_x: i32, max_z: i32, y: i32) -> bool {
        for cz in (min_z >> 4)..=(max_z >> 4) {
            for cx in (min_x >> 4)..=(max_x >> 4) {
                let Some(i) = self.index(cx << 4, cz << 4) else {
                    // Outside the region every column reads as the minimum Y.
                    if self.min_y >= y {
                        return true;
                    }
                    continue;
                };
                let heights = &self.chunks[i].heightmaps;
                if heights.upper_bound(kind) < y {
                    continue;
                }
                let (x0, x1) = ((min_x.max(cx << 4) & 15) as usize, (max_x.min((cx << 4) + 15) & 15) as usize);
                let (z0, z1) = ((min_z.max(cz << 4) & 15) as usize, (max_z.min((cz << 4) + 15) & 15) as usize);
                for gz in (z0 >> 2)..=(z1 >> 2) {
                    for gx in (x0 >> 2)..=(x1 >> 2) {
                        if heights.group_upper_bound(kind, gx << 2, gz << 2) < y {
                            continue;
                        }
                        for z in z0.max(gz << 2)..=z1.min((gz << 2) + 3) {
                            for x in x0.max(gx << 2)..=x1.min((gx << 2) + 3) {
                                if heights.get(kind, x, z) >= y {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// `WorldGenRegion.getHeight`: the first free Y above the heightmap.
    pub fn height_at(&self, kind: HeightmapKind, x: i32, z: i32) -> i32 {
        match self.index(x, z) {
            Some(i) => self.chunks[i].heightmaps.get(kind, (x & 15) as usize, (z & 15) as usize),
            None => self.min_y,
        }
    }

    /// `LevelReader.getNoiseBiome` from the region's chunks.
    pub fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> Option<BiomeId> {
        let i = self.index(qx << 2, qz << 2)?;
        Some(self.chunks[i].biome((qx & 3) as usize, qy, (qz & 3) as usize))
    }

    /// `LevelReader.getBiome`: the zoomed biome at a block.
    pub fn biome(&self, x: i32, y: i32, z: i32) -> Option<BiomeId> {
        let [qx, qy, qz] = self.zoom.borrow_mut().quart(x, y, z);
        self.noise_biome(qx, qy, qz)
    }
}
