//! Shared block-shape collision for bodies with arbitrary dimensions.
//!
//! Source: Java 26.3 Entity.collide/collectCandidateStepUpHeights and
//! Direction.axisStepOrder. Uses the authored world's existing shape resolver;
//! world-border and entity colliders are not yet included.
use crate::{clip_axis, shapes_near, Box3, Pos, World};
use glam::DVec3;

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub min: DVec3,
    pub max: DVec3,
}

impl Bounds {
    pub fn standing(feet: DVec3, width: f32, height: f32) -> Self {
        let radius = width as f64 / 2.0;
        Self {
            min: feet - DVec3::new(radius, 0.0, radius),
            max: feet + DVec3::new(radius, height as f64, radius),
        }
    }
}

fn sweep(b: Box3, delta: DVec3) -> Box3 {
    Box3::new(b.min.min(b.min + delta), b.max.max(b.max + delta))
}

fn resolve(b: Box3, shapes: &[Box3], delta: DVec3) -> DVec3 {
    if shapes.is_empty() {
        return delta;
    }
    let order = if delta.x.abs() < delta.z.abs() { [1, 2, 0] } else { [1, 0, 2] };
    let mut result = DVec3::ZERO;
    for axis in order {
        if delta[axis] != 0.0 {
            result[axis] = clip_axis(b.offset(result), shapes, axis, delta[axis]);
        }
    }
    result
}

/// Resolve one displacement against the same block shapes used by player/items.
/// `step_height` is a float attribute; candidate heights also round to floats.
pub fn move_body(world: &impl World, bounds: Bounds, delta: DVec3, on_ground: bool, step_height: f32) -> DVec3 {
    if delta.length_squared() == 0.0 {
        return delta;
    }
    let body = Box3::new(bounds.min, bounds.max);
    let swept = sweep(body, delta);
    // Padding finds shapes that extend outside their owner block (e.g. fences).
    let shapes = shapes_near(world, swept, 1.0);
    let normal: Vec<_> = shapes.iter().copied().filter(|s| s.intersects(swept)).collect();
    let clipped = resolve(body, &normal, delta);
    let landed = delta.y != clipped.y && delta.y < 0.0;
    if step_height <= 0.0 || !(landed || on_ground) || (delta.x == clipped.x && delta.z == clipped.z) {
        return clipped;
    }
    let grounded = if landed { body.offset(DVec3::new(0.0, clipped.y, 0.0)) } else { body };
    let mut area = sweep(grounded, DVec3::new(delta.x, step_height as f64, delta.z));
    if !landed {
        area.min.y -= 1.0e-5_f32 as f64;
    }
    let colliders: Vec<_> = shapes_near(world, area, 1.0).into_iter().filter(|s| s.intersects(area)).collect();
    let mut heights = Vec::new();
    for s in &colliders {
        for y in [s.min.y, s.max.y] {
            let h = (y - grounded.min.y) as f32;
            if h >= 0.0 && h <= step_height && h != clipped.y as f32 {
                heights.push(h);
            }
        }
    }
    heights.sort_by(f32::total_cmp);
    heights.dedup();
    for h in heights {
        let step = resolve(grounded, &colliders, DVec3::new(delta.x, h as f64, delta.z));
        if step.x * step.x + step.z * step.z > clipped.x * clipped.x + clipped.z * clipped.z {
            return step - DVec3::new(0.0, body.min.y - grounded.min.y, 0.0);
        }
    }
    clipped
}

