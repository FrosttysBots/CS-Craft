//! Structure templates (vanilla `StructureTemplate`, `StructurePlaceSettings`
//! and `StructureTemplateManager` for built-in templates).
//!
//! Source-informed from the pinned 26.3 JAR. Templates load from the data
//! pack's `structure/*.nbt` files. Placement keeps vanilla's order: palette
//! blocks sorted into full blocks, other blocks and block entities (each by
//! y, x, z); processors run per block and then finalize the whole list;
//! waterlogging and the fluid fill pass follow; then edge shape updates,
//! neighbour-shape updates and entities.

pub mod processor;
pub mod transform;

use super::blocks::FluidType;
use super::tree::{Bounds, VoxelShape};
use super::update::{shape_at_edge, update_shape};
use super::Ctx;
use minecraftoss_core::ident::Identifier;
use minecraftoss_core::nbt::{self, Tag};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{positional_seed, LegacyRandom, RandomSource};
use minecraftoss_core::{BlockPos, BlockStateId, Registries};
pub use processor::Processor;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
pub use transform::{BoundingBox, Mirror, Rotation, StateTransforms};

/// `StructureTemplate.StructureBlockInfo`.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockInfo {
    pub pos: BlockPos,
    pub state: BlockStateId,
    pub nbt: Option<Tag>,
}

/// `StructureTemplate.JigsawBlockInfo`.
#[derive(Clone, Debug)]
pub struct JigsawInfo {
    pub pos: BlockPos,
    pub front: Direction,
    pub top: Direction,
    pub rollable: bool,
    /// `None` only for a feature pool element's synthetic jigsaw.
    pub name: Option<Arc<str>>,
    pub pool: Arc<str>,
    pub target: Arc<str>,
    pub placement_priority: i32,
    pub selection_priority: i32,
}

impl JigsawInfo {
    /// `JigsawBlockInfo.parse`.
    fn parse(registries: &Registries, info: &BlockInfo) -> Option<Self> {
        let orientation = registries.blocks.property(info.state, "orientation")?;
        let (front, top) = orientation.split_once('_')?;
        let (front, top) = (Direction::from_name(front)?, Direction::from_name(top)?);
        let nbt = info.nbt.as_ref();
        let text = |key: &str| nbt.and_then(|n| n.get(key)).and_then(Tag::as_str).map(Arc::<str>::from);
        let int = |key: &str| nbt.and_then(|n| n.get(key)).and_then(Tag::as_i64).unwrap_or(0) as i32;
        let empty: Arc<str> = Arc::from("minecraft:empty");
        let rollable = match text("joint").as_deref() {
            Some("rollable") => true,
            Some("aligned") => false,
            _ => !front.is_horizontal(),
        };
        Some(Self {
            pos: info.pos,
            front,
            top,
            rollable,
            name: Some(text("name").unwrap_or_else(|| empty.clone())),
            pool: text("pool").unwrap_or_else(|| empty.clone()),
            target: text("target").unwrap_or(empty),
            placement_priority: int("placement_priority"),
            selection_priority: int("selection_priority"),
        })
    }

    /// `JigsawBlock.canAttach`.
    pub fn can_attach(&self, target: &JigsawInfo) -> bool {
        self.front == target.front.opposite() && (self.rollable || self.top == target.top) && target.name.as_ref().is_none_or(|name| self.target == *name)
    }
}

/// `StructureTemplate.StructureEntityInfo`.
#[derive(Clone, Debug)]
pub struct EntityInfo {
    pub pos: [f64; 3],
    pub block_pos: BlockPos,
    pub nbt: Tag,
}

/// A loaded structure template.
#[derive(Debug, Default)]
pub struct Template {
    pub size: [i32; 3],
    palettes: Vec<Vec<BlockInfo>>,
    entities: Vec<EntityInfo>,
    /// Each palette's parsed jigsaw blocks, in palette order.
    jigsaws: std::sync::OnceLock<Vec<Vec<JigsawInfo>>>,
}

/// Blocks with `hasDynamicShape`, which never count as full blocks when a
/// palette is sorted.
const DYNAMIC_SHAPE: &[&str] = &[
    "minecraft:moving_piston",
    "minecraft:bamboo",
    "minecraft:scaffolding",
    "minecraft:powder_snow",
    "minecraft:pointed_dripstone",
    "minecraft:sulfur_spike",
    "minecraft:firefly_bush",
];

