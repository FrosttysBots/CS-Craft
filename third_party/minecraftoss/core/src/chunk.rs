//! Chunk columns: sections, biomes, heightmaps and generation status.

use crate::biome::BiomeId;
use crate::block::BlockStateId;
use crate::palette::PalettedContainer;
use crate::pos::ChunkPos;
use crate::registries::Registries;

/// Vanilla 26.3 `ChunkStatus`, in generation order. 26.3 merged the older
/// NOISE, SURFACE and CARVERS steps into one TERRAIN step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChunkStatus {
    Empty,
    StructureStarts,
    StructureReferences,
    Biomes,
    Terrain,
    Features,
    InitializeLight,
    Light,
    Spawn,
    Full,
}

impl ChunkStatus {
    pub const ALL: [Self; 10] = [
        Self::Empty,
        Self::StructureStarts,
        Self::StructureReferences,
        Self::Biomes,
        Self::Terrain,
        Self::Features,
        Self::InitializeLight,
        Self::Light,
        Self::Spawn,
        Self::Full,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub fn next(self) -> Option<Self> {
        Self::ALL.get(self.index() + 1).copied()
    }

    /// Heightmaps a proto chunk keeps at this status (`ChunkStatus.heightmapsAfter`).
    pub fn heightmaps(self) -> &'static [HeightmapKind] {
        if self < Self::Terrain {
            &HeightmapKind::WORLDGEN
        } else {
            &HeightmapKind::FINAL
        }
    }
}

/// Vanilla `Heightmap.Types`, in ID order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeightmapKind {
    WorldSurfaceWg,
    WorldSurface,
    OceanFloorWg,
    OceanFloor,
    MotionBlocking,
    MotionBlockingNoLeaves,
}

impl HeightmapKind {
    pub const ALL: [Self; 6] = [
        Self::WorldSurfaceWg,
        Self::WorldSurface,
        Self::OceanFloorWg,
        Self::OceanFloor,
        Self::MotionBlocking,
        Self::MotionBlockingNoLeaves,
    ];
    /// Types kept before `TERRAIN` (`ChunkStatus.WORLDGEN_HEIGHTMAPS`).
    pub const WORLDGEN: [Self; 2] = [Self::OceanFloorWg, Self::WorldSurfaceWg];
    /// Types kept from `TERRAIN` on (`ChunkStatus.FINAL_HEIGHTMAPS`).
    pub const FINAL: [Self; 4] = [
        Self::WorldSurface,
        Self::OceanFloor,
        Self::MotionBlocking,
        Self::MotionBlockingNoLeaves,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    /// Bit in `Registries::heightmap_mask`.
    pub const fn bit(self) -> u8 {
        1 << self.index()
    }
}

pub const SECTION_VOLUME: usize = 4096;
pub const SECTION_BIOMES: usize = 64;

/// Index of a block within a section, as in vanilla `PalettedContainer` (y, z, x).
pub const fn block_index(x: usize, y: usize, z: usize) -> usize {
    (y << 8) | (z << 4) | x
}

/// Index of a 4x4x4 biome cell within a section.
pub const fn biome_index(x: usize, y: usize, z: usize) -> usize {
    (y << 4) | (z << 2) | x
}

#[derive(Clone, Debug)]
pub struct ChunkSection {
    /// Written through `set_block`, which keeps `non_air` and the light
    /// emitter cache right; bulk writers must reset both.
    pub blocks: PalettedContainer<BlockStateId, SECTION_VOLUME>,
    pub biomes: PalettedContainer<BiomeId, SECTION_BIOMES>,
    non_air: u16,
    /// Block indices of light-emitting states, found once per content
    /// (each section is lit as part of nine chunks' neighbourhoods).
    pub(crate) emitters: std::sync::OnceLock<Box<[u16]>>,
}

impl ChunkSection {
    pub fn empty(biome: BiomeId) -> Self {
        Self {
            blocks: PalettedContainer::Single(BlockStateId::AIR),
            biomes: PalettedContainer::Single(biome),
            non_air: 0,
            emitters: std::sync::OnceLock::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.non_air == 0
    }

    pub fn block(&self, x: usize, y: usize, z: usize) -> BlockStateId {
        self.blocks.get(block_index(x, y, z))
    }

    /// Writes one block and returns the previous state.
    pub fn set_block(
        &mut self,
        x: usize,
        y: usize,
        z: usize,
        state: BlockStateId,
        registries: &Registries,
    ) -> BlockStateId {
        let previous = self.blocks.set(block_index(x, y, z), state);
        if previous != state && self.emitters.get().is_some() {
            self.emitters = std::sync::OnceLock::new();
        }
        let was_air = registries.blocks.is_air(previous);
        let is_air = registries.blocks.is_air(state);
        if was_air && !is_air {
            self.non_air += 1;
        } else if !was_air && is_air {
            self.non_air -= 1;
        }
        previous
    }

    /// Recounts non-air blocks after bulk writes to `blocks`.
    pub fn recount(&mut self, registries: &Registries) {
        self.non_air = match &self.blocks {
            PalettedContainer::Single(s) => {
                if registries.blocks.is_air(*s) {
                    0
                } else {
                    SECTION_VOLUME as u16
                }
            }
            PalettedContainer::Direct(values) => values
                .iter()
                .filter(|&&s| !registries.blocks.is_air(s))
                .count() as u16,
        };
    }
}

/// Six heightmaps; each column stores vanilla's "first available" Y.
#[derive(Clone, Debug)]
pub struct Heightmaps {
    first_available: [[i32; 256]; 6],
    /// Per heightmap, the highest value any column has held, over the
    /// chunk and over each 4x4 group of columns (`(z / 4) * 4 + x / 4`).
    upper: [i32; 6],
    group_upper: [[i32; 16]; 6],
}

impl Heightmaps {
    fn new(min_y: i32) -> Self {
        Self {
            first_available: [[min_y; 256]; 6],
            upper: [min_y; 6],
            group_upper: [[min_y; 16]; 6],
        }
    }

