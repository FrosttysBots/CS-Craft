//! Ocean ruins (vanilla `OceanRuinStructure`, `OceanRuinPieces`).
//!
//! Source-informed from the pinned 26.3 JAR. Every placement re-fits the
//! ruin to the ocean floor (vanilla keeps no flag), sinking it one block
//! into uneven ground; large ruins may bring a cluster of small ones.

use super::template_piece::TemplatePiece;
use crate::feature::blocks::FluidType;
use crate::feature::template::processor::Processor;
use crate::feature::template::{transform, BoundingBox, Mirror, PlaceSettings, Rotation};
use crate::feature::{Ctx, Library};
use crate::structure::piece::{Piece, PieceBase};
use crate::structure::{GenerationContext, PieceList, StructureKind, Stub};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::random::{LegacyRandom, RandomSource, WorldgenRandom};
use minecraftoss_core::{BlockPos, ChunkPos};
use serde_json::Value;

#[derive(Debug)]
pub struct OceanRuin {
    warm: bool,
    large_probability: f32,
    cluster_probability: f32,
}

impl OceanRuin {
    pub fn parse(json: &Value) -> Result<Self, String> {
        Ok(Self {
            warm: json.get("biome_temp").and_then(Value::as_str) != Some("cold"),
            large_probability: json["large_probability"].as_f64().ok_or("ocean ruin lacks large_probability")? as f32,
            cluster_probability: json["cluster_probability"].as_f64().ok_or("ocean ruin lacks cluster_probability")? as f32,
        })
    }
}

/// `Mth.nextInt(random, min, max)`.
fn next_int(random: &mut impl RandomSource, min: i32, max: i32) -> i32 {
    if min >= max { min } else { random.next_i32_bound(max - min + 1) + min }
}

impl StructureKind for OceanRuin {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let offset = BlockPos::new(ctx.chunk.min_block_x(), 90, ctx.chunk.min_block_z());
        ctx.on_top_of_chunk_center(HeightmapKind::OceanFloorWg, move |ctx: &mut GenerationContext| {
            let rotation = Rotation::random(&mut ctx.random);
            let mut pieces: PieceList = Vec::new();
            let large = ctx.random.next_f32() <= self.large_probability;
            let integrity = if large { 0.9 } else { 0.8 };
            self.add_piece(ctx.lib, offset, rotation, &mut pieces, &mut ctx.random, large, integrity);
            if large && ctx.random.next_f32() <= self.cluster_probability {
                self.add_cluster(ctx.lib, &mut ctx.random, rotation, offset, &mut pieces);
            }
            pieces
        })
    }
}

const WARM: usize = 8;

impl OceanRuin {
    #[allow(clippy::too_many_arguments)]
    fn add_piece(&self, lib: &Library, position: BlockPos, rotation: Rotation, pieces: &mut PieceList, random: &mut LegacyRandom, large: bool, integrity: f32) {
        let make = |name: String, integrity: f32| Box::new(OceanRuinPiece::new(lib, &name, position, rotation, integrity, self.warm, large)) as Box<dyn Piece>;
        if self.warm {
            let name = if large {
                let big = [4, 5, 6, 7];
                format!("minecraft:underwater_ruin/big_warm_{}", big[random.next_i32_bound(4) as usize])
            } else {
                format!("minecraft:underwater_ruin/warm_{}", random.next_i32_bound(WARM as i32) + 1)
            };
            pieces.push(make(name, integrity));
        } else {
            let (count, prefix) = if large { (4, "big_") } else { (8, "") };
            let index = random.next_i32_bound(count);
            let suffix = if large { [1, 2, 3, 8][index as usize] } else { index + 1 };
            pieces.push(make(format!("minecraft:underwater_ruin/{prefix}brick_{suffix}"), integrity));
            pieces.push(make(format!("minecraft:underwater_ruin/{prefix}cracked_{suffix}"), 0.7));
            pieces.push(make(format!("minecraft:underwater_ruin/{prefix}mossy_{suffix}"), 0.5));
        }
    }

    /// `OceanRuinPieces.addClusterRuins`.
    fn add_cluster(&self, lib: &Library, random: &mut LegacyRandom, rotation: Rotation, p: BlockPos, pieces: &mut PieceList) {
        let parent_pos = BlockPos::new(p.x, 90, p.z);
        let parent_corner = transform::transform(BlockPos::new(15, 0, 15), Mirror::None, rotation, BlockPos::new(0, 0, 0)).offset_pos(parent_pos);
        let parent_bb = BoundingBox::from_corners(parent_pos, parent_corner);
        let origin = BlockPos::new(parent_pos.x.min(parent_corner.x), parent_pos.y, parent_pos.z.min(parent_corner.z));
        let mut positions = vec![
            origin.offset(-16 + next_int(random, 1, 8), 0, 16 + next_int(random, 1, 7)),
            origin.offset(-16 + next_int(random, 1, 8), 0, next_int(random, 1, 7)),
            origin.offset(-16 + next_int(random, 1, 8), 0, -16 + next_int(random, 4, 8)),
            origin.offset(next_int(random, 1, 7), 0, 16 + next_int(random, 1, 7)),
            origin.offset(next_int(random, 1, 7), 0, -16 + next_int(random, 4, 6)),
            origin.offset(16 + next_int(random, 1, 7), 0, 16 + next_int(random, 3, 8)),
            origin.offset(16 + next_int(random, 1, 7), 0, next_int(random, 1, 7)),
            origin.offset(16 + next_int(random, 1, 7), 0, -16 + next_int(random, 4, 8)),
        ];
        let ruins = next_int(random, 4, 8);
        for _ in 0..ruins {
            if positions.is_empty() {
                continue;
            }
            let index = random.next_i32_bound(positions.len() as i32) as usize;
            let pos = positions.remove(index);
            let next_rotation = Rotation::random(random);
            let corner = transform::transform(BlockPos::new(5, 0, 6), Mirror::None, next_rotation, BlockPos::new(0, 0, 0)).offset_pos(pos);
            if !BoundingBox::from_corners(pos, corner).intersects(&parent_bb) {
                self.add_piece(lib, pos, next_rotation, pieces, random, false, 0.8);
            }
        }
    }
}

