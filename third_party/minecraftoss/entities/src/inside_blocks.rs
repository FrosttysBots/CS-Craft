//! What the blocks an entity moved through this tick do to it (pinned 26.3
//! `Entity.applyEffectsFromBlocks`/`checkInsideBlocks`,
//! `BlockGetter.forEachBlockIntersectedBetween`,
//! `InsideBlockEffectApplier.StepBasedCollector`,
//! `BlockPos.betweenCornersInDirection`, and the blocks' `entityInside` and
//! `stepOn`). Each move's box is swept from where it started to where it
//! ended; every block it meets counts once, at the step of the walk that
//! reached it. Fire, soul fire and lava set their effect types on the step
//! (applied after the walk in `InsideBlockEffectType` order, their damage
//! just after), water puts fire out, and cactus, lit campfires, sweet berry
//! bushes and cobwebs act at once. Before the walk the block underfoot is
//! stepped on (magma burns). `isInWall` is here too: a suffocating block
//! across the eyes.
use crate::movement::Body;
use glam::DVec3;
use minecraftoss_player::{Pos, World};
use std::collections::HashSet;

/// A move `Entity.move` recorded: where it began and ended, and the
/// movement it was asked for (`axisDependentOriginalMovement`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Move {
    pub from: DVec3,
    pub to: DVec3,
    pub delta: Option<DVec3>,
}

/// The damage types of the world's hazards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hazard {
    InFire,
    Lava,
    Cactus,
    Campfire,
    HotFloor,
    SweetBerryBush,
    /// Burning (`on_fire`, once a second).
    OnFire,
    /// Suffocating in a block (`in_wall`).
    InWall,
    /// Too many mobs in one place (`cramming`).
    Cramming,
}

impl Hazard {
    /// `#is_fire`: fire resistance turns it away.
    pub fn is_fire(self) -> bool {
        matches!(self, Self::InFire | Self::Lava | Self::Campfire | Self::HotFloor | Self::OnFire)
    }

    /// `#panic_causes` (through `#panic_environmental_causes`).
    pub fn panics(self) -> bool {
        matches!(self, Self::InFire | Self::Lava | Self::Cactus | Self::HotFloor | Self::OnFire)
    }

    /// `#bypasses_armor`.
    pub fn bypasses_armor(self) -> bool {
        matches!(self, Self::OnFire | Self::InWall | Self::Cramming)
    }

    /// The damage type's ID.
    pub fn damage_type(self) -> &'static str {
        match self {
            Self::InFire => "minecraft:in_fire",
            Self::Lava => "minecraft:lava",
            Self::Cactus => "minecraft:cactus",
            Self::Campfire => "minecraft:campfire",
            Self::HotFloor => "minecraft:hot_floor",
            Self::SweetBerryBush => "minecraft:sweet_berry_bush",
            Self::OnFire => "minecraft:on_fire",
            Self::InWall => "minecraft:in_wall",
            Self::Cramming => "minecraft:cramming",
        }
    }
}

/// One thing a block does to the entity, in the order it happens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlockEffect {
    /// `entity.hurt` from the block itself.
    Hurt(Hazard, f32),
    /// `makeStuckInBlock`: the next move is scaled and the motion stopped.
    Stuck(DVec3),
    /// `InsideBlockEffectType.CLEAR_FREEZE` (no freezing yet: nothing).
    ClearFreeze,
    /// `BaseFireBlock.fireIgnite`.
    FireIgnite,
    /// `Entity.lavaIgnite`: fifteen seconds of fire.
    LavaIgnite,
    /// `Entity.clearFire`.
    Extinguish,
    /// Fire's damage after igniting (`inFire`).
    FireHurt(f32),
    /// `Entity.lavaHurt`: 4 damage and a burn sound.
    LavaHurt,
}

/// `InsideBlockEffectType` in its declared (apply) order; freezing is
/// not modelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    ClearFreeze,
    FireIgnite,
    LavaIgnite,
    Extinguish,
}

const APPLY_ORDER: [Kind; 4] = [Kind::ClearFreeze, Kind::FireIgnite, Kind::LavaIgnite, Kind::Extinguish];

/// `StepBasedCollector`: the effect types of the current step and what
/// runs after them, flushed in apply order when the step changes.
#[derive(Default)]
struct Collector {
    in_step: Vec<Kind>,
    after: Vec<(Kind, BlockEffect)>,
    last_step: Option<i32>,
    out: Vec<BlockEffect>,
}

