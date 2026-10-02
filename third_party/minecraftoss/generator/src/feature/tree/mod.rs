//! `TreeFeature` and `FallenTreeFeature` with their placers and decorators.

pub mod decorator;
pub mod foliage;
pub mod trunk;

use super::blocks::{BlockSet, FluidType};
use super::java_set::JavaHashSet;
use super::kinds::{bool_or, float, int};
use super::state::{try_with, StateProvider};
use super::{update, Ctx, Library};
use crate::providers::IntProvider;
use decorator::{Context, Decorator};
use foliage::FoliagePlacer;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::{BlockPos, BlockStateId};
use serde_json::Value;
use std::sync::Arc;
use trunk::TrunkPlacer;

/// `FoliagePlacer.FoliageAttachment`.
#[derive(Clone, Copy, Debug)]
pub struct Attachment {
    pub pos: BlockPos,
    pub radius_offset: i32,
    pub foliage_height_offset: i32,
    pub size_x: i32,
    pub size_z: i32,
}

impl Attachment {
    pub fn new(pos: BlockPos, radius_offset: i32, double_trunk: bool) -> Self {
        let size = if double_trunk { 2 } else { 1 };
        Self { pos, radius_offset, foliage_height_offset: 0, size_x: size, size_z: size }
    }

    pub fn double_trunk(&self) -> bool {
        self.size_x == 2 && self.size_z == 2
    }
}

/// The position sets `TreeFeature.place` collects.
#[derive(Default)]
pub struct Sets {
    pub roots: JavaHashSet,
    pub trunks: JavaHashSet,
    pub foliage: JavaHashSet,
    pub decorations: JavaHashSet,
}

impl Sets {
    pub fn set_root(&mut self, ctx: &mut Ctx, pos: BlockPos, state: BlockStateId) {
        self.roots.insert(pos);
        ctx.set_block_flags(pos, state, 19);
    }

    pub fn set_trunk(&mut self, ctx: &mut Ctx, pos: BlockPos, state: BlockStateId) {
        self.trunks.insert(pos);
        ctx.set_block_flags(pos, state, 19);
    }

    pub fn set_foliage(&mut self, ctx: &mut Ctx, pos: BlockPos, state: BlockStateId) {
        self.foliage.insert(pos);
        ctx.set_block_flags(pos, state, 19);
    }
}

/// `TreeFeature.validTreePos`.
pub fn valid_tree_pos(ctx: &Ctx, pos: BlockPos) -> bool {
    let state = ctx.block(pos);
    ctx.is_air(state) || ctx.in_tag(state, ctx.lib.tags.replaceable_by_trees)
}

/// `TreeFeature.isAirOrLeaves`.
pub fn is_air_or_leaves(ctx: &Ctx, pos: BlockPos) -> bool {
    let state = ctx.block(pos);
    ctx.is_air(state) || ctx.in_tag(state, ctx.lib.tags.leaves)
}

#[derive(Debug)]
enum FeatureSize {
    Two { limit: i32, lower: i32, upper: i32, min_clipped: Option<i32> },
    Three { limit: i32, upper_limit: i32, lower: i32, middle: i32, upper: i32, min_clipped: Option<i32> },
}

impl FeatureSize {
    fn parse(json: &Value) -> Result<Self, String> {
        let get = |k: &str, d: i32| json.get(k).and_then(Value::as_i64).map_or(d, |v| v as i32);
        let min_clipped = json.get("min_clipped_height").and_then(Value::as_i64).map(|v| v as i32);
        Ok(match json["type"].as_str().unwrap_or_default().trim_start_matches("minecraft:") {
            "two_layers_feature_size" => Self::Two { limit: get("limit", 1), lower: get("lower_size", 0), upper: get("upper_size", 1), min_clipped },
            "three_layers_feature_size" => Self::Three {
                limit: get("limit", 1),
                upper_limit: get("upper_limit", 1),
                lower: get("lower_size", 0),
                middle: get("middle_size", 1),
                upper: get("upper_size", 1),
                min_clipped,
            },
            other => return Err(format!("unknown feature size {other}")),
        })
    }

