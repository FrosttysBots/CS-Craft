//! Mineshafts (vanilla `MineshaftStructure`, `MineshaftPieces`).
//!
//! Source-informed from the pinned 26.3 JAR. The layout grows depth-first
//! from a room, corridors, crossings and stairs (each child's own children
//! first), rejecting boxes that collide with any placed piece; the whole
//! shaft then moves below sea level (or, in badlands, towards the surface).
//! Pieces never replace their own wood, and skip placement entirely where
//! liquid touches their bounds or the biome blocks mineshafts.

use crate::feature::blocks::Behaviour;
use crate::feature::template::BoundingBox;
use crate::feature::Ctx;
use crate::structure::piece::{is_replaceable_by_structures, Piece, PieceBase};
use crate::structure::{GenerationContext, PieceList, StructureKind, Stub};
use minecraftoss_core::block::{flags, SupportType};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::{Axis, Direction};
use minecraftoss_core::random::{LegacyRandom, RandomSource, WorldgenRandom};
use minecraftoss_core::{BlockPos, BlockStateId, ChunkPos};
use serde_json::Value;

#[derive(Debug)]
pub struct Mineshaft {
    mesa: bool,
}

impl Mineshaft {
    pub fn parse(json: &Value) -> Result<Self, String> {
        Ok(Self { mesa: json.get("mineshaft_type").and_then(Value::as_str) == Some("mesa") })
    }
}

impl StructureKind for Mineshaft {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        ctx.random.next_f64();
        let chunk = ctx.chunk;
        let start = BlockPos::new(chunk.min_block_x() + 8, 50, chunk.min_block_z());
        let mut pieces: Vec<MinePiece> = Vec::new();
        let room = MinePiece::room(0, &mut ctx.random, chunk.min_block_x() + 2, chunk.min_block_z() + 2, self.mesa);
        pieces.push(room);
        add_children(&mut pieces, 0, &mut ctx.random);
        let sea_level = ctx.terrain.sea_level;
        let bounds = BoundingBox::encapsulating_all(pieces.iter().map(|p| &p.base.bbox)).expect("mineshaft room");
        let dy = if self.mesa {
            let center = bounds.center();
            let surface = ctx.first_free_height(center.x, center.z, HeightmapKind::WorldSurfaceWg);
            let target = if surface <= sea_level { sea_level } else { ctx.random.next_i32_bound(surface - sea_level + 1) + sea_level };
            target - center.y
        } else {
            // `moveBelowSeaLevel(seaLevel, minY, random, 10)`.
            let max_y = sea_level - 10;
            let mut y1 = bounds.y_span() + ctx.terrain.min_y + 1;
            if y1 < max_y {
                y1 += ctx.random.next_i32_bound(max_y - y1);
            }
            y1 - bounds.max_y
        };
        for p in &mut pieces {
            p.move_by(0, dy, 0);
        }
        let list: PieceList = pieces.into_iter().map(|p| Box::new(p) as Box<dyn Piece>).collect();
        Some(Stub::built(start.offset(0, dy, 0), list))
    }
}

#[derive(Debug)]
enum Kind {
    Room { entrances: Vec<BoundingBox> },
    Corridor { has_rails: bool, spider: bool, placed_spider: bool, sections: i32 },
    Crossing { direction: Option<Direction>, two_floored: bool },
    Stairs,
}

#[derive(Debug)]
pub struct MinePiece {
    base: PieceBase,
    mesa: bool,
    kind: Kind,
}

impl MinePiece {
    fn room(depth: i32, random: &mut LegacyRandom, west: i32, north: i32, mesa: bool) -> Self {
        let max_x = west + 7 + random.next_i32_bound(6);
        let max_y = 54 + random.next_i32_bound(6);
        let max_z = north + 7 + random.next_i32_bound(6);
        Self { base: PieceBase::new(depth, BoundingBox::new(west, 50, north, max_x, max_y, max_z)), mesa, kind: Kind::Room { entrances: Vec::new() } }
    }
}

fn find_collision(pieces: &[MinePiece], b: &BoundingBox) -> bool {
    pieces.iter().any(|p| p.base.bbox.intersects(b))
}

fn directional(direction: Direction, north: BoundingBox, south: BoundingBox, west: BoundingBox, east: BoundingBox) -> BoundingBox {
    match direction {
        Direction::South => south,
        Direction::West => west,
        Direction::East => east,
        _ => north,
    }
}

