//! Selectors and small block-placing features.

use super::{bool_or, float_or, int};
use crate::feature::blocks::BlockSet;
use crate::feature::predicate::{parse_direction, BlockPredicate};
use crate::feature::state::StateProvider;
use crate::feature::{place_placed, Ctx, Library, PlacedId};
use crate::providers::{IntProvider, Weighted};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{RandomSource, WorldgenRandom};
use minecraftoss_core::{BlockPos, BlockStateId};
use serde_json::Value;
use std::sync::Arc;

fn placed_list(lib: &mut Library, json: &Value) -> Result<Vec<PlacedId>, String> {
    match json {
        Value::Array(list) => list.iter().map(|p| lib.placed_ref(p)).collect(),
        other => Ok(vec![lib.placed_ref(other)?]),
    }
}

#[derive(Debug)]
pub struct SimpleBlock {
    to_place: Arc<StateProvider>,
    schedule_tick: bool,
}

impl SimpleBlock {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self { to_place: StateProvider::parse(lib, &json["to_place"])?, schedule_tick: bool_or(json, "schedule_tick", false) })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let Some(state) = self.to_place.get_optional(ctx, random, origin) else {
            return false;
        };
        if !ctx.can_survive(state, origin) {
            return false;
        }
        match ctx.lib.survival.shape_class(&ctx.lib.registries, state) {
            PlantShape::DoublePlant => {
                let above = ctx.block(origin.above());
                let fluid = |s: BlockStateId| ctx.registries().blocks.state(s).fluid;
                if !ctx.is_air(above) && (fluid(state) != fluid(above) || !ctx.is_replaceable(above)) {
                    return false;
                }
                place_double_plant(ctx, state, origin, 2);
            }
            PlantShape::MossyCarpet => place_mossy_carpet(ctx, origin, 2),
            PlantShape::Other => {
                ctx.set_block(origin, state);
            }
        }
        if self.schedule_tick {
            ctx.schedule_block_tick(origin);
        }
        true
    }
}

/// How `SimpleBlockFeature` places a state.
pub enum PlantShape {
    DoublePlant,
    MossyCarpet,
    Other,
}

/// `DoublePlantBlock.placeAt`.
pub fn place_double_plant(ctx: &mut Ctx, state: BlockStateId, lower: BlockPos, flags: u32) {
    let upper = lower.above();
    let low = copy_waterlogged(ctx, lower, ctx.with(state, "half", "lower"));
    ctx.set_block_flags(lower, low, flags);
    let high = copy_waterlogged(ctx, upper, ctx.with(state, "half", "upper"));
    ctx.set_block_flags(upper, high, flags);
}

/// `DoublePlantBlock.copyWaterloggedFrom`.
pub fn copy_waterlogged(ctx: &Ctx, pos: BlockPos, state: BlockStateId) -> BlockStateId {
    if ctx.property(state, "waterlogged").is_none() {
        return state;
    }
    let water = matches!(ctx.fluid_at(pos), crate::feature::blocks::FluidType::Water | crate::feature::blocks::FluidType::FlowingWater);
    ctx.with(state, "waterlogged", if water { "true" } else { "false" })
}

const SIDES: [Direction; 4] = [Direction::North, Direction::East, Direction::South, Direction::West];

/// `MultifaceBlock.canAttachTo(level, direction, neighbourPos, neighbour)`.
pub fn can_attach_to<W: crate::feature::World + ?Sized>(ctx: &Ctx<W>, direction: Direction, neighbor: BlockState) -> bool {
    let face = direction.opposite();
    let blocks = &ctx.registries().blocks;
    blocks.is_face_sturdy(neighbor, face, minecraftoss_core::SupportType::Full)
        || blocks.collision_shape(neighbor).is_some_and(|shape| crate::feature::survive::is_face_full(shape, face))
}

type BlockState = BlockStateId;

