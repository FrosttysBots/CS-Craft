//! End cities (vanilla `EndCityStructure`, `EndCityPieces`).
//!
//! Source-informed from the pinned 26.3 JAR. Towers, bridges, fat towers
//! and the ship grow recursively; each branch builds into its own list,
//! whose pieces are tagged with a random "generation depth" and kept only if
//! none of them first collides with an earlier piece of a different tag.

use super::template_piece::TemplatePiece;
use crate::feature::template::processor::Processor;
use crate::feature::template::{transform, BoundingBox, PlaceSettings, Rotation};
use crate::feature::{Ctx, Library};
use crate::structure::piece::{Piece, PieceBase};
use crate::structure::{GenerationContext, PieceList, StructureKind, Stub};
use minecraftoss_core::block::flags;
use minecraftoss_core::random::{LegacyRandom, RandomSource, WorldgenRandom};
use minecraftoss_core::{BlockPos, ChunkPos};

#[derive(Debug)]
pub struct EndCity;

impl StructureKind for EndCity {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let (x, z) = (ctx.chunk.min_block_x() + 7, ctx.chunk.min_block_z() + 7);
        if !ctx.could_valid_biome_exist_in_terrain_column(x, z) {
            return None;
        }
        let rotation = Rotation::random(&mut ctx.random);
        // `getLowestYIn5by5Box`.
        let (ox, oz) = match rotation {
            Rotation::Clockwise90 => (-5, 5),
            Rotation::Clockwise180 => (-5, -5),
            Rotation::Counterclockwise90 => (5, -5),
            Rotation::None => (5, 5),
        };
        let y = ctx.lowest_y_at(x, z, ox, oz);
        if y < 60 {
            return None;
        }
        let start = BlockPos::new(x, y, z);
        Some(Stub::deferred(start, move |ctx: &mut GenerationContext| {
            let mut builder = Builder { lib: ctx.lib, ship_created: false };
            let mut pieces = Vec::new();
            builder.start_house_tower(start, rotation, &mut pieces, &mut ctx.random);
            pieces.into_iter().map(|p| Box::new(p) as Box<dyn Piece>).collect::<PieceList>()
        }))
    }
}

#[derive(Clone, Debug)]
pub struct EndCityPiece {
    t: TemplatePiece,
}

impl EndCityPiece {
    fn new(lib: &Library, name: &str, position: BlockPos, rotation: Rotation, overwrite: bool) -> Self {
        let ignore = if overwrite { lib.processor_blocks.structure_block() } else { lib.processor_blocks.structure_and_air() };
        let settings = PlaceSettings { ignore_entities: true, processors: vec![Processor::BlockIgnore(ignore)], rotation, ..PlaceSettings::default() };
        Self { t: TemplatePiece::new(lib, 0, &format!("minecraft:end_city/{name}"), settings, position) }
    }

    fn gen_depth(&self) -> i32 {
        self.t.base.gen_depth
    }

    fn set_gen_depth(&mut self, depth: i32) {
        self.t.base.gen_depth = depth;
    }

    fn rotation(&self) -> Rotation {
        self.t.settings.rotation
    }
}

#[derive(Clone, Copy)]
enum Section {
    HouseTower,
    Tower,
    TowerBridge,
    FatTower,
}

const TOWER_BRIDGES: [(Rotation, (i32, i32, i32)); 4] =
    [(Rotation::None, (1, -1, 0)), (Rotation::Clockwise90, (6, -1, 1)), (Rotation::Counterclockwise90, (0, -1, 5)), (Rotation::Clockwise180, (5, -1, 6))];

const FAT_TOWER_BRIDGES: [(Rotation, (i32, i32, i32)); 4] =
    [(Rotation::None, (4, -1, 0)), (Rotation::Clockwise90, (12, -1, 4)), (Rotation::Counterclockwise90, (0, -1, 8)), (Rotation::Clockwise180, (8, -1, 12))];

struct Builder<'a> {
    lib: &'a Library,
    /// `TOWER_BRIDGE_GENERATOR.shipCreated`, reset per city.
    ship_created: bool,
}