    fn size_at(&self, tree_height: i32, yo: i32) -> i32 {
        match *self {
            Self::Two { limit, lower, upper, .. } => {
                if yo < limit {
                    lower
                } else {
                    upper
                }
            }
            Self::Three { limit, upper_limit, lower, middle, upper, .. } => {
                if yo < limit {
                    lower
                } else if yo >= tree_height - upper_limit {
                    upper
                } else {
                    middle
                }
            }
        }
    }

    fn min_clipped(&self) -> Option<i32> {
        match *self {
            Self::Two { min_clipped, .. } | Self::Three { min_clipped, .. } => min_clipped,
        }
    }
}

/// `MangroveRootPlacer` (the only root placer type).
#[derive(Debug)]
struct RootPlacer {
    trunk_offset_y: IntProvider,
    root_provider: Arc<StateProvider>,
    above: Option<(Arc<StateProvider>, f32)>,
    can_grow_through: BlockSet,
    muddy_roots_in: BlockSet,
    muddy_roots_provider: Arc<StateProvider>,
    max_root_width: i32,
    max_root_length: i32,
    random_skew_chance: f32,
}

impl RootPlacer {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let kind = json["type"].as_str().unwrap_or_default();
        if kind.trim_start_matches("minecraft:") != "mangrove_root_placer" {
            return Err(format!("unknown root placer {kind}"));
        }
        let placement = &json["mangrove_root_placement"];
        let above = match json.get("above_root_placement") {
            Some(a) => Some((StateProvider::parse(lib, &a["above_root_provider"])?, float(a, "above_root_placement_chance")?)),
            None => None,
        };
        Ok(Self {
            trunk_offset_y: IntProvider::parse(&json["trunk_offset_y"])?,
            root_provider: StateProvider::parse(lib, &json["root_provider"])?,
            above,
            can_grow_through: BlockSet::parse(&lib.registries, &placement["can_grow_through"])?,
            muddy_roots_in: BlockSet::parse(&lib.registries, &placement["muddy_roots_in"])?,
            muddy_roots_provider: StateProvider::parse(lib, &placement["muddy_roots_provider"])?,
            max_root_width: int(placement, "max_root_width")?,
            max_root_length: int(placement, "max_root_length")?,
            random_skew_chance: float(placement, "random_skew_chance")?,
        })
    }

    fn can_place(&self, ctx: &Ctx, pos: BlockPos) -> bool {
        valid_tree_pos(ctx, pos) || self.can_grow_through.contains(ctx.registries(), ctx.block(pos))
    }

    fn waterlogged(ctx: &Ctx, pos: BlockPos, state: BlockStateId) -> BlockStateId {
        if ctx.property(state, "waterlogged").is_none() {
            return state;
        }
        let water = matches!(ctx.fluid_at(pos), FluidType::Water | FluidType::FlowingWater);
        ctx.with(state, "waterlogged", if water { "true" } else { "false" })
    }

    fn place_roots(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, origin: BlockPos, trunk_origin: BlockPos) -> bool {
        let mut column = origin;
        while column.y < trunk_origin.y {
            if !self.can_place(ctx, column) {
                return false;
            }
            column = column.above();
        }
        let mut positions = vec![trunk_origin.below()];
        for dir in Direction::HORIZONTAL {
            let pos = trunk_origin.relative(dir, 1);
            let mut in_direction = Vec::new();
            if !self.simulate(ctx, random, pos, dir, trunk_origin, &mut in_direction, 0) {
                return false;
            }
            positions.extend(in_direction);
            positions.push(trunk_origin.relative(dir, 1));
        }
        for pos in positions {
            self.place_root(ctx, random, sets, pos);
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn simulate(&self, ctx: &Ctx, random: &mut WorldgenRandom, root: BlockPos, dir: Direction, origin: BlockPos, out: &mut Vec<BlockPos>, layer: i32) -> bool {
        if layer == self.max_root_length || out.len() as i32 > self.max_root_length {
            return false;
        }
        for pos in self.potential(root, dir, random, origin) {
            if self.can_place(ctx, pos) {
                out.push(pos);
                if !self.simulate(ctx, random, pos, dir, origin, out, layer + 1) {
                    return false;
                }
            }
        }
        true
    }

    fn potential(&self, pos: BlockPos, prev: Direction, random: &mut WorldgenRandom, origin: BlockPos) -> Vec<BlockPos> {
        let below = pos.below();
        let next_to = pos.relative(prev, 1);
        let width = pos.dist_manhattan(origin);
        let max = self.max_root_width;
        if width > max - 3 && width <= max {
            if random.next_f32() < self.random_skew_chance {
                vec![below, next_to.below()]
            } else {
                vec![below]
            }
        } else if width > max {
            vec![below]
        } else if random.next_f32() < self.random_skew_chance {
            vec![below]
        } else if random.next_bool() {
            vec![next_to]
        } else {
            vec![below]
        }
    }

    fn place_root(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, pos: BlockPos) {
        if self.muddy_roots_in.contains(ctx.registries(), ctx.block(pos)) {
            let state = self.muddy_roots_provider.get(ctx, random, pos);
            let state = Self::waterlogged(ctx, pos, state);
            sets.set_root(ctx, pos, state);
            return;
        }
        if !self.can_place(ctx, pos) {
            return;
        }
        let state = self.root_provider.get(ctx, random, pos);
        let state = Self::waterlogged(ctx, pos, state);
        sets.set_root(ctx, pos, state);
        if let Some((provider, chance)) = &self.above {
            let above = pos.above();
            if random.next_f32() < *chance && ctx.is_empty_block(above) {
                let state = provider.get(ctx, random, above);
                let state = Self::waterlogged(ctx, above, state);
                sets.set_root(ctx, above, state);
            }
        }
    }
}

#[derive(Debug)]
pub struct Tree {
    pub trunk_provider: Arc<StateProvider>,
    trunk: TrunkPlacer,
    pub foliage_provider: Arc<StateProvider>,
    foliage: FoliagePlacer,
    root: Option<RootPlacer>,
    minimum_size: FeatureSize,
    decorators: Vec<Decorator>,
    ignore_vines: bool,
    pub below_trunk_provider: Arc<StateProvider>,
}

impl Tree {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let mut decorators = Vec::new();
        for d in json["decorators"].as_array().ok_or("tree lacks decorators")? {
            decorators.push(Decorator::parse(lib, d)?);
        }
        Ok(Self {
            trunk_provider: StateProvider::parse(lib, &json["trunk_provider"])?,
            trunk: TrunkPlacer::parse(&lib.registries, &json["trunk_placer"])?,
            foliage_provider: StateProvider::parse(lib, &json["foliage_provider"])?,
            foliage: FoliagePlacer::parse(&json["foliage_placer"])?,
            root: json.get("root_placer").map(|r| RootPlacer::parse(lib, r)).transpose()?,
            minimum_size: FeatureSize::parse(&json["minimum_size"])?,
            decorators,
            ignore_vines: bool_or(json, "ignore_vines", false),
            below_trunk_provider: StateProvider::parse(lib, &json["below_trunk_provider"])?,
        })
    }

    fn max_free_height(&self, ctx: &Ctx, max_height: i32, pos: BlockPos) -> i32 {
        for y in 0..=max_height + 1 {
            let r = self.minimum_size.size_at(max_height, y);
            for x in -r..=r {
                for z in -r..=r {
                    let p = pos.offset(x, y, z);
                    if !self.trunk.is_free(ctx, p) || !self.ignore_vines && ctx.is(ctx.block(p), "minecraft:vine") {
                        return y - 2;
                    }
                }
            }
        }
        max_height
    }

    fn do_place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, origin: BlockPos) -> bool {
        let tree_height = self.trunk.tree_height(random);
        let foliage_height = self.foliage.foliage_height(random, tree_height);
        let trunk_height = tree_height - foliage_height;
        let leaf_radius = self.foliage.foliage_radius(random, trunk_height);
        let trunk_origin = match &self.root {
            Some(root) => origin.offset(0, root.trunk_offset_y.sample(random), 0),
            None => origin,
        };
        let min_y = origin.y.min(trunk_origin.y);
        let max_y = origin.y.max(trunk_origin.y) + tree_height + 1;
        if min_y < ctx.min_y() + 1 || max_y > ctx.max_y() + 1 {
            return false;
        }
        let clipped = self.max_free_height(ctx, tree_height, trunk_origin);
        if !(clipped >= tree_height || self.minimum_size.min_clipped().is_some_and(|m| clipped >= m)) {
            return false;
        }
        if let Some(root) = &self.root {
            if !root.place_roots(ctx, random, sets, origin, trunk_origin) {
                return false;
            }
        }
        let attachments = self.trunk.place(ctx, random, sets, self, clipped, trunk_origin);
        for attachment in attachments {
            self.foliage.create(ctx, random, sets, self, attachment, foliage_height, leaf_radius);
        }
        true
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut sets = Sets::default();
        let result = self.do_place(ctx, random, &mut sets, origin);
        if !result || sets.trunks.is_empty() && sets.foliage.is_empty() {
            return false;
        }
        if !self.decorators.is_empty() {
            let Sets { roots, trunks, foliage, decorations } = &mut sets;
            let mut context = Context::new(trunks, foliage, roots, decorations, true);
            for decorator in &self.decorators {
                decorator.place(ctx, random, &mut context);
            }
        }
        let all: Vec<BlockPos> = sets.roots.iter().chain(sets.trunks.iter()).chain(sets.foliage.iter()).chain(sets.decorations.iter()).collect();
        let Some(bounds) = Bounds::encapsulating(&all) else {
            return false;
        };
        let shape = update_leaves(ctx, &bounds, &sets.trunks, &sets.decorations, &sets.roots);
        update::shape_at_edge(ctx, &shape, 3);
        true
    }
}