/// `MossyCarpetBlock.getUpdatedState`.
pub(crate) fn mossy_updated<W: crate::feature::World + ?Sized>(ctx: &Ctx<W>, mut state: BlockState, pos: BlockPos, mut create_sides: bool) -> BlockState {
    let mut above: Option<BlockState> = None;
    let mut below: Option<BlockState> = None;
    let base = ctx.property(state, "bottom") == Some("true");
    create_sides |= base;
    for direction in SIDES {
        let name = direction.name();
        let supported = direction != Direction::Up && can_attach_to(ctx, direction, ctx.block(pos.relative(direction, 1)));
        let mut side = if supported {
            if create_sides { "low" } else { ctx.property(state, name).unwrap_or("none") }
        } else {
            "none"
        };
        if side == "low" {
            let a = *above.get_or_insert_with(|| ctx.block(pos.above()));
            if ctx.is(a, "minecraft:pale_moss_carpet") && ctx.property(a, name) != Some("none") && ctx.property(a, "bottom") != Some("true") {
                side = "tall";
            }
            if ctx.property(state, "bottom") != Some("true") {
                let b = *below.get_or_insert_with(|| ctx.block(pos.below()));
                if ctx.is(b, "minecraft:pale_moss_carpet") && ctx.property(b, name) == Some("none") {
                    side = "none";
                }
            }
        }
        state = ctx.with(state, name, side);
    }
    state
}

pub(crate) fn mossy_has_faces<W: crate::feature::World + ?Sized>(ctx: &Ctx<W>, state: BlockState) -> bool {
    ctx.property(state, "bottom") == Some("true") || SIDES.iter().any(|d| ctx.property(state, d.name()) != Some("none"))
}

/// `MossyCarpetBlock.placeAt` with the region's own random.
fn place_mossy_carpet(ctx: &mut Ctx, pos: BlockPos, flags: u32) {
    let carpet = ctx.registries().blocks.parse_state("minecraft:pale_moss_carpet").expect("pale moss carpet exists");
    let adjusted = mossy_updated(ctx, carpet, pos, true);
    ctx.set_block_flags(pos, adjusted, flags);
    // createTopperWithSideChance
    let above = pos.above();
    let previous = ctx.block(above);
    let is_carpet = ctx.is(previous, "minecraft:pale_moss_carpet");
    let topper = if (!is_carpet || ctx.property(previous, "bottom") != Some("true")) && (is_carpet || ctx.is_replaceable(previous)) {
        let no_base = ctx.with(carpet, "bottom", "false");
        let mut top = mossy_updated(ctx, no_base, above, true);
        for direction in SIDES {
            if ctx.property(top, direction.name()) != Some("none") && !ctx.region.level_random().next_bool() {
                top = ctx.with(top, direction.name(), "none");
            }
        }
        if mossy_has_faces(ctx, top) && top != previous { top } else { ctx.lib.blocks.air }
    } else {
        ctx.lib.blocks.air
    };
    if !ctx.is_air(topper) {
        ctx.set_block_flags(above, topper, flags);
        let bottom = mossy_updated(ctx, adjusted, pos, true);
        ctx.set_block_flags(pos, bottom, flags);
    }
}

#[derive(Debug)]
struct Layer {
    height: IntProvider,
    provider: Arc<StateProvider>,
}

#[derive(Debug)]
pub struct BlockColumn {
    layers: Vec<Layer>,
    direction: Direction,
    allowed: BlockPredicate,
    prioritize_tip: bool,
}

