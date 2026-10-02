//! Structures (vanilla `Structure`, `StructureSet`, `StructureStart`,
//! `ChunkGenerator.createStructures`/`createReferences`, `StructureManager`
//! and the structure part of `applyBiomeDecoration`).
//!
//! Source-informed from the pinned 26.3 JAR. Starts are a pure function of
//! the seed and chunk, so they are computed on first use and kept, one
//! shared instance per chunk: pieces may change while they are placed
//! (scattered features settle on the ground once), exactly like vanilla's
//! in-memory starts. References are the starts within eight chunks whose
//! bounds reach a chunk, visited in fastutil `LongOpenHashSet` order.

pub mod beardifier;
pub mod jigsaw;
pub mod kinds;
pub mod piece;
pub mod placement;

use crate::feature::template::BoundingBox;
use crate::feature::{Ctx, Library};
use crate::terrain::TerrainGenerator;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::ident::Identifier;
use minecraftoss_core::random::{LegacyRandom, WorldgenRandom};
use minecraftoss_core::{BlockPos, BlockStateId, ChunkPos, Registries};
use piece::Piece;
use placement::{Placement, PlacementContext};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// `GenerationStep.Decoration` in ordinal order.
pub const STEPS: [&str; 11] = [
    "raw_generation",
    "lakes",
    "local_modifications",
    "underground_structures",
    "surface_structures",
    "strongholds",
    "underground_ores",
    "underground_decoration",
    "fluid_springs",
    "vegetal_decoration",
    "top_layer_modification",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StructureId(pub u16);

/// `TerrainAdjustment`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainAdjustment {
    None,
    Bury,
    BeardThin,
    BeardBox,
    Encapsulate,
}

pub type PieceList = Vec<Box<dyn Piece>>;

/// `Structure.GenerationStub`: a start position and its pieces, built now
/// or after the biome check.
pub struct Stub<'s> {
    pub position: BlockPos,
    pieces: StubPieces<'s>,
}

enum StubPieces<'s> {
    Built(PieceList),
    Deferred(Box<dyn FnOnce(&mut GenerationContext) -> PieceList + 's>),
}

impl<'s> Stub<'s> {
    pub fn built(position: BlockPos, pieces: PieceList) -> Self {
        Self { position, pieces: StubPieces::Built(pieces) }
    }

    pub fn deferred(position: BlockPos, generate: impl FnOnce(&mut GenerationContext) -> PieceList + 's) -> Self {
        Self { position, pieces: StubPieces::Deferred(Box::new(generate)) }
    }

    fn into_pieces(self, ctx: &mut GenerationContext) -> PieceList {
        match self.pieces {
            StubPieces::Built(p) => p,
            StubPieces::Deferred(f) => f(ctx),
        }
    }
}

/// One structure type's generation (`Structure.findGenerationPoint`).
pub trait StructureKind: Send + Sync + std::fmt::Debug {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>>;

    /// `Structure.afterPlace`.
    fn after_place(&self, _ctx: &mut Ctx, _random: &mut WorldgenRandom, _chunk_bb: &BoundingBox, _chunk: ChunkPos, _pieces: &[Box<dyn Piece>]) {}
}

/// A structure type this engine does not generate yet.
#[derive(Debug)]
pub struct Unsupported(pub String);

impl StructureKind for Unsupported {
    fn find_generation_point<'s>(&'s self, _ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        None
    }
}

/// One `worldgen/structure` entry.
#[derive(Debug)]
pub struct Structure {
    pub name: String,
    /// Per biome ID: whether the structure may start there.
    pub biomes: Vec<bool>,
    pub step: usize,
    pub adaptation: TerrainAdjustment,
    pub kind: Box<dyn StructureKind>,
}

/// One `worldgen/structure_set` entry (its placement is kept alongside).
#[derive(Debug)]
pub struct StructureSet {
    pub name: String,
    pub structures: Vec<(StructureId, i32)>,
}

/// `Structure.GenerationContext`.
pub struct GenerationContext<'a> {
    pub terrain: &'a TerrainGenerator,
    pub lib: &'a Library,
    pub pools: &'a jigsaw::Pools,
    /// `WorldgenRandom(LegacyRandomSource)` with the large feature seed.
    pub random: LegacyRandom,
    pub seed: i64,
    pub chunk: ChunkPos,
    valid_biomes: &'a [bool],
}

