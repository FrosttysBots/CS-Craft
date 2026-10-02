//! Jigsaw structures (vanilla `JigsawStructure`, `JigsawPlacement`,
//! `StructureTemplatePool`, pool elements, pool aliases and
//! `PoolElementStructurePiece`).
//!
//! Source-informed from the pinned 26.3 JAR. The placer keeps vanilla's
//! random order (shuffled jigsaws, pools and rotations), its priority queue
//! (highest placement priority first, FIFO within one priority), its
//! free-space shapes (an outer box minus placed pieces, where a child placed
//! inside its parent's box gets the parent's box as its own free space) and
//! the village expansion hack.

use super::piece::{Piece, PieceBase};
use super::{GenerationContext, PieceList, Stub, TerrainAdjustment};
use crate::feature::template::{processor::Processor, BoundingBox, JigsawInfo, PlaceSettings, Rotation, Template};
use crate::feature::{Ctx, Library, PlacedId};
use crate::providers::{HeightProvider, Weighted};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{LegacyRandom, RandomSource, WorldgenRandom};
use minecraftoss_core::{BlockPos, ChunkPos};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Rigid,
    /// Adds `GravityProcessor(WORLD_SURFACE_WG, -1)`.
    TerrainMatching,
}

/// A `StructurePoolElement`.
#[derive(Debug)]
pub enum Element {
    Empty,
    Single {
        template: String,
        processors: Arc<Vec<Processor>>,
        projection: Projection,
        legacy: bool,
        waterlogging: Option<bool>,
        /// The loaded template, cached on first use.
        loaded: OnceLock<Arc<Template>>,
    },
    List { elements: Vec<Element>, projection: Projection },
    Feature { feature: PlacedId, projection: Projection },
}

impl Element {
    fn parse(lib: &mut Library, json: &Value, projection_override: Option<Projection>) -> Result<Self, String> {
        let kind = json["element_type"].as_str().ok_or("pool element lacks element_type")?;
        let projection = match projection_override {
            Some(p) => p,
            None => match json.get("projection").and_then(Value::as_str) {
                Some("terrain_matching") => Projection::TerrainMatching,
                _ => Projection::Rigid,
            },
        };
        Ok(match kind.trim_start_matches("minecraft:") {
            "empty_pool_element" => Self::Empty,
            k @ ("single_pool_element" | "legacy_single_pool_element") => Self::Single {
                template: json["location"].as_str().ok_or("single element lacks location")?.to_owned(),
                processors: lib.processor_list(json.get("processors").unwrap_or(&Value::Array(Vec::new())))?,
                projection,
                legacy: k.starts_with("legacy"),
                waterlogging: json.get("override_liquid_settings").and_then(Value::as_str).map(|s| s == "apply_waterlogging"),
                loaded: OnceLock::new(),
            },
            "list_pool_element" => Self::List {
                elements: json["elements"]
                    .as_array()
                    .ok_or("list element lacks elements")?
                    .iter()
                    .map(|e| Self::parse(lib, e, Some(projection)))
                    .collect::<Result<_, _>>()?,
                projection,
            },
            "feature_pool_element" => Self::Feature { feature: lib.placed_ref(&json["feature"])?, projection },
            other => return Err(format!("unknown pool element {other}")),
        })
    }

    pub fn projection(&self) -> Projection {
        match self {
            Self::Empty => Projection::Rigid,
            Self::Single { projection, .. } | Self::List { projection, .. } | Self::Feature { projection, .. } => *projection,
        }
    }

    /// `StructurePoolElement.getGroundLevelDelta`.
    pub fn ground_level_delta(&self) -> i32 {
        1
    }

    fn template(&self, lib: &Library) -> Option<Arc<Template>> {
        match self {
            Self::Single { template, loaded, .. } => Some(loaded.get_or_init(|| lib.templates.get(&lib.registries, template)).clone()),
            _ => None,
        }
    }

