//! Shipwrecks (vanilla `ShipwreckStructure`, `ShipwreckPieces`).
//!
//! Source-informed from the pinned 26.3 JAR. A wreck settles on the mean
//! ocean floor (or, beached, into the lowest surface) of its footprint the
//! first time a chunk places it; wrecks too big for one generation region
//! are fitted when the start is made instead.

use super::template_piece::TemplatePiece;
use crate::feature::template::processor::Processor;
use crate::feature::template::{BoundingBox, Mirror, PlaceSettings, Rotation};
use crate::feature::{Ctx, Library};
use crate::structure::piece::{Piece, PieceBase};
use crate::structure::{GenerationContext, StructureKind, Stub};
use minecraftoss_core::block::flags;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::random::{RandomSource, WorldgenRandom};
use minecraftoss_core::{BlockPos, ChunkPos};
use serde_json::Value;

const BEACHED: [&str; 11] = [
    "with_mast",
    "sideways_full",
    "sideways_fronthalf",
    "sideways_backhalf",
    "rightsideup_full",
    "rightsideup_fronthalf",
    "rightsideup_backhalf",
    "with_mast_degraded",
    "rightsideup_full_degraded",
    "rightsideup_fronthalf_degraded",
    "rightsideup_backhalf_degraded",
];

const OCEAN: [&str; 20] = [
    "with_mast",
    "upsidedown_full",
    "upsidedown_fronthalf",
    "upsidedown_backhalf",
    "sideways_full",
    "sideways_fronthalf",
    "sideways_backhalf",
    "rightsideup_full",
    "rightsideup_fronthalf",
    "rightsideup_backhalf",
    "with_mast_degraded",
    "upsidedown_full_degraded",
    "upsidedown_fronthalf_degraded",
    "upsidedown_backhalf_degraded",
    "sideways_full_degraded",
    "sideways_fronthalf_degraded",
    "sideways_backhalf_degraded",
    "rightsideup_full_degraded",
    "rightsideup_fronthalf_degraded",
    "rightsideup_backhalf_degraded",
];

#[derive(Debug)]
pub struct Shipwreck {
    beached: bool,
}

impl Shipwreck {
    pub fn parse(json: &Value) -> Result<Self, String> {
        Ok(Self { beached: json.get("is_beached").and_then(Value::as_bool).unwrap_or(false) })
    }
}

impl StructureKind for Shipwreck {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let kind = if self.beached { HeightmapKind::WorldSurfaceWg } else { HeightmapKind::OceanFloorWg };
        let beached = self.beached;
        let offset = BlockPos::new(ctx.chunk.min_block_x(), 90, ctx.chunk.min_block_z());
        ctx.on_top_of_chunk_center(kind, move |ctx: &mut GenerationContext| {
            let rotation = Rotation::random(&mut ctx.random);
            let names: &[&str] = if beached { &BEACHED } else { &OCEAN };
            let name = names[ctx.random.next_i32_bound(names.len() as i32) as usize];
            let mut piece = ShipwreckPiece::new(ctx.lib, &format!("minecraft:shipwreck/{name}"), offset, rotation, beached);
            if piece.too_big() {
                let b = piece.t.base.bbox;
                let height = if beached {
                    let min_y = ctx.lowest_y_at(b.min_x, b.min_z, b.x_span(), b.z_span());
                    piece.beached_position(min_y, &mut ctx.random)
                } else {
                    ctx.mean_first_occupied_height(b.min_x, b.x_span(), b.min_z, b.z_span())
                };
                piece.adjust_height(height);
            }
            vec![Box::new(piece) as Box<dyn Piece>]
        })
    }
}

#[derive(Debug)]
pub struct ShipwreckPiece {
    t: TemplatePiece,
    beached: bool,
    height_adjusted: bool,
}

impl ShipwreckPiece {
    fn new(lib: &Library, name: &str, position: BlockPos, rotation: Rotation, beached: bool) -> Self {
        let settings = PlaceSettings {
            rotation,
            mirror: Mirror::None,
            pivot: BlockPos::new(4, 0, 15),
            processors: vec![Processor::BlockIgnore(lib.processor_blocks.structure_and_air())],
            ..PlaceSettings::default()
        };
        Self { t: TemplatePiece::new(lib, 0, name, settings, position), beached, height_adjusted: false }
    }

    /// `isTooBigToFitInWorldGenRegion`.
    fn too_big(&self) -> bool {
        let s = self.t.template.size;
        s[0] > 32 || s[1] > 32
    }

    /// `calculateBeachedPosition`.
    fn beached_position(&self, min_y: i32, random: &mut impl RandomSource) -> i32 {
        min_y - self.t.template.size[1] / 2 - random.next_i32_bound(3)
    }

    /// `adjustPositionHeight`: only the template position moves.
    fn adjust_height(&mut self, height: i32) {
        self.height_adjusted = true;
        self.t.position = BlockPos::new(self.t.position.x, height, self.t.position.z);
    }
}

impl Piece for ShipwreckPiece {
    fn base(&self) -> &PieceBase {
        &self.t.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.t.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:shipwreck"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.t.move_by(dx, dy, dz);
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, reference: BlockPos) {
        if !self.height_adjusted && !self.too_big() {
            let size = self.t.template.size;
            let kind = if self.beached { HeightmapKind::WorldSurfaceWg } else { HeightmapKind::OceanFloorWg };
            let base = size[0] * size[2];
            let p = self.t.position;
            let (mut min_y, mut mean) = (ctx.max_y() + 1, 0);
            if base == 0 {
                mean = ctx.height(kind, p.x, p.z);
            } else {
                for z in p.z..p.z + size[2] {
                    for x in p.x..p.x + size[0] {
                        let h = ctx.height(kind, x, z);
                        mean += h;
                        min_y = min_y.min(h);
                    }
                }
                mean /= base;
            }
            let height = if self.beached { self.beached_position(min_y, random) } else { mean };
            self.adjust_height(height);
        }
        self.t.place(ctx, random, chunk_bb, reference, &mut |ctx, marker, pos, random, _bb| {
            let loot = match marker {
                "map_chest" => "minecraft:chests/shipwreck_map",
                "treasure_chest" => "minecraft:chests/shipwreck_treasure",
                "supply_chest" => "minecraft:chests/shipwreck_supply",
                _ => return,
            };
            let chest = pos.below();
            let state = ctx.block(chest);
            let name = ctx.name(state);
            if ctx.registries().blocks.is(state, flags::HAS_BLOCK_ENTITY) && (name.ends_with("chest") && name != "minecraft:ender_chest" || name == "minecraft:barrel") {
                let seed = random.next_i64();
                ctx.region.set_loot_table(chest.x, chest.y, chest.z, loot, seed);
            }
        });
    }
}
