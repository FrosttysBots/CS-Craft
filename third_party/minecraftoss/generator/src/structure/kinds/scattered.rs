//! Single-piece surface structures (vanilla `SinglePieceStructure` and
//! `ScatteredFeaturePiece`): the start sits on the chunk centre and the
//! piece settles on the ground the first time a chunk places it.

use crate::feature::template::BoundingBox;
use crate::feature::Ctx;
use crate::structure::piece::{Piece, PieceBase};
use crate::structure::{GenerationContext, PieceList, Stub};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::LegacyRandom;
use minecraftoss_core::BlockPos;

/// `SinglePieceStructure.findGenerationPoint`.
pub fn single_piece<'s>(
    ctx: &mut GenerationContext,
    width: i32,
    depth: i32,
    construct: impl FnOnce(&mut LegacyRandom, i32, i32) -> Box<dyn Piece> + 's,
) -> Option<Stub<'s>> {
    if !ctx.could_valid_biome_exist_on_top_of_chunk_center() {
        return None;
    }
    if ctx.lowest_y(width, depth) < ctx.terrain.sea_level {
        return None;
    }
    let (x, z) = (ctx.chunk.min_block_x(), ctx.chunk.min_block_z());
    Some(ctx.on_top_of_chunk_center_without_biome_check(HeightmapKind::WorldSurfaceWg, move |ctx: &mut GenerationContext| {
        vec![construct(&mut ctx.random, x, z)] as PieceList
    }))
}

/// `ScatteredFeaturePiece` state.
#[derive(Clone, Debug)]
pub struct Scattered {
    pub base: PieceBase,
    pub width: i32,
    pub height: i32,
    pub depth: i32,
    pub height_position: i32,
}

impl Scattered {
    #[allow(clippy::too_many_arguments)]
    pub fn new(west: i32, floor: i32, north: i32, width: i32, height: i32, depth: i32, direction: Direction) -> Self {
        let mut base = PieceBase::new(0, PieceBase::make_bounding_box(west, floor, north, direction, width, height, depth));
        base.set_orientation(Some(direction));
        Self { base, width, height, depth, height_position: -1 }
    }

    /// `updateAverageGroundHeight`: the mean `MOTION_BLOCKING_NO_LEAVES`
    /// height over the part of the piece in this chunk, fixed once.
    pub fn update_average_ground_height(&mut self, ctx: &Ctx, chunk_bb: &BoundingBox, offset: i32) -> bool {
        if self.height_position >= 0 {
            return true;
        }
        let (mut total, mut count) = (0, 0);
        let b = self.base.bbox;
        for z in b.min_z..=b.max_z {
            for x in b.min_x..=b.max_x {
                if chunk_bb.is_inside(BlockPos::new(x, 64, z)) {
                    total += ctx.height(HeightmapKind::MotionBlockingNoLeaves, x, z);
                    count += 1;
                }
            }
        }
        if count == 0 {
            return false;
        }
        self.height_position = total / count;
        self.base.bbox = b.moved(0, self.height_position - b.min_y + offset, 0);
        true
    }

    /// `updateHeightPositionToLowestGroundHeight`: the lowest
    /// `MOTION_BLOCKING_NO_LEAVES` height under the whole piece, fixed once.
    pub fn update_to_lowest_ground_height(&mut self, ctx: &Ctx, offset: i32) -> bool {
        if self.height_position >= 0 {
            return true;
        }
        let mut lowest = ctx.max_y() + 1;
        let b = self.base.bbox;
        for z in b.min_z..=b.max_z {
            for x in b.min_x..=b.max_x {
                lowest = lowest.min(ctx.height(HeightmapKind::MotionBlockingNoLeaves, x, z));
            }
        }
        self.height_position = lowest;
        self.base.bbox = b.moved(0, self.height_position - b.min_y + offset, 0);
        true
    }
}
