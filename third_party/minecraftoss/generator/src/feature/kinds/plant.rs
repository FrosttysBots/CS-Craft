//! Plant-like features: bamboo, huge mushrooms, coral, block blobs (forest
//! rocks) and ice spikes.

use super::{float, float_or, int_or, shuffle, Placeable};
use crate::feature::blocks::parse_state;
use crate::feature::predicate::BlockPredicate;
use crate::feature::state::StateProvider;
use crate::feature::{place_placed, Ctx, Library, PlacedId};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::tags::TagId;
use minecraftoss_core::{BlockPos, BlockStateId};
use serde_json::Value;
use std::sync::Arc;

pub fn parse(lib: &mut Library, kind: &str, json: &Value) -> Option<Result<Box<dyn Placeable>, String>> {
    Some(match kind {
        "bamboo" => Bamboo::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "huge_brown_mushroom" => HugeMushroom::parse(lib, json, false).map(|f| Box::new(f) as Box<dyn Placeable>),
        "huge_red_mushroom" => HugeMushroom::parse(lib, json, true).map(|f| Box::new(f) as Box<dyn Placeable>),
        "coral_tree" => lib.placed_ref(&json["feature"]).map(|f| Box::new(CoralTree(f)) as Box<dyn Placeable>),
        "coral_claw" => lib.placed_ref(&json["feature"]).map(|f| Box::new(CoralClaw(f)) as Box<dyn Placeable>),
        "block_blob" => BlockBlob::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "spike" => Spike::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        _ => return None,
    })
}

#[derive(Debug)]
struct Bamboo {
    probability: f32,
    trunk: BlockStateId,
    final_large: BlockStateId,
    top_large: BlockStateId,
    top_small: BlockStateId,
    default: BlockStateId,
    podzol: BlockStateId,
    podzol_replaceable: TagId,
}

impl Bamboo {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let b = |s: &str| lib.registries.blocks.parse_state(s);
        Ok(Self {
            probability: float(json, "probability")?,
            trunk: b("minecraft:bamboo[age=1,leaves=none,stage=0]")?,
            final_large: b("minecraft:bamboo[age=1,leaves=large,stage=1]")?,
            top_large: b("minecraft:bamboo[age=1,leaves=large,stage=0]")?,
            top_small: b("minecraft:bamboo[age=1,leaves=small,stage=0]")?,
            default: b("minecraft:bamboo")?,
            podzol: b("minecraft:podzol")?,
            podzol_replaceable: lib.registries.block_tags.require("minecraft:beneath_bamboo_podzol_replaceable")?,
        })
    }
}

impl Placeable for Bamboo {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !ctx.is_empty_block(origin) {
            return false;
        }
        if ctx.can_survive(self.default, origin) {
            let height = random.next_i32_bound(12) + 5;
            if random.next_f32() < self.probability {
                let r = random.next_i32_bound(4) + 1;
                for xx in origin.x - r..=origin.x + r {
                    for zz in origin.z - r..=origin.z + r {
                        let (xd, zd) = (xx - origin.x, zz - origin.z);
                        if xd * xd + zd * zd <= r * r {
                            let p = BlockPos::new(xx, ctx.height(HeightmapKind::WorldSurface, xx, zz) - 1, zz);
                            if ctx.in_tag(ctx.block(p), self.podzol_replaceable) {
                                ctx.set_block(p, self.podzol);
                            }
                        }
                    }
                }
            }
            let mut pos = origin;
            let mut i = 0;
            while i < height && ctx.is_empty_block(pos) {
                ctx.set_block(pos, self.trunk);
                pos = pos.above();
                i += 1;
            }
            if pos.y - origin.y >= 3 {
                ctx.set_block(pos, self.final_large);
                ctx.set_block(pos.below(), self.top_large);
                ctx.set_block(pos.offset(0, -2, 0), self.top_small);
            }
        }
        true
    }
}

#[derive(Debug)]
struct HugeMushroom {
    red: bool,
    cap: Arc<StateProvider>,
    stem: Arc<StateProvider>,
    radius: i32,
    can_place_on: BlockPredicate,
    replaceable: TagId,
}

impl HugeMushroom {
    fn parse(lib: &mut Library, json: &Value, red: bool) -> Result<Self, String> {
        Ok(Self {
            red,
            cap: StateProvider::parse(lib, &json["cap_provider"])?,
            stem: StateProvider::parse(lib, &json["stem_provider"])?,
            radius: int_or(json, "foliage_radius", 2),
            can_place_on: BlockPredicate::parse(&lib.registries, &json["can_place_on"])?,
            replaceable: lib.registries.block_tags.require("minecraft:replaceable_by_mushrooms")?,
        })
    }

    fn place_block(&self, ctx: &mut Ctx, pos: BlockPos, state: BlockStateId) {
        let current = ctx.block(pos);
        if ctx.is_air(current) || ctx.in_tag(current, self.replaceable) {
            ctx.set_block_update(pos, state);
        }
    }