impl BlockColumn {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let mut layers = Vec::new();
        for layer in json["layers"].as_array().ok_or("block_column lacks layers")? {
            layers.push(Layer { height: IntProvider::parse(&layer["height"])?, provider: StateProvider::parse(lib, &layer["provider"])? });
        }
        Ok(Self {
            layers,
            direction: parse_direction(&json["direction"])?,
            allowed: BlockPredicate::parse(&lib.registries, &json["allowed_placement"])?,
            prioritize_tip: bool_or(json, "prioritize_tip", false),
        })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut heights: Vec<i32> = self.layers.iter().map(|l| l.height.sample(random)).collect();
        let total: i32 = heights.iter().sum();
        if total == 0 {
            return false;
        }
        let mut next = origin.relative(self.direction, 1);
        for y in 0..total {
            if !self.allowed.test(ctx, next) {
                // truncate
                let mut remove = total - y;
                let order: Vec<usize> = if self.prioritize_tip { (0..heights.len()).collect() } else { (0..heights.len()).rev().collect() };
                for i in order {
                    if remove <= 0 {
                        break;
                    }
                    let take = heights[i].min(remove);
                    remove -= take;
                    heights[i] -= take;
                }
                break;
            }
            next = next.relative(self.direction, 1);
        }
        let mut pos = origin;
        for (layer, &count) in self.layers.iter().zip(&heights) {
            for _ in 0..count {
                let state = layer.provider.get(ctx, random, pos);
                ctx.set_block(pos, state);
                pos = pos.relative(self.direction, 1);
            }
        }
        true
    }
}

#[derive(Debug)]
pub struct BlockPile {
    provider: Arc<StateProvider>,
}

impl BlockPile {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self { provider: StateProvider::parse(lib, &json["state_provider"])? })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if origin.y < ctx.min_y() + 5 {
            return false;
        }
        let xr = 2 + random.next_i32_bound(2);
        let zr = 2 + random.next_i32_bound(2);
        // BlockPos.betweenClosed iterates x fastest, then y, then z.
        for z in -zr..=zr {
            for y in 0..=1 {
                for x in -xr..=xr {
                    let pos = origin.offset(x, y, z);
                    let (xd, zd) = (-x, -z);
                    let limit = random.next_f32() * 10.0 - random.next_f32() * 6.0;
                    if ((xd * xd + zd * zd) as f32) <= limit {
                        self.try_place(ctx, random, pos);
                    } else if f64::from(random.next_f32()) < 0.031 {
                        self.try_place(ctx, random, pos);
                    }
                }
            }
        }
        true
    }

    fn try_place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos) {
        if !ctx.is_empty_block(pos) {
            return;
        }
        let below = ctx.block(pos.below());
        let may = if ctx.is(below, "minecraft:dirt_path") {
            random.next_bool()
        } else {
            ctx.registries().blocks.is_face_sturdy(below, Direction::Up, minecraftoss_core::SupportType::Full)
        };
        if may {
            let state = self.provider.get(ctx, random, pos);
            ctx.set_block_flags(pos, state, 260);
        }
    }
}

#[derive(Debug)]
pub struct RandomSelector {
    features: Vec<(PlacedId, f32)>,
    default: PlacedId,
}

impl RandomSelector {
    /// The placed features `getSubFeatures` visits, in order.
    pub fn nested(&self) -> Vec<PlacedId> {
        self.features.iter().map(|&(p, _)| p).chain(std::iter::once(self.default)).collect()
    }

    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let mut features = Vec::new();
        for entry in json["features"].as_array().ok_or("random_selector lacks features")? {
            features.push((lib.placed_ref(&entry["feature"])?, entry["chance"].as_f64().ok_or("missing chance")? as f32));
        }
        Ok(Self { features, default: lib.placed_ref(&json["default"])? })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        for &(feature, chance) in &self.features {
            if random.next_f32() < chance {
                return place_placed(ctx, random, feature, origin);
            }
        }
        place_placed(ctx, random, self.default, origin)
    }
}

#[derive(Debug)]
pub struct SimpleRandomSelector {
    features: Vec<PlacedId>,
}