#[derive(Debug)]
pub struct OceanRuinPiece {
    t: TemplatePiece,
    large: bool,
}

impl OceanRuinPiece {
    fn new(lib: &Library, name: &str, position: BlockPos, rotation: Rotation, integrity: f32, warm: bool, large: bool) -> Self {
        let archaeology = if warm {
            Processor::archaeology(&lib.registries, "minecraft:sand", "minecraft:suspicious_sand", "minecraft:archaeology/ocean_ruin_warm")
        } else {
            Processor::archaeology(&lib.registries, "minecraft:gravel", "minecraft:suspicious_gravel", "minecraft:archaeology/ocean_ruin_cold")
        }
        .expect("archaeology processor");
        let settings = PlaceSettings {
            rotation,
            mirror: Mirror::None,
            processors: vec![
                Processor::BlockRot { rottable: None, integrity },
                Processor::BlockIgnore(lib.processor_blocks.structure_and_air()),
                archaeology,
            ],
            ..PlaceSettings::default()
        };
        Self { t: TemplatePiece::new(lib, 0, name, settings, position), large }
    }

    /// `OceanRuinPiece.getHeight`: sink one block into uneven ground.
    fn fitted_height(ctx: &Ctx, pos: BlockPos, corner: BlockPos) -> i32 {
        let mut new_y = pos.y;
        let mut min_y = 512;
        let top_y = new_y - 1;
        let mut area = 0;
        let ice = ctx.registries().block_tags.id("minecraft:ice");
        for z in pos.z.min(corner.z)..=pos.z.max(corner.z) {
            for x in pos.x.min(corner.x)..=pos.x.max(corner.x) {
                let mut floor_y = pos.y - 1;
                loop {
                    let state = ctx.block(BlockPos::new(x, floor_y, z));
                    let fluid = ctx.fluid(state);
                    let water = matches!(fluid, FluidType::Water | FluidType::FlowingWater);
                    let icy = ice.is_some_and(|tag| ctx.in_tag(state, tag));
                    if !((ctx.is_air(state) || water || icy) && floor_y > ctx.min_y() + 1) {
                        break;
                    }
                    floor_y -= 1;
                }
                min_y = min_y.min(floor_y);
                if floor_y < top_y - 2 {
                    area += 1;
                }
            }
        }
        let width = (pos.x - corner.x).abs();
        if top_y - min_y > 2 && area > width - 2 {
            new_y = min_y + 1;
        }
        new_y
    }
}

impl Piece for OceanRuinPiece {
    fn base(&self) -> &PieceBase {
        &self.t.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.t.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:orp"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.t.move_by(dx, dy, dz);
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, reference: BlockPos) {
        let p = self.t.position;
        let height = ctx.height(HeightmapKind::OceanFloorWg, p.x, p.z);
        self.t.position = BlockPos::new(p.x, height, p.z);
        let size = self.t.template.size;
        let corner = transform::transform(BlockPos::new(size[0] - 1, 0, size[2] - 1), Mirror::None, self.t.settings.rotation, BlockPos::new(0, 0, 0)).offset_pos(self.t.position);
        let fitted = Self::fitted_height(ctx, self.t.position, corner);
        self.t.position = BlockPos::new(p.x, fitted, p.z);
        let large = self.large;
        let sea_level = ctx.lib.generation.sea_level;
        self.t.place(ctx, random, chunk_bb, reference, &mut |ctx, marker, pos, random, _bb| {
            if marker == "chest" {
                let water = matches!(ctx.fluid_at(pos), FluidType::Water | FluidType::FlowingWater);
                let chest = ctx.registries().blocks.parse_state("minecraft:chest").expect("chest");
                let chest = ctx.with(chest, "waterlogged", if water { "true" } else { "false" });
                ctx.set_block(pos, chest);
                let seed = random.next_i64();
                let loot = if large { "minecraft:chests/underwater_ruin_big" } else { "minecraft:chests/underwater_ruin_small" };
                ctx.region.set_loot_table(pos.x, pos.y, pos.z, loot, seed);
            } else if marker == "drowned" && ctx.lib.can_spawn("minecraft:drowned") {
                let at = [f64::from(pos.x) + 0.5, f64::from(pos.y), f64::from(pos.z) + 0.5];
                if let Some(mut entity) = crate::feature::entities::create(ctx, "minecraft:drowned", at, 0.0, 0.0) {
                    crate::feature::entities::set(&mut entity, [("PersistenceRequired", minecraftoss_core::nbt::Tag::Byte(1))]);
                    crate::feature::entities::finalize_spawn(ctx, &mut entity, false);
                    ctx.region.add_entity(entity);
                }
                let state = if pos.y > sea_level { ctx.lib.blocks.air } else { ctx.lib.blocks.water };
                ctx.set_block(pos, state);
            }
        });
    }
}
