//! Streamed, generated terrain: the client half of the integrated server.
//!
//! `minecraftoss_world::ChunkMap` decides which chunks exist and sends them
//! nearest-first. This module applies those packets to the client scene,
//! lights each chunk once its neighbors are present, and builds 16x16x16
//! render sections on worker threads in the order vanilla 26.3 would (see
//! `sections.rs`): the occlusion graph grows from the camera as sections
//! compile, only frustum-visible dirty sections are built, nearest first,
//! and each new section fades in from fog color.
//!
//! Meshing reads a dense copy of a section and its one-block border and
//! resolves models once per block state, then emits faces through the same
//! `mesh::append_block` as the authored scene, so both look identical.

use crate::frame_spans::span;
use crate::lighting::SkyLight;
use crate::mesh::{self, Atlas, BiomeTint, ChunkMesh};
use crate::model::{resolve_block_variants, ResolvedModel};
use crate::pack::{PackStack, ResourceId};
use crate::scene::{BiomeSample, Block, BlockPos, ChunkPos, HandcraftedScene, Scene};
use crate::sections::{CompileQueue, CullCamera, SectionPos, Sections, VisGraph, VisibilitySet};
use anyhow::{anyhow, Result};
use glam::DVec3;
use minecraftoss_core::block::flags;
use minecraftoss_core::{BiomeId, BlockStateId, Chunk, Registries};
use minecraftoss_generator::terrain::TerrainGenerator;
use minecraftoss_generator::zoom;
use minecraftoss_world::chunk_map::WorldGen;
use minecraftoss_world::storage::ChunkStorage;
use minecraftoss_world::{spawn, ChunkEvent, ChunkMap};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

/// Presentation data for every block state and biome of a world.
pub struct BlockStates {
    registries: Arc<Registries>,
    blocks: Vec<Option<Block>>,
    /// Per-state light properties for the vanilla light solver.
    light_table: minecraftoss_core::light::LightTable,
    /// `DimensionType.hasSkyLight`.
    sky_light: bool,
    /// `BlockState.isSolidRender`, which closes a cell in `VisGraph`.
    solid_render: Vec<bool>,
    /// Per state, what section building asks of its block's name and
    /// properties: whether it darkens a corner's ambient occlusion (opaque
    /// or leaves), the block whose same neighbours hide its faces (water
    /// and glass; `u32::MAX` for none), fluid, chest, waterlogged, tint.
    ao_occluder: Vec<bool>,
    shared_face: Vec<u32>,
    fluid: Vec<bool>,
    chest: Vec<bool>,
    waterlogged: Vec<bool>,
    tint_kind: Vec<mesh::TintKind>,
    /// Per state, `FluidCell::from_block` and `fluid::full_collision`.
    fluid_cell: Vec<Option<crate::fluid::FluidCell>>,
    full_collision: Vec<bool>,
    biomes: Vec<BiomeSample>,
    plains: BiomeSample,
    zoom_seed: i64,
    min_y: i32,
    height: i32,
}

impl BlockStates {
    pub fn new(registries: Arc<Registries>, seed: i64, min_y: i32, height: i32) -> Result<Self> {
        let count = registries.blocks.state_count();
        let mut blocks = Vec::with_capacity(count);
        let mut solid_render = Vec::with_capacity(count);
        let (mut ao_occluder, mut shared_face, mut fluid, mut chest, mut waterlogged, mut tint_kind) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut fluid_cell, mut full_collision) = (Vec::new(), Vec::new());
        let mut shared_groups: HashMap<String, u32> = HashMap::new();
        for index in 0..count {
            let state = BlockStateId(index as u16);
            let block = if registries.blocks.is_air(state) {
                None
            } else {
                Some(block_from_state(&registries.blocks.state_to_string(state))?)
            };
            solid_render.push(registries.blocks.is(state, flags::SOLID_RENDER));
            let path = block.as_ref().map_or("", |b| b.id.path.as_str());
            ao_occluder.push(block.as_ref().is_some_and(|b| shade_darkens(&registries.blocks, state, b)));
            let shares = path == "water" || path == "glass" || path == "tinted_glass" || path.ends_with("_stained_glass");
            shared_face.push(match &block {
                Some(b) if shares => {
                    let next = shared_groups.len() as u32;
                    *shared_groups.entry(b.id.key()).or_insert(next)
                }
                _ => u32::MAX,
            });
            fluid.push(matches!(path, "water" | "lava"));
            chest.push(path == "chest");
            waterlogged.push(block.as_ref().is_some_and(|b| b.properties.get("waterlogged").is_some_and(|value| value == "true")));
            tint_kind.push(block.as_ref().map_or(mesh::TintKind::Other, mesh::TintKind::of));
            fluid_cell.push(block.as_ref().and_then(crate::fluid::FluidCell::from_block));
            full_collision.push(crate::fluid::full_collision(block.as_ref()));
            blocks.push(block);
        }
        let biomes = registries
            .biomes
            .iter()
            .map(|(_, info)| biome_sample(info))
            .collect::<Vec<_>>();
        let light_table = minecraftoss_core::light::LightTable::new(&registries.blocks);
        let plains = registries
            .biomes
            .id("minecraft:plains")
            .map_or(BiomeSample::THE_VOID, |id| biomes[usize::from(id.0)]);
        Ok(Self {
            registries,
            blocks,
            light_table,
            sky_light: true,
            solid_render,
            ao_occluder,
            shared_face,
            fluid,
            chest,
            waterlogged,
            tint_kind,
            fluid_cell,
            full_collision,
            biomes,
            plains,
            zoom_seed: zoom::zoom_seed(seed),
            min_y,
            height,
        })
    }

    pub fn registries(&self) -> &Arc<Registries> {
        &self.registries
    }

    /// Whether a block darkens the corners of faces beside it
    /// (`shade_darkens`).
    pub fn shade_darkens(&self, block: &Block) -> bool {
        self.state_of(block).map_or_else(|| block.is_opaque() || block.id.path.ends_with("_leaves"), |state| self.ao_occluder[usize::from(state.0)])
    }

    /// The scene block of a state; air states have none.
    pub fn block(&self, state: BlockStateId) -> Option<&Block> {
        self.blocks[usize::from(state.0)].as_ref()
    }

    /// The state an edited block stands for. Unlisted properties keep the
    /// block's defaults; an unknown block or value reads as its default state.
    pub fn state_of(&self, block: &Block) -> Option<BlockStateId> {
        let registry = &self.registries.blocks;
        let name = block.id.key();
        let id = registry.block_by_name(&name)?;
        let mut state = registry.block(id).default_state();
        for (key, value) in &block.properties {
            if let Some(next) = registry.with_property(state, key, value) {
                state = next;
            }
        }
        Some(state)
    }

    /// `BlockState.getSoundType()` of a block's state, when the catalog
    /// records sound types.
    pub fn sound_type(&self, block: &Block) -> Option<&minecraftoss_core::block::SoundType> {
        let state = self.state_of(block)?;
        self.registries.blocks.sound_type(state)
    }

    /// The sound type of the block named `id` in its default state.
    pub fn sound_type_of(&self, id: &str) -> Option<&minecraftoss_core::block::SoundType> {
        let registry = &self.registries.blocks;
        let block = registry.block_by_name(id)?;
        registry.sound_type(registry.block(block).default_state())
    }

    pub fn vertical_range(&self) -> std::ops::Range<i32> {
        self.min_y..self.min_y + self.height
    }

    fn section_range(&self) -> (i32, i32) {
        (self.min_y >> 4, (self.min_y + self.height - 1) >> 4)
    }

    /// `BiomeManager.getBiome`: the zoomed noise biome at a block. Missing
    /// chunks read as plains, as on the vanilla client.
    pub fn biome_at<'c>(
        &self,
        (x, y, z): BlockPos,
        chunk: impl Fn(ChunkPos) -> Option<&'c Chunk>,
    ) -> BiomeSample {
        let [qx, qy, qz] = zoom::quart_for_block(self.zoom_seed, x, y, z);
        chunk((qx >> 2, qz >> 2)).map_or(self.plains, |chunk| {
            let id = chunk.biome((qx & 3) as usize, qy, (qz & 3) as usize);
            self.biome(id)
        })
    }

    pub fn biome(&self, id: BiomeId) -> BiomeSample {
        self.biomes[usize::from(id.0)]
    }

    /// `BiomeManager.getNoiseBiomeAtQuart` on the client: the chunk's noise
    /// biome, or plains where no chunk is loaded.
    pub fn noise_biome<'c>(&self, (qx, qy, qz): (i32, i32, i32), chunk: impl Fn(ChunkPos) -> Option<&'c Chunk>) -> BiomeId {
        chunk((qx >> 2, qz >> 2)).map_or_else(
            || self.registries.biomes.id("minecraft:plains").unwrap_or(BiomeId(0)),
            |chunk| chunk.biome((qx & 3) as usize, qy, (qz & 3) as usize),
        )
    }
}