impl SimpleRandomSelector {
    /// The placed features `getSubFeatures` visits, in order.
    pub fn nested(&self) -> Vec<PlacedId> {
        self.features.clone()
    }

    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self { features: placed_list(lib, &json["features"])? })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let index = random.next_i32_bound(self.features.len() as i32) as usize;
        place_placed(ctx, random, self.features[index], origin)
    }
}

#[derive(Debug)]
pub struct WeightedRandomSelector {
    features: Weighted<PlacedId>,
}

impl WeightedRandomSelector {
    /// The placed features `getSubFeatures` visits, in order.
    pub fn nested(&self) -> Vec<PlacedId> {
        self.features.items().copied().collect()
    }

    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let list = json["features"].as_array().ok_or("weighted_random_selector lacks features")?;
        let mut entries = Vec::new();
        for entry in list {
            entries.push((lib.placed_ref(&entry["data"])?, entry["weight"].as_i64().ok_or("missing weight")? as i32));
        }
        Ok(Self { features: Weighted::from_entries(entries) })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        match self.features.pick(random) {
            Some(&feature) => place_placed(ctx, random, feature, origin),
            None => false,
        }
    }
}

#[derive(Debug)]
pub struct RandomBooleanSelector {
    feature_true: PlacedId,
    feature_false: PlacedId,
}

impl RandomBooleanSelector {
    /// The placed features `getSubFeatures` visits, in order.
    pub fn nested(&self) -> Vec<PlacedId> {
        vec![self.feature_true, self.feature_false]
    }

    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self { feature_true: lib.placed_ref(&json["feature_true"])?, feature_false: lib.placed_ref(&json["feature_false"])? })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let feature = if random.next_bool() { self.feature_true } else { self.feature_false };
        place_placed(ctx, random, feature, origin)
    }
}

#[derive(Debug)]
pub struct Sequence {
    features: Vec<PlacedId>,
}

impl Sequence {
    /// The placed features `getSubFeatures` visits, in order.
    pub fn nested(&self) -> Vec<PlacedId> {
        self.features.clone()
    }

    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self { features: placed_list(lib, &json["features"])? })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        for &feature in &self.features {
            if !place_placed(ctx, random, feature, origin) {
                return false;
            }
        }
        true
    }
}

#[derive(Debug)]
pub struct Overlay {
    features: Vec<PlacedId>,
}

impl Overlay {
    /// The placed features `getSubFeatures` visits, in order.
    pub fn nested(&self) -> Vec<PlacedId> {
        self.features.clone()
    }

    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self { features: placed_list(lib, &json["features"])? })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut any = false;
        for &feature in &self.features {
            any |= place_placed(ctx, random, feature, origin);
        }
        any
    }
}

#[derive(Debug)]
pub struct SingleBlockPillar {
    block: Arc<StateProvider>,
    can_replace: BlockPredicate,
    direction: Direction,
    chance_to_continue: f32,
    cap: Option<PlacedId>,
}

impl SingleBlockPillar {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            block: StateProvider::parse(lib, &json["block"])?,
            can_replace: match json.get("can_replace") {
                Some(p) => BlockPredicate::parse(&lib.registries, p)?,
                None => BlockPredicate::True,
            },
            direction: parse_direction(&json["direction"])?,
            chance_to_continue: float_or(json, "chance_to_continue", 1.0),
            cap: json.get("cap_feature").map(|c| lib.placed_ref(c)).transpose()?,
        })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut pos = origin;
        while self.can_replace.test(ctx, pos) && random.next_f32() < self.chance_to_continue && !ctx.is_outside_build_height(pos.y) {
            let state = self.block.get(ctx, random, pos);
            ctx.set_block(pos, state);
            pos = pos.relative(self.direction, 1);
        }
        pos = pos.relative(self.direction.opposite(), 1);
        if let Some(cap) = self.cap {
            place_placed(ctx, random, cap, pos);
        }
        true
    }
}

#[derive(Debug)]
pub struct ProjectedRandomPatchySquare {
    block: Arc<StateProvider>,
    project_through: BlockPredicate,
    size: IntProvider,
    max_projection_height: i32,
}

