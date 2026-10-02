//! Desert pyramids (vanilla `DesertPyramidStructure`, `DesertPyramidPiece`).
//!
//! Source-informed from the pinned 26.3 JAR. The piece sinks to the lowest
//! ground under it (minus up to two blocks) the first time it is placed;
//! chests are placed once each; the cellar's suspicious sand is chosen in
//! `afterPlace` from a positional random at the start's centre.

use super::scattered::{single_piece, Scattered};
use crate::feature::template::BoundingBox;
use crate::feature::Ctx;
use crate::structure::piece::{Piece, PieceBase};
use crate::structure::{GenerationContext, StructureKind, Stub};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{LegacyRandom, RandomSource, WorldgenRandom};
use minecraftoss_core::{BlockPos, BlockStateId, ChunkPos};

#[derive(Debug)]
pub struct DesertPyramid;

impl StructureKind for DesertPyramid {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        single_piece(ctx, 21, 21, |random, x, z| Box::new(DesertPyramidPiece::new(random, x, z)))
    }

    fn after_place(&self, ctx: &mut Ctx, _random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, pieces: &[Box<dyn Piece>]) {
        let mut unique: Vec<BlockPos> = Vec::new();
        for piece in pieces {
            if let Some(p) = piece.as_any().downcast_ref::<DesertPyramidPiece>() {
                unique.extend(p.sand_positions.iter().copied());
                place_suspicious_sand(ctx, chunk_bb, p.collapsed_roof);
            }
        }
        // `SortedArraySet` by `Vec3i.compareTo`: Y, then Z, then X.
        unique.sort_by_key(|p| (p.y, p.z, p.x));
        unique.dedup();
        let bounds = BoundingBox::encapsulating_all(pieces.iter().map(|p| &p.base().bbox)).expect("pieces");
        let center = bounds.center();
        let mut seed = LegacyRandom::new(ctx.region.world_seed());
        let mut random = seed.fork_positional().at(center.x, center.y, center.z);
        let mut shuffled = unique.clone();
        for i in (2..=shuffled.len()).rev() {
            let j = random.next_i32_bound(i as i32) as usize;
            shuffled.swap(i - 1, j);
        }
        let mut to_place = (unique.len() as i32).min(5 + random.next_i32_bound(3));
        let sand = ctx.registries().blocks.parse_state("minecraft:sand").expect("sand");
        for pos in shuffled {
            if to_place > 0 {
                to_place -= 1;
                place_suspicious_sand(ctx, chunk_bb, pos);
            } else if chunk_bb.is_inside(pos) {
                ctx.set_block(pos, sand);
            }
        }
    }
}

/// `DesertPyramidStructure.placeSuspiciousSand`.
fn place_suspicious_sand(ctx: &mut Ctx, chunk_bb: &BoundingBox, pos: BlockPos) {
    if chunk_bb.is_inside(pos) {
        let state = ctx.registries().blocks.parse_state("minecraft:suspicious_sand").expect("suspicious sand");
        ctx.set_block(pos, state);
        ctx.region.set_brushable_loot(pos.x, pos.y, pos.z, "minecraft:archaeology/desert_pyramid", pos.pack());
    }
}

#[derive(Debug)]
pub struct DesertPyramidPiece {
    s: Scattered,
    has_placed_chest: [bool; 4],
    sand_positions: Vec<BlockPos>,
    collapsed_roof: BlockPos,
}

impl DesertPyramidPiece {
    fn new(random: &mut LegacyRandom, west: i32, north: i32) -> Self {
        let direction = PieceBase::random_horizontal_direction(random);
        Self { s: Scattered::new(west, 64, north, 21, 15, 21, direction), has_placed_chest: [false; 4], sand_positions: Vec::new(), collapsed_roof: BlockPos::new(0, 0, 0) }
    }
}

