//! Tree decorators (vanilla `feature.treedecorators`).

use crate::feature::kinds::{float, int, int_or, shuffle};
use crate::feature::state::StateProvider;
use crate::feature::{place_feature, Ctx, FeatureId, Library};
use crate::feature::java_set::JavaHashSet;
use minecraftoss_core::block::flags;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::{BlockPos, BlockStateId};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug)]
pub enum Decorator {
    TrunkVine,
    LeaveVine(f32),
    Cocoa(f32),
    Beehive(f32),
    AlterGround(Arc<StateProvider>),
    AttachedToLeaves { probability: f32, exclusion_xz: i32, exclusion_y: i32, provider: Arc<StateProvider>, required_empty: i32, directions: Vec<Direction> },
    AttachedToLogs { probability: f32, provider: Arc<StateProvider>, directions: Vec<Direction> },
    CreakingHeart(f32),
    PaleMoss { leaves: f32, trunk: f32, ground: f32, patch: Option<FeatureId> },
    PlaceOnGround { tries: i32, radius: i32, height: i32, provider: Arc<StateProvider> },
    ShelfMushroom(f32),
}

fn directions(json: &Value) -> Result<Vec<Direction>, String> {
    json.as_array()
        .ok_or("directions is not a list")?
        .iter()
        .map(crate::feature::predicate::parse_direction)
        .collect()
}

impl Decorator {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let kind = json["type"].as_str().ok_or("tree decorator lacks a type")?;
        Ok(match kind.trim_start_matches("minecraft:") {
            "trunk_vine" => Self::TrunkVine,
            "leave_vine" => Self::LeaveVine(float(json, "probability")?),
            "cocoa" => Self::Cocoa(float(json, "probability")?),
            "beehive" => Self::Beehive(float(json, "probability")?),
            "alter_ground" => Self::AlterGround(StateProvider::parse(lib, &json["provider"])?),
            "attached_to_leaves" => Self::AttachedToLeaves {
                probability: float(json, "probability")?,
                exclusion_xz: int(json, "exclusion_radius_xz")?,
                exclusion_y: int(json, "exclusion_radius_y")?,
                provider: StateProvider::parse(lib, &json["block_provider"])?,
                required_empty: int(json, "required_empty_blocks")?,
                directions: directions(&json["directions"])?,
            },
            "attached_to_logs" => Self::AttachedToLogs {
                probability: float(json, "probability")?,
                provider: StateProvider::parse(lib, &json["block_provider"])?,
                directions: directions(&json["directions"])?,
            },
            "creaking_heart" => Self::CreakingHeart(float(json, "probability")?),
            "pale_moss" => Self::PaleMoss {
                leaves: float(json, "leaves_probability")?,
                trunk: float(json, "trunk_probability")?,
                ground: float(json, "ground_probability")?,
                patch: lib.feature_by_name("minecraft:pale_moss_patch").or_else(|| lib.feature_ref(&Value::String("minecraft:pale_moss_patch".into())).ok()),
            },
            "place_on_ground" => Self::PlaceOnGround {
                tries: int_or(json, "tries", 128),
                radius: int_or(json, "radius", 2),
                height: int_or(json, "height", 1),
                provider: StateProvider::parse(lib, &json["block_state_provider"])?,
            },
            "shelf_mushroom" => Self::ShelfMushroom(float(json, "probability")?),
            other => return Err(format!("unknown tree decorator {other}")),
        })
    }
}

/// `TreeDecorator.Context`: position lists sorted by Y (stable over hash order).
pub struct Context<'s> {
    pub logs: Vec<BlockPos>,
    pub leaves: Vec<BlockPos>,
    pub roots: Vec<BlockPos>,
    pub decorations: &'s mut JavaHashSet,
    /// Whether the setter records into `decorations` (trees) or only sets (fallen trees).
    pub record: bool,
}

impl Context<'_> {
    pub fn new<'s>(logs: &JavaHashSet, leaves: &JavaHashSet, roots: &JavaHashSet, decorations: &'s mut JavaHashSet, record: bool) -> Context<'s> {
        let sorted = |set: &JavaHashSet| {
            let mut v = set.to_vec();
            v.sort_by_key(|p| p.y);
            v
        };
        Context { logs: sorted(logs), leaves: sorted(leaves), roots: sorted(roots), decorations, record }
    }

    fn set(&mut self, ctx: &mut Ctx, pos: BlockPos, state: BlockStateId) {
        if self.record {
            self.decorations.insert(pos);
        }
        ctx.set_block_flags(pos, state, 19);
    }

    fn vine(&mut self, ctx: &mut Ctx, pos: BlockPos, face: Direction) {
        let vine = ctx.with(ctx.lib.blocks.vine, face.name(), "true");
        self.set(ctx, pos, vine);
    }

    /// `TreeFeature.getLowestTrunkOrRootOfTree`.
    fn lowest_trunk_or_root(&self) -> Vec<BlockPos> {
        if self.roots.is_empty() {
            self.logs.clone()
        } else if !self.logs.is_empty() && self.roots[0].y == self.logs[0].y {
            let mut v = self.logs.clone();
            v.extend(&self.roots);
            v
        } else {
            self.roots.clone()
        }
    }
}