impl Builder<'_> {
    /// `EndCityPieces.addPiece`: the child placed at `offset` in the
    /// parent's rotated frame.
    fn add(&self, parent: &EndCityPiece, offset: (i32, i32, i32), name: &str, rotation: Rotation, overwrite: bool) -> EndCityPiece {
        let mut child = EndCityPiece::new(self.lib, name, parent.t.position, rotation, overwrite);
        let s = &parent.t.settings;
        let origin = transform::transform(BlockPos::new(offset.0, offset.1, offset.2), s.mirror, s.rotation, s.pivot);
        child.t.move_by(origin.x, origin.y, origin.z);
        child
    }

    fn push(pieces: &mut Vec<EndCityPiece>, piece: EndCityPiece) -> EndCityPiece {
        pieces.push(piece.clone());
        piece
    }

    fn start_house_tower(&mut self, origin: BlockPos, rotation: Rotation, pieces: &mut Vec<EndCityPiece>, random: &mut LegacyRandom) {
        self.ship_created = false;
        let last = Self::push(pieces, EndCityPiece::new(self.lib, "base_floor", origin, rotation, true));
        let last = Self::push(pieces, self.add(&last, (-1, 0, -1), "second_floor_1", rotation, false));
        let last = Self::push(pieces, self.add(&last, (-1, 4, -1), "third_floor_1", rotation, false));
        let last = Self::push(pieces, self.add(&last, (-1, 8, -1), "third_roof", rotation, true));
        self.recursive_children(Section::Tower, 1, &last, None, pieces, random);
    }

    /// `EndCityPieces.recursiveChildren`.
    fn recursive_children(
        &mut self,
        section: Section,
        depth: i32,
        parent: &EndCityPiece,
        offset: Option<(i32, i32, i32)>,
        pieces: &mut Vec<EndCityPiece>,
        random: &mut LegacyRandom,
    ) -> bool {
        if depth > 8 {
            return false;
        }
        let mut children = Vec::new();
        if !self.generate(section, depth, parent, offset, &mut children, random) {
            return false;
        }
        // Every kept child is tagged with one random "depth", so later
        // collisions only tolerate pieces of the parent's own branch.
        let tag = random.next_i32();
        for child in &mut children {
            child.set_gen_depth(tag);
            let first = pieces.iter().find(|p| p.t.base.bbox.intersects(&child.t.base.bbox));
            if first.is_some_and(|p| p.gen_depth() != parent.gen_depth()) {
                return false;
            }
        }
        pieces.extend(children);
        true
    }

    fn generate(
        &mut self,
        section: Section,
        depth: i32,
        parent: &EndCityPiece,
        offset: Option<(i32, i32, i32)>,
        pieces: &mut Vec<EndCityPiece>,
        random: &mut LegacyRandom,
    ) -> bool {
        let rotation = parent.rotation();
        match section {
            Section::HouseTower => {
                if depth > 8 {
                    return false;
                }
                let offset = offset.unwrap_or((0, 0, 0));
                let mut last = Self::push(pieces, self.add(parent, offset, "base_floor", rotation, true));
                match random.next_i32_bound(3) {
                    0 => {
                        Self::push(pieces, self.add(&last, (-1, 4, -1), "base_roof", rotation, true));
                    }
                    1 => {
                        last = Self::push(pieces, self.add(&last, (-1, 0, -1), "second_floor_2", rotation, false));
                        last = Self::push(pieces, self.add(&last, (-1, 8, -1), "second_roof", rotation, false));
                        self.recursive_children(Section::Tower, depth + 1, &last, None, pieces, random);
                    }
                    _ => {
                        last = Self::push(pieces, self.add(&last, (-1, 0, -1), "second_floor_2", rotation, false));
                        last = Self::push(pieces, self.add(&last, (-1, 4, -1), "third_floor_2", rotation, false));
                        last = Self::push(pieces, self.add(&last, (-1, 8, -1), "third_roof", rotation, true));
                        self.recursive_children(Section::Tower, depth + 1, &last, None, pieces, random);
                    }
                }
                true
            }
            Section::Tower => {
                let dx = 3 + random.next_i32_bound(2);
                let dz = 3 + random.next_i32_bound(2);
                let mut last = Self::push(pieces, self.add(parent, (dx, -3, dz), "tower_base", rotation, true));
                last = Self::push(pieces, self.add(&last, (0, 7, 0), "tower_piece", rotation, true));
                let mut bridge = if random.next_i32_bound(3) == 0 { Some(last.clone()) } else { None };
                let height = 1 + random.next_i32_bound(3);
                for i in 0..height {
                    last = Self::push(pieces, self.add(&last, (0, 4, 0), "tower_piece", rotation, true));
                    if i < height - 1 && random.next_bool() {
                        bridge = Some(last.clone());
                    }
                }
                if let Some(bridge) = bridge {
                    for (r, off) in TOWER_BRIDGES {
                        if random.next_bool() {
                            let start = Self::push(pieces, self.add(&bridge, off, "bridge_end", rotation.then(r), true));
                            self.recursive_children(Section::TowerBridge, depth + 1, &start, None, pieces, random);
                        }
                    }
                    Self::push(pieces, self.add(&last, (-1, 4, -1), "tower_top", rotation, true));
                } else {
                    if depth != 7 {
                        return self.recursive_children(Section::FatTower, depth + 1, &last, None, pieces, random);
                    }
                    Self::push(pieces, self.add(&last, (-1, 4, -1), "tower_top", rotation, true));
                }
                true
            }
            Section::TowerBridge => {
                let length = random.next_i32_bound(4) + 1;
                let mut first = self.add(parent, (0, 0, -4), "bridge_piece", rotation, true);
                first.set_gen_depth(-1);
                let mut last = Self::push(pieces, first);
                let mut next_y = 0;
                for _ in 0..length {
                    if random.next_bool() {
                        last = Self::push(pieces, self.add(&last, (0, next_y, -4), "bridge_piece", rotation, true));
                        next_y = 0;
                    } else {
                        if random.next_bool() {
                            last = Self::push(pieces, self.add(&last, (0, next_y, -4), "bridge_steep_stairs", rotation, true));
                        } else {
                            last = Self::push(pieces, self.add(&last, (0, next_y, -8), "bridge_gentle_stairs", rotation, true));
                        }
                        next_y = 4;
                    }
                }
                if !self.ship_created && random.next_i32_bound(10 - depth) == 0 {
                    let x = -8 + random.next_i32_bound(8);
                    let z = -70 + random.next_i32_bound(10);
                    Self::push(pieces, self.add(&last, (x, next_y, z), "ship", rotation, true));
                    self.ship_created = true;
                } else if !self.recursive_children(Section::HouseTower, depth + 1, &last, Some((-3, next_y + 1, -11)), pieces, random) {
                    return false;
                }
                let mut end = self.add(&last, (4, next_y, 0), "bridge_end", rotation.then(Rotation::Clockwise180), true);
                end.set_gen_depth(-1);
                Self::push(pieces, end);
                true
            }
            Section::FatTower => {
                let mut last = Self::push(pieces, self.add(parent, (-3, 4, -3), "fat_tower_base", rotation, true));
                last = Self::push(pieces, self.add(&last, (0, 4, 0), "fat_tower_middle", rotation, true));
                let mut i = 0;
                while i < 2 && random.next_i32_bound(3) != 0 {
                    last = Self::push(pieces, self.add(&last, (0, 8, 0), "fat_tower_middle", rotation, true));
                    for (r, off) in FAT_TOWER_BRIDGES {
                        if random.next_bool() {
                            let start = Self::push(pieces, self.add(&last, off, "bridge_end", rotation.then(r), true));
                            self.recursive_children(Section::TowerBridge, depth + 1, &start, None, pieces, random);
                        }
                    }
                    i += 1;
                }
                Self::push(pieces, self.add(&last, (-2, 8, -2), "fat_tower_top", rotation, true));
                true
            }
        }
    }
}

