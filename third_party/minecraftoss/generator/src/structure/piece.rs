//! Structure pieces (vanilla `StructurePiece`): the orientation transform
//! from piece-local to world coordinates and the block helpers legacy pieces
//! build with. Every helper clips to the chunk being decorated, as vanilla
//! does.

use crate::feature::blocks::FluidType;
use crate::feature::template::{BoundingBox, Mirror, Rotation};
use crate::feature::Ctx;
use minecraftoss_core::block::flags;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{RandomSource, WorldgenRandom};
use minecraftoss_core::{BlockPos, BlockStateId, ChunkPos};

/// A placed structure piece.
pub trait Piece: Send + Sync + std::fmt::Debug {
    fn base(&self) -> &PieceBase;
    fn base_mut(&mut self) -> &mut PieceBase;
    /// The registered `StructurePieceType` ID.
    fn type_name(&self) -> &'static str;
    /// `StructurePiece.postProcess`.
    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, chunk: ChunkPos, reference: BlockPos);
    /// `StructurePiece.move`.
    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        let b = &mut self.base_mut().bbox;
        *b = b.moved(dx, dy, dz);
    }
    /// The jigsaw piece, when this is one (for terrain adaptation).
    fn pool_piece(&self) -> Option<&crate::structure::jigsaw::PoolPiece> {
        None
    }
    /// For structures whose `afterPlace` reads their own piece type.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Blocks placed by `placeBlock` that are marked for post-processing.
const SHAPE_CHECK: &[&str] = &[
    "minecraft:nether_brick_fence",
    "minecraft:torch",
    "minecraft:wall_torch",
    "minecraft:oak_fence",
    "minecraft:spruce_fence",
    "minecraft:dark_oak_fence",
    "minecraft:pale_oak_fence",
    "minecraft:acacia_fence",
    "minecraft:birch_fence",
    "minecraft:jungle_fence",
    "minecraft:ladder",
    "minecraft:iron_bars",
];

/// The state every `StructurePiece` carries.
#[derive(Clone, Debug)]
pub struct PieceBase {
    pub bbox: BoundingBox,
    orientation: Option<Direction>,
    mirror: Mirror,
    rotation: Rotation,
    pub gen_depth: i32,
    /// Blocks `placeBlock` never replaces (mineshafts' `canBeReplaced`).
    pub keep: Vec<minecraftoss_core::BlockId>,
}

/// `StructurePiece.BlockSelector`.
pub trait BlockSelector {
    fn next(&mut self, ctx: &Ctx, random: &mut WorldgenRandom, x: i32, y: i32, z: i32, edge: bool) -> BlockStateId;
}

impl PieceBase {
    pub fn new(gen_depth: i32, bbox: BoundingBox) -> Self {
        Self { bbox, orientation: None, mirror: Mirror::None, rotation: Rotation::None, gen_depth, keep: Vec::new() }
    }

    /// `StructurePiece.makeBoundingBox`.
    pub fn make_bounding_box(x: i32, y: i32, z: i32, direction: Direction, width: i32, height: i32, depth: i32) -> BoundingBox {
        if direction.axis() == minecraftoss_core::pos::Axis::Z {
            BoundingBox::new(x, y, z, x + width - 1, y + height - 1, z + depth - 1)
        } else {
            BoundingBox::new(x, y, z, x + depth - 1, y + height - 1, z + width - 1)
        }
    }

    /// `StructurePiece.getRandomHorizontalDirection`.
    pub fn random_horizontal_direction(random: &mut impl RandomSource) -> Direction {
        Direction::HORIZONTAL[random.next_i32_bound(4) as usize]
    }

    pub fn orientation(&self) -> Option<Direction> {
        self.orientation
    }

    pub fn mirror(&self) -> Mirror {
        self.mirror
    }

    pub fn rotation(&self) -> Rotation {
        self.rotation
    }

    /// `StructurePiece.setOrientation`: south mirrors, west mirrors and
    /// rotates, east rotates.
    pub fn set_orientation(&mut self, orientation: Option<Direction>) {
        self.orientation = orientation;
        (self.mirror, self.rotation) = match orientation {
            Some(Direction::South) => (Mirror::LeftRight, Rotation::None),
            Some(Direction::West) => (Mirror::LeftRight, Rotation::Clockwise90),
            Some(Direction::East) => (Mirror::None, Rotation::Clockwise90),
            _ => (Mirror::None, Rotation::None),
        };
    }

