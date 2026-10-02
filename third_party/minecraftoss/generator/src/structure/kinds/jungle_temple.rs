//! Jungle temples (vanilla `JungleTempleStructure`, `JungleTemplePiece`).
//!
//! Source-informed from the pinned 26.3 JAR, block by block: random
//! cobblestone/mossy walls, the tripwire and lever puzzles, and each chest
//! and dispenser placed once.

use super::scattered::{single_piece, Scattered};
use crate::feature::template::BoundingBox;
use crate::feature::Ctx;
use crate::structure::piece::{BlockSelector, Piece, PieceBase};
use crate::structure::{GenerationContext, StructureKind, Stub};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::{BlockPos, BlockStateId, ChunkPos};

#[derive(Debug)]
pub struct JungleTemple;

impl StructureKind for JungleTemple {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        single_piece(ctx, 12, 15, |random, x, z| {
            let direction = PieceBase::random_horizontal_direction(random);
            Box::new(JungleTemplePiece {
                s: Scattered::new(x, 64, z, 12, 10, 15, direction),
                placed_main_chest: false,
                placed_hidden_chest: false,
                placed_trap1: false,
                placed_trap2: false,
            })
        })
    }
}

#[derive(Debug)]
pub struct JungleTemplePiece {
    s: Scattered,
    placed_main_chest: bool,
    placed_hidden_chest: bool,
    placed_trap1: bool,
    placed_trap2: bool,
}

/// `JungleTemplePiece.MossStoneSelector`.
struct MossStone {
    cobblestone: BlockStateId,
    mossy: BlockStateId,
}

impl BlockSelector for MossStone {
    fn next(&mut self, _ctx: &Ctx, random: &mut WorldgenRandom, _x: i32, _y: i32, _z: i32, _edge: bool) -> BlockStateId {
        if random.next_f32() < 0.4 { self.cobblestone } else { self.mossy }
    }
}