impl Collector {
    fn advance(&mut self, step: i32) {
        if self.last_step != Some(step) {
            self.last_step = Some(step);
            self.flush();
        }
    }

    fn flush(&mut self) {
        for kind in APPLY_ORDER {
            if let Some(i) = self.in_step.iter().position(|&k| k == kind) {
                self.in_step.remove(i);
                self.out.push(match kind {
                    Kind::ClearFreeze => BlockEffect::ClearFreeze,
                    Kind::FireIgnite => BlockEffect::FireIgnite,
                    Kind::LavaIgnite => BlockEffect::LavaIgnite,
                    Kind::Extinguish => BlockEffect::Extinguish,
                });
            }
            let mut i = 0;
            while i < self.after.len() {
                if self.after[i].0 == kind {
                    let (_, effect) = self.after.remove(i);
                    self.out.push(effect);
                } else {
                    i += 1;
                }
            }
        }
    }

    fn apply(&mut self, kind: Kind) {
        if !self.in_step.contains(&kind) {
            self.in_step.push(kind);
        }
    }
}

/// The entity's box at `at` (`makeBoundingBox`).
fn bounds(at: DVec3, width: f32, height: f32) -> (DVec3, DVec3) {
    let half = f64::from(width / 2.0);
    (DVec3::new(at.x - half, at.y, at.z - half), DVec3::new(at.x + half, at.y + f64::from(height), at.z + half))
}

/// `AABB.getCenter`: a halfway lerp on each axis.
fn center(min: DVec3, max: DVec3) -> DVec3 {
    DVec3::new(min.x + 0.5 * (max.x - min.x), min.y + 0.5 * (max.y - min.y), min.z + 0.5 * (max.z - min.z))
}

/// `Direction.axisStepOrder`: y first, then the smaller horizontal axis.
fn axis_step_order(v: DVec3) -> [usize; 3] {
    if v.x.abs() < v.z.abs() {
        [1, 2, 0]
    } else {
        [1, 0, 2]
    }
}

/// `BlockPos.betweenCornersInDirection`: the blocks between two corners,
/// walked from the corner the direction leaves, the axes in step order.
fn between_corners_in_direction(first: [i32; 3], second: [i32; 3], direction: DVec3) -> Vec<Pos> {
    let min = [first[0].min(second[0]), first[1].min(second[1]), first[2].min(second[2])];
    let max = [first[0].max(second[0]), first[1].max(second[1]), first[2].max(second[2])];
    let start = [0, 1, 2].map(|a| if direction[a] >= 0.0 { min[a] } else { max[a] });
    let sign = [0, 1, 2].map(|a| if direction[a] >= 0.0 { 1 } else { -1 });
    let axes = axis_step_order(direction);
    let span = axes.map(|a| max[a] - min[a]);
    let mut out = Vec::with_capacity(((span[0] + 1) * (span[1] + 1) * (span[2] + 1)) as usize);
    for i in 0..=span[0] {
        for j in 0..=span[1] {
            for k in 0..=span[2] {
                let mut p = start;
                p[axes[0]] += sign[axes[0]] * i;
                p[axes[1]] += sign[axes[1]] * j;
                p[axes[2]] += sign[axes[2]] * k;
                out.push((p[0], p[1], p[2]));
            }
        }
    }
    out
}

fn floor_corner(v: DVec3) -> [i32; 3] {
    [v.x.floor() as i32, v.y.floor() as i32, v.z.floor() as i32]
}

/// `BlockGetter.getFurthestCorner`.
fn furthest_corner(d: DVec3) -> [i32; 3] {
    let (xd, yd, zd) = (d.x.abs(), d.y.abs(), d.z.abs());
    let xs = if d.x >= 0.0 { 1 } else { -1 };
    let ys = if d.y >= 0.0 { 1 } else { -1 };
    let zs = if d.z >= 0.0 { 1 } else { -1 };
    if xd <= yd && xd <= zd {
        [-xs, -zs, ys]
    } else if yd <= zd {
        [zs, -ys, -xs]
    } else {
        [-ys, xs, -zs]
    }
}