/// `MineshaftPieces.createRandomShaftPiece`.
fn create_random_piece(pieces: &[MinePiece], random: &mut LegacyRandom, foot: (i32, i32, i32), direction: Direction, depth: i32, mesa: bool) -> Option<MinePiece> {
    let (fx, fy, fz) = foot;
    let selection = random.next_i32_bound(100);
    if selection >= 80 {
        let y1 = if random.next_i32_bound(4) == 0 { 6 } else { 2 };
        let b = directional(
            direction,
            BoundingBox::new(-1, 0, -4, 3, y1, 0),
            BoundingBox::new(-1, 0, 0, 3, y1, 4),
            BoundingBox::new(-4, 0, -1, 0, y1, 3),
            BoundingBox::new(0, 0, -1, 4, y1, 3),
        )
        .moved(fx, fy, fz);
        if find_collision(pieces, &b) {
            return None;
        }
        Some(MinePiece { base: PieceBase::new(depth, b), mesa, kind: Kind::Crossing { direction: Some(direction), two_floored: b.y_span() > 3 } })
    } else if selection >= 70 {
        let b = directional(
            direction,
            BoundingBox::new(0, -5, -8, 2, 2, 0),
            BoundingBox::new(0, -5, 0, 2, 2, 8),
            BoundingBox::new(-8, -5, 0, 0, 2, 2),
            BoundingBox::new(0, -5, 0, 8, 2, 2),
        )
        .moved(fx, fy, fz);
        if find_collision(pieces, &b) {
            return None;
        }
        let mut base = PieceBase::new(depth, b);
        base.set_orientation(Some(direction));
        Some(MinePiece { base, mesa, kind: Kind::Stairs })
    } else {
        let mut length = random.next_i32_bound(3) + 2;
        let mut found = None;
        while length > 0 {
            let l = length * 5;
            let b = directional(
                direction,
                BoundingBox::new(0, 0, -(l - 1), 2, 2, 0),
                BoundingBox::new(0, 0, 0, 2, 2, l - 1),
                BoundingBox::new(-(l - 1), 0, 0, 0, 2, 2),
                BoundingBox::new(0, 0, 0, l - 1, 2, 2),
            )
            .moved(fx, fy, fz);
            if !find_collision(pieces, &b) {
                found = Some(b);
                break;
            }
            length -= 1;
        }
        let b = found?;
        let mut base = PieceBase::new(depth, b);
        base.set_orientation(Some(direction));
        let has_rails = random.next_i32_bound(3) == 0;
        let spider = !has_rails && random.next_i32_bound(23) == 0;
        let sections = if direction.axis() == Axis::Z { b.z_span() / 5 } else { b.x_span() / 5 };
        Some(MinePiece { base, mesa, kind: Kind::Corridor { has_rails, spider, placed_spider: false, sections } })
    }
}

/// `MineshaftPieces.generateAndAddPiece`: returns the new piece's index.
fn generate_and_add(pieces: &mut Vec<MinePiece>, random: &mut LegacyRandom, foot: (i32, i32, i32), direction: Direction, depth: i32) -> Option<usize> {
    if depth > 8 {
        return None;
    }
    let start = pieces[0].base.bbox;
    if (foot.0 - start.min_x).abs() > 80 || (foot.2 - start.min_z).abs() > 80 {
        return None;
    }
    let mesa = pieces[0].mesa;
    let piece = create_random_piece(pieces, random, foot, direction, depth + 1, mesa)?;
    pieces.push(piece);
    let index = pieces.len() - 1;
    add_children(pieces, index, random);
    Some(index)
}