    /// `getTreeRadiusForHeight(-1, -1, radius, dy)`: vanilla passes -1 as the
    /// tree height, so red mushrooms never check a radius.
    fn radius_for(&self, dy: i32) -> i32 {
        if self.red {
            0
        } else if dy <= 3 {
            0
        } else {
            self.radius
        }
    }

    fn sides(ctx: &Ctx, state: BlockStateId, values: &[(&str, bool)]) -> BlockStateId {
        if values.iter().all(|(k, _)| ctx.property(state, k).is_some()) {
            values.iter().fold(state, |s, (k, v)| ctx.with(s, k, if *v { "true" } else { "false" }))
        } else {
            state
        }
    }
}

impl Placeable for HugeMushroom {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut height = random.next_i32_bound(3) + 4;
        if random.next_i32_bound(12) == 0 {
            height *= 2;
        }
        // isValidPosition
        if origin.y < ctx.min_y() + 1 || origin.y + height + 1 > ctx.max_y() {
            return false;
        }
        if !self.can_place_on.test(ctx, origin.below()) {
            return false;
        }
        for dy in 0..=height {
            let r = self.radius_for(dy);
            for dx in -r..=r {
                for dz in -r..=r {
                    let s = ctx.block(origin.offset(dx, dy, dz));
                    if !ctx.is_air(s) && !ctx.in_tag(s, ctx.lib.tags.leaves) {
                        return false;
                    }
                }
            }
        }
        // makeCap
        let r = self.radius;
        if self.red {
            for dy in height - 3..=height {
                let radius = if dy < height { r } else { r - 1 };
                let center = r - 2;
                for dx in -radius..=radius {
                    for dz in -radius..=radius {
                        let x_edge = dx == -radius || dx == radius;
                        let z_edge = dz == -radius || dz == radius;
                        if dy >= height || x_edge != z_edge {
                            let state = self.cap.get(ctx, random, origin);
                            let state = Self::sides(
                                ctx,
                                state,
                                &[("up", dy >= height - 1), ("west", dx < -center), ("east", dx > center), ("north", dz < -center), ("south", dz > center)],
                            );
                            self.place_block(ctx, origin.offset(dx, dy, dz), state);
                        }
                    }
                }
            }
        } else {
            for dx in -r..=r {
                for dz in -r..=r {
                    let (min_x, max_x, min_z, max_z) = (dx == -r, dx == r, dz == -r, dz == r);
                    let (x_edge, z_edge) = (min_x || max_x, min_z || max_z);
                    if x_edge && z_edge {
                        continue;
                    }
                    let west = min_x || z_edge && dx == 1 - r;
                    let east = max_x || z_edge && dx == r - 1;
                    let north = min_z || x_edge && dz == 1 - r;
                    let south = max_z || x_edge && dz == r - 1;
                    let state = self.cap.get(ctx, random, origin);
                    let state = Self::sides(ctx, state, &[("west", west), ("east", east), ("north", north), ("south", south)]);
                    self.place_block(ctx, origin.offset(dx, height, dz), state);
                }
            }
        }
        // placeTrunk
        for dy in 0..height {
            let state = self.stem.get(ctx, random, origin);
            self.place_block(ctx, origin.offset(0, dy, 0), state);
        }
        true
    }
}

#[derive(Debug)]
struct CoralTree(PlacedId);

impl Placeable for CoralTree {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut pos = origin;
        let trunk = random.next_i32_bound(3) + 1;
        for _ in 0..trunk {
            if !place_placed(ctx, random, self.0, pos) {
                return true;
            }
            pos = pos.above();
        }
        let top = pos;
        let branches = random.next_i32_bound(3) + 2;
        let mut directions = Direction::HORIZONTAL;
        shuffle(&mut directions, random);
        for &direction in &directions[..branches as usize] {
            let mut pos = top.relative(direction, 1);
            let height = random.next_i32_bound(5) + 2;
            let mut segment = 0;
            let mut j = 0;
            while j < height && place_placed(ctx, random, self.0, pos) {
                segment += 1;
                pos = pos.above();
                if j == 0 || segment >= 2 && random.next_f32() < 0.25 {
                    pos = pos.relative(direction, 1);
                    segment = 0;
                }
                j += 1;
            }
        }
        true
    }
}

#[derive(Debug)]
struct CoralClaw(PlacedId);

impl Placeable for CoralClaw {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !place_placed(ctx, random, self.0, origin) {
            return false;
        }
        let claw = Direction::HORIZONTAL[random.next_i32_bound(4) as usize];
        let branches = random.next_i32_bound(2) + 2;
        let mut possible = [claw, claw.clockwise(), claw.counter_clockwise()];
        shuffle(&mut possible, random);
        for &branch in &possible[..branches as usize] {
            let mut pos = origin;
            let sideways = random.next_i32_bound(2) + 1;
            pos = pos.relative(branch, 1);
            let (segment, inway) = if branch == claw {
                (claw, random.next_i32_bound(3) + 2)
            } else {
                pos = pos.above();
                let segment = [branch, Direction::Up][random.next_i32_bound(2) as usize];
                (segment, random.next_i32_bound(3) + 3)
            };
            let mut i = 0;
            while i < sideways && place_placed(ctx, random, self.0, pos) {
                pos = pos.relative(segment, 1);
                i += 1;
            }
            pos = pos.relative(segment.opposite(), 1).above();
            for _ in 0..inway {
                pos = pos.relative(claw, 1);
                if !place_placed(ctx, random, self.0, pos) {
                    break;
                }
                if random.next_f32() < 0.25 {
                    pos = pos.above();
                }
            }
        }
        true
    }
}