impl ProjectedRandomPatchySquare {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            block: StateProvider::parse(lib, &json["block"])?,
            project_through: BlockPredicate::parse(&lib.registries, &json["project_through"])?,
            size: IntProvider::parse(&json["size"])?,
            max_projection_height: int(json, "max_projection_height")?,
        })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let size = self.size.sample(random);
        let bound = size * size + 1;
        for dx in -size..=size {
            for dz in -size..=size {
                let probability = dx.abs() * dz.abs();
                if random.next_i32_bound(bound) < bound - probability {
                    let mut base = origin.offset(dx, 0, dz);
                    let mut drop = self.max_projection_height;
                    while self.project_through.test(ctx, base.below()) {
                        base = base.below();
                        drop -= 1;
                        if drop <= 0 {
                            break;
                        }
                    }
                    if let Some(state) = self.block.get_optional(ctx, random, base) {
                        ctx.set_block(base, state);
                    }
                }
            }
        }
        true
    }
}

#[derive(Debug)]
pub struct RandomNeighborSpread {
    block: Arc<StateProvider>,
    accepted: BlockSet,
    can_replace: BlockPredicate,
    attempts: IntProvider,
    xz_offset: IntProvider,
    y_offset: IntProvider,
}

impl RandomNeighborSpread {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            block: StateProvider::parse(lib, &json["block"])?,
            accepted: BlockSet::parse(&lib.registries, &json["accepted_neighbors"])?,
            can_replace: BlockPredicate::parse(&lib.registries, &json["can_replace"])?,
            attempts: IntProvider::parse(&json["attempts"])?,
            xz_offset: IntProvider::parse(&json["xz_offset"])?,
            y_offset: IntProvider::parse(&json["y_offset"])?,
        })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let state = self.block.get(ctx, random, origin);
        ctx.set_block(origin, state);
        let attempts = self.attempts.sample(random);
        for _ in 0..attempts {
            let dx = self.xz_offset.sample(random);
            let dy = self.y_offset.sample(random);
            let dz = self.xz_offset.sample(random);
            let pos = origin.offset(dx, dy, dz);
            if !self.can_replace.test(ctx, pos) {
                continue;
            }
            let mut neighbors = 0;
            for direction in Direction::ALL {
                if self.accepted.contains(ctx.registries(), ctx.block(pos.relative(direction, 1))) {
                    neighbors += 1;
                }
                if neighbors > 1 {
                    break;
                }
            }
            if neighbors == 1 {
                let state = self.block.get(ctx, random, pos);
                ctx.set_block(pos, state);
            }
        }
        true
    }
}

#[derive(Debug)]
pub struct FillLayer {
    height: i32,
    state: BlockStateId,
}

impl FillLayer {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self { height: int(json, "height")?, state: crate::feature::blocks::parse_state(&lib.registries, &json["state"])? })
    }

    pub fn place(&self, ctx: &mut Ctx, origin: BlockPos) -> bool {
        for dx in 0..16 {
            for dz in 0..16 {
                let pos = BlockPos::new(origin.x + dx, ctx.min_y() + self.height, origin.z + dz);
                if ctx.is_empty_block(pos) {
                    ctx.set_block(pos, self.state);
                }
            }
        }
        true
    }
}