struct Blocks {
    air: BlockStateId,
    sand: BlockStateId,
    sandstone: BlockStateId,
    cut: BlockStateId,
    chiseled: BlockStateId,
    slab: BlockStateId,
    orange: BlockStateId,
    blue: BlockStateId,
    stairs: BlockStateId,
    plate: BlockStateId,
    tnt: BlockStateId,
}

impl Blocks {
    fn load(ctx: &Ctx) -> Self {
        let s = |name: &str| ctx.registries().blocks.parse_state(name).expect("pyramid block");
        Self {
            air: s("minecraft:air"),
            sand: s("minecraft:sand"),
            sandstone: s("minecraft:sandstone"),
            cut: s("minecraft:cut_sandstone"),
            chiseled: s("minecraft:chiseled_sandstone"),
            slab: s("minecraft:sandstone_slab"),
            orange: s("minecraft:orange_terracotta"),
            blue: s("minecraft:blue_terracotta"),
            stairs: s("minecraft:sandstone_stairs"),
            plate: s("minecraft:stone_pressure_plate"),
            tnt: s("minecraft:tnt"),
        }
    }
}

impl Piece for DesertPyramidPiece {
    fn base(&self) -> &PieceBase {
        &self.s.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.s.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:tedp"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, _reference: BlockPos) {
        let offset = -random.next_i32_bound(3);
        if !self.s.update_to_lowest_ground_height(ctx, offset) {
            return;
        }
        let b = Blocks::load(ctx);
        let (w, d) = (self.s.width, self.s.depth);
        let p = self.s.base.clone();
        let bb = chunk_bb;
        p.generate_box(ctx, bb, 0, -4, 0, w - 1, 0, d - 1, b.sandstone, b.sandstone, false);
        for pos in 1..=9 {
            p.generate_box(ctx, bb, pos, pos, pos, w - 1 - pos, pos, d - 1 - pos, b.sandstone, b.sandstone, false);
            p.generate_box(ctx, bb, pos + 1, pos, pos + 1, w - 2 - pos, pos, d - 2 - pos, b.air, b.air, false);
        }
        for x in 0..w {
            for z in 0..d {
                p.fill_column_down(ctx, b.sandstone, x, -5, z, bb);
            }
        }
        let stairs = |dir: Direction| ctx.with(b.stairs, "facing", dir.name());
        let (north, south, east, west) = (stairs(Direction::North), stairs(Direction::South), stairs(Direction::East), stairs(Direction::West));
        p.generate_box(ctx, bb, 0, 0, 0, 4, 9, 4, b.sandstone, b.air, false);
        p.generate_box(ctx, bb, 1, 10, 1, 3, 10, 3, b.sandstone, b.sandstone, false);
        p.place_block(ctx, north, 2, 10, 0, bb);
        p.place_block(ctx, south, 2, 10, 4, bb);
        p.place_block(ctx, east, 0, 10, 2, bb);
        p.place_block(ctx, west, 4, 10, 2, bb);
        p.generate_box(ctx, bb, w - 5, 0, 0, w - 1, 9, 4, b.sandstone, b.air, false);
        p.generate_box(ctx, bb, w - 4, 10, 1, w - 2, 10, 3, b.sandstone, b.sandstone, false);
        p.place_block(ctx, north, w - 3, 10, 0, bb);
        p.place_block(ctx, south, w - 3, 10, 4, bb);
        p.place_block(ctx, east, w - 5, 10, 2, bb);
        p.place_block(ctx, west, w - 1, 10, 2, bb);
        p.generate_box(ctx, bb, 8, 0, 0, 12, 4, 4, b.sandstone, b.air, false);
        p.generate_box(ctx, bb, 9, 1, 0, 11, 3, 4, b.air, b.air, false);
        for (x, y) in [(9, 1), (9, 2), (9, 3), (10, 3), (11, 3), (11, 2), (11, 1)] {
            p.place_block(ctx, b.cut, x, y, 1, bb);
        }
        p.generate_box(ctx, bb, 4, 1, 1, 8, 3, 3, b.sandstone, b.air, false);
        p.generate_box(ctx, bb, 4, 1, 2, 8, 2, 2, b.air, b.air, false);
        p.generate_box(ctx, bb, 12, 1, 1, 16, 3, 3, b.sandstone, b.air, false);
        p.generate_box(ctx, bb, 12, 1, 2, 16, 2, 2, b.air, b.air, false);
        p.generate_box(ctx, bb, 5, 4, 5, w - 6, 4, d - 6, b.sandstone, b.sandstone, false);
        p.generate_box(ctx, bb, 9, 4, 9, 11, 4, 11, b.air, b.air, false);
        p.generate_box(ctx, bb, 8, 1, 8, 8, 3, 8, b.cut, b.cut, false);
        p.generate_box(ctx, bb, 12, 1, 8, 12, 3, 8, b.cut, b.cut, false);
        p.generate_box(ctx, bb, 8, 1, 12, 8, 3, 12, b.cut, b.cut, false);
        p.generate_box(ctx, bb, 12, 1, 12, 12, 3, 12, b.cut, b.cut, false);
        p.generate_box(ctx, bb, 1, 1, 5, 4, 4, 11, b.sandstone, b.sandstone, false);
        p.generate_box(ctx, bb, w - 5, 1, 5, w - 2, 4, 11, b.sandstone, b.sandstone, false);
        p.generate_box(ctx, bb, 6, 7, 9, 6, 7, 11, b.sandstone, b.sandstone, false);
        p.generate_box(ctx, bb, w - 7, 7, 9, w - 7, 7, 11, b.sandstone, b.sandstone, false);
        p.generate_box(ctx, bb, 5, 5, 9, 5, 7, 11, b.cut, b.cut, false);
        p.generate_box(ctx, bb, w - 6, 5, 9, w - 6, 7, 11, b.cut, b.cut, false);
        p.place_block(ctx, b.air, 5, 5, 10, bb);
        p.place_block(ctx, b.air, 5, 6, 10, bb);
        p.place_block(ctx, b.air, 6, 6, 10, bb);
        p.place_block(ctx, b.air, w - 6, 5, 10, bb);
        p.place_block(ctx, b.air, w - 6, 6, 10, bb);
        p.place_block(ctx, b.air, w - 7, 6, 10, bb);
        p.generate_box(ctx, bb, 2, 4, 4, 2, 6, 4, b.air, b.air, false);
        p.generate_box(ctx, bb, w - 3, 4, 4, w - 3, 6, 4, b.air, b.air, false);
        p.place_block(ctx, north, 2, 4, 5, bb);
        p.place_block(ctx, north, 2, 3, 4, bb);
        p.place_block(ctx, north, w - 3, 4, 5, bb);
        p.place_block(ctx, north, w - 3, 3, 4, bb);
        p.generate_box(ctx, bb, 1, 1, 3, 2, 2, 3, b.sandstone, b.sandstone, false);
        p.generate_box(ctx, bb, w - 3, 1, 3, w - 2, 2, 3, b.sandstone, b.sandstone, false);
        p.place_block(ctx, b.sandstone, 1, 1, 2, bb);
        p.place_block(ctx, b.sandstone, w - 2, 1, 2, bb);
        p.place_block(ctx, b.slab, 1, 2, 2, bb);
        p.place_block(ctx, b.slab, w - 2, 2, 2, bb);
        p.place_block(ctx, west, 2, 1, 2, bb);
        p.place_block(ctx, east, w - 3, 1, 2, bb);
        p.generate_box(ctx, bb, 4, 3, 5, 4, 3, 17, b.sandstone, b.sandstone, false);
        p.generate_box(ctx, bb, w - 5, 3, 5, w - 5, 3, 17, b.sandstone, b.sandstone, false);
        p.generate_box(ctx, bb, 3, 1, 5, 4, 2, 16, b.air, b.air, false);
        p.generate_box(ctx, bb, w - 6, 1, 5, w - 5, 2, 16, b.air, b.air, false);
        for z in (5..=17).step_by(2) {
            p.place_block(ctx, b.cut, 4, 1, z, bb);
            p.place_block(ctx, b.chiseled, 4, 2, z, bb);
            p.place_block(ctx, b.cut, w - 5, 1, z, bb);
            p.place_block(ctx, b.chiseled, w - 5, 2, z, bb);
        }
        for (x, z) in [(10, 7), (10, 8), (9, 9), (11, 9), (8, 10), (12, 10), (7, 10), (13, 10), (9, 11), (11, 11), (10, 12), (10, 13)] {
            p.place_block(ctx, b.orange, x, 0, z, bb);
        }
        p.place_block(ctx, b.blue, 10, 0, 10, bb);
        for x in [0, w - 1] {
            let column = [
                (2, 1, b.cut),
                (2, 2, b.orange),
                (2, 3, b.cut),
                (3, 1, b.cut),
                (3, 2, b.orange),
                (3, 3, b.cut),
                (4, 1, b.orange),
                (4, 2, b.chiseled),
                (4, 3, b.orange),
                (5, 1, b.cut),
                (5, 2, b.orange),
                (5, 3, b.cut),
                (6, 1, b.orange),
                (6, 2, b.chiseled),
                (6, 3, b.orange),
                (7, 1, b.orange),
                (7, 2, b.orange),
                (7, 3, b.orange),
                (8, 1, b.cut),
                (8, 2, b.cut),
                (8, 3, b.cut),
            ];
            for (y, z, state) in column {
                p.place_block(ctx, state, x, y, z, bb);
            }
        }
        for x in [2, w - 3] {
            let front = [
                (-1, 2, b.cut),
                (0, 2, b.orange),
                (1, 2, b.cut),
                (-1, 3, b.cut),
                (0, 3, b.orange),
                (1, 3, b.cut),
                (-1, 4, b.orange),
                (0, 4, b.chiseled),
                (1, 4, b.orange),
                (-1, 5, b.cut),
                (0, 5, b.orange),
                (1, 5, b.cut),
                (-1, 6, b.orange),
                (0, 6, b.chiseled),
                (1, 6, b.orange),
                (-1, 7, b.orange),
                (0, 7, b.orange),
                (1, 7, b.orange),
                (-1, 8, b.cut),
                (0, 8, b.cut),
                (1, 8, b.cut),
            ];
            for (dx, y, state) in front {
                p.place_block(ctx, state, x + dx, y, 0, bb);
            }
        }
        p.generate_box(ctx, bb, 8, 4, 0, 12, 6, 0, b.cut, b.cut, false);
        p.place_block(ctx, b.air, 8, 6, 0, bb);
        p.place_block(ctx, b.air, 12, 6, 0, bb);
        p.place_block(ctx, b.orange, 9, 5, 0, bb);
        p.place_block(ctx, b.chiseled, 10, 5, 0, bb);
        p.place_block(ctx, b.orange, 11, 5, 0, bb);
        p.generate_box(ctx, bb, 8, -14, 8, 12, -11, 12, b.cut, b.cut, false);
        p.generate_box(ctx, bb, 8, -10, 8, 12, -10, 12, b.chiseled, b.chiseled, false);
        p.generate_box(ctx, bb, 8, -9, 8, 12, -9, 12, b.cut, b.cut, false);
        p.generate_box(ctx, bb, 8, -8, 8, 12, -1, 12, b.sandstone, b.sandstone, false);
        p.generate_box(ctx, bb, 9, -11, 9, 11, -1, 11, b.air, b.air, false);
        p.place_block(ctx, b.plate, 10, -11, 10, bb);
        p.generate_box(ctx, bb, 9, -13, 9, 11, -13, 11, b.tnt, b.air, false);
        p.place_block(ctx, b.air, 8, -11, 10, bb);
        p.place_block(ctx, b.air, 8, -10, 10, bb);
        p.place_block(ctx, b.chiseled, 7, -10, 10, bb);
        p.place_block(ctx, b.cut, 7, -11, 10, bb);
        p.place_block(ctx, b.air, 12, -11, 10, bb);
        p.place_block(ctx, b.air, 12, -10, 10, bb);
        p.place_block(ctx, b.chiseled, 13, -10, 10, bb);
        p.place_block(ctx, b.cut, 13, -11, 10, bb);
        p.place_block(ctx, b.air, 10, -11, 8, bb);
        p.place_block(ctx, b.air, 10, -10, 8, bb);
        p.place_block(ctx, b.chiseled, 10, -10, 7, bb);
        p.place_block(ctx, b.cut, 10, -11, 7, bb);
        p.place_block(ctx, b.air, 10, -11, 12, bb);
        p.place_block(ctx, b.air, 10, -10, 12, bb);
        p.place_block(ctx, b.chiseled, 10, -10, 13, bb);
        p.place_block(ctx, b.cut, 10, -11, 13, bb);
        // Direction.Plane.HORIZONTAL order; get2DDataValue indexes the flags.
        for direction in Direction::HORIZONTAL {
            let index = match direction {
                Direction::South => 0,
                Direction::West => 1,
                Direction::North => 2,
                _ => 3,
            };
            if !self.has_placed_chest[index] {
                let (sx, _, sz) = direction.offset();
                self.has_placed_chest[index] = p.create_chest(ctx, bb, random, 10 + sx * 2, -11, 10 + sz * 2, "minecraft:chests/desert_pyramid");
            }
        }
        self.add_cellar(ctx, &b, bb);
    }
}

