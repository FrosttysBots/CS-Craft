//! Nether fortresses (vanilla `NetherFortressStructure`, `NetherFortressPieces`).
//!
//! Source-informed from the pinned 26.3 JAR. Children are expanded from a
//! pending list in random order; each piece choice walks the weighted list
//! and, when the chosen piece does not fit, keeps trying every later entry
//! (the selection stays negative), as vanilla does. The fortress then moves
//! to a random height between 48 and 70.

use crate::feature::template::BoundingBox;
use crate::feature::Ctx;
use crate::structure::piece::{Piece, PieceBase};
use crate::structure::{GenerationContext, PieceList, StructureKind, Stub};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{LegacyRandom, WorldgenRandom};
use minecraftoss_core::{BlockPos, BlockStateId, ChunkPos};

#[derive(Debug)]
pub struct Fortress;

impl StructureKind for Fortress {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let chunk = ctx.chunk;
        let start = BlockPos::new(chunk.min_block_x(), 64, chunk.min_block_z());
        Some(Stub::deferred(start, move |ctx: &mut GenerationContext| generate(&mut ctx.random, chunk.min_block_x() + 2, chunk.min_block_z() + 2)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Start,
    BridgeCrossing,
    BridgeEndFiller { seed: i32 },
    BridgeStraight,
    CastleCorridorStairs,
    CastleCorridorTBalcony,
    CastleEntrance,
    CastleSmallCorridorCrossing,
    CastleSmallCorridorLeftTurn { needs_chest: bool },
    CastleSmallCorridor,
    CastleSmallCorridorRightTurn { needs_chest: bool },
    CastleStalkRoom,
    MonsterThrone { placed_spawner: bool },
    RoomCrossing,
    StairsRoom,
}

/// Which piece a weight entry makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    BridgeStraight,
    BridgeCrossing,
    RoomCrossing,
    StairsRoom,
    MonsterThrone,
    CastleEntrance,
    CastleSmallCorridor,
    CastleSmallCorridorCrossing,
    CastleSmallCorridorRightTurn,
    CastleSmallCorridorLeftTurn,
    CastleCorridorStairs,
    CastleCorridorTBalcony,
    CastleStalkRoom,
}

/// `NetherFortressPieces.PieceWeight`.
#[derive(Clone, Copy, Debug)]
struct Weight {
    choice: Choice,
    weight: i32,
    place_count: i32,
    max_place_count: i32,
    allow_in_row: bool,
}

impl Weight {
    const fn new(choice: Choice, weight: i32, max_place_count: i32, allow_in_row: bool) -> Self {
        Self { choice, weight, place_count: 0, max_place_count, allow_in_row }
    }