fn block_from_state(text: &str) -> Result<Block> {
    let (name, properties) = match text.split_once('[') {
        Some((name, rest)) => (name, rest.trim_end_matches(']')),
        None => (text, ""),
    };
    let mut block = Block {
        id: ResourceId::parse(name)?,
        properties: BTreeMap::new(),
    };
    for pair in properties.split(',').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| anyhow!("bad block state {text}"))?;
        block.properties.insert(key.into(), value.into());
    }
    Ok(block)
}

/// `#rrggbb` strings or packed integers, as 26.3 biome codecs accept.
fn color(value: &serde_json::Value) -> Option<[u8; 3]> {
    let packed = match value {
        serde_json::Value::String(text) => {
            u32::from_str_radix(text.strip_prefix('#').unwrap_or(text), 16).ok()?
        }
        serde_json::Value::Number(number) => number.as_i64()? as u32,
        _ => return None,
    };
    Some([(packed >> 16) as u8, (packed >> 8) as u8, packed as u8])
}

/// The presentation sample of `minecraft:plains`.
pub fn plains_sample(registries: &Registries) -> BiomeSample {
    registries.biomes.id("minecraft:plains").map_or(BiomeSample::THE_VOID, |id| biome_sample(registries.biomes.get(id)))
}

fn biome_sample(info: &minecraftoss_core::biome::BiomeInfo) -> BiomeSample {
    let effect = |key: &str| info.effects.get(key).and_then(color);
    let attribute = |key: &str| info.attributes.get(key).and_then(color);
    BiomeSample {
        temperature: info.temperature,
        downfall: info.downfall,
        has_precipitation: info.has_precipitation,
        grass_color: effect("grass_color"),
        foliage_color: effect("foliage_color"),
        water_color: effect("water_color").unwrap_or(BiomeSample::THE_VOID.water_color),
        sky_color: attribute("minecraft:visual/sky_color"),
        fog_color: attribute("minecraft:visual/fog_color"),
    }
}

/// `BlockState.getShadeBrightness` is 0.2 rather than 1: the block darkens
/// the corners of faces beside it. A full-block collision shape does
/// (`BlockBehaviour`), except where a block says otherwise: glass, stained
/// and tinted glass and copper grates (`TransparentBlock`), barriers, light
/// blocks and structure voids never do; mud, soul sand and a full stack of
/// snow always do. Plants, with no collision, never darken the ground.
fn shade_darkens(registry: &minecraftoss_core::block::BlockRegistry, state: BlockStateId, block: &Block) -> bool {
    use minecraftoss_core::block::FaceShape;
    let path = block.id.path.as_str();
    match path {
        "glass" | "tinted_glass" | "barrier" | "light" | "structure_void" => return false,
        _ if path.ends_with("_stained_glass") || path.ends_with("copper_grate") => return false,
        "mud" | "soul_sand" => return true,
        "snow" => return block.properties.get("layers").is_some_and(|layers| layers == "8"),
        _ => {}
    }
    match registry.collision_shape(state) {
        Some(FaceShape::Full) => true,
        Some(FaceShape::Boxes(boxes)) => boxes.len() == 1 && boxes[0] == [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        Some(FaceShape::Empty) => false,
        // A catalog without shapes: the older name-based guess.
        None => block.is_opaque() || path.ends_with("_leaves"),
    }
}

/// A chunk and its eight neighbors, with their edits, as one job sees them.
pub struct Neighborhood {
    pub center: ChunkPos,
    chunks: [Arc<Chunk>; 9],
    placed: [Option<Arc<BTreeMap<BlockPos, Block>>>; 9],
    cleared: [Option<Arc<BTreeSet<BlockPos>>>; 9],
}

impl Neighborhood {
    /// `None` until all nine chunks are loaded.
    pub fn of(scene: &HandcraftedScene, center: ChunkPos) -> Option<Self> {
        let at = |i: usize| (center.0 + i as i32 % 3 - 1, center.1 + i as i32 / 3 - 1);
        let chunks = (0..9)
            .map(|i| scene.generated_chunk(at(i)).cloned())
            .collect::<Option<Vec<_>>>()?;
        let edits = |i: usize| scene.chunk_edits(at(i));
        Some(Self {
            center,
            chunks: chunks.try_into().ok()?,
            placed: std::array::from_fn(|i| edits(i).0.cloned()),
            cleared: std::array::from_fn(|i| edits(i).1.cloned()),
        })
    }

    fn chunk(&self, (cx, cz): ChunkPos) -> Option<&Chunk> {
        let (dx, dz) = (cx - self.center.0, cz - self.center.1);
        ((-1..=1).contains(&dx) && (-1..=1).contains(&dz))
            .then(|| &*self.chunks[((dz + 1) * 3 + dx + 1) as usize])
    }
}

/// Dense block states of a box inside a neighborhood, readable as a `Scene`.
struct View<'a> {
    states: &'a BlockStates,
    hood: &'a Neighborhood,
    min: BlockPos,
    size: (usize, usize, usize),
    cells: Vec<BlockStateId>,
}

impl<'a> View<'a> {
    fn new(states: &'a BlockStates, hood: &'a Neighborhood, min: BlockPos, size: (usize, usize, usize)) -> Self {
        let (sx, sy, sz) = size;
        let mut cells = vec![BlockStateId::AIR; sx * sy * sz];
        let world = states.vertical_range();
        let (y0, y1) = (min.1.max(world.start), (min.1 + sy as i32).min(world.end));
        for dx in 0..sx {
            for dz in 0..sz {
                let (x, z) = (min.0 + dx as i32, min.2 + dz as i32);
                let Some(chunk) = hood.chunk((x >> 4, z >> 4)) else {
                    continue;
                };
                let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
                let base = (dx * sz + dz) * sy;
                let mut y = y0;
                while y < y1 {
                    let section = &chunk.sections()[((y >> 4) - chunk.min_section_y()) as usize];
                    let end = ((y >> 4) + 1) * 16;
                    let end = end.min(y1);
                    match section.blocks.single() {
                        Some(state) => {
                            let from = base + (y - min.1) as usize;
                            cells[from..from + (end - y) as usize].fill(state);
                        }
                        None => {
                            for yy in y..end {
                                cells[base + (yy - min.1) as usize] =
                                    section.block(lx, (yy & 15) as usize, lz);
                            }
                        }
                    }
                    y = end;
                }
            }
        }
        let mut view = Self {
            states,
            hood,
            min,
            size,
            cells,
        };
        for i in 0..9 {
            for &pos in hood.cleared[i].iter().flat_map(|set| set.iter()) {
                if let Some(index) = view.index(pos) {
                    view.cells[index] = BlockStateId::AIR;
                }
            }
            for (&pos, block) in hood.placed[i].iter().flat_map(|map| map.iter()) {
                if let (Some(index), Some(state)) = (view.index(pos), states.state_of(block)) {
                    view.cells[index] = state;
                }
            }
        }
        view
    }

    fn index(&self, (x, y, z): BlockPos) -> Option<usize> {
        let (dx, dy, dz) = (x - self.min.0, y - self.min.1, z - self.min.2);
        let (sx, sy, sz) = self.size;
        if dx < 0 || dy < 0 || dz < 0 || dx as usize >= sx || dy as usize >= sy || dz as usize >= sz {
            return None;
        }
        Some((dx as usize * sz + dz as usize) * sy + dy as usize)
    }

    fn state(&self, pos: BlockPos) -> BlockStateId {
        self.index(pos).map_or(BlockStateId::AIR, |i| self.cells[i])
    }
}

impl Scene for View<'_> {
    fn block(&self, pos: BlockPos) -> Option<&Block> {
        self.states.block(self.state(pos))
    }
    fn chunks(&self) -> Vec<ChunkPos> {
        vec![self.hood.center]
    }
    fn vertical_range(&self) -> std::ops::Range<i32> {
        self.states.vertical_range()
    }
    fn revision(&self) -> u64 {
        0
    }
    fn biome_at(&self, pos: BlockPos) -> BiomeSample {
        self.states.biome_at(pos, |chunk| self.hood.chunk(chunk))
    }
    fn fluid_at(&self, pos: BlockPos) -> Option<crate::fluid::FluidCell> {
        self.states.fluid_cell[usize::from(self.state(pos).0)]
    }
    fn full_collision_at(&self, pos: BlockPos) -> bool {
        self.states.full_collision[usize::from(self.state(pos).0)]
    }
    fn shade_darkens_at(&self, pos: BlockPos) -> bool {
        self.states.ao_occluder[usize::from(self.state(pos).0)]
    }
}