    /// `getBoundingBox` (not defined for the empty element).
    pub fn bounding_box(&self, lib: &Library, position: BlockPos, rotation: Rotation) -> BoundingBox {
        match self {
            Self::Empty => BoundingBox::at(position),
            Self::Single { .. } => {
                let template = self.template(lib).expect("single element");
                template.bounding_box(&PlaceSettings { rotation, ..PlaceSettings::default() }, position)
            }
            Self::List { elements, .. } => {
                let boxes: Vec<BoundingBox> = elements.iter().filter(|e| !matches!(e, Self::Empty)).map(|e| e.bounding_box(lib, position, rotation)).collect();
                BoundingBox::encapsulating_all(&boxes).unwrap_or_else(|| BoundingBox::at(position))
            }
            Self::Feature { .. } => BoundingBox::at(position),
        }
    }

    /// `getShuffledJigsawBlocks`.
    pub fn shuffled_jigsaws(&self, lib: &Library, position: BlockPos, rotation: Rotation, random: &mut impl RandomSource) -> Vec<JigsawInfo> {
        match self {
            Self::Empty => Vec::new(),
            Self::Single { .. } => {
                let template = self.template(lib).expect("single element");
                let mut jigsaws = template.jigsaws(&lib.registries, position, rotation);
                shuffle(&mut jigsaws, random);
                // Stable sort, highest selection priority first.
                jigsaws.sort_by(|a, b| b.selection_priority.cmp(&a.selection_priority));
                jigsaws
            }
            Self::List { elements, .. } => elements[0].shuffled_jigsaws(lib, position, rotation, random),
            Self::Feature { .. } => vec![JigsawInfo {
                pos: position,
                front: Direction::Down,
                top: Direction::South,
                rollable: true,
                name: None,
                pool: Arc::from("minecraft:empty"),
                target: Arc::from("minecraft:empty"),
                placement_priority: 0,
                selection_priority: 0,
            }],
        }
    }

    /// `StructurePoolElement.place`.
    #[allow(clippy::too_many_arguments)]
    fn place(&self, ctx: &mut Ctx, position: BlockPos, reference: BlockPos, rotation: Rotation, chunk_bb: &BoundingBox, random: &mut WorldgenRandom, waterlogging: bool) -> bool {
        match self {
            Self::Empty => true,
            Self::Single { processors, projection, legacy, waterlogging: over, .. } => {
                let lib = ctx.lib;
                let template = self.template(lib).expect("single element");
                let blocks = &lib.processor_blocks;
                let mut list = vec![if *legacy { Processor::BlockIgnore(blocks.structure_and_air()) } else { Processor::BlockIgnore(blocks.structure_block()) }];
                list.push(Processor::JigsawReplacement);
                list.extend(processors.iter().cloned());
                if *projection == Projection::TerrainMatching {
                    list.push(Processor::Gravity { heightmap: HeightmapKind::WorldSurfaceWg, offset: -1 });
                }
                // `BlockIgnoreProcessor.STRUCTURE_BLOCK` comes first, then jigsaw
                // replacement; the legacy element swaps the first for
                // `STRUCTURE_AND_AIR`, which moves it to the end.
                if *legacy {
                    let first = list.remove(0);
                    list.push(first);
                }
                let settings = PlaceSettings {
                    rotation,
                    bounding_box: Some(*chunk_bb),
                    known_shape: true,
                    ignore_entities: false,
                    finalize_entities: true,
                    waterlogging: over.unwrap_or(waterlogging),
                    processors: list,
                    ..PlaceSettings::default()
                };
                template.place_in_world(ctx, position, reference, &settings, random, 18)
            }
            Self::List { elements, .. } => {
                for element in elements {
                    if !element.place(ctx, position, reference, rotation, chunk_bb, random, waterlogging) {
                        return false;
                    }
                }
                true
            }
            Self::Feature { feature, .. } => crate::feature::place_placed(ctx, random, *feature, position),
        }
    }
}

/// `Util.shuffle`.
fn shuffle<T>(list: &mut [T], random: &mut impl RandomSource) {
    for i in (2..=list.len()).rev() {
        let j = random.next_i32_bound(i as i32) as usize;
        list.swap(i - 1, j);
    }
}

/// A `StructureTemplatePool`.
#[derive(Debug)]
pub struct Pool {
    pub name: String,
    /// Elements repeated by weight.
    templates: Vec<Arc<Element>>,
    fallback: String,
    max_size: OnceLock<i32>,
}