/// `BlockGetter.forEachBlockIntersectedBetween`: every block the box
/// (`min`, `max`, where the move ended) meets swept back to its start,
/// with the step of the walk; `visit` stops the walk by returning false.
pub fn for_each_block_intersected_between(from: DVec3, to: DVec3, min: DVec3, max: DVec3, mut visit: impl FnMut(Pos, i32) -> bool) -> bool {
    let travel = to - from;
    if travel.length_squared() < f64::from(1.0e-5_f32 * 1.0e-5_f32) {
        // `BlockPos.betweenClosed(aabb)`: x fastest, then y, then z.
        let (lo, hi) = (floor_corner(min), floor_corner(max));
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    if !visit((x, y, z), 0) {
                        return false;
                    }
                }
            }
        }
        return true;
    }
    let mut visited: HashSet<Pos> = HashSet::new();
    for pos in between_corners_in_direction(floor_corner(min - travel), floor_corner(max - travel), travel) {
        if !visit(pos, 0) {
            return false;
        }
        visited.insert(pos);
    }
    let Some(iterations) = collisions_along_travel(&mut visited, travel, min, max, &mut visit) else { return false };
    for pos in between_corners_in_direction(floor_corner(min), floor_corner(max), travel) {
        if visited.insert(pos) && !visit(pos, iterations + 1) {
            return false;
        }
    }
    true
}

/// `BlockGetter.addCollisionsAlongTravel`: the blocks the box's leading
/// corner crosses, each with the far corner's reach.
fn collisions_along_travel(visited: &mut HashSet<Pos>, delta: DVec3, min: DVec3, max: DVec3, visit: &mut impl FnMut(Pos, i32) -> bool) -> Option<i32> {
    let size = max - min;
    let corner = furthest_corner(delta);
    let center = center(min, max);
    let to_corner = DVec3::new(
        center.x + size.x * 0.5 * f64::from(corner[0]),
        center.y + size.y * 0.5 * f64::from(corner[1]),
        center.z + size.z * 0.5 * f64::from(corner[2]),
    );
    let from_corner = to_corner - delta;
    let mut block = floor_corner(from_corner);
    let sign = [delta.x, delta.y, delta.z].map(|d| if d > 0.0 { 1 } else if d < 0.0 { -1 } else { 0 });
    let t_delta = [0, 1, 2].map(|a| if sign[a] == 0 { f64::MAX } else { f64::from(sign[a]) / delta[a] });
    let frac = |v: f64| v - v.floor();
    let mut t = [0, 1, 2].map(|a| t_delta[a] * if sign[a] > 0 { 1.0 - frac(from_corner[a]) } else { frac(from_corner[a]) });
    let mut iterations = 0;
    while t[0] <= 1.0 || t[1] <= 1.0 || t[2] <= 1.0 {
        let axis = if t[0] < t[1] {
            if t[0] < t[2] {
                0
            } else {
                2
            }
        } else if t[1] < t[2] {
            1
        } else {
            2
        };
        block[axis] += sign[axis];
        t[axis] += t_delta[axis];
        let (lo, hi) = (DVec3::new(block[0] as f64, block[1] as f64, block[2] as f64), DVec3::new(block[0] as f64 + 1.0, block[1] as f64 + 1.0, block[2] as f64 + 1.0));
        let Some(hit) = crate::sight::clip_box(lo, hi, from_corner, to_corner) else { continue };
        iterations += 1;
        // The lower bound is an int plus a float: float arithmetic.
        let clamp = |v: f64, a: usize| v.clamp(f64::from(block[a] as f32 + 1.0e-5_f32), block[a] as f64 + 1.0 - f64::from(1.0e-5_f32));
        let corner_hit = DVec3::new(clamp(hit.x, 0), clamp(hit.y, 1), clamp(hit.z, 2));
        let opposite = [
            (corner_hit.x - size.x * f64::from(corner[0])).floor() as i32,
            (corner_hit.y - size.y * f64::from(corner[1])).floor() as i32,
            (corner_hit.z - size.z * f64::from(corner[2])).floor() as i32,
        ];
        for pos in between_corners_in_direction(block, opposite, delta) {
            if visited.insert(pos) && !visit(pos, iterations) {
                return None;
            }
        }
    }
    Some(iterations)
}

/// `AABB.collidedAlongVector`: whether the box, swept along `travel`,
/// meets one of the boxes.
fn collided_along(min: DVec3, max: DVec3, travel: DVec3, boxes: &[(DVec3, DVec3)]) -> bool {
    let from = center(min, max);
    let to = from + travel;
    let grow = DVec3::new((max.x - min.x) * 0.5 - 1.0e-7, (max.y - min.y) * 0.5 - 1.0e-7, (max.z - min.z) * 0.5 - 1.0e-7);
    let contains = |lo: DVec3, hi: DVec3, p: DVec3| p.x >= lo.x && p.x < hi.x && p.y >= lo.y && p.y < hi.y && p.z >= lo.z && p.z < hi.z;
    boxes.iter().any(|&(lo, hi)| {
        let (lo, hi) = (lo - grow, hi + grow);
        contains(lo, hi, to) || contains(lo, hi, from) || crate::sight::clip_box(lo, hi, from, to).is_some()
    })
}