/// Lights a chunk from its neighborhood with the vanilla light solver
/// (`minecraftoss_core::light`), edits included. The result covers the
/// chunk and a one-block border, everything its sections' faces sample.
fn light_chunk(states: &BlockStates, hood: &Neighborhood) -> SkyLight {
    let (cx, cz) = hood.center;
    let height = states.height as usize;
    let view = View::new(states, hood, (cx * 16 - 16, states.min_y, cz * 16 - 16), (48, height, 48));
    // The solver's layout: y from one section below the build range.
    let region_height = height + 32;
    let mut cells = vec![BlockStateId::AIR; 48 * 48 * region_height];
    let blocks = &states.registries.blocks;
    let sections = height / 16;
    let mut non_empty = vec![false; 9 * sections];
    for x in 0..48 {
        for z in 0..48 {
            for y in 0..height {
                let state = view.cells[(x * 48 + z) * height + y];
                cells[((y + 16) * 48 + z) * 48 + x] = state;
                if !blocks.is(state, minecraftoss_core::block::flags::AIR) {
                    non_empty[((z / 16) * 3 + x / 16) * sections + y / 16] = true;
                }
            }
        }
    }
    let min_section = states.min_y >> 4;
    let region = minecraftoss_core::light::light_region(
        blocks,
        &states.light_table,
        min_section,
        sections,
        cells,
        |dx, dz, sy| {
            let s = sy - min_section;
            (-1..=1).contains(&dx) && (-1..=1).contains(&dz) && (0..sections as i32).contains(&s) && non_empty[((dz + 1) * 3 + dx + 1) as usize * sections + s as usize]
        },
        states.sky_light,
    );
    // Keep columns up to the highest cell below full sky light or with
    // block light; lookups above return those defaults.
    let mut top = 0;
    for x in 15..33 {
        for z in 15..33 {
            if let Some(y) = (0..region_height).rev().find(|&y| {
                let (block, sky) = region.get(x, y, z);
                sky < 15 || block > 0
            }) {
                top = top.max(y + 1);
            }
        }
    }
    let (mut sky, mut block) = (Vec::with_capacity(18 * 18 * top), Vec::with_capacity(18 * 18 * top));
    for x in 15..33 {
        for z in 15..33 {
            for y in 0..top {
                let (b, s) = region.get(x, y, z);
                block.push(b);
                sky.push(s);
            }
        }
    }
    SkyLight::from_levels((cx * 16 - 1, states.min_y - 16, cz * 16 - 1), (18, top, 18), sky, block)
}

/// A chunk's light as the server sent it, as [`light_chunk`] lays it out:
/// the column and a one-block border from the nine chunks' own light, as
/// vanilla's client takes light from the server. `None` when a chunk has
/// no light or the neighborhood has edits the server light misses.
fn server_light(states: &BlockStates, hood: &Neighborhood) -> Option<SkyLight> {
    if hood.placed.iter().any(Option::is_some) || hood.cleared.iter().any(Option::is_some) {
        return None;
    }
    let lights: Vec<&minecraftoss_core::light::ChunkLight> = hood.chunks.iter().map(|chunk| chunk.light.as_deref()).collect::<Option<_>>()?;
    let (cx, cz) = hood.center;
    let region_min_y = states.min_y - 16;
    let region_height = states.height as usize + 32;
    // Each border column's (sky, block) levels from the bottom of the region.
    let mut columns: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(18 * 18);
    let mut top = 0;
    for x in cx * 16 - 1..=cx * 16 + 16 {
        for z in cz * 16 - 1..=cz * 16 + 16 {
            let light = lights[(((z >> 4) - cz + 1) * 3 + (x >> 4) - cx + 1) as usize];
            let (mut sky, mut block) = (Vec::with_capacity(region_height), Vec::with_capacity(region_height));
            for y in 0..region_height {
                let world_y = region_min_y + y as i32;
                // Without skylight the solver's sky levels are all 0.
                let s = if states.sky_light { light.sky_at(x, world_y, z) } else { 0 };
                let b = light.block_at(x, world_y, z);
                sky.push(s as u8);
                block.push(b as u8);
                if s < 15 || b > 0 {
                    top = top.max(y + 1);
                }
            }
            columns.push((sky, block));
        }
    }
    let (mut sky, mut block) = (Vec::with_capacity(18 * 18 * top), Vec::with_capacity(18 * 18 * top));
    for (column_sky, column_block) in &columns {
        sky.extend_from_slice(&column_sky[..top]);
        block.extend_from_slice(&column_block[..top]);
    }
    Some(SkyLight::from_levels((cx * 16 - 1, region_min_y, cz * 16 - 1), (18, top, 18), sky, block))
}

enum Slot {
    Unresolved,
    Missing,
    Model {
        variants: Vec<(ResolvedModel, u32)>,
        occludes: bool,
        /// Each variant's faces baked; `None` if a face could not be.
        baked: Option<Vec<Vec<mesh::BakedQuad>>>,
    },
}

/// Resolved models per block state, for one pack generation.
#[derive(Default)]
struct ModelCache {
    slots: Vec<Slot>,
}

impl ModelCache {
    fn prepare(&mut self, states: &BlockStates, packs: &PackStack, atlas: &Atlas, view: &View) {
        if self.slots.len() != states.blocks.len() {
            self.slots = (0..states.blocks.len()).map(|_| Slot::Unresolved).collect();
        }
        for &state in &view.cells {
            let slot = &mut self.slots[usize::from(state.0)];
            if !matches!(slot, Slot::Unresolved) {
                continue;
            }
            let Some(block) = states.block(state) else {
                *slot = Slot::Missing;
                continue;
            };
            *slot = match resolve_block_variants(packs, block) {
                Ok(variants) => Slot::Model {
                    occludes: block.is_opaque() && mesh::all_full_cubes(&variants),
                    baked: variants.iter().map(|(model, _)| mesh::bake_quads(block, model, atlas)).collect::<Result<Vec<_>>>().ok(),
                    variants,
                },
                Err(e) => {
                    eprintln!("no model for {}: {e:#}", block.id.key());
                    Slot::Missing
                }
            };
        }
    }

    fn occludes(&self, state: BlockStateId) -> bool {
        matches!(self.slots[usize::from(state.0)], Slot::Model { occludes: true, .. })
    }
}

/// Profiling: summed time per section compile phase over the process.
pub mod compile_profile {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;
    pub const VIEW: usize = 0;
    pub const MODELS: usize = 1;
    pub const BLOCKS: usize = 2;
    pub const CONVERT: usize = 3;
    const NAMES: [&str; 4] = ["compile: view", "compile: models", "compile: blocks", "compile: gpu format"];
    static MICROS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
    static COUNTS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
    pub fn add(phase: usize, since: Instant) {
        MICROS[phase].fetch_add(since.elapsed().as_micros() as u64, Ordering::Relaxed);
        COUNTS[phase].fetch_add(1, Ordering::Relaxed);
    }
    pub fn report() -> String {
        let mut out = String::new();
        for (i, name) in NAMES.iter().enumerate() {
            let (micros, count) = (MICROS[i].load(Ordering::Relaxed), COUNTS[i].load(Ordering::Relaxed));
            out += &format!("    {name:32} {:8.2}s {count:7} x {:8.0} us\n", micros as f64 / 1e6, micros.checked_div(count).unwrap_or(0));
        }
        out
    }
}

/// `SectionCompiler.compile`: one section's faces and visibility set.
fn compile_section(
    states: &BlockStates,
    hood: &Neighborhood,
    (sx, sy, sz): SectionPos,
    light: &SkyLight,
    models: &mut ModelCache,
    packs: &PackStack,
    atlas: &Atlas,
    tint: &BiomeTint,
) -> Result<(ChunkMesh, VisibilitySet)> {
    let origin = (sx * 16, sy * 16, sz * 16);
    let started = Instant::now();
    let view = View::new(states, hood, (origin.0 - 1, origin.1 - 1, origin.2 - 1), (18, 18, 18));
    compile_profile::add(compile_profile::VIEW, started);
    let started = Instant::now();
    models.prepare(states, packs, atlas, &view);
    compile_profile::add(compile_profile::MODELS, started);
    let started = Instant::now();
    let models = &*models;
    let built = build_section_blocks(states, &view, origin, light, models, atlas, tint, true)?;
    if std::env::var_os("MINECRAFTOSS_MESH_CHECK").is_some() {
        // The baked faces must give exactly the unbaked build's mesh.
        mesh::FLUID_FULL_PATH.with(|full| full.set(true));
        let slow = build_section_blocks(states, &view, origin, light, models, atlas, tint, false);
        mesh::FLUID_FULL_PATH.with(|full| full.set(false));
        let slow = slow?;
        let bits = |m: &ChunkMesh| -> Vec<u32> {
            m.vertices.iter().flat_map(|v| v.position.iter().chain(&v.uv).chain(&v.color).chain([&v.sky_light, &v.block_light]).map(|f| f.to_bits()).collect::<Vec<_>>()).collect()
        };
        assert!(
            bits(&built.0) == bits(&slow.0) && built.0.indices == slow.0.indices && built.0.faces == slow.0.faces && built.1 == slow.1,
            "baked section {:?} differs",
            (sx, sy, sz)
        );
    }
    let (mesh, transparent, vis) = (built.0, built.1, built.2);
    compile_profile::add(compile_profile::BLOCKS, started);
    Ok((mesh::finish_mesh(mesh, transparent), vis.resolve()))
}