impl Pool {
    fn size(&self) -> usize {
        self.templates.len()
    }

    /// `getRandomTemplate`.
    fn random_template(&self, random: &mut impl RandomSource) -> Arc<Element> {
        if self.templates.is_empty() {
            return Arc::new(Element::Empty);
        }
        self.templates[random.next_i32_bound(self.templates.len() as i32) as usize].clone()
    }

    /// `getShuffledTemplates`.
    fn shuffled(&self, random: &mut impl RandomSource) -> Vec<Arc<Element>> {
        let mut list = self.templates.clone();
        shuffle(&mut list, random);
        list
    }

    /// `getMaxSize`: the tallest non-empty element.
    fn max_size(&self, lib: &Library) -> i32 {
        *self.max_size.get_or_init(|| {
            self.templates
                .iter()
                .filter(|t| !matches!(***t, Element::Empty))
                .map(|t| t.bounding_box(lib, BlockPos::new(0, 0, 0), Rotation::None).y_span())
                .max()
                .unwrap_or(0)
        })
    }
}

/// Every template pool of the data pack.
#[derive(Debug, Default)]
pub struct Pools {
    pools: Vec<Pool>,
    by_name: HashMap<String, usize>,
}

impl Pools {
    pub fn load(lib: &mut Library) -> Result<Self, String> {
        let mut out = Self::default();
        let registries = lib.registries.clone();
        for id in registries.datapack.list("worldgen/template_pool")? {
            let json = registries.datapack.read_json("worldgen/template_pool", &id)?;
            let mut templates = Vec::new();
            for entry in json["elements"].as_array().ok_or_else(|| format!("pool {id} lacks elements"))? {
                let element = Arc::new(Element::parse(lib, &entry["element"], None).map_err(|e| format!("pool {id}: {e}"))?);
                for _ in 0..entry["weight"].as_i64().unwrap_or(1) {
                    templates.push(element.clone());
                }
            }
            let fallback = json["fallback"].as_str().unwrap_or("minecraft:empty").to_owned();
            out.by_name.insert(id.to_string(), out.pools.len());
            out.pools.push(Pool { name: id.to_string(), templates, fallback, max_size: OnceLock::new() });
        }
        Ok(out)
    }

    fn get(&self, name: &str) -> Option<&Pool> {
        self.by_name.get(name).map(|&i| &self.pools[i])
    }
}

/// `JigsawJunction`.
#[derive(Clone, Copy, Debug)]
pub struct Junction {
    pub source_x: i32,
    pub source_ground_y: i32,
    pub source_z: i32,
    pub delta_y: i32,
    pub dest_projection: Projection,
}

/// `PoolElementStructurePiece`.
#[derive(Debug)]
pub struct PoolPiece {
    base: PieceBase,
    pub element: Arc<Element>,
    pub position: BlockPos,
    pub ground_level_delta: i32,
    pub rotation: Rotation,
    pub junctions: Vec<Junction>,
    waterlogging: bool,
}