/// `addChildren` of the piece at `index`.
fn add_children(pieces: &mut Vec<MinePiece>, index: usize, random: &mut LegacyRandom) {
    let depth = pieces[index].base.gen_depth;
    let b = pieces[index].base.bbox;
    match pieces[index].kind {
        Kind::Room { .. } => {
            let height_space = (b.y_span() - 3 - 1).max(1);
            let mut entrances = Vec::new();
            let mut pos = 0;
            while pos < b.x_span() {
                pos += random.next_i32_bound(b.x_span());
                if pos + 3 > b.x_span() {
                    break;
                }
                let y = b.min_y + random.next_i32_bound(height_space) + 1;
                if let Some(child) = generate_and_add(pieces, random, (b.min_x + pos, y, b.min_z - 1), Direction::North, depth) {
                    let c = pieces[child].base.bbox;
                    entrances.push(BoundingBox::new(c.min_x, c.min_y, b.min_z, c.max_x, c.max_y, b.min_z + 1));
                }
                pos += 4;
            }
            pos = 0;
            while pos < b.x_span() {
                pos += random.next_i32_bound(b.x_span());
                if pos + 3 > b.x_span() {
                    break;
                }
                let y = b.min_y + random.next_i32_bound(height_space) + 1;
                if let Some(child) = generate_and_add(pieces, random, (b.min_x + pos, y, b.max_z + 1), Direction::South, depth) {
                    let c = pieces[child].base.bbox;
                    entrances.push(BoundingBox::new(c.min_x, c.min_y, b.max_z - 1, c.max_x, c.max_y, b.max_z));
                }
                pos += 4;
            }
            pos = 0;
            while pos < b.z_span() {
                pos += random.next_i32_bound(b.z_span());
                if pos + 3 > b.z_span() {
                    break;
                }
                let y = b.min_y + random.next_i32_bound(height_space) + 1;
                if let Some(child) = generate_and_add(pieces, random, (b.min_x - 1, y, b.min_z + pos), Direction::West, depth) {
                    let c = pieces[child].base.bbox;
                    entrances.push(BoundingBox::new(b.min_x, c.min_y, c.min_z, b.min_x + 1, c.max_y, c.max_z));
                }
                pos += 4;
            }
            pos = 0;
            while pos < b.z_span() {
                pos += random.next_i32_bound(b.z_span());
                if pos + 3 > b.z_span() {
                    break;
                }
                let y = b.min_y + random.next_i32_bound(height_space) + 1;
                if let Some(child) = generate_and_add(pieces, random, (b.max_x + 1, y, b.min_z + pos), Direction::East, depth) {
                    let c = pieces[child].base.bbox;
                    entrances.push(BoundingBox::new(b.max_x - 1, c.min_y, c.min_z, b.max_x, c.max_y, c.max_z));
                }
                pos += 4;
            }
            if let Kind::Room { entrances: e } = &mut pieces[index].kind {
                *e = entrances;
            }
        }
        Kind::Corridor { .. } => {
            let end = random.next_i32_bound(4);
            let orientation = pieces[index].base.orientation();
            if let Some(o) = orientation {
                let y = |random: &mut LegacyRandom| b.min_y - 1 + random.next_i32_bound(3);
                match o {
                    Direction::South => {
                        if end <= 1 {
                            let y = y(random);
                            generate_and_add(pieces, random, (b.min_x, y, b.max_z + 1), o, depth);
                        } else if end == 2 {
                            let y = y(random);
                            generate_and_add(pieces, random, (b.min_x - 1, y, b.max_z - 3), Direction::West, depth);
                        } else {
                            let y = y(random);
                            generate_and_add(pieces, random, (b.max_x + 1, y, b.max_z - 3), Direction::East, depth);
                        }
                    }
                    Direction::West => {
                        if end <= 1 {
                            let y = y(random);
                            generate_and_add(pieces, random, (b.min_x - 1, y, b.min_z), o, depth);
                        } else if end == 2 {
                            let y = y(random);
                            generate_and_add(pieces, random, (b.min_x, y, b.min_z - 1), Direction::North, depth);
                        } else {
                            let y = y(random);
                            generate_and_add(pieces, random, (b.min_x, y, b.max_z + 1), Direction::South, depth);
                        }
                    }
                    Direction::East => {
                        if end <= 1 {
                            let y = y(random);
                            generate_and_add(pieces, random, (b.max_x + 1, y, b.min_z), o, depth);
                        } else if end == 2 {
                            let y = y(random);
                            generate_and_add(pieces, random, (b.max_x - 3, y, b.min_z - 1), Direction::North, depth);
                        } else {
                            let y = y(random);
                            generate_and_add(pieces, random, (b.max_x - 3, y, b.max_z + 1), Direction::South, depth);
                        }
                    }
                    _ => {
                        if end <= 1 {
                            let y = y(random);
                            generate_and_add(pieces, random, (b.min_x, y, b.min_z - 1), o, depth);
                        } else if end == 2 {
                            let y = y(random);
                            generate_and_add(pieces, random, (b.min_x - 1, y, b.min_z), Direction::West, depth);
                        } else {
                            let y = y(random);
                            generate_and_add(pieces, random, (b.max_x + 1, y, b.min_z), Direction::East, depth);
                        }
                    }
                }
            }
            if depth < 8 {
                if orientation != Some(Direction::North) && orientation != Some(Direction::South) {
                    let mut x = b.min_x + 3;
                    while x + 3 <= b.max_x {
                        let selection = random.next_i32_bound(5);
                        if selection == 0 {
                            generate_and_add(pieces, random, (x, b.min_y, b.min_z - 1), Direction::North, depth + 1);
                        } else if selection == 1 {
                            generate_and_add(pieces, random, (x, b.min_y, b.max_z + 1), Direction::South, depth + 1);
                        }
                        x += 5;
                    }
                } else {
                    let mut z = b.min_z + 3;
                    while z + 3 <= b.max_z {
                        let selection = random.next_i32_bound(5);
                        if selection == 0 {
                            generate_and_add(pieces, random, (b.min_x - 1, b.min_y, z), Direction::West, depth + 1);
                        } else if selection == 1 {
                            generate_and_add(pieces, random, (b.max_x + 1, b.min_y, z), Direction::East, depth + 1);
                        }
                        z += 5;
                    }
                }
            }
        }
        Kind::Crossing { direction, two_floored } => {
            match direction.unwrap_or(Direction::North) {
                Direction::South => {
                    generate_and_add(pieces, random, (b.min_x + 1, b.min_y, b.max_z + 1), Direction::South, depth);
                    generate_and_add(pieces, random, (b.min_x - 1, b.min_y, b.min_z + 1), Direction::West, depth);
                    generate_and_add(pieces, random, (b.max_x + 1, b.min_y, b.min_z + 1), Direction::East, depth);
                }
                Direction::West => {
                    generate_and_add(pieces, random, (b.min_x + 1, b.min_y, b.min_z - 1), Direction::North, depth);
                    generate_and_add(pieces, random, (b.min_x + 1, b.min_y, b.max_z + 1), Direction::South, depth);
                    generate_and_add(pieces, random, (b.min_x - 1, b.min_y, b.min_z + 1), Direction::West, depth);
                }
                Direction::East => {
                    generate_and_add(pieces, random, (b.min_x + 1, b.min_y, b.min_z - 1), Direction::North, depth);
                    generate_and_add(pieces, random, (b.min_x + 1, b.min_y, b.max_z + 1), Direction::South, depth);
                    generate_and_add(pieces, random, (b.max_x + 1, b.min_y, b.min_z + 1), Direction::East, depth);
                }
                _ => {
                    generate_and_add(pieces, random, (b.min_x + 1, b.min_y, b.min_z - 1), Direction::North, depth);
                    generate_and_add(pieces, random, (b.min_x - 1, b.min_y, b.min_z + 1), Direction::West, depth);
                    generate_and_add(pieces, random, (b.max_x + 1, b.min_y, b.min_z + 1), Direction::East, depth);
                }
            }
            if two_floored {
                let up = b.min_y + 3 + 1;
                if random.next_bool() {
                    generate_and_add(pieces, random, (b.min_x + 1, up, b.min_z - 1), Direction::North, depth);
                }
                if random.next_bool() {
                    generate_and_add(pieces, random, (b.min_x - 1, up, b.min_z + 1), Direction::West, depth);
                }
                if random.next_bool() {
                    generate_and_add(pieces, random, (b.max_x + 1, up, b.min_z + 1), Direction::East, depth);
                }
                if random.next_bool() {
                    generate_and_add(pieces, random, (b.min_x + 1, up, b.max_z + 1), Direction::South, depth);
                }
            }
        }
        Kind::Stairs => match pieces[index].base.orientation() {
            Some(Direction::South) => {
                generate_and_add(pieces, random, (b.min_x, b.min_y, b.max_z + 1), Direction::South, depth);
            }
            Some(Direction::West) => {
                generate_and_add(pieces, random, (b.min_x - 1, b.min_y, b.min_z), Direction::West, depth);
            }
            Some(Direction::East) => {
                generate_and_add(pieces, random, (b.max_x + 1, b.min_y, b.min_z), Direction::East, depth);
            }
            Some(_) => {
                generate_and_add(pieces, random, (b.min_x, b.min_y, b.min_z - 1), Direction::North, depth);
            }
            None => {}
        },
    }
}

