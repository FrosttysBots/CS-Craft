//! Collision geometry for entities (26.3 `AABB`, `VoxelShape.collide`,
//! `Shapes.collide`, `BlockCollisions` and `Entity.collideWithShapes`).
//!
//! Block shapes use vanilla's own voxel grids (the catalog's
//! `collision_grid`), so the epsilon rules of `VoxelShape.collideX` walk the
//! same cells as vanilla, including for boxes already overlapping a shape.

use super::Level;
use minecraftoss_core::block::FaceShape;
use minecraftoss_core::{BlockPos, BlockStateId};

/// `net.minecraft.world.phys.AABB`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Aabb {
    pub fn new(min_x: f64, min_y: f64, min_z: f64, max_x: f64, max_y: f64, max_z: f64) -> Self {
        Self { min: [min_x.min(max_x), min_y.min(max_y), min_z.min(max_z)], max: [min_x.max(max_x), min_y.max(max_y), min_z.max(max_z)] }
    }

    /// `EntityDimensions.makeBoundingBox`: centred on x/z, standing on y.
    pub fn for_entity(x: f64, y: f64, z: f64, width: f32, height: f32) -> Self {
        let half = f64::from(width) / 2.0;
        let height = f64::from(height);
        Self::new(x - half, y, z - half, x + half, y + height, z + half)
    }

    pub fn moved(&self, d: [f64; 3]) -> Self {
        Self { min: [self.min[0] + d[0], self.min[1] + d[1], self.min[2] + d[2]], max: [self.max[0] + d[0], self.max[1] + d[1], self.max[2] + d[2]] }
    }

    /// `AABB.expandTowards`.
    pub fn expand_towards(&self, d: [f64; 3]) -> Self {
        let mut out = *self;
        for axis in 0..3 {
            if d[axis] < 0.0 {
                out.min[axis] += d[axis];
            } else if d[axis] > 0.0 {
                out.max[axis] += d[axis];
            }
        }
        out
    }

    /// `AABB.inflate` (negative deflates).
    pub fn inflate(&self, amount: f64) -> Self {
        Self::new(
            self.min[0] - amount,
            self.min[1] - amount,
            self.min[2] - amount,
            self.max[0] + amount,
            self.max[1] + amount,
            self.max[2] + amount,
        )
    }

    /// `AABB.intersects`: overlap with positive volume.
    pub fn intersects(&self, other: &Aabb) -> bool {
        (0..3).all(|a| self.min[a] < other.max[a] && self.max[a] > other.min[a])
    }
}

/// `ClipContext.Block`: which block shapes a clip ray tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipBlocks {
    /// `COLLIDER`: collision shapes.
    Collider,
    /// `FALLDAMAGE_RESETTING`: full blocks for the `fall_damage_resetting`
    /// tag, nothing else.
    FallDamageResetting,
}

/// `ClipContext.Fluid`: which fluid shapes a clip ray tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipFluids {
    None,
    /// Water only (`WATER`).
    Water,
    /// Source blocks of any fluid (`SOURCE_ONLY`).
    SourceOnly,
}

/// `Mth.lerp`.
fn lerp(delta: f64, a: f64, b: f64) -> f64 {
    a + delta * (b - a)
}

/// `Mth.frac`.
fn frac(v: f64) -> f64 {
    v - v.floor()
}

/// `Mth.sign` for doubles.
fn sign(v: f64) -> i32 {
    if v == 0.0 {
        0
    } else if v > 0.0 {
        1
    } else {
        -1
    }
}

/// A block's collision shape placed in the world: vanilla's voxel grid.
#[derive(Clone, Debug)]
pub struct VoxelShape {
    coords: [Vec<f64>; 3],
    /// Full cells, `(x * ny + y) * nz + z`.
    full: Vec<bool>,
    /// `toAabbs()` in the world, as `min` then `max` corners.
    boxes: Vec<[f64; 6]>,
}