impl Piece for PoolPiece {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:jigsaw"
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, reference: BlockPos) {
        self.element.place(ctx, self.position, reference, self.rotation, chunk_bb, random, self.waterlogging);
    }

    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox = self.base.bbox.moved(dx, dy, dz);
        self.position = self.position.offset(dx, dy, dz);
    }

    fn pool_piece(&self) -> Option<&PoolPiece> {
        Some(self)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `PoolAliasBinding`.
#[derive(Debug)]
enum AliasBinding {
    Direct { alias: String, target: String },
    Random { alias: String, targets: Weighted<String> },
    RandomGroup { groups: Weighted<Vec<AliasBinding>> },
}

impl AliasBinding {
    fn parse(json: &Value) -> Result<Self, String> {
        let kind = json["type"].as_str().ok_or("pool alias lacks type")?;
        let string = |v: &Value| v.as_str().map(str::to_owned).ok_or_else(|| format!("bad pool alias entry {v}"));
        Ok(match kind.trim_start_matches("minecraft:") {
            "direct" => Self::Direct { alias: string(&json["alias"])?, target: string(&json["target"])? },
            "random" => Self::Random { alias: string(&json["alias"])?, targets: Weighted::parse(&json["targets"], string)? },
            "random_group" => Self::RandomGroup {
                groups: Weighted::parse(&json["groups"], |g| g.as_array().ok_or("alias group is not a list")?.iter().map(Self::parse).collect())?,
            },
            other => return Err(format!("unknown pool alias {other}")),
        })
    }

    fn resolve(&self, random: &mut LegacyRandom, out: &mut HashMap<String, String>) {
        match self {
            Self::Direct { alias, target } => {
                out.insert(alias.clone(), target.clone());
            }
            Self::Random { alias, targets } => {
                if let Some(target) = targets.pick(random) {
                    out.insert(alias.clone(), target.clone());
                }
            }
            Self::RandomGroup { groups } => {
                if let Some(group) = groups.pick(random) {
                    for binding in group {
                        binding.resolve(random, out);
                    }
                }
            }
        }
    }
}

/// `JigsawStructure`.
#[derive(Debug)]
pub struct JigsawStructure {
    start_pool: String,
    start_jigsaw: Option<String>,
    max_depth: i32,
    start_height: HeightProvider,
    expansion_hack: bool,
    project_start: Option<HeightmapKind>,
    max_distance: (i32, i32),
    aliases: Vec<AliasBinding>,
    padding: (i32, i32),
    waterlogging: bool,
}

impl JigsawStructure {
    pub fn parse(json: &Value) -> Result<Self, String> {
        let max_distance = match &json["max_distance_from_center"] {
            Value::Number(n) => (n.as_i64().unwrap_or(80) as i32, n.as_i64().unwrap_or(80) as i32),
            Value::Object(o) => (
                o.get("horizontal").and_then(Value::as_i64).unwrap_or(80) as i32,
                o.get("vertical").and_then(Value::as_i64).unwrap_or(80) as i32,
            ),
            _ => (80, 80),
        };
        let padding = match &json["dimension_padding"] {
            Value::Number(n) => (n.as_i64().unwrap_or(0) as i32, n.as_i64().unwrap_or(0) as i32),
            Value::Object(o) => (o.get("bottom").and_then(Value::as_i64).unwrap_or(0) as i32, o.get("top").and_then(Value::as_i64).unwrap_or(0) as i32),
            _ => (0, 0),
        };
        Ok(Self {
            start_pool: json["start_pool"].as_str().ok_or("jigsaw lacks start_pool")?.to_owned(),
            start_jigsaw: json.get("start_jigsaw_name").and_then(Value::as_str).map(str::to_owned),
            max_depth: json["size"].as_i64().ok_or("jigsaw lacks size")? as i32,
            start_height: HeightProvider::parse(&json["start_height"])?,
            expansion_hack: json.get("use_expansion_hack").and_then(Value::as_bool).unwrap_or(false),
            project_start: match json.get("project_start_to_heightmap") {
                Some(h) => Some(crate::feature::placement::parse_heightmap(h)?),
                None => None,
            },
            max_distance,
            aliases: match json.get("pool_aliases").and_then(Value::as_array) {
                Some(list) => list.iter().map(AliasBinding::parse).collect::<Result<_, _>>()?,
                None => Vec::new(),
            },
            padding,
            waterlogging: json.get("liquid_settings").and_then(Value::as_str) != Some("ignore_waterlogging"),
        })
    }
}

impl super::StructureKind for JigsawStructure {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let height = self.start_height.sample(&mut ctx.random, &ctx.lib.generation);
        let start = BlockPos::new(ctx.chunk.min_block_x(), height, ctx.chunk.min_block_z());
        let aliases = if self.aliases.is_empty() {
            HashMap::new()
        } else {
            let mut seed = LegacyRandom::new(ctx.seed);
            let mut random = seed.fork_positional().at(start.x, start.y, start.z);
            let mut map = HashMap::new();
            for binding in &self.aliases {
                binding.resolve(&mut random, &mut map);
            }
            map
        };
        add_pieces(ctx, self, start, aliases)
    }
}

/// `SequencedPriorityIterator`: highest priority first, FIFO within one priority.
struct PriorityQueue<T> {
    queues: BTreeMap<i32, VecDeque<T>>,
}