/// The wood blocks of one mineshaft type.
struct Wood {
    planks: BlockStateId,
    log: BlockStateId,
    fence: BlockStateId,
    cave_air: BlockStateId,
    cobweb: BlockStateId,
}

impl Wood {
    fn load(ctx: &Ctx, mesa: bool) -> Self {
        let s = |name: &str| ctx.registries().blocks.parse_state(name).expect("mineshaft block");
        let (planks, log, fence) = if mesa {
            (s("minecraft:dark_oak_planks"), s("minecraft:dark_oak_log"), s("minecraft:dark_oak_fence"))
        } else {
            (s("minecraft:oak_planks"), s("minecraft:oak_log"), s("minecraft:oak_fence"))
        };
        Self { planks, log, fence, cave_air: ctx.lib.blocks.cave_air, cobweb: s("minecraft:cobweb") }
    }
}

fn sturdy(ctx: &Ctx, pos: BlockPos, face: Direction, support: SupportType) -> bool {
    Behaviour { registries: ctx.registries() }.is_face_sturdy_as(ctx.block(pos), face, support)
}

/// `FallingBlock` subclasses: sand, red sand, gravel, concrete powder,
/// anvils and the dragon egg.
fn is_falling_block(name: &str) -> bool {
    matches!(
        name,
        "minecraft:sand" | "minecraft:red_sand" | "minecraft:gravel" | "minecraft:anvil" | "minecraft:chipped_anvil" | "minecraft:damaged_anvil" | "minecraft:dragon_egg"
    ) || name.ends_with("_concrete_powder")
}

