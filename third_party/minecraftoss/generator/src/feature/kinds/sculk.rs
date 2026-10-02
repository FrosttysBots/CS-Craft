//! `SculkPatchFeature` with the world-generation `SculkSpreader`.

use super::multiface::{Spreader, DEFAULT_ORDER};
use super::{int, shuffle, Placeable};
use crate::feature::blocks::FluidType;
use crate::feature::kinds::simple::can_attach_to;
use crate::feature::{Ctx, Library};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::tags::TagId;
use minecraftoss_core::{BlockId, BlockPos, BlockStateId};
use serde_json::Value;

pub fn parse(lib: &mut Library, kind: &str, json: &Value) -> Option<Result<Box<dyn Placeable>, String>> {
    match kind {
        "sculk_patch" => Some(SculkPatch::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>)),
        _ => None,
    }
}

// `SculkSpreader.createWorldGenSpreader`.
const GROWTH_SPAWN_COST: i32 = 50;
const NO_GROWTH_RADIUS: i32 = 1;
const CHARGE_DECAY_RATE: i32 = 5;
const ADDITIONAL_DECAY_RATE: i32 = 10;

#[derive(Debug)]
struct SculkPatch {
    charge_count: i32,
    amount_per_charge: i32,
    spread_attempts: i32,
    growth_rounds: i32,
    spread_rounds: i32,
    sculk: BlockStateId,
    sculk_block: BlockId,
    vein: BlockId,
    sensor: BlockStateId,
    shrieker: BlockStateId,
    replaceable_world_gen: TagId,
    replaceable: TagId,
    growth_inhibitors: TagId,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    Sculk,
    Vein,
    Default,
}

struct Cursor {
    pos: BlockPos,
    charge: i32,
    update_delay: i32,
    decay_delay: i32,
    /// `MultifaceBlock.availableFaces` in `Direction` order, or `None`.
    facings: Option<Vec<Direction>>,
}

/// `ChargeCursor.NON_CORNER_NEIGHBOURS` in `betweenClosed` order.
fn non_corner_neighbours() -> Vec<(i32, i32, i32)> {
    let mut out = Vec::with_capacity(18);
    for z in -1..=1 {
        for y in -1..=1 {
            for x in -1..=1 {
                if (x == 0 || y == 0 || z == 0) && (x, y, z) != (0, 0, 0) {
                    out.push((x, y, z));
                }
            }
        }
    }
    out
}