impl VoxelShape {
    /// `Shapes.block().move(pos)`.
    pub fn block(pos: BlockPos) -> Self {
        let (x, y, z) = (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
        Self { coords: [vec![x, x + 1.0], vec![y, y + 1.0], vec![z, z + 1.0]], full: vec![true], boxes: vec![[x, y, z, x + 1.0, y + 1.0, z + 1.0]] }
    }

    /// A catalog shape (its boxes over its voxel grid) moved to a position.
    fn from_boxes(boxes: &[[f64; 6]], grid: Option<&[Vec<f64>; 3]>, pos: BlockPos) -> Self {
        let offset = [f64::from(pos.x), f64::from(pos.y), f64::from(pos.z)];
        let local: [Vec<f64>; 3] = match grid {
            Some(grid) => grid.clone(),
            None => std::array::from_fn(|axis| {
                let mut values: Vec<f64> = boxes.iter().flat_map(|b| [b[axis], b[axis + 3]]).collect();
                values.sort_by(f64::total_cmp);
                values.dedup();
                values
            }),
        };
        let sizes = [local[0].len() - 1, local[1].len() - 1, local[2].len() - 1];
        let mut full = vec![false; sizes[0] * sizes[1] * sizes[2]];
        for x in 0..sizes[0] {
            let cx = (local[0][x] + local[0][x + 1]) / 2.0;
            for y in 0..sizes[1] {
                let cy = (local[1][y] + local[1][y + 1]) / 2.0;
                for z in 0..sizes[2] {
                    let cz = (local[2][z] + local[2][z + 1]) / 2.0;
                    full[(x * sizes[1] + y) * sizes[2] + z] =
                        boxes.iter().any(|b| b[0] <= cx && cx <= b[3] && b[1] <= cy && cy <= b[4] && b[2] <= cz && cz <= b[5]);
                }
            }
        }
        let coords = std::array::from_fn(|axis| local[axis].iter().map(|v| v + offset[axis]).collect());
        let boxes = boxes.iter().map(|b| [b[0] + offset[0], b[1] + offset[1], b[2] + offset[2], b[3] + offset[0], b[4] + offset[1], b[5] + offset[2]]).collect();
        Self { coords, full, boxes }
    }

    /// `Shapes.box(0, 0, 0, 1, height, 1)` at a position.
    pub fn column(pos: BlockPos, height: f64) -> Self {
        Self::from_boxes(&[[0.0, 0.0, 0.0, 1.0, height, 1.0]], None, pos)
    }

    /// `VoxelShape.clip(from, to, pos) != null`: a point just past `from`
    /// inside a full cell, or an entry face of one of the boxes
    /// (`AABB.clip` over `toAabbs`).
    pub fn clip_hits(&self, from: [f64; 3], to: [f64; 3]) -> bool {
        if !self.full.iter().any(|&f| f) {
            return false;
        }
        let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 1.0e-7 {
            return false;
        }
        let test = [from[0] + d[0] * 0.001, from[1] + d[1] * 0.001, from[2] + d[2] * 0.001];
        let cell = [self.find_index(0, test[0]), self.find_index(1, test[1]), self.find_index(2, test[2])];
        if (0..3).all(|a| cell[a] >= 0 && (cell[a] as usize) < self.size(a)) && self.is_full(cell.map(|c| c as usize)) {
            return true;
        }
        self.boxes.iter().any(|b| {
            let (min, max) = ([b[0], b[1], b[2]], [b[3], b[4], b[5]]);
            [(0usize, 1usize, 2usize), (1, 2, 0), (2, 0, 1)].into_iter().any(|(a, b, c)| {
                let face = if d[a] > 1.0e-7 {
                    min[a]
                } else if d[a] < -1.0e-7 {
                    max[a]
                } else {
                    return false;
                };
                let s = (face - from[a]) / d[a];
                let pb = from[b] + s * d[b];
                let pc = from[c] + s * d[c];
                0.0 < s && s < 1.0 && min[b] - 1.0e-7 < pb && pb < max[b] + 1.0e-7 && min[c] - 1.0e-7 < pc && pc < max[c] + 1.0e-7
            })
        })
    }

    fn size(&self, axis: usize) -> usize {
        self.coords[axis].len() - 1
    }

    fn is_full(&self, cell: [usize; 3]) -> bool {
        self.full[(cell[0] * self.size(1) + cell[1]) * self.size(2) + cell[2]]
    }

    /// `VoxelShape.findIndex`: the last coordinate at or below `value`
    /// (-1 below the first, the cell count above the last).
    fn find_index(&self, axis: usize, value: f64) -> i64 {
        let coords = &self.coords[axis];
        coords.partition_point(|&c| c <= value) as i64 - 1
    }

    /// `VoxelShape.collide(axis, moving, distance)` (`collideX`).
    pub fn collide(&self, axis: usize, moving: &Aabb, mut distance: f64) -> f64 {
        if self.full.iter().all(|f| !f) {
            return distance;
        }
        if distance.abs() < 1.0e-7 {
            return 0.0;
        }
        let (b_axis, c_axis) = ((axis + 1) % 3, (axis + 2) % 3);
        let max_a = moving.max[axis];
        let min_a = moving.min[axis];
        let a_min = self.find_index(axis, min_a + 1.0e-7);
        let a_max = self.find_index(axis, max_a - 1.0e-7);
        let b_min = self.find_index(b_axis, moving.min[b_axis] + 1.0e-7).max(0);
        let b_max = (self.find_index(b_axis, moving.max[b_axis] - 1.0e-7) + 1).min(self.size(b_axis) as i64);
        let c_min = self.find_index(c_axis, moving.min[c_axis] + 1.0e-7).max(0);
        let c_max = (self.find_index(c_axis, moving.max[c_axis] - 1.0e-7) + 1).min(self.size(c_axis) as i64);
        let a_size = self.size(axis) as i64;
        let cell = |a: i64, b: i64, c: i64| {
            let mut cell = [0usize; 3];
            cell[axis] = a as usize;
            cell[b_axis] = b as usize;
            cell[c_axis] = c as usize;
            cell
        };
        let layer_full = |a: i64| (b_min..b_max).any(|b| (c_min..c_max).any(|c| self.is_full(cell(a, b, c))));
        if distance > 0.0 {
            for a in (a_max + 1).max(0)..a_size {
                if layer_full(a) {
                    let new_distance = self.coords[axis][a as usize] - max_a;
                    if new_distance >= -1.0e-7 {
                        distance = distance.min(new_distance);
                    }
                    return distance;
                }
            }
        } else if distance < 0.0 {
            let mut a = (a_min - 1).min(a_size - 1);
            while a >= 0 {
                if layer_full(a) {
                    let new_distance = self.coords[axis][a as usize + 1] - min_a;
                    if new_distance <= 1.0e-7 {
                        distance = distance.max(new_distance);
                    }
                    return distance;
                }
                a -= 1;
            }
        }
        distance
    }

    /// `Shapes.joinIsNotEmpty(this, Shapes.create(box), AND)`.
    fn overlaps(&self, other: &Aabb) -> bool {
        let (nx, ny, nz) = (self.size(0), self.size(1), self.size(2));
        for x in 0..nx {
            if !(self.coords[0][x] < other.max[0] && self.coords[0][x + 1] > other.min[0]) {
                continue;
            }
            for y in 0..ny {
                if !(self.coords[1][y] < other.max[1] && self.coords[1][y + 1] > other.min[1]) {
                    continue;
                }
                for z in 0..nz {
                    if self.coords[2][z] < other.max[2] && self.coords[2][z + 1] > other.min[2] && self.is_full([x, y, z]) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// The coordinates along Y (`VoxelShape.getCoords(Y)`).
    pub fn y_coords(&self) -> &[f64] {
        &self.coords[1]
    }
}

/// `Shapes.collide(axis, moving, shapes, distance)`.
pub fn collide_shapes(axis: usize, moving: &Aabb, shapes: &[VoxelShape], mut distance: f64) -> f64 {
    for shape in shapes {
        if distance.abs() < 1.0e-7 {
            return 0.0;
        }
        distance = shape.collide(axis, moving, distance);
    }
    distance
}

/// `Direction.axisStepOrder`: Y first, then the larger horizontal movement.
fn axis_step_order(movement: [f64; 3]) -> [usize; 3] {
    if movement[0].abs() < movement[2].abs() {
        [1, 2, 0]
    } else {
        [1, 0, 2]
    }
}

/// `Entity.collideWithShapes`.
pub fn collide_with_shapes(movement: [f64; 3], bounding_box: &Aabb, shapes: &[VoxelShape]) -> [f64; 3] {
    if shapes.is_empty() {
        return movement;
    }
    let mut resolved = [0.0; 3];
    for axis in axis_step_order(movement) {
        if movement[axis] != 0.0 {
            resolved[axis] = collide_shapes(axis, &bounding_box.moved(resolved), shapes, movement[axis]);
        }
    }
    resolved
}

impl Level<'_> {
    /// A block's collision shape at a position (`None` when empty).
    /// Moving pistons collide through their block entity, which is not
    /// simulated yet: they are treated as empty.
    pub fn block_collision_shape(&self, state: BlockStateId, pos: BlockPos) -> Option<VoxelShape> {
        let blocks = &self.registries().blocks;
        match blocks.collision_shape(state)? {
            FaceShape::Empty => None,
            FaceShape::Full => Some(VoxelShape::block(pos)),
            FaceShape::Boxes(boxes) => Some(VoxelShape::from_boxes(boxes, blocks.collision_grid(state), pos)),
        }
    }

    /// `BlockState.hasLargeCollisionShape`: a collision box leaving the block.
    fn has_large_collision_shape(&self, state: BlockStateId) -> bool {
        match self.registries().blocks.collision_shape(state) {
            Some(FaceShape::Boxes(boxes)) => boxes.iter().any(|b| b[0] < 0.0 || b[1] < 0.0 || b[2] < 0.0 || b[3] > 1.0 || b[4] > 1.0 || b[5] > 1.0),
            _ => false,
        }
    }

    /// `BlockGetter.clip(ClipContext)` hitting anything: `traverseBlocks`
    /// testing each block's shape for `blocks` and, with `water`, water's
    /// fluid shape (`ClipContext.Fluid.WATER`). Vanilla caches each fluid
    /// state's shape at its first use; the height is computed here directly.
    pub fn clip_hits(&self, from: [f64; 3], to: [f64; 3], blocks: ClipBlocks, water: bool) -> bool {
        self.clip_first_hit(from, to, blocks, if water { ClipFluids::Water } else { ClipFluids::None }).is_some()
    }

    /// `BlockGetter.clip`'s hit block: the first block along the segment
    /// whose tested shape the segment enters.
    pub fn clip_first_hit(&self, from: [f64; 3], to: [f64; 3], blocks: ClipBlocks, fluids: ClipFluids) -> Option<BlockPos> {
        if from == to {
            return None;
        }
        let fall_resetting = self.lib.registries.block_tags.require("minecraft:fall_damage_resetting").expect("tag exists");
        let hits_at = |x: i32, y: i32, z: i32| {
            let pos = BlockPos::new(x, y, z);
            let state = self.block(pos);
            let shape = match blocks {
                ClipBlocks::Collider => self.block_collision_shape(state, pos),
                ClipBlocks::FallDamageResetting => self.lib.registries.block_in_tag(state, fall_resetting).then(|| VoxelShape::block(pos)),
            };
            if shape.is_some_and(|shape| shape.clip_hits(from, to)) {
                return true;
            }
            let fluid = self.fluid_state(state).filter(|f| match fluids {
                ClipFluids::None => false,
                ClipFluids::Water => f.kind == minecraftoss_core::block::FluidKind::Water,
                ClipFluids::SourceOnly => f.source,
            });
            if let Some(fluid) = fluid {
                let above = self.fluid_state(self.block(pos.above())).is_some_and(|f| f.kind == fluid.kind);
                let height = if above { 1.0 } else { f64::from(f32::from(fluid.amount) / 9.0) };
                return VoxelShape::column(pos, height).clip_hits(from, to);
            }
            false
        };
        let t = [lerp(-1.0e-7, to[0], from[0]), lerp(-1.0e-7, to[1], from[1]), lerp(-1.0e-7, to[2], from[2])];
        let f = [lerp(-1.0e-7, from[0], to[0]), lerp(-1.0e-7, from[1], to[1]), lerp(-1.0e-7, from[2], to[2])];
        let mut block = [f[0].floor() as i32, f[1].floor() as i32, f[2].floor() as i32];
        if hits_at(block[0], block[1], block[2]) {
            return Some(BlockPos::new(block[0], block[1], block[2]));
        }
        let d = [t[0] - f[0], t[1] - f[1], t[2] - f[2]];
        let s = [sign(d[0]), sign(d[1]), sign(d[2])];
        let t_delta: [f64; 3] = std::array::from_fn(|a| if s[a] == 0 { f64::MAX } else { f64::from(s[a]) / d[a] });
        let mut tt: [f64; 3] = std::array::from_fn(|a| t_delta[a] * if s[a] > 0 { 1.0 - frac(f[a]) } else { frac(f[a]) });
        while tt[0] <= 1.0 || tt[1] <= 1.0 || tt[2] <= 1.0 {
            let axis = if tt[0] < tt[1] {
                if tt[0] < tt[2] { 0 } else { 2 }
            } else if tt[1] < tt[2] {
                1
            } else {
                2
            };
            block[axis] += s[axis];
            tt[axis] += t_delta[axis];
            if hits_at(block[0], block[1], block[2]) {
                return Some(BlockPos::new(block[0], block[1], block[2]));
            }
        }
        None
    }

    /// `CollisionGetter.getBlockCollisions` (`BlockCollisions`).
    pub fn block_collisions(&self, bounding_box: &Aabb) -> Vec<VoxelShape> {
        let mut out = Vec::new();
        self.for_block_collisions(bounding_box, |_, shape| out.push(shape));
        out
    }

    fn for_block_collisions(&self, bounding_box: &Aabb, mut visit: impl FnMut(BlockPos, VoxelShape)) {
        let lo = |v: f64| (v - 1.0e-7).floor() as i32 - 1;
        let hi = |v: f64| (v + 1.0e-7).floor() as i32 + 1;
        let (x0, x1) = (lo(bounding_box.min[0]), hi(bounding_box.max[0]));
        let (y0, y1) = (lo(bounding_box.min[1]), hi(bounding_box.max[1]));
        let (z0, z1) = (lo(bounding_box.min[2]), hi(bounding_box.max[2]));
        for z in z0..=z1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    // `Cursor3D.getNextType`: how many faces of the range the cell is on.
                    let face_type = i32::from(x == x0 || x == x1) + i32::from(y == y0 || y == y1) + i32::from(z == z0 || z == z1);
                    if face_type == 3 {
                        continue;
                    }
                    let pos = BlockPos::new(x, y, z);
                    if self.chunk(pos.chunk()).is_none() {
                        continue;
                    }
                    let state = self.block(pos);
                    if face_type == 1 && !self.has_large_collision_shape(state) {
                        continue;
                    }
                    if face_type == 2 && self.name(state) != "minecraft:moving_piston" {
                        continue;
                    }
                    let Some(shape) = self.block_collision_shape(state, pos) else { continue };
                    if shape.overlaps(bounding_box) {
                        visit(pos, shape);
                    }
                }
            }
        }
    }

    /// The positions `BlockCollisions` yields for a box.
    pub fn block_collision_positions(&self, bounding_box: &Aabb) -> Vec<BlockPos> {
        let mut out = Vec::new();
        self.for_block_collisions(bounding_box, |pos, _| out.push(pos));
        out
    }

    /// `CollisionGetter.noCollision(entity, box)` for blocks.
    pub fn no_block_collision(&self, bounding_box: &Aabb) -> bool {
        self.block_collisions(bounding_box).is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falling_box_lands_on_a_block() {
        let floor = VoxelShape::block(BlockPos::new(0, 0, 0));
        let item = Aabb::for_entity(0.5, 1.5, 0.5, 0.25, 0.25);
        let moved = collide_with_shapes([0.0, -1.0, 0.0], &item, &[floor]);
        assert!((moved[1] + 0.5).abs() < 1e-12, "{moved:?}");
    }

    #[test]
    fn slab_grid_stops_on_its_top() {
        let slab = VoxelShape::from_boxes(&[[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]], Some(&[vec![0.0, 1.0], vec![0.0, 0.5, 1.0], vec![0.0, 1.0]]), BlockPos::new(0, 0, 0));
        let item = Aabb::for_entity(0.5, 1.0, 0.5, 0.25, 0.25);
        let moved = collide_with_shapes([0.0, -1.0, 0.0], &item, &[slab]);
        assert!((moved[1] + 0.5).abs() < 1e-12, "{moved:?}");
    }
}