impl GenerationContext<'_> {
    /// The chunk's `getMinY` (the dimension type's bounds).
    pub fn min_y(&self) -> i32 {
        self.terrain.chunk_min_y
    }

    pub fn max_y(&self) -> i32 {
        self.terrain.chunk_min_y + self.terrain.chunk_height - 1
    }

    pub fn height(&self) -> i32 {
        self.terrain.chunk_height
    }

    fn valid(&self, qx: i32, qy: i32, qz: i32) -> bool {
        let biome = self.terrain.biome_at_quart(qx, qy, qz);
        self.valid_biomes.get(usize::from(biome.0)).copied().unwrap_or(false)
    }

    /// `GenerationContext.isValidBiome` at a stub position.
    pub fn is_valid_biome_at(&self, pos: BlockPos) -> bool {
        self.valid(pos.x >> 2, pos.y >> 2, pos.z >> 2)
    }

    /// `couldStructureExistInColumn`.
    pub fn could_structure_exist_in_column(&self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool {
        let (qx, qz) = (x >> 2, z >> 2);
        (min_y >> 2..=max_y >> 2).any(|qy| self.valid(qx, qy, qz))
    }

    /// `couldValidBiomeExistInTerrainColumn`.
    pub fn could_valid_biome_exist_in_terrain_column(&self, x: i32, z: i32) -> bool {
        self.could_structure_exist_in_column(x, z, self.min_y() - 1, self.max_y())
    }

    /// `couldValidBiomeExistOnTopOfChunkCenter`.
    pub fn could_valid_biome_exist_on_top_of_chunk_center(&self) -> bool {
        self.could_valid_biome_exist_in_terrain_column(self.chunk.min_block_x() + 8, self.chunk.min_block_z() + 8)
    }

    /// `ChunkGenerator.getFirstFreeHeight` (the raw noise base height).
    pub fn first_free_height(&self, x: i32, z: i32, kind: HeightmapKind) -> i32 {
        self.terrain.base_height(x, z, kind)
    }

    pub fn first_occupied_height(&self, x: i32, z: i32, kind: HeightmapKind) -> i32 {
        self.terrain.base_height(x, z, kind) - 1
    }

    /// `ChunkGenerator.getBaseColumn`: the minimum Y and the column's states.
    pub fn base_column(&self, x: i32, z: i32) -> (i32, Vec<BlockStateId>) {
        self.terrain.base_column(x, z)
    }

    /// `Structure.getCornerHeights` on `WORLD_SURFACE_WG`.
    pub fn corner_heights(&self, min_x: i32, size_x: i32, min_z: i32, size_z: i32) -> [i32; 4] {
        let h = |x, z| self.first_occupied_height(x, z, HeightmapKind::WorldSurfaceWg);
        [h(min_x, min_z), h(min_x, min_z + size_z), h(min_x + size_x, min_z), h(min_x + size_x, min_z + size_z)]
    }

    /// `Structure.getMeanFirstOccupiedHeight`.
    pub fn mean_first_occupied_height(&self, min_x: i32, size_x: i32, min_z: i32, size_z: i32) -> i32 {
        self.corner_heights(min_x, size_x, min_z, size_z).iter().sum::<i32>() / 4
    }

    /// `Structure.getLowestY(context, minX, minZ, sizeX, sizeZ)`.
    pub fn lowest_y_at(&self, min_x: i32, min_z: i32, size_x: i32, size_z: i32) -> i32 {
        self.corner_heights(min_x, size_x, min_z, size_z).into_iter().min().unwrap_or(0)
    }

    /// `Structure.getLowestY(context, sizeX, sizeZ)` from the chunk corner.
    pub fn lowest_y(&self, size_x: i32, size_z: i32) -> i32 {
        self.lowest_y_at(self.chunk.min_block_x(), self.chunk.min_block_z(), size_x, size_z)
    }

    /// `Structure.onTopOfChunkCenter`.
    pub fn on_top_of_chunk_center<'s>(&self, kind: HeightmapKind, generate: impl FnOnce(&mut GenerationContext) -> PieceList + 's) -> Option<Stub<'s>> {
        if !self.could_valid_biome_exist_on_top_of_chunk_center() {
            return None;
        }
        Some(self.on_top_of_chunk_center_without_biome_check(kind, generate))
    }

    pub fn on_top_of_chunk_center_without_biome_check<'s>(&self, kind: HeightmapKind, generate: impl FnOnce(&mut GenerationContext) -> PieceList + 's) -> Stub<'s> {
        let (x, z) = (self.chunk.min_block_x() + 8, self.chunk.min_block_z() + 8);
        let y = self.first_occupied_height(x, z, kind);
        Stub::deferred(BlockPos::new(x, y, z), generate)
    }
}

