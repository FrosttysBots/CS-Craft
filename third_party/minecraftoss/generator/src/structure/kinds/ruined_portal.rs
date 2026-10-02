//! Ruined portals (vanilla `RuinedPortalStructure`, `RuinedPortalPiece`).
//!
//! Source-informed from the pinned 26.3 JAR. A portal is placed only by the
//! chunk holding its centre, which widens its box to the whole portal; the
//! netherrack spread, drip columns, vines and leaves follow in vanilla's
//! iteration order.

use super::template_piece::TemplatePiece;
use crate::feature::blocks::BlockSet;
use crate::feature::rule_test::RuleTest;
use crate::feature::survive::is_face_full;
use crate::feature::template::processor::{BlockEntityModifier, Processor, ProcessorRule};
use crate::feature::template::{BoundingBox, Mirror, PlaceSettings, Rotation};
use crate::feature::{Ctx, Library};
use crate::structure::piece::{Piece, PieceBase};
use crate::structure::{GenerationContext, StructureKind, Stub};
use minecraftoss_core::block::FaceShape;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::random::{LegacyRandom, RandomSource, WorldgenRandom};
use minecraftoss_core::{BlockPos, BlockStateId, ChunkPos};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placement {
    OnLandSurface,
    PartlyBuried,
    OnOceanFloor,
    InMountain,
    Underground,
    InNether,
}

#[derive(Clone, Copy, Debug)]
struct Setup {
    placement: Placement,
    air_pocket_probability: f32,
    mossiness: f32,
    overgrown: bool,
    vines: bool,
    can_be_cold: bool,
    replace_with_blackstone: bool,
    weight: f32,
}

#[derive(Clone, Copy, Debug)]
struct Properties {
    cold: bool,
    mossiness: f32,
    air_pocket: bool,
    overgrown: bool,
    vines: bool,
    replace_with_blackstone: bool,
}

#[derive(Debug)]
pub struct RuinedPortal {
    setups: Vec<Setup>,
}

impl RuinedPortal {
    pub fn parse(json: &Value) -> Result<Self, String> {
        let setups = json["setups"]
            .as_array()
            .ok_or("ruined portal lacks setups")?
            .iter()
            .map(|s| {
                let f = |k: &str| s.get(k).and_then(Value::as_f64).map_or(0.0, |v| v as f32);
                let b = |k: &str| s.get(k).and_then(Value::as_bool).unwrap_or(false);
                let placement = match s["placement"].as_str().unwrap_or("on_land_surface") {
                    "partly_buried" => Placement::PartlyBuried,
                    "on_ocean_floor" => Placement::OnOceanFloor,
                    "in_mountain" => Placement::InMountain,
                    "underground" => Placement::Underground,
                    "in_nether" => Placement::InNether,
                    _ => Placement::OnLandSurface,
                };
                Setup {
                    placement,
                    air_pocket_probability: f("air_pocket_probability"),
                    mossiness: f("mossiness"),
                    overgrown: b("overgrown"),
                    vines: b("vines"),
                    can_be_cold: b("can_be_cold"),
                    replace_with_blackstone: b("replace_with_blackstone"),
                    weight: f("weight"),
                }
            })
            .collect();
        Ok(Self { setups })
    }
}

fn heightmap(placement: Placement) -> HeightmapKind {
    if placement == Placement::OnOceanFloor { HeightmapKind::OceanFloorWg } else { HeightmapKind::WorldSurfaceWg }
}

/// `Mth.randomBetweenInclusive`.
fn between(random: &mut impl RandomSource, min: i32, max: i32) -> i32 {
    random.next_i32_bound(max - min + 1) + min
}

