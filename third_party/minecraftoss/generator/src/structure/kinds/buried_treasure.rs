//! Buried treasure (vanilla `BuriedTreasureStructure`, `BuriedTreasurePieces`).
//!
//! Source-informed from the pinned 26.3 JAR: the chest sinks from the ocean
//! floor to the first block above sandstone or stone, and the blocks around
//! it are filled so it stays buried.

use crate::feature::template::BoundingBox;
use crate::feature::Ctx;
use crate::structure::piece::{create_chest, Piece, PieceBase};
use crate::structure::{GenerationContext, StructureKind, Stub};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::{BlockPos, BlockStateId, ChunkPos};

#[derive(Debug)]
pub struct BuriedTreasure;

impl StructureKind for BuriedTreasure {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let pos = BlockPos::new(ctx.chunk.min_block_x() + 9, 90, ctx.chunk.min_block_z() + 9);
        ctx.on_top_of_chunk_center(HeightmapKind::OceanFloorWg, move |_ctx: &mut GenerationContext| {
            vec![Box::new(BuriedTreasurePiece { base: PieceBase::new(0, BoundingBox::at(pos)) }) as Box<dyn Piece>]
        })
    }
}

#[derive(Debug)]
pub struct BuriedTreasurePiece {
    base: PieceBase,
}

fn is_liquid(ctx: &Ctx, state: BlockStateId) -> bool {
    ctx.is(state, "minecraft:water") || ctx.is(state, "minecraft:lava")
}

impl Piece for BuriedTreasurePiece {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:btp"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, _reference: BlockPos) {
        let b = self.base.bbox;
        let y = ctx.height(HeightmapKind::OceanFloorWg, b.min_x, b.min_z);
        let mut pos = BlockPos::new(b.min_x, y, b.min_z);
        let sand = ctx.registries().blocks.parse_state("minecraft:sand").expect("sand");
        while pos.y > ctx.min_y() {
            let current = ctx.block(pos);
            let below = ctx.block(pos.below());
            let base = matches!(ctx.name(below), "minecraft:sandstone" | "minecraft:stone" | "minecraft:andesite" | "minecraft:granite" | "minecraft:diorite");
            if base {
                let soft = if !ctx.is_air(current) && !is_liquid(ctx, current) { current } else { sand };
                for direction in Direction::ALL {
                    let relative = pos.relative(direction, 1);
                    let state = ctx.block(relative);
                    if ctx.is_air(state) || is_liquid(ctx, state) {
                        let below_relative = ctx.block(relative.below());
                        let fill = if (ctx.is_air(below_relative) || is_liquid(ctx, below_relative)) && direction != Direction::Up { below } else { soft };
                        ctx.set_block_update(relative, fill);
                    }
                }
                self.base.bbox = BoundingBox::at(pos);
                create_chest(ctx, chunk_bb, random, pos, "minecraft:chests/buried_treasure", None);
                return;
            }
            pos = pos.below();
        }
    }
}