/// The block loop of a section build: its faces (opaque, then the
/// translucent indices apart) and its visibility graph. `baked` uses the
/// baked faces where a block has them.
#[allow(clippy::too_many_arguments)]
fn build_section_blocks(states: &BlockStates, view: &View, origin: BlockPos, light: &SkyLight, models: &ModelCache, atlas: &Atlas, tint: &BiomeTint, baked: bool) -> Result<(ChunkMesh, Vec<u32>, VisGraph)> {
    let mut mesh = ChunkMesh::default();
    let mut transparent = Vec::new();
    let mut vis = VisGraph::default();
    // BlockPos.betweenClosed order: X fastest, then Y, then Z.
    for z in 0..16 {
        for y in 0..16 {
            for x in 0..16 {
                let pos = (origin.0 + x, origin.1 + y, origin.2 + z);
                let state = view.state(pos);
                let Some(block) = states.block(state) else {
                    continue;
                };
                if states.solid_render[usize::from(state.0)] {
                    vis.set_opaque(x as usize, y as usize, z as usize);
                }
                let index = usize::from(state.0);
                let slot = &models.slots[index];
                let fluid = states.fluid[index];
                if !fluid && !matches!(slot, Slot::Model { .. }) {
                    continue;
                }
                if let Slot::Model { variants, baked: Some(quads), .. } = slot {
                    if baked && !fluid && !states.chest[index] {
                        let shared = states.shared_face[index];
                        mesh::append_baked(
                            view,
                            pos,
                            &quads[mesh::variant_for(variants, pos)],
                            states.tint_kind[index],
                            states.waterlogged[index],
                            |neighbor| {
                                let other = view.state(neighbor);
                                let other_index = usize::from(other.0);
                                states.blocks[other_index].is_some() && (models.occludes(other) || (shared != u32::MAX && states.shared_face[other_index] == shared))
                            },
                            |at| states.ao_occluder[usize::from(view.state(at).0)],
                            atlas,
                            tint,
                            light,
                            &mut mesh,
                            &mut transparent,
                        )?;
                        continue;
                    }
                }
                mesh::append_block(
                    view,
                    pos,
                    block,
                    || match slot {
                        Slot::Model { variants, .. } => Ok(variants.as_slice()),
                        _ => Err(anyhow!("unresolved model")),
                    },
                    |neighbor| {
                        let other = view.state(neighbor);
                        states.block(other).is_some_and(|other_block| {
                            models.occludes(other) || mesh::hides_shared_face(block, other_block)
                        })
                    },
                    atlas,
                    tint,
                    light,
                    &mut mesh,
                    &mut transparent,
                )?;
            }
        }
    }
    Ok((mesh, transparent, vis))
}

/// What a job needs besides its own inputs.
#[derive(Clone)]
struct JobContext {
    states: Arc<BlockStates>,
    atlas: Arc<Atlas>,
    pack_sources: Vec<PathBuf>,
    pack_generation: u64,
}

struct LightJob {
    previous: Option<Arc<SkyLight>>,
    chunk: ChunkPos,
    epoch: u64,
    hood: Neighborhood,
}

struct CompileJob {
    serial: u64,
    hood: Neighborhood,
    light: Arc<SkyLight>,
    context: JobContext,
}

struct WorkQueue {
    light: Vec<(LightJob, Arc<BlockStates>)>,
    compile: CompileQueue<CompileJob>,
    eye: DVec3,
    /// Jobs a worker has taken but not finished.
    active: usize,
    stop: bool,
}

enum Job {
    Light(LightJob, Arc<BlockStates>),
    Compile(SectionPos, CompileJob),
}

enum Done {
    Light {
        changed_sections: Vec<i32>,
        chunk: ChunkPos,
        epoch: u64,
        light: Arc<SkyLight>,
        micros: u64,
    },
    Compile {
        section: SectionPos,
        serial: u64,
        pack_generation: u64,
        micros: u64,
        outcome: Result<(mesh::SectionMesh, VisibilitySet)>,
    },
}

type Work = Arc<(Mutex<WorkQueue>, Condvar)>;

/// Lighting gates compiling, so a worker takes the nearest light job first,
/// then the section `SectionTaskDynamicQueue` would pick.
fn next_job(queue: &mut WorkQueue) -> Option<Job> {
    let eye = queue.eye;
    let nearest = queue
        .light
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let d = |c: ChunkPos| {
                let (x, z) = (f64::from(c.0 * 16 + 8) - eye.x, f64::from(c.1 * 16 + 8) - eye.z);
                x * x + z * z
            };
            d(a.0.chunk).total_cmp(&d(b.0.chunk))
        })
        .map(|(i, _)| i);
    if let Some(i) = nearest {
        let (job, states) = queue.light.swap_remove(i);
        return Some(Job::Light(job, states));
    }
    queue.compile.poll(eye).map(|(pos, job)| Job::Compile(pos, job))
}

fn worker(work: &Work, done: &mpsc::Sender<Done>) {
    let mut packs: Option<(Vec<PathBuf>, u64, PackStack, BiomeTint)> = None;
    let mut models = ModelCache::default();
    loop {
        let job = {
            let (lock, ready) = &**work;
            let mut queue = lock.lock().expect("terrain work queue");
            loop {
                if queue.stop {
                    return;
                }
                if let Some(job) = next_job(&mut queue) {
                    queue.active += 1;
                    break job;
                }
                queue = ready.wait(queue).expect("terrain work queue");
            }
        };
        let started = Instant::now();
        let result = match job {
            Job::Light(job, states) => {
                let light=Arc::new(server_light(&states, &job.hood).unwrap_or_else(||light_chunk(&states,&job.hood)));
                let (min,max)=states.section_range();
                // Comparing changed-height columns can visit every voxel. Keep it off the render thread.
                // The epoch rejects this result if another edit invalidates its captured previous light.
                let changed_sections=job.previous.as_ref().map_or_else(Vec::new,|old|old.differing_sections(&light,job.chunk,min..=max));
                Done::Light {chunk:job.chunk,epoch:job.epoch,light,changed_sections,micros:started.elapsed().as_micros() as u64}
            },
            Job::Compile(section, job) => {
                let context = &job.context;
                if packs.as_ref().is_none_or(|(sources, generation, _, _)| {
                    sources != &context.pack_sources || *generation != context.pack_generation
                }) {
                    models = ModelCache::default();
                    packs = PackStack::open(context.pack_sources.clone())
                        .and_then(|stack| Ok((BiomeTint::from_pack(&stack)?, stack)))
                        .map(|(tint, stack)| (context.pack_sources.clone(), context.pack_generation, stack, tint))
                        .map_err(|e| eprintln!("section compiler cannot open packs: {e:#}"))
                        .ok();
                }
                let outcome = match &packs {
                    Some((_, _, stack, tint)) => compile_section(
                        &context.states,
                        &job.hood,
                        section,
                        &job.light,
                        &mut models,
                        stack,
                        &context.atlas,
                        tint,
                    ),
                    None => Err(anyhow!("resource packs unavailable")),
                }
                // The GPU format is made here, off the render thread.
                .map(|(mesh, visibility)| {
                    let started = Instant::now();
                    let mesh = mesh::SectionMesh::from(mesh);
                    compile_profile::add(compile_profile::CONVERT, started);
                    (mesh, visibility)
                });
                Done::Compile {
                    section,
                    serial: job.serial,
                    pack_generation: context.pack_generation,
                    micros: started.elapsed().as_micros() as u64,
                    outcome,
                }
            }
        };
        let sent = done.send(result);
        work.0.lock().expect("terrain work queue").active -= 1;
        if sent.is_err() {
            return;
        }
    }
}

/// `PlayerSpawnFinder` starts at an unseeded random candidate.
fn unseeded() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.subsec_nanos())
}

/// Renderer changes from one frame of terrain work.
#[derive(Default)]
pub struct FrameUpdate {
    /// New or rebuilt section meshes; an empty mesh removes the section.
    pub uploads: Vec<(SectionPos, mesh::SectionMesh)>,
    pub removed: Vec<SectionPos>,
    /// Chunk light columns to set or clear in the world light.
    pub lights: Vec<(ChunkPos, Option<Arc<SkyLight>>)>,
    /// Sections to draw, nearest first, with their fade-in visibility.
    pub visible: Vec<(SectionPos, f32)>,
}

#[derive(Default)]
struct Stats {
    lit: u64,
    light_micros: u64,
    compiled: u64,
    compile_micros: u64,
}

/// The integrated server's chunk map and the client's section rendering.
pub struct TerrainStream {
    pub states: Arc<BlockStates>,
    /// Vanilla's world spawn block (`MinecraftServer.setInitialSpawn`).
    pub world_spawn: (i32, i32, i32),
    /// Where the player first appears, within the respawn radius of it.
    pub player_spawn: (f64, f64, f64),
    server: ChunkMap,
    /// Dropped after `server`, whose drop saves into it.
    _session: Option<SessionDir>,
    sections: Sections,
    work: Work,
    done: mpsc::Receiver<Done>,
    workers: Vec<JoinHandle<()>>,
    lights: HashMap<ChunkPos, Arc<SkyLight>>,
    /// Bumped whenever a chunk must be (re)lit; stale light results drop.
    light_epochs: HashMap<ChunkPos, u64>,
    lights_pending: HashSet<ChunkPos>,
    /// Bumped whenever a section compile is scheduled; stale results drop.
    serials: HashMap<SectionPos, u64>,
    next_serial: u64,
    /// Edited sections and relit chunks the player is waiting on.
    player_sections: HashSet<SectionPos>,
    player_relights: HashSet<ChunkPos>,
    removed: Vec<SectionPos>,
    light_changes: Vec<(ChunkPos, Option<Arc<SkyLight>>)>,
    scheduled_last_frame: usize,
    clock: Instant,
    stats: Stats,
}