fn ints3(tag: Option<&Tag>) -> [i32; 3] {
    let v = tag.and_then(Tag::as_ints).unwrap_or_default();
    [v.first().copied().unwrap_or(0), v.get(1).copied().unwrap_or(0), v.get(2).copied().unwrap_or(0)]
}

/// `NbtUtils.readBlockState`: unknown blocks read as air, unknown
/// properties and values are skipped.
fn read_state(registries: &Registries, tag: &Tag) -> BlockStateId {
    let blocks = &registries.blocks;
    let Some(block) = tag.get("id").and_then(Tag::as_str).and_then(|name| blocks.block_by_name(name)) else {
        return BlockStateId::AIR;
    };
    let mut state = blocks.block(block).default_state();
    if let Some(props) = tag.get("properties").and_then(Tag::as_compound) {
        for (key, value) in props {
            if let Some(value) = value.as_str() {
                state = blocks.with_property(state, key, value).unwrap_or(state);
            }
        }
    }
    state
}

impl Template {
    /// `StructureTemplate.load`.
    pub fn load(registries: &Registries, tag: &Tag) -> Self {
        let size = ints3(tag.get("size"));
        let blocks: &[Tag] = tag.get("blocks").and_then(Tag::as_list).unwrap_or(&[]);
        let mut palettes = Vec::new();
        if let Some(lists) = tag.get("palettes").and_then(Tag::as_list) {
            for list in lists {
                palettes.push(Self::load_palette(registries, list.as_list().unwrap_or(&[]), blocks));
            }
        } else {
            palettes.push(Self::load_palette(registries, tag.get("palette").and_then(Tag::as_list).unwrap_or(&[]), blocks));
        }
        let mut entities = Vec::new();
        for entity in tag.get("entities").and_then(Tag::as_list).unwrap_or(&[]) {
            let pos = entity.get("pos").and_then(Tag::as_list).unwrap_or(&[]);
            let d = |i: usize| pos.get(i).and_then(Tag::as_f64).unwrap_or(0.0);
            let [bx, by, bz] = ints3(entity.get("blockPos"));
            if let Some(nbt) = entity.get("nbt").filter(|n| n.as_compound().is_some()) {
                entities.push(EntityInfo { pos: [d(0), d(1), d(2)], block_pos: BlockPos::new(bx, by, bz), nbt: nbt.clone() });
            }
        }
        Self { size, palettes, entities, jigsaws: std::sync::OnceLock::new() }
    }

    fn load_palette(registries: &Registries, palette: &[Tag], blocks: &[Tag]) -> Vec<BlockInfo> {
        let states: Vec<BlockStateId> = palette.iter().map(|t| read_state(registries, t)).collect();
        let (mut full, mut entities, mut other) = (Vec::new(), Vec::new(), Vec::new());
        for block in blocks {
            let [x, y, z] = ints3(block.get("pos"));
            let index = block.get("state").and_then(Tag::as_i64).unwrap_or(0);
            let state = usize::try_from(index).ok().and_then(|i| states.get(i)).copied().unwrap_or(BlockStateId::AIR);
            let nbt = block.get("nbt").filter(|n| n.as_compound().is_some()).cloned();
            let info = BlockInfo { pos: BlockPos::new(x, y, z), state, nbt };
            let name = registries.blocks.block(registries.blocks.block_of(state)).name.as_str();
            if info.nbt.is_some() {
                entities.push(info);
            } else if !DYNAMIC_SHAPE.contains(&name) && registries.blocks.state(state).collision_full_block {
                full.push(info);
            } else {
                other.push(info);
            }
        }
        let key = |b: &BlockInfo| (b.pos.y, b.pos.x, b.pos.z);
        full.sort_by_key(key);
        other.sort_by_key(key);
        entities.sort_by_key(key);
        full.extend(other);
        full.extend(entities);
        full
    }

    pub fn is_empty(&self) -> bool {
        self.palettes.is_empty()
    }