/// A `BoundingBox`.
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub min: BlockPos,
    pub max: BlockPos,
}

impl Bounds {
    pub fn encapsulating(positions: &[BlockPos]) -> Option<Self> {
        let first = *positions.first()?;
        let (mut min, mut max) = (first, first);
        for p in positions {
            min = BlockPos::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
            max = BlockPos::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
        }
        Some(Self { min, max })
    }

    pub fn contains(&self, p: BlockPos) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y && p.z >= self.min.z && p.z <= self.max.z
    }

    pub fn span(&self) -> (i32, i32, i32) {
        (self.max.x - self.min.x + 1, self.max.y - self.min.y + 1, self.max.z - self.min.z + 1)
    }
}

/// A `BitSetDiscreteVoxelShape` anchored at a bounding box corner.
pub struct VoxelShape {
    pub origin: BlockPos,
    pub size: (i32, i32, i32),
    bits: Vec<bool>,
}

impl VoxelShape {
    pub fn new(bounds: &Bounds) -> Self {
        let size = bounds.span();
        Self { origin: bounds.min, size, bits: vec![false; (size.0 * size.1 * size.2) as usize] }
    }

    fn index(&self, x: i32, y: i32, z: i32) -> usize {
        ((x * self.size.1 + y) * self.size.2 + z) as usize
    }