    /// `StructurePiece.isCloseToChunk`.
    pub fn is_close_to_chunk(&self, chunk: ChunkPos, distance: i32) -> bool {
        let (cx, cz) = (chunk.min_block_x(), chunk.min_block_z());
        self.bbox.intersects_xz(cx - distance, cz - distance, cx + 15 + distance, cz + 15 + distance)
    }

    pub fn world_x(&self, x: i32, z: i32) -> i32 {
        match self.orientation {
            Some(Direction::North | Direction::South) => self.bbox.min_x + x,
            Some(Direction::West) => self.bbox.max_x - z,
            Some(Direction::East) => self.bbox.min_x + z,
            _ => x,
        }
    }

    pub fn world_y(&self, y: i32) -> i32 {
        if self.orientation.is_none() { y } else { y + self.bbox.min_y }
    }

    pub fn world_z(&self, x: i32, z: i32) -> i32 {
        match self.orientation {
            Some(Direction::North) => self.bbox.max_z - z,
            Some(Direction::South) => self.bbox.min_z + z,
            Some(Direction::West | Direction::East) => self.bbox.min_z + x,
            _ => z,
        }
    }

    pub fn world_pos(&self, x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos::new(self.world_x(x, z), self.world_y(y), self.world_z(x, z))
    }

    /// `StructurePiece.placeBlock` with the default `canBeReplaced`.
    pub fn place_block(&self, ctx: &mut Ctx, state: BlockStateId, x: i32, y: i32, z: i32, chunk_bb: &BoundingBox) {
        self.place_block_if(ctx, state, x, y, z, chunk_bb, |_, _| true);
    }

    /// `StructurePiece.placeBlock` with a piece's own `canBeReplaced` test
    /// (called with the world position when it is inside the chunk).
    #[allow(clippy::too_many_arguments)]
    pub fn place_block_if(&self, ctx: &mut Ctx, state: BlockStateId, x: i32, y: i32, z: i32, chunk_bb: &BoundingBox, can_replace: impl Fn(&Ctx, BlockPos) -> bool) {
        let pos = self.world_pos(x, y, z);
        if !chunk_bb.is_inside(pos) || !can_replace(ctx, pos) {
            return;
        }
        if !self.keep.is_empty() && self.keep.contains(&ctx.registries().blocks.block_of(ctx.block(pos))) {
            return;
        }
        let transforms = &ctx.lib.transforms;
        let state = transforms.rotate(transforms.mirror(state, self.mirror), self.rotation);
        ctx.set_block(pos, state);
        if ctx.fluid_at(pos) != FluidType::Empty {
            ctx.schedule_fluid_tick(pos);
        }
        if SHAPE_CHECK.contains(&ctx.name(state)) {
            ctx.region.mark_post_processing(pos.x, pos.y, pos.z);
        }
    }

    /// `StructurePiece.getBlock`: air outside the chunk.
    pub fn get_block(&self, ctx: &Ctx, x: i32, y: i32, z: i32, chunk_bb: &BoundingBox) -> BlockStateId {
        let pos = self.world_pos(x, y, z);
        if chunk_bb.is_inside(pos) { ctx.block(pos) } else { BlockStateId::AIR }
    }