#[derive(Debug)]
struct BlockBlob {
    state: BlockStateId,
    can_place_on: BlockPredicate,
}

impl BlockBlob {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self { state: parse_state(&lib.registries, &json["state"])?, can_place_on: BlockPredicate::parse(&lib.registries, &json["can_place_on"])? })
    }
}

impl Placeable for BlockBlob {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut origin = origin;
        while origin.y > ctx.min_y() + 3 && !self.can_place_on.test(ctx, origin.below()) {
            origin = origin.below();
        }
        if origin.y <= ctx.min_y() + 3 {
            return false;
        }
        for _ in 0..3 {
            let xr = random.next_i32_bound(2);
            let yr = random.next_i32_bound(2);
            let zr = random.next_i32_bound(2);
            let tr = (xr + yr + zr) as f32 * 0.333 + 0.5;
            for z in -zr..=zr {
                for y in -yr..=yr {
                    for x in -xr..=xr {
                        let p = origin.offset(x, y, z);
                        if p.dist_sqr(origin) <= f64::from(tr * tr) {
                            ctx.set_block_update(p, self.state);
                        }
                    }
                }
            }
            origin = origin.offset(-1 + random.next_i32_bound(2), -random.next_i32_bound(2), -1 + random.next_i32_bound(2));
        }
        true
    }
}

#[derive(Debug)]
struct Spike {
    state: BlockStateId,
    can_place_on: BlockPredicate,
    can_replace: BlockPredicate,
}

impl Spike {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            state: parse_state(&lib.registries, &json["state"])?,
            can_place_on: BlockPredicate::parse(&lib.registries, &json["can_place_on"])?,
            can_replace: BlockPredicate::parse(&lib.registries, &json["can_replace"])?,
        })
    }
}

impl Placeable for Spike {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut origin = origin;
        while ctx.is_empty_block(origin) && origin.y > ctx.min_y() + 2 {
            origin = origin.below();
        }
        if !self.can_place_on.test(ctx, origin) {
            return false;
        }
        origin = origin.offset(0, random.next_i32_bound(4), 0);
        let height = random.next_i32_bound(4) + 7;
        let width = height / 4 + random.next_i32_bound(2);
        if width > 1 && random.next_i32_bound(60) == 0 {
            origin = origin.offset(0, 10 + random.next_i32_bound(30), 0);
        }
        for y_off in 0..height {
            let scale = (1.0 - y_off as f32 / height as f32) * width as f32;
            let new_width = f64::from(scale).ceil() as i32;
            for xo in -new_width..=new_width {
                let dx = xo.abs() as f32 - 0.25;
                for zo in -new_width..=new_width {
                    let dz = zo.abs() as f32 - 0.25;
                    let inside = xo == 0 && zo == 0 || dx * dx + dz * dz <= scale * scale;
                    let on_edge = xo == -new_width || xo == new_width || zo == -new_width || zo == new_width;
                    if inside && (!on_edge || random.next_f32() <= 0.75) {
                        let up = origin.offset(xo, y_off, zo);
                        if ctx.is_empty_block(up) || self.can_replace.test(ctx, up) {
                            ctx.set_block_update(up, self.state);
                        }
                        if y_off != 0 && new_width > 1 {
                            let down = origin.offset(xo, -y_off, zo);
                            if ctx.is_empty_block(down) || self.can_replace.test(ctx, down) {
                                ctx.set_block_update(down, self.state);
                            }
                        }
                    }
                }
            }
        }
        let pillar = (width - 1).clamp(0, 1);
        for xo in -pillar..=pillar {
            for zo in -pillar..=pillar {
                let mut cursor = origin.offset(xo, -1, zo);
                let mut run = if xo.abs() == 1 && zo.abs() == 1 { random.next_i32_bound(5) } else { 50 };
                while cursor.y > 50 {
                    let s = ctx.block(cursor);
                    if !ctx.is_air(s) && !self.can_replace.test(ctx, cursor) && s != self.state {
                        break;
                    }
                    ctx.set_block_update(cursor, self.state);
                    cursor = cursor.below();
                    run -= 1;
                    if run <= 0 {
                        cursor = cursor.offset(0, -(random.next_i32_bound(5) + 1), 0);
                        run = random.next_i32_bound(5);
                    }
                }
            }
        }
        true
    }
}

#[allow(dead_code)]
fn unused(json: &Value) -> f32 {
    float_or(json, "", 0.0)
}