/// A generated structure start.
#[derive(Debug)]
pub struct StructureStart {
    pub structure: StructureId,
    pub chunk: ChunkPos,
    pub pieces: Mutex<PieceList>,
    /// `getBoundingBox`, fixed when the start is made.
    bbox: BoundingBox,
    piece_count: usize,
}

impl StructureStart {
    pub fn bounding_box(&self) -> BoundingBox {
        self.bbox
    }

    pub fn piece_count(&self) -> usize {
        self.piece_count
    }

    /// `StructureStart.placeInChunk`.
    pub fn place_in_chunk(&self, structures: &Structures, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, chunk: ChunkPos) {
        let mut pieces = self.pieces.lock().expect("structure pieces");
        let Some(first) = pieces.first() else { return };
        let center_bb = first.base().bbox;
        let center = center_bb.center();
        let reference = BlockPos::new(center.x, center_bb.min_y, center.z);
        for piece in pieces.iter_mut() {
            if piece.base().bbox.intersects(chunk_bb) {
                piece.post_process(ctx, random, chunk_bb, chunk, reference);
            }
        }
        structures.defs[usize::from(self.structure.0)].kind.after_place(ctx, random, chunk_bb, chunk, &pieces);
    }
}

type ChunkStarts = Arc<Vec<Arc<StructureStart>>>;

/// Every structure and structure set of one dimension, and the starts
/// generated so far.
pub struct Structures {
    pub defs: Vec<Structure>,
    names: HashMap<String, StructureId>,
    sets: Vec<StructureSet>,
    /// Each set's placement, parallel to `sets`.
    placements: Vec<Placement>,
    /// Sets whose structures may appear in this biome source, in registry order.
    possible_sets: Vec<usize>,
    /// Per decoration step: every registered structure of that step.
    by_step: Vec<Vec<StructureId>>,
    pools: jigsaw::Pools,
    seed: i64,
    /// `WorldOptions.generateStructures`.
    pub enabled: bool,
    starts: RwLock<HashMap<ChunkPos, Arc<OnceLock<ChunkStarts>>>>,
}

/// A `HolderSet<Biome>`: `#tag`, one biome, or a list.
fn parse_biomes(registries: &Registries, json: &Value) -> Result<Vec<bool>, String> {
    let mut out = vec![false; registries.biomes.len()];
    let mut add = |name: &str| -> Result<(), String> {
        if let Some(tag) = name.strip_prefix('#') {
            let tag = registries.biome_tags.require(tag)?;
            for (i, slot) in out.iter_mut().enumerate() {
                if registries.biome_tags.contains(tag, i) {
                    *slot = true;
                }
            }
        } else if let Some(id) = registries.biomes.id(name) {
            out[usize::from(id.0)] = true;
        }
        Ok(())
    };
    match json {
        Value::String(s) => add(s)?,
        Value::Array(list) => {
            for v in list {
                add(v.as_str().ok_or("biome list entry is not a string")?)?;
            }
        }
        other => return Err(format!("invalid biome set {other}")),
    }
    Ok(out)
}