impl<T> Default for PriorityQueue<T> {
    fn default() -> Self {
        Self { queues: BTreeMap::new() }
    }
}

impl<T> PriorityQueue<T> {
    fn push(&mut self, item: T, priority: i32) {
        self.queues.entry(priority).or_default().push_back(item);
    }

    fn pop(&mut self) -> Option<T> {
        let (&priority, queue) = self.queues.iter_mut().next_back()?;
        let item = queue.pop_front();
        if queue.is_empty() {
            self.queues.remove(&priority);
        }
        item
    }
}

/// A free-space `VoxelShape`: an outer box minus placed boxes (min
/// inclusive, max exclusive, as `AABB.of` makes them).
#[derive(Clone, Debug)]
struct FreeShape {
    outer: [i32; 6],
    holes: Vec<[i32; 6]>,
}

fn aabb(b: &BoundingBox) -> [i32; 6] {
    [b.min_x, b.min_y, b.min_z, b.max_x + 1, b.max_y + 1, b.max_z + 1]
}

impl FreeShape {
    /// `!joinIsNotEmpty(free, target.deflate(0.25), ONLY_SECOND)`: the
    /// target lies inside the outer box and overlaps no hole.
    fn fits(&self, target: &BoundingBox) -> bool {
        let t = aabb(target);
        let inside = (0..3).all(|i| t[i] >= self.outer[i] && t[i + 3] <= self.outer[i + 3]);
        inside && !self.holes.iter().any(|h| (0..3).all(|i| t[i] < h[i + 3] && t[i + 3] > h[i]))
    }
}

struct PieceState {
    piece: usize,
    free: usize,
    depth: i32,
}

/// `JigsawPlacement.addPieces` for a structure start.
fn add_pieces<'s>(ctx: &mut GenerationContext, structure: &'s JigsawStructure, position: BlockPos, aliases: HashMap<String, String>) -> Option<Stub<'s>> {
    let lib = ctx.lib;
    let rotation = Rotation::random(&mut ctx.random);
    let pools = ctx.pools;
    let center_pool = pools.get(&alias(&aliases, &structure.start_pool))?;
    let center_element = center_pool.random_template(&mut ctx.random);
    if matches!(*center_element, Element::Empty) {
        return None;
    }
    let anchored = match &structure.start_jigsaw {
        Some(name) => {
            let jigsaws = center_element.shuffled_jigsaws(lib, position, rotation, &mut ctx.random);
            jigsaws.iter().find(|j| j.name.as_deref() == Some(name.as_str()))?.pos
        }
        None => position,
    };
    let local_anchor = (anchored.x - position.x, anchored.y - position.y, anchored.z - position.z);
    let adjusted = position.offset(-local_anchor.0, -local_anchor.1, -local_anchor.2);
    let bbox = center_element.bounding_box(lib, adjusted, rotation);
    let mut center = PoolPiece {
        base: PieceBase::new(0, bbox),
        element: center_element.clone(),
        position: adjusted,
        ground_level_delta: center_element.ground_level_delta(),
        rotation,
        junctions: Vec::new(),
        waterlogging: structure.waterlogging,
    };
    let center_x = (bbox.max_x + bbox.min_x) / 2;
    let center_z = (bbox.max_z + bbox.min_z) / 2;
    let bottom_y = match structure.project_start {
        None => adjusted.y,
        Some(kind) => {
            if !ctx.could_structure_exist_in_column(center_x, center_z, ctx.min_y(), ctx.max_y()) {
                return None;
            }
            position.y + ctx.first_free_height(center_x, center_z, kind)
        }
    };
    let old_ground = bbox.min_y + center.ground_level_delta;
    center.move_by(0, bottom_y - old_ground, 0);
    let moved = center.base.bbox;
    let (pad_bottom, pad_top) = structure.padding;
    if (pad_bottom, pad_top) != (0, 0) && (moved.min_y < ctx.min_y() + pad_bottom || moved.max_y > ctx.max_y() - pad_top) {
        return None;
    }
    let center_y = bottom_y + local_anchor.1;
    Some(Stub::deferred(BlockPos::new(center_x, center_y, center_z), move |ctx: &mut GenerationContext| {
        let mut pieces = vec![center];
        if structure.max_depth > 0 {
            let (h, v) = structure.max_distance;
            let outer = [
                center_x - h,
                (center_y - v).max(ctx.min_y() + pad_bottom),
                center_z - h,
                center_x + h + 1,
                (center_y + v + 1).min(ctx.max_y() + 1 - pad_top),
                center_z + h + 1,
            ];
            let free = FreeShape { outer, holes: vec![aabb(&moved)] };
            let mut placer = Placer { structure, aliases: &aliases, pieces, shapes: vec![free], queue: PriorityQueue::default() };
            placer.try_placing_children(ctx, 0, 0, 0);
            while let Some(state) = placer.queue.pop() {
                placer.try_placing_children(ctx, state.piece, state.free, state.depth);
            }
            pieces = placer.pieces;
        }
        pieces.into_iter().map(|p| Box::new(p) as Box<dyn Piece>).collect::<PieceList>()
    }))
}