/// Which dimension a streamed world generates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dimension {
    #[default]
    Overworld,
    Nether,
    End,
}

impl Dimension {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.trim_start_matches("minecraft:") {
            "overworld" => Self::Overworld,
            "nether" | "the_nether" => Self::Nether,
            "end" | "the_end" => Self::End,
            _ => return None,
        })
    }

    pub fn dimension_type(self) -> &'static str {
        match self {
            Self::Overworld => "minecraft:overworld",
            Self::Nether => "minecraft:the_nether",
            Self::End => "minecraft:the_end",
        }
    }
}

/// A standing spot in the Nether near the origin: two air blocks over a
/// solid block, below the bedrock roof, searched outward from chunk 0,0.
fn nether_spawn(server: &mut ChunkMap) -> (f64, f64, f64) {
    let registries = server.generator().registries.clone();
    let blocks = &registries.blocks;
    for ring in 0..4i32 {
        for cz in -ring..=ring {
            for cx in -ring..=ring {
                if cx.abs().max(cz.abs()) != ring {
                    continue;
                }
                let chunk = server.load_now(minecraftoss_core::ChunkPos::new(cx, cz));
                for z in 0..16usize {
                    for x in 0..16usize {
                        for y in (32..110).rev() {
                            let floor = chunk.block(x, y - 1, z);
                            let solid = blocks.is(floor, minecraftoss_core::block::flags::SOLID_RENDER);
                            if solid && blocks.is_air(chunk.block(x, y, z)) && blocks.is_air(chunk.block(x, y + 1, z)) {
                                let bx = f64::from(cx * 16 + x as i32) + 0.5;
                                let bz = f64::from(cz * 16 + z as i32) + 0.5;
                                return (bx, f64::from(y), bz);
                            }
                        }
                    }
                }
            }
        }
    }
    (0.5, 64.0, 0.5)
}

impl TerrainStream {
    pub fn new(registries: Arc<Registries>, seed: i64, view_distance: i32) -> Result<Self> {
        Self::for_dimension(registries, seed, view_distance, Dimension::Overworld, None)
    }