impl DesertPyramidPiece {
    fn add_cellar(&mut self, ctx: &mut Ctx, b: &Blocks, bb: &BoundingBox) {
        let (x, y, z) = (16, -4, 13);
        let p = self.s.base.clone();
        // Default stairs face north; rotated counterclockwise they face west.
        let stairs = ctx.with(b.stairs, "facing", "west");
        p.place_block(ctx, stairs, 13, -1, 17, bb);
        p.place_block(ctx, stairs, 14, -2, 17, bb);
        p.place_block(ctx, stairs, 15, -3, 17, bb);
        let variant = ctx.region.level_random().next_bool();
        for (dx, dy, state) in [
            (-4, 4, b.sand),
            (-3, 4, b.sand),
            (-2, 4, b.sand),
            (-1, 4, b.sand),
            (0, 4, b.sand),
            (-2, 3, b.sand),
            (-1, 3, if variant { b.sand } else { b.sandstone }),
            (0, 3, if !variant { b.sand } else { b.sandstone }),
            (-1, 2, b.sand),
            (0, 2, b.sandstone),
            (0, 1, b.sand),
        ] {
            p.place_block(ctx, state, x + dx, y + dy, z + 4, bb);
        }
        let (cut, glyphs) = (b.cut, b.chiseled);
        p.generate_box(ctx, bb, x - 3, y + 1, z - 3, x - 3, y + 1, z + 2, cut, cut, true);
        p.generate_box(ctx, bb, x + 3, y + 1, z - 3, x + 3, y + 1, z + 2, cut, cut, true);
        p.generate_box(ctx, bb, x - 3, y + 1, z - 3, x + 3, y + 1, z - 2, cut, cut, true);
        p.generate_box(ctx, bb, x - 3, y + 1, z + 3, x + 3, y + 1, z + 3, cut, cut, true);
        p.generate_box(ctx, bb, x - 3, y + 2, z - 3, x - 3, y + 2, z + 2, glyphs, glyphs, true);
        p.generate_box(ctx, bb, x + 3, y + 2, z - 3, x + 3, y + 2, z + 2, glyphs, glyphs, true);
        p.generate_box(ctx, bb, x - 3, y + 2, z - 3, x + 3, y + 2, z - 2, glyphs, glyphs, true);
        p.generate_box(ctx, bb, x - 3, y + 2, z + 3, x + 3, y + 2, z + 3, glyphs, glyphs, true);
        p.generate_box(ctx, bb, x - 3, -1, z - 3, x - 3, -1, z + 2, cut, cut, true);
        p.generate_box(ctx, bb, x + 3, -1, z - 3, x + 3, -1, z + 2, cut, cut, true);
        p.generate_box(ctx, bb, x - 3, -1, z - 3, x + 3, -1, z - 2, cut, cut, true);
        p.generate_box(ctx, bb, x - 3, -1, z + 3, x + 3, -1, z + 3, cut, cut, true);
        for sy in y + 1..=y + 3 {
            for sx in x - 2..=x + 2 {
                for sz in z - 2..=z + 2 {
                    self.sand_positions.push(p.world_pos(sx, sy, sz));
                }
            }
        }
        self.place_collapsed_roof(ctx, b, bb, x - 2, y + 4, z - 2, x + 2, z + 2);
        let (orange, blue) = (b.orange, b.blue);
        p.place_block(ctx, blue, x, y, z, bb);
        for (dx, dz) in [(1, -1), (1, 1), (-1, -1), (-1, 1), (2, 0), (-2, 0), (0, 2), (0, -2)] {
            p.place_block(ctx, orange, x + dx, y, z + dz, bb);
        }
        p.place_block(ctx, orange, x + 3, y, z, bb);
        self.sand_positions.push(p.world_pos(x + 3, y + 1, z));
        self.sand_positions.push(p.world_pos(x + 3, y + 2, z));
        p.place_block(ctx, cut, x + 4, y + 1, z, bb);
        p.place_block(ctx, glyphs, x + 4, y + 2, z, bb);
        p.place_block(ctx, orange, x - 3, y, z, bb);
        self.sand_positions.push(p.world_pos(x - 3, y + 1, z));
        self.sand_positions.push(p.world_pos(x - 3, y + 2, z));
        p.place_block(ctx, cut, x - 4, y + 1, z, bb);
        p.place_block(ctx, glyphs, x - 4, y + 2, z, bb);
        p.place_block(ctx, orange, x, y, z + 3, bb);
        self.sand_positions.push(p.world_pos(x, y + 1, z + 3));
        self.sand_positions.push(p.world_pos(x, y + 2, z + 3));
        p.place_block(ctx, orange, x, y, z - 3, bb);
        self.sand_positions.push(p.world_pos(x, y + 1, z - 3));
        self.sand_positions.push(p.world_pos(x, y + 2, z - 3));
        p.place_block(ctx, cut, x, y + 1, z - 4, bb);
        p.place_block(ctx, glyphs, x, -2, z - 4, bb);
    }

    #[allow(clippy::too_many_arguments)]
    fn place_collapsed_roof(&mut self, ctx: &mut Ctx, b: &Blocks, bb: &BoundingBox, x0: i32, y0: i32, z0: i32, x1: i32, z1: i32) {
        let p = self.s.base.clone();
        for x in x0..=x1 {
            for z in z0..=z1 {
                let state = if ctx.region.level_random().next_f32() < 0.33 { b.sandstone } else { b.sand };
                p.place_block(ctx, state, x, y0, z, bb);
            }
        }
        let origin = p.world_pos(x0, y0, z0);
        let mut seed = LegacyRandom::new(ctx.region.world_seed());
        let mut random = seed.fork_positional().at(origin.x, origin.y, origin.z);
        let roof_x = x0 + random.next_i32_bound(x1 - x0 + 1);
        let roof_z = z0 + random.next_i32_bound(z1 - z0 + 1);
        self.collapsed_roof = BlockPos::new(p.world_x(roof_x, roof_z), p.world_y(y0), p.world_z(roof_x, roof_z));
    }
}