impl Structures {
    /// Loads every structure, set and template pool; `possible_biomes` is the
    /// dimension's biome source.
    pub fn load(lib: &mut Library, terrain: &TerrainGenerator, enabled: bool) -> Result<Self, String> {
        let registries = lib.registries.clone();
        let pools = jigsaw::Pools::load(lib)?;
        let mut defs = Vec::new();
        let mut names = HashMap::new();
        for id in registries.datapack.list("worldgen/structure")? {
            let json = registries.datapack.read_json("worldgen/structure", &id)?;
            let step_name = json.get("step").and_then(Value::as_str).unwrap_or("surface_structures");
            let step = STEPS.iter().position(|s| *s == step_name).ok_or_else(|| format!("structure {id}: unknown step {step_name}"))?;
            let adaptation = match json.get("terrain_adaptation").and_then(Value::as_str).unwrap_or("none") {
                "none" => TerrainAdjustment::None,
                "bury" => TerrainAdjustment::Bury,
                "beard_thin" => TerrainAdjustment::BeardThin,
                "beard_box" => TerrainAdjustment::BeardBox,
                "encapsulate" => TerrainAdjustment::Encapsulate,
                other => return Err(format!("structure {id}: unknown terrain adaptation {other}")),
            };
            let kind = kinds::parse(lib, &json).map_err(|e| format!("structure {id}: {e}"))?;
            names.insert(id.to_string(), StructureId(defs.len() as u16));
            defs.push(Structure { name: id.to_string(), biomes: parse_biomes(&registries, &json["biomes"])?, step, adaptation, kind });
        }
        let set_ids = registries.datapack.list("worldgen/structure_set")?;
        let set_names: Vec<String> = set_ids.iter().map(ToString::to_string).collect();
        let mut sets = Vec::new();
        let mut placements = Vec::new();
        for id in &set_ids {
            let json = registries.datapack.read_json("worldgen/structure_set", id)?;
            let mut structures = Vec::new();
            for entry in json["structures"].as_array().ok_or_else(|| format!("structure set {id} lacks structures"))? {
                let name = entry["structure"].as_str().ok_or("structure set entry lacks structure")?;
                let key = Identifier::parse(name)?.to_string();
                let sid = *names.get(&key).ok_or_else(|| format!("structure set {id}: unknown structure {name}"))?;
                structures.push((sid, entry.get("weight").and_then(Value::as_i64).unwrap_or(1) as i32));
            }
            let placement = Placement::parse(
                &json["placement"],
                registries.biomes.len(),
                |v| parse_biomes(&registries, v),
                |name| {
                    let key = Identifier::parse(name).ok()?.to_string();
                    set_names.iter().position(|n| *n == key)
                },
                terrain.seed,
            )
            .map_err(|e| format!("structure set {id}: {e}"))?;
            sets.push(StructureSet { name: id.to_string(), structures });
            placements.push(placement);
        }
        let possible = terrain.possible_biomes();
        let possible_sets = (0..sets.len())
            .filter(|&i| sets[i].structures.iter().any(|(sid, _)| possible.iter().any(|b| defs[usize::from(sid.0)].biomes[usize::from(b.0)])))
            .collect();
        let mut by_step = vec![Vec::new(); STEPS.len()];
        for (i, def) in defs.iter().enumerate() {
            by_step[def.step].push(StructureId(i as u16));
        }
        Ok(Self { defs, names, sets, placements, possible_sets, by_step, pools, seed: terrain.seed, enabled, starts: RwLock::new(HashMap::new()) })
    }

    pub fn id(&self, name: &str) -> Option<StructureId> {
        self.names.get(name).copied()
    }

    pub fn sets(&self) -> &[StructureSet] {
        &self.sets
    }

    pub fn placement(&self, set: usize) -> &Placement {
        &self.placements[set]
    }

    fn placement_context<'a>(&'a self, terrain: &'a TerrainGenerator) -> PlacementContext<'a> {
        PlacementContext { seed: self.seed, terrain, all: &self.placements }
    }

    /// `StructurePlacement.isStructureChunk` for one set.
    pub fn is_structure_chunk(&self, terrain: &TerrainGenerator, set: usize, chunk: ChunkPos) -> bool {
        self.placements[set].is_structure_chunk(&self.placement_context(terrain), chunk.x, chunk.z)
    }

    /// Forgets every start (and the piece state placement changed); for
    /// tools that regenerate the same chunks independently.
    pub fn reset_starts(&self) {
        self.starts.write().expect("structure starts").clear();
    }