    fn can_place(&self) -> bool {
        self.max_place_count == 0 || self.place_count < self.max_place_count
    }
}

const BRIDGE: [Weight; 6] = [
    Weight::new(Choice::BridgeStraight, 30, 0, true),
    Weight::new(Choice::BridgeCrossing, 10, 4, false),
    Weight::new(Choice::RoomCrossing, 10, 4, false),
    Weight::new(Choice::StairsRoom, 10, 3, false),
    Weight::new(Choice::MonsterThrone, 5, 2, false),
    Weight::new(Choice::CastleEntrance, 5, 1, false),
];

const CASTLE: [Weight; 7] = [
    Weight::new(Choice::CastleSmallCorridor, 25, 0, true),
    Weight::new(Choice::CastleSmallCorridorCrossing, 15, 5, false),
    Weight::new(Choice::CastleSmallCorridorRightTurn, 5, 10, false),
    Weight::new(Choice::CastleSmallCorridorLeftTurn, 5, 10, false),
    Weight::new(Choice::CastleCorridorStairs, 10, 3, true),
    Weight::new(Choice::CastleCorridorTBalcony, 7, 2, false),
    Weight::new(Choice::CastleStalkRoom, 5, 2, false),
];

#[derive(Debug)]
pub struct FortressPiece {
    base: PieceBase,
    kind: Kind,
}

/// The layout under construction (`StartPiece` state and the builder).
struct Layout {
    pieces: Vec<FortressPiece>,
    pending: Vec<usize>,
    bridge: Vec<Weight>,
    castle: Vec<Weight>,
    previous: Option<Choice>,
}

fn oriented(depth: i32, bbox: BoundingBox, direction: Direction, kind: Kind) -> FortressPiece {
    let mut base = PieceBase::new(depth, bbox);
    base.set_orientation(Some(direction));
    FortressPiece { base, kind }
}

/// `NetherFortressStructure.generatePieces`.
fn generate(random: &mut LegacyRandom, west: i32, north: i32) -> PieceList {
    let direction = PieceBase::random_horizontal_direction(random);
    let start = oriented(0, PieceBase::make_bounding_box(west, 64, north, direction, 19, 10, 19), direction, Kind::Start);
    let mut layout = Layout { pieces: vec![start], pending: Vec::new(), bridge: BRIDGE.to_vec(), castle: CASTLE.to_vec(), previous: None };
    add_children(&mut layout, 0, random);
    while !layout.pending.is_empty() {
        let pos = random.next_i32_bound(layout.pending.len() as i32) as usize;
        let piece = layout.pending.remove(pos);
        add_children(&mut layout, piece, random);
    }
    // `moveInsideHeights(random, 48, 70)`.
    let bounds = BoundingBox::encapsulating_all(layout.pieces.iter().map(|p| &p.base.bbox)).expect("fortress start");
    let span = 70 - 48 + 1 - bounds.y_span();
    let y0 = if span > 1 { 48 + random.next_i32_bound(span) } else { 48 };
    let dy = y0 - bounds.min_y;
    layout
        .pieces
        .into_iter()
        .map(|mut p| {
            p.base.bbox = p.base.bbox.moved(0, dy, 0);
            Box::new(p) as Box<dyn Piece>
        })
        .collect()
}

fn collides(layout: &Layout, b: &BoundingBox) -> bool {
    layout.pieces.iter().any(|p| p.base.bbox.intersects(b))
}

/// `isOkBox` and no collision.
fn fits(layout: &Layout, b: &BoundingBox) -> bool {
    b.min_y > 10 && !collides(layout, b)
}

/// `BridgeEndFiller.createPiece`.
fn end_filler(layout: &Layout, random: &mut LegacyRandom, foot: (i32, i32, i32), direction: Direction, depth: i32) -> Option<FortressPiece> {
    let b = BoundingBox::orient_box(foot.0, foot.1, foot.2, -1, -3, 0, 5, 10, 8, direction);
    fits(layout, &b).then(|| {
        let seed = random.next_i32();
        oriented(depth, b, direction, Kind::BridgeEndFiller { seed })
    })
}

/// `findAndCreateBridgePieceFactory`.
fn create(layout: &Layout, choice: Choice, random: &mut LegacyRandom, foot: (i32, i32, i32), direction: Direction, depth: i32) -> Option<FortressPiece> {
    let (x, y, z) = foot;
    let make = |ox: i32, oy: i32, oz: i32, w: i32, h: i32, d: i32| BoundingBox::orient_box(x, y, z, ox, oy, oz, w, h, d, direction);
    let (b, kind) = match choice {
        Choice::BridgeStraight => (make(-1, -3, 0, 5, 10, 19), Kind::BridgeStraight),
        Choice::BridgeCrossing => (make(-8, -3, 0, 19, 10, 19), Kind::BridgeCrossing),
        Choice::RoomCrossing => (make(-2, 0, 0, 7, 9, 7), Kind::RoomCrossing),
        Choice::StairsRoom => (make(-2, 0, 0, 7, 11, 7), Kind::StairsRoom),
        Choice::MonsterThrone => (make(-2, 0, 0, 7, 8, 9), Kind::MonsterThrone { placed_spawner: false }),
        Choice::CastleEntrance => (make(-5, -3, 0, 13, 14, 13), Kind::CastleEntrance),
        Choice::CastleSmallCorridor => (make(-1, 0, 0, 5, 7, 5), Kind::CastleSmallCorridor),
        Choice::CastleSmallCorridorRightTurn => {
            let b = make(-1, 0, 0, 5, 7, 5);
            if !fits(layout, &b) {
                return None;
            }
            let needs_chest = random.next_i32_bound(3) == 0;
            return Some(oriented(depth, b, direction, Kind::CastleSmallCorridorRightTurn { needs_chest }));
        }
        Choice::CastleSmallCorridorLeftTurn => {
            let b = make(-1, 0, 0, 5, 7, 5);
            if !fits(layout, &b) {
                return None;
            }
            let needs_chest = random.next_i32_bound(3) == 0;
            return Some(oriented(depth, b, direction, Kind::CastleSmallCorridorLeftTurn { needs_chest }));
        }
        Choice::CastleCorridorStairs => (make(-1, -7, 0, 5, 14, 10), Kind::CastleCorridorStairs),
        Choice::CastleCorridorTBalcony => (make(-3, 0, 0, 9, 7, 9), Kind::CastleCorridorTBalcony),
        Choice::CastleSmallCorridorCrossing => (make(-1, 0, 0, 5, 7, 5), Kind::CastleSmallCorridorCrossing),
        Choice::CastleStalkRoom => (make(-5, -3, 0, 13, 14, 13), Kind::CastleStalkRoom),
    };
    fits(layout, &b).then(|| oriented(depth, b, direction, kind))
}

/// `NetherBridgePiece.generatePiece`.
fn generate_piece(layout: &mut Layout, castle: bool, random: &mut LegacyRandom, foot: (i32, i32, i32), direction: Direction, depth: i32) -> Option<FortressPiece> {
    let list = if castle { &layout.castle } else { &layout.bridge };
    let any = list.iter().any(|w| w.max_place_count > 0 && w.place_count < w.max_place_count);
    let total: i32 = list.iter().map(|w| w.weight).sum();
    let total = if any { total } else { -1 };
    if total > 0 && depth <= 30 {
        for _ in 0..5 {
            let mut selection = random.next_i32_bound(total);
            let count = if castle { layout.castle.len() } else { layout.bridge.len() };
            for i in 0..count {
                let w = if castle { layout.castle[i] } else { layout.bridge[i] };
                selection -= w.weight;
                if selection < 0 {
                    if !w.can_place() || (Some(w.choice) == layout.previous && !w.allow_in_row) {
                        break;
                    }
                    if let Some(piece) = create(layout, w.choice, random, foot, direction, depth) {
                        let list = if castle { &mut layout.castle } else { &mut layout.bridge };
                        list[i].place_count += 1;
                        layout.previous = Some(w.choice);
                        if !list[i].can_place() {
                            list.remove(i);
                        }
                        return Some(piece);
                    }
                }
            }
        }
    }
    end_filler(layout, random, foot, direction, depth)
}

/// `NetherBridgePiece.generateAndAddPiece`.
fn generate_and_add(layout: &mut Layout, random: &mut LegacyRandom, foot: (i32, i32, i32), direction: Direction, depth: i32, castle: bool) {
    let start = layout.pieces[0].base.bbox;
    if (foot.0 - start.min_x).abs() <= 112 && (foot.2 - start.min_z).abs() <= 112 {
        if let Some(piece) = generate_piece(layout, castle, random, foot, direction, depth + 1) {
            layout.pieces.push(piece);
            layout.pending.push(layout.pieces.len() - 1);
        }
    } else {
        // Created (drawing its seed) but never added.
        end_filler(layout, random, foot, direction, depth);
    }
}

fn child_forward(layout: &mut Layout, index: usize, random: &mut LegacyRandom, x_off: i32, y_off: i32, castle: bool) {
    let p = &layout.pieces[index].base;
    let (b, depth) = (p.bbox, p.gen_depth);
    let Some(o) = p.orientation() else { return };
    let foot = match o {
        Direction::North => (b.min_x + x_off, b.min_y + y_off, b.min_z - 1),
        Direction::South => (b.min_x + x_off, b.min_y + y_off, b.max_z + 1),
        Direction::West => (b.min_x - 1, b.min_y + y_off, b.min_z + x_off),
        _ => (b.max_x + 1, b.min_y + y_off, b.min_z + x_off),
    };
    generate_and_add(layout, random, foot, o, depth, castle);
}

fn child_left(layout: &mut Layout, index: usize, random: &mut LegacyRandom, y_off: i32, z_off: i32, castle: bool) {
    let p = &layout.pieces[index].base;
    let (b, depth) = (p.bbox, p.gen_depth);
    let Some(o) = p.orientation() else { return };
    let (foot, direction) = match o {
        Direction::North | Direction::South => ((b.min_x - 1, b.min_y + y_off, b.min_z + z_off), Direction::West),
        _ => ((b.min_x + z_off, b.min_y + y_off, b.min_z - 1), Direction::North),
    };
    generate_and_add(layout, random, foot, direction, depth, castle);
}

fn child_right(layout: &mut Layout, index: usize, random: &mut LegacyRandom, y_off: i32, z_off: i32, castle: bool) {
    let p = &layout.pieces[index].base;
    let (b, depth) = (p.bbox, p.gen_depth);
    let Some(o) = p.orientation() else { return };
    let (foot, direction) = match o {
        Direction::North | Direction::South => ((b.max_x + 1, b.min_y + y_off, b.min_z + z_off), Direction::East),
        _ => ((b.min_x + z_off, b.min_y + y_off, b.max_z + 1), Direction::South),
    };
    generate_and_add(layout, random, foot, direction, depth, castle);
}

/// `addChildren` of the piece at `index`.
fn add_children(layout: &mut Layout, index: usize, random: &mut LegacyRandom) {
    match layout.pieces[index].kind {
        Kind::Start | Kind::BridgeCrossing => {
            child_forward(layout, index, random, 8, 3, false);
            child_left(layout, index, random, 3, 8, false);
            child_right(layout, index, random, 3, 8, false);
        }
        Kind::BridgeStraight => child_forward(layout, index, random, 1, 3, false),
        Kind::CastleCorridorStairs => child_forward(layout, index, random, 1, 0, true),
        Kind::CastleCorridorTBalcony => {
            let o = layout.pieces[index].base.orientation();
            let z_off = if matches!(o, Some(Direction::West | Direction::North)) { 5 } else { 1 };
            let castle = random.next_i32_bound(8) > 0;
            child_left(layout, index, random, 0, z_off, castle);
            let castle = random.next_i32_bound(8) > 0;
            child_right(layout, index, random, 0, z_off, castle);
        }
        Kind::CastleEntrance => child_forward(layout, index, random, 5, 3, true),
        Kind::CastleSmallCorridorCrossing => {
            child_forward(layout, index, random, 1, 0, true);
            child_left(layout, index, random, 0, 1, true);
            child_right(layout, index, random, 0, 1, true);
        }
        Kind::CastleSmallCorridorLeftTurn { .. } => child_left(layout, index, random, 0, 1, true),
        Kind::CastleSmallCorridor => child_forward(layout, index, random, 1, 0, true),
        Kind::CastleSmallCorridorRightTurn { .. } => child_right(layout, index, random, 0, 1, true),
        Kind::CastleStalkRoom => {
            child_forward(layout, index, random, 5, 3, true);
            child_forward(layout, index, random, 5, 11, true);
        }
        Kind::RoomCrossing => {
            child_forward(layout, index, random, 2, 0, false);
            child_left(layout, index, random, 0, 2, false);
            child_right(layout, index, random, 0, 2, false);
        }
        Kind::StairsRoom => child_right(layout, index, random, 6, 2, false),
        Kind::MonsterThrone { .. } | Kind::BridgeEndFiller { .. } => {}
    }
}

/// Block states the pieces use.
struct B {
    bricks: BlockStateId,
    air: BlockStateId,
    fence: BlockStateId,
    stairs: BlockStateId,
    lava: BlockStateId,
    soul_sand: BlockStateId,
    wart: BlockStateId,
    spawner: BlockStateId,
}

impl B {
    fn load(ctx: &Ctx) -> Self {
        let s = |name: &str| ctx.registries().blocks.parse_state(name).expect("fortress block");
        Self {
            bricks: s("minecraft:nether_bricks"),
            air: s("minecraft:air"),
            fence: s("minecraft:nether_brick_fence"),
            stairs: s("minecraft:nether_brick_stairs"),
            lava: s("minecraft:lava"),
            soul_sand: s("minecraft:soul_sand"),
            wart: s("minecraft:nether_wart"),
            spawner: s("minecraft:spawner"),
        }
    }