    pub fn get(&self, kind: HeightmapKind, x: usize, z: usize) -> i32 {
        self.first_available[kind.index()][z * 16 + x]
    }

    /// No column of `kind` is higher than this (it can be higher than every
    /// column once heights have dropped).
    pub fn upper_bound(&self, kind: HeightmapKind) -> i32 {
        self.upper[kind.index()]
    }

    /// `upper_bound` for the 4x4 group of columns holding `(x, z)`.
    pub fn group_upper_bound(&self, kind: HeightmapKind, x: usize, z: usize) -> i32 {
        self.group_upper[kind.index()][(z >> 2) * 4 + (x >> 2)]
    }

    pub(crate) fn set(&mut self, kind: HeightmapKind, x: usize, z: usize, height: i32) {
        let k = kind.index();
        self.first_available[k][z * 16 + x] = height;
        self.upper[k] = self.upper[k].max(height);
        let group = &mut self.group_upper[k][(z >> 2) * 4 + (x >> 2)];
        *group = (*group).max(height);
    }
}

/// What generation leaves in a proto chunk besides blocks, biomes and
/// heightmaps, for its FULL state (`ProtoChunk` post-processing marks, tick
/// lists and pending block entities, and entities generation spawned).
#[derive(Clone, Debug, Default)]
pub struct GenerationData {
    /// `markPosForPostProcessing` world positions in mark order, duplicates
    /// kept. Vanilla keeps one list per section; process them section by
    /// section in this order.
    pub post_processing: Vec<(i32, i32, i32)>,
    /// Scheduled ticks in schedule order (`ProtoChunkTicks`).
    pub ticks: Vec<ScheduledTick>,
    /// Entities generation created, as saved (`ProtoChunk.getEntities`).
    pub entities: Vec<crate::nbt::Tag>,
}

/// A tick a proto chunk keeps (`SavedTick`): what was scheduled (a block
/// or fluid ID), where, the delay (0 in proto chunks) and the priority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledTick {
    pub pos: (i32, i32, i32),
    pub fluid: bool,
    pub id: String,
    pub delay: i32,
    pub priority: i32,
}

impl GenerationData {
    /// `ProtoChunkTicks.schedule`: one tick per type and position, the
    /// first kept. Returns whether the tick was added.
    pub fn schedule_tick(&mut self, tick: ScheduledTick) -> bool {
        let duplicate = self.ticks.iter().any(|t| t.pos == tick.pos && t.fluid == tick.fluid && t.id == tick.id);
        if !duplicate {
            self.ticks.push(tick);
        }
        !duplicate
    }
}

/// A chunk column at any generation status.
#[derive(Clone, Debug)]
pub struct Chunk {
    pub pos: ChunkPos,
    pub status: ChunkStatus,
    min_section_y: i32,
    sections: Vec<ChunkSection>,
    pub heightmaps: Heightmaps,
    pub generation: GenerationData,
    pub block_entities: BlockEntityStore,
    /// Block and sky light once the chunk has been lit (`isLightOn`).
    pub light: Option<std::sync::Arc<crate::light::ChunkLight>>,
    /// `ChunkAccess.inhabitedTime`: ticks spent spawning around players.
    pub inhabited_time: i64,
}

/// A chunk's block entity data, in saved form (`BlockEntity.saveWithFullMetadata`).
/// Proto chunks hold `pendingBlockEntities` (tags, `{"id": "DUMMY"}` for a
/// block placed during generation) beside the block entities that exist;
/// FULL chunks hold block entities only once pending tags are promoted
/// (`crate::block_entity::promote`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockEntityStore {
    pub pending: std::collections::BTreeMap<(i32, i32, i32), crate::nbt::Tag>,
    pub entities: std::collections::BTreeMap<(i32, i32, i32), crate::nbt::Tag>,
}

