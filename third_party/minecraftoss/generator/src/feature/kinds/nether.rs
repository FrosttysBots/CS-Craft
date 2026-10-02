//! Nether features: basalt deltas, stepped basalt column clusters and huge fungi.

use super::ore::Manhattan;
use super::{bool_or, Placeable};
use crate::feature::blocks::{parse_state, BlockSet};
use crate::feature::predicate::BlockPredicate;
use crate::feature::state::StateProvider;
use crate::feature::{Ctx, Library};
use crate::providers::{next_int_between, IntProvider};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::{BlockId, BlockPos, BlockStateId};
use serde_json::Value;
use std::sync::Arc;

pub fn parse(lib: &mut Library, kind: &str, json: &Value) -> Option<Result<Box<dyn Placeable>, String>> {
    Some(match kind {
        "delta_feature" => Delta::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "stepped_column_cluster" => ColumnCluster::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "huge_fungus" => HugeFungus::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        _ => return None,
    })
}

#[derive(Debug)]
struct Delta {
    contents: BlockStateId,
    rim: BlockStateId,
    size: IntProvider,
    rim_size: IntProvider,
    cannot_replace: Vec<BlockId>,
}

impl Delta {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let blocks = &lib.registries.blocks;
        let cannot_replace = ["bedrock", "nether_bricks", "nether_brick_fence", "nether_brick_stairs", "nether_wart", "chest", "spawner"]
            .iter()
            .map(|n| blocks.block_by_name(&format!("minecraft:{n}")).ok_or_else(|| format!("unknown block {n}")))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            contents: parse_state(&lib.registries, &json["contents"])?,
            rim: parse_state(&lib.registries, &json["rim"])?,
            size: IntProvider::parse(&json["size"])?,
            rim_size: IntProvider::parse(&json["rim_size"])?,
            cannot_replace,
        })
    }

    fn is_clear(&self, ctx: &Ctx, pos: BlockPos) -> bool {
        let blocks = &ctx.registries().blocks;
        let block = blocks.block_of(ctx.block(pos));
        if block == blocks.block_of(self.contents) || self.cannot_replace.contains(&block) {
            return false;
        }
        for d in Direction::ALL {
            let air = ctx.is_empty_block(pos.relative(d, 1));
            if air && d != Direction::Up || !air && d == Direction::Up {
                return false;
            }
        }
        true
    }
}

impl Placeable for Delta {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut any = false;
        let spawn_rim = random.next_f64() < 0.9;
        let rim_x = if spawn_rim { self.rim_size.sample(random) } else { 0 };
        let rim_z = if spawn_rim { self.rim_size.sample(random) } else { 0 };
        let has_rim = spawn_rim && rim_x != 0 && rim_z != 0;
        let rx = self.size.sample(random);
        let rz = self.size.sample(random);
        let limit = rx.max(rz);
        for pos in Manhattan::new(origin, rx, 0, rz, rx + rz) {
            if pos.dist_manhattan(origin) > limit {
                break;
            }
            if self.is_clear(ctx, pos) {
                if has_rim {
                    any = true;
                    ctx.set_block_update(pos, self.rim);
                }
                let offset = pos.offset(rim_x, 0, rim_z);
                if self.is_clear(ctx, offset) {
                    any = true;
                    ctx.set_block_update(offset, self.contents);
                }
            }
        }
        any
    }
}

#[derive(Debug)]
struct ColumnCluster {
    block: Arc<StateProvider>,
    continue_through: BlockPredicate,
    can_replace: BlockPredicate,
    cannot_place_on: BlockSet,
    cluster_reach: IntProvider,
    column_count: IntProvider,
    column_reach: IntProvider,
    height: IntProvider,
}