    /// A streamed dimension; with `world`, chunks load from and save to that
    /// world directory's region files.
    pub fn for_dimension(registries: Arc<Registries>, seed: i64, view_distance: i32, dimension: Dimension, world: Option<&std::path::Path>) -> Result<Self> {
        let started = Instant::now();
        let generator = match dimension {
            Dimension::Overworld => TerrainGenerator::overworld(registries.clone(), seed),
            Dimension::Nether => TerrainGenerator::nether(registries.clone(), seed),
            Dimension::End => TerrainGenerator::end(registries.clone(), seed),
        }
        .map_err(|e| anyhow!(e))?;
        let mut states = BlockStates::new(registries, seed, generator.chunk_min_y, generator.chunk_height)?;
        states.sky_light = dimension != Dimension::Nether;
        let states = Arc::new(states);
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        // Generation and section building run at background priority, so
        // they may ask for more threads than there are cores: the render
        // and main threads still come first. Three quarters of the hardware
        // threads generate (a fast flight at 32 chunks needs about 400
        // chunks a second) and half less one build sections.
        // MINECRAFTOSS_GEN_THREADS and MINECRAFTOSS_SECTION_THREADS override.
        let env = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0);
        let generation_threads = env("MINECRAFTOSS_GEN_THREADS").unwrap_or((threads * 3 / 4).max(1));
        let section_threads = env("MINECRAFTOSS_SECTION_THREADS").unwrap_or((threads / 2).saturating_sub(1).max(1));
        // Chunks leaving the loaded area are always stored, as vanilla saves
        // them: without a world directory, in a session directory removed
        // when the stream closes. A chunk that comes back is loaded as it
        // was, never generated a second time over finished neighbours.
        let session = world.is_none().then(SessionDir::new).transpose()?;
        let world_path: &Path = match (world, &session) {
            (Some(w), _) => w,
            (None, Some(session)) => &session.0,
            (None, None) => unreachable!("a session directory exists without a world"),
        };
        let storage = Some(ChunkStorage::new(world_path, dimension.dimension_type(), states.registries().clone(), generator.chunk_min_y, generator.chunk_height));
        let worldgen = WorldGen::for_dimension(Arc::new(generator), dimension.dimension_type()).map_err(|e| anyhow!(e))?;
        eprintln!("world load: generator, features and structures in {:.2}s", started.elapsed().as_secs_f64());
        let mut server = ChunkMap::with_storage(Arc::new(worldgen), view_distance, generation_threads, storage);
        let searching = Instant::now();
        let (world_spawn, player_spawn) = match dimension {
            Dimension::Overworld => {
                let world_spawn = spawn::world_spawn(&mut server);
                (world_spawn, spawn::player_spawn(&mut server, world_spawn, spawn::DEFAULT_RESPAWN_RADIUS, unseeded()))
            }
            Dimension::Nether => {
                let (x, y, z) = nether_spawn(&mut server);
                ((x as i32, y as i32, z as i32), (x, y, z))
            }
            // ServerLevel.END_SPAWN_POINT: the obsidian platform at 100, 49, 0.
            Dimension::End => ((100, 49, 0), (100.5, 49.0, 0.5)),
        };
        eprintln!("spawn found in {:.2}s ({} chunks generated)", searching.elapsed().as_secs_f64(), server.stats().generated);
        let (min_section, max_section) = states.section_range();
        let work: Work = Arc::new((
            Mutex::new(WorkQueue {
                light: Vec::new(),
                compile: CompileQueue::default(),
                eye: DVec3::ZERO,
                active: 0,
                stop: false,
            }),
            Condvar::new(),
        ));
        let (sender, done) = mpsc::channel();
        let workers = (0..section_threads)
            .map(|index| {
                let (work, sender) = (work.clone(), sender.clone());
                std::thread::Builder::new()
                    .name(format!("section-builder-{index}"))
                    .spawn(move || {
                        minecraftoss_core::thread_priority::background();
                        worker(&work, &sender)
                    })
                    .map_err(Into::into)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            states,
            world_spawn,
            player_spawn,
            sections: Sections::new(server.view_distance(), min_section, max_section),
            server,
            _session: session,
            work,
            done,
            workers,
            lights: HashMap::new(),
            light_epochs: HashMap::new(),
            lights_pending: HashSet::new(),
            serials: HashMap::new(),
            next_serial: 0,
            player_sections: HashSet::new(),
            player_relights: HashSet::new(),
            removed: Vec::new(),
            light_changes: Vec::new(),
            scheduled_last_frame: 0,
            clock: Instant::now(),
            stats: Stats::default(),
        })
    }

    /// Flush live chunks for the host game autosave.
    pub fn save_all(&mut self) { self.server.save_all(); }

    pub fn view_distance(&self) -> i32 {
        self.server.view_distance()
    }

    /// Generation threads started for this stream.
    pub fn generation_threads(&self) -> usize {
        self.server.workers()
    }

    /// How many generation threads take work (host change; see
    /// `ChunkMap::set_active_workers`).
    pub fn set_generation_threads(&self, n: usize) {
        self.server.set_active_workers(n);
    }

    /// A new render distance from the options screen: the server tracks
    /// the new area and every section is rebuilt on a new grid, as vanilla's
    /// `LevelRenderer.allChanged`. Whether it changed (the renderer then
    /// drops its sections).
    pub fn set_view_distance(&mut self, scene: &HandcraftedScene, view_distance: i32) -> bool {
        self.server.set_view_distance(view_distance);
        let view_distance = self.server.view_distance();
        if view_distance == self.sections.view_distance() {
            return false;
        }
        let (min_section, max_section) = self.states.section_range();
        self.sections = Sections::new(view_distance, min_section, max_section);
        for pos in scene.generated_chunks().collect::<Vec<_>>() {
            let chunk = scene.generated_chunk(pos).expect("listed").clone();
            let empty = self.empty_sections(scene, &chunk);
            self.sections.chunk_loaded(pos, empty);
        }
        // Builds queued for the old grid are dropped.
        self.serials.clear();
        self.player_sections.clear();
        self.removed.clear();
        self.work.0.lock().expect("terrain work queue").compile.retain(|_, _| false);
        true
    }

    /// `PlayerSpawnFinder.findSpawn` again, for a respawn.
    pub fn respawn_position(&mut self) -> (f64, f64, f64) {
        spawn::player_spawn(&mut self.server, self.world_spawn, spawn::DEFAULT_RESPAWN_RADIUS, unseeded())
    }

    fn now(&self) -> u64 {
        self.clock.elapsed().as_millis() as u64
    }

    fn empty_sections(&self, scene: &HandcraftedScene, chunk: &Chunk) -> Vec<i32> {
        let placed = scene.chunk_edits((chunk.pos.x, chunk.pos.z)).0;
        chunk
            .sections()
            .iter()
            .enumerate()
            .map(|(i, section)| (chunk.min_section_y() + i as i32, section))
            .filter(|(y, section)| {
                section.is_empty()
                    && !placed.is_some_and(|placed| placed.keys().any(|pos| pos.1 >> 4 == *y))
            })
            .map(|(y, _)| y)
            .collect()
    }

    /// Queues a (re)light of a chunk once its neighborhood is loaded.
    fn request_light(&mut self, scene: &HandcraftedScene, chunk: ChunkPos) {
        let Some(hood) = Neighborhood::of(scene, chunk) else {
            return;
        };
        let epoch = self.light_epochs.entry(chunk).or_default();
        *epoch += 1;
        let job = LightJob {
            previous: self.lights.get(&chunk).cloned(),
            chunk,
            epoch: *epoch,
            hood,
        };
        self.lights_pending.insert(chunk);
        let (lock, ready) = &*self.work;
        let mut queue = lock.lock().expect("terrain work queue");
        queue.light.retain(|(queued, _)| queued.chunk != chunk);
        queue.light.push((job, self.states.clone()));
        ready.notify_one();
    }

    /// The world generation data the chunk map runs.
    pub fn world_gen(&self) -> Arc<minecraftoss_world::chunk_map::WorldGen> {
        self.server.world_gen().clone()
    }

    /// The storage chunks are saved to, which the level's entities share.
    pub fn storage(&self) -> Option<Arc<minecraftoss_world::storage::ChunkStorage>> {
        self.server.storage()
    }

    /// One server tick: runs the chunk map for the player and applies what it
    /// sends to the scene. Returns the chunks it loaded and the positions it
    /// forgot, for the world simulation.
    pub fn server_tick(&mut self, player: BlockPos, scene: &mut HandcraftedScene) -> (Vec<Arc<Chunk>>, Vec<minecraftoss_core::ChunkPos>) {
        let player_chunk = minecraftoss_core::ChunkPos::new(player.0 >> 4, player.2 >> 4);
        let (mut loaded, mut forgotten) = (Vec::new(), Vec::new());
        // Columns whose queued work is cancelled, in one pass at the end.
        let mut cancelled: crate::fast_hash::FxHashSet<ChunkPos> = Default::default();
        let events = {
            let _span = span("chunk_map.tick");
            self.server.tick(player_chunk)
        };
        let [tracking, collecting, lock_wait, scheduling, sending, locked, evicting] = self.server.last_tick_ms;
        crate::frame_spans::record("  chunk_map: schedule under lock", locked);
        crate::frame_spans::record("  chunk_map: evict", evicting);
        crate::frame_spans::record("  chunk_map: tracking", tracking);
        crate::frame_spans::record("  chunk_map: collect", collecting);
        crate::frame_spans::record("  chunk_map: world lock wait", lock_wait);
        crate::frame_spans::record("  chunk_map: schedule", scheduling);
        crate::frame_spans::record("  chunk_map: send", sending);
        for event in events {
            match event {
                ChunkEvent::Center(_) => {}
                ChunkEvent::Load(chunk) => {
                    let _span = span("chunk load (client)");
                    loaded.push(chunk.clone());
                    let pos = (chunk.pos.x, chunk.pos.z);
                    let empty = {
                        let _span = span("  empty_sections");
                        self.empty_sections(scene, &chunk)
                    };
                    {
                        let _span = span("  scene.insert_chunk");
                        scene.insert_chunk(chunk);
                    }
                    self.sections.chunk_loaded(pos, empty);
                    for x in pos.0 - 1..=pos.0 + 1 {
                        for z in pos.1 - 1..=pos.1 + 1 {
                            if !self.lights.contains_key(&(x, z)) && !self.lights_pending.contains(&(x, z)) {
                                self.request_light(scene, (x, z));
                            }
                        }
                    }
                }
                ChunkEvent::Forget(pos) => {
                    let _span = span("chunk forget (client)");
                    forgotten.push(pos);
                    let pos = (pos.x, pos.z);
                    if scene.generated_chunk(pos).is_none() {
                        continue;
                    }
                    scene.remove_chunk(pos);
                    self.removed.extend(self.sections.chunk_unloaded(pos));
                    *self.light_epochs.entry(pos).or_default() += 1;
                    self.lights_pending.remove(&pos);
                    self.player_relights.remove(&pos);
                    self.player_sections.retain(|s| (s.0, s.2) != pos);
                    if self.lights.remove(&pos).is_some() {
                        self.light_changes.push((pos, None));
                    }
                    cancelled.insert(pos);
                }
            }
        }
        if !cancelled.is_empty() {
            let _span = span("  cancel queued work");
            let (lock, _) = &*self.work;
            let mut queue = lock.lock().expect("terrain work queue");
            queue.light.retain(|(job, _)| !cancelled.contains(&job.chunk));
            queue.compile.retain(|section, _| !cancelled.contains(&(section.0, section.2)));
        }
        (loaded, forgotten)
    }

    /// Hands edited positions to the integrated server, whose chunks are
    /// the ones saved: each takes the block the scene now shows there.
    pub fn record_edits(&mut self, scene: &HandcraftedScene, positions: &[BlockPos]) {
        let edits: Vec<(minecraftoss_core::BlockPos, minecraftoss_core::BlockStateId)> = positions
            .iter()
            .map(|&(x, y, z)| {
                let state = Scene::block(scene, (x, y, z)).and_then(|b| self.states.state_of(b)).unwrap_or(minecraftoss_core::BlockStateId::AIR);
                (minecraftoss_core::BlockPos::new(x, y, z), state)
            })
            .collect();
        let _span = span("  edits: chunk map");
        self.server.set_blocks(&edits);
    }

    /// An edit at these positions: nearby sections rebuild for the player
    /// (`LevelRenderer.setBlockDirty`), and nearby chunks relight; sections
    /// whose light changed rebuild when that finishes.
    pub fn mark_edited(&mut self, scene: &HandcraftedScene, positions: &[BlockPos]) {
        let mut sections = BTreeSet::new();
        let mut chunks = BTreeSet::new();
        let mut touched: BTreeMap<crate::sections::SectionPos, bool> = BTreeMap::new();
        for &(x, y, z) in positions {
            for sx in (x - 1) >> 4..=(x + 1) >> 4 {
                for sy in (y - 1) >> 4..=(y + 1) >> 4 {
                    for sz in (z - 1) >> 4..=(z + 1) >> 4 {
                        sections.insert((sx, sy, sz));
                    }
                }
            }
            for cx in (x - 15) >> 4..=(x + 15) >> 4 {
                for cz in (z - 15) >> 4..=(z + 15) >> 4 {
                    chunks.insert((cx, cz));
                }
            }
            // A section holding any changed non-air block is not empty.
            let section = (x >> 4, y >> 4, z >> 4);
            let placed = Scene::block(scene, (x, y, z)).is_some();
            let entry = touched.entry(section).or_insert(false);
            *entry |= placed;
        }
        // Whether each touched section now holds only air, checked once.
        let empty_span = span("  edits: emptiness");
        for (section, placed) in touched {
            let empty = !placed && scene.section_is_empty(section);
            self.sections.set_empty(section, empty);
        }
        drop(empty_span);
        for pos in sections {
            if self.sections.section(pos).is_some() {
                self.sections.set_dirty(pos, true);
                self.player_sections.insert(pos);
            }
        }
        let _span = span("  edits: relight requests");
        for chunk in chunks {
            if self.lights.contains_key(&chunk) {
                self.player_relights.insert(chunk);
                self.request_light(scene, chunk);
            }
        }
    }

    /// Rebuilds every section, after a resource reload.
    pub fn remesh_all(&mut self, scene: &HandcraftedScene) {
        for chunk in scene.generated_chunks().collect::<Vec<_>>() {
            self.sections.set_range_dirty(chunk, false);
        }
    }

    /// Whether every edit-triggered rebuild has finished.
    pub fn edits_settled(&self) -> bool {
        self.player_sections.is_empty() && self.player_relights.is_empty()
    }

    /// Everything in view is generated, sent, lit and built.
    pub fn is_settled(&self) -> bool {
        let queue = self.work.0.lock().expect("terrain work queue");
        self.server.is_idle()
            && queue.light.is_empty()
            && queue.compile.is_empty()
            && queue.active == 0
            && self.lights_pending.is_empty()
            && self.scheduled_last_frame == 0
    }

    /// One frame: collect finished work, update the section graph from the
    /// camera, schedule what vanilla would build next, and report renderer
    /// changes.
    pub fn frame(
        &mut self,
        scene: &HandcraftedScene,
        camera: &CullCamera,
        fade_millis: u64,
        atlas: &Arc<Atlas>,
        packs: &PackStack,
    ) -> FrameUpdate {
        let now = self.now();
        let mut update = FrameUpdate {
            removed: std::mem::take(&mut self.removed),
            ..FrameUpdate::default()
        };
        let drain_span = span("terrain.frame drain");
        let drain_started = std::time::Instant::now();
        let mut upload_vertices = 0usize;
        while update.uploads.len() < 2 && upload_vertices < 120_000 && drain_started.elapsed().as_micros() < 1500 {
            let Ok(done) = self.done.try_recv() else { break };
            match done {
                Done::Light { chunk, epoch, light, changed_sections, micros } => {
                    let _span = span("  drain light");
                    if self.light_epochs.get(&chunk) != Some(&epoch) || scene.generated_chunk(chunk).is_none() {
                        continue;
                    }
                    self.stats.lit += 1;
                    self.stats.light_micros += micros;
                    self.lights_pending.remove(&chunk);
                    let from_player = self.player_relights.remove(&chunk);
                    if self.lights.insert(chunk, light.clone()).is_some() {
                        let _span = span("  drain light differs");
                        // A light section update rebuilds that section and its
                        // neighbors (`setSectionDirtyWithNeighbors`).
                        for sy in changed_sections {
                            {
                                for dx in -1..=1 {
                                    for dy in -1..=1 {
                                        for dz in -1..=1 {
                                            let pos = (chunk.0 + dx, sy + dy, chunk.1 + dz);
                                            self.sections.set_dirty(pos, from_player);
                                            if from_player && self.sections.section(pos).is_some() {
                                                self.player_sections.insert(pos);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    self.light_changes.push((chunk, Some(light)));
                }
                Done::Compile { section, serial, pack_generation, micros, outcome } => {
                    let _span = span("  drain compile");
                    if self.serials.get(&section) != Some(&serial) || pack_generation != packs.generation {
                        continue;
                    }
                    self.serials.remove(&section);
                    self.player_sections.remove(&section);
                    self.stats.compiled += 1;
                    self.stats.compile_micros += micros;
                    match outcome {
                        Ok((mesh, visibility)) => {
                            if self.sections.section(section).is_some() {
                                self.sections.compiled(section, visibility, now);
                                upload_vertices += mesh.vertices.len();
                                update.uploads.push((section, mesh));
                            }
                        }
                        Err(e) => eprintln!("section {section:?} could not build: {e:#}"),
                    }
                }
            }
        }
        drop(drain_span);
        update.lights = std::mem::take(&mut self.light_changes);
        {
            let _span = span("sections.reposition");
            update.removed.extend(self.sections.reposition(camera));
        }
        let lights = &self.lights;
        let update_span = span("sections.update");
        let scheduled = self.sections.update(camera, now, |(x, _, z)| {
            lights.contains_key(&(x, z))
                && (-1..=1).all(|dx| (-1..=1).all(|dz| scene.generated_chunk((x + dx, z + dz)).is_some()))
        });
        drop(update_span);
        let _schedule_span = span("terrain.frame schedule");
        let context = JobContext {
            states: self.states.clone(),
            atlas: atlas.clone(),
            pack_sources: packs.sources().to_vec(),
            pack_generation: packs.generation,
        };
        let mut jobs = Vec::new();
        for (pos, recompile) in scheduled.compile {
            let chunk = (pos.0, pos.2);
            let (Some(hood), Some(light)) = (Neighborhood::of(scene, chunk), self.lights.get(&chunk)) else {
                // Wait for the missing neighbor or light.
                self.sections.set_dirty(pos, false);
                continue;
            };
            self.next_serial += 1;
            self.serials.insert(pos, self.next_serial);
            jobs.push((
                pos,
                recompile,
                CompileJob {
                    serial: self.next_serial,
                    hood,
                    light: light.clone(),
                    context: context.clone(),
                },
            ));
        }
        // Sections still waiting on a neighbour or light are not work.
        self.scheduled_last_frame = jobs.len();
        {
            let (lock, ready) = &*self.work;
            let mut queue = lock.lock().expect("terrain work queue");
            queue.eye = camera.position;
            // Scheduling a section cancels its earlier task.
            if !jobs.is_empty() {
                let rescheduled: crate::fast_hash::FxHashSet<SectionPos> = jobs.iter().map(|(pos, _, _)| *pos).collect();
                queue.compile.retain(|queued, _| !rescheduled.contains(&queued));
            }
            for (pos, recompile, job) in jobs {
                queue.compile.push(pos, recompile, job);
            }
            if !queue.compile.is_empty() || !queue.light.is_empty() {
                ready.notify_all();
            }
        }
        update.visible = self
            .sections
            .visible()
            .iter()
            .filter_map(|&pos| {
                let section = self.sections.section(pos)?;
                matches!(section.mesh, crate::sections::MeshState::Compiled(_))
                    .then(|| (pos, section.visibility(now, fade_millis)))
            })
            .collect();
        update
    }

    /// Diagnostics (F9): every chunk column within the view distance, by
    /// where it stands in the pipeline, nearest examples first.
    pub fn hole_report(&self, scene: &HandcraftedScene, gpu_has: &dyn Fn(SectionPos) -> bool) -> String {
        use std::collections::BTreeMap;
        let (cx, _, cz) = self.sections.camera_section();
        let vd = self.server.view_distance();
        let mut columns: BTreeMap<&'static str, Vec<(i32, ChunkPos, String)>> = BTreeMap::new();
        let mut sections: BTreeMap<&'static str, Vec<(i32, SectionPos)>> = BTreeMap::new();
        let (min_section, max_section) = self.states.section_range();
        for x in cx - vd..=cx + vd {
            for z in cz - vd..=cz + vd {
                let d = (x - cx).abs().max((z - cz).abs());
                let pos = (x, z);
                let category = if scene.generated_chunk(pos).is_none() {
                    "not in scene (server has not sent it)"
                } else if !self.lights.contains_key(&pos) {
                    if self.lights_pending.contains(&pos) { "no light (pending)" } else { "no light (never requested)" }
                } else {
                    "in scene and lit"
                };
                let server = if category.starts_with("not in scene") { self.server.debug_slot(minecraftoss_core::ChunkPos::new(x, z)) } else { String::new() };
                columns.entry(category).or_default().push((d, pos, server));
                if category != "in scene and lit" {
                    continue;
                }
                for y in min_section..=max_section {
                    let section = (x, y, z);
                    let label = match self.sections.debug_section(section) {
                        None if self.sections.is_empty_section(section) => continue,
                        None => "no section yet (not reached: occluded or pending)",
                        Some((mesh, dirty, in_graph, reached, visible)) => match mesh {
                            crate::sections::MeshState::Empty => continue,
                            crate::sections::MeshState::Uncompiled if self.serials.contains_key(&section) => "uncompiled, compile queued",
                            crate::sections::MeshState::Uncompiled if dirty && !in_graph => "uncompiled, not reached by graph",
                            crate::sections::MeshState::Uncompiled if dirty && !visible => "uncompiled, reached but not in frustum",
                            crate::sections::MeshState::Uncompiled if dirty => "uncompiled, visible and dirty (not ready?)",
                            crate::sections::MeshState::Uncompiled => "uncompiled, not dirty",
                            crate::sections::MeshState::Compiled(_) if !reached => "compiled, not reached by graph",
                            crate::sections::MeshState::Compiled(_) if visible && !gpu_has(section) => "compiled and visible, not on the GPU",
                            crate::sections::MeshState::Compiled(_) => continue,
                        },
                    };
                    sections.entry(label).or_default().push((d, section));
                }
            }
        }
        let mut out = format!("hole report at camera section {:?}, view distance {vd}\n{}\n", self.sections.camera_section(), self.server.debug_queue());
        for (category, mut list) in columns {
            list.sort();
            out += &format!("  columns {category}: {}\n", list.len());
            if category != "in scene and lit" {
                for (d, pos, server) in list.iter().take(12) {
                    out += &format!("      d{d} {pos:?} {server}\n");
                }
            }
        }
        for (label, mut list) in sections {
            list.sort();
            out += &format!("  sections {label}: {}\n", list.len());
            for (d, pos) in list.iter().take(12) {
                out += &format!("      d{d} {pos:?}\n");
            }
        }
        out
    }

    /// Loading progress around `center` for the load profile: how many
    /// rings are complete at each stage, and the pipeline's counters.
    pub fn load_metrics(&self, scene: &HandcraftedScene, center: ChunkPos) -> LoadMetrics {
        let vd = self.server.view_distance();
        // The tracked area is a circle (`ChunkTrackingView`); a column can
        // be lit once its whole 3x3 is tracked.
        let view = minecraftoss_world::TrackingView::new(minecraftoss_core::ChunkPos::new(center.0, center.1), vd);
        let tracked = |x: i32, z: i32| view.contains(minecraftoss_core::ChunkPos::new(x, z));
        let (mut columns, mut missing_sent, mut missing_lit) = (0, 0, 0);
        let (mut sent_radius, mut lit_radius) = (f64::from(vd + 1), f64::from(vd + 1));
        for x in center.0 - vd - 1..=center.0 + vd + 1 {
            for z in center.1 - vd - 1..=center.1 + vd + 1 {
                if !tracked(x, z) {
                    continue;
                }
                columns += 1;
                let distance = f64::from((x - center.0).pow(2) + (z - center.1).pow(2)).sqrt();
                let sent = scene.generated_chunk((x, z)).is_some();
                if !sent {
                    missing_sent += 1;
                    sent_radius = sent_radius.min(distance);
                }
                let litable = (-1..=1).all(|dx| (-1..=1).all(|dz| tracked(x + dx, z + dz)));
                if litable && !(sent && self.lights.contains_key(&(x, z))) {
                    missing_lit += 1;
                    lit_radius = lit_radius.min(distance);
                }
            }
        }
        let visible = self.sections.visible();
        let visible_compiled = visible
            .iter()
            .filter(|&&pos| self.sections.section(pos).is_some_and(|s| matches!(s.mesh, crate::sections::MeshState::Compiled(_))))
            .count();
        let stats = self.server.stats();
        let queue = self.work.0.lock().expect("terrain work queue");
        LoadMetrics {
            columns,
            missing_sent,
            missing_lit,
            sent_radius,
            lit_radius,
            visible: visible.len(),
            visible_compiled,
            generated: stats.generated,
            decorated: stats.decorated,
            sent: scene.generated_chunks().count(),
            lit: self.stats.lit,
            compiled: self.stats.compiled,
            light_queue: queue.light.len(),
            compile_queue: queue.compile.len(),
            generation_micros: stats.mean_generation_micros,
            decoration_micros: stats.mean_decoration_micros,
            light_micros: self.stats.light_micros.checked_div(self.stats.lit).unwrap_or(0),
            compile_micros: self.stats.compile_micros.checked_div(self.stats.compiled).unwrap_or(0),
        }
    }

    /// Pending worker light jobs and their lifetime mean duration (not render-thread cost).
    pub fn lighting_stats(&self) -> (usize, u64) { (self.lights_pending.len(), self.stats.light_micros.checked_div(self.stats.lit).unwrap_or(0)) }

    /// Detailed state for the diagnostics log.
    pub fn diag(&self) -> String {
        format!(
            "{} | {} | lights {} pending {} | player sections {} relights {}",
            self.debug_line(),
            self.sections.diag(),
            self.lights.len(),
            self.lights_pending.len(),
            self.player_sections.len(),
            self.player_relights.len(),
        )
    }

    /// The F3 chunk lines: server and client progress.
    pub fn debug_line(&self) -> String {
        let stats = self.server.stats();
        let queue = self.work.0.lock().expect("terrain work queue");
        format!(
            "Chunks: {} loaded, {} queued, {} generating ({} us/chunk); light {} queued ({} us); sections {} visible, {} queued, {} building ({} us/section)",
            stats.loaded,
            stats.queued,
            stats.generating,
            stats.mean_generation_micros,
            queue.light.len(),
            self.stats.light_micros.checked_div(self.stats.lit).unwrap_or(0),
            self.sections.visible().len(),
            queue.compile.len(),
            queue.active,
            self.stats.compile_micros.checked_div(self.stats.compiled).unwrap_or(0),
        )
    }
}

/// Loading progress for the load profile (`TerrainStream::load_metrics`).
#[derive(Clone, Copy, Debug, Default)]
pub struct LoadMetrics {
    /// Tracked columns (a circle around the player), and how many of them
    /// are not sent yet, or not lit (of those whose 3x3 is tracked).
    pub columns: usize,
    pub missing_sent: usize,
    pub missing_lit: usize,
    /// The distance in chunks to the nearest tracked column not sent, and
    /// not lit (one more than the view distance when there is none).
    pub sent_radius: f64,
    pub lit_radius: f64,
    /// Sections the graph reaches in the frustum, and how many are built.
    pub visible: usize,
    pub visible_compiled: usize,
    pub generated: u64,
    pub decorated: u64,
    pub sent: usize,
    pub lit: u64,
    pub compiled: u64,
    pub light_queue: usize,
    pub compile_queue: usize,
    pub generation_micros: u64,
    pub decoration_micros: u64,
    pub light_micros: u64,
    pub compile_micros: u64,
}

/// A temporary world directory for a session without one.
struct SessionDir(std::path::PathBuf);

impl SessionDir {
    fn new() -> Result<Self> {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("minecraftoss-session-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for SessionDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Drop for TerrainStream {
    fn drop(&mut self) {
        self.work.0.lock().expect("terrain work queue").stop = true;
        self.work.1.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cargo test --release -p minecraftoss-viewer --lib section_mesh -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the local data pack, block catalog and resource pack; slow in debug"]
    fn section_meshes_match_the_scene_mesher() {
        let Ok(paths) = minecraftoss_core::registries::DataPaths::discover() else {
            return;
        };
        let pack = paths
            .datapack
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|root| root.join("resourcepacks/local/minecraft-26.3"));
        let Some(pack) = pack.filter(|p| p.is_dir() && paths.block_catalog.is_file()) else {
            eprintln!("skipping: local data not restored");
            return;
        };
        let registries = Arc::new(Registries::load(&paths).unwrap());
        let seed = 1234;
        let generator = Arc::new(TerrainGenerator::overworld(registries.clone(), seed).unwrap());
        let states = Arc::new(
            BlockStates::new(registries, seed, generator.min_y, generator.height).unwrap(),
        );
        let mut server = ChunkMap::new(generator, 2, 4);
        let origin = minecraftoss_core::ChunkPos::new(0, 0);
        while {
            server.tick(origin);
            !server.is_idle()
        } {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut scene = HandcraftedScene::streamed(states.clone());
        for x in -1..=1 {
            for z in -1..=1 {
                let chunk = server.chunk(minecraftoss_core::ChunkPos::new(x, z)).unwrap();
                scene.insert_chunk(chunk.clone());
            }
        }
        // An edit shows through both paths.
        scene.set((3, 70, 4), Some(Block::new("minecraft:glass")));
        let packs = PackStack::open(vec![pack]).unwrap();
        let reference = mesh::build(&scene, &packs).unwrap();
        let tint = BiomeTint::from_pack(&packs).unwrap();
        let hood = Neighborhood::of(&scene, (0, 0)).unwrap();
        let started = Instant::now();
        let light = light_chunk(&states, &hood);
        eprintln!("chunk light: {:?}", started.elapsed());
        let global = reference.sky_light.as_ref().unwrap();
        for x in -1..17 {
            for z in -1..17 {
                for y in -65..330 {
                    assert_eq!(light.get((x, y, z)), global.get((x, y, z)), "sky {x} {y} {z}");
                    assert_eq!(light.get_block((x, y, z)), global.get_block((x, y, z)), "block {x} {y} {z}");
                }
            }
        }
        let mut models = ModelCache::default();
        let started = Instant::now();
        let mut vertices = Vec::new();
        let mut opaque = 0;
        for sy in -4..20 {
            let (mesh, _) =
                compile_section(&states, &hood, (0, sy, 0), &light, &mut models, &packs, &reference.atlas, &tint).unwrap();
            opaque += mesh.transparent_start.unwrap_or(mesh.indices.len() as u32) as usize / 6;
            vertices.extend(mesh.vertices);
        }
        eprintln!("24 sections: {:?}", started.elapsed());
        let expected = &reference.chunks[&(0, 0)];
        // Sections emit the same faces in a different order.
        let key = |v: &mesh::Vertex| format!("{v:?}");
        let mut got: Vec<String> = vertices.iter().map(key).collect();
        let mut want: Vec<String> = expected.vertices.iter().map(key).collect();
        got.sort();
        want.sort();
        assert_eq!(got.len(), want.len());
        assert!(got == want, "section vertices differ from the chunk mesher");
        assert_eq!(opaque, expected.transparent_start.unwrap_or(expected.indices.len() as u32) as usize / 6);
    }

    #[test]
    fn block_states_parse_into_scene_blocks() {
        let block = block_from_state("minecraft:grass_block[snowy=false]").unwrap();
        assert_eq!(block, Block::new("minecraft:grass_block").with("snowy", "false"));
        assert_eq!(color(&serde_json::json!("#3f76e4")), Some([0x3f, 0x76, 0xe4]));
        assert_eq!(color(&serde_json::json!(4159204)), Some([0x3f, 0x76, 0xe4]));
    }

    #[test]
    fn only_full_collision_blocks_shade_corners() {
        let Ok(paths) = minecraftoss_core::registries::DataPaths::discover() else { return };
        let registries = Registries::load(&paths).unwrap();
        let blocks = &registries.blocks;
        let darkens = |text: &str| {
            let state = blocks.parse_state(text).unwrap();
            shade_darkens(blocks, state, &block_from_state(&blocks.state_to_string(state)).unwrap())
        };
        for solid in ["minecraft:grass_block", "minecraft:stone", "minecraft:oak_leaves", "minecraft:mud", "minecraft:snow[layers=8]"] {
            assert!(darkens(solid), "{solid}");
        }
        for open in ["minecraft:short_grass", "minecraft:poppy", "minecraft:tall_grass", "minecraft:glass", "minecraft:red_stained_glass", "minecraft:snow[layers=2]", "minecraft:oak_slab", "minecraft:barrier"] {
            assert!(!darkens(open), "{open}");
        }
    }
}

/// Positions of the crystals on this seed's generated End pillars.
pub fn end_crystal_positions(seed:i64)->Vec<[f64;3]>{
    minecraftoss_generator::feature::kinds::the_end::spikes_for_seed(seed).into_iter().map(|s|[s.center_x as f64+0.5,s.height as f64+1.,s.center_z as f64+0.5]).collect()
}