    pub fn fill(&mut self, x: i32, y: i32, z: i32) {
        let i = self.index(x, y, z);
        self.bits[i] = true;
    }

    pub fn is_full(&self, x: i32, y: i32, z: i32) -> bool {
        x >= 0 && y >= 0 && z >= 0 && x < self.size.0 && y < self.size.1 && z < self.size.2 && self.bits[self.index(x, y, z)]
    }
}

/// `LeavesBlock.getOptionalDistanceAt`.
fn leaf_distance(ctx: &Ctx, state: BlockStateId) -> Option<i32> {
    if ctx.in_tag(state, ctx.lib.tags.logs) {
        return Some(0);
    }
    ctx.property(state, "distance").and_then(|d| d.parse().ok())
}

/// `TreeFeature.updateLeaves`.
fn update_leaves(ctx: &mut Ctx, bounds: &Bounds, logs: &JavaHashSet, decorations: &JavaHashSet, roots: &JavaHashSet) -> VoxelShape {
    let mut shape = VoxelShape::new(bounds);
    let o = bounds.min;
    for pos in decorations.iter().chain(roots.iter()) {
        if bounds.contains(pos) {
            shape.fill(pos.x - o.x, pos.y - o.y, pos.z - o.z);
        }
    }
    let mut to_check: Vec<JavaHashSet> = (0..7).map(|_| JavaHashSet::new()).collect();
    for pos in logs.iter() {
        to_check[0].insert(pos);
    }
    let mut smallest = 0usize;
    loop {
        while smallest >= 7 || !to_check[smallest].is_empty() {
            if smallest >= 7 {
                return shape;
            }
            let pos = to_check[smallest].pop_first().expect("non-empty");
            if !bounds.contains(pos) {
                continue;
            }
            if smallest != 0 {
                let state = ctx.block(pos);
                let state = try_with(ctx.registries(), state, "distance", &smallest.to_string());
                ctx.set_block_flags(pos, state, 19);
            }
            shape.fill(pos.x - o.x, pos.y - o.y, pos.z - o.z);
            for direction in Direction::ALL {
                let n = pos.relative(direction, 1);
                if !bounds.contains(n) || shape.is_full(n.x - o.x, n.y - o.y, n.z - o.z) {
                    continue;
                }
                if let Some(distance) = leaf_distance(ctx, ctx.block(n)) {
                    let new = distance.min(smallest as i32 + 1);
                    if new < 7 {
                        to_check[new as usize].insert(n);
                        smallest = smallest.min(new as usize);
                    }
                }
            }
        }
        smallest += 1;
    }
}

