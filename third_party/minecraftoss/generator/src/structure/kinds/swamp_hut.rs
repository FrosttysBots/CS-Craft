//! Swamp huts (vanilla `SwampHutStructure`, `SwampHutPiece`).
//!
//! Source-informed from the pinned 26.3 JAR. The hut settles on the mean
//! ground height of the part of it in the first chunk that places it. The
//! witch and cat are recorded once, in the chunk holding their spawn block.

use super::scattered::Scattered;
use crate::feature::template::BoundingBox;
use crate::feature::Ctx;
use crate::structure::piece::{Piece, PieceBase};
use crate::structure::{GenerationContext, StructureKind, Stub};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::{BlockPos, ChunkPos};

#[derive(Debug)]
pub struct SwampHut;

impl StructureKind for SwampHut {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let (x, z) = (ctx.chunk.min_block_x(), ctx.chunk.min_block_z());
        ctx.on_top_of_chunk_center(HeightmapKind::WorldSurfaceWg, move |ctx: &mut GenerationContext| {
            let direction = PieceBase::random_horizontal_direction(&mut ctx.random);
            vec![Box::new(SwampHutPiece { s: Scattered::new(x, 64, z, 7, 7, 9, direction), spawned_witch: false, spawned_cat: false }) as Box<dyn Piece>]
        })
    }
}

#[derive(Debug)]
pub struct SwampHutPiece {
    s: Scattered,
    spawned_witch: bool,
    spawned_cat: bool,
}

impl Piece for SwampHutPiece {
    fn base(&self) -> &PieceBase {
        &self.s.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.s.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:tesh"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn post_process(&mut self, ctx: &mut Ctx, _random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, _reference: BlockPos) {
        if !self.s.update_average_ground_height(ctx, chunk_bb, 0) {
            return;
        }
        let st = |name: &str| ctx.registries().blocks.parse_state(name).expect("swamp hut block");
        let planks = st("minecraft:spruce_planks");
        let log = st("minecraft:oak_log");
        let fence = st("minecraft:oak_fence");
        let air = st("minecraft:air");
        let pot = st("minecraft:potted_red_mushroom");
        let table = st("minecraft:crafting_table");
        let cauldron = st("minecraft:cauldron");
        let north = st("minecraft:spruce_stairs[facing=north]");
        let east = st("minecraft:spruce_stairs[facing=east]");
        let west = st("minecraft:spruce_stairs[facing=west]");
        let south = st("minecraft:spruce_stairs[facing=south]");
        let north_outer_right = st("minecraft:spruce_stairs[facing=north,shape=outer_right]");
        let north_outer_left = st("minecraft:spruce_stairs[facing=north,shape=outer_left]");
        let south_outer_left = st("minecraft:spruce_stairs[facing=south,shape=outer_left]");
        let south_outer_right = st("minecraft:spruce_stairs[facing=south,shape=outer_right]");
        let p = self.s.base.clone();
        let bb = chunk_bb;
        p.generate_box(ctx, bb, 1, 1, 1, 5, 1, 7, planks, planks, false);
        p.generate_box(ctx, bb, 1, 4, 2, 5, 4, 7, planks, planks, false);
        p.generate_box(ctx, bb, 2, 1, 0, 4, 1, 0, planks, planks, false);
        p.generate_box(ctx, bb, 2, 2, 2, 3, 3, 2, planks, planks, false);
        p.generate_box(ctx, bb, 1, 2, 3, 1, 3, 6, planks, planks, false);
        p.generate_box(ctx, bb, 5, 2, 3, 5, 3, 6, planks, planks, false);
        p.generate_box(ctx, bb, 2, 2, 7, 4, 3, 7, planks, planks, false);
        p.generate_box(ctx, bb, 1, 0, 2, 1, 3, 2, log, log, false);
        p.generate_box(ctx, bb, 5, 0, 2, 5, 3, 2, log, log, false);
        p.generate_box(ctx, bb, 1, 0, 7, 1, 3, 7, log, log, false);
        p.generate_box(ctx, bb, 5, 0, 7, 5, 3, 7, log, log, false);
        p.place_block(ctx, fence, 2, 3, 2, bb);
        p.place_block(ctx, fence, 3, 3, 7, bb);
        p.place_block(ctx, air, 1, 3, 4, bb);
        p.place_block(ctx, air, 5, 3, 4, bb);
        p.place_block(ctx, air, 5, 3, 5, bb);
        p.place_block(ctx, pot, 1, 3, 5, bb);
        p.place_block(ctx, table, 3, 2, 6, bb);
        p.place_block(ctx, cauldron, 4, 2, 6, bb);
        p.place_block(ctx, fence, 1, 2, 1, bb);
        p.place_block(ctx, fence, 5, 2, 1, bb);
        p.generate_box(ctx, bb, 0, 4, 1, 6, 4, 1, north, north, false);
        p.generate_box(ctx, bb, 0, 4, 2, 0, 4, 7, east, east, false);
        p.generate_box(ctx, bb, 6, 4, 2, 6, 4, 7, west, west, false);
        p.generate_box(ctx, bb, 0, 4, 8, 6, 4, 8, south, south, false);
        p.place_block(ctx, north_outer_right, 0, 4, 1, bb);
        p.place_block(ctx, north_outer_left, 6, 4, 1, bb);
        p.place_block(ctx, south_outer_left, 0, 4, 8, bb);
        p.place_block(ctx, south_outer_right, 6, 4, 8, bb);
        for z in [2, 7] {
            for x in [1, 5] {
                p.fill_column_down(ctx, log, x, -1, z, bb);
            }
        }
        let spawn = p.world_pos(2, 2, 5);
        if !self.spawned_witch && chunk_bb.is_inside(spawn) {
            self.spawned_witch = true;
            if ctx.lib.can_spawn("minecraft:witch") {
                spawn_mob(ctx, "minecraft:witch", spawn, false);
            }
        }
        if !self.spawned_cat && chunk_bb.is_inside(spawn) {
            self.spawned_cat = true;
            spawn_mob(ctx, "minecraft:cat", spawn, true);
        }
    }
}

/// `SwampHutPiece.spawnWitch`/`spawnCat`: a persistent mob at the block's
/// bottom center, finalized; the hut is a `#cats_spawn_as_black` structure.
fn spawn_mob(ctx: &mut Ctx, kind: &str, pos: BlockPos, black_cat: bool) {
    let at = [f64::from(pos.x) + 0.5, f64::from(pos.y), f64::from(pos.z) + 0.5];
    if let Some(mut mob) = crate::feature::entities::create(ctx, kind, at, 0.0, 0.0) {
        crate::feature::entities::set(&mut mob, [("PersistenceRequired", minecraftoss_core::nbt::Tag::Byte(1))]);
        crate::feature::entities::finalize_spawn(ctx, &mut mob, black_cat);
        ctx.region.add_entity(mob);
    }
}