impl ColumnCluster {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            block: StateProvider::parse(lib, &json["block"])?,
            continue_through: BlockPredicate::parse(&lib.registries, &json["continue_through"])?,
            can_replace: BlockPredicate::parse(&lib.registries, &json["can_replace"])?,
            cannot_place_on: BlockSet::parse(&lib.registries, &json["cannot_place_on"])?,
            cluster_reach: IntProvider::parse(&json["cluster_reach"])?,
            column_count: IntProvider::parse(&json["column_count"])?,
            column_reach: IntProvider::parse(&json["column_reach"])?,
            height: IntProvider::parse(&json["height"])?,
        })
    }

    fn can_place_at(&self, ctx: &Ctx, pos: BlockPos) -> bool {
        if !self.can_replace.test(ctx, pos) {
            return false;
        }
        let below = ctx.block(pos.below());
        !ctx.is_air(below) && !self.cannot_place_on.contains(ctx.registries(), below)
    }

    fn find_surface(&self, ctx: &Ctx, mut pos: BlockPos, mut limit: i32) -> Option<BlockPos> {
        while pos.y > ctx.min_y() + 1 && limit > 0 {
            limit -= 1;
            if self.can_place_at(ctx, pos) {
                return Some(pos);
            }
            pos = pos.below();
        }
        None
    }

    fn find_air(&self, ctx: &Ctx, mut pos: BlockPos, mut limit: i32) -> Option<BlockPos> {
        while pos.y <= ctx.max_y() && limit > 0 {
            limit -= 1;
            let state = ctx.block(pos);
            if self.cannot_place_on.contains(ctx.registries(), state) {
                return None;
            }
            if ctx.is_air(state) {
                return Some(pos);
            }
            pos = pos.above();
        }
        None
    }

    fn column(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos, height: i32, reach: i32) -> bool {
        let mut any = false;
        for z in -reach..=reach {
            for x in -reach..=reach {
                let pos = origin.offset(x, 0, z);
                let step = pos.dist_manhattan(origin);
                let column = if self.can_replace.test(ctx, pos) { self.find_surface(ctx, pos, step) } else { self.find_air(ctx, pos, step) };
                let Some(mut cursor) = column else { continue };
                let mut blocks = height - step / 2;
                while blocks >= 0 {
                    if self.can_replace.test(ctx, cursor) {
                        let state = self.block.get(ctx, random, cursor);
                        ctx.set_block_update(cursor, state);
                        cursor = cursor.above();
                        any = true;
                    } else {
                        if !self.continue_through.test(ctx, cursor) {
                            break;
                        }
                        cursor = cursor.above();
                    }
                    blocks -= 1;
                }
            }
        }
        any
    }
}

impl Placeable for ColumnCluster {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !self.can_place_at(ctx, origin) {
            return false;
        }
        let height = self.height.sample(random);
        let reach = height.min(self.cluster_reach.sample(random));
        let count = self.column_count.sample(random);
        let width = 2 * reach + 1;
        let mut placed = false;
        // BlockPos.randomBetweenClosed draws lazily, interleaved with the body.
        for _ in 0..count {
            let x = origin.x - reach + random.next_i32_bound(width);
            let y = origin.y + random.next_i32_bound(1);
            let z = origin.z - reach + random.next_i32_bound(width);
            let pos = BlockPos::new(x, y, z);
            let blocks = height - pos.dist_manhattan(origin);
            if blocks >= 0 {
                let reach = self.column_reach.sample(random);
                placed |= self.column(ctx, random, pos, blocks, reach);
            }
        }
        placed
    }
}

#[derive(Debug)]
struct HugeFungus {
    valid_base: BlockId,
    stem: BlockStateId,
    hat: BlockStateId,
    decor: BlockStateId,
    replaceable: BlockPredicate,
    planted: bool,
    weeping: BlockStateId,
    weeping_plant: BlockStateId,
}