/// `PoolAliasLookup.lookup`.
fn alias(aliases: &HashMap<String, String>, name: &str) -> String {
    aliases.get(name).cloned().unwrap_or_else(|| name.to_owned())
}

struct Placer<'a> {
    structure: &'a JigsawStructure,
    aliases: &'a HashMap<String, String>,
    pieces: Vec<PoolPiece>,
    shapes: Vec<FreeShape>,
    queue: PriorityQueue<PieceState>,
}

impl Placer<'_> {
    /// `JigsawPlacement.Placer.tryPlacingChildren`.
    fn try_placing_children(&mut self, ctx: &mut GenerationContext, source_index: usize, context_free: usize, depth: i32) {
        let lib = ctx.lib;
        let pools = ctx.pools;
        let source_element = self.pieces[source_index].element.clone();
        let source_position = self.pieces[source_index].position;
        let source_rotation = self.pieces[source_index].rotation;
        let source_rigid = source_element.projection() == Projection::Rigid;
        let mut source_free: Option<usize> = None;
        let source_bb = self.pieces[source_index].base.bbox;
        let source_box_y = source_bb.min_y;
        let jigsaws = source_element.shuffled_jigsaws(lib, source_position, source_rotation, &mut ctx.random);
        'jigsaws: for source_jigsaw in &jigsaws {
            let source_direction = source_jigsaw.front;
            let source_pos = source_jigsaw.pos;
            let target_pos = source_pos.relative(source_direction, 1);
            let source_local_y = source_pos.y - source_box_y;
            let mut source_base_height = i32::MIN;
            let Some(target_pool) = pools.get(&alias(self.aliases, &source_jigsaw.pool)) else { continue };
            if target_pool.size() == 0 && target_pool.name != "minecraft:empty" {
                continue;
            }
            let Some(fallback) = pools.get(&target_pool.fallback) else { continue };
            if fallback.size() == 0 && fallback.name != "minecraft:empty" {
                continue;
            }
            let children_free = if source_bb.is_inside(target_pos) {
                *source_free.get_or_insert_with(|| {
                    self.shapes.push(FreeShape { outer: aabb(&source_bb), holes: Vec::new() });
                    self.shapes.len() - 1
                })
            } else {
                context_free
            };
            let mut targets: Vec<Arc<Element>> = Vec::new();
            if depth != self.structure.max_depth {
                targets.extend(target_pool.shuffled(&mut ctx.random));
            }
            targets.extend(fallback.shuffled(&mut ctx.random));
            let placement_priority = source_jigsaw.placement_priority;
            for target_element in targets {
                if matches!(*target_element, Element::Empty) {
                    break;
                }
                let mut rotations = Rotation::ALL;
                shuffle(&mut rotations, &mut ctx.random);
                for target_rotation in rotations {
                    let target_jigsaws = target_element.shuffled_jigsaws(lib, BlockPos::new(0, 0, 0), target_rotation, &mut ctx.random);
                    let hack_box = target_element.bounding_box(lib, BlockPos::new(0, 0, 0), target_rotation);
                    let expand_to = if self.structure.expansion_hack && hack_box.y_span() <= 16 {
                        target_jigsaws
                            .iter()
                            .map(|tj| {
                                if !hack_box.is_inside(tj.pos.relative(tj.front, 1)) {
                                    return 0;
                                }
                                let child = pools.get(&alias(self.aliases, &tj.pool));
                                let child_size = child.map_or(0, |p| p.max_size(lib));
                                let fallback_size = child.and_then(|p| pools.get(&p.fallback)).map_or(0, |p| p.max_size(lib));
                                child_size.max(fallback_size)
                            })
                            .max()
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    for target_jigsaw in &target_jigsaws {
                        if !source_jigsaw.can_attach(target_jigsaw) {
                            continue;
                        }
                        let target_local = target_jigsaw.pos;
                        let raw_box_pos = target_pos.offset(-target_local.x, -target_local.y, -target_local.z);
                        let raw_bb = target_element.bounding_box(lib, raw_box_pos, target_rotation);
                        let raw_y = raw_bb.min_y;
                        let target_rigid = target_element.projection() == Projection::Rigid;
                        let target_local_y = target_local.y;
                        let delta_y = source_local_y - target_local_y + source_direction.offset().1;
                        let target_box_y = if source_rigid && target_rigid {
                            source_box_y + delta_y
                        } else {
                            if source_base_height == i32::MIN {
                                source_base_height = ctx.first_free_height(source_pos.x, source_pos.z, HeightmapKind::WorldSurfaceWg);
                            }
                            source_base_height - target_local_y
                        };
                        let y_offset = target_box_y - raw_y;
                        let mut target_bb = raw_bb.moved(0, y_offset, 0);
                        let target_box_pos = raw_box_pos.offset(0, y_offset, 0);
                        if expand_to > 0 {
                            let new_size = (expand_to + 1).max(target_bb.max_y - target_bb.min_y);
                            target_bb.encapsulate_pos(BlockPos::new(target_bb.min_x, target_bb.min_y + new_size, target_bb.min_z));
                        }
                        if !self.shapes[children_free].fits(&target_bb) {
                            continue;
                        }
                        self.shapes[children_free].holes.push(aabb(&target_bb));
                        let source_ground_delta = self.pieces[source_index].ground_level_delta;
                        let target_ground_delta = if target_rigid { source_ground_delta - delta_y } else { target_element.ground_level_delta() };
                        let mut target_piece = PoolPiece {
                            base: PieceBase::new(0, target_bb),
                            element: target_element.clone(),
                            position: target_box_pos,
                            ground_level_delta: target_ground_delta,
                            rotation: target_rotation,
                            junctions: Vec::new(),
                            waterlogging: self.structure.waterlogging,
                        };
                        let junction_y = if source_rigid {
                            source_box_y + source_local_y
                        } else if target_rigid {
                            target_box_y + target_local_y
                        } else {
                            if source_base_height == i32::MIN {
                                source_base_height = ctx.first_free_height(source_pos.x, source_pos.z, HeightmapKind::WorldSurfaceWg);
                            }
                            source_base_height + delta_y / 2
                        };
                        self.pieces[source_index].junctions.push(Junction {
                            source_x: target_pos.x,
                            source_ground_y: junction_y - source_local_y + source_ground_delta,
                            source_z: target_pos.z,
                            delta_y,
                            dest_projection: target_element.projection(),
                        });
                        target_piece.junctions.push(Junction {
                            source_x: source_pos.x,
                            source_ground_y: junction_y - target_local_y + target_ground_delta,
                            source_z: source_pos.z,
                            delta_y: -delta_y,
                            dest_projection: source_element.projection(),
                        });
                        self.pieces.push(target_piece);
                        if depth < self.structure.max_depth {
                            self.queue.push(PieceState { piece: self.pieces.len() - 1, free: children_free, depth: depth + 1 }, placement_priority);
                        }
                        continue 'jigsaws;
                    }
                }
            }
        }
    }
}

/// Terrain adaptation inputs from one jigsaw piece.
pub fn is_rigid(piece: &PoolPiece) -> bool {
    piece.element.projection() == Projection::Rigid
}

pub fn adaptation_uses_beard(adaptation: TerrainAdjustment) -> bool {
    adaptation != TerrainAdjustment::None
}