impl MinePiece {
    /// `MineShaftPiece.isInInvalidLocation`.
    fn is_in_invalid_location(&self, ctx: &Ctx, chunk_bb: &BoundingBox) -> bool {
        let b = &self.base.bbox;
        let x0 = (b.min_x - 1).max(chunk_bb.min_x);
        let y0 = (b.min_y - 1).max(chunk_bb.min_y);
        let z0 = (b.min_z - 1).max(chunk_bb.min_z);
        let x1 = (b.max_x + 1).min(chunk_bb.max_x);
        let y1 = (b.max_y + 1).min(chunk_bb.max_y);
        let z1 = (b.max_z + 1).min(chunk_bb.max_z);
        let center = BlockPos::new((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2);
        if let (Some(biome), Some(tag)) = (ctx.biome(center), ctx.registries().biome_tags.id("minecraft:mineshaft_blocking")) {
            if ctx.registries().biome_tags.contains(tag, usize::from(biome.0)) {
                return true;
            }
        }
        let liquid = |x: i32, y: i32, z: i32| ctx.registries().blocks.is(ctx.block(BlockPos::new(x, y, z)), flags::LIQUID);
        for x in x0..=x1 {
            for z in z0..=z1 {
                if liquid(x, y0, z) || liquid(x, y1, z) {
                    return true;
                }
            }
        }
        for x in x0..=x1 {
            for y in y0..=y1 {
                if liquid(x, y, z0) || liquid(x, y, z1) {
                    return true;
                }
            }
        }
        for z in z0..=z1 {
            for y in y0..=y1 {
                if liquid(x0, y, z) || liquid(x1, y, z) {
                    return true;
                }
            }
        }
        false
    }

    /// `MineShaftPiece.setPlanksBlock`.
    fn set_planks(&self, ctx: &mut Ctx, chunk_bb: &BoundingBox, planks: BlockStateId, x: i32, y: i32, z: i32) {
        if self.base.is_interior(ctx, x, y, z, chunk_bb) {
            let pos = self.base.world_pos(x, y, z);
            if !sturdy(ctx, pos, Direction::Up, SupportType::Full) {
                ctx.set_block(pos, planks);
            }
        }
    }

    /// `isSupportingBox`.
    fn is_supporting_box(&self, ctx: &Ctx, chunk_bb: &BoundingBox, x0: i32, x1: i32, y1: i32, z: i32) -> bool {
        (x0..=x1).all(|x| !ctx.is_air(self.base.get_block(ctx, x, y1 + 1, z, chunk_bb)))
    }

    /// `MineShaftCorridor.placeSupport`.
    #[allow(clippy::too_many_arguments)]
    fn place_support(&self, ctx: &mut Ctx, chunk_bb: &BoundingBox, w: &Wood, x0: i32, y0: i32, z: i32, y1: i32, x1: i32, random: &mut WorldgenRandom) {
        if !self.is_supporting_box(ctx, chunk_bb, x0, x1, y1, z) {
            return;
        }
        let p = &self.base;
        let fence_west = ctx.with(w.fence, "west", "true");
        let fence_east = ctx.with(w.fence, "east", "true");
        p.generate_box(ctx, chunk_bb, x0, y0, z, x0, y1 - 1, z, fence_west, w.cave_air, false);
        p.generate_box(ctx, chunk_bb, x1, y0, z, x1, y1 - 1, z, fence_east, w.cave_air, false);
        if random.next_i32_bound(4) == 0 {
            p.generate_box(ctx, chunk_bb, x0, y1, z, x0, y1, z, w.planks, w.cave_air, false);
            p.generate_box(ctx, chunk_bb, x1, y1, z, x1, y1, z, w.planks, w.cave_air, false);
        } else {
            p.generate_box(ctx, chunk_bb, x0, y1, z, x1, y1, z, w.planks, w.cave_air, false);
            let torch_south = ctx.registries().blocks.parse_state("minecraft:wall_torch[facing=south]").expect("torch");
            let torch_north = ctx.registries().blocks.parse_state("minecraft:wall_torch[facing=north]").expect("torch");
            p.maybe_generate_block(ctx, chunk_bb, random, 0.05, x0 + 1, y1, z - 1, torch_south);
            p.maybe_generate_block(ctx, chunk_bb, random, 0.05, x0 + 1, y1, z + 1, torch_north);
        }
    }

    /// `maybePlaceCobWeb`.
    #[allow(clippy::too_many_arguments)]
    fn maybe_place_cobweb(&self, ctx: &mut Ctx, chunk_bb: &BoundingBox, random: &mut WorldgenRandom, cobweb: BlockStateId, probability: f32, x: i32, y: i32, z: i32) {
        if self.base.is_interior(ctx, x, y, z, chunk_bb) && random.next_f32() < probability && self.has_sturdy_neighbours(ctx, chunk_bb, x, y, z, 2) {
            self.base.place_block(ctx, cobweb, x, y, z, chunk_bb);
        }
    }

    fn has_sturdy_neighbours(&self, ctx: &Ctx, chunk_bb: &BoundingBox, x: i32, y: i32, z: i32, count: i32) -> bool {
        let pos = self.base.world_pos(x, y, z);
        let mut sturdy_count = 0;
        for direction in Direction::ALL {
            let n = pos.relative(direction, 1);
            if chunk_bb.is_inside(n) && sturdy(ctx, n, direction.opposite(), SupportType::Full) {
                sturdy_count += 1;
                if sturdy_count >= count {
                    return true;
                }
            }
        }
        false
    }

    /// `MineShaftCorridor.createChest`: a rail with a chest minecart.
    #[allow(clippy::too_many_arguments)]
    fn create_minecart_chest(&self, ctx: &mut Ctx, chunk_bb: &BoundingBox, random: &mut WorldgenRandom, x: i32, y: i32, z: i32) -> bool {
        let pos = self.base.world_pos(x, y, z);
        if chunk_bb.is_inside(pos) && ctx.is_air(ctx.block(pos)) && !ctx.is_air(ctx.block(pos.below())) {
            let shape = if random.next_bool() { "north_south" } else { "east_west" };
            let rail = ctx.registries().blocks.parse_state(&format!("minecraft:rail[shape={shape}]")).expect("rail");
            self.base.place_block(ctx, rail, x, y, z, chunk_bb);
            // `MinecartChest.setInitialPos` and `setLootTable`.
            let seed = random.next_i64();
            let at = [f64::from(pos.x) + 0.5, f64::from(pos.y) + 0.5, f64::from(pos.z) + 0.5];
            if let Some(mut entity) = crate::feature::entities::create(ctx, "minecraft:chest_minecart", at, 0.0, 0.0) {
                if let minecraftoss_core::nbt::Tag::Compound(map) = &mut entity {
                    map.remove("Items");
                    map.insert("LootTable".into(), minecraftoss_core::nbt::Tag::String("minecraft:chests/abandoned_mineshaft".into()));
                    if seed != 0 {
                        map.insert("LootTableSeed".into(), minecraftoss_core::nbt::Tag::Long(seed));
                    }
                }
                ctx.region.add_entity(entity);
            }
            true
        } else {
            false
        }
    }

    /// `placeDoubleLowerOrUpperSupport`.
    fn place_double_support(&self, ctx: &mut Ctx, chunk_bb: &BoundingBox, w: &Wood, x: i32, y: i32, z: i32) {
        let planks = ctx.registries().blocks.block_of(w.planks);
        if ctx.registries().blocks.block_of(self.base.get_block(ctx, x, y, z, chunk_bb)) == planks {
            self.fill_pillar_down_or_chain_up(ctx, w, x, y, z, chunk_bb);
        }
        if ctx.registries().blocks.block_of(self.base.get_block(ctx, x + 2, y, z, chunk_bb)) == planks {
            self.fill_pillar_down_or_chain_up(ctx, w, x + 2, y, z, chunk_bb);
        }
    }

    /// `fillPillarDownOrChainUp`.
    fn fill_pillar_down_or_chain_up(&self, ctx: &mut Ctx, w: &Wood, x: i32, y: i32, z: i32, chunk_bb: &BoundingBox) {
        let pos = self.base.world_pos(x, y, z);
        if !chunk_bb.is_inside(pos) {
            return;
        }
        let world_y = pos.y;
        let mut distance = 1;
        let (mut check_below, mut check_above) = (true, true);
        while check_below || check_above {
            if check_below {
                let below = pos.at_y(world_y - distance);
                let state = ctx.block(below);
                let empty = is_replaceable_by_structures(ctx, state) && !ctx.is(state, "minecraft:lava");
                if !empty && sturdy(ctx, below, Direction::Up, SupportType::Full) {
                    for py in world_y - distance + 1..world_y {
                        ctx.set_block(pos.at_y(py), w.log);
                    }
                    return;
                }
                check_below = distance <= 20 && empty && below.y > ctx.min_y() + 1;
            }
            if check_above {
                let above = pos.at_y(world_y + distance);
                let state = ctx.block(above);
                let empty = is_replaceable_by_structures(ctx, state);
                if !empty && sturdy(ctx, above, Direction::Down, SupportType::Center) && !is_falling_block(ctx.name(state)) {
                    ctx.set_block(pos.at_y(world_y + 1), w.fence);
                    let chain = ctx.registries().blocks.parse_state("minecraft:iron_chain").expect("chain");
                    for py in world_y + 2..world_y + distance {
                        ctx.set_block(pos.at_y(py), chain);
                    }
                    return;
                }
                check_above = distance <= 50 && empty && above.y < ctx.max_y();
            }
            distance += 1;
        }
    }
}

impl Piece for MinePiece {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn type_name(&self) -> &'static str {
        match self.kind {
            Kind::Room { .. } => "minecraft:msroom",
            Kind::Corridor { .. } => "minecraft:mscorridor",
            Kind::Crossing { .. } => "minecraft:mscrossing",
            Kind::Stairs => "minecraft:msstairs",
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox = self.base.bbox.moved(dx, dy, dz);
        if let Kind::Room { entrances } = &mut self.kind {
            for e in entrances {
                *e = e.moved(dx, dy, dz);
            }
        }
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, _reference: BlockPos) {
        if self.is_in_invalid_location(ctx, chunk_bb) {
            return;
        }
        let w = Wood::load(ctx, self.mesa);
        if self.base.keep.is_empty() {
            let blocks = &ctx.registries().blocks;
            let chain = blocks.block_by_name("minecraft:iron_chain").expect("chain");
            self.base.keep = vec![blocks.block_of(w.planks), blocks.block_of(w.log), blocks.block_of(w.fence), chain];
        }
        let air = w.cave_air;
        let b = self.base.bbox;
        let bb = chunk_bb;
        match &mut self.kind {
            Kind::Room { entrances } => {
                let p = &self.base;
                p.generate_box(ctx, bb, b.min_x, b.min_y + 1, b.min_z, b.max_x, (b.min_y + 3).min(b.max_y), b.max_z, air, air, false);
                for e in entrances.iter() {
                    p.generate_box(ctx, bb, e.min_x, e.max_y - 2, e.min_z, e.max_x, e.max_y, e.max_z, air, air, false);
                }
                p.generate_upper_half_sphere(ctx, bb, b.min_x, b.min_y + 4, b.min_z, b.max_x, b.max_y, b.max_z, air, false);
            }
            Kind::Stairs => {
                let p = &self.base;
                p.generate_box(ctx, bb, 0, 5, 0, 2, 7, 1, air, air, false);
                p.generate_box(ctx, bb, 0, 0, 7, 2, 2, 8, air, air, false);
                for i in 0..5 {
                    p.generate_box(ctx, bb, 0, 5 - i - if i < 4 { 1 } else { 0 }, 2 + i, 2, 7 - i, 2 + i, air, air, false);
                }
            }
            Kind::Crossing { two_floored, .. } => {
                let two_floored = *two_floored;
                let p = &self.base;
                if two_floored {
                    p.generate_box(ctx, bb, b.min_x + 1, b.min_y, b.min_z, b.max_x - 1, b.min_y + 3 - 1, b.max_z, air, air, false);
                    p.generate_box(ctx, bb, b.min_x, b.min_y, b.min_z + 1, b.max_x, b.min_y + 3 - 1, b.max_z - 1, air, air, false);
                    p.generate_box(ctx, bb, b.min_x + 1, b.max_y - 2, b.min_z, b.max_x - 1, b.max_y, b.max_z, air, air, false);
                    p.generate_box(ctx, bb, b.min_x, b.max_y - 2, b.min_z + 1, b.max_x, b.max_y, b.max_z - 1, air, air, false);
                    p.generate_box(ctx, bb, b.min_x + 1, b.min_y + 3, b.min_z + 1, b.max_x - 1, b.min_y + 3, b.max_z - 1, air, air, false);
                } else {
                    p.generate_box(ctx, bb, b.min_x + 1, b.min_y, b.min_z, b.max_x - 1, b.max_y, b.max_z, air, air, false);
                    p.generate_box(ctx, bb, b.min_x, b.min_y, b.min_z + 1, b.max_x, b.max_y, b.max_z - 1, air, air, false);
                }
                let pillar = |ctx: &mut Ctx, x: i32, z: i32| {
                    if !ctx.is_air(p.get_block(ctx, x, b.max_y + 1, z, bb)) {
                        p.generate_box(ctx, bb, x, b.min_y, z, x, b.max_y, z, w.planks, air, false);
                    }
                };
                pillar(ctx, b.min_x + 1, b.min_z + 1);
                pillar(ctx, b.min_x + 1, b.max_z - 1);
                pillar(ctx, b.max_x - 1, b.min_z + 1);
                pillar(ctx, b.max_x - 1, b.max_z - 1);
                let y = b.min_y - 1;
                for x in b.min_x..=b.max_x {
                    for z in b.min_z..=b.max_z {
                        self.set_planks(ctx, bb, w.planks, x, y, z);
                    }
                }
            }
            Kind::Corridor { has_rails, spider, placed_spider, sections } => {
                let (has_rails, spider, sections) = (*has_rails, *spider, *sections);
                let mut spider_placed = *placed_spider;
                let length = sections * 5 - 1;
                let p = self.base.clone();
                p.generate_box(ctx, bb, 0, 0, 0, 2, 1, length, air, air, false);
                p.generate_maybe_box(ctx, bb, random, 0.8, 0, 2, 0, 2, 2, length, air, air, false, false);
                if spider {
                    p.generate_maybe_box(ctx, bb, random, 0.6, 0, 0, 0, 2, 1, length, w.cobweb, air, false, true);
                }
                for section in 0..sections {
                    let z = 2 + section * 5;
                    self.place_support(ctx, bb, &w, 0, 0, z, 2, 2, random);
                    self.maybe_place_cobweb(ctx, bb, random, w.cobweb, 0.1, 0, 2, z - 1);
                    self.maybe_place_cobweb(ctx, bb, random, w.cobweb, 0.1, 2, 2, z - 1);
                    self.maybe_place_cobweb(ctx, bb, random, w.cobweb, 0.1, 0, 2, z + 1);
                    self.maybe_place_cobweb(ctx, bb, random, w.cobweb, 0.1, 2, 2, z + 1);
                    self.maybe_place_cobweb(ctx, bb, random, w.cobweb, 0.05, 0, 2, z - 2);
                    self.maybe_place_cobweb(ctx, bb, random, w.cobweb, 0.05, 2, 2, z - 2);
                    self.maybe_place_cobweb(ctx, bb, random, w.cobweb, 0.05, 0, 2, z + 2);
                    self.maybe_place_cobweb(ctx, bb, random, w.cobweb, 0.05, 2, 2, z + 2);
                    if random.next_i32_bound(100) == 0 {
                        self.create_minecart_chest(ctx, bb, random, 2, 0, z - 1);
                    }
                    if random.next_i32_bound(100) == 0 {
                        self.create_minecart_chest(ctx, bb, random, 0, 0, z + 1);
                    }
                    if spider && !spider_placed {
                        let new_z = z - 1 + random.next_i32_bound(3);
                        let pos = p.world_pos(1, 0, new_z);
                        if bb.is_inside(pos) && p.is_interior(ctx, 1, 0, new_z, bb) {
                            spider_placed = true;
                            let spawner = ctx.registries().blocks.parse_state("minecraft:spawner").expect("spawner");
                            ctx.set_block(pos, spawner);
                            ctx.region.set_spawner_entity(pos.x, pos.y, pos.z, "minecraft:cave_spider");
                        }
                    }
                }
                for x in 0..=2 {
                    for z in 0..=length {
                        self.set_planks(ctx, bb, w.planks, x, -1, z);
                    }
                }
                self.place_double_support(ctx, bb, &w, 0, -1, 2);
                if sections > 1 {
                    self.place_double_support(ctx, bb, &w, 0, -1, length - 2);
                }
                if has_rails {
                    let rail = ctx.registries().blocks.parse_state("minecraft:rail[shape=north_south]").expect("rail");
                    for z in 0..=length {
                        let floor = p.get_block(ctx, 1, -1, z, bb);
                        if !ctx.is_air(floor) && ctx.registries().blocks.is(floor, flags::SOLID_RENDER) {
                            let probability = if p.is_interior(ctx, 1, 0, z, bb) { 0.7 } else { 0.9 };
                            p.maybe_generate_block(ctx, bb, random, probability, 1, 0, z, rail);
                        }
                    }
                }
                if let Kind::Corridor { placed_spider, .. } = &mut self.kind {
                    *placed_spider = spider_placed;
                }
            }
        }
    }
}
