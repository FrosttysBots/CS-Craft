//! Rays through blocks, as 26.3 `BlockGetter.clip` casts them with
//! `ClipContext.Block.COLLIDER` and `Fluid.NONE`: `traverseBlocks` walks the
//! blocks along the segment, and each block's collision shape is clipped by
//! `VoxelShape.clip` (a start inside the shape hits at once) and
//! `AABB.clip`. On top: `LivingEntity.hasLineOfSight` (eye to eye) and
//! `ServerExplosion.getSeenPercent`.
use glam::DVec3;
use minecraftoss_player::World;

type Pos = (i32, i32, i32);

/// `Mth.lerp`.
fn lerp(delta: f64, a: f64, b: f64) -> f64 {
    a + delta * (b - a)
}

/// `Mth.frac`.
fn frac(v: f64) -> f64 {
    v - v.floor()
}

/// `AABB.clipPoint` on one face plane: the ray fraction if it enters the
/// face, closer than the best so far.
#[allow(clippy::too_many_arguments)]
fn clip_point(best: &mut f64, da: f64, db: f64, dc: f64, point: f64, min_b: f64, max_b: f64, min_c: f64, max_c: f64, origin_a: f64, origin_b: f64, origin_c: f64) -> bool {
    let s = (point - origin_a) / da;
    let pb = origin_b + s * db;
    let pc = origin_c + s * dc;
    if 0.0 < s && s < *best && min_b - 1.0e-7 < pb && pb < max_b + 1.0e-7 && min_c - 1.0e-7 < pc && pc < max_c + 1.0e-7 {
        *best = s;
        true
    } else {
        false
    }
}

/// `AABB.clip` over a shape's boxes (world coordinates): the share of the
/// segment at which it first enters one.
fn clip_boxes(boxes: &[[f64; 6]], from: DVec3, to: DVec3) -> Option<f64> {
    let d = to - from;
    let mut best = 1.0;
    let mut hit = false;
    for b in boxes {
        if d.x > 1.0e-7 {
            hit |= clip_point(&mut best, d.x, d.y, d.z, b[0], b[1], b[4], b[2], b[5], from.x, from.y, from.z);
        } else if d.x < -1.0e-7 {
            hit |= clip_point(&mut best, d.x, d.y, d.z, b[3], b[1], b[4], b[2], b[5], from.x, from.y, from.z);
        }
        if d.y > 1.0e-7 {
            hit |= clip_point(&mut best, d.y, d.z, d.x, b[1], b[2], b[5], b[0], b[3], from.y, from.z, from.x);
        } else if d.y < -1.0e-7 {
            hit |= clip_point(&mut best, d.y, d.z, d.x, b[4], b[2], b[5], b[0], b[3], from.y, from.z, from.x);
        }
        if d.z > 1.0e-7 {
            hit |= clip_point(&mut best, d.z, d.x, d.y, b[2], b[0], b[3], b[1], b[4], from.z, from.x, from.y);
        } else if d.z < -1.0e-7 {
            hit |= clip_point(&mut best, d.z, d.x, d.y, b[5], b[0], b[3], b[1], b[4], from.z, from.x, from.y);
        }
    }
    hit.then_some(best)
}

/// `AABB.clip(from, to)` for one box: where the segment enters it (none
/// when it starts inside).
pub fn clip_box(min: DVec3, max: DVec3, from: DVec3, to: DVec3) -> Option<DVec3> {
    let scale = clip_boxes(&[[min.x, min.y, min.z, max.x, max.y, max.z]], from, to)?;
    let d = to - from;
    Some(DVec3::new(from.x + scale * d.x, from.y + scale * d.y, from.z + scale * d.z))
}

/// `VoxelShape.clip` for the block at `pos`: a point just past the start
/// inside the shape hits there; otherwise the boxes are clipped. Returns
/// where it hits (`BlockHitResult.getLocation`).
fn clip_block(world: &dyn World, pos: Pos, from: DVec3, to: DVec3) -> Option<DVec3> {
    let local = world.collision_boxes(pos);
    if local.is_empty() {
        return None;
    }
    let d = to - from;
    if d.length_squared() < 1.0e-7 {
        return None;
    }
    let (ox, oy, oz) = (f64::from(pos.0), f64::from(pos.1), f64::from(pos.2));
    let boxes: Vec<[f64; 6]> = local.iter().map(|b| [b[0] + ox, b[1] + oy, b[2] + oz, b[3] + ox, b[4] + oy, b[5] + oz]).collect();
    // `from.add(diff.scale(0.001))`, tested against the shape in block
    // coordinates.
    let test = DVec3::new(from.x + d.x * 0.001, from.y + d.y * 0.001, from.z + d.z * 0.001);
    let (lx, ly, lz) = (test.x - ox, test.y - oy, test.z - oz);
    if local.iter().any(|b| b[0] <= lx && lx < b[3] && b[1] <= ly && ly < b[4] && b[2] <= lz && lz < b[5]) {
        return Some(test);
    }
    let scale = clip_boxes(&boxes, from, to)?;
    Some(DVec3::new(from.x + scale * d.x, from.y + scale * d.y, from.z + scale * d.z))
}

/// `BlockGetter.clip` with collider shapes and no fluids: whether a block
/// stops the segment.
pub fn blocked(world: &dyn World, from: DVec3, to: DVec3) -> bool {
    first_hit(world, from, to).is_some()
}

/// `BlockGetter.clip` with collider shapes and no fluids: the first block
/// that stops the segment (`BlockHitResult.getBlockPos` of a hit).
pub fn first_hit(world: &dyn World, from: DVec3, to: DVec3) -> Option<Pos> {
    clip(world, from, to).map(|(pos, _)| pos)
}