impl HugeFungus {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let r = &lib.registries;
        let base = parse_state(r, &json["valid_base_block"])?;
        Ok(Self {
            valid_base: r.blocks.block_of(base),
            stem: parse_state(r, &json["stem_state"])?,
            hat: parse_state(r, &json["hat_state"])?,
            decor: parse_state(r, &json["decor_state"])?,
            replaceable: BlockPredicate::parse(r, &json["replaceable_blocks"])?,
            planted: bool_or(json, "planted", false),
            weeping: r.blocks.parse_state("minecraft:weeping_vines")?,
            weeping_plant: r.blocks.parse_state("minecraft:weeping_vines_plant")?,
        })
    }

    fn replaceable(&self, ctx: &Ctx, pos: BlockPos, plants: bool) -> bool {
        ctx.is_replaceable(ctx.block(pos)) || plants && self.replaceable.test(ctx, pos)
    }

    fn vines(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, hat: BlockPos) {
        let mut pos = hat.below();
        if !ctx.is_empty_block(pos) {
            return;
        }
        let mut goal = next_int_between(random, 1, 5);
        if random.next_i32_bound(7) == 0 {
            goal *= 2;
        }
        for height in 0..=goal {
            if ctx.is_empty_block(pos) {
                if height == goal || !ctx.is_empty_block(pos.below()) {
                    let age = next_int_between(random, 23, 25);
                    let state = ctx.with(self.weeping, "age", &age.to_string());
                    ctx.set_block(pos, state);
                    break;
                }
                ctx.set_block(pos, self.weeping_plant);
            }
            pos = pos.below();
        }
    }

    fn hat_block(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos, decor: f32, hat: f32, vines: f32) {
        if random.next_f32() < decor {
            ctx.set_block_update(pos, self.decor);
        } else if random.next_f32() < hat {
            ctx.set_block_update(pos, self.hat);
            if random.next_f32() < vines {
                self.vines(ctx, random, pos);
            }
        }
    }

    fn hat_drop(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos, vines: bool) {
        let blocks = &ctx.registries().blocks;
        if blocks.block_of(ctx.block(pos.below())) == blocks.block_of(self.hat) {
            ctx.set_block_update(pos, self.hat);
        } else if f64::from(random.next_f32()) < 0.15 {
            ctx.set_block_update(pos, self.hat);
            if vines && random.next_i32_bound(11) == 0 {
                self.vines(ctx, random, pos);
            }
        }
    }
}

impl Placeable for HugeFungus {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if ctx.registries().blocks.block_of(ctx.block(origin.below())) != self.valid_base {
            return false;
        }
        let mut total = next_int_between(random, 4, 13);
        if random.next_i32_bound(12) == 0 {
            total *= 2;
        }
        if !self.planted && origin.y + total + 1 >= ctx.lib.generation.depth {
            return false;
        }
        let huge = !self.planted && random.next_f32() < 0.06;
        ctx.set_block_flags(origin, ctx.lib.blocks.air, 260);
        // placeStem
        let r = i32::from(huge);
        for dx in -r..=r {
            for dz in -r..=r {
                let corner = huge && dx.abs() == r && dz.abs() == r;
                for dy in 0..total {
                    let pos = origin.offset(dx, dy, dz);
                    if !self.replaceable(ctx, pos, true) {
                        continue;
                    }
                    if corner {
                        if random.next_f32() < 0.1 {
                            ctx.set_block_update(pos, self.stem);
                        }
                    } else {
                        ctx.set_block_update(pos, self.stem);
                    }
                }
            }
        }
        // placeHat
        let vines = ctx.is(self.hat, "minecraft:nether_wart_block");
        let hat_height = (random.next_i32_bound(1 + total / 3) + 5).min(total);
        let start = total - hat_height;
        for dy in start..=total {
            let mut radius = if dy < total - random.next_i32_bound(3) { 2 } else { 1 };
            if hat_height > 8 && dy < start + 4 {
                radius = 3;
            }
            if huge {
                radius += 1;
            }
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    let edge_x = dx == -radius || dx == radius;
                    let edge_z = dz == -radius || dz == radius;
                    let inside = !edge_x && !edge_z && dy != total;
                    let corner = edge_x && edge_z;
                    let bottom = dy < start + 3;
                    let pos = origin.offset(dx, dy, dz);
                    if !self.replaceable(ctx, pos, false) {
                        continue;
                    }
                    if bottom {
                        if !inside {
                            self.hat_drop(ctx, random, pos, vines);
                        }
                    } else if inside {
                        self.hat_block(ctx, random, pos, 0.1, 0.2, if vines { 0.1 } else { 0.0 });
                    } else if corner {
                        self.hat_block(ctx, random, pos, 0.01, 0.7, if vines { 0.083 } else { 0.0 });
                    } else {
                        self.hat_block(ctx, random, pos, 5.0e-4, 0.98, if vines { 0.07 } else { 0.0 });
                    }
                }
            }
        }
        true
    }
}