/// `FallenTreeFeature`.
#[derive(Debug)]
pub struct FallenTree {
    trunk_provider: Arc<StateProvider>,
    log_length: IntProvider,
    stump_decorators: Vec<Decorator>,
    log_decorators: Vec<Decorator>,
}

impl FallenTree {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let mut list = |key: &str| -> Result<Vec<Decorator>, String> {
            json[key].as_array().ok_or_else(|| format!("fallen_tree lacks {key}"))?.iter().map(|d| Decorator::parse(lib, d)).collect()
        };
        let stump_decorators = list("stump_decorators")?;
        let log_decorators = list("log_decorators")?;
        Ok(Self {
            trunk_provider: StateProvider::parse(lib, &json["trunk_provider"])?,
            log_length: IntProvider::parse(&json["log_length"])?,
            stump_decorators,
            log_decorators,
        })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let stump = self.place_log(ctx, random, origin, None);
        let mut stump_set = JavaHashSet::new();
        stump_set.insert(stump);
        self.decorate(ctx, random, &stump_set, &self.stump_decorators);
        let direction = Direction::HORIZONTAL[random.next_i32_bound(4) as usize];
        let length = self.log_length.sample(random) - 2;
        let mut start = origin.relative(direction, 2 + random.next_i32_bound(2));
        // setGroundHeightForFallenLogStartPos
        start = start.above();
        for _ in 0..6 {
            if valid_tree_pos(ctx, start) && self.over_solid(ctx, start) {
                break;
            }
            start = start.below();
        }
        if self.can_place_all(ctx, length, start, direction) {
            let mut logs = JavaHashSet::new();
            let mut pos = start;
            for _ in 0..length {
                logs.insert(self.place_log(ctx, random, pos, Some(direction)));
                pos = pos.relative(direction, 1);
            }
            self.decorate(ctx, random, &logs, &self.log_decorators);
        }
        true
    }

    fn over_solid(&self, ctx: &Ctx, pos: BlockPos) -> bool {
        ctx.registries().blocks.is_face_sturdy(ctx.block(pos.below()), Direction::Up, minecraftoss_core::SupportType::Full)
    }

    fn can_place_all(&self, ctx: &Ctx, length: i32, start: BlockPos, direction: Direction) -> bool {
        let mut gap = 0;
        let mut pos = start;
        for _ in 0..length {
            if !valid_tree_pos(ctx, pos) {
                return false;
            }
            if !self.over_solid(ctx, pos) {
                gap += 1;
                if gap > 2 {
                    return false;
                }
            } else {
                gap = 0;
            }
            pos = pos.relative(direction, 1);
        }
        true
    }

    fn place_log(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos, sideways: Option<Direction>) -> BlockPos {
        let mut state = self.trunk_provider.get(ctx, random, pos);
        if let Some(d) = sideways {
            state = try_with(ctx.registries(), state, "axis", d.axis().name());
        }
        ctx.set_block_update(pos, state);
        ctx.mark_above_for_post_processing(pos);
        pos
    }

    fn decorate(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, logs: &JavaHashSet, decorators: &[Decorator]) {
        if decorators.is_empty() {
            return;
        }
        let empty = JavaHashSet::new();
        let mut decorations = JavaHashSet::new();
        let mut context = Context::new(logs, &empty, &empty, &mut decorations, false);
        for decorator in decorators {
            decorator.place(ctx, random, &mut context);
        }
    }
}