/// `BlockGetter.clip` with collider shapes and no fluids: the first block
/// that stops the segment and where it is struck.
pub fn clip(world: &dyn World, from: DVec3, to: DVec3) -> Option<(Pos, DVec3)> {
    if from == to {
        return None;
    }
    // `traverseBlocks`: both ends pulled a hair inwards.
    let to_x = lerp(-1.0e-7, to.x, from.x);
    let to_y = lerp(-1.0e-7, to.y, from.y);
    let to_z = lerp(-1.0e-7, to.z, from.z);
    let from_x = lerp(-1.0e-7, from.x, to.x);
    let from_y = lerp(-1.0e-7, from.y, to.y);
    let from_z = lerp(-1.0e-7, from.z, to.z);
    let (mut bx, mut by, mut bz) = (from_x.floor() as i32, from_y.floor() as i32, from_z.floor() as i32);
    if let Some(at) = clip_block(world, (bx, by, bz), from, to) {
        return Some(((bx, by, bz), at));
    }
    let (dx, dy, dz) = (to_x - from_x, to_y - from_y, to_z - from_z);
    let sign = |v: f64| if v > 0.0 { 1 } else if v < 0.0 { -1 } else { 0 };
    let (sx, sy, sz) = (sign(dx), sign(dy), sign(dz));
    let delta = |s: i32, d: f64| if s == 0 { f64::MAX } else { f64::from(s) / d };
    let (tdx, tdy, tdz) = (delta(sx, dx), delta(sy, dy), delta(sz, dz));
    let start = |s: i32, v: f64, t: f64| t * if s > 0 { 1.0 - frac(v) } else { frac(v) };
    let (mut tx, mut ty, mut tz) = (start(sx, from_x, tdx), start(sy, from_y, tdy), start(sz, from_z, tdz));
    while tx <= 1.0 || ty <= 1.0 || tz <= 1.0 {
        if tx < ty {
            if tx < tz {
                bx += sx;
                tx += tdx;
            } else {
                bz += sz;
                tz += tdz;
            }
        } else if ty < tz {
            by += sy;
            ty += tdy;
        } else {
            bz += sz;
            tz += tdz;
        }
        if let Some(at) = clip_block(world, (bx, by, bz), from, to) {
            return Some(((bx, by, bz), at));
        }
    }
    None
}

/// `LivingEntity.hasLineOfSight`: eye to eye, within 128 blocks, with no
/// block in the way.
pub fn line_of_sight(world: &dyn World, from_eye: DVec3, to_eye: DVec3) -> bool {
    to_eye.distance(from_eye) <= 128.0 && !blocked(world, from_eye, to_eye)
}

/// `ServerExplosion.getSeenPercent`: the share of a grid of points over
/// the box (`min`, `max`) with a clear ray to the explosion's centre.
pub fn seen_percent(world: &dyn World, center: DVec3, min: DVec3, max: DVec3) -> f32 {
    let xs = 1.0 / ((max.x - min.x) * 2.0 + 1.0);
    let ys = 1.0 / ((max.y - min.y) * 2.0 + 1.0);
    let zs = 1.0 / ((max.z - min.z) * 2.0 + 1.0);
    let x_offset = (1.0 - (1.0 / xs).floor() * xs) / 2.0;
    let z_offset = (1.0 - (1.0 / zs).floor() * zs) / 2.0;
    if xs < 0.0 || ys < 0.0 || zs < 0.0 {
        return 0.0;
    }
    let (mut hits, mut count) = (0u32, 0u32);
    let mut xx = 0.0;
    while xx <= 1.0 {
        let mut yy = 0.0;
        while yy <= 1.0 {
            let mut zz = 0.0;
            while zz <= 1.0 {
                let x = lerp(xx, min.x, max.x);
                let y = lerp(yy, min.y, max.y);
                let z = lerp(zz, min.z, max.z);
                if !blocked(world, DVec3::new(x + x_offset, y, z + z_offset), center) {
                    hits += 1;
                }
                count += 1;
                zz += zs;
            }
            yy += ys;
        }
        xx += xs;
    }
    hits as f32 / count as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use minecraftoss_player::{Block, Pos};
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Scene(BTreeMap<Pos, Block>);
    impl World for Scene {
        fn block(&self, pos: Pos) -> Option<Block> {
            self.0.get(&pos).cloned()
        }
        fn set_block(&mut self, pos: Pos, block: Option<Block>) {
            match block {
                Some(block) => self.0.insert(pos, block),
                None => self.0.remove(&pos),
            };
        }
    }

    #[test]
    fn a_wall_blocks_sight_and_open_air_does_not() {
        let mut scene = Scene::default();
        let (a, b) = (DVec3::new(0.5, 1.62, 0.5), DVec3::new(6.5, 1.62, 0.5));
        assert!(line_of_sight(&scene, a, b));
        scene.set_block((3, 1, 0), Some(Block::new("minecraft:stone")));
        assert!(!line_of_sight(&scene, a, b));
        // Above the wall's top the ray passes.
        assert!(line_of_sight(&scene, DVec3::new(0.5, 2.5, 0.5), DVec3::new(6.5, 2.5, 0.5)));
    }

    #[test]
    fn an_exposed_box_is_fully_seen_and_a_covered_one_not() {
        let mut scene = Scene::default();
        let center = DVec3::new(0.5, 1.0, 0.5);
        let (min, max) = (DVec3::new(3.2, 1.0, 0.2), DVec3::new(3.8, 2.8, 0.8));
        assert_eq!(seen_percent(&scene, center, min, max), 1.0);
        for y in 0..4 {
            for z in -1..=1 {
                scene.set_block((2, y, z), Some(Block::new("minecraft:stone")));
            }
        }
        assert_eq!(seen_percent(&scene, center, min, max), 0.0);
    }
}