/// A fluid's box in the block (`Fluid.getAABB`): its height, a full block
/// under the same fluid. Waterlogged blocks hold a water source.
fn fluid_box(world: &impl World, pos: Pos) -> Option<(bool, DVec3, DVec3)> {
    let block = world.block(pos)?;
    let waterlogged = block.property("waterlogged") == Some("true");
    let lava = match block.id.as_str() {
        "minecraft:water" => false,
        "minecraft:lava" => true,
        _ if waterlogged => false,
        _ => return None,
    };
    let level = if waterlogged { 0 } else { block.property("level").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0) };
    let same_above = world.block((pos.0, pos.1 + 1, pos.2)).is_some_and(|above| {
        if lava {
            above.id == "minecraft:lava"
        } else {
            above.id == "minecraft:water" || above.property("waterlogged") == Some("true")
        }
    });
    let amount = if level == 0 || level >= 8 { 8 } else { 8 - level };
    let height = if same_above { 1.0 } else { amount as f32 / 9.0 };
    let lo = DVec3::new(pos.0 as f64, pos.1 as f64, pos.2 as f64);
    Some((lava, lo, DVec3::new(lo.x + 1.0, lo.y + f64::from(height), lo.z + 1.0)))
}

/// `Entity.applyEffectsFromBlocks`' walk for a living mob: the block
/// underfoot's `stepOn`, then what every block and fluid the moves passed
/// through does, in the order vanilla carries it out (immediate hurts
/// during the walk, then the collected effects). `moves` are the moves
/// this tick; with none, the entity counts as having moved from
/// `old_position` to where it is. The caller stops once the mob dies.
pub fn block_effects(world: &impl World, body: &Body, old_position: DVec3, moves: &[Move]) -> Vec<BlockEffect> {
    let mut finals: Vec<Move> = moves.to_vec();
    match finals.last() {
        None => finals.push(Move { from: old_position, to: body.position, delta: None }),
        Some(last) if last.to.distance_squared(body.position) > f64::from(9.9999994e-11_f32) => {
            let from = last.to;
            finals.push(Move { from, to: body.position, delta: None });
        }
        _ => {}
    }
    let mut now = Vec::new();
    // `stepOn` of the block under the feet (`getOnPosLegacy`): magma burns
    // a mob that does not step carefully.
    if body.on_ground && world.block(body.on_pos(world, 0.2)).is_some_and(|b| b.id == "minecraft:magma_block") {
        now.push(BlockEffect::Hurt(Hazard::HotFloor, 1.0));
    }
    let mut collector = Collector::default();
    let mut visited: HashSet<Pos> = HashSet::new();
    let moved_horizontally = {
        // `oldPosition().subtract(position())` (sweet berry bushes).
        let m = old_position - body.position;
        (m.x * m.x + m.z * m.z > 0.0).then_some(m)
    };
    for movement in &finals {
        let delta = movement.to - movement.from;
        let mut budget = 16;
        let check = |from: DVec3, to: DVec3, budget: i32, visited: &mut HashSet<Pos>, collector: &mut Collector, now: &mut Vec<BlockEffect>| -> i32 {
            let (min, max) = bounds(to, body.width, body.height);
            let d = f64::from(1.0e-5_f32);
            let (min, max) = (min + DVec3::splat(d), max - DVec3::splat(d));
            let mut used = 0;
            for_each_block_intersected_between(from, to, min, max, |pos, step| {
                if step >= budget {
                    return false;
                }
                used = step;
                let Some(block) = world.block(pos) else { return true };
                if block.id == "minecraft:air" {
                    return true;
                }
                // `getEntityInsideCollisionShape` is the whole block for all
                // the blocks here.
                let (fmin, fmax) = bounds(from, body.width, body.height);
                let fluid = fluid_box(world, pos).filter(|&(_, lo, hi)| collided_along(fmin, fmax, to - from, &[(lo, hi)]));
                if !visited.insert(pos) {
                    return true;
                }
                collector.advance(step);
                inside_block(&block, collector, now, moved_horizontally);
                if let Some((lava, _, _)) = fluid {
                    collector.advance(step);
                    if lava {
                        collector.apply(Kind::ClearFreeze);
                        collector.apply(Kind::LavaIgnite);
                        collector.after.push((Kind::LavaIgnite, BlockEffect::LavaHurt));
                    } else {
                        collector.apply(Kind::Extinguish);
                    }
                }
                true
            });
            used
        };
        match movement.delta.filter(|_| delta.length_squared() > 0.0) {
            Some(original) => {
                let mut pos = movement.from;
                for axis in axis_step_order(original) {
                    let axis_move = delta[axis];
                    if axis_move != 0.0 {
                        let mut to = pos;
                        to[axis] += axis_move;
                        budget -= check(pos, to, budget, &mut visited, &mut collector, &mut now);
                        pos = to;
                    }
                }
            }
            None => {
                budget -= check(movement.from, movement.to, 16, &mut visited, &mut collector, &mut now);
            }
        }
        if budget <= 0 {
            check(movement.to, movement.to, 1, &mut visited, &mut collector, &mut now);
        }
    }
    collector.flush();
    now.extend(collector.out);
    now
}