    /// `StructureTemplate.getJigsaws`: the jigsaw blocks of the palette
    /// `Mth.getSeed(position)` picks, moved to `position` and rotated.
    /// Each palette's jigsaws are parsed once (`Palette.jigsaws`).
    pub fn jigsaws(&self, registries: &Registries, position: BlockPos, rotation: Rotation) -> Vec<JigsawInfo> {
        if self.palettes.is_empty() {
            return Vec::new();
        }
        let parsed = self.jigsaws.get_or_init(|| {
            let jigsaw = registries.blocks.block_by_name("minecraft:jigsaw");
            self.palettes
                .iter()
                .map(|palette| {
                    palette.iter().filter(|b| Some(registries.blocks.block_of(b.state)) == jigsaw).filter_map(|b| JigsawInfo::parse(registries, b)).collect()
                })
                .collect()
        });
        let mut random = LegacyRandom::new(positional_seed(position.x, position.y, position.z));
        let palette = &parsed[random.next_i32_bound(self.palettes.len() as i32) as usize];
        palette
            .iter()
            .map(|j| JigsawInfo {
                pos: transform::transform(j.pos, Mirror::None, rotation, BlockPos::new(0, 0, 0)).offset(position.x, position.y, position.z),
                front: rotation.rotate(j.front),
                top: rotation.rotate(j.top),
                ..j.clone()
            })
            .collect()
    }

    /// `StructureTemplate.filterBlocks(position, settings, block)`: one
    /// block type's entries in the settings' palette, at world positions
    /// inside the settings' bounds, rotated (not mirrored).
    pub fn filter_blocks<R: RandomSource>(&self, lib: &super::Library, position: BlockPos, settings: &PlaceSettings, block: &str, random: &mut R) -> Vec<BlockInfo> {
        if self.palettes.is_empty() {
            return Vec::new();
        }
        let id = lib.registries.blocks.block_by_name(block);
        let palette = self.random_palette(settings, random, position);
        palette
            .iter()
            .filter(|b| Some(lib.registries.blocks.block_of(b.state)) == id)
            .filter_map(|b| {
                let pos = transform::transform(b.pos, settings.mirror, settings.rotation, settings.pivot).offset(position.x, position.y, position.z);
                if settings.bounding_box.is_some_and(|bb| !bb.is_inside(pos)) {
                    return None;
                }
                Some(BlockInfo { pos, state: lib.transforms.rotate(b.state, settings.rotation), nbt: b.nbt.clone() })
            })
            .collect()
    }

    /// `StructureTemplate.getSize(Rotation)`.
    pub fn size(&self, rotation: Rotation) -> [i32; 3] {
        match rotation {
            Rotation::Clockwise90 | Rotation::Counterclockwise90 => [self.size[2], self.size[1], self.size[0]],
            _ => self.size,
        }
    }

    pub fn zero_position_with_transform(&self, zero: BlockPos, mirror: Mirror, rotation: Rotation) -> BlockPos {
        transform::zero_position_with_transform(zero, mirror, rotation, self.size[0], self.size[2])
    }

    /// `StructureTemplate.getBoundingBox(settings, position)`.
    pub fn bounding_box(&self, settings: &PlaceSettings, position: BlockPos) -> BoundingBox {
        let (m, r, p) = (settings.mirror, settings.rotation, settings.pivot);
        let a = transform::transform(BlockPos::new(0, 0, 0), m, r, p);
        let b = transform::transform(BlockPos::new(self.size[0] - 1, self.size[1] - 1, self.size[2] - 1), m, r, p);
        BoundingBox::from_corners(a, b).moved(position.x, position.y, position.z)
    }

    /// `StructurePlaceSettings.getRandomPalette`: always draws, even with one palette.
    fn random_palette<R: RandomSource>(&self, settings: &PlaceSettings, random: &mut R, pos: BlockPos) -> &[BlockInfo] {
        let index = settings.random_at(random, pos).next_i32_bound(self.palettes.len() as i32);
        &self.palettes[index as usize]
    }