/// `BlueIceFeature`.
pub fn blue_ice(ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
    let b = &ctx.lib.blocks;
    let (water, packed, blue, ice) = (b.water, b.packed_ice, b.blue_ice, b.ice);
    if origin.y > ctx.lib.generation.sea_level - 1 {
        return false;
    }
    if ctx.block(origin) != water && ctx.block(origin.below()) != water {
        return false;
    }
    let is = |ctx: &Ctx, s: BlockStateId, t: BlockStateId| ctx.registries().blocks.block_of(s) == ctx.registries().blocks.block_of(t);
    let found = Direction::ALL.iter().any(|&d| d != Direction::Down && is(ctx, ctx.block(origin.relative(d, 1)), packed));
    if !found {
        return false;
    }
    ctx.set_block(origin, blue);
    for _ in 0..200 {
        let y_off = random.next_i32_bound(5) - random.next_i32_bound(6);
        let mut xz = 3;
        if y_off < 2 {
            xz += y_off / 2;
        }
        if xz >= 1 {
            let dx = random.next_i32_bound(xz) - random.next_i32_bound(xz);
            let dz = random.next_i32_bound(xz) - random.next_i32_bound(xz);
            let pos = origin.offset(dx, y_off, dz);
            let s = ctx.block(pos);
            if ctx.is_air(s) || is(ctx, s, water) || is(ctx, s, packed) || is(ctx, s, ice) {
                for d in Direction::ALL {
                    if is(ctx, ctx.block(pos.relative(d, 1)), blue) {
                        ctx.set_block(pos, blue);
                        break;
                    }
                }
            }
        }
    }
    true
}

/// `SnowAndFreezeFeature`.
pub fn freeze_top_layer(ctx: &mut Ctx, origin: BlockPos) -> bool {
    let (ice, snow) = (ctx.lib.blocks.ice, ctx.lib.blocks.snow);
    for dx in 0..16 {
        for dz in 0..16 {
            let (x, z) = (origin.x + dx, origin.z + dz);
            let top = BlockPos::new(x, ctx.height(HeightmapKind::MotionBlocking, x, z), z);
            let below = top.below();
            let Some(biome) = ctx.biome(top) else { continue };
            if should_freeze(ctx, biome, below) {
                ctx.set_block(below, ice);
            }
            if should_snow(ctx, biome, top) {
                ctx.set_block(top, snow);
                let below_state = ctx.block(below);
                if ctx.property(below_state, "snowy").is_some() {
                    let snowy = ctx.with(below_state, "snowy", "true");
                    ctx.set_block(below, snowy);
                }
            }
        }
    }
    true
}

/// `Biome.warmEnoughToRain`.
pub fn warm_enough_to_rain(ctx: &Ctx, biome: minecraftoss_core::BiomeId, pos: BlockPos) -> bool {
    ctx.temperature(biome, pos) >= 0.15
}

/// `Biome.shouldFreeze(level, pos, false)` in an unlit chunk.
pub fn should_freeze(ctx: &Ctx, biome: minecraftoss_core::BiomeId, pos: BlockPos) -> bool {
    if warm_enough_to_rain(ctx, biome, pos) {
        return false;
    }
    !ctx.is_outside_build_height(pos.y) && ctx.block(pos) == ctx.lib.blocks.water
}

/// `Biome.shouldSnow` in an unlit chunk.
pub fn should_snow(ctx: &Ctx, biome: minecraftoss_core::BiomeId, pos: BlockPos) -> bool {
    let info = ctx.registries().biomes.get(biome);
    if !info.has_precipitation || warm_enough_to_rain(ctx, biome, pos) {
        return false;
    }
    if ctx.is_outside_build_height(pos.y) {
        return false;
    }
    let state = ctx.block(pos);
    (ctx.is_air(state) || ctx.is(state, "minecraft:snow")) && ctx.can_survive(ctx.lib.blocks.snow, pos)
}

/// `VinesFeature`.
pub fn vines(ctx: &mut Ctx, origin: BlockPos) -> bool {
    if !ctx.is_empty_block(origin) {
        return false;
    }
    for direction in Direction::ALL {
        if direction != Direction::Down && can_attach_to(ctx, direction, ctx.block(origin.relative(direction, 1))) {
            let vine = ctx.with(ctx.lib.blocks.vine, direction.name(), "true");
            ctx.set_block(origin, vine);
            return true;
        }
    }
    false
}