/// A block's `entityInside` for a living mob.
fn inside_block(block: &minecraftoss_player::Block, collector: &mut Collector, now: &mut Vec<BlockEffect>, moved: Option<DVec3>) {
    match block.id.as_str() {
        "minecraft:fire" | "minecraft:soul_fire" => {
            let damage = if block.id == "minecraft:soul_fire" { 2.0 } else { 1.0 };
            collector.apply(Kind::ClearFreeze);
            collector.apply(Kind::FireIgnite);
            collector.after.push((Kind::FireIgnite, BlockEffect::FireHurt(damage)));
        }
        "minecraft:cactus" => now.push(BlockEffect::Hurt(Hazard::Cactus, 1.0)),
        "minecraft:campfire" | "minecraft:soul_campfire" if block.property("lit") != Some("false") => {
            let damage = if block.id == "minecraft:soul_campfire" { 2.0 } else { 1.0 };
            now.push(BlockEffect::Hurt(Hazard::Campfire, damage));
        }
        "minecraft:sweet_berry_bush" => {
            now.push(BlockEffect::Stuck(DVec3::new(f64::from(0.8_f32), 0.75, f64::from(0.8_f32))));
            let grown = block.property("age").is_some_and(|age| age != "0");
            let threshold = f64::from(0.003_f32);
            if grown && moved.is_some_and(|m| m.x.abs() >= threshold || m.z.abs() >= threshold) {
                now.push(BlockEffect::Hurt(Hazard::SweetBerryBush, 1.0));
            }
        }
        "minecraft:cobweb" => now.push(BlockEffect::Stuck(DVec3::new(0.25, f64::from(0.05_f32), 0.25))),
        _ => {}
    }
}

/// `LivingEntity.isInWall`: a suffocating block's collision shape across
/// a thin square at the eyes, 0.8 of the width across.
pub fn in_wall(world: &impl World, body: &Body, eye_height: f32) -> bool {
    let half = f64::from(body.width * 0.8_f32) / 2.0;
    let eye = body.position + DVec3::new(0.0, f64::from(eye_height), 0.0);
    let (lo, hi) = (DVec3::new(eye.x - half, eye.y - 5.0e-7, eye.z - half), DVec3::new(eye.x + half, eye.y + 5.0e-7, eye.z + half));
    for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
        for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
            for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
                let pos = (x, y, z);
                if !world.block(pos).is_some_and(|b| b.id != "minecraft:air") || !world.suffocating(pos) {
                    continue;
                }
                let (bx, by, bz) = (f64::from(x), f64::from(y), f64::from(z));
                if world.collision_boxes(pos).iter().any(|b| b[0] + bx < hi.x && b[3] + bx > lo.x && b[1] + by < hi.y && b[4] + by > lo.y && b[2] + bz < hi.z && b[5] + bz > lo.z) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_walk_from_the_side_the_direction_leaves() {
        let blocks = between_corners_in_direction([0, 0, 0], [1, 0, 1], DVec3::new(-1.0, 0.0, 0.5));
        // y first (one layer), then x (the larger horizontal), then z.
        assert_eq!(blocks, vec![(1, 0, 0), (1, 0, 1), (0, 0, 0), (0, 0, 1)]);
    }

    #[test]
    fn a_still_box_meets_the_blocks_it_covers() {
        let mut met = Vec::new();
        for_each_block_intersected_between(DVec3::ZERO, DVec3::ZERO, DVec3::new(0.2, 0.0, 0.2), DVec3::new(0.8, 1.8, 0.8), |p, step| {
            met.push((p, step));
            true
        });
        assert_eq!(met, vec![((0, 0, 0), 0), ((0, 1, 0), 0)]);
    }
}