    /// A nether brick fence connected on the given sides.
    fn fence(&self, ctx: &Ctx, sides: &[&str]) -> BlockStateId {
        sides.iter().fold(self.fence, |s, side| ctx.with(s, side, "true"))
    }

    fn stairs(&self, ctx: &Ctx, facing: Direction) -> BlockStateId {
        ctx.with(self.stairs, "facing", facing.name())
    }
}

impl Piece for FortressPiece {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn type_name(&self) -> &'static str {
        match self.kind {
            // StartPiece is built through the BridgeCrossing constructor.
            Kind::Start => "minecraft:nebcr",
            Kind::BridgeCrossing => "minecraft:nebcr",
            Kind::BridgeEndFiller { .. } => "minecraft:nebef",
            Kind::BridgeStraight => "minecraft:nebs",
            Kind::CastleCorridorStairs => "minecraft:neccs",
            Kind::CastleCorridorTBalcony => "minecraft:nectb",
            Kind::CastleEntrance => "minecraft:nece",
            Kind::CastleSmallCorridorCrossing => "minecraft:nescsc",
            Kind::CastleSmallCorridorLeftTurn { .. } => "minecraft:nesclt",
            Kind::CastleSmallCorridor => "minecraft:nesc",
            Kind::CastleSmallCorridorRightTurn { .. } => "minecraft:nescrt",
            Kind::CastleStalkRoom => "minecraft:necsr",
            Kind::MonsterThrone { .. } => "minecraft:nemt",
            Kind::RoomCrossing => "minecraft:nerc",
            Kind::StairsRoom => "minecraft:nesr",
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, _reference: BlockPos) {
        let b = B::load(ctx);
        let p = self.base.clone();
        let bb = chunk_bb;
        let bricks = b.bricks;
        let air = b.air;
        let bx = |ctx: &mut Ctx, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, s: BlockStateId| p.generate_box(ctx, bb, x0, y0, z0, x1, y1, z1, s, s, false);
        let column = |ctx: &mut Ctx, x: i32, z: i32| p.fill_column_down(ctx, bricks, x, -1, z, bb);
        match &mut self.kind {
            Kind::Start | Kind::BridgeCrossing => {
                bx(ctx, 7, 3, 0, 11, 4, 18, bricks);
                bx(ctx, 0, 3, 7, 18, 4, 11, bricks);
                bx(ctx, 8, 5, 0, 10, 7, 18, air);
                bx(ctx, 0, 5, 8, 18, 7, 10, air);
                bx(ctx, 7, 5, 0, 7, 5, 7, bricks);
                bx(ctx, 7, 5, 11, 7, 5, 18, bricks);
                bx(ctx, 11, 5, 0, 11, 5, 7, bricks);
                bx(ctx, 11, 5, 11, 11, 5, 18, bricks);
                bx(ctx, 0, 5, 7, 7, 5, 7, bricks);
                bx(ctx, 11, 5, 7, 18, 5, 7, bricks);
                bx(ctx, 0, 5, 11, 7, 5, 11, bricks);
                bx(ctx, 11, 5, 11, 18, 5, 11, bricks);
                bx(ctx, 7, 2, 0, 11, 2, 5, bricks);
                bx(ctx, 7, 2, 13, 11, 2, 18, bricks);
                bx(ctx, 7, 0, 0, 11, 1, 3, bricks);
                bx(ctx, 7, 0, 15, 11, 1, 18, bricks);
                for x in 7..=11 {
                    for z in 0..=2 {
                        column(ctx, x, z);
                        column(ctx, x, 18 - z);
                    }
                }
                bx(ctx, 0, 2, 7, 5, 2, 11, bricks);
                bx(ctx, 13, 2, 7, 18, 2, 11, bricks);
                bx(ctx, 0, 0, 7, 3, 1, 11, bricks);
                bx(ctx, 15, 0, 7, 18, 1, 11, bricks);
                for x in 0..=2 {
                    for z in 7..=11 {
                        column(ctx, x, z);
                        column(ctx, 18 - x, z);
                    }
                }
            }
            Kind::BridgeEndFiller { seed } => {
                let mut own = LegacyRandom::new(i64::from(*seed));
                for x in 0..=4 {
                    for y in 3..=4 {
                        let z = own.next_i32_bound(8);
                        bx(ctx, x, y, 0, x, y, z, bricks);
                    }
                }
                let z = own.next_i32_bound(8);
                bx(ctx, 0, 5, 0, 0, 5, z, bricks);
                let z = own.next_i32_bound(8);
                bx(ctx, 4, 5, 0, 4, 5, z, bricks);
                for x in 0..=4 {
                    let z = own.next_i32_bound(5);
                    bx(ctx, x, 2, 0, x, 2, z, bricks);
                }
                for x in 0..=4 {
                    for y in 0..=1 {
                        let z = own.next_i32_bound(3);
                        bx(ctx, x, y, 0, x, y, z, bricks);
                    }
                }
            }
            Kind::BridgeStraight => {
                bx(ctx, 0, 3, 0, 4, 4, 18, bricks);
                bx(ctx, 1, 5, 0, 3, 7, 18, air);
                bx(ctx, 0, 5, 0, 0, 5, 18, bricks);
                bx(ctx, 4, 5, 0, 4, 5, 18, bricks);
                bx(ctx, 0, 2, 0, 4, 2, 5, bricks);
                bx(ctx, 0, 2, 13, 4, 2, 18, bricks);
                bx(ctx, 0, 0, 0, 4, 1, 3, bricks);
                bx(ctx, 0, 0, 15, 4, 1, 18, bricks);
                for x in 0..=4 {
                    for z in 0..=2 {
                        column(ctx, x, z);
                        column(ctx, x, 18 - z);
                    }
                }
                let nse = b.fence(ctx, &["north", "south", "east"]);
                let nsw = b.fence(ctx, &["north", "south", "west"]);
                bx(ctx, 0, 1, 1, 0, 4, 1, nse);
                bx(ctx, 0, 3, 4, 0, 4, 4, nse);
                bx(ctx, 0, 3, 14, 0, 4, 14, nse);
                bx(ctx, 0, 1, 17, 0, 4, 17, nse);
                bx(ctx, 4, 1, 1, 4, 4, 1, nsw);
                bx(ctx, 4, 3, 4, 4, 4, 4, nsw);
                bx(ctx, 4, 3, 14, 4, 4, 14, nsw);
                bx(ctx, 4, 1, 17, 4, 4, 17, nsw);
            }
            Kind::CastleCorridorStairs => {
                let stairs = b.stairs(ctx, Direction::South);
                let ns = b.fence(ctx, &["north", "south"]);
                for step in 0..=9 {
                    let floor = (7 - step).max(1);
                    let roof = (floor + 5).max(14 - step).min(13);
                    let z = step;
                    bx(ctx, 0, 0, z, 4, floor, z, bricks);
                    bx(ctx, 1, floor + 1, z, 3, roof - 1, z, air);
                    if step <= 6 {
                        p.place_block(ctx, stairs, 1, floor + 1, z, bb);
                        p.place_block(ctx, stairs, 2, floor + 1, z, bb);
                        p.place_block(ctx, stairs, 3, floor + 1, z, bb);
                    }
                    bx(ctx, 0, roof, z, 4, roof, z, bricks);
                    bx(ctx, 0, floor + 1, z, 0, roof - 1, z, bricks);
                    bx(ctx, 4, floor + 1, z, 4, roof - 1, z, bricks);
                    if step & 1 == 0 {
                        bx(ctx, 0, floor + 2, z, 0, floor + 3, z, ns);
                        bx(ctx, 4, floor + 2, z, 4, floor + 3, z, ns);
                    }
                    for x in 0..=4 {
                        column(ctx, x, z);
                    }
                }
            }
            Kind::CastleCorridorTBalcony => {
                let ns = b.fence(ctx, &["north", "south"]);
                let we = b.fence(ctx, &["west", "east"]);
                bx(ctx, 0, 0, 0, 8, 1, 8, bricks);
                bx(ctx, 0, 2, 0, 8, 5, 8, air);
                bx(ctx, 0, 6, 0, 8, 6, 5, bricks);
                bx(ctx, 0, 2, 0, 2, 5, 0, bricks);
                bx(ctx, 6, 2, 0, 8, 5, 0, bricks);
                bx(ctx, 1, 3, 0, 1, 4, 0, we);
                bx(ctx, 7, 3, 0, 7, 4, 0, we);
                bx(ctx, 0, 2, 4, 8, 2, 8, bricks);
                bx(ctx, 1, 1, 4, 2, 2, 4, air);
                bx(ctx, 6, 1, 4, 7, 2, 4, air);
                bx(ctx, 1, 3, 8, 7, 3, 8, we);
                let es = b.fence(ctx, &["east", "south"]);
                let ws = b.fence(ctx, &["west", "south"]);
                p.place_block(ctx, es, 0, 3, 8, bb);
                p.place_block(ctx, ws, 8, 3, 8, bb);
                bx(ctx, 0, 3, 6, 0, 3, 7, ns);
                bx(ctx, 8, 3, 6, 8, 3, 7, ns);
                bx(ctx, 0, 3, 4, 0, 5, 5, bricks);
                bx(ctx, 8, 3, 4, 8, 5, 5, bricks);
                bx(ctx, 1, 3, 5, 2, 5, 5, bricks);
                bx(ctx, 6, 3, 5, 7, 5, 5, bricks);
                bx(ctx, 1, 4, 5, 1, 5, 5, we);
                bx(ctx, 7, 4, 5, 7, 5, 5, we);
                for z in 0..=5 {
                    for x in 0..=8 {
                        column(ctx, x, z);
                    }
                }
            }
            Kind::CastleEntrance => {
                castle_room_walls(ctx, &p, bb, &b);
                bx(ctx, 5, 8, 0, 7, 8, 0, b.fence);
                let we = b.fence(ctx, &["west", "east"]);
                let ns = b.fence(ctx, &["north", "south"]);
                castle_room_crown(ctx, &p, bb, &b, we, ns);
                let nsw = b.fence(ctx, &["north", "south", "west"]);
                let nse = b.fence(ctx, &["north", "south", "east"]);
                for z in (3..=9).step_by(2) {
                    bx(ctx, 1, 7, z, 1, 8, z, nsw);
                    bx(ctx, 11, 7, z, 11, 8, z, nse);
                }
                castle_room_base(ctx, &p, bb, &b);
                bx(ctx, 5, 5, 5, 7, 5, 7, bricks);
                bx(ctx, 6, 1, 6, 6, 4, 6, air);
                p.place_block(ctx, bricks, 6, 0, 6, bb);
                p.place_block(ctx, b.lava, 6, 5, 6, bb);
                let pos = p.world_pos(6, 5, 6);
                if bb.is_inside(pos) {
                    ctx.schedule_fluid_tick(pos);
                }
            }
            Kind::CastleSmallCorridorCrossing => {
                bx(ctx, 0, 0, 0, 4, 1, 4, bricks);
                bx(ctx, 0, 2, 0, 4, 5, 4, air);
                bx(ctx, 0, 2, 0, 0, 5, 0, bricks);
                bx(ctx, 4, 2, 0, 4, 5, 0, bricks);
                bx(ctx, 0, 2, 4, 0, 5, 4, bricks);
                bx(ctx, 4, 2, 4, 4, 5, 4, bricks);
                bx(ctx, 0, 6, 0, 4, 6, 4, bricks);
                for x in 0..=4 {
                    for z in 0..=4 {
                        column(ctx, x, z);
                    }
                }
            }
            Kind::CastleSmallCorridorLeftTurn { needs_chest } => {
                bx(ctx, 0, 0, 0, 4, 1, 4, bricks);
                bx(ctx, 0, 2, 0, 4, 5, 4, air);
                let we = b.fence(ctx, &["west", "east"]);
                let ns = b.fence(ctx, &["north", "south"]);
                bx(ctx, 4, 2, 0, 4, 5, 4, bricks);
                bx(ctx, 4, 3, 1, 4, 4, 1, ns);
                bx(ctx, 4, 3, 3, 4, 4, 3, ns);
                bx(ctx, 0, 2, 0, 0, 5, 0, bricks);
                bx(ctx, 0, 2, 4, 3, 5, 4, bricks);
                bx(ctx, 1, 3, 4, 1, 4, 4, we);
                bx(ctx, 3, 3, 4, 3, 4, 4, we);
                if *needs_chest && bb.is_inside(p.world_pos(3, 2, 3)) {
                    *needs_chest = false;
                    p.create_chest(ctx, bb, random, 3, 2, 3, "minecraft:chests/nether_bridge");
                }
                bx(ctx, 0, 6, 0, 4, 6, 4, bricks);
                for x in 0..=4 {
                    for z in 0..=4 {
                        column(ctx, x, z);
                    }
                }
            }
            Kind::CastleSmallCorridor => {
                bx(ctx, 0, 0, 0, 4, 1, 4, bricks);
                bx(ctx, 0, 2, 0, 4, 5, 4, air);
                let ns = b.fence(ctx, &["north", "south"]);
                bx(ctx, 0, 2, 0, 0, 5, 4, bricks);
                bx(ctx, 4, 2, 0, 4, 5, 4, bricks);
                bx(ctx, 0, 3, 1, 0, 4, 1, ns);
                bx(ctx, 0, 3, 3, 0, 4, 3, ns);
                bx(ctx, 4, 3, 1, 4, 4, 1, ns);
                bx(ctx, 4, 3, 3, 4, 4, 3, ns);
                bx(ctx, 0, 6, 0, 4, 6, 4, bricks);
                for x in 0..=4 {
                    for z in 0..=4 {
                        column(ctx, x, z);
                    }
                }
            }
            Kind::CastleSmallCorridorRightTurn { needs_chest } => {
                bx(ctx, 0, 0, 0, 4, 1, 4, bricks);
                bx(ctx, 0, 2, 0, 4, 5, 4, air);
                let we = b.fence(ctx, &["west", "east"]);
                let ns = b.fence(ctx, &["north", "south"]);
                bx(ctx, 0, 2, 0, 0, 5, 4, bricks);
                bx(ctx, 0, 3, 1, 0, 4, 1, ns);
                bx(ctx, 0, 3, 3, 0, 4, 3, ns);
                bx(ctx, 4, 2, 0, 4, 5, 0, bricks);
                bx(ctx, 1, 2, 4, 4, 5, 4, bricks);
                bx(ctx, 1, 3, 4, 1, 4, 4, we);
                bx(ctx, 3, 3, 4, 3, 4, 4, we);
                if *needs_chest && bb.is_inside(p.world_pos(1, 2, 3)) {
                    *needs_chest = false;
                    p.create_chest(ctx, bb, random, 1, 2, 3, "minecraft:chests/nether_bridge");
                }
                bx(ctx, 0, 6, 0, 4, 6, 4, bricks);
                for x in 0..=4 {
                    for z in 0..=4 {
                        column(ctx, x, z);
                    }
                }
            }
            Kind::CastleStalkRoom => {
                castle_room_walls(ctx, &p, bb, &b);
                let we = b.fence(ctx, &["west", "east"]);
                let ns = b.fence(ctx, &["north", "south"]);
                let nsw = b.fence(ctx, &["north", "south", "west"]);
                let nse = b.fence(ctx, &["north", "south", "east"]);
                castle_room_crown(ctx, &p, bb, &b, we, ns);
                for z in (3..=9).step_by(2) {
                    bx(ctx, 1, 7, z, 1, 8, z, nsw);
                    bx(ctx, 11, 7, z, 11, 8, z, nse);
                }
                let stairs = b.stairs(ctx, Direction::North);
                for i in 0..=6 {
                    let z = i + 4;
                    for x in 5..=7 {
                        p.place_block(ctx, stairs, x, 5 + i, z, bb);
                    }
                    if (5..=8).contains(&z) {
                        bx(ctx, 5, 5, z, 7, i + 4, z, bricks);
                    } else if (9..=10).contains(&z) {
                        bx(ctx, 5, 8, z, 7, i + 4, z, bricks);
                    }
                    if i >= 1 {
                        bx(ctx, 5, 6 + i, z, 7, 9 + i, z, air);
                    }
                }
                for x in 5..=7 {
                    p.place_block(ctx, stairs, x, 12, 11, bb);
                }
                bx(ctx, 5, 6, 7, 5, 7, 7, nse);
                bx(ctx, 7, 6, 7, 7, 7, 7, nsw);
                bx(ctx, 5, 13, 12, 7, 13, 12, air);
                bx(ctx, 2, 5, 2, 3, 5, 3, bricks);
                bx(ctx, 2, 5, 9, 3, 5, 10, bricks);
                bx(ctx, 2, 5, 4, 2, 5, 8, bricks);
                bx(ctx, 9, 5, 2, 10, 5, 3, bricks);
                bx(ctx, 9, 5, 9, 10, 5, 10, bricks);
                bx(ctx, 10, 5, 4, 10, 5, 8, bricks);
                let east = ctx.with(stairs, "facing", "east");
                let west = ctx.with(stairs, "facing", "west");
                for z in [2, 3, 9, 10] {
                    p.place_block(ctx, west, 4, 5, z, bb);
                }
                for z in [2, 3, 9, 10] {
                    p.place_block(ctx, east, 8, 5, z, bb);
                }
                bx(ctx, 3, 4, 4, 4, 4, 8, b.soul_sand);
                bx(ctx, 8, 4, 4, 9, 4, 8, b.soul_sand);
                bx(ctx, 3, 5, 4, 4, 5, 8, b.wart);
                bx(ctx, 8, 5, 4, 9, 5, 8, b.wart);
                castle_room_base(ctx, &p, bb, &b);
            }
            Kind::MonsterThrone { placed_spawner } => {
                bx(ctx, 0, 2, 0, 6, 7, 7, air);
                bx(ctx, 1, 0, 0, 5, 1, 7, bricks);
                bx(ctx, 1, 2, 1, 5, 2, 7, bricks);
                bx(ctx, 1, 3, 2, 5, 3, 7, bricks);
                bx(ctx, 1, 4, 3, 5, 4, 7, bricks);
                bx(ctx, 1, 2, 0, 1, 4, 2, bricks);
                bx(ctx, 5, 2, 0, 5, 4, 2, bricks);
                bx(ctx, 1, 5, 2, 1, 5, 3, bricks);
                bx(ctx, 5, 5, 2, 5, 5, 3, bricks);
                bx(ctx, 0, 5, 3, 0, 5, 8, bricks);
                bx(ctx, 6, 5, 3, 6, 5, 8, bricks);
                bx(ctx, 1, 5, 8, 5, 5, 8, bricks);
                let we = b.fence(ctx, &["west", "east"]);
                let ns = b.fence(ctx, &["north", "south"]);
                let w = b.fence(ctx, &["west"]);
                let e = b.fence(ctx, &["east"]);
                let en = b.fence(ctx, &["east", "north"]);
                let wn = b.fence(ctx, &["west", "north"]);
                let es = b.fence(ctx, &["east", "south"]);
                let ws = b.fence(ctx, &["west", "south"]);
                p.place_block(ctx, w, 1, 6, 3, bb);
                p.place_block(ctx, e, 5, 6, 3, bb);
                p.place_block(ctx, en, 0, 6, 3, bb);
                p.place_block(ctx, wn, 6, 6, 3, bb);
                bx(ctx, 0, 6, 4, 0, 6, 7, ns);
                bx(ctx, 6, 6, 4, 6, 6, 7, ns);
                p.place_block(ctx, es, 0, 6, 8, bb);
                p.place_block(ctx, ws, 6, 6, 8, bb);
                bx(ctx, 1, 6, 8, 5, 6, 8, we);
                p.place_block(ctx, e, 1, 7, 8, bb);
                bx(ctx, 2, 7, 8, 4, 7, 8, we);
                p.place_block(ctx, w, 5, 7, 8, bb);
                p.place_block(ctx, e, 2, 8, 8, bb);
                p.place_block(ctx, we, 3, 8, 8, bb);
                p.place_block(ctx, w, 4, 8, 8, bb);
                if !*placed_spawner {
                    let pos = p.world_pos(3, 5, 5);
                    if bb.is_inside(pos) {
                        *placed_spawner = true;
                        ctx.set_block(pos, b.spawner);
                        ctx.region.set_spawner_entity(pos.x, pos.y, pos.z, "minecraft:blaze");
                    }
                }
                for x in 0..=6 {
                    for z in 0..=6 {
                        column(ctx, x, z);
                    }
                }
            }
            Kind::RoomCrossing => {
                bx(ctx, 0, 0, 0, 6, 1, 6, bricks);
                bx(ctx, 0, 2, 0, 6, 7, 6, air);
                bx(ctx, 0, 2, 0, 1, 6, 0, bricks);
                bx(ctx, 0, 2, 6, 1, 6, 6, bricks);
                bx(ctx, 5, 2, 0, 6, 6, 0, bricks);
                bx(ctx, 5, 2, 6, 6, 6, 6, bricks);
                bx(ctx, 0, 2, 0, 0, 6, 1, bricks);
                bx(ctx, 0, 2, 5, 0, 6, 6, bricks);
                bx(ctx, 6, 2, 0, 6, 6, 1, bricks);
                bx(ctx, 6, 2, 5, 6, 6, 6, bricks);
                let we = b.fence(ctx, &["west", "east"]);
                let ns = b.fence(ctx, &["north", "south"]);
                bx(ctx, 2, 6, 0, 4, 6, 0, bricks);
                bx(ctx, 2, 5, 0, 4, 5, 0, we);
                bx(ctx, 2, 6, 6, 4, 6, 6, bricks);
                bx(ctx, 2, 5, 6, 4, 5, 6, we);
                bx(ctx, 0, 6, 2, 0, 6, 4, bricks);
                bx(ctx, 0, 5, 2, 0, 5, 4, ns);
                bx(ctx, 6, 6, 2, 6, 6, 4, bricks);
                bx(ctx, 6, 5, 2, 6, 5, 4, ns);
                for x in 0..=6 {
                    for z in 0..=6 {
                        column(ctx, x, z);
                    }
                }
            }
            Kind::StairsRoom => {
                bx(ctx, 0, 0, 0, 6, 1, 6, bricks);
                bx(ctx, 0, 2, 0, 6, 10, 6, air);
                bx(ctx, 0, 2, 0, 1, 8, 0, bricks);
                bx(ctx, 5, 2, 0, 6, 8, 0, bricks);
                bx(ctx, 0, 2, 1, 0, 8, 6, bricks);
                bx(ctx, 6, 2, 1, 6, 8, 6, bricks);
                bx(ctx, 1, 2, 6, 5, 8, 6, bricks);
                let we = b.fence(ctx, &["west", "east"]);
                let ns = b.fence(ctx, &["north", "south"]);
                bx(ctx, 0, 3, 2, 0, 5, 4, ns);
                bx(ctx, 6, 3, 2, 6, 5, 2, ns);
                bx(ctx, 6, 3, 4, 6, 5, 4, ns);
                p.place_block(ctx, bricks, 5, 2, 5, bb);
                bx(ctx, 4, 2, 5, 4, 3, 5, bricks);
                bx(ctx, 3, 2, 5, 3, 4, 5, bricks);
                bx(ctx, 2, 2, 5, 2, 5, 5, bricks);
                bx(ctx, 1, 2, 5, 1, 6, 5, bricks);
                bx(ctx, 1, 7, 1, 5, 7, 4, bricks);
                bx(ctx, 6, 8, 2, 6, 8, 4, air);
                bx(ctx, 2, 6, 0, 4, 8, 0, bricks);
                bx(ctx, 2, 5, 0, 4, 5, 0, we);
                for x in 0..=6 {
                    for z in 0..=6 {
                        column(ctx, x, z);
                    }
                }
            }
        }
    }
}

/// The shared outer walls of `CastleEntrance` and `CastleStalkRoom`.
fn castle_room_walls(ctx: &mut Ctx, p: &PieceBase, bb: &BoundingBox, b: &B) {
    let (bricks, air) = (b.bricks, b.air);
    let bx = |ctx: &mut Ctx, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, s: BlockStateId| p.generate_box(ctx, bb, x0, y0, z0, x1, y1, z1, s, s, false);
    bx(ctx, 0, 3, 0, 12, 4, 12, bricks);
    bx(ctx, 0, 5, 0, 12, 13, 12, air);
    bx(ctx, 0, 5, 0, 1, 12, 12, bricks);
    bx(ctx, 11, 5, 0, 12, 12, 12, bricks);
    bx(ctx, 2, 5, 11, 4, 12, 12, bricks);
    bx(ctx, 8, 5, 11, 10, 12, 12, bricks);
    bx(ctx, 5, 9, 11, 7, 12, 12, bricks);
    bx(ctx, 2, 5, 0, 4, 12, 1, bricks);
    bx(ctx, 8, 5, 0, 10, 12, 1, bricks);
    bx(ctx, 5, 9, 0, 7, 12, 1, bricks);
    bx(ctx, 2, 11, 2, 10, 12, 10, bricks);
}

/// The fenced parapet of `CastleEntrance` and `CastleStalkRoom`.
fn castle_room_crown(ctx: &mut Ctx, p: &PieceBase, bb: &BoundingBox, b: &B, we: BlockStateId, ns: BlockStateId) {
    let bx = |ctx: &mut Ctx, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, s: BlockStateId| p.generate_box(ctx, bb, x0, y0, z0, x1, y1, z1, s, s, false);
    for i in (1..=11).step_by(2) {
        bx(ctx, i, 10, 0, i, 11, 0, we);
        bx(ctx, i, 10, 12, i, 11, 12, we);
        bx(ctx, 0, 10, i, 0, 11, i, ns);
        bx(ctx, 12, 10, i, 12, 11, i, ns);
        p.place_block(ctx, b.bricks, i, 13, 0, bb);
        p.place_block(ctx, b.bricks, i, 13, 12, bb);
        p.place_block(ctx, b.bricks, 0, 13, i, bb);
        p.place_block(ctx, b.bricks, 12, 13, i, bb);
        if i != 11 {
            p.place_block(ctx, we, i + 1, 13, 0, bb);
            p.place_block(ctx, we, i + 1, 13, 12, bb);
            p.place_block(ctx, ns, 0, 13, i + 1, bb);
            p.place_block(ctx, ns, 12, 13, i + 1, bb);
        }
    }
    let ne = b.fence(ctx, &["north", "east"]);
    let se = b.fence(ctx, &["south", "east"]);
    let sw = b.fence(ctx, &["south", "west"]);
    let nw = b.fence(ctx, &["north", "west"]);
    p.place_block(ctx, ne, 0, 13, 0, bb);
    p.place_block(ctx, se, 0, 13, 12, bb);
    p.place_block(ctx, sw, 12, 13, 12, bb);
    p.place_block(ctx, nw, 12, 13, 0, bb);
}

/// The cross-shaped foundation of `CastleEntrance` and `CastleStalkRoom`.
fn castle_room_base(ctx: &mut Ctx, p: &PieceBase, bb: &BoundingBox, b: &B) {
    let bricks = b.bricks;
    let bx = |ctx: &mut Ctx, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32| p.generate_box(ctx, bb, x0, y0, z0, x1, y1, z1, bricks, bricks, false);
    bx(ctx, 4, 2, 0, 8, 2, 12);
    bx(ctx, 0, 2, 4, 12, 2, 8);
    bx(ctx, 4, 0, 0, 8, 1, 3);
    bx(ctx, 4, 0, 9, 8, 1, 12);
    bx(ctx, 0, 0, 4, 3, 1, 8);
    bx(ctx, 9, 0, 4, 12, 1, 8);
    for x in 4..=8 {
        for z in 0..=2 {
            p.fill_column_down(ctx, bricks, x, -1, z, bb);
            p.fill_column_down(ctx, bricks, x, -1, 12 - z, bb);
        }
    }
    for x in 0..=2 {
        for z in 4..=8 {
            p.fill_column_down(ctx, bricks, x, -1, z, bb);
            p.fill_column_down(ctx, bricks, 12 - x, -1, z, bb);
        }
    }
}