    /// `StructureTemplate.placeInWorld`.
    pub fn place_in_world<R: RandomSource>(
        &self,
        ctx: &mut Ctx,
        position: BlockPos,
        reference: BlockPos,
        settings: &PlaceSettings,
        random: &mut R,
        flags: u32,
    ) -> bool {
        if self.palettes.is_empty() {
            return false;
        }
        let blocks = self.random_palette(settings, random, position);
        if (blocks.is_empty() && (settings.ignore_entities || self.entities.is_empty())) || self.size.iter().any(|&s| s < 1) {
            return false;
        }
        let processed = process_block_infos(ctx, position, reference, settings, blocks, random);
        let lib = ctx.lib;
        let registries = &lib.registries;
        let mut to_fill: Vec<BlockPos> = Vec::new();
        let mut locked: Vec<BlockPos> = Vec::new();
        let mut placed: Vec<(BlockPos, bool)> = Vec::with_capacity(processed.len());
        let (mut min, mut max) = (BlockPos::new(i32::MAX, i32::MAX, i32::MAX), BlockPos::new(i32::MIN, i32::MIN, i32::MIN));
        for mut info in processed {
            let pos = info.pos;
            if settings.bounding_box.is_some_and(|b| !b.is_inside(pos)) {
                continue;
            }
            let previous = settings.waterlogging.then(|| ctx.fluid_at(pos));
            let state = lib.transforms.rotate(lib.transforms.mirror(info.state, settings.mirror), settings.rotation);
            if info.nbt.is_some() {
                ctx.set_block_flags(pos, lib.processor_blocks.barrier, 820);
            }
            if !ctx.set_block_flags(pos, state, flags) {
                continue;
            }
            min = BlockPos::new(min.x.min(pos.x), min.y.min(pos.y), min.z.min(pos.z));
            max = BlockPos::new(max.x.max(pos.x), max.y.max(pos.y), max.z.max(pos.z));
            placed.push((pos, info.nbt.is_some()));
            if let Some(nbt) = &mut info.nbt {
                if registries.blocks.is(state, minecraftoss_core::block::flags::HAS_BLOCK_ENTITY) {
                    if is_randomizable_container(ctx.name(state)) {
                        if let Tag::Compound(map) = nbt {
                            map.insert("LootTableSeed".into(), Tag::Long(random.next_i64()));
                        }
                    }
                    ctx.region.load_block_entity(pos.x, pos.y, pos.z, nbt);
                }
            }
            if let Some(previous) = previous {
                if is_source(ctx.fluid(state)) {
                    locked.push(pos);
                } else if is_liquid_container(ctx, state) {
                    place_liquid(ctx, pos, state, previous);
                    if !is_source(previous) {
                        to_fill.push(pos);
                    }
                }
            }
        }
        const FILL: [Direction; 5] = [Direction::Up, Direction::North, Direction::East, Direction::South, Direction::West];
        let mut filled = true;
        while filled && !to_fill.is_empty() {
            filled = false;
            let mut i = 0;
            while i < to_fill.len() {
                let pos = to_fill[i];
                let mut to_place = ctx.fluid_at(pos);
                for direction in FILL {
                    if is_source(to_place) {
                        break;
                    }
                    let neighbor_pos = pos.relative(direction, 1);
                    let neighbor = ctx.fluid_at(neighbor_pos);
                    if is_source(neighbor) && !locked.contains(&neighbor_pos) {
                        to_place = neighbor;
                    }
                }
                if is_source(to_place) {
                    let state = ctx.block(pos);
                    if is_liquid_container(ctx, state) {
                        place_liquid(ctx, pos, state, to_place);
                        filled = true;
                        to_fill.remove(i);
                        continue;
                    }
                }
                i += 1;
            }
        }
        if min.x <= max.x {
            if !settings.known_shape {
                let mut shape = VoxelShape::new(&Bounds { min, max });
                for (pos, _) in &placed {
                    shape.fill(pos.x - min.x, pos.y - min.y, pos.z - min.z);
                }
                shape_at_edge(ctx, &shape, flags);
                for (pos, _) in &placed {
                    let state = ctx.block(*pos);
                    let new_state = update_from_neighbour_shapes(ctx, state, *pos);
                    if new_state != state {
                        ctx.set_block_flags(*pos, new_state, (flags & !1) | 16);
                    }
                }
            }
        }
        if !settings.ignore_entities {
            self.place_entities(ctx, position, settings);
        }
        true
    }

