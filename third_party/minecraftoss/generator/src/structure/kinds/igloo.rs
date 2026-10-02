//! Igloos (vanilla `IglooStructure`, `IglooPieces`).
//!
//! Source-informed from the pinned 26.3 JAR. Half of all igloos get a
//! basement: a ladder shaft of 3-block segments and a laboratory. Each piece
//! is lifted to the `WORLD_SURFACE_WG` height at the igloo entrance while it
//! is placed.

use super::template_piece::TemplatePiece;
use crate::feature::template::processor::Processor;
use crate::feature::template::{transform, BoundingBox, Mirror, PlaceSettings, Rotation};
use crate::feature::{Ctx, Library};
use crate::structure::piece::{Piece, PieceBase};
use crate::structure::{GenerationContext, PieceList, StructureKind, Stub};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::random::{LegacyRandom, WorldgenRandom};
use minecraftoss_core::{BlockPos, ChunkPos};

const TOP: &str = "minecraft:igloo/top";
const MIDDLE: &str = "minecraft:igloo/middle";
const BOTTOM: &str = "minecraft:igloo/bottom";

fn pivot(name: &str) -> BlockPos {
    match name {
        TOP => BlockPos::new(3, 5, 5),
        MIDDLE => BlockPos::new(1, 3, 1),
        _ => BlockPos::new(3, 6, 7),
    }
}

fn offset(name: &str) -> BlockPos {
    match name {
        TOP => BlockPos::new(0, 0, 0),
        MIDDLE => BlockPos::new(2, -3, 4),
        _ => BlockPos::new(0, -3, -2),
    }
}

#[derive(Debug)]
pub struct Igloo;

impl StructureKind for Igloo {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let (x, z) = (ctx.chunk.min_block_x(), ctx.chunk.min_block_z());
        ctx.on_top_of_chunk_center(HeightmapKind::WorldSurfaceWg, move |ctx: &mut GenerationContext| {
            let rotation = Rotation::random(&mut ctx.random);
            add_pieces(ctx.lib, BlockPos::new(x, 90, z), rotation, &mut ctx.random)
        })
    }
}

/// `IglooPieces.addPieces`.
fn add_pieces(lib: &Library, position: BlockPos, rotation: Rotation, random: &mut LegacyRandom) -> PieceList {
    let mut pieces: PieceList = Vec::new();
    if random.next_f64() < 0.5 {
        let depth = random.next_i32_bound(8) + 4;
        pieces.push(Box::new(IglooPiece::new(lib, BOTTOM, position, rotation, depth * 3)));
        for i in 0..depth - 1 {
            pieces.push(Box::new(IglooPiece::new(lib, MIDDLE, position, rotation, i * 3)));
        }
    }
    pieces.push(Box::new(IglooPiece::new(lib, TOP, position, rotation, 0)));
    pieces
}

fn settings(lib: &Library, rotation: Rotation, name: &str) -> PlaceSettings {
    PlaceSettings {
        rotation,
        mirror: Mirror::None,
        pivot: pivot(name),
        processors: vec![Processor::BlockIgnore(lib.processor_blocks.structure_block())],
        waterlogging: false,
        ..PlaceSettings::default()
    }
}

#[derive(Debug)]
pub struct IglooPiece {
    t: TemplatePiece,
}

impl IglooPiece {
    fn new(lib: &Library, name: &str, position: BlockPos, rotation: Rotation, depth: i32) -> Self {
        let o = offset(name);
        let position = position.offset(o.x, o.y - depth, o.z);
        Self { t: TemplatePiece::new(lib, 0, name, settings(lib, rotation, name), position) }
    }
}

impl Piece for IglooPiece {
    fn base(&self) -> &PieceBase {
        &self.t.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.t.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:iglu"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.t.move_by(dx, dy, dz);
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, reference: BlockPos) {
        let name = self.t.template_name.clone();
        let s = settings(ctx.lib, self.t.settings.rotation, &name);
        let o = offset(&name);
        let entrance = self.t.position.offset_pos(transform::transform(BlockPos::new(3 - o.x, 0, -o.z), s.mirror, s.rotation, s.pivot));
        let height = ctx.height(HeightmapKind::WorldSurfaceWg, entrance.x, entrance.z);
        let old = self.t.position;
        self.t.position = self.t.position.offset(0, height - 90 - 1, 0);
        self.t.place(ctx, random, chunk_bb, reference, &mut |ctx, marker, pos, random, _bb| {
            if marker == "chest" {
                ctx.set_block_update(pos, ctx.lib.blocks.air);
                let below = ctx.name(ctx.block(pos.below()));
                if below.ends_with("chest") && below != "minecraft:ender_chest" {
                    let seed = random.next_i64();
                    ctx.region.set_loot_table(pos.x, pos.y - 1, pos.z, "minecraft:chests/igloo_chest", seed);
                }
            }
        });
        if name == TOP {
            let trapdoor = self.t.position.offset_pos(transform::transform(BlockPos::new(3, 0, 5), s.mirror, s.rotation, s.pivot));
            let below = ctx.block(trapdoor.below());
            if !ctx.is_air(below) && !ctx.is(below, "minecraft:ladder") {
                let snow = ctx.lib.blocks.snow_block;
                ctx.set_block_update(trapdoor, snow);
            }
        }
        self.t.position = old;
    }
}