impl StructureKind for RuinedPortal {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let random = &mut ctx.random;
        let setup = if self.setups.len() > 1 {
            let total: f32 = self.setups.iter().map(|s| s.weight).sum();
            let mut pick = random.next_f32();
            let mut chosen = None;
            for s in &self.setups {
                pick -= s.weight / total;
                if pick < 0.0 {
                    chosen = Some(*s);
                    break;
                }
            }
            chosen?
        } else {
            self.setups[0]
        };
        let air_pocket = match setup.air_pocket_probability {
            p if p == 0.0 => false,
            p if p == 1.0 => true,
            p => random.next_f32() < p,
        };
        let name = if random.next_f32() < 0.05 {
            format!("minecraft:ruined_portal/giant_portal_{}", random.next_i32_bound(3) + 1)
        } else {
            format!("minecraft:ruined_portal/portal_{}", random.next_i32_bound(10) + 1)
        };
        let template = ctx.lib.templates.get(&ctx.lib.registries, &name);
        let rotation = Rotation::ALL[random.next_i32_bound(4) as usize];
        let mirror = if random.next_f32() < 0.5 { Mirror::None } else { Mirror::FrontBack };
        let pivot = BlockPos::new(template.size[0] / 2, 0, template.size[2] / 2);
        let base = BlockPos::new(ctx.chunk.min_block_x(), 0, ctx.chunk.min_block_z());
        let settings = PlaceSettings { rotation, mirror, pivot, ..PlaceSettings::default() };
        let bbox = template.bounding_box(&settings, base);
        let center = bbox.center();
        let surface_y = ctx.first_free_height(center.x, center.z, heightmap(setup.placement)) - 1;
        let projected = find_suitable_y(ctx, setup.placement, air_pocket, surface_y, bbox.y_span(), &bbox);
        let origin = BlockPos::new(base.x, projected, base.z);
        Some(Stub::deferred(origin, move |ctx: &mut GenerationContext| {
            let cold = setup.can_be_cold && {
                let biome = ctx.terrain.biome_at_quart(origin.x >> 2, origin.y >> 2, origin.z >> 2);
                let info = ctx.lib.registries.biomes.get(biome);
                let temperature = ctx.lib.temperature.at(info.temperature, info.frozen_temperature_modifier, origin.x, origin.y, origin.z, ctx.terrain.sea_level);
                temperature < 0.15
            };
            let properties = Properties {
                cold,
                mossiness: setup.mossiness,
                air_pocket,
                overgrown: setup.overgrown,
                vines: setup.vines,
                replace_with_blackstone: setup.replace_with_blackstone,
            };
            vec![Box::new(RuinedPortalPiece::new(ctx.lib, &name, origin, setup.placement, properties, rotation, mirror, pivot)) as Box<dyn Piece>]
        }))
    }
}

/// `RuinedPortalStructure.findSuitableY`.
fn find_suitable_y(ctx: &mut GenerationContext, placement: Placement, air_pocket: bool, surface_y: i32, y_span: i32, bbox: &BoundingBox) -> i32 {
    let min_y = ctx.min_y() + 15;
    let random = &mut ctx.random;
    let within = |random: &mut LegacyRandom, min: i32, max: i32| if min < max { between(random, min, max) } else { max };
    let new_y = match placement {
        Placement::InNether => {
            if air_pocket {
                between(random, 32, 100)
            } else if random.next_f32() < 0.5 {
                between(random, 27, 29)
            } else {
                between(random, 29, 100)
            }
        }
        Placement::InMountain => within(random, 70, surface_y - y_span),
        Placement::Underground => within(random, min_y, surface_y - y_span),
        Placement::PartlyBuried => surface_y - y_span + between(random, 2, 8),
        _ => surface_y,
    };
    let corners = [(bbox.min_x, bbox.min_z), (bbox.max_x, bbox.min_z), (bbox.min_x, bbox.max_z), (bbox.max_x, bbox.max_z)];
    let columns: Vec<(i32, Vec<BlockStateId>)> = corners.iter().map(|&(x, z)| ctx.base_column(x, z)).collect();
    let kind = if placement == Placement::OnOceanFloor { HeightmapKind::OceanFloorWg } else { HeightmapKind::WorldSurfaceWg };
    let registries = &ctx.lib.registries;
    let mut y = new_y;
    while y > min_y {
        let mut solid = 0;
        for (column_min, column) in &columns {
            let state = usize::try_from(y - column_min).ok().and_then(|i| column.get(i)).copied().unwrap_or(BlockStateId::AIR);
            if registries.heightmap_mask(state) & kind.bit() != 0 {
                solid += 1;
                if solid == 3 {
                    return y;
                }
            }
        }
        y -= 1;
    }
    y
}