impl Piece for EndCityPiece {
    fn base(&self) -> &PieceBase {
        &self.t.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.t.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:ecp"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.t.move_by(dx, dy, dz);
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, reference: BlockPos) {
        let rotation = self.t.settings.rotation;
        self.t.place(ctx, random, chunk_bb, reference, &mut |ctx, marker, pos, random, bb| {
            if marker.starts_with("Chest") {
                let chest = pos.below();
                if bb.is_inside(chest) && ctx.registries().blocks.is(ctx.block(chest), flags::HAS_BLOCK_ENTITY) {
                    let name = ctx.name(ctx.block(chest));
                    if name.ends_with("chest") && name != "minecraft:ender_chest" || name.ends_with("shulker_box") || name == "minecraft:barrel" {
                        let seed = random.next_i64();
                        ctx.region.set_loot_table(chest.x, chest.y, chest.z, "minecraft:chests/end_city_treasure", seed);
                    }
                }
            } else if bb.is_inside(pos) {
                if marker.starts_with("Sentry") {
                    // `Shulker.setPos`: its yaw comes from its own unseeded random.
                    let at = [f64::from(pos.x) + 0.5, f64::from(pos.y), f64::from(pos.z) + 0.5];
                    if let Some(entity) = crate::feature::entities::create(ctx, "minecraft:shulker", at, 0.0, 0.0) {
                        ctx.region.add_entity(entity);
                    }
                } else if marker.starts_with("Elytra") {
                    elytra_frame(ctx, pos, rotation);
                }
            }
        });
    }
}

/// `new ItemFrame(level, pos, rotation.rotate(SOUTH))` holding an elytra
/// (`EndCityPiece.handleDataMarker`, "Elytra").
fn elytra_frame(ctx: &mut Ctx, pos: BlockPos, rotation: Rotation) {
    use minecraftoss_core::nbt::Tag;
    // The facing's 2D data value (S 0, W 1, N 2, E 3), legacy 3D id and step.
    let (data_2d, facing, (sx, sz)) = match rotation {
        Rotation::None => (0, 3, (0.0, 1.0)),
        Rotation::Clockwise90 => (1, 4, (-1.0, 0.0)),
        Rotation::Clockwise180 => (2, 2, (0.0, -1.0)),
        Rotation::Counterclockwise90 => (3, 5, (1.0, 0.0)),
    };
    // `createBoundingBox`: the block center moved 0.46875 against the facing.
    let at = [f64::from(pos.x) + 0.5 - 0.46875 * sx, f64::from(pos.y) + 0.5, f64::from(pos.z) + 0.5 - 0.46875 * sz];
    let Some(mut frame) = crate::feature::entities::create(ctx, "minecraft:item_frame", at, (data_2d * 90) as f32, 0.0) else { return };
    let item = crate::feature::entities::compound([("id", Tag::String("minecraft:elytra".into())), ("count", Tag::Int(1))]);
    crate::feature::entities::set(
        &mut frame,
        [("Facing", Tag::Byte(facing)), ("block_pos", Tag::IntArray(vec![pos.x, pos.y, pos.z])), ("Item", item)],
    );
    ctx.region.add_entity(frame);
}