    /// `StructurePiece.isInterior`.
    pub fn is_interior(&self, ctx: &Ctx, x: i32, y: i32, z: i32, chunk_bb: &BoundingBox) -> bool {
        let pos = self.world_pos(x, y + 1, z);
        chunk_bb.is_inside(pos) && pos.y < ctx.height(HeightmapKind::OceanFloorWg, pos.x, pos.z)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn generate_air_box(&self, ctx: &mut Ctx, chunk_bb: &BoundingBox, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) {
        let air = ctx.lib.blocks.air;
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    self.place_block(ctx, air, x, y, z, chunk_bb);
                }
            }
        }
    }

    /// `StructurePiece.generateBox` with an edge and a fill block.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_box(&self, ctx: &mut Ctx, chunk_bb: &BoundingBox, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, edge: BlockStateId, fill: BlockStateId, skip_air: bool) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if skip_air && ctx.is_air(self.get_block(ctx, x, y, z, chunk_bb)) {
                        continue;
                    }
                    let interior = y != y0 && y != y1 && x != x0 && x != x1 && z != z0 && z != z1;
                    self.place_block(ctx, if interior { fill } else { edge }, x, y, z, chunk_bb);
                }
            }
        }
    }

    /// `generateBox` over a piece-local box.
    pub fn generate_box_in(&self, ctx: &mut Ctx, chunk_bb: &BoundingBox, b: &BoundingBox, edge: BlockStateId, fill: BlockStateId, skip_air: bool) {
        self.generate_box(ctx, chunk_bb, b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z, edge, fill, skip_air);
    }

    /// `StructurePiece.generateBox` with a block selector.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_box_with(
        &self,
        ctx: &mut Ctx,
        chunk_bb: &BoundingBox,
        x0: i32,
        y0: i32,
        z0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        skip_air: bool,
        random: &mut WorldgenRandom,
        selector: &mut dyn BlockSelector,
    ) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if skip_air && ctx.is_air(self.get_block(ctx, x, y, z, chunk_bb)) {
                        continue;
                    }
                    let edge = y == y0 || y == y1 || x == x0 || x == x1 || z == z0 || z == z1;
                    let state = selector.next(ctx, random, x, y, z, edge);
                    self.place_block(ctx, state, x, y, z, chunk_bb);
                }
            }
        }
    }

    /// `StructurePiece.generateMaybeBox`.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_maybe_box(
        &self,
        ctx: &mut Ctx,
        chunk_bb: &BoundingBox,
        random: &mut WorldgenRandom,
        probability: f32,
        x0: i32,
        y0: i32,
        z0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        edge: BlockStateId,
        fill: BlockStateId,
        skip_air: bool,
        has_to_be_inside: bool,
    ) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if random.next_f32() > probability {
                        continue;
                    }
                    if skip_air && ctx.is_air(self.get_block(ctx, x, y, z, chunk_bb)) {
                        continue;
                    }
                    if has_to_be_inside && !self.is_interior(ctx, x, y, z, chunk_bb) {
                        continue;
                    }
                    let interior = y != y0 && y != y1 && x != x0 && x != x1 && z != z0 && z != z1;
                    self.place_block(ctx, if interior { fill } else { edge }, x, y, z, chunk_bb);
                }
            }
        }
    }

    /// `StructurePiece.maybeGenerateBlock`.
    #[allow(clippy::too_many_arguments)]
    pub fn maybe_generate_block(&self, ctx: &mut Ctx, chunk_bb: &BoundingBox, random: &mut WorldgenRandom, probability: f32, x: i32, y: i32, z: i32, state: BlockStateId) {
        if random.next_f32() < probability {
            self.place_block(ctx, state, x, y, z, chunk_bb);
        }
    }

    /// `StructurePiece.generateUpperHalfSphere`.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_upper_half_sphere(&self, ctx: &mut Ctx, chunk_bb: &BoundingBox, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, fill: BlockStateId, skip_air: bool) {
        let diag_x = (x1 - x0 + 1) as f32;
        let diag_y = (y1 - y0 + 1) as f32;
        let diag_z = (z1 - z0 + 1) as f32;
        let cx = x0 as f32 + diag_x / 2.0;
        let cz = z0 as f32 + diag_z / 2.0;
        for y in y0..=y1 {
            let ny = (y - y0) as f32 / diag_y;
            for x in x0..=x1 {
                let nx = (x as f32 - cx) / (diag_x * 0.5);
                for z in z0..=z1 {
                    let nz = (z as f32 - cz) / (diag_z * 0.5);
                    if skip_air && ctx.is_air(self.get_block(ctx, x, y, z, chunk_bb)) {
                        continue;
                    }
                    let dist = nx * nx + ny * ny + nz * nz;
                    if dist <= 1.05 {
                        self.place_block(ctx, fill, x, y, z, chunk_bb);
                    }
                }
            }
        }
    }

    /// `StructurePiece.fillColumnDown`.
    pub fn fill_column_down(&self, ctx: &mut Ctx, state: BlockStateId, x: i32, start_y: i32, z: i32, chunk_bb: &BoundingBox) {
        let mut pos = self.world_pos(x, start_y, z);
        if !chunk_bb.is_inside(pos) {
            return;
        }
        while is_replaceable_by_structures(ctx, ctx.block(pos)) && pos.y > ctx.min_y() + 1 {
            ctx.set_block(pos, state);
            pos = pos.below();
        }
    }

    /// `StructurePiece.createChest` at piece-local coordinates.
    #[allow(clippy::too_many_arguments)]
    pub fn create_chest(&self, ctx: &mut Ctx, chunk_bb: &BoundingBox, random: &mut WorldgenRandom, x: i32, y: i32, z: i32, loot: &str) -> bool {
        create_chest(ctx, chunk_bb, random, self.world_pos(x, y, z), loot, None)
    }

    /// `StructurePiece.createDispenser`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_dispenser(&self, ctx: &mut Ctx, chunk_bb: &BoundingBox, random: &mut WorldgenRandom, x: i32, y: i32, z: i32, facing: Direction, loot: &str) -> bool {
        let pos = self.world_pos(x, y, z);
        let dispenser = ctx.registries().blocks.parse_state("minecraft:dispenser").expect("dispenser");
        if !chunk_bb.is_inside(pos) || ctx.registries().blocks.block_of(ctx.block(pos)) == ctx.registries().blocks.block_of(dispenser) {
            return false;
        }
        let state = ctx.with(dispenser, "facing", facing.name());
        self.place_block(ctx, state, x, y, z, chunk_bb);
        if ctx.is(ctx.block(pos), "minecraft:dispenser") {
            let seed = random.next_i64();
            ctx.region.set_loot_table(pos.x, pos.y, pos.z, loot, seed);
        }
        true
    }
}