impl Piece for JungleTemplePiece {
    fn base(&self) -> &PieceBase {
        &self.s.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.s.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:tejp"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, _reference: BlockPos) {
        if !self.s.update_average_ground_height(ctx, chunk_bb, 0) {
            return;
        }
        let st = |name: &str| ctx.registries().blocks.parse_state(name).expect("jungle temple block");
        let air = st("minecraft:air");
        let mossy = st("minecraft:mossy_cobblestone");
        let chiseled = st("minecraft:chiseled_stone_bricks");
        let mut sel = MossStone { cobblestone: st("minecraft:cobblestone"), mossy };
        let east = st("minecraft:cobblestone_stairs[facing=east]");
        let west = st("minecraft:cobblestone_stairs[facing=west]");
        let south = st("minecraft:cobblestone_stairs[facing=south]");
        let north = st("minecraft:cobblestone_stairs[facing=north]");
        let hook_east = st("minecraft:tripwire_hook[facing=east,attached=true]");
        let hook_west = st("minecraft:tripwire_hook[facing=west,attached=true]");
        let hook_north = st("minecraft:tripwire_hook[facing=north,attached=true]");
        let hook_south = st("minecraft:tripwire_hook[facing=south,attached=true]");
        let wire_ew = st("minecraft:tripwire[east=true,west=true,attached=true]");
        let wire_ns = st("minecraft:tripwire[north=true,south=true,attached=true]");
        let redstone_ns = st("minecraft:redstone_wire[north=side,south=side]");
        let redstone_nw = st("minecraft:redstone_wire[north=side,west=side]");
        let redstone_ew = st("minecraft:redstone_wire[east=side,west=side]");
        let redstone_ws = st("minecraft:redstone_wire[west=side,south=side]");
        let redstone_n_su = st("minecraft:redstone_wire[north=side,south=up]");
        let redstone_all = st("minecraft:redstone_wire[north=side,south=side,east=side,west=side]");
        let vine_south = st("minecraft:vine[south=true]");
        let vine_east = st("minecraft:vine[east=true]");
        let lever = st("minecraft:lever[facing=north,face=wall]");
        let piston_up = st("minecraft:sticky_piston[facing=up]");
        let piston_west = st("minecraft:sticky_piston[facing=west]");
        let repeater = st("minecraft:repeater[facing=north]");
        let p = self.s.base.clone();
        let (w, d) = (self.s.width, self.s.depth);
        let bb = chunk_bb;
        let mut stone = |ctx: &mut Ctx, random: &mut WorldgenRandom, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32| {
            p.generate_box_with(ctx, bb, x0, y0, z0, x1, y1, z1, false, random, &mut sel);
        };
        stone(ctx, random, 0, -4, 0, w - 1, 0, d - 1);
        stone(ctx, random, 2, 1, 2, 9, 2, 2);
        stone(ctx, random, 2, 1, 12, 9, 2, 12);
        stone(ctx, random, 2, 1, 3, 2, 2, 11);
        stone(ctx, random, 9, 1, 3, 9, 2, 11);
        stone(ctx, random, 1, 3, 1, 10, 6, 1);
        stone(ctx, random, 1, 3, 13, 10, 6, 13);
        stone(ctx, random, 1, 3, 2, 1, 6, 12);
        stone(ctx, random, 10, 3, 2, 10, 6, 12);
        stone(ctx, random, 2, 3, 2, 9, 3, 12);
        stone(ctx, random, 2, 6, 2, 9, 6, 12);
        stone(ctx, random, 3, 7, 3, 8, 7, 11);
        stone(ctx, random, 4, 8, 4, 7, 8, 10);
        p.generate_air_box(ctx, bb, 3, 1, 3, 8, 2, 11);
        p.generate_air_box(ctx, bb, 4, 3, 6, 7, 3, 9);
        p.generate_air_box(ctx, bb, 2, 4, 2, 9, 5, 12);
        p.generate_air_box(ctx, bb, 4, 6, 5, 7, 6, 9);
        p.generate_air_box(ctx, bb, 5, 7, 6, 6, 7, 8);
        p.generate_air_box(ctx, bb, 5, 1, 2, 6, 2, 2);
        p.generate_air_box(ctx, bb, 5, 2, 12, 6, 2, 12);
        p.generate_air_box(ctx, bb, 5, 5, 1, 6, 5, 1);
        p.generate_air_box(ctx, bb, 5, 5, 13, 6, 5, 13);
        p.place_block(ctx, air, 1, 5, 5, bb);
        p.place_block(ctx, air, 10, 5, 5, bb);
        p.place_block(ctx, air, 1, 5, 9, bb);
        p.place_block(ctx, air, 10, 5, 9, bb);
        for z in [0, 14] {
            stone(ctx, random, 2, 4, z, 2, 5, z);
            stone(ctx, random, 4, 4, z, 4, 5, z);
            stone(ctx, random, 7, 4, z, 7, 5, z);
            stone(ctx, random, 9, 4, z, 9, 5, z);
        }
        stone(ctx, random, 5, 6, 0, 6, 6, 0);
        for x in [0, 11] {
            for z in (2..=12).step_by(2) {
                stone(ctx, random, x, 4, z, x, 5, z);
            }
            stone(ctx, random, x, 6, 5, x, 6, 5);
            stone(ctx, random, x, 6, 9, x, 6, 9);
        }
        stone(ctx, random, 2, 7, 2, 2, 9, 2);
        stone(ctx, random, 9, 7, 2, 9, 9, 2);
        stone(ctx, random, 2, 7, 12, 2, 9, 12);
        stone(ctx, random, 9, 7, 12, 9, 9, 12);
        stone(ctx, random, 4, 9, 4, 4, 9, 4);
        stone(ctx, random, 7, 9, 4, 7, 9, 4);
        stone(ctx, random, 4, 9, 10, 4, 9, 10);
        stone(ctx, random, 7, 9, 10, 7, 9, 10);
        stone(ctx, random, 5, 9, 7, 6, 9, 7);
        p.place_block(ctx, north, 5, 9, 6, bb);
        p.place_block(ctx, north, 6, 9, 6, bb);
        p.place_block(ctx, south, 5, 9, 8, bb);
        p.place_block(ctx, south, 6, 9, 8, bb);
        for x in 4..=7 {
            p.place_block(ctx, north, x, 0, 0, bb);
        }
        p.place_block(ctx, north, 4, 1, 8, bb);
        p.place_block(ctx, north, 4, 2, 9, bb);
        p.place_block(ctx, north, 4, 3, 10, bb);
        p.place_block(ctx, north, 7, 1, 8, bb);
        p.place_block(ctx, north, 7, 2, 9, bb);
        p.place_block(ctx, north, 7, 3, 10, bb);
        stone(ctx, random, 4, 1, 9, 4, 1, 9);
        stone(ctx, random, 7, 1, 9, 7, 1, 9);
        stone(ctx, random, 4, 1, 10, 7, 2, 10);
        stone(ctx, random, 5, 4, 5, 6, 4, 5);
        p.place_block(ctx, east, 4, 4, 5, bb);
        p.place_block(ctx, west, 7, 4, 5, bb);
        for i in 0..4 {
            p.place_block(ctx, south, 5, -i, 6 + i, bb);
            p.place_block(ctx, south, 6, -i, 6 + i, bb);
            p.generate_air_box(ctx, bb, 5, -i, 7 + i, 6, -i, 9 + i);
        }
        p.generate_air_box(ctx, bb, 1, -3, 12, 10, -1, 13);
        p.generate_air_box(ctx, bb, 1, -3, 1, 3, -1, 13);
        p.generate_air_box(ctx, bb, 1, -3, 1, 9, -1, 5);
        for z in (1..=13).step_by(2) {
            stone(ctx, random, 1, -3, z, 1, -2, z);
        }
        for z in (2..=12).step_by(2) {
            stone(ctx, random, 1, -1, z, 3, -1, z);
        }
        stone(ctx, random, 2, -2, 1, 5, -2, 1);
        stone(ctx, random, 7, -2, 1, 9, -2, 1);
        stone(ctx, random, 6, -3, 1, 6, -3, 1);
        stone(ctx, random, 6, -1, 1, 6, -1, 1);
        p.place_block(ctx, hook_east, 1, -3, 8, bb);
        p.place_block(ctx, hook_west, 4, -3, 8, bb);
        p.place_block(ctx, wire_ew, 2, -3, 8, bb);
        p.place_block(ctx, wire_ew, 3, -3, 8, bb);
        for z in (2..=7).rev() {
            p.place_block(ctx, redstone_ns, 5, -3, z, bb);
        }
        p.place_block(ctx, redstone_nw, 5, -3, 1, bb);
        p.place_block(ctx, redstone_ew, 4, -3, 1, bb);
        p.place_block(ctx, mossy, 3, -3, 1, bb);
        if !self.placed_trap1 {
            self.placed_trap1 = p.create_dispenser(ctx, bb, random, 3, -2, 1, Direction::North, "minecraft:chests/jungle_temple_dispenser");
        }
        p.place_block(ctx, vine_south, 3, -2, 2, bb);
        p.place_block(ctx, hook_north, 7, -3, 1, bb);
        p.place_block(ctx, hook_south, 7, -3, 5, bb);
        p.place_block(ctx, wire_ns, 7, -3, 2, bb);
        p.place_block(ctx, wire_ns, 7, -3, 3, bb);
        p.place_block(ctx, wire_ns, 7, -3, 4, bb);
        p.place_block(ctx, redstone_ew, 8, -3, 6, bb);
        p.place_block(ctx, redstone_ws, 9, -3, 6, bb);
        p.place_block(ctx, redstone_n_su, 9, -3, 5, bb);
        p.place_block(ctx, mossy, 9, -3, 4, bb);
        p.place_block(ctx, redstone_ns, 9, -2, 4, bb);
        if !self.placed_trap2 {
            self.placed_trap2 = p.create_dispenser(ctx, bb, random, 9, -2, 3, Direction::West, "minecraft:chests/jungle_temple_dispenser");
        }
        p.place_block(ctx, vine_east, 8, -1, 3, bb);
        p.place_block(ctx, vine_east, 8, -2, 3, bb);
        if !self.placed_main_chest {
            self.placed_main_chest = p.create_chest(ctx, bb, random, 8, -3, 3, "minecraft:chests/jungle_temple");
        }
        for (x, y, z) in [(9, -3, 2), (8, -3, 1), (4, -3, 5), (5, -2, 5), (5, -1, 5), (6, -3, 5), (7, -2, 5), (7, -1, 5), (8, -3, 5)] {
            p.place_block(ctx, mossy, x, y, z, bb);
        }
        stone(ctx, random, 9, -1, 1, 9, -1, 5);
        p.generate_air_box(ctx, bb, 8, -3, 8, 10, -1, 10);
        p.place_block(ctx, chiseled, 8, -2, 11, bb);
        p.place_block(ctx, chiseled, 9, -2, 11, bb);
        p.place_block(ctx, chiseled, 10, -2, 11, bb);
        p.place_block(ctx, lever, 8, -2, 12, bb);
        p.place_block(ctx, lever, 9, -2, 12, bb);
        p.place_block(ctx, lever, 10, -2, 12, bb);
        stone(ctx, random, 8, -3, 8, 8, -3, 10);
        stone(ctx, random, 10, -3, 8, 10, -3, 10);
        p.place_block(ctx, mossy, 10, -2, 9, bb);
        p.place_block(ctx, redstone_ns, 8, -2, 9, bb);
        p.place_block(ctx, redstone_ns, 8, -2, 10, bb);
        p.place_block(ctx, redstone_all, 10, -1, 9, bb);
        p.place_block(ctx, piston_up, 9, -2, 8, bb);
        p.place_block(ctx, piston_west, 10, -2, 8, bb);
        p.place_block(ctx, piston_west, 10, -1, 8, bb);
        p.place_block(ctx, repeater, 10, -2, 10, bb);
        if !self.placed_hidden_chest {
            self.placed_hidden_chest = p.create_chest(ctx, bb, random, 9, -3, 10, "minecraft:chests/jungle_temple");
        }
    }
}
