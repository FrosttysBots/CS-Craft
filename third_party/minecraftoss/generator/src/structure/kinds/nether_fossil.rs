//! Nether fossils (vanilla `NetherFossilStructure`, `NetherFossilPieces`).
//!
//! Source-informed from the pinned 26.3 JAR. The start searches the raw
//! noise column down to open air above soul sand or a sturdy floor. When
//! placed, the piece widens the chunk box it was given to the whole fossil
//! (vanilla mutates it), so each chunk that places it writes all of it that
//! the region allows; half of all fossils get a dried ghast.

use super::template_piece::TemplatePiece;
use crate::feature::blocks::Behaviour;
use crate::feature::template::processor::Processor;
use crate::feature::template::{BoundingBox, Mirror, PlaceSettings, Rotation};
use crate::feature::{Ctx, Library};
use crate::providers::HeightProvider;
use crate::structure::piece::{Piece, PieceBase};
use crate::structure::{GenerationContext, StructureKind, Stub};
use minecraftoss_core::block::SupportType;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{LegacyRandom, WorldgenRandom};
use minecraftoss_core::{BlockPos, BlockStateId, ChunkPos};
use serde_json::Value;

#[derive(Debug)]
pub struct NetherFossil {
    height: HeightProvider,
}

impl NetherFossil {
    pub fn parse(json: &Value) -> Result<Self, String> {
        Ok(Self { height: HeightProvider::parse(&json["height"])? })
    }
}

impl StructureKind for NetherFossil {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let x = ctx.chunk.min_block_x() + ctx.random.next_i32_bound(16);
        let z = ctx.chunk.min_block_z() + ctx.random.next_i32_bound(16);
        let sea_level = ctx.terrain.sea_level;
        let mut y = self.height.sample(&mut ctx.random, &ctx.lib.generation);
        let (min_y, column) = ctx.base_column(x, z);
        let block = |y: i32| -> BlockStateId { usize::try_from(y - min_y).ok().and_then(|i| column.get(i)).copied().unwrap_or(BlockStateId::AIR) };
        let behaviour = Behaviour { registries: &ctx.lib.registries };
        let soul_sand = ctx.lib.registries.blocks.block_by_name("minecraft:soul_sand");
        while y > sea_level {
            let current = block(y);
            y -= 1;
            let below = block(y);
            let floor = Some(ctx.lib.registries.blocks.block_of(below)) == soul_sand || behaviour.is_face_sturdy_as(below, Direction::Up, SupportType::Full);
            if behaviour.is_air(current) && floor {
                break;
            }
        }
        if y <= sea_level {
            return None;
        }
        let position = BlockPos::new(x, y, z);
        Some(Stub::deferred(position, move |ctx: &mut GenerationContext| {
            let rotation = Rotation::random(&mut ctx.random);
            let index = ctx.random.next_i32_bound(14) + 1;
            let name = format!("minecraft:nether_fossils/fossil_{index}");
            vec![Box::new(NetherFossilPiece::new(ctx.lib, &name, position, rotation)) as Box<dyn Piece>]
        }))
    }
}

#[derive(Debug)]
pub struct NetherFossilPiece {
    t: TemplatePiece,
}

impl NetherFossilPiece {
    fn new(lib: &Library, name: &str, position: BlockPos, rotation: Rotation) -> Self {
        let settings = PlaceSettings {
            rotation,
            mirror: Mirror::None,
            processors: vec![Processor::BlockIgnore(lib.processor_blocks.structure_and_air())],
            ..PlaceSettings::default()
        };
        Self { t: TemplatePiece::new(lib, 0, name, settings, position) }
    }
}

impl Piece for NetherFossilPiece {
    fn base(&self) -> &PieceBase {
        &self.t.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.t.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:nefos"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.t.move_by(dx, dy, dz);
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, reference: BlockPos) {
        let fossil = self.t.template.bounding_box(&self.t.settings, self.t.position);
        let widened = BoundingBox::encapsulating(chunk_bb, &fossil);
        self.t.place(ctx, random, &widened, reference, &mut |_, _, _, _, _| {});
        // `placeDriedGhast`.
        let center = fossil.center();
        let mut seed = LegacyRandom::new(ctx.region.world_seed());
        let mut r = seed.fork_positional().at(center.x, center.y, center.z);
        if r.next_f32() < 0.5 {
            let x = fossil.min_x + r.next_i32_bound(fossil.x_span());
            let y = fossil.min_y;
            let z = fossil.min_z + r.next_i32_bound(fossil.z_span());
            let pos = BlockPos::new(x, y, z);
            if ctx.is_air(ctx.block(pos)) && widened.is_inside(pos) {
                let ghast = ctx.registries().blocks.parse_state("minecraft:dried_ghast").expect("dried ghast");
                let rotation = Rotation::random(&mut r);
                let state = ctx.lib.transforms.rotate(ghast, rotation);
                ctx.set_block(pos, state);
            }
        }
    }
}