    /// `StructureTemplate.placeEntities`: entities are recorded for the chunk.
    fn place_entities(&self, ctx: &mut Ctx, position: BlockPos, settings: &PlaceSettings) {
        for entity in &self.entities {
            let block_pos = transform::transform(entity.block_pos, settings.mirror, settings.rotation, settings.pivot);
            let block_pos = block_pos.offset(position.x, position.y, position.z);
            if settings.bounding_box.is_some_and(|b| !b.is_inside(block_pos)) {
                continue;
            }
            let [x, y, z] = transform::transform_vec(entity.pos, settings.mirror, settings.rotation, settings.pivot);
            let (x, y, z) = (x + f64::from(position.x), y + f64::from(position.y), z + f64::from(position.z));
            // `EntityType.create` is empty for hostile mobs on peaceful.
            let id = entity.nbt.get("id").and_then(Tag::as_str).unwrap_or("?");
            if !ctx.lib.can_spawn(id) {
                continue;
            }
            crate::feature::entities::place_template_entity(ctx, &entity.nbt, [x, y, z], block_pos, settings.rotation, settings.mirror, settings.finalize_entities);
        }
    }
}

fn is_source(fluid: FluidType) -> bool {
    matches!(fluid, FluidType::Water | FluidType::Lava)
}

/// `RandomizableContainer` block entities, which get a loot seed on placement.
fn is_randomizable_container(name: &str) -> bool {
    let name = name.trim_start_matches("minecraft:");
    (name.ends_with("chest") && name != "ender_chest")
        || name.ends_with("shulker_box")
        || matches!(name, "barrel" | "crafter" | "decorated_pot" | "dispenser" | "dropper" | "hopper")
}

/// `LiquidBlockContainer`: waterloggable blocks, kelp and seagrass.
fn is_liquid_container(ctx: &Ctx, state: BlockStateId) -> bool {
    ctx.property(state, "waterlogged").is_some()
        || matches!(ctx.name(state), "minecraft:kelp" | "minecraft:kelp_plant" | "minecraft:seagrass" | "minecraft:tall_seagrass")
}

/// `LiquidBlockContainer.placeLiquid`: only `SimpleWaterloggedBlock` takes
/// water, and only a water source.
fn place_liquid(ctx: &mut Ctx, pos: BlockPos, state: BlockStateId, fluid: FluidType) -> bool {
    if ctx.property(state, "waterlogged") == Some("false") && fluid == FluidType::Water {
        let wet = ctx.with(state, "waterlogged", "true");
        ctx.set_block_update(pos, wet);
        ctx.schedule_fluid_tick(pos);
        return true;
    }
    false
}

/// `Block.updateFromNeighbourShapes` in `UPDATE_SHAPE_ORDER`.
pub fn update_from_neighbour_shapes<W: crate::feature::World + ?Sized>(ctx: &mut Ctx<W>, state: BlockStateId, pos: BlockPos) -> BlockStateId {
    const ORDER: [Direction; 6] = [Direction::West, Direction::East, Direction::North, Direction::South, Direction::Down, Direction::Up];
    let mut new_state = state;
    for direction in ORDER {
        let neighbor = ctx.block(pos.relative(direction, 1));
        new_state = update_shape(ctx, new_state, pos, direction, neighbor);
    }
    new_state
}

/// Where `StructurePlaceSettings.getRandom` draws from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RandomMode {
    /// No random set: a fresh `LegacyRandom` seeded by `Mth.getSeed(pos)`.
    #[default]
    PerPosition,
    /// The random passed to the placement (features set their own random).
    Shared,
}

/// `StructurePlaceSettings`.
#[derive(Clone, Debug)]
pub struct PlaceSettings {
    pub mirror: Mirror,
    pub rotation: Rotation,
    pub pivot: BlockPos,
    pub ignore_entities: bool,
    pub bounding_box: Option<BoundingBox>,
    /// `LiquidSettings.APPLY_WATERLOGGING`.
    pub waterlogging: bool,
    pub random: RandomMode,
    pub processors: Vec<Processor>,
    pub known_shape: bool,
    pub finalize_entities: bool,
}

impl Default for PlaceSettings {
    fn default() -> Self {
        Self {
            mirror: Mirror::None,
            rotation: Rotation::None,
            pivot: BlockPos::new(0, 0, 0),
            ignore_entities: false,
            bounding_box: None,
            waterlogging: true,
            random: RandomMode::PerPosition,
            processors: Vec::new(),
            known_shape: false,
            finalize_entities: false,
        }
    }
}