impl SculkPatch {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let r = &lib.registries;
        let s = |n: &str| r.blocks.parse_state(n);
        Ok(Self {
            charge_count: int(json, "charge_count")?,
            amount_per_charge: int(json, "amount_per_charge")?,
            spread_attempts: int(json, "spread_attempts")?,
            growth_rounds: int(json, "growth_rounds")?,
            spread_rounds: int(json, "spread_rounds")?,
            sculk: s("minecraft:sculk")?,
            sculk_block: r.blocks.block_by_name("minecraft:sculk").ok_or("no sculk")?,
            vein: r.blocks.block_by_name("minecraft:sculk_vein").ok_or("no sculk vein")?,
            sensor: s("minecraft:sculk_sensor")?,
            shrieker: s("minecraft:sculk_shrieker[can_summon=true]")?,
            replaceable_world_gen: r.block_tags.require("minecraft:sculk_replaceable_world_gen")?,
            replaceable: r.block_tags.require("minecraft:sculk_replaceable")?,
            growth_inhibitors: r.block_tags.require("minecraft:sculk_growth_inhibitors")?,
        })
    }

    fn vein_spreader(&self) -> Spreader {
        Spreader { block: self.vein, sculk: true, types: DEFAULT_ORDER }
    }

    fn same_space_spreader(&self) -> Spreader {
        Spreader { block: self.vein, sculk: true, types: &[super::multiface::SpreadType::SamePosition] }
    }

    fn behaviour(&self, ctx: &Ctx, state: BlockStateId) -> Behaviour {
        let block = ctx.registries().blocks.block_of(state);
        if block == self.sculk_block {
            Behaviour::Sculk
        } else if block == self.vein {
            Behaviour::Vein
        } else {
            Behaviour::Default
        }
    }

    fn faces(ctx: &Ctx, state: BlockStateId) -> Vec<Direction> {
        Direction::ALL.into_iter().filter(|&d| Spreader::has_face(ctx, state, d)).collect()
    }

    fn can_spread_from(&self, ctx: &Ctx, origin: BlockPos) -> bool {
        let start = ctx.block(origin);
        if self.behaviour(ctx, start) != Behaviour::Default {
            return true;
        }
        if !ctx.is_air(start) && !(start == ctx.lib.blocks.water) {
            return false;
        }
        Direction::ALL.iter().any(|&d| ctx.registries().blocks.state(ctx.block(origin.relative(d, 1))).collision_full_block)
    }

    fn attempt_spread_vein(&self, ctx: &mut Ctx, behaviour: Behaviour, pos: BlockPos, state: BlockStateId, facings: &Option<Vec<Direction>>) -> bool {
        match behaviour {
            Behaviour::Sculk | Behaviour::Vein => self.vein_spreader().spread_all(ctx, state, pos, true) > 0,
            Behaviour::Default => match facings {
                None => {
                    let current = ctx.block(pos);
                    self.same_space_spreader().spread_all(ctx, current, pos, true) > 0
                }
                Some(faces) if !faces.is_empty() => {
                    if !ctx.is_air(state) && ctx.fluid(state) != FluidType::Water {
                        return false;
                    }
                    self.regrow(ctx, pos, state, faces)
                }
                Some(_) => self.vein_spreader().spread_all(ctx, state, pos, true) > 0,
            },
        }
    }

    /// `SculkVeinBlock.regrow`.
    fn regrow(&self, ctx: &mut Ctx, pos: BlockPos, existing: BlockStateId, faces: &[Direction]) -> bool {
        let mut state = ctx.registries().blocks.block(self.vein).default_state();
        let mut any = false;
        for &face in faces {
            if can_attach_to(ctx, face, ctx.block(pos.relative(face, 1))) {
                state = ctx.with(state, face.name(), "true");
                any = true;
            }
        }
        if !any {
            return false;
        }
        if ctx.fluid(existing) != FluidType::Empty {
            state = ctx.with(state, "waterlogged", "true");
        }
        ctx.set_block_update(pos, state);
        true
    }

    /// `SculkVeinBlock.onDischarged`.
    fn vein_discharged(&self, ctx: &mut Ctx, mut state: BlockStateId, pos: BlockPos) {
        if ctx.registries().blocks.block_of(state) != self.vein {
            return;
        }
        for dir in Direction::ALL {
            if Spreader::has_face(ctx, state, dir) && ctx.registries().blocks.block_of(ctx.block(pos.relative(dir, 1))) == self.sculk_block {
                state = ctx.with(state, dir.name(), "false");
            }
        }
        if Self::faces(ctx, state).is_empty() {
            state = if ctx.fluid_at(pos) == FluidType::Empty { ctx.lib.blocks.air } else { ctx.lib.blocks.water };
        }
        ctx.set_block_update(pos, state);
    }

    fn discharged(&self, ctx: &mut Ctx, behaviour: Behaviour, state: BlockStateId, pos: BlockPos) {
        if behaviour == Behaviour::Vein {
            self.vein_discharged(ctx, state, pos);
        }
    }

    fn use_charge(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, behaviour: Behaviour, cursor: &Cursor, origin: BlockPos, spread_veins: bool) -> i32 {
        match behaviour {
            Behaviour::Default => {
                if cursor.decay_delay > 0 {
                    cursor.charge
                } else {
                    0
                }
            }
            Behaviour::Vein => {
                if spread_veins && self.place_sculk(ctx, random, cursor.pos) {
                    cursor.charge - 1
                } else if random.next_i32_bound(CHARGE_DECAY_RATE) == 0 {
                    (cursor.charge as f32 * 0.5).floor() as i32
                } else {
                    cursor.charge
                }
            }
            Behaviour::Sculk => {
                let charge = cursor.charge;
                if charge == 0 || random.next_i32_bound(CHARGE_DECAY_RATE) != 0 {
                    return charge;
                }
                let pos = cursor.pos;
                let close = pos.dist_sqr(origin) < f64::from(NO_GROWTH_RADIUS * NO_GROWTH_RADIUS);
                if !close && self.can_place_growth(ctx, pos) {
                    if random.next_i32_bound(GROWTH_SPAWN_COST) < charge {
                        let growth = pos.above();
                        let mut state = if random.next_i32_bound(11) == 0 { self.shrieker } else { self.sensor };
                        if ctx.property(state, "waterlogged").is_some() && ctx.fluid_at(growth) != FluidType::Empty {
                            state = ctx.with(state, "waterlogged", "true");
                        }
                        ctx.set_block_update(growth, state);
                    }
                    (charge - GROWTH_SPAWN_COST).max(0)
                } else if random.next_i32_bound(ADDITIONAL_DECAY_RATE) != 0 {
                    charge
                } else {
                    charge - if close { 1 } else { decay_penalty(pos, origin, charge) }
                }
            }
        }
    }

    fn can_place_growth(&self, ctx: &Ctx, pos: BlockPos) -> bool {
        let above = ctx.block(pos.above());
        if !(ctx.is_air(above) || above == ctx.lib.blocks.water) {
            return false;
        }
        let mut count = 0;
        for y in 0..=2 {
            for z in -4..=4 {
                for x in -4..=4 {
                    if ctx.in_tag(ctx.block(pos.offset(x, y, z)), self.growth_inhibitors) {
                        count += 1;
                        if count > 2 {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    /// `SculkVeinBlock.attemptPlaceSculk`.
    fn place_sculk(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos) -> bool {
        let state = ctx.block(pos);
        let mut supports = Direction::ALL;
        shuffle(&mut supports, random);
        for support in supports {
            if !Spreader::has_face(ctx, state, support) {
                continue;
            }
            let support_pos = pos.relative(support, 1);
            if !ctx.in_tag(ctx.block(support_pos), self.replaceable_world_gen) {
                continue;
            }
            ctx.set_block_update(support_pos, self.sculk);
            self.vein_spreader().spread_all(ctx, self.sculk, support_pos, true);
            let skip = support.opposite();
            for dir in Direction::ALL {
                if dir == skip {
                    continue;
                }
                let vein_pos = support_pos.relative(dir, 1);
                let vein = ctx.block(vein_pos);
                if ctx.registries().blocks.block_of(vein) == self.vein {
                    self.vein_discharged(ctx, vein, vein_pos);
                }
            }
            return true;
        }
        false
    }

    fn has_substrate_access(&self, ctx: &Ctx, state: BlockStateId, pos: BlockPos) -> bool {
        if ctx.registries().blocks.block_of(state) != self.vein {
            return false;
        }
        Direction::ALL.iter().any(|&d| Spreader::has_face(ctx, state, d) && ctx.in_tag(ctx.block(pos.relative(d, 1)), self.replaceable))
    }

    fn movement_unobstructed(ctx: &Ctx, from: BlockPos, to: BlockPos) -> bool {
        if from.dist_manhattan(to) == 1 {
            return true;
        }
        let (dx, dy, dz) = (to.x - from.x, to.y - from.y, to.z - from.z);
        let dir = |axis, neg: bool| Direction::from_axis(axis, !neg);
        let x = dir(minecraftoss_core::pos::Axis::X, dx < 0);
        let y = dir(minecraftoss_core::pos::Axis::Y, dy < 0);
        let z = dir(minecraftoss_core::pos::Axis::Z, dz < 0);
        let open = |d: Direction| {
            let test = from.relative(d, 1);
            !ctx.registries().blocks.is_face_sturdy(ctx.block(test), d.opposite(), minecraftoss_core::SupportType::Full)
        };
        if dx == 0 {
            open(y) || open(z)
        } else if dy == 0 {
            open(x) || open(z)
        } else {
            open(x) || open(y)
        }
    }

    fn movement_pos(&self, ctx: &Ctx, random: &mut WorldgenRandom, pos: BlockPos, origin: BlockPos) -> Option<BlockPos> {
        let mut offsets = non_corner_neighbours();
        shuffle(&mut offsets, random);
        let mut found = pos;
        for (x, y, z) in offsets {
            let neighbour = pos.offset(x, y, z);
            let d2 = (origin.x - neighbour.x).pow(2) + (origin.z - neighbour.z).pow(2);
            if d2 > 144 {
                continue;
            }
            let state = ctx.block(neighbour);
            if self.behaviour(ctx, state) != Behaviour::Default && Self::movement_unobstructed(ctx, pos, neighbour) {
                found = neighbour;
                if self.has_substrate_access(ctx, state, neighbour) {
                    break;
                }
            }
        }
        (found != pos).then_some(found)
    }

    /// `ChargeCursor.update` for the world-generation spreader.
    fn update(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, cursor: &mut Cursor, origin: BlockPos, spread_veins: bool) {
        if cursor.charge <= 0 {
            return;
        }
        if cursor.update_delay > 0 {
            cursor.update_delay -= 1;
            return;
        }
        let mut state = ctx.block(cursor.pos);
        let mut behaviour = self.behaviour(ctx, state);
        if spread_veins && self.attempt_spread_vein(ctx, behaviour, cursor.pos, state, &cursor.facings) && behaviour != Behaviour::Sculk {
            state = ctx.block(cursor.pos);
            behaviour = self.behaviour(ctx, state);
        }
        cursor.charge = self.use_charge(ctx, random, behaviour, cursor, origin, spread_veins);
        if cursor.charge <= 0 {
            self.discharged(ctx, behaviour, state, cursor.pos);
            return;
        }
        match self.movement_pos(ctx, random, cursor.pos, origin) {
            Some(transfer) => {
                self.discharged(ctx, behaviour, state, cursor.pos);
                cursor.pos = transfer;
                state = ctx.block(transfer);
            }
            None => {
                self.discharged(ctx, behaviour, state, cursor.pos);
                cursor.charge = 0;
                return;
            }
        }
        if self.behaviour(ctx, state) != Behaviour::Default {
            cursor.facings = Some(if ctx.registries().blocks.block_of(state) == self.vein { Self::faces(ctx, state) } else { Vec::new() });
        }
        cursor.decay_delay = if behaviour == Behaviour::Default { (cursor.decay_delay - 1).max(0) } else { 1 };
        cursor.update_delay = 1;
    }
}

/// `SculkBlock.getDecayPenalty` for the world-generation spreader.
fn decay_penalty(pos: BlockPos, origin: BlockPos, charge: i32) -> i32 {
    let outer = (pos.dist_sqr(origin).sqrt() as f32 - NO_GROWTH_RADIUS as f32).powi(2);
    let max_reach = ((24 - NO_GROWTH_RADIUS) * (24 - NO_GROWTH_RADIUS)) as f32;
    let factor = 1.0f32.min(outer / max_reach);
    1.max((charge as f32 * factor * 0.5) as i32)
}

impl Placeable for SculkPatch {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !self.can_spread_from(ctx, origin) {
            return false;
        }
        let mut cursors: Vec<Cursor> = Vec::new();
        for round in 0..self.spread_rounds + self.growth_rounds {
            for _ in 0..self.charge_count {
                let mut charge = self.amount_per_charge;
                while charge > 0 {
                    let current = charge.min(1000);
                    if cursors.len() < 32 {
                        cursors.push(Cursor { pos: origin, charge: current, update_delay: 0, decay_delay: 1, facings: None });
                    }
                    charge -= current;
                }
            }
            let spread_veins = round < self.spread_rounds;
            for _ in 0..self.spread_attempts {
                if cursors.is_empty() {
                    continue;
                }
                let mut kept = Vec::with_capacity(cursors.len());
                for mut cursor in cursors.drain(..) {
                    let far = (cursor.pos.x - origin.x).abs().max((cursor.pos.y - origin.y).abs()).max((cursor.pos.z - origin.z).abs()) > 1024;
                    if far {
                        continue;
                    }
                    self.update(ctx, random, &mut cursor, origin, spread_veins);
                    if cursor.charge > 0 {
                        kept.push(cursor);
                    }
                }
                cursors = kept;
            }
            cursors.clear();
        }
        true
    }
}