/// `StructurePiece.isReplaceableByStructures`.
pub fn is_replaceable_by_structures(ctx: &Ctx, state: BlockStateId) -> bool {
    ctx.is_air(state)
        || ctx.registries().blocks.is(state, flags::LIQUID)
        || matches!(ctx.name(state), "minecraft:glow_lichen" | "minecraft:seagrass" | "minecraft:tall_seagrass")
}

/// `StructurePiece.reorient`: a chest faces away from its one solid
/// neighbour, or away from walls it would open into.
pub fn reorient(ctx: &Ctx, pos: BlockPos, state: BlockStateId) -> BlockStateId {
    let solid = |p: BlockPos| ctx.registries().blocks.is(ctx.block(p), flags::SOLID_RENDER);
    let mut solid_neighbor = None;
    for direction in Direction::HORIZONTAL {
        let neighbor = ctx.block(pos.relative(direction, 1));
        if ctx.is(neighbor, "minecraft:chest") {
            return state;
        }
        if solid(pos.relative(direction, 1)) {
            if solid_neighbor.is_some() {
                solid_neighbor = None;
                break;
            }
            solid_neighbor = Some(direction);
        }
    }
    if let Some(d) = solid_neighbor {
        return ctx.with(state, "facing", d.opposite().name());
    }
    let mut lock = ctx.property(state, "facing").and_then(Direction::from_name).unwrap_or(Direction::North);
    if solid(pos.relative(lock, 1)) {
        lock = lock.opposite();
    }
    if solid(pos.relative(lock, 1)) {
        lock = lock.clockwise();
    }
    if solid(pos.relative(lock, 1)) {
        lock = lock.opposite();
    }
    ctx.with(state, "facing", lock.name())
}

/// `StructurePiece.createChest` at a world position: a loot chest, reoriented
/// unless a state is given; the loot seed draws one long.
pub fn create_chest(ctx: &mut Ctx, chunk_bb: &BoundingBox, random: &mut WorldgenRandom, pos: BlockPos, loot: &str, state: Option<BlockStateId>) -> bool {
    if !chunk_bb.is_inside(pos) || ctx.is(ctx.block(pos), "minecraft:chest") {
        return false;
    }
    let state = match state {
        Some(s) => s,
        None => {
            let chest = ctx.registries().blocks.parse_state("minecraft:chest").expect("chest");
            reorient(ctx, pos, chest)
        }
    };
    ctx.set_block(pos, state);
    // `ChestBlockEntity` (trapped and copper chests extend it).
    let name = ctx.name(ctx.block(pos));
    if name.ends_with("chest") && name != "minecraft:ender_chest" {
        let seed = random.next_i64();
        ctx.region.set_loot_table(pos.x, pos.y, pos.z, loot, seed);
    }
    true
}