impl PlaceSettings {
    /// `StructurePlaceSettings.getRandom(pos)`.
    pub fn random_at<'a, R: RandomSource>(&self, shared: &'a mut R, pos: BlockPos) -> SettingsRandom<'a, R> {
        match self.random {
            RandomMode::Shared => SettingsRandom::Shared(shared),
            RandomMode::PerPosition => SettingsRandom::Local(LegacyRandom::new(positional_seed(pos.x, pos.y, pos.z))),
        }
    }
}

/// The random `StructurePlaceSettings.getRandom` returns.
pub enum SettingsRandom<'a, R> {
    Shared(&'a mut R),
    Local(LegacyRandom),
}

macro_rules! delegate {
    ($self:ident, $r:ident => $e:expr) => {
        match $self {
            SettingsRandom::Shared($r) => $e,
            SettingsRandom::Local($r) => $e,
        }
    };
}

impl<R: RandomSource> RandomSource for SettingsRandom<'_, R> {
    fn next_i32(&mut self) -> i32 {
        delegate!(self, r => r.next_i32())
    }
    fn next_i32_bound(&mut self, bound: i32) -> i32 {
        delegate!(self, r => r.next_i32_bound(bound))
    }
    fn next_i64(&mut self) -> i64 {
        delegate!(self, r => r.next_i64())
    }
    fn next_f32(&mut self) -> f32 {
        delegate!(self, r => r.next_f32())
    }
    fn next_f64(&mut self) -> f64 {
        delegate!(self, r => r.next_f64())
    }
    fn next_bool(&mut self) -> bool {
        delegate!(self, r => r.next_bool())
    }
    fn next_gaussian(&mut self) -> f64 {
        delegate!(self, r => r.next_gaussian())
    }
}

/// `StructureTemplate.processBlockInfos`.
pub fn process_block_infos<R: RandomSource>(
    ctx: &Ctx,
    position: BlockPos,
    reference: BlockPos,
    settings: &PlaceSettings,
    blocks: &[BlockInfo],
    random: &mut R,
) -> Vec<BlockInfo> {
    let only_in_box = !settings.processors.iter().any(Processor::evaluates_entire_piece);
    let mut original = Vec::new();
    let mut processed = Vec::new();
    for info in blocks {
        let pos = transform::transform(info.pos, settings.mirror, settings.rotation, settings.pivot).offset(position.x, position.y, position.z);
        if only_in_box && settings.bounding_box.is_some_and(|b| !b.is_inside(pos)) {
            continue;
        }
        let mut current = Some(BlockInfo { pos, state: info.state, nbt: info.nbt.clone() });
        for processor in &settings.processors {
            let Some(block) = current else { break };
            current = processor.process(ctx, position, reference, info.pos, block, settings, random);
        }
        if let Some(block) = current {
            processed.push(block);
            original.push(info.clone());
        }
    }
    for processor in &settings.processors {
        processed = processor.finalize(ctx, position, reference, &original, processed, settings, random);
    }
    processed
}

/// Built-in templates, loaded on first use (`StructureTemplateManager`).
#[derive(Default)]
pub struct TemplateManager {
    cache: RwLock<HashMap<String, Arc<Template>>>,
}

impl std::fmt::Debug for TemplateManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TemplateManager")
    }
}

impl TemplateManager {
    /// `getOrCreate`: a missing template is an empty one.
    pub fn get(&self, registries: &Registries, name: &str) -> Arc<Template> {
        if let Some(t) = self.cache.read().expect("template cache").get(name) {
            return t.clone();
        }
        let template = Arc::new(Self::read(registries, name).unwrap_or_default());
        self.cache.write().expect("template cache").entry(name.to_owned()).or_insert(template).clone()
    }

    fn read(registries: &Registries, name: &str) -> Option<Template> {
        let id = Identifier::parse(name).ok()?;
        let path = registries.datapack.root().join("data").join(id.namespace()).join("structure").join(format!("{}.nbt", id.path()));
        let bytes = std::fs::read(path).ok()?;
        let tag = nbt::read(&bytes).ok()?;
        Some(Template::load(registries, &tag))
    }
}