/// `BlockCollisions`: the blocks whose collision shapes meet the box from
/// `min` to `max` (the ring around the box's blocks counts only for shapes
/// reaching out of their block; overlaps thinner than 1e-7 merge away, as
/// `Shapes.joinIsNotEmpty` does), in the cursor's order.
pub fn colliding_blocks<W: World + ?Sized>(world: &W, min: DVec3, max: DVec3) -> Vec<Pos> {
    let area = Box3::new(min, max);
    let (x0, x1) = ((min.x - 1.0e-7).floor() as i32 - 1, (max.x + 1.0e-7).floor() as i32 + 1);
    let (y0, y1) = ((min.y - 1.0e-7).floor() as i32 - 1, (max.y + 1.0e-7).floor() as i32 + 1);
    let (z0, z1) = ((min.z - 1.0e-7).floor() as i32 - 1, (max.z + 1.0e-7).floor() as i32 + 1);
    let mut out = Vec::new();
    for y in y0..=y1 {
        for z in z0..=z1 {
            for x in x0..=x1 {
                let face = [x == x0 || x == x1, y == y0 || y == y1, z == z0 || z == z1].iter().filter(|&&edge| edge).count();
                // Corners never; edges only for moving pistons.
                if face >= 2 {
                    continue;
                }
                let origin = DVec3::new(f64::from(x), f64::from(y), f64::from(z));
                let boxes: Vec<(DVec3, DVec3)> = world
                    .collision_boxes((x, y, z))
                    .into_iter()
                    .map(|b| (DVec3::new(b[0], b[1], b[2]) + origin, DVec3::new(b[3], b[4], b[5]) + origin))
                    .collect();
                if boxes.is_empty() {
                    continue;
                }
                if face == 1 && !boxes.iter().any(|&(lo, hi)| (lo - origin).min_element() < 0.0 || (hi - origin).max_element() > 1.0) {
                    continue;
                }
                let full = boxes.len() == 1 && boxes[0] == (origin, origin + DVec3::ONE);
                let meets = |lo: DVec3, hi: DVec3| {
                    if full {
                        area.intersects(Box3::new(lo, hi))
                    } else {
                        (0..3).all(|axis| hi[axis].min(max[axis]) - lo[axis].max(min[axis]) > 1.0e-7)
                    }
                };
                if boxes.iter().any(|&(lo, hi)| meets(lo, hi)) {
                    out.push((x, y, z));
                }
            }
        }
    }
    out
}

/// `CollisionGetter.noCollision` for a box no entity owns: no block's
/// collision shape meets it (the harness's worlds hold no entities that
/// collide).
pub fn no_block_collision<W: World + ?Sized>(world: &W, min: DVec3, max: DVec3) -> bool {
    colliding_blocks(world, min, max).is_empty()
}

/// `CollisionGetter.findSupportingBlock`: of the blocks whose collision
/// shapes meet the box from `min` to `max`, the one whose centre is
/// nearest `position`; ties go to the greatest by y, then z, then x
/// (`Vec3i.compareTo`).
pub fn find_supporting_block(world: &impl World, position: DVec3, min: DVec3, max: DVec3) -> Option<Pos> {
    let mut best: Option<(Pos, f64)> = None;
    for (x, y, z) in colliding_blocks(world, min, max) {
        let origin = DVec3::new(f64::from(x), f64::from(y), f64::from(z));
        let distance = (origin + DVec3::splat(0.5) - position).length_squared();
        let pos = (x, y, z);
        let greater = |a: Pos, b: Pos| (a.1, a.2, a.0) > (b.1, b.2, b.0);
        if best.is_none_or(|(held, held_distance)| distance < held_distance || distance == held_distance && greater(pos, held)) {
            best = Some((pos, distance));
        }
    }
    best.map(|(pos, _)| pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Block, Pos};
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Scene(BTreeMap<Pos, Block>);
    impl World for Scene {
        fn block(&self, p: Pos) -> Option<Block> { self.0.get(&p).cloned() }
        fn set_block(&mut self, p: Pos, block: Option<Block>) {
            if let Some(b) = block { self.0.insert(p, b); } else { self.0.remove(&p); }
        }
    }
    #[test]
    fn step_candidates_follow_the_obstacle_not_the_maximum_height() {
        let mut scene = Scene::default();
        scene.set_block((1, 0, 0), Some(Block::new("minecraft:stone_slab")));
        let bounds = Bounds::standing(DVec3::new(0.5, 0.0, 0.5), 0.6, 1.8);
        let delta = DVec3::new(0.8, -0.08, 0.0);
        let moved = move_body(&scene, bounds, delta, true, 0.6);
        assert_eq!(moved, DVec3::new(0.8, 0.5, 0.0));
        assert!(move_body(&scene, bounds, delta, true, 0.4).x < 0.8);
    }
    #[test]
    fn ceiling_prevents_stepping_and_fences_extend_above_their_block() {
        let mut scene = Scene::default();
        scene.set_block((1, 0, 0), Some(Block::new("minecraft:stone_slab")));
        scene.set_block((0, 2, 0), Some(Block::new("minecraft:stone")));
        let bounds = Bounds::standing(DVec3::new(0.5, 0.0, 0.5), 0.6, 1.8);
        assert!(move_body(&scene, bounds, DVec3::X, true, 0.6).x < 1.0);
        scene.set_block((1, 0, 0), Some(Block::new("minecraft:oak_fence_gate").with("facing", "east")));
        let bounds = Bounds::standing(DVec3::new(0.5, 1.1, 0.5), 0.6, 0.2);
        assert!(move_body(&scene, bounds, DVec3::X, false, 0.0).x < 1.0);
    }
}