    /// The starts made in one chunk (`STRUCTURE_STARTS`), computed once.
    pub fn starts_in(&self, lib: &Library, terrain: &TerrainGenerator, chunk: ChunkPos) -> ChunkStarts {
        if !self.enabled {
            return Arc::new(Vec::new());
        }
        let existing = self.starts.read().expect("structure starts").get(&chunk).cloned();
        let cell = match existing {
            Some(cell) => cell,
            None => self.starts.write().expect("structure starts").entry(chunk).or_default().clone(),
        };
        cell.get_or_init(|| Arc::new(self.create_starts(lib, terrain, chunk))).clone()
    }

    /// `ChunkGenerator.createStructures`.
    fn create_starts(&self, lib: &Library, terrain: &TerrainGenerator, chunk: ChunkPos) -> Vec<Arc<StructureStart>> {
        let mut out: Vec<Arc<StructureStart>> = Vec::new();
        let ctx = self.placement_context(terrain);
        for &set_index in &self.possible_sets {
            let set = &self.sets[set_index];
            if set.structures.iter().any(|(sid, _)| out.iter().any(|s| s.structure == *sid)) {
                continue;
            }
            if !self.placements[set_index].is_structure_chunk(&ctx, chunk.x, chunk.z) {
                continue;
            }
            if set.structures.len() == 1 {
                if let Some(start) = self.try_generate(lib, terrain, set.structures[0].0, chunk) {
                    out.push(start);
                }
                continue;
            }
            let mut options = set.structures.clone();
            let mut random = LegacyRandom::new(0);
            placement::large_feature_seed(&mut random, self.seed, chunk.x, chunk.z);
            let mut total: i32 = options.iter().map(|(_, w)| w).sum();
            while !options.is_empty() {
                let mut choice = random.next_i32_bound(total);
                let mut index = 0;
                for (_, weight) in &options {
                    choice -= weight;
                    if choice < 0 {
                        break;
                    }
                    index += 1;
                }
                let (sid, weight) = options[index];
                if let Some(start) = self.try_generate(lib, terrain, sid, chunk) {
                    out.push(start);
                    break;
                }
                options.remove(index);
                total -= weight;
            }
        }
        out
    }

    /// `ChunkGenerator.tryGenerateStructure` and `Structure.generate`.
    fn try_generate(&self, lib: &Library, terrain: &TerrainGenerator, id: StructureId, chunk: ChunkPos) -> Option<Arc<StructureStart>> {
        let def = &self.defs[usize::from(id.0)];
        let mut random = LegacyRandom::new(0);
        placement::large_feature_seed(&mut random, self.seed, chunk.x, chunk.z);
        let mut ctx = GenerationContext { terrain, lib, pools: &self.pools, random, seed: self.seed, chunk, valid_biomes: &def.biomes };
        let stub = def.kind.find_generation_point(&mut ctx)?;
        if !ctx.is_valid_biome_at(stub.position) {
            return None;
        }
        let pieces = stub.into_pieces(&mut ctx);
        if pieces.is_empty() {
            return None;
        }
        let bbox = BoundingBox::encapsulating_all(pieces.iter().map(|p| &p.base().bbox)).expect("pieces");
        let bbox = if def.adaptation != TerrainAdjustment::None { bbox.inflated(12, 12, 12) } else { bbox };
        Some(Arc::new(StructureStart { structure: id, chunk, piece_count: pieces.len(), pieces: Mutex::new(pieces), bbox }))
    }

    /// `ChunkGenerator.createReferences`, then `getReferencesForStructure`
    /// iteration: per structure (in registry order), the starts whose
    /// bounds reach `chunk`, in `LongOpenHashSet` order of their chunks.
    pub fn references(&self, lib: &Library, terrain: &TerrainGenerator, chunk: ChunkPos) -> References {
        let (min_x, min_z) = (chunk.min_block_x(), chunk.min_block_z());
        let mut by_structure: Vec<(StructureId, LongSet, Vec<Arc<StructureStart>>)> = Vec::new();
        for sx in chunk.x - 8..=chunk.x + 8 {
            for sz in chunk.z - 8..=chunk.z + 8 {
                for start in self.starts_in(lib, terrain, ChunkPos::new(sx, sz)).iter() {
                    if !start.bbox.intersects_xz(min_x, min_z, min_x + 15, min_z + 15) {
                        continue;
                    }
                    let index = match by_structure.iter().position(|(s, _, _)| *s == start.structure) {
                        Some(i) => i,
                        None => {
                            by_structure.push((start.structure, LongSet::default(), Vec::new()));
                            by_structure.len() - 1
                        }
                    };
                    let entry = &mut by_structure[index];
                    entry.1.insert(start.chunk.pack());
                    entry.2.push(start.clone());
                }
            }
        }
        by_structure.sort_by_key(|(s, _, _)| *s);
        by_structure
            .into_iter()
            .map(|(sid, set, starts)| {
                let ordered = set.iter().filter_map(|key| starts.iter().find(|s| s.chunk.pack() == key).cloned()).collect();
                (sid, ordered)
            })
            .collect()
    }