#[derive(Debug)]
pub struct RuinedPortalPiece {
    t: TemplatePiece,
    placement: Placement,
    properties: Properties,
}

impl RuinedPortalPiece {
    #[allow(clippy::too_many_arguments)]
    fn new(lib: &Library, name: &str, position: BlockPos, placement: Placement, properties: Properties, rotation: Rotation, mirror: Mirror, pivot: BlockPos) -> Self {
        let registries = &lib.registries;
        let block = |n: &str| registries.blocks.block_by_name(n).expect("portal block");
        let state = |n: &str| registries.blocks.parse_state(n).expect("portal block");
        let replace = |source: &str, probability: Option<f32>, target: &str| {
            let test = match probability {
                Some(p) => RuleTest::RandomBlock(block(source), p),
                None => RuleTest::Block(block(source)),
            };
            ProcessorRule::new(test, state(target), BlockEntityModifier::Passthrough)
        };
        let mut rules = vec![replace("minecraft:gold_block", Some(0.3), "minecraft:air")];
        rules.push(if placement == Placement::OnOceanFloor {
            replace("minecraft:lava", None, "minecraft:magma_block")
        } else if properties.cold {
            replace("minecraft:lava", None, "minecraft:netherrack")
        } else {
            replace("minecraft:lava", Some(0.2), "minecraft:magma_block")
        });
        if !properties.cold {
            rules.push(replace("minecraft:netherrack", Some(0.07), "minecraft:magma_block"));
        }
        let ignore = if properties.air_pocket { lib.processor_blocks.structure_block() } else { lib.processor_blocks.structure_and_air() };
        let protected = BlockSet::Tag(registries.block_tags.require("minecraft:features_cannot_replace").expect("tag"));
        let mut processors = vec![
            Processor::BlockIgnore(ignore),
            Processor::Rule(rules),
            Processor::BlockAge { mossiness: properties.mossiness },
            Processor::ProtectedBlocks(protected),
            Processor::LavaSubmerged,
        ];
        if properties.replace_with_blackstone {
            processors.push(Processor::BlackstoneReplace);
        }
        let settings = PlaceSettings { rotation, mirror, pivot, processors, ..PlaceSettings::default() };
        Self { t: TemplatePiece::new(lib, 0, name, settings, position), placement, properties }
    }

    fn replaceable(&self, ctx: &Ctx, state: BlockStateId) -> bool {
        let tag = ctx.registries().block_tags.id("minecraft:features_cannot_replace");
        !ctx.is(state, "minecraft:air")
            && !ctx.is(state, "minecraft:obsidian")
            && !tag.is_some_and(|t| ctx.in_tag(state, t))
            && (self.placement == Placement::InNether || !ctx.is(state, "minecraft:lava"))
    }

    fn place_netherrack_or_magma(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos) {
        if !self.replaceable(ctx, ctx.block(pos)) {
            return;
        }
        let name = if !self.properties.cold && random.next_f32() < 0.07 { "minecraft:magma_block" } else { "minecraft:netherrack" };
        let state = ctx.registries().blocks.parse_state(name).expect("netherrack");
        ctx.set_block_update(pos, state);
    }

    fn drip_column(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos) {
        let mut current = pos;
        self.place_netherrack_or_magma(ctx, random, current);
        let mut cap = 8;
        while cap > 0 && random.next_f32() < 0.5 {
            current = current.below();
            cap -= 1;
            self.place_netherrack_or_magma(ctx, random, current);
        }
    }

    fn maybe_add_leaves_above(ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos) {
        if random.next_f32() < 0.5 && ctx.is(ctx.block(pos), "minecraft:netherrack") && ctx.is_air(ctx.block(pos.above())) {
            let leaves = ctx.registries().blocks.parse_state("minecraft:jungle_leaves[persistent=true]").expect("leaves");
            ctx.set_block_update(pos.above(), leaves);
        }
    }