fn is_water_or_near(ctx: &Ctx, pos: BlockPos) -> bool {
    let water = ctx.lib.blocks.water;
    [pos, pos.offset(1, 0, 0), pos.offset(-1, 0, 0), pos.offset(0, 0, -1), pos.offset(0, 0, 1)].iter().any(|&p| ctx.block(p) == water)
}

impl Decorator {
    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, c: &mut Context) {
        match self {
            Self::TrunkVine => {
                for pos in c.logs.clone() {
                    for (offset, face) in [((-1, 0), Direction::East), ((1, 0), Direction::West), ((0, -1), Direction::South), ((0, 1), Direction::North)] {
                        if random.next_i32_bound(3) > 0 {
                            let p = pos.offset(offset.0, 0, offset.1);
                            if ctx.is_empty_block(p) {
                                c.vine(ctx, p, face);
                            }
                        }
                    }
                }
            }
            Self::LeaveVine(probability) => {
                for pos in c.leaves.clone() {
                    for (offset, face) in [((-1, 0), Direction::East), ((1, 0), Direction::West), ((0, -1), Direction::South), ((0, 1), Direction::North)] {
                        if random.next_f32() < *probability {
                            let p = pos.offset(offset.0, 0, offset.1);
                            if ctx.is_empty_block(p) {
                                c.vine(ctx, p, face);
                                let mut below = p.below();
                                let mut max = 4;
                                while ctx.is_empty_block(below) && max > 0 {
                                    c.vine(ctx, below, face);
                                    below = below.below();
                                    max -= 1;
                                }
                            }
                        }
                    }
                }
            }
            Self::Cocoa(probability) => {
                if random.next_f32() >= *probability || c.logs.is_empty() {
                    return;
                }
                let tree_y = c.logs[0].y;
                let cocoa = ctx.registries().blocks.parse_state("minecraft:cocoa").expect("cocoa exists");
                for pos in c.logs.clone() {
                    if pos.y - tree_y > 2 {
                        continue;
                    }
                    for direction in Direction::HORIZONTAL {
                        if random.next_f32() <= 0.25 {
                            let (dx, _, dz) = direction.opposite().offset();
                            let p = pos.offset(dx, 0, dz);
                            if ctx.is_empty_block(p) {
                                let age = random.next_i32_bound(3);
                                let state = ctx.with(ctx.with(cocoa, "age", &age.to_string()), "facing", direction.name());
                                c.set(ctx, p, state);
                            }
                        }
                    }
                }
            }
            Self::Beehive(probability) => {
                if c.logs.is_empty() {
                    return;
                }
                if random.next_f32() >= *probability {
                    return;
                }
                let hive_y = if !c.leaves.is_empty() {
                    (c.leaves[0].y - 1).max(c.logs[0].y + 1)
                } else {
                    (c.logs[0].y + 1 + random.next_i32_bound(3)).min(c.logs[c.logs.len() - 1].y)
                };
                // SPAWN_DIRECTIONS: horizontal except NORTH (the worldgen facing is SOUTH).
                let spawn = [Direction::East, Direction::South, Direction::West];
                let mut placements: Vec<BlockPos> =
                    c.logs.iter().filter(|p| p.y == hive_y).flat_map(|&p| spawn.iter().map(move |&d| p.relative(d, 1))).collect();
                if placements.is_empty() {
                    return;
                }
                shuffle(&mut placements, random);
                let found = placements.into_iter().find(|&p| ctx.is_empty_block(p) && ctx.is_empty_block(p.relative(Direction::South, 1)));
                if let Some(pos) = found {
                    let nest = ctx.registries().blocks.parse_state("minecraft:bee_nest").expect("bee nest exists");
                    let nest = ctx.with(nest, "facing", "south");
                    c.set(ctx, pos, nest);
                    // The hive's block entity exists (a DUMMY tag), so bees are stored.
                    let bees = 2 + random.next_i32_bound(2);
                    let ticks: Vec<i32> = (0..bees).map(|_| random.next_i32_bound(599)).collect();
                    ctx.region.add_bees(pos.x, pos.y, pos.z, &ticks);
                }
            }
            Self::AlterGround(provider) => {
                let positions = c.lowest_trunk_or_root();
                if positions.is_empty() {
                    return;
                }
                let min_y = positions[0].y;
                for pos in positions.into_iter().filter(|p| p.y == min_y) {
                    for corner in [pos.offset(-1, 0, -1), pos.offset(2, 0, -1), pos.offset(-1, 0, 2), pos.offset(2, 0, 2)] {
                        alter_circle(ctx, random, c, provider, corner);
                    }
                    for _ in 0..5 {
                        let placement = random.next_i32_bound(64);
                        let (xx, zz) = (placement % 8, placement / 8);
                        if xx == 0 || xx == 7 || zz == 0 || zz == 7 {
                            alter_circle(ctx, random, c, provider, pos.offset(-3 + xx, 0, -3 + zz));
                        }
                    }
                }
            }
            Self::AttachedToLeaves { probability, exclusion_xz, exclusion_y, provider, required_empty, directions } => {
                let mut blacklist = std::collections::HashSet::new();
                let mut leaves = c.leaves.clone();
                shuffle(&mut leaves, random);
                for leaf in leaves {
                    let direction = directions[random.next_i32_bound(directions.len() as i32) as usize];
                    let placement = leaf.relative(direction, 1);
                    if blacklist.contains(&placement) || random.next_f32() >= *probability {
                        continue;
                    }
                    let empty = (1..=*required_empty).all(|i| ctx.is_empty_block(leaf.relative(direction, i)));
                    if !empty {
                        continue;
                    }
                    for x in -exclusion_xz..=*exclusion_xz {
                        for y in -exclusion_y..=*exclusion_y {
                            for z in -exclusion_xz..=*exclusion_xz {
                                blacklist.insert(placement.offset(x, y, z));
                            }
                        }
                    }
                    let state = provider.get(ctx, random, placement);
                    c.set(ctx, placement, state);
                }
            }
            Self::AttachedToLogs { probability, provider, directions } => {
                let mut logs = c.logs.clone();
                shuffle(&mut logs, random);
                for log in logs {
                    let direction = directions[random.next_i32_bound(directions.len() as i32) as usize];
                    let placement = log.relative(direction, 1);
                    if random.next_f32() <= *probability && ctx.is_empty_block(placement) {
                        let state = provider.get(ctx, random, placement);
                        c.set(ctx, placement, state);
                    }
                }
            }
            Self::CreakingHeart(probability) => {
                if c.logs.is_empty() || random.next_f32() >= *probability {
                    return;
                }
                let mut placements = c.logs.clone();
                shuffle(&mut placements, random);
                let logs = ctx.lib.tags.logs;
                let target = placements.into_iter().find(|&p| Direction::ALL.iter().all(|&d| ctx.in_tag(ctx.block(p.relative(d, 1)), logs)));
                if let Some(pos) = target {
                    let heart = ctx.registries().blocks.parse_state("minecraft:creaking_heart").expect("creaking heart exists");
                    let heart = ctx.with(ctx.with(heart, "creaking_heart_state", "dormant"), "natural", "true");
                    c.set(ctx, pos, heart);
                }
            }
            Self::PaleMoss { leaves, trunk, ground, patch } => {
                let mut logs = c.logs.clone();
                shuffle(&mut logs, random);
                if logs.is_empty() {
                    return;
                }
                // Collections.min: the first with the lowest Y in shuffled order.
                let origin = logs.iter().copied().reduce(|a, b| if b.y < a.y { b } else { a }).expect("non-empty");
                if random.next_f32() < *ground {
                    if let Some(patch) = patch {
                        place_feature(ctx, random, *patch, origin.above());
                    }
                }
                for pos in c.logs.clone() {
                    if random.next_f32() < *trunk {
                        let down = pos.below();
                        if ctx.is_empty_block(down) {
                            moss_hanger(ctx, random, c, down);
                        }
                    }
                }
                for pos in c.leaves.clone() {
                    if random.next_f32() < *leaves {
                        let down = pos.below();
                        if ctx.is_empty_block(down) {
                            moss_hanger(ctx, random, c, down);
                        }
                    }
                }
            }
            Self::PlaceOnGround { tries, radius, height, provider } => {
                let positions = c.lowest_trunk_or_root();
                if positions.is_empty() {
                    return;
                }
                let origin = positions[0];
                let (mut min_x, mut max_x, mut min_z, mut max_z) = (origin.x, origin.x, origin.z, origin.z);
                for p in &positions {
                    if p.y == origin.y {
                        min_x = min_x.min(p.x);
                        max_x = max_x.max(p.x);
                        min_z = min_z.min(p.z);
                        max_z = max_z.max(p.z);
                    }
                }
                let (bx0, bx1) = (min_x - radius, max_x + radius);
                let (by0, by1) = (origin.y - height, origin.y + height);
                let (bz0, bz1) = (min_z - radius, max_z + radius);
                for _ in 0..*tries {
                    let x = crate::providers::random_between_inclusive(random, bx0, bx1);
                    let y = crate::providers::random_between_inclusive(random, by0, by1);
                    let z = crate::providers::random_between_inclusive(random, bz0, bz1);
                    let pos = BlockPos::new(x, y, z);
                    let above = pos.above();
                    let above_state = ctx.block(above);
                    if (ctx.is_air(above_state) || above_state == ctx.lib.blocks.vine || ctx.is(above_state, "minecraft:vine"))
                        && ctx.registries().blocks.is(ctx.block(pos), flags::SOLID_RENDER)
                        && ctx.height(HeightmapKind::MotionBlockingNoLeaves, pos.x, pos.z) <= above.y
                    {
                        let state = provider.get(ctx, random, above);
                        c.set(ctx, above, state);
                    }
                }
            }
            Self::ShelfMushroom(probability) => {
                if random.next_f32() >= *probability || c.logs.is_empty() {
                    return;
                }
                let logs = c.logs.clone();
                let fallen = logs[0].y == logs[logs.len() - 1].y;
                if fallen {
                    let (first, last) = (logs[0], logs[logs.len() - 1]);
                    let dirs = if first.x != last.x { [Direction::North, Direction::South] } else { [Direction::East, Direction::West] };
                    for &log in &logs {
                        for facing in dirs {
                            if random.next_f32() <= 0.25 {
                                let p = log.relative(facing, 1);
                                if shelf_replaceable(ctx, p) && !shelf_adjacent(ctx, p) && !shelf_adjacent(ctx, log) {
                                    place_shelf(ctx, random, c, p, facing);
                                }
                            }
                        }
                    }
                } else {
                    let first = Direction::HORIZONTAL[random.next_i32_bound(4) as usize];
                    let dirs = [first, first.clockwise()];
                    let base = logs[0].y;
                    for &log in &logs {
                        let dy = log.y - base;
                        if !(1..=4).contains(&dy) {
                            continue;
                        }
                        for facing in dirs {
                            if random.next_f32() <= 0.25 {
                                let p = log.relative(facing, 1);
                                if shelf_replaceable(ctx, p) && !ctx.is(ctx.block(p.below()), "minecraft:shelf_mushroom") {
                                    place_shelf(ctx, random, c, p, facing);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn shelf_replaceable(ctx: &Ctx, pos: BlockPos) -> bool {
    ctx.is_replaceable(ctx.block(pos)) && !is_water_or_near(ctx, pos)
}

fn shelf_adjacent(ctx: &Ctx, pos: BlockPos) -> bool {
    Direction::HORIZONTAL.iter().any(|&d| ctx.is(ctx.block(pos.relative(d, 1)), "minecraft:shelf_mushroom"))
}

fn place_shelf(ctx: &mut Ctx, random: &mut WorldgenRandom, c: &mut Context, pos: BlockPos, facing: Direction) {
    let mushroom = ctx.registries().blocks.parse_state("minecraft:shelf_mushroom").expect("shelf mushroom exists");
    let age = random.next_i32_bound(2);
    let state = ctx.with(ctx.with(mushroom, "age", &age.to_string()), "facing", facing.name());
    c.set(ctx, pos, state);
}

fn moss_hanger(ctx: &mut Ctx, random: &mut WorldgenRandom, c: &mut Context, mut pos: BlockPos) {
    let moss = ctx.registries().blocks.parse_state("minecraft:pale_hanging_moss").expect("pale hanging moss exists");
    while ctx.is_empty_block(pos.below()) && random.next_f32() >= 0.5 {
        let state = ctx.with(moss, "tip", "false");
        c.set(ctx, pos, state);
        pos = pos.below();
    }
    let state = ctx.with(moss, "tip", "true");
    c.set(ctx, pos, state);
}

fn alter_circle(ctx: &mut Ctx, random: &mut WorldgenRandom, c: &mut Context, provider: &StateProvider, pos: BlockPos) {
    for xx in -2i32..=2 {
        for zz in -2i32..=2 {
            if xx.abs() != 2 || zz.abs() != 2 {
                let base = pos.offset(xx, 0, zz);
                let mut dy = 2;
                while dy >= -3 {
                    let cursor = base.offset(0, dy, 0);
                    if let Some(state) = provider.get_optional(ctx, random, cursor) {
                        c.set(ctx, cursor, state);
                        break;
                    }
                    if !ctx.is_empty_block(cursor) && dy < 0 {
                        break;
                    }
                    dy -= 1;
                }
            }
        }
    }
}