    /// The structure part of `applyBiomeDecoration` for one step: every
    /// registered structure of the step in registry order, each with its
    /// feature seed, placing the starts that reference the chunk.
    pub fn place_step(&self, ctx: &mut Ctx, references: &References, random: &mut WorldgenRandom, decoration_seed: i64, step: usize) {
        if !self.enabled {
            return;
        }
        let Some(structures) = self.by_step.get(step) else { return };
        let center = ctx.region.center();
        let (x, z) = (center.min_block_x(), center.min_block_z());
        let chunk_bb = BoundingBox::new(x, ctx.min_y() + 1, z, x + 15, ctx.max_y(), z + 15);
        for (index, sid) in structures.iter().enumerate() {
            random.feature_seed(decoration_seed, index as i32, step as i32);
            if let Some((_, starts)) = references.iter().find(|(s, _)| s == sid) {
                for start in starts {
                    start.place_in_chunk(self, ctx, random, &chunk_bb, center);
                }
            }
        }
    }
}

/// Per structure, the starts referencing one chunk.
pub type References = Vec<(StructureId, Vec<Arc<StructureStart>>)>;

/// fastutil `LongOpenHashSet` with its default capacity: insertion into an
/// open-addressed table, iteration from the last slot down.
#[derive(Debug)]
pub struct LongSet {
    keys: Vec<i64>,
    has_zero: bool,
    size: usize,
}

impl Default for LongSet {
    fn default() -> Self {
        Self { keys: vec![0; 32], has_zero: false, size: 0 }
    }
}

impl LongSet {
    /// `HashCommon.mix`.
    fn mix(x: i64) -> i64 {
        let h = x.wrapping_mul(0x9E37_79B9_7F4A_7C15_u64 as i64);
        let h = h ^ ((h as u64) >> 32) as i64;
        h ^ ((h as u64) >> 16) as i64
    }

    pub fn insert(&mut self, key: i64) -> bool {
        if key == 0 {
            if self.has_zero {
                return false;
            }
            self.has_zero = true;
        } else {
            let mask = self.keys.len() - 1;
            let mut pos = (Self::mix(key) as usize) & mask;
            while self.keys[pos] != 0 {
                if self.keys[pos] == key {
                    return false;
                }
                pos = (pos + 1) & mask;
            }
            self.keys[pos] = key;
        }
        // `if (size++ >= maxFill) rehash(arraySize(size + 1, f))`.
        let max_fill = (self.keys.len() * 3).div_ceil(4).min(self.keys.len() - 1);
        let old = self.size;
        self.size += 1;
        if old >= max_fill {
            let wanted = ((self.size + 1) as f64 / 0.75).ceil() as usize;
            self.rehash(wanted.next_power_of_two().max(2));
        }
        true
    }

    fn rehash(&mut self, capacity: usize) {
        let old = std::mem::replace(&mut self.keys, vec![0; capacity]);
        let mask = capacity - 1;
        // fastutil walks the old table from the end when rehashing.
        for &key in old.iter().rev() {
            if key != 0 {
                let mut pos = (Self::mix(key) as usize) & mask;
                while self.keys[pos] != 0 {
                    pos = (pos + 1) & mask;
                }
                self.keys[pos] = key;
            }
        }
    }

    /// Iteration order: the zero key first, then slots from the end.
    pub fn iter(&self) -> impl Iterator<Item = i64> + '_ {
        self.has_zero.then_some(0).into_iter().chain(self.keys.iter().rev().copied().filter(|&k| k != 0))
    }
}