    fn maybe_add_vines(ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos) {
        let state = ctx.block(pos);
        if ctx.is_air(state) || ctx.is(state, "minecraft:vine") {
            return;
        }
        let direction = PieceBase::random_horizontal_direction(random);
        let neighbour = pos.relative(direction, 1);
        if !ctx.is_air(ctx.block(neighbour)) {
            return;
        }
        let shape = ctx.registries().blocks.collision_shape(state).cloned().unwrap_or(FaceShape::Empty);
        if is_face_full(&shape, direction) {
            let vine = ctx.with(ctx.lib.blocks.vine, direction.opposite().name(), "true");
            ctx.set_block_update(neighbour, vine);
        }
    }

    /// `spreadNetherrack`.
    fn spread_netherrack(&self, ctx: &mut Ctx, random: &mut WorldgenRandom) {
        let follow_ground = matches!(self.placement, Placement::OnLandSurface | Placement::OnOceanFloor);
        let b = self.t.base.bbox;
        let center = b.center();
        const PROBABILITY: [f32; 14] = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.9, 0.9, 0.8, 0.7, 0.6, 0.4, 0.2];
        let max_distance = PROBABILITY.len() as i32;
        let average_width = (b.x_span() + b.z_span()) / 2;
        let adjustment = random.next_i32_bound(1.max(8 - average_width / 2));
        for x in center.x - max_distance..=center.x + max_distance {
            for z in center.z - max_distance..=center.z + max_distance {
                let distance = (x - center.x).abs() + (z - center.z).abs();
                let adjusted = 0.max(distance + adjustment);
                if adjusted >= max_distance {
                    continue;
                }
                if random.next_f64() >= f64::from(PROBABILITY[adjusted as usize]) {
                    continue;
                }
                let surface_y = ctx.height(heightmap(self.placement), x, z) - 1;
                let y = if follow_ground { surface_y } else { b.min_y.min(surface_y) };
                let pos = BlockPos::new(x, y, z);
                if (y - b.min_y).abs() <= 3 && self.replaceable(ctx, ctx.block(pos)) {
                    self.place_netherrack_or_magma(ctx, random, pos);
                    if self.properties.overgrown {
                        Self::maybe_add_leaves_above(ctx, random, pos);
                    }
                    self.drip_column(ctx, random, pos.below());
                }
            }
        }
    }
}

impl Piece for RuinedPortalPiece {
    fn base(&self) -> &PieceBase {
        &self.t.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.t.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:rupo"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.t.move_by(dx, dy, dz);
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, reference: BlockPos) {
        let portal = self.t.template.bounding_box(&self.t.settings, self.t.position);
        if !chunk_bb.is_inside(portal.center()) {
            return;
        }
        let widened = BoundingBox::encapsulating(chunk_bb, &portal);
        self.t.place(ctx, random, &widened, reference, &mut |_, _, _, _, _| {});
        self.spread_netherrack(ctx, random);
        // `addNetherrackDripColumnsBelowPortal`.
        let b = self.t.base.bbox;
        for x in b.min_x + 1..b.max_x {
            for z in b.min_z + 1..b.max_z {
                let pos = BlockPos::new(x, b.min_y, z);
                if ctx.is(ctx.block(pos), "minecraft:netherrack") {
                    self.drip_column(ctx, random, pos.below());
                }
            }
        }
        if self.properties.vines || self.properties.overgrown {
            // `BlockPos.betweenClosedStream`: X fastest, then Y, then Z.
            for z in b.min_z..=b.max_z {
                for y in b.min_y..=b.max_y {
                    for x in b.min_x..=b.max_x {
                        let pos = BlockPos::new(x, y, z);
                        if self.properties.vines {
                            Self::maybe_add_vines(ctx, random, pos);
                        }
                        if self.properties.overgrown {
                            Self::maybe_add_leaves_above(ctx, random, pos);
                        }
                    }
                }
            }
        }
    }
}