impl Chunk {
    pub fn new(pos: ChunkPos, min_y: i32, height: i32, biome: BiomeId) -> Self {
        assert!(
            min_y % 16 == 0 && height > 0 && height % 16 == 0,
            "chunk bounds must be section aligned"
        );
        Self {
            pos,
            status: ChunkStatus::Empty,
            min_section_y: min_y >> 4,
            sections: (0..height / 16)
                .map(|_| ChunkSection::empty(biome))
                .collect(),
            heightmaps: Heightmaps::new(min_y),
            generation: GenerationData::default(),
            block_entities: BlockEntityStore::default(),
            light: None,
            inhabited_time: 0,
        }
    }

    pub fn min_y(&self) -> i32 {
        self.min_section_y << 4
    }

    pub fn height(&self) -> i32 {
        self.sections.len() as i32 * 16
    }

    pub fn min_section_y(&self) -> i32 {
        self.min_section_y
    }

    pub fn sections(&self) -> &[ChunkSection] {
        &self.sections
    }

    pub fn sections_mut(&mut self) -> &mut [ChunkSection] {
        &mut self.sections
    }

    fn section_index(&self, y: i32) -> Option<usize> {
        usize::try_from((y >> 4) - self.min_section_y)
            .ok()
            .filter(|&i| i < self.sections.len())
    }

    /// Block at chunk-local X/Z and world Y; outside the height range is air.
    pub fn block(&self, x: usize, y: i32, z: usize) -> BlockStateId {
        match self.section_index(y) {
            Some(i) => self.sections[i].block(x, (y & 15) as usize, z),
            None => BlockStateId::AIR,
        }
    }

    /// Writes a block without heightmap maintenance; returns the previous state, or `None` outside the range.
    pub fn set_block_raw(
        &mut self,
        x: usize,
        y: i32,
        z: usize,
        state: BlockStateId,
        registries: &Registries,
    ) -> Option<BlockStateId> {
        let i = self.section_index(y)?;
        Some(self.sections[i].set_block(x, (y & 15) as usize, z, state, registries))
    }

    /// Biome at quart (4-block) coordinates local to the chunk; Y is a world quart.
    pub fn biome(&self, qx: usize, qy: i32, qz: usize) -> BiomeId {
        let qy = qy.clamp(
            self.min_section_y * 4,
            (self.min_section_y + self.sections.len() as i32) * 4 - 1,
        );
        let i = self.section_index(qy << 2).expect("clamped into range");
        self.sections[i]
            .biomes
            .get(crate::chunk::biome_index(qx, (qy & 3) as usize, qz))
    }

    /// Vanilla `ChunkAccess.getHighestSectionPosition`: bottom Y of the highest non-empty section, or min Y.
    pub fn highest_section_position(&self) -> i32 {
        match self.sections.iter().rposition(|s| !s.is_empty()) {
            Some(i) => (self.min_section_y + i as i32) << 4,
            None => self.min_y(),
        }
    }

    /// Vanilla `Heightmap.primeHeightmaps`: rescans columns downward for the given kinds.
    pub fn prime_heightmaps(&mut self, kinds: &[HeightmapKind], registries: &Registries) {
        let air = registries.plain_air;
        let top = self.highest_section_position() + 16;
        let min_y = self.min_y();
        for x in 0..16 {
            for z in 0..16 {
                let mut remaining: u8 = kinds.iter().fold(0, |m, k| m | k.bit());
                for &kind in kinds {
                    self.heightmaps.set(kind, x, z, min_y);
                }
                let mut y = top - 1;
                while y >= min_y && remaining != 0 {
                    let state = self.block(x, y, z);
                    if state != air {
                        let hits = registries.heightmap_mask(state) & remaining;
                        for &kind in kinds {
                            if hits & kind.bit() != 0 {
                                self.heightmaps.set(kind, x, z, y + 1);
                            }
                        }
                        remaining &= !hits;
                    }
                    y -= 1;
                }
            }
        }
    }

    /// Vanilla `Heightmap.update` for one kind after a block change at local X/Z and world Y.
    pub fn update_heightmap(
        &mut self,
        kind: HeightmapKind,
        x: usize,
        y: i32,
        z: usize,
        state: BlockStateId,
        registries: &Registries,
    ) -> bool {
        let first_available = self.heightmaps.get(kind, x, z);
        if y <= first_available - 2 {
            return false;
        }
        if registries.heightmap_mask(state) & kind.bit() != 0 {
            if y >= first_available {
                self.heightmaps.set(kind, x, z, y + 1);
                return true;
            }
        } else if first_available - 1 == y {
            for below in (self.min_y()..y).rev() {
                if registries.heightmap_mask(self.block(x, below, z)) & kind.bit() != 0 {
                    self.heightmaps.set(kind, x, z, below + 1);
                    return true;
                }
            }
            self.heightmaps.set(kind, x, z, self.min_y());
            return true;
        }
        false
    }
}
